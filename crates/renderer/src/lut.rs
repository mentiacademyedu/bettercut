//! Colour lookup tables, as an effect-graph node (§20, §45).
//!
//! The table itself — reading `.cube` files, what the numbers mean — lives in
//! `bettercut_timeline::lut`. This is the GPU half: each table the caller hands
//! over becomes a 3D texture, and the node runs a layer through the one its
//! clip names.
//!
//! ## Tables are loaded, not looked up
//!
//! The renderer does no file IO. The preview and the export each read a
//! project's `.cube` files and give the compositor the parsed tables
//! ([`crate::Compositor::load_lut`]) before drawing — the same tables through
//! the same node, so the two cannot grade differently (§46). A clip naming a
//! table that was never loaded — its file has gone missing — is drawn
//! ungraded, never as an error: §66's rule for missing media, applied to a
//! missing look.
//!
//! ## 8-bit tables
//!
//! Stored as `Rgba8Unorm`: filterable on every adapter without an optional
//! feature, and the output is 8-bit video anyway. The hardware interpolates
//! between entries, so the table's own steps never show as bands.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use bettercut_foundation::LutId;
use bettercut_timeline::CubeLut;
use eframe::wgpu;

use crate::error::RenderError;
use crate::graph::{EffectContext, EffectInput, EffectNode, EffectParams, EffectTexture};

const UNIFORM_STRIDE: u64 = 256;
/// Bytes in `LutParams`. Must match `lut.wgsl` exactly.
const PARAMS_SIZE: u64 = 32;
const INITIAL_PASS_CAPACITY: u64 = 16;

/// A table on the GPU.
struct LoadedLut {
    bind_group: wgpu::BindGroup,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    size: u32,
}

/// Every table the compositor has been given, shared between it (which loads
/// them) and the node (which draws with them).
#[derive(Clone)]
pub struct LutTables {
    loaded: Arc<Mutex<HashMap<LutId, LoadedLut>>>,
    /// The one layout every table is bound with — the node's pipeline is built
    /// against it, so a table loaded later still fits.
    layout: wgpu::BindGroupLayout,
}

impl LutTables {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lut table bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            }],
        });
        Self {
            loaded: Arc::new(Mutex::new(HashMap::new())),
            layout,
        }
    }

    pub fn contains(&self, id: LutId) -> bool {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Upload `lut` under `id`, replacing any table already there.
    pub fn load(&self, device: &wgpu::Device, queue: &wgpu::Queue, id: LutId, lut: &CubeLut) {
        let size = lut.size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lut table"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Red fastest, then green, then blue: exactly the texture's own x, y,
        // z order, so the table uploads as it was read.
        let bytes: Vec<u8> = lut
            .table
            .iter()
            .flat_map(|rgb| {
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                [q(rgb[0]), q(rgb[1]), q(rgb[2]), 255]
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lut table bind group"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                LoadedLut {
                    bind_group,
                    domain_min: lut.domain_min,
                    domain_max: lut.domain_max,
                    size,
                },
            );
    }
}

pub struct LutPass {
    pipeline: wgpu::RenderPipeline,
    params_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    capacity: u64,
    next_pass: u64,
    tables: LutTables,
}

impl LutPass {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        tables: LutTables,
    ) -> Self {
        // Linear, so a 3D lookup interpolates between table entries; clamped,
        // so the extremes read the table's edge rather than wrapping round.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lut sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lut params bgl"),
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
            label: Some("lut params"),
            size: INITIAL_PASS_CAPACITY * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = params_bind_group(device, &params_layout, &sampler, &uniforms);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lut.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lut.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lut layout"),
            bind_group_layouts: &[
                Some(&params_layout),
                Some(texture_layout),
                Some(&tables.layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lut pipeline"),
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
                entry_point: Some("fs_lut"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            tables,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, passes: u64) {
        if passes <= self.capacity {
            return;
        }
        let capacity = passes.next_power_of_two().max(INITIAL_PASS_CAPACITY);
        self.uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lut params"),
            size: capacity * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.capacity = capacity;
        self.bind_group =
            params_bind_group(device, &self.params_layout, &self.sampler, &self.uniforms);
    }
}

impl EffectNode for LutPass {
    fn name(&self) -> &'static str {
        "lut"
    }

    fn begin_frame(&mut self, device: &wgpu::Device, layers: usize) {
        self.next_pass = 0;
        self.ensure_capacity(device, layers as u64);
    }

    fn apply(
        &mut self,
        ctx: &mut EffectContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        params: EffectParams,
        input: EffectInput<'_>,
    ) -> Result<Option<EffectTexture>, RenderError> {
        let Some(clip_lut) = params.lut.map(bettercut_timeline::ClipLut::clamped) else {
            return Ok(None);
        };
        if clip_lut.strength <= 0.0 {
            return Ok(None);
        }
        let tables = self
            .tables
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Named but never loaded — its file is missing. Drawn as it is.
        let Some(table) = tables.get(&clip_lut.lut) else {
            return Ok(None);
        };

        let slot = self.next_pass;
        self.next_pass += 1;
        if slot >= self.capacity {
            tracing::warn!(
                pass = slot,
                "lut ran out of uniform slots; skipping this layer"
            );
            return Ok(None);
        }

        let mut bytes = [0_u8; PARAMS_SIZE as usize];
        for c in 0..3 {
            bytes[c * 4..c * 4 + 4].copy_from_slice(&table.domain_min[c].to_ne_bytes());
            bytes[16 + c * 4..20 + c * 4].copy_from_slice(&table.domain_max[c].to_ne_bytes());
        }
        bytes[12..16].copy_from_slice(&clip_lut.strength.to_ne_bytes());
        bytes[28..32].copy_from_slice(&(table.size as f32).to_ne_bytes());
        ctx.queue()
            .write_buffer(&self.uniforms, slot * UNIFORM_STRIDE, &bytes);

        let target = ctx.acquire(input.width, input.height);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lut pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
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
            pass.set_bind_group(2, &table.bind_group, &[]);
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
        label: Some("lut params bind group"),
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
