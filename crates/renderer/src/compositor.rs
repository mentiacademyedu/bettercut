//! The wgpu compositor (§21, §22).
//!
//! ```text
//! Decode source frame
//! -> upload texture + colour conversion   (§21a, done at decode for now)
//! -> transform
//! -> composite track
//! -> output to preview texture OR encoder
//! ```
//!
//! The output is a `wgpu::Texture`. §4.1's whole argument rests on that: with
//! egui sharing the device, `egui-wgpu` registers this texture as a `TextureId`
//! and paints it. There is no copy, no interop layer, and no overlay window.
//! The Milestone 0 spike measured that path end to end (ADR 005).

use std::collections::HashMap;

use bettercut_media::VideoFrame;
use bettercut_timeline::{MasterLook, Resolution, Transform};
use eframe::wgpu;

use crate::blur::BlurPass;
use crate::config::RenderConfig;
use crate::error::RenderError;
use crate::graph::{
    EffectContext, EffectInput, EffectNode, EffectParams, EffectTexture, TargetPool, run_chain,
};

/// Uniform stride. wgpu requires dynamic uniform offsets to be aligned, and
/// 256 is the limit on every backend we target.
const UNIFORM_STRIDE: u64 = 256;

/// How many layers one composite may draw before the uniform buffer grows.
const INITIAL_LAYER_CAPACITY: u64 = 16;

/// The longest [`Compositor::read_pixel`] waits for its one-texel copy.
///
/// A single pixel from a texture already rendered takes a frame or two. Two
/// seconds is a hundred times that — long enough that a busy GPU still answers,
/// short enough that a stalled one reads as "nothing to pick" rather than as
/// the editor having frozen.
const READ_PIXEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// One thing to draw, in compositing order.
pub struct Layer<'a> {
    pub frame: &'a VideoFrame,

    /// Everything about how this layer is drawn, as one value.
    ///
    /// Six separate fields once, which made this struct a *second* list of what
    /// a clip's appearance contains — and the preview and the export each
    /// unpacked a `ClipLook` into it by hand. §46 is enforced by golden frames
    /// from the layer inwards, so a field dropped on one of those two paths and
    /// not the other went unnoticed: removing the mask from the export alone
    /// passed every test in the workspace. One value, filled straight from the
    /// request, is one list.
    pub look: bettercut_timeline::ClipLook,
}

/// A grade part-way up the stack: an adjustment layer, as the renderer sees it.
///
/// Applies to the picture composited beneath it and to nothing drawn above, so
/// a filter over a stretch of footage leaves the titles on top alone. See
/// [`Compositor::composite_graded`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grade {
    /// How many layers, counted from the bottom, it grades.
    pub beneath: usize,
    /// What it does, carried whole from the adjustment clip — for the reason
    /// `Layer::look` is one value rather than a field each.
    pub look: bettercut_timeline::AdjustmentLook,
}

/// A GPU texture holding one decoded frame.
struct SourceTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,

    config: RenderConfig,

    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    /// The same texture seen as [`Compositor::PRESENT_FORMAT`].
    present_view: wgpu::TextureView,
    /// A second copy of the finished frame, made on request
    /// ([`Compositor::keep_snapshot`]).
    ///
    /// What "before and after, side by side" needs: two versions of the same
    /// instant on screen at once means compositing twice, and the first result
    /// has to survive the second composite. A texture-to-texture copy on the
    /// GPU, so nothing travels back across the bus for it.
    snapshot: Option<(wgpu::Texture, wgpu::TextureView)>,

    /// One per §22 blend mode, in `BlendMode::ALL` order. Pipelines are the
    /// only place a blend state can live in wgpu, so four modes are four
    /// pipelines — they share everything else, including the shader.
    pipelines: [wgpu::RenderPipeline; 4],
    layer_bind_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    uniform_capacity: u64,

    /// Reused between frames, keyed by size.
    ///
    /// §68 and §73: allocating a 1080p texture per frame would churn tens of
    /// megabytes a second through the driver. Sizes repeat almost always, so a
    /// map keyed by size is enough.
    pool: HashMap<(u32, u32), Vec<SourceTexture>>,
    in_flight: Vec<SourceTexture>,

    /// The effect graph (§20): effects that need their own passes, in the order
    /// they run, ahead of the composite draw.
    ///
    /// Fixed for now because there is one of them. It is a `Vec<Box<dyn ..>>`
    /// rather than a field per effect because that is the difference between
    /// adding an effect and *editing the compositor* to add an effect — and
    /// §29 wants effect chains to become data, which starts by them being a
    /// list at all.
    effects: Vec<Box<dyn EffectNode>>,
    /// Scratch textures shared by every node, so a two-node chain reuses what
    /// the frame before finished with instead of allocating twice as much.
    targets: TargetPool,
    /// The frame number the next composite's film grain is drawn for.
    grain_seed: u32,
    /// Clear to nothing instead of the background colour, so what no layer
    /// covers stays see-through — for an export with a transparent background.
    transparent: bool,
    /// Colour lookup tables the caller has loaded, by id (`crate::lut`).
    luts: crate::lut::LutTables,
}

