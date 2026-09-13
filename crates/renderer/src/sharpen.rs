//! Sharpen: an unsharp mask, as an effect-graph node (§20, §45).
//!
//! The second node in the graph. Blur was the first and for a long time the
//! only one, which meant the graph's claim to *be* a graph — a list of nodes
//! the compositor runs in order without knowing what they are — had never been
//! tested by a second thing to run. This is that second thing.
//!
//! The shader explains the mask itself. What lives here is how big it is and
//! when it is worth drawing at all.

use eframe::wgpu;

use crate::error::RenderError;
use crate::graph::{EffectContext, EffectInput, EffectNode, EffectParams, EffectTexture};

/// Uniform stride, matching the blur's and the compositor's.
const UNIFORM_STRIDE: u64 = 256;

/// Bytes in `SharpenParams`. Must match `sharpen.wgsl` exactly.
const PARAMS_SIZE: u64 = 16;

/// How many passes may be recorded before the uniform buffer grows.
const INITIAL_PASS_CAPACITY: u64 = 16;

/// The neighbourhood's radius, as a fraction of the texture's height.
///
/// A fraction rather than a texel count, for the reason the blur's radius is
/// one: a proxy and the original have to be sharpened over the same share of
/// the frame, or the preview shows a different picture from the export. One
/// texel on a 1080-line frame.
const RADIUS_FRACTION: f32 = 1.0 / 1080.0;

/// The detail added back at the top of the slider, beyond the picture itself.
///
/// Two times: past this every edge carries a visible halo, which is the point
/// at which "sharper" has become "damaged".
const MAX_AMOUNT: f32 = 2.0;

/// A sharpen resolved against a texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenPlan {
    /// Neighbour distance in texels.
    pub radius: f32,
    /// Detail gain, 0 to [`MAX_AMOUNT`].
    pub amount: f32,
}

impl SharpenPlan {
    /// How to sharpen a `height`-texel-tall texture by `amount` (0–100).
    ///
    /// `None` when there is nothing to do, so a clip left alone costs no pass
    /// and no texture.
    pub fn new(amount: f32, height: u32) -> Option<Self> {
        // As the blur's is: a NaN from a hand-edited project would be a NaN
        // gain, and every pixel of the clip NaN.
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }
        let share = amount.min(bettercut_timeline::MAX_SHARPEN) / bettercut_timeline::MAX_SHARPEN;
        Some(Self {
            // Never under a texel. On a small proxy a fraction of a texel would
            // sample between the centre and its neighbour, and the "detail"
            // would be mostly the centre itself — a sharpen that fades out on
            // exactly the frames it is being previewed on. A proxy has less
            // fine detail to bring out whatever the radius; this at least
            // brings out what it has.
            radius: (RADIUS_FRACTION * height.max(1) as f32).max(1.0),
            amount: share * MAX_AMOUNT,
        })
    }

    fn write(self, width: u32, height: u32) -> [u8; PARAMS_SIZE as usize] {
        let mut bytes = [0_u8; PARAMS_SIZE as usize];
        let step_x = self.radius / width.max(1) as f32;
        let step_y = self.radius / height.max(1) as f32;
        bytes[0..4].copy_from_slice(&step_x.to_ne_bytes());
        bytes[4..8].copy_from_slice(&step_y.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.amount.to_ne_bytes());
        // 12..16 is the explicit padding `sharpen.wgsl` declares.
        bytes
    }
}

pub struct SharpenPass {
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

impl SharpenPass {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        // Clamped, so the neighbours of an edge texel repeat the edge rather
        // than reading nothing and darkening the border of every sharpened clip.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sharpen sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sharpen params bgl"),
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
            label: Some("sharpen params"),
            size: INITIAL_PASS_CAPACITY * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = params_bind_group(device, &params_layout, &sampler, &uniforms);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sharpen.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sharpen.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sharpen layout"),
            // The compositor's own texture layout in group 1, as the blur uses,
            // so a decoded frame and the blur's output are interchangeable
            // inputs — which is what lets the two run in either order.
            bind_group_layouts: &[Some(&params_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sharpen pipeline"),
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
                entry_point: Some("fs_sharpen"),
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
            label: Some("sharpen params"),
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

impl EffectNode for SharpenPass {
    fn name(&self) -> &'static str {
        "sharpen"
    }

    fn begin_frame(&mut self, device: &wgpu::Device, layers: usize) {
        self.next_pass = 0;
        // One pass per chain, worst case every chain sharpened.
        self.ensure_capacity(device, layers as u64);
    }

    fn apply(
        &mut self,
        ctx: &mut EffectContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        params: EffectParams,
        input: EffectInput<'_>,
    ) -> Result<Option<EffectTexture>, RenderError> {
        let Some(plan) = SharpenPlan::new(params.sharpen, input.height) else {
            return Ok(None);
        };

        let slot = self.next_pass;
        self.next_pass += 1;
        if slot >= self.capacity {
            // `begin_frame` sizes for every chain, so this cannot happen.
            tracing::warn!(
                pass = slot,
                capacity = self.capacity,
                "sharpen ran out of uniform slots; skipping this layer"
            );
            return Ok(None);
        }

        ctx.queue().write_buffer(
            &self.uniforms,
            slot * UNIFORM_STRIDE,
            &plan.write(input.width, input.height),
        );

        let target = ctx.acquire(input.width, input.height);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sharpen pass"),
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
        label: Some("sharpen params bind group"),
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
    fn no_sharpen_costs_nothing() {
        assert!(SharpenPlan::new(0.0, 1080).is_none());
        assert!(SharpenPlan::new(-5.0, 1080).is_none());
        assert!(SharpenPlan::new(f32::NAN, 1080).is_none());
        assert!(SharpenPlan::new(f32::INFINITY, 1080).is_none());
    }

    /// The same share of the frame at any size — as long as that share is at
    /// least a texel.
    #[test]
    fn the_radius_follows_the_frame_and_never_drops_under_a_texel() {
        let original = SharpenPlan::new(50.0, 2160).expect("sharpen");
        let full_hd = SharpenPlan::new(50.0, 1080).expect("sharpen");
        let proxy = SharpenPlan::new(50.0, 540).expect("sharpen");
        assert!((original.radius - 2.0 * full_hd.radius).abs() < 1e-4);
        assert_eq!(proxy.radius, 1.0, "a proxy's radius fell under a texel");
    }

    /// The slider tops out at the limit, however far a file says to go.
    #[test]
    fn the_amount_is_held_to_its_ceiling() {
        let top = SharpenPlan::new(bettercut_timeline::MAX_SHARPEN, 1080).expect("sharpen");
        let past = SharpenPlan::new(10_000.0, 1080).expect("sharpen");
        assert_eq!(top.amount, MAX_AMOUNT);
        assert_eq!(past.amount, MAX_AMOUNT);
    }
}
