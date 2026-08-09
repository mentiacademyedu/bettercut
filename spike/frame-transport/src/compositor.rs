//! Milestone 0 spike — the wgpu compositor.
//!
//! Validates §5: the compositor's output is *already* a `wgpu::Texture`, which
//! `egui-wgpu` registers as a `TextureId` and paints into a rect. No interop
//! layer, no frame copy, no IPC, no overlay window.
//!
//! It also validates §46's shape: one render graph, two output sinks. Here the
//! sink is a texture; at Milestone 6 the same pass writes to an encoder input.

use eframe::egui;
use eframe::egui_wgpu::RenderState;
use eframe::wgpu;

/// Matches the `Uniforms` struct in `composite.wgsl`.
#[derive(Clone, Copy)]
pub struct CompositeParams {
    pub overlay_offset: [f32; 2],
    pub overlay_scale: [f32; 2],
    pub overlay_opacity: f32,
    pub expand_limited_range: bool,
}

impl CompositeParams {
    /// Serialize without pulling in `bytemuck` (§74: justify every dependency).
    fn to_bytes(self) -> [u8; 32] {
        let floats = [
            self.overlay_offset[0],
            self.overlay_offset[1],
            self.overlay_scale[0],
            self.overlay_scale[1],
            self.overlay_opacity,
            if self.expand_limited_range { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ];
        let mut bytes = [0u8; 32];
        for (i, f) in floats.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
        }
        bytes
    }
}

pub struct Compositor {
    // `wgpu::Device`/`Queue` are cheap refcounted handles; cloning shares the
    // *same* device as egui. That shared device is the whole point of §4.
    device: wgpu::Device,
    queue: wgpu::Queue,

    /// "Decoded video frame" layer.
    layer_a: wgpu::Texture,
    /// Overlay layer — a second track, resident on the GPU.
    _layer_b: wgpu::Texture,
    /// Compositor output. This is what egui paints. It never leaves the GPU.
    _target: wgpu::Texture,
    target_view: wgpu::TextureView,

    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,

    pub texture_id: egui::TextureId,
    pub width: u32,
    pub height: u32,
}

impl Compositor {
    pub fn new(
        rs: &RenderState,
        width: u32,
        height: u32,
        overlay: &[u8],
        overlay_w: u32,
        overlay_h: u32,
    ) -> Self {
        let device = rs.device.clone();
        let queue = rs.queue.clone();

        // §21a.1: the v1 working space is sRGB-encoded 8-bit RGBA.
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

        let layer_a = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer_a (decoded frame)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let layer_b = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer_b (overlay track)"),
            size: wgpu::Extent3d {
                width: overlay_w,
                height: overlay_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &layer_b,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            overlay,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(overlay_w * 4),
                rows_per_image: Some(overlay_h),
            },
            wgpu::Extent3d {
                width: overlay_w,
                height: overlay_h,
                depth_or_array_layers: 1,
            },
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composite target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view_a = layer_a.create_view(&wgpu::TextureViewDescriptor::default());
        let view_b = layer_b.create_view(&wgpu::TextureViewDescriptor::default());
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

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

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite uniforms"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite bgl"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite bind group"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view_b),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniforms.as_entire_binding(),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite pipeline"),
            layout: Some(&layout),
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
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // The one line that makes this whole architecture work (§4.1).
        let texture_id = rs.renderer.write().register_native_texture(
            &device,
            &target_view,
            wgpu::FilterMode::Linear,
        );

        Self {
            device,
            queue,
            layer_a,
            _layer_b: layer_b,
            _target: target,
            target_view,
            pipeline,
            bind_group,
            uniforms,
            texture_id,
            width,
            height,
        }
    }

    /// The software-fallback transport (§5): frame in system RAM -> GPU texture.
    /// Measured so the ADR can state what the RAM round trip actually costs.
    pub fn upload_frame(&self, rgba: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.layer_a,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Composite both layers into the target texture.
    pub fn composite(&self, params: CompositeParams) {
        self.queue
            .write_buffer(&self.uniforms, 0, &params.to_bytes());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
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
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Block until the GPU has finished all submitted work. Used only by the
    /// measurement mode — never on a normal frame, since it stalls the pipeline.
    pub fn wait_for_gpu(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}