impl Compositor {
    /// The working-space format (§21a.1): sRGB-encoded 8-bit RGBA.
    ///
    /// sRGB-*aware*, deliberately: the hardware linearizes on sample and
    /// re-encodes on write, so §22's alpha blending happens in linear light,
    /// which is the only place it is physically correct.
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    /// The same bytes, viewed as plain unorm, for handing to egui.
    ///
    /// **Not cosmetic.** `egui.wgsl` states its contract in a comment — *"We
    /// expect 'normal' textures that are NOT sRGB-aware"* — and then applies
    /// `linear_from_gamma_rgb` to whatever it samples. Give it an sRGB view and
    /// the hardware linearizes first, egui linearizes the result a second time,
    /// and the picture arrives far too dark: mid-grey 0.5 lands near 0.21.
    ///
    /// Both of egui's fragment entry points assume the same thing, so the
    /// surface format does not rescue it. The fix is to hand egui the raw
    /// stored bytes, which are already sRGB-encoded, exactly as it expects.
    pub const PRESENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: RenderConfig,
    ) -> Result<Self, RenderError> {
        let (target, target_view, present_view) = create_target(&device, config.resolution)?;

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("composite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let uniform_capacity = INITIAL_LAYER_CAPACITY;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer uniforms"),
            size: uniform_capacity * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        // One buffer, one binding, a different slice per layer.
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                },
            ],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("source texture bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let layer_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer bind group"),
            layout: &layer_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    }),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite layout"),
            bind_group_layouts: &[Some(&layer_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let build = |mode: bettercut_timeline::BlendMode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("composite pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::FORMAT,
                        blend: Some(blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let pipelines = bettercut_timeline::BlendMode::ALL.map(build);

        // In order: the LUT, then blur, then sharpen. None knows the others are
        // there — a node reads whatever the one before it wrote, or the frame
        // when nothing did — which is the whole of what makes this a graph.
        // The LUT goes first because it is a grade of the footage as shot, and
        // grading after a blur would grade the blur.
        let luts = crate::lut::LutTables::new(&device);
        let effects: Vec<Box<dyn EffectNode>> = vec![
            // First of all: a reflection rebuilds the picture out of part of
            // itself, and every effect after it then treats the result as one
            // picture — a blur runs smoothly across the fold.
            Box::new(crate::reflect::ReflectPass::new(
                &device,
                &texture_layout,
                Self::FORMAT,
            )),
            Box::new(crate::lut::LutPass::new(
                &device,
                &texture_layout,
                Self::FORMAT,
                luts.clone(),
            )),
            Box::new(BlurPass::new(&device, &texture_layout, Self::FORMAT)),
            Box::new(crate::sharpen::SharpenPass::new(
                &device,
                &texture_layout,
                Self::FORMAT,
            )),
            // Last: the glitch breaks up the finished shot, and a sharpen run
            // after it would put halos on the split channels' edges.
            Box::new(crate::glitch::GlitchPass::new(
                &device,
                &texture_layout,
                Self::FORMAT,
            )),
        ];

        Ok(Self {
            device,
            queue,
            config,
            target,
            target_view,
            present_view,
            snapshot: None,
            pipelines,
            layer_bind_group,
            texture_layout,
            uniforms,
            uniform_capacity,
            pool: HashMap::new(),
            in_flight: Vec::new(),
            effects,
            targets: TargetPool::new(Self::FORMAT),
            grain_seed: 0,
            transparent: false,
            luts,
        })
    }

    pub fn config(&self) -> RenderConfig {
        self.config
    }

    /// The render attachment. sRGB-aware, so blending is linear.
    pub fn target_view(&self) -> &wgpu::TextureView {
        &self.target_view
    }

    /// The view to hand a UI toolkit for display.
    ///
    /// See [`Compositor::PRESENT_FORMAT`] for why this is not `target_view`.
    pub fn present_view(&self) -> &wgpu::TextureView {
        &self.present_view
    }

    pub fn target(&self) -> &wgpu::Texture {
        &self.target
    }

    /// Keep the frame just composited, so the next one can be shown beside it.
    ///
    /// The kept copy lives until the next call; a compositor that is never
    /// asked pays nothing for this.
    pub fn keep_snapshot(&mut self) -> Result<(), RenderError> {
        let size = self.target.size();
        let matches = self
            .snapshot
            .as_ref()
            .is_some_and(|(texture, _)| texture.size() == size);
        if !matches {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("composite snapshot"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: Self::FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[Self::PRESENT_FORMAT],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("composite snapshot view"),
                format: Some(Self::PRESENT_FORMAT),
                ..Default::default()
            });
            self.snapshot = Some((texture, view));
        }
        let Some((texture, _)) = &self.snapshot else {
            return Ok(());
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("snapshot copy"),
            });
        encoder.copy_texture_to_texture(self.target.as_image_copy(), texture.as_image_copy(), size);
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// The kept frame, for drawing beside the current one. `None` until
    /// [`Self::keep_snapshot`] has been called.
    pub fn snapshot_view(&self) -> Option<&wgpu::TextureView> {
        self.snapshot.as_ref().map(|(_, view)| view)
    }

    /// The kept frame's texture, for reading pixels back in a test.
    pub fn snapshot_texture(&self) -> Option<&wgpu::Texture> {
        self.snapshot.as_ref().map(|(texture, _)| texture)
    }

    /// The colour of one rendered pixel, in sRGB, or `None` off the frame.
    ///
    /// For picking a green screen off the shot. Keying starts by naming the
    /// screen's colour, and a colour wheel is the wrong instrument for that —
    /// the answer is already on screen, and no two green screens are the same
    /// green once a light has been near them.
    ///
    /// sRGB because that is what the target stores and what
    /// [`bettercut_timeline::ChromaKey`] holds; the shader does its own
    /// conversion to linear. Reading the composited frame rather than the
    /// clip's own texture, deliberately: the user points at what they can see,
    /// and what they can see has the grade and the transform already on it.
    ///
    /// Synchronous — it maps a 1×1 buffer and waits. That is a stall on the
    /// thread that calls it, which is fine for a click and would not be for a
    /// frame.
    pub fn read_pixel(&self, x: u32, y: u32) -> Option<[f32; 3]> {
        let size = self.config.resolution;
        if x >= size.width || y >= size.height {
            return None;
        }

        // `copy_texture_to_buffer` wants rows aligned to 256 bytes, and one
        // pixel is four — so the buffer is a row's worth of padding holding a
        // single texel.
        const ALIGNED_ROW: u32 = 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pixel read back"),
            size: u64::from(ALIGNED_ROW),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pixel read back"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ALIGNED_ROW),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let submitted = self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        // Bounded, and waiting on *this* copy rather than whatever happens to be
        // the latest submission.
        //
        // This runs on the interface thread — the eyedropper calls it from a
        // click — so an unbounded wait turns a stalled GPU into a frozen editor
        // with no way out but killing it. A pixel that cannot be read in time
        // is `None`, which is what this returns for any other reason it has
        // nothing to report, and the eyedropper already says so.
        //
        // It is not hypothetical: the pixel tests, which make hundreds of these
        // across parallel threads, hung the whole test run indefinitely on this
        // line before it had a limit.
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: Some(READ_PIXEL_TIMEOUT),
            })
            .ok()?;
        let mapped = slice.get_mapped_range().ok()?;
        let colour = [
            f32::from(mapped[0]) / 255.0,
            f32::from(mapped[1]) / 255.0,
            f32::from(mapped[2]) / 255.0,
        ];
        drop(mapped);
        buffer.unmap();
        Some(colour)
    }

    pub fn resolution(&self) -> Resolution {
        self.config.resolution
    }

    /// Resize the output. Returns whether the texture was actually replaced,
    /// so the caller knows to re-register it with egui.
    pub fn set_resolution(&mut self, resolution: Resolution) -> Result<bool, RenderError> {
        if resolution == self.config.resolution {
            return Ok(false);
        }
        let (target, view, present_view) = create_target(&self.device, resolution)?;
        self.target = target;
        self.target_view = view;
        self.present_view = present_view;
        self.config.resolution = resolution;
        Ok(true)
    }

    /// Composite `layers` bottom-to-top into the target.
    ///
    /// Track order is compositing order (§22), so the caller passes them in
    /// that order and this does not reorder them. The same as
    /// [`Self::composite_graded`] with no grades part-way up the stack.
    pub fn composite(
        &mut self,
        layers: &[Layer<'_>],
        master: MasterLook,
    ) -> Result<(), RenderError> {
        self.composite_graded(layers, &[], master)
    }

    /// Composite `layers` bottom-to-top, grading the picture part-way up.
    ///
    /// Each [`Grade`] applies to everything composited *beneath* it — the first
    /// `beneath` layers — and to nothing drawn above. That is an adjustment
    /// layer: a filter over a stretch of footage that leaves the titles on top
    /// of it alone.
    ///
    /// Built from the master adjustment's own machinery rather than beside it.
    /// The master already composites into a scratch texture and draws that
    /// through a grade; a grade part-way up does the same thing at an earlier
    /// point, and then carries on compositing the rest on top of the result.
    /// With no grades this is exactly the path it always was, and allocates
    /// nothing it did not before.
    pub fn composite_graded(
        &mut self,
        layers: &[Layer<'_>],
        grades: &[Grade],
        master: MasterLook,
    ) -> Result<(), RenderError> {
        self.recycle();

        // Grades that change nothing are dropped before they cost a pass, and
        // the rest are put in stack order. `beneath` past the top of the stack
        // means "all of it".
        let mut grades: Vec<Grade> = grades
            .iter()
            .copied()
            .filter(|grade| !grade.look.is_identity())
            .map(|grade| Grade {
                beneath: grade.beneath.min(layers.len()),
                ..grade
            })
            .collect();
        grades.sort_by_key(|grade| grade.beneath);

        // Slots: one per layer, then the master, then a plain copy of the
        // picture (only drawn when there are grades), then one per grade.
        let master_slot = layers.len() as u64;
        let copy_slot = master_slot + 1;
        let grade_slot = |index: usize| copy_slot + 1 + index as u64;
        self.ensure_uniform_capacity(layers.len() as u64 + 2 + grades.len() as u64)?;

        // Upload every frame first, so the render pass borrows nothing that is
        // still being written.
        let mut uploaded = Vec::with_capacity(layers.len());
        for (index, layer) in layers.iter().enumerate() {
            let texture = self.upload(layer.frame)?;
            let uniform = layer_uniform(
                layer.look.transform,
                LayerLook {
                    bars: 0.0,
                    opacity: layer.look.opacity,
                    color: layer.look.color,
                    key: layer.look.chroma_key,
                    mask: layer.look.mask,
                    crop: layer.look.crop,
                    // The clip's own vignette, around its own picture; grain is
                    // the film the whole frame is on.
                    vignette: layer.look.vignette,
                    border: layer.look.border,
                    shadow: layer.look.shadow,
                    corner_pin: layer.look.corner_pin,
                    grain: 0.0,
                    grain_seed: 0,
                },
                texture.width,
                texture.height,
                self.config.resolution,
            );
            self.queue
                .write_buffer(&self.uniforms, index as u64 * UNIFORM_STRIDE, &uniform);
            uploaded.push(texture);
        }

        // Every full-frame draw — the master, the copy, each grade — is built
        // exactly like a layer's, with a source the same shape as the frame,
        // which makes the fit term one and leaves the transform meaning what it
        // says about the output.
        let grain_seed = self.grain_seed;
        let full_frame =
            |transform: Transform, opacity: f32, color, vignette: f32, grain: f32, bars: f32| {
                layer_uniform(
                    transform,
                    LayerLook {
                        opacity,
                        color,
                        // A grade works on the assembled picture; a key belongs to
                        // the clip that was shot against a screen, a mask to the
                        // clip it was drawn on, and a crop to the shot it was set
                        // on — §36's canvas shape is how a sequence is re-framed.
                        key: None,
                        mask: None,
                        crop: bettercut_timeline::Crop::NONE,
                        vignette,
                        // A frame's own edges are the frame; nothing to round.
                        border: bettercut_timeline::Border::NONE,
                        shadow: bettercut_timeline::Shadow::NONE,
                        bars,
                        grain,
                        grain_seed,
                        // A grade covers the whole frame; only a clip is pinned.
                        corner_pin: bettercut_timeline::CornerPin::NONE,
                    },
                    self.config.resolution.width,
                    self.config.resolution.height,
                    self.config.resolution,
                )
            };
        let master_uniform = full_frame(
            master.transform,
            master.opacity,
            master.color,
            master.vignette,
            master.grain,
            // Last, over the finished picture and every title on it.
            master.bars,
        );
        // The picture as it was: no grade, no vignette.
        let copy_uniform = full_frame(
            Transform::default(),
            1.0,
            bettercut_timeline::ColorAdjust::IDENTITY,
            0.0,
            0.0,
            0.0,
        );
        let grade_uniforms: Vec<_> = grades
            .iter()
            .map(|grade| {
                let look = grade.look.clamped();
                full_frame(
                    Transform::default(),
                    look.strength,
                    look.color,
                    look.vignette,
                    look.grain,
                    0.0,
                )
            })
            .collect();
        self.queue.write_buffer(
            &self.uniforms,
            master_slot * UNIFORM_STRIDE,
            &master_uniform,
        );
        self.queue
            .write_buffer(&self.uniforms, copy_slot * UNIFORM_STRIDE, &copy_uniform);
        for (index, uniform) in grade_uniforms.iter().enumerate() {
            self.queue
                .write_buffer(&self.uniforms, grade_slot(index) * UNIFORM_STRIDE, uniform);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite encoder"),
            });

        // Each layer runs its effect chain on its own texture, before
        // compositing: an affected clip still scales and rotates like any
        // other, and the geometry stays in one place rather than being
        // duplicated into every effect shader (§20).
        //
        // `effected[i]` is what layer `i` ended up with, or `None` for the
        // layers no node touched — which is most of them, and they cost
        // nothing but the comparison.
        let effected = {
            let Self {
                effects,
                targets,
                device,
                queue,
                texture_layout,
                config,
                ..
            } = self;
            let mut ctx = EffectContext::new(
                device,
                queue,
                texture_layout,
                config.effect_quality,
                targets,
            );
            // Extra chains: the master runs the same nodes over the composited
            // image, and so does every grade. Counted here because a node sizes
            // its per-chain resources now and cannot grow them mid-frame — the
            // blur, short of a slot, skips the chain and says so only in a log.
            for node in effects.iter_mut() {
                node.begin_frame(ctx.device(), layers.len() + 1 + grades.len());
            }

            let mut effected = Vec::with_capacity(layers.len());
            for (index, layer) in layers.iter().enumerate() {
                let source = EffectInput {
                    bind_group: &uploaded[index].bind_group,
                    width: uploaded[index].width,
                    height: uploaded[index].height,
                };
                let params = EffectParams {
                    blur: layer.look.blur,
                    sharpen: layer.look.sharpen,
                    lut: layer.look.lut,
                    rgb_split: layer.look.rgb_split,
                    glitch: layer.look.glitch,
                    pixelate: layer.look.pixelate,
                    zoom_blur: layer.look.zoom_blur,
                    glow: layer.look.glow,
                    old_film: layer.look.old_film,
                    reflection: layer.look.reflection,
                    seed: grain_seed,
                };
                effected.push(run_chain(effects, &mut ctx, &mut encoder, params, source)?);
            }
            effected
        };

        // A sequence-wide adjustment applies to the *finished* picture, not to
        // each clip: blurring two stacked clips separately and then compositing
        // them is a different image from compositing them and blurring the
        // result, and the second is what "adjust the whole video" means.
        //
        // So when there is one — or any grade part-way up — the layers
        // composite into a scratch texture and a final draw puts that through
        // the master's own transform, opacity and colour. When there is not,
        // the normal case, nothing changes and nothing is allocated.
        let mut canvas = if master.is_identity() && grades.is_empty() {
            None
        } else {
            Some(self.scratch())
        };

        // Textures finished with this frame, held until after submission: the
        // GPU has not run any of it yet.
        let mut spent: Vec<EffectTexture> = Vec::new();

        // sRGB from the model into the linear values wgpu clears with — the
        // target is an sRGB texture, so it encodes on write and a value handed
        // over unconverted would come out visibly pale.
        let background = if self.transparent {
            wgpu::Color::TRANSPARENT
        } else {
            clear_colour(master.background)
        };

        // The stack in runs: the layers beneath the first grade, then the
        // layers between each grade and the next, then the rest.
        let boundaries: Vec<usize> = grades
            .iter()
            .map(|grade| grade.beneath)
            .chain(std::iter::once(layers.len()))
            .collect();
        let mut from = 0;
        for (run, &to) in boundaries.iter().enumerate() {
            // The first run clears to §22's background; later ones draw on top
            // of the graded picture beneath them. A run with no layers still
            // gets its pass when it is the first, so a grade at the very bottom
            // has a background to grade.
            if run == 0 || to > from {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("composite pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: canvas
                            .as_ref()
                            .map_or(&self.target_view, EffectTexture::view),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if run == 0 {
                                // What a viewer sees in the letterbox and in
                                // the gaps. Opaque whatever the colour — §21a's
                                // output has no alpha to speak of.
                                wgpu::LoadOp::Clear(background)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                for index in from..to {
                    // Per layer, not per pass: the blend state lives in the
                    // pipeline, so an overlay set to screen needs a different
                    // one from the shot beneath it.
                    let mode = layers[index].look.blend;
                    let slot = bettercut_timeline::BlendMode::ALL
                        .iter()
                        .position(|candidate| *candidate == mode)
                        .unwrap_or(0);
                    pass.set_pipeline(&self.pipelines[slot]);
                    let offset = (index as u64 * UNIFORM_STRIDE) as u32;
                    // A layer whose chain produced something composites from
                    // that instead of the frame that was uploaded.
                    let source = effected[index]
                        .as_ref()
                        .map_or(&uploaded[index].bind_group, EffectTexture::bind_group);
                    pass.set_bind_group(0, &self.layer_bind_group, &[offset]);
                    pass.set_bind_group(1, source, &[]);
                    pass.draw(0..6, 0..1);
                }
            }
            from = to;

            // Then the grade that sits at this point, if one does.
            let Some(grade) = grades.get(run) else {
                continue;
            };
            let Some(beneath) = canvas.take() else {
                continue; // unreachable: grades always composite into a canvas
            };

            let graded = {
                let Self {
                    effects,
                    targets,
                    device,
                    queue,
                    texture_layout,
                    config,
                    ..
                } = self;
                let mut ctx = EffectContext::new(
                    device,
                    queue,
                    texture_layout,
                    config.effect_quality,
                    targets,
                );
                let params = EffectParams {
                    old_film: 0.0,
                    glow: 0.0,
                    blur: grade.look.clamped().blur,
                    // As the master: a grade is not a place to sharpen a shot.
                    sharpen: 0.0,
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    pixelate: 0.0,
                    zoom_blur: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    seed: 0,
                };
                run_chain(effects, &mut ctx, &mut encoder, params, beneath.as_input())?
            };

            let next = self.scratch();
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("grade pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: next.view(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(background),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipelines[0]);

                // The picture as it was, and the graded picture over it at the
                // grade's strength. Two draws rather than a mix in the shader:
                // the picture beneath is opaque, so alpha-over at `strength` is
                // exactly the blend of the two, and it needs no new pipeline.
                pass.set_bind_group(
                    0,
                    &self.layer_bind_group,
                    &[(copy_slot * UNIFORM_STRIDE) as u32],
                );
                pass.set_bind_group(1, beneath.bind_group(), &[]);
                pass.draw(0..6, 0..1);

                pass.set_bind_group(
                    0,
                    &self.layer_bind_group,
                    &[(grade_slot(run) * UNIFORM_STRIDE) as u32],
                );
                pass.set_bind_group(1, graded.as_ref().unwrap_or(&beneath).bind_group(), &[]);
                pass.draw(0..6, 0..1);
            }
            spent.push(beneath);
            spent.extend(graded);
            canvas = Some(next);
        }

        // The master's own effect chain, then the draw that puts the adjusted
        // picture on the target.
        if let Some(scratch) = &canvas {
            let source = {
                let Self {
                    effects,
                    targets,
                    device,
                    queue,
                    texture_layout,
                    config,
                    ..
                } = self;
                let mut ctx = EffectContext::new(
                    device,
                    queue,
                    texture_layout,
                    config.effect_quality,
                    targets,
                );
                // The whole picture is graded, not sharpened: sharpening is a
                // decision about a shot's own detail.
                let params = EffectParams {
                    old_film: 0.0,
                    glow: 0.0,
                    blur: master.blur,
                    sharpen: 0.0,
                    lut: None,
                    rgb_split: 0.0,
                    glitch: 0.0,
                    pixelate: 0.0,
                    zoom_blur: 0.0,
                    reflection: bettercut_timeline::Reflection::None,
                    seed: 0,
                };
                run_chain(effects, &mut ctx, &mut encoder, params, scratch.as_input())?
            };

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("master pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // The same background: a master transform that
                            // shrinks the whole frame shows it around the edge.
                            load: wgpu::LoadOp::Clear(background),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                // The master draw puts the graded picture onto the target, so
                // it is alpha-over whatever the clips beneath chose.
                pass.set_pipeline(&self.pipelines[0]);
                pass.set_bind_group(
                    0,
                    &self.layer_bind_group,
                    &[(master_slot * UNIFORM_STRIDE) as u32],
                );
                pass.set_bind_group(1, source.as_ref().map_or(scratch, |t| t).bind_group(), &[]);
                pass.draw(0..6, 0..1);
            }
            spent.extend(source);
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Hold the textures until the next composite: the GPU may still be
        // reading them, and returning them to the pool now would let the next
        // frame overwrite pixels that are still being sampled.
        self.in_flight = uploaded;
        for texture in effected.into_iter().flatten() {
            self.targets.retire(texture);
        }
        for texture in canvas.into_iter().chain(spent) {
            self.targets.retire(texture);
        }
        Ok(())
    }

    /// A full-frame scratch texture from the shared pool.
    fn scratch(&mut self) -> EffectTexture {
        let Self {
            targets,
            device,
            queue,
            texture_layout,
            config,
            ..
        } = self;
        let mut ctx = EffectContext::new(
            device,
            queue,
            texture_layout,
            config.effect_quality,
            targets,
        );
        ctx.acquire(config.resolution.width, config.resolution.height)
    }

    /// Intermediate textures the effect graph is holding, for §73's memory
    /// diagnostics — and for asserting that they are reused rather than
    /// reallocated every frame.
    /// Which frame the next composite is, for film grain: grain moves every
    /// frame, and the same frame must always get the same grain, so the caller
    /// says which frame this is (`bettercut_playback::grain_seed`, asked by the
    /// preview and the export alike). Zero until set.
    pub fn set_grain_seed(&mut self, seed: u32) {
        self.grain_seed = seed;
    }

    /// Leave the background see-through rather than filling it with the
    /// sequence's background colour. The picture read back is then
    /// premultiplied by its alpha.
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }

    /// Give the compositor a colour lookup table to draw clips with. Loaded
    /// once per table, by the preview and the export alike — the renderer
    /// reads no files. Replaces a table already loaded under `id`.
    pub fn load_lut(&mut self, id: bettercut_foundation::LutId, lut: &bettercut_timeline::CubeLut) {
        self.luts.load(&self.device, &self.queue, id, lut);
    }

    /// Whether a table has been loaded under `id`.
    pub fn has_lut(&self, id: bettercut_foundation::LutId) -> bool {
        self.luts.contains(id)
    }

    pub fn intermediate_count(&self) -> usize {
        self.targets.len()
    }

    /// Return last frame's textures to the pool.
    fn recycle(&mut self) {
        for texture in self.in_flight.drain(..) {
            self.pool
                .entry((texture.width, texture.height))
                .or_default()
                .push(texture);
        }
        self.targets.recycle();
    }

    fn ensure_uniform_capacity(&mut self, layers: u64) -> Result<(), RenderError> {
        if layers <= self.uniform_capacity {
            return Ok(());
        }
        // Grow generously; a project that reaches 20 tracks will reach 40.
        let capacity = layers.next_power_of_two().max(INITIAL_LAYER_CAPACITY);
        self.uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer uniforms"),
            size: capacity * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_capacity = capacity;

        // The bind group referenced the old buffer.
        // All four pipelines share a layout; the first will do.
        let layout = self.pipelines[0].get_bind_group_layout(0);
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("composite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        self.layer_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    }),
                },
            ],
        });
        Ok(())
    }

    /// Get a texture holding this frame's pixels.
    ///
    /// §5's fallback path: the frame is in system RAM, so it has to be
    /// uploaded. A GPU-resident frame would skip this entirely — that is the
    /// hardware-decode interop work ADR 005 still lists as open.
    fn upload(&mut self, frame: &VideoFrame) -> Result<SourceTexture, RenderError> {
        let bettercut_media::FrameStorage::System { data, stride } = &frame.storage else {
            return Err(RenderError::UnsupportedFrameStorage);
        };

        let expected = (*stride as usize) * frame.height as usize;
        if data.len() < expected {
            return Err(RenderError::FrameTooSmall {
                got: data.len(),
                expected,
            });
        }

        let texture = self.take_from_pool(frame.width, frame.height);

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(*stride),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );

        Ok(texture)
    }

    fn take_from_pool(&mut self, width: u32, height: u32) -> SourceTexture {
        if let Some(pooled) = self
            .pool
            .get_mut(&(width, height))
            .and_then(std::vec::Vec::pop)
        {
            return pooled;
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("source frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("source texture bind group"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        SourceTexture {
            texture,
            bind_group,
            width,
            height,
        }
    }
}

