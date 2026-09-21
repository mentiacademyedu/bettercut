//! RGB split and glitch, as an effect-graph node (§20).
//!
//! The shader does the work; this decides when it is worth a pass and turns
//! the Inspector's percentages into the shader's units. Both amounts are
//! shares of the picture's size rather than pixels, so a proxy and the original
//! break up in the same places (§46).

use eframe::wgpu;

use crate::error::RenderError;
use crate::graph::{EffectContext, EffectInput, EffectNode, EffectParams, EffectTexture};

/// Uniform stride, matching the other nodes'.
const UNIFORM_STRIDE: u64 = 256;

/// Bytes in `GlitchParams`. Must match `glitch.wgsl` exactly.
const PARAMS_SIZE: u64 = 32;

const INITIAL_PASS_CAPACITY: u64 = 16;

/// The widest RGB split, as a share of the picture's width, at 100%. Past a
/// couple of percent the channels stop reading as one picture at all.
pub const MAX_SPLIT: f32 = 0.02;

/// The biggest pixelate block, as a share of the picture's longer side, at
/// 100%: a face at that size is unrecognisable, and the frame still reads.
pub const MAX_BLOCK: f32 = 0.08;

/// The strongest zoom blur, at 100%: each point takes in the picture this
/// share of its distance towards the middle. A quarter reads as a rush forward without the
/// picture dissolving.
pub const MAX_ZOOM: f32 = 0.25;

/// The strongest glow, at 100%: how much of the gathered light is added back.
pub const MAX_GLOW: f32 = 1.5;

/// An RGB split, glitch, pixelate and zoom blur resolved for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlitchPlan {
    /// Channel separation in UV.
    pub split: f32,
    /// How much of the picture breaks up, 0–1.
    pub glitch: f32,
    /// Block size as a share of the longer side; zero is off.
    pub block: f32,
    /// How far each point streaks towards the middle; zero is off.
    pub zoom: f32,
    /// How much bloom is added; zero is off.
    pub glow: f32,
    /// How worn the film looks, 0–1; zero is off.
    pub film: f32,
    pub seed: u32,
}

