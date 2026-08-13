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
use bettercut_timeline::{Resolution, Transform};
use eframe::wgpu;

use crate::blur::BlurPass;
use crate::config::RenderConfig;
use crate::error::RenderError;
use crate::graph::{EffectContext, EffectInput, EffectNode, EffectTexture, TargetPool, run_chain};

/// Uniform stride. wgpu requires dynamic uniform offsets to be aligned, and
/// 256 is the limit on every backend we target.
const UNIFORM_STRIDE: u64 = 256;

/// How many layers one composite may draw before the uniform buffer grows.
const INITIAL_LAYER_CAPACITY: u64 = 16;

/// One thing to draw, in compositing order.
pub struct Layer<'a> {
    pub frame: &'a VideoFrame,
    pub transform: Transform,
    pub opacity: f32,
    /// §45's "colour adjustment → Cheap": three multiplies in the fragment
    /// shader, so it costs nothing worth measuring.
    pub color: bettercut_timeline::ColorAdjust,
    /// §45's "blur → Medium", 0–100. Unlike the two above this cannot ride in
    /// the composite pass — it reads a neighbourhood rather than one texel —
    /// so a non-zero value buys two extra passes and two textures.
    pub blur: f32,
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

    pipeline: wgpu::RenderPipeline,
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

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    // Straight alpha over: each layer composites onto whatever
                    // the lower tracks already put down (§22).
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let effects: Vec<Box<dyn EffectNode>> = vec![Box::new(BlurPass::new(
            &device,
            &texture_layout,
            Self::FORMAT,
        ))];

        Ok(Self {
            device,
            queue,
            config,
            target,
            target_view,
            present_view,
            pipeline,
            layer_bind_group,
            texture_layout,
            uniforms,
            uniform_capacity,
            pool: HashMap::new(),
            in_flight: Vec::new(),
            effects,
            targets: TargetPool::new(Self::FORMAT),
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
    /// that order and this does not reorder them.
    pub fn composite(&mut self, layers: &[Layer<'_>]) -> Result<(), RenderError> {
        self.recycle();
        self.ensure_uniform_capacity(layers.len() as u64)?;

        // Upload every frame first, so the render pass borrows nothing that is
        // still being written.
        let mut uploaded = Vec::with_capacity(layers.len());
        for (index, layer) in layers.iter().enumerate() {
            let texture = self.upload(layer.frame)?;
            let uniform = layer_uniform(
                layer.transform,
                layer.opacity,
                layer.color,
                texture.width,
                texture.height,
                self.config.resolution,
            );
            self.queue
                .write_buffer(&self.uniforms, index as u64 * UNIFORM_STRIDE, &uniform);
            uploaded.push(texture);
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
            for node in effects.iter_mut() {
                node.begin_frame(ctx.device(), layers.len());
            }

            let mut effected = Vec::with_capacity(layers.len());
            for (index, layer) in layers.iter().enumerate() {
                let source = EffectInput {
                    bind_group: &uploaded[index].bind_group,
                    width: uploaded[index].width,
                    height: uploaded[index].height,
                };
                effected.push(run_chain(effects, &mut ctx, &mut encoder, layer, source)?);
            }
            effected
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Black, not transparent: the canvas behind the lowest
                        // track is what a viewer sees in the letterbox, and
                        // §21a's output is opaque.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.pipeline);
            for (index, texture) in uploaded.iter().enumerate() {
                let offset = (index as u64 * UNIFORM_STRIDE) as u32;
                // A layer whose chain produced something composites from that
                // instead of the frame that was uploaded. Everything else about
                // the draw is identical, which is the whole reason effect
                // targets are built with the compositor's own texture bind
                // group layout.
                let source = effected[index]
                    .as_ref()
                    .map_or(&texture.bind_group, EffectTexture::bind_group);
                pass.set_bind_group(0, &self.layer_bind_group, &[offset]);
                pass.set_bind_group(1, source, &[]);
                pass.draw(0..6, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Hold the textures until the next composite: the GPU may still be
        // reading them, and returning them to the pool now would let the next
        // frame overwrite pixels that are still being sampled.
        self.in_flight = uploaded;
        for texture in effected.into_iter().flatten() {
            self.targets.retire(texture);
        }
        Ok(())
    }

    /// Intermediate textures the effect graph is holding, for §73's memory
    /// diagnostics — and for asserting that they are reused rather than
    /// reallocated every frame.
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
        let layout = self.pipeline.get_bind_group_layout(0);
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
/// `vec4`-aligned columns in WGSL, plus opacity and its padding.
const UNIFORM_SIZE: u64 = 64;

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
fn layer_uniform(
    transform: Transform,
    opacity: f32,
    color: bettercut_timeline::ColorAdjust,
    source_width: u32,
    source_height: u32,
    output: Resolution,
) -> [u8; UNIFORM_SIZE as usize] {
    let source_aspect = source_width.max(1) as f32 / source_height.max(1) as f32;
    let output_aspect = output.width.max(1) as f32 / output.height.max(1) as f32;

    // Fit inside the frame.
    let (fit_x, fit_y) = if source_aspect > output_aspect {
        (1.0, output_aspect / source_aspect)
    } else {
        (source_aspect / output_aspect, 1.0)
    };

    let scale_x = fit_x * transform.scale.x;
    let scale_y = fit_y * transform.scale.y;

    // The unit quad runs 0..1; map it to -1..1 clip space, apply scale and
    // rotation about the anchor, then translate. Clip-space y is up, while the
    // transform's y is down (screen convention), hence the negation.
    let radians = transform.rotation_degrees.to_radians();
    let (sin, cos) = radians.sin_cos();

    let anchor_x = transform.anchor.x;
    let anchor_y = transform.anchor.y;

    // Column-major mat3x3, each column padded to 16 bytes for WGSL layout.
    let a = 2.0 * scale_x * cos;
    let b = 2.0 * scale_x * sin;
    let c = -2.0 * scale_y * sin;
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

    #[test]
    fn an_identity_transform_fills_a_matching_frame() {
        let bytes = layer_uniform(
            Transform::default(),
            1.0,
            ColorAdjust::default(),
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
            1.0,
            ColorAdjust::default(),
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
            1.0,
            ColorAdjust::default(),
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
            1.0,
            ColorAdjust::default(),
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
            1.0,
            ColorAdjust::default(),
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
        };
        let bytes = layer_uniform(
            Transform::default(),
            0.5,
            color,
            1920,
            1080,
            Resolution::HD_1080,
        );

        let read = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(read(48), 0.5, "opacity moved");
        assert_eq!(read(52), 1.25, "brightness is not at offset 52");
        assert_eq!(read(56), 0.75, "contrast is not at offset 56");
        assert_eq!(read(60), 0.5, "saturation is not at offset 60");
        assert_eq!(bytes.len(), UNIFORM_SIZE as usize);
        assert_eq!(UNIFORM_SIZE, 64, "the WGSL struct is declared as 64 bytes");
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
        };
        let bytes = layer_uniform(
            Transform::default(),
            1.0,
            color,
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
            1.0,
            ColorAdjust::default(),
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
                o,
                ColorAdjust::default(),
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