/// Bytes actually used by the uniform struct: `mat3x3` occupies three
/// `vec4`-aligned columns in WGSL, then opacity and the three colour values
/// fill its padding, then the chroma key's `vec3` starts at 64 with tolerance
/// in its tail. A `vec3` aligns to 16, so the struct rounds to 96.
/// 128 through the white balance, then §22's crop as two `vec2`s at 128 and
/// 136 — which is why this is 144 and not 136: the struct aligns to 16.
/// Then vignette and grain to 160, the border and shadow to 192, and the
/// cinematic bars at 192, rounding to 208.
const UNIFORM_SIZE: u64 = 240;

fn create_target(
    device: &wgpu::Device,
    resolution: Resolution,
) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::TextureView), RenderError> {
    if resolution.width == 0 || resolution.height == 0 {
        return Err(RenderError::InvalidResolution {
            width: resolution.width,
            height: resolution.height,
        });
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("composite target"),
        size: wgpu::Extent3d {
            width: resolution.width,
            height: resolution.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: Compositor::FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            // COPY_SRC so §51.1's golden-frame tests can read the result back.
            | wgpu::TextureUsages::COPY_SRC,
        // Permits the second, non-sRGB view below. Reinterpreting the same
        // bytes costs nothing at run time.
        view_formats: &[Compositor::PRESENT_FORMAT],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let present_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("composite target (raw bytes, for egui)"),
        format: Some(Compositor::PRESENT_FORMAT),
        ..Default::default()
    });
    Ok((texture, view, present_view))
}