impl GlitchPlan {
    /// `rgb_split` and `glitch` are the Inspector's 0–100. `None` when neither
    /// does anything, so a clip without them costs no pass.
    pub fn new(
        rgb_split: f32,
        glitch: f32,
        pixelate: f32,
        zoom_blur: f32,
        glow: f32,
        old_film: f32,
        seed: u32,
    ) -> Option<Self> {
        let share = |amount: f32| {
            if amount.is_finite() {
                (amount / bettercut_timeline::MAX_GLITCH).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let (split, glitch, pixelate) = (share(rgb_split), share(glitch), share(pixelate));
        let zoom = share(zoom_blur);
        let glow = share(glow);
        let film = share(old_film);
        if split <= 0.0
            && glitch <= 0.0
            && pixelate <= 0.0
            && zoom <= 0.0
            && glow <= 0.0
            && film <= 0.0
        {
            return None;
        }
        Some(Self {
            split: split * MAX_SPLIT,
            glitch,
            block: pixelate * MAX_BLOCK,
            zoom: zoom * MAX_ZOOM,
            glow: glow * MAX_GLOW,
            film,
            seed,
        })
    }

    fn write(self) -> [u8; PARAMS_SIZE as usize] {
        let mut bytes = [0_u8; PARAMS_SIZE as usize];
        bytes[0..4].copy_from_slice(&self.split.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.glitch.to_ne_bytes());
        // Below 2^24, where every whole number is exact in an f32.
        bytes[8..12].copy_from_slice(&((self.seed % (1 << 24)) as f32).to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.block.to_ne_bytes());
        bytes[16..20].copy_from_slice(&self.zoom.to_ne_bytes());
        bytes[20..24].copy_from_slice(&self.glow.to_ne_bytes());
        bytes[24..28].copy_from_slice(&self.film.to_ne_bytes());
        bytes
    }
}

pub struct GlitchPass {
    pipeline: wgpu::RenderPipeline,
    params_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    capacity: u64,
    /// Pass slots used so far this frame, for the reason the blur keeps one:
    /// every layer's parameters must survive to the submit.
    next_pass: u64,
}

impl GlitchPass {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        // Clamped, so the neighbours of an edge texel repeat the edge rather
        // than reading nothing and darkening the border of every glitched clip.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glitch sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glitch params bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
            ],
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glitch params"),
            size: INITIAL_PASS_CAPACITY * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = params_bind_group(device, &params_layout, &sampler, &uniforms);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glitch.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glitch.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glitch layout"),
            // The compositor's own texture layout in group 1, as the blur uses,
            // so a decoded frame and the blur's output are interchangeable
            // inputs — which is what lets the two run in either order.
            bind_group_layouts: &[Some(&params_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glitch pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_glitch"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Replace: this writes a whole picture, not onto one.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            params_layout,
            sampler,
            bind_group,
            uniforms,
            capacity: INITIAL_PASS_CAPACITY,
            next_pass: 0,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, passes: u64) {
        if passes <= self.capacity {
            return;
        }
        let capacity = passes.next_power_of_two().max(INITIAL_PASS_CAPACITY);
        self.uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glitch params"),
            size: capacity * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.capacity = capacity;
        // The old bind group points at the buffer just replaced.
        self.bind_group =
            params_bind_group(device, &self.params_layout, &self.sampler, &self.uniforms);
    }
}

impl EffectNode for GlitchPass {
    fn name(&self) -> &'static str {
        "glitch"
    }

    fn begin_frame(&mut self, device: &wgpu::Device, layers: usize) {
        self.next_pass = 0;
        // One pass per chain, worst case every chain glitched.
        self.ensure_capacity(device, layers as u64);
    }

    fn apply(
        &mut self,
        ctx: &mut EffectContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        params: EffectParams,
        input: EffectInput<'_>,
    ) -> Result<Option<EffectTexture>, RenderError> {
        let Some(plan) = GlitchPlan::new(
            params.rgb_split,
            params.glitch,
            params.pixelate,
            params.zoom_blur,
            params.glow,
            params.old_film,
            params.seed,
        ) else {
            return Ok(None);
        };

        let slot = self.next_pass;
        self.next_pass += 1;
        if slot >= self.capacity {
            tracing::warn!(
                pass = slot,
                capacity = self.capacity,
                "glitch ran out of uniform slots; skipping this layer"
            );
            return Ok(None);
        }

        ctx.queue()
            .write_buffer(&self.uniforms, slot * UNIFORM_STRIDE, &plan.write());

        let target = ctx.acquire(input.width, input.height);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glitch pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Every texel is drawn, so what was there is irrelevant.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[(slot * UNIFORM_STRIDE) as u32]);
            pass.set_bind_group(1, input.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        Ok(Some(target))
    }
}

fn params_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glitch params bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(PARAMS_SIZE),
                }),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_costs_nothing() {
        assert!(GlitchPlan::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7).is_none());
        assert!(GlitchPlan::new(-3.0, f32::NAN, -1.0, f32::NAN, f32::NAN, f32::NAN, 7).is_none());
    }

    #[test]
    fn amounts_are_held_to_their_ceilings() {
        let full = GlitchPlan::new(100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 0).expect("plan");
        let past = GlitchPlan::new(1e6, 1e6, 1e6, 1e6, 1e6, 1e6, 0).expect("plan");
        assert_eq!(full, past);
        assert_eq!(full.split, MAX_SPLIT);
        assert_eq!(full.glitch, 1.0);
        assert_eq!(full.block, MAX_BLOCK);
        assert_eq!(full.zoom, MAX_ZOOM);
        assert!(
            GlitchPlan::new(0.0, 0.0, 50.0, 0.0, 0.0, 0.0, 0).is_some(),
            "pixelate alone is a pass"
        );
        assert!(
            GlitchPlan::new(0.0, 0.0, 0.0, 50.0, 0.0, 0.0, 0).is_some(),
            "zoom blur alone is a pass"
        );
    }
}
