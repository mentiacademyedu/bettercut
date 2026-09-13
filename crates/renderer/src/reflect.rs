//! Reflections — mirrored halves, four-way, kaleidoscope — as an effect-graph
//! node (§20).
//!
//! The mapping itself is `bettercut_timeline::Reflection::source_uv`; the
//! shader transcribes it, and a GPU test holds the two together. This node
//! only decides when a pass is worth it and hands the shader the kind and the
//! picture's shape.

use eframe::wgpu;

use bettercut_timeline::Reflection;

use crate::error::RenderError;
use crate::graph::{EffectContext, EffectInput, EffectNode, EffectParams, EffectTexture};

/// Uniform stride, matching the other nodes'.
const UNIFORM_STRIDE: u64 = 256;

/// Bytes in `ReflectParams`. Must match `reflect.wgsl` exactly.
const PARAMS_SIZE: u64 = 16;

const INITIAL_PASS_CAPACITY: u64 = 16;

/// The shader's parameters for one layer: which reflection, and the aspect.
fn params_bytes(reflection: Reflection, width: u32, height: u32) -> [u8; PARAMS_SIZE as usize] {
    let mut bytes = [0_u8; PARAMS_SIZE as usize];
    bytes[0..4].copy_from_slice(&(reflection.code() as f32).to_ne_bytes());
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    bytes[4..8].copy_from_slice(&aspect.to_ne_bytes());
    bytes
}

pub struct ReflectPass {
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

impl ReflectPass {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        // Clamped: the mapping already keeps every read inside the picture, and
        // clamping keeps a bilinear read at the very edge from wrapping round.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflect sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflect params bgl"),
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
            label: Some("reflect params"),
            size: INITIAL_PASS_CAPACITY * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = params_bind_group(device, &params_layout, &sampler, &uniforms);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reflect.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("reflect.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("reflect layout"),
            // The compositor's own texture layout in group 1, as the blur uses,
            // so a decoded frame and the blur's output are interchangeable
            // inputs — which is what lets the two run in either order.
            bind_group_layouts: &[Some(&params_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("reflect pipeline"),
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
                entry_point: Some("fs_reflect"),
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
            label: Some("reflect params"),
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

impl EffectNode for ReflectPass {
    fn name(&self) -> &'static str {
        "reflect"
    }

    fn begin_frame(&mut self, device: &wgpu::Device, layers: usize) {
        self.next_pass = 0;
        // One pass per chain, worst case every chain reflected.
        self.ensure_capacity(device, layers as u64);
    }

    fn apply(
        &mut self,
        ctx: &mut EffectContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        params: EffectParams,
        input: EffectInput<'_>,
    ) -> Result<Option<EffectTexture>, RenderError> {
        if params.reflection == Reflection::None {
            return Ok(None);
        }

        let slot = self.next_pass;
        self.next_pass += 1;
        if slot >= self.capacity {
            tracing::warn!(
                pass = slot,
                capacity = self.capacity,
                "reflect ran out of uniform slots; skipping this layer"
            );
            return Ok(None);
        }

        ctx.queue().write_buffer(
            &self.uniforms,
            slot * UNIFORM_STRIDE,
            &params_bytes(params.reflection, input.width, input.height),
        );

        let target = ctx.acquire(input.width, input.height);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reflect pass"),
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
        label: Some("reflect params bind group"),
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