/// Build one layer's uniform block.
///
/// Maps the unit quad onto the output, letterboxing the source so it fills the
/// frame without distorting — a 4:3 clip in a 16:9 sequence gets pillarbox
/// bars, not stretched faces.
/// Re-exported so this crate's callers keep one obvious place to find it.
/// The implementation lives in the timeline crate, because the preview
/// handles and §26's titles both need it and neither can depend on the
/// renderer — see [`bettercut_timeline::fit_scale`].
pub use bettercut_timeline::fit_scale;

/// The blend state for one of §22's modes.
///
/// Every one assumes the shader's **premultiplied** output: the colour has
/// already been scaled by its alpha, so `src` below means "colour × alpha".
/// That is what lets a half-transparent overlay screen at half strength rather
/// than at full.
fn blend_state(mode: bettercut_timeline::BlendMode) -> wgpu::BlendState {
    use bettercut_timeline::BlendMode;

    let colour = match mode {
        // `src + dst*(1-a)`: alpha-over, written for premultiplied source.
        BlendMode::Normal => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        // `src*(1-dst) + dst` = `src + dst - src*dst`. Never darkens.
        BlendMode::Screen => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDst,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        // `src*dst + dst*(1-a)`: the product where the layer is opaque, and
        // what was already there where it is not. Never lightens.
        BlendMode::Multiply => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Dst,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        // `src + dst`, straight.
        BlendMode::Add => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };

    wgpu::BlendState {
        color: colour,
        // The standard over rule, on every mode.
        //
        // It cannot currently matter: the composite pass clears its target to
        // opaque black (§21a's output is opaque), and with a destination alpha
        // of 1 all four colour rules happen to leave it at 1 as well. It is
        // written out rather than left to the colour rule because that
        // coincidence is a property of the clear, not of the modes — a target
        // ever cleared transparent would need exactly this.
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// One channel of sRGB to linear light.
///
/// The key colour is picked and stored in sRGB, like every other colour in the
/// project, but the shader compares it against samples from an sRGB-aware
/// texture — which are linear by the time they arrive (§21a.1). Converting here
/// rather than there keeps it to once per layer instead of once per pixel.
/// A background colour as wgpu wants it to clear with.
fn clear_colour(srgb: [f32; 3]) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(srgb_to_linear(srgb[0])),
        g: f64::from(srgb_to_linear(srgb[1])),
        b: f64::from(srgb_to_linear(srgb[2])),
        a: 1.0,
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// What a layer looks like, as the uniform packer needs it.
///
/// Gathered into one value rather than passed as five: they are one idea — how
/// this layer is drawn — and a packer with eight positional arguments is one
/// where a caller eventually swaps two of them without the compiler noticing.
#[derive(Debug, Clone, Copy, Default)]
struct LayerLook {
    opacity: f32,
    color: bettercut_timeline::ColorAdjust,
    key: Option<bettercut_timeline::ChromaKey>,
    mask: Option<bettercut_timeline::Mask>,
    /// §22's first stage. Here rather than a parameter of its own for the
    /// reason above: it is part of how this layer is drawn.
    crop: bettercut_timeline::Crop,
    /// Darkened edges, for a full-frame draw only. Zero for every clip.
    vignette: f32,
    /// Rounded corners and a border on a clip's own picture.
    border: bettercut_timeline::Border,
    /// Set on a shadow's own layer: drawn as the soft dark shape, not the
    /// picture.
    shadow: bettercut_timeline::Shadow,
    /// Cinematic bars, as the shape they cut to; the master draw only.
    bars: f32,
    /// Film grain, full-frame draws only, and the frame it is drawn for.
    grain: f32,
    grain_seed: u32,
    /// §45's corner pin: each corner moved, in output-frame units.
    corner_pin: bettercut_timeline::CornerPin,
}

fn layer_uniform(
    transform: Transform,
    look: LayerLook,
    source_width: u32,
    source_height: u32,
    output: Resolution,
) -> [u8; UNIFORM_SIZE as usize] {
    let LayerLook {
        opacity,
        color,
        key,
        mask,
        crop,
        vignette,
        border,
        shadow,
        bars,
        grain,
        grain_seed,
        corner_pin,
    } = look;
    // §22 crops *before* transforming, and that ordering is visible right here:
    // the aspect fitted to the frame is the **cropped** picture's, not the
    // source's. Fitting the source's instead would be a rectangular mask — the
    // shot would stay its original shape with part of it missing, rather than
    // the remainder becoming a new picture of a new shape.
    let crop = crop.clamped();
    let (kept_x, kept_y) = crop.remaining();
    let (fit_x, fit_y) = fit_scale(
        crop.applied_to(source_width.max(1) as f32 / source_height.max(1) as f32),
        output.width.max(1) as f32 / output.height.max(1) as f32,
    );

    // A mirror is the scale run backwards *with the anchor mirrored to match*.
    //
    // Negating the scale alone would turn the quad about its anchor, and an
    // off-centre anchor would then throw the picture across the frame. Taking
    // the anchor to `1 - anchor` on the same axis cancels that exactly: the two
    // ends of the quad swap and every other point of it stays where it was, so
    // a mirrored clip covers the identical region of the frame. The one thing
    // that changes is which part of the source lands there, which is all a
    // mirror is.
    //
    // Before rotation, because the scale is: mirroring a turned clip reverses
    // the picture inside its turned frame rather than turning it the other way.
    let flip_x = if transform.flip_h { -1.0 } else { 1.0 };
    let flip_y = if transform.flip_v { -1.0 } else { 1.0 };
    let scale_x = fit_x * transform.scale.x * flip_x;
    let scale_y = fit_y * transform.scale.y * flip_y;

    // The unit quad runs 0..1; map it to -1..1 clip space, apply scale and
    // rotation about the anchor, then translate. Clip-space y is up, while the
    // transform's y is down (screen convention), hence the negation.
    let radians = transform.rotation_degrees.to_radians();
    let (sin, cos) = radians.sin_cos();

    let anchor_x = if transform.flip_h {
        1.0 - transform.anchor.x
    } else {
        transform.anchor.x
    };
    let anchor_y = if transform.flip_v {
        1.0 - transform.anchor.y
    } else {
        transform.anchor.y
    };

    // The rotation has to happen in *pixels*, not in clip space. Clip space
    // runs -1..1 on both axes whatever the frame's shape, so on a 16:9 frame a
    // clip-space unit is 1.8× wider than it is tall, and rotating there is a
    // shear. The off-diagonal terms carry the frame's aspect to undo that.
    //
    // Derived in y-down screen pixels, where "clockwise" is the ordinary
    // matrix, then converted to y-up clip space. This used to have `b` with
    // the wrong sign and neither aspect factor: the two axes then turned in
    // opposite directions, which is a shear rather than a rotation — a clip
    // lost area as it turned (half of it at 30°) and vanished entirely at
    // 45°, where the matrix's determinant, -cos 2θ, is zero. The only test
    // checked one point at 90°, which depends on `c` and `d` alone.
    let aspect = output.width.max(1) as f32 / output.height.max(1) as f32;

    // Column-major mat3x3, each column padded to 16 bytes for WGSL layout.
    let a = 2.0 * scale_x * cos;
    let b = -2.0 * scale_x * aspect * sin;
    let c = -2.0 * scale_y * sin / aspect;
    let d = -2.0 * scale_y * cos;

    // Translation so the anchor lands at the requested position.
    let tx = transform.position.x * 2.0 - (a * anchor_x + c * anchor_y);
    let ty = -transform.position.y * 2.0 - (b * anchor_x + d * anchor_y);

    let columns: [[f32; 4]; 3] = [[a, b, 0.0, 0.0], [c, d, 0.0, 0.0], [tx, ty, 1.0, 0.0]];

    let mut bytes = [0_u8; UNIFORM_SIZE as usize];
    let mut offset = 0;
    for column in columns {
        for value in column {
            bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
            offset += 4;
        }
    }
    bytes[48..52].copy_from_slice(&opacity.clamp(0.0, 1.0).to_ne_bytes());
    // Offsets 52/56/60 are the padding `opacity` leaves; the WGSL struct
    // declares them as the three colour values, so the total stays 64 bytes.
    //
    // Clamped here rather than trusted: a negative brightness inverts the
    // picture and a huge contrast produces values the sRGB encode turns into
    // NaN, and neither is something a project file should be able to cause.
    bytes[52..56].copy_from_slice(&color.brightness.clamp(0.0, 4.0).to_ne_bytes());
    bytes[56..60].copy_from_slice(&color.contrast.clamp(0.0, 4.0).to_ne_bytes());
    bytes[60..64].copy_from_slice(&color.saturation.clamp(0.0, 4.0).to_ne_bytes());
    // Vibrance rides in the tail padding at 196; see the WGSL struct.
    bytes[196..200].copy_from_slice(&color.vibrance.clamp(-1.0, 1.0).to_ne_bytes());
    // §45's corner pin, four `vec2`s. A `vec2` aligns to 8, so the first one
    // lands at 200 — right after `vibrance` at 196 — and the struct then runs
    // to 232 and rounds to 240. Clip space runs -1..1 across the frame where
    // the pin's offsets are in frames, so each is doubled; and its y runs the
    // other way from the picture's.
    let pin = corner_pin.clamped();
    for (index, corner) in pin.offsets.iter().enumerate() {
        let at = 200 + index * 8;
        bytes[at..at + 4].copy_from_slice(&(corner[0] * 2.0).to_ne_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&(-corner[1] * 2.0).to_ne_bytes());
    }

    // The chroma key, or zeroes — which the shader reads as "no key", because
    // a tolerance and softness of zero remove nothing.
    if let Some(key) = key.map(bettercut_timeline::ChromaKey::clamped) {
        for (index, channel) in key.color.into_iter().enumerate() {
            let at = 64 + index * 4;
            bytes[at..at + 4].copy_from_slice(&srgb_to_linear(channel).to_ne_bytes());
        }
        bytes[76..80].copy_from_slice(&key.tolerance.to_ne_bytes());
        bytes[80..84].copy_from_slice(&key.softness.to_ne_bytes());
        bytes[84..88].copy_from_slice(&key.spill.to_ne_bytes());
    }

    // The mask, or a shape of zero — which the shader reads as "no mask" and
    // skips entirely.
    if let Some(mask) = mask.map(bettercut_timeline::Mask::clamped) {
        let shape: u32 = match mask.shape {
            bettercut_timeline::MaskShape::Linear => 1,
            bettercut_timeline::MaskShape::Rectangle => 2,
            bettercut_timeline::MaskShape::Ellipse => 3,
            bettercut_timeline::MaskShape::Star => 4,
            bettercut_timeline::MaskShape::Heart => 5,
        };
        bytes[88..92].copy_from_slice(&shape.to_ne_bytes());
        bytes[92..96].copy_from_slice(&mask.feather.to_ne_bytes());
        bytes[96..100].copy_from_slice(&mask.center[0].to_ne_bytes());
        bytes[100..104].copy_from_slice(&mask.center[1].to_ne_bytes());
        bytes[104..108].copy_from_slice(&mask.size[0].to_ne_bytes());
        bytes[108..112].copy_from_slice(&mask.size[1].to_ne_bytes());
        bytes[112..116].copy_from_slice(&mask.rotation_degrees.to_ne_bytes());
        bytes[116..120].copy_from_slice(&f32::from(mask.invert).to_ne_bytes());
    }

    // The white balance, in the eight bytes the struct's alignment already
    // reserved. Outside the mask's `if`, because it belongs to every layer.
    bytes[120..124].copy_from_slice(&color.temperature.clamp(-1.0, 1.0).to_ne_bytes());
    bytes[124..128].copy_from_slice(&color.tint.clamp(-1.0, 1.0).to_ne_bytes());

    // The crop as the window to sample through. Clamped above, so this cannot
    // be a zero-sized or inverted rectangle whatever the project file said.
    bytes[128..132].copy_from_slice(&crop.left.to_ne_bytes());
    bytes[132..136].copy_from_slice(&crop.top.to_ne_bytes());
    bytes[136..140].copy_from_slice(&kept_x.to_ne_bytes());
    bytes[140..144].copy_from_slice(&kept_y.to_ne_bytes());

    // Clamped here, as the colour is: the shader trusts what it is given.
    let vignette = if vignette.is_finite() {
        vignette.clamp(0.0, bettercut_timeline::MAX_VIGNETTE)
    } else {
        0.0
    };
    bytes[144..148].copy_from_slice(&vignette.to_ne_bytes());

    let grain = if grain.is_finite() {
        grain.clamp(0.0, bettercut_timeline::MAX_GRAIN)
    } else {
        0.0
    };
    bytes[148..152].copy_from_slice(&grain.to_ne_bytes());
    // Kept below 2^24, where every whole number is exact in an f32 — past
    // that, neighbouring frames would round to the same grain.
    let seed = (grain_seed % (1 << 24)) as f32;
    bytes[152..156].copy_from_slice(&seed.to_ne_bytes());
    // One grain is one pixel at 1080 lines, and grows with the frame, so a 4K
    // export has the same texture as the 1080p preview rather than a finer
    // one nobody can see.
    let cell = (output.height as f32 / 1080.0).max(1.0);
    bytes[156..160].copy_from_slice(&cell.to_ne_bytes());

    // Rounded corners and a border, measured against the picture's shorter
    // side — which needs the picture's shape as it is drawn: the cropped
    // source, stretched by any uneven scale.
    let border = border.clamped();
    bytes[160..164].copy_from_slice(&border.radius.to_ne_bytes());
    bytes[164..168].copy_from_slice(&border.width.to_ne_bytes());
    // A shadow's layer is drawn in its colour and has no border of its own,
    // so the border's colour slots carry the shadow's.
    let shadow = shadow.clamped();
    let colour = if shadow.is_visible() {
        shadow.colour
    } else {
        border.colour
    };
    for (index, channel) in colour.into_iter().enumerate() {
        let at = 168 + index * 4;
        let linear = srgb_to_linear(f32::from(channel) / 255.0);
        bytes[at..at + 4].copy_from_slice(&linear.to_ne_bytes());
    }
    let stretch = (transform.scale.x / transform.scale.y).abs();
    let picture_aspect = crop.applied_to(source_width.max(1) as f32 / source_height.max(1) as f32)
        * if stretch.is_finite() && stretch > 0.0 {
            stretch
        } else {
            1.0
        };
    bytes[180..184].copy_from_slice(&picture_aspect.to_ne_bytes());
    if shadow.is_visible() {
        bytes[184..188].copy_from_slice(&shadow.softness.to_ne_bytes());
        bytes[188..192].copy_from_slice(&1.0_f32.to_ne_bytes());
    }
    // The bars as the share of the height each covers, worked out against the
    // output's shape so the shader only compares.
    let bar = bettercut_timeline::bar_height(
        output.width.max(1) as f32 / output.height.max(1) as f32,
        bars,
    );
    bytes[192..196].copy_from_slice(&bar.to_ne_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_timeline::{ColorAdjust, Vec2};

    /// The two target views must differ in exactly one way: sRGB-awareness.
    ///
    /// Collapsing them back into one view is an easy-looking simplification
    /// and it silently ruins the picture. egui decodes gamma itself, so an
    /// sRGB-aware view gets linearized twice; the error is small at the
    /// extremes — saturated colour bars barely move — and brutal in the
    /// midtones, where 124 came out as 51. Anything shot by a camera is mostly
    /// midtones, so it reads as "everything is too dark" rather than as a
    /// colour bug.
    #[test]
    fn the_present_view_format_is_not_srgb_aware() {
        assert_eq!(Compositor::FORMAT, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(
            Compositor::PRESENT_FORMAT,
            wgpu::TextureFormat::Rgba8Unorm,
            "egui expects raw sRGB bytes; an sRGB-aware view is decoded twice"
        );
        assert_ne!(
            Compositor::FORMAT,
            Compositor::PRESENT_FORMAT,
            "the presentation view must not be the render target's own format"
        );
        // Same bytes, same size — only the interpretation differs.
        assert_eq!(
            Compositor::FORMAT.block_copy_size(None),
            Compositor::PRESENT_FORMAT.block_copy_size(None)
        );
    }

    fn read_matrix(bytes: &[u8; UNIFORM_SIZE as usize]) -> [f32; 9] {
        let mut out = [0.0; 9];
        // Three columns of vec4, of which the first three floats matter.
        for column in 0..3 {
            for row in 0..3 {
                let at = column * 16 + row * 4;
                out[column * 3 + row] =
                    f32::from_ne_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            }
        }
        out
    }

    fn apply(matrix: &[f32; 9], point: (f32, f32)) -> (f32, f32) {
        let (x, y) = point;
        (
            matrix[0] * x + matrix[3] * y + matrix[6],
            matrix[1] * x + matrix[4] * y + matrix[7],
        )
    }

    /// §45's corner pin rides in the tail the struct gained at 208: four
    /// `vec2`s, in clip-space units, with the picture's y turned the other way
    /// up.
    #[test]
    fn a_corner_pin_is_written_into_the_tail() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
                corner_pin: bettercut_timeline::CornerPin::NONE.with_corner(0, [-0.25, -0.25]),
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(200), -0.5, "the pinned corner's x is not at 200");
        assert_eq!(read(204), 0.5, "its y is not at 204, or is upside down");
        for at in (208..232).step_by(4) {
            assert_eq!(read(at), 0.0, "an unpinned corner at {at} was written");
        }
    }

    #[test]
    fn an_identity_transform_fills_a_matching_frame() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let matrix = read_matrix(&bytes);

        // The unit quad's corners must land on the clip-space corners.
        let top_left = apply(&matrix, (0.0, 0.0));
        let bottom_right = apply(&matrix, (1.0, 1.0));

        assert!(
            (top_left.0 + 1.0).abs() < 1e-5,
            "left edge at {}",
            top_left.0
        );
        assert!(
            (top_left.1 - 1.0).abs() < 1e-5,
            "top edge at {}",
            top_left.1
        );
        assert!((bottom_right.0 - 1.0).abs() < 1e-5);
        assert!((bottom_right.1 + 1.0).abs() < 1e-5);
    }

    /// A 4:3 source in a 16:9 frame must be pillarboxed, never stretched.
    #[test]
    fn a_narrower_source_is_pillarboxed() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1440,
            1080,
            Resolution::HD_1080,
        );
        let matrix = read_matrix(&bytes);

        let left = apply(&matrix, (0.0, 0.5)).0;
        let right = apply(&matrix, (1.0, 0.5)).0;
        let top = apply(&matrix, (0.5, 0.0)).1;
        let bottom = apply(&matrix, (0.5, 1.0)).1;

        assert!(left > -1.0 + 1e-3, "no pillarbox on the left: {left}");
        assert!(right < 1.0 - 1e-3, "no pillarbox on the right: {right}");
        assert!((top - 1.0).abs() < 1e-5, "height should still fill");
        assert!((bottom + 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_wider_source_is_letterboxed() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1920,
            800,
            Resolution::HD_1080,
        );
        let matrix = read_matrix(&bytes);

        let top = apply(&matrix, (0.5, 0.0)).1;
        let bottom = apply(&matrix, (0.5, 1.0)).1;
        assert!(top < 1.0 - 1e-3, "no letterbox at the top: {top}");
        assert!(bottom > -1.0 + 1e-3);
    }

    #[test]
    fn scaling_shrinks_around_the_anchor() {
        let transform = Transform {
            scale: Vec2::new(0.5, 0.5),
            ..Default::default()
        };
        let bytes = layer_uniform(
            transform,
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let matrix = read_matrix(&bytes);

        // The centre stays put; the corners move halfway in.
        let centre = apply(&matrix, (0.5, 0.5));
        assert!(centre.0.abs() < 1e-5, "centre drifted to {}", centre.0);
        assert!(centre.1.abs() < 1e-5);

        let corner = apply(&matrix, (0.0, 0.0));
        assert!((corner.0 + 0.5).abs() < 1e-5, "corner at {}", corner.0);
    }

    #[test]
    fn position_moves_the_layer() {
        let transform = Transform {
            position: Vec2::new(0.25, 0.0),
            ..Default::default()
        };
        let bytes = layer_uniform(
            transform,
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let matrix = read_matrix(&bytes);

        let centre = apply(&matrix, (0.5, 0.5));
        assert!(
            (centre.0 - 0.5).abs() < 1e-5,
            "expected the centre at 0.5, got {}",
            centre.0
        );
    }

    /// The colour values ride in the padding `opacity` leaves behind, at
    /// offsets 52/56/60. If the WGSL struct and this writer ever disagree, the
    /// shader reads whatever happens to be in those bytes — a silent wrong
    /// picture rather than a validation error, because the size is unchanged.
    #[test]
    fn colour_is_packed_into_the_padding_after_opacity() {
        let color = ColorAdjust {
            brightness: 1.25,
            contrast: 0.75,
            saturation: 0.5,
            temperature: -0.6,
            tint: 0.4,
            vibrance: 0.3,
        };
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 0.5,
                color,
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );

        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(48), 0.5, "opacity moved");
        assert_eq!(read(52), 1.25, "brightness is not at offset 52");
        assert_eq!(read(56), 0.75, "contrast is not at offset 56");
        assert_eq!(read(60), 0.5, "saturation is not at offset 60");
        // The white balance rides in the tail the struct's 16-byte alignment
        // already reserved, which is why adding it cost no bytes at all.
        assert_eq!(read(120), -0.6, "temperature is not at offset 120");
        assert_eq!(read(124), 0.4, "tint is not at offset 124");
        // §22's crop, which is what took the struct past 128: two `vec2`s at
        // 128 and 136, and a 16-aligned struct then rounds to 144.
        assert_eq!(read(128), 0.0, "the crop origin is not at offset 128");
        assert_eq!(read(136), 1.0, "the crop size is not at offset 136");
        // The vignette, after the crop: offset 144, and the struct rounds to
        // 160. Zero here because this is a clip's look, and a clip has none.
        assert_eq!(read(144), 0.0, "the vignette is not at offset 144");
        // Vibrance, in the padding the cinematic bars left behind at 196.
        assert_eq!(read(196), 0.3, "vibrance is not at offset 196");
        assert_eq!(bytes.len(), UNIFORM_SIZE as usize);
        assert_eq!(
            UNIFORM_SIZE, 240,
            "the WGSL struct is declared as 240 bytes"
        );
    }

    /// Grain rides in the last of the tail: amount at 148, the frame at 152,
    /// the size of one grain at 156 — and the struct is still 160 bytes.
    #[test]
    fn grain_is_packed_into_the_tail() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.6,
                grain_seed: 1234,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::IDENTITY,
                key: None,
                mask: None,
            },
            3840,
            2160,
            Resolution::new(3840, 2160),
        );
        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(148), 0.6, "the grain is not at offset 148");
        assert_eq!(read(152), 1234.0, "the frame is not at offset 152");
        assert_eq!(
            read(156),
            2.0,
            "one grain at 2160 lines should be two pixels"
        );
        assert_eq!(bytes.len(), UNIFORM_SIZE as usize);
        assert_eq!(
            UNIFORM_SIZE, 240,
            "the WGSL struct is declared as 240 bytes"
        );
    }

    /// The border's place at the end of the struct, and the picture's drawn
    /// shape it is measured against: a 16:9 source cropped to a square and
    /// stretched twice as wide is 2:1.
    #[test]
    fn the_border_is_packed_after_the_grain() {
        let bytes = layer_uniform(
            Transform {
                scale: Vec2::new(1.0, 0.5),
                ..Transform::default()
            },
            LayerLook {
                opacity: 1.0,
                crop: bettercut_timeline::crop_to_aspect(16.0 / 9.0, 1.0),
                border: bettercut_timeline::Border {
                    radius: 0.5,
                    width: 0.1,
                    colour: [255, 0, 255],
                },
                ..LayerLook::default()
            },
            1920,
            1080,
            Resolution::new(1920, 1080),
        );
        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(160), 0.5, "the radius is not at offset 160");
        assert!((read(164) - 0.1).abs() < 1e-6, "the width is not at 164");
        assert_eq!((read(168), read(172), read(176)), (1.0, 0.0, 1.0));
        assert!((read(180) - 2.0).abs() < 0.01, "shape {}", read(180));
    }

    /// The chroma key's own place in that struct.
    ///
    /// Offsets are the whole contract between this file and the shader: WGSL
    /// aligns a `vec3` to 16, so the key colour starts at 64 and `tolerance`
    /// rides in its tail padding at 76. Get any of it wrong and the shader
    /// reads a transform column as a tolerance — which is not a crash, just a
    /// key that behaves madly.
    #[test]
    fn the_chroma_key_is_packed_where_the_shader_looks_for_it() {
        let key = bettercut_timeline::ChromaKey {
            // Already linear, so the conversion below is the identity and the
            // offsets are what is being tested rather than the maths.
            color: [0.0, 1.0, 0.0],
            tolerance: 0.25,
            softness: 0.125,
            spill: 0.5,
        };
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: Some(key),
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );

        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(64), 0.0, "the key's red is not at offset 64");
        assert_eq!(read(68), 1.0, "the key's green is not at offset 68");
        assert_eq!(read(72), 0.0, "the key's blue is not at offset 72");
        assert_eq!(read(76), 0.25, "tolerance is not at offset 76");
        assert_eq!(read(80), 0.125, "softness is not at offset 80");
        assert_eq!(read(84), 0.5, "spill is not at offset 84");
    }

    /// No key writes zeroes, which the shader reads as "no key": a tolerance
    /// and softness of zero remove nothing. Anything else there would key a
    /// clip nobody asked to key.
    #[test]
    fn no_key_leaves_the_shader_nothing_to_do() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        assert!(
            bytes[64..96].iter().all(|byte| *byte == 0),
            "a clip with no key carries one anyway"
        );
    }

    /// The key is picked in sRGB and compared against linear samples (§21a.1),
    /// so it is converted on the way in — once per layer rather than once per
    /// pixel. Mid-grey is the case that shows it: 0.5 sRGB is 0.214 linear,
    /// and keying on 0.5 would key a much lighter colour than the one picked.
    #[test]
    fn the_key_colour_is_converted_to_linear() {
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color: ColorAdjust::default(),
                key: Some(bettercut_timeline::ChromaKey {
                    color: [0.5, 0.5, 0.5],
                    ..bettercut_timeline::ChromaKey::default()
                }),
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let red = f32::from_ne_bytes(bytes[64..68].try_into().expect("4 bytes"));
        assert!(
            (red - 0.2140).abs() < 0.001,
            "mid-grey went to the shader as {red}, not 0.214"
        );
    }

    /// §50: a project file can carry anything. A negative brightness inverts
    /// the picture and a huge contrast produces values the sRGB encode turns
    /// into NaN, so the uniform clamps rather than trusts.
    #[test]
    fn absurd_colour_values_are_clamped_before_reaching_the_shader() {
        let color = ColorAdjust {
            brightness: -5.0,
            contrast: 1e9,
            saturation: f32::NAN,
            // The white balance is a gain too, so the same argument applies: a
            // temperature of -40 sends the red channel deeply negative.
            temperature: -40.0,
            tint: 12.0,
            vibrance: 0.0,
        };
        let bytes = layer_uniform(
            Transform::default(),
            LayerLook {
                bars: 0.0,
                shadow: Default::default(),
                border: bettercut_timeline::Border::NONE,
                vignette: 0.0,
                grain: 0.0,
                grain_seed: 0,
                corner_pin: bettercut_timeline::CornerPin::NONE,
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                color,
                key: None,
                mask: None,
            },
            1920,
            1080,
            Resolution::HD_1080,
        );
        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));

        assert_eq!(read(52), 0.0, "negative brightness was not clamped");
        assert_eq!(read(56), 4.0, "runaway contrast was not clamped");
        // `f32::clamp` on NaN returns NaN, so this documents what actually
        // reaches the GPU rather than pretending otherwise.
        assert!(read(60).is_nan() || (0.0..=4.0).contains(&read(60)));
        assert_eq!(read(120), -1.0, "a runaway temperature was not clamped");
        assert_eq!(read(124), 1.0, "a runaway tint was not clamped");
    }

    /// Positive rotation must turn the same way on screen as the number
    /// suggests; a sign error here mirrors every rotated clip.
    #[test]
    fn rotation_turns_clockwise_on_screen() {
        let transform = Transform {
            rotation_degrees: 90.0,
            ..Default::default()
        };
        let bytes = layer_uniform(
            transform,
            LayerLook {
                opacity: 1.0,
                ..LayerLook::default()
            },
            1080,
            1080,
            Resolution::new(1080, 1080),
        );
        let matrix = read_matrix(&bytes);

        // The top-centre of the quad should swing to the right-hand side.
        let top_centre = apply(&matrix, (0.5, 0.0));
        assert!(
            top_centre.0 > 0.5,
            "top edge went to x={} - rotation is mirrored",
            top_centre.0
        );
        assert!(top_centre.1.abs() < 1e-4);
    }

    #[test]
    fn opacity_is_stored_and_clamped() {
        let read = |o: f32| {
            let bytes = layer_uniform(
                Transform::default(),
                LayerLook {
                    opacity: o,
                    ..LayerLook::default()
                },
                16,
                9,
                Resolution::new(16, 9),
            );
            f32::from_ne_bytes([bytes[48], bytes[49], bytes[50], bytes[51]])
        };
        assert!((read(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(read(2.0), 1.0);
        assert_eq!(read(-1.0), 0.0);
    }
}
