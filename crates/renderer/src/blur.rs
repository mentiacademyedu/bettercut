//! Gaussian blur (§45: "Blur -> Medium", Milestone 8).
//!
//! The first effect that does not fit in the composite pass. Opacity and
//! colour are per-pixel functions of one texel, so they ride along in
//! `composite.wgsl` for free; blur reads a neighbourhood, which needs its own
//! passes and somewhere to put the intermediate result.
//!
//! Two things here are easy to get wrong and expensive to discover later.
//!
//! **The radius is not in pixels.** §46 says preview and export differ in
//! resolution and in whether they read a proxy or the original. A blur of "12
//! pixels" applied to a 720p proxy and to the 1080p original is two different
//! pictures, so the preview would be showing something the export will not
//! produce. The stored amount is therefore a fraction of frame height, and the
//! texel radius is derived per pass from the texture actually being sampled.
//!
//! **The tap count is the quality tier.** §45 puts blur in the middle band and
//! §46 requires that preview and export be configurations of one graph rather
//! than two implementations. So the tier changes exactly one number — how many
//! taps the kernel may spend — and nothing else. There is no separate
//! "fast blur" shader to diverge from the real one.

use std::collections::HashMap;

use eframe::wgpu;

use crate::config::QualityTier;
use crate::error::RenderError;

/// Sigma at full strength, as a fraction of frame height.
///
/// At `MAX_BLUR` on a 1080p frame this is a sigma of 27 texels, whose visible
/// radius is about 81 — a face is unrecognisable well before the slider ends,
/// which is the point at which a range stops being useful.
const MAX_SIGMA_FRACTION: f32 = 0.025;

/// Uniform stride, matching the compositor's: wgpu requires dynamic uniform
/// offsets to be aligned to 256 on every backend we target.
const UNIFORM_STRIDE: u64 = 256;

/// Bytes in `BlurParams`. Must match `blur.wgsl` exactly.
const PARAMS_SIZE: u64 = 32;

/// How many passes may be recorded before the uniform buffer grows.
const INITIAL_PASS_CAPACITY: u64 = 16;

impl QualityTier {
    /// Taps either side of centre that a blur kernel may spend (§45, §46).
    ///
    /// The only thing the tier changes. Preview is generous enough that the
    /// approximation is invisible at the scale a preview panel is actually
    /// viewed at; `Full` roughly doubles it, which is what keeps the stride
    /// near 2 texels at 1080p.
    ///
    /// These are per pass and there are two passes, so the cost per output
    /// pixel is `2 * (2 * half + 1)` samples: 66 for preview, 162 for full.
    pub fn blur_half_taps(self) -> i32 {
        match self {
            Self::Preview => 16,
            Self::Full => 40,
        }
    }
}

/// A blur resolved against a specific texture size and quality tier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurPlan {
    /// Standard deviation, in texels of the texture being sampled.
    pub sigma: f32,
    pub half_taps: i32,
    /// Texels between taps. Above 1.0 the kernel is sparse.
    pub stride: f32,
}

impl BlurPlan {
    /// Work out how to blur a `height`-texel-tall texture by `amount` (0–100).
    ///
    /// Returns `None` when there is nothing to do, so an unblurred clip costs
    /// no textures and no passes rather than a wide kernel of zero weights.
    pub fn new(amount: f32, height: u32, tier: QualityTier) -> Option<Self> {
        // `is_finite` is doing real work, not decoration: a NaN or infinite
        // amount from a hand-edited project would make sigma NaN, and the
        // shader divides by the summed weights — every weight NaN, the whole
        // clip black. Refusing the effect is the safe reading of nonsense.
        if !amount.is_finite() || amount <= 0.0 {
            return None;
        }

        let fraction = amount.min(bettercut_timeline::MAX_BLUR) / bettercut_timeline::MAX_BLUR;
        let sigma = fraction * MAX_SIGMA_FRACTION * height.max(1) as f32;
        // Below about a third of a texel the kernel collapses onto the centre
        // tap and the picture is unchanged, but the maths still runs.
        if sigma < 0.3 {
            return None;
        }

        // Three sigma covers 99.7% of the distribution; past that the weights
        // are too small to change an 8-bit result.
        let radius = 3.0 * sigma;
        let half = (radius.ceil() as i32).clamp(1, tier.blur_half_taps());

        Some(Self {
            sigma,
            half_taps: half,
            // When the radius fits inside the tap budget this is 1.0 and every
            // texel is sampled. When it does not, the kernel spreads out to
            // still span the full radius rather than silently blurring less
            // than asked — a truncated kernel would make the slider stop
            // having an effect past a certain point, which reads as a bug.
            stride: (radius / half as f32).max(1.0),
        })
    }

    fn write(self, step: [f32; 2]) -> [u8; PARAMS_SIZE as usize] {
        let mut bytes = [0_u8; PARAMS_SIZE as usize];
        bytes[0..4].copy_from_slice(&step[0].to_ne_bytes());
        bytes[4..8].copy_from_slice(&step[1].to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.sigma.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.stride.to_ne_bytes());
        bytes[16..20].copy_from_slice(&self.half_taps.to_ne_bytes());
        // 20..32 is the explicit padding `blur.wgsl` declares.
        bytes
    }
}

/// An intermediate texture a blur pass renders into.
///
/// Carries a bind group built from the compositor's *source* texture layout,
/// so the result of a blur can be fed straight back in as the next pass's
/// input and, finally, as the layer the composite pass draws.
pub struct BlurTarget {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl BlurTarget {
    /// The bind group to hand the composite pass in place of the source frame.
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

/// One layer that needs blurring.
pub struct BlurJob<'a> {
    pub source: &'a wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    pub plan: BlurPlan,
}

pub struct BlurPass {
    pipeline: wgpu::RenderPipeline,
    params_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    capacity: u64,

    /// Intermediate textures, keyed by size and reused between frames — the
    /// same argument as the compositor's source pool (§68, §73): a blurred
    /// 1080p layer would otherwise allocate 16 MB of texture per frame.
    pool: HashMap<(u32, u32), Vec<BlurTarget>>,
}

impl BlurPass {
    pub fn new(
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        // ClampToEdge, so taps that fall outside the frame repeat the border
        // texel. The alternative is sampling nothing and fading the edges of
        // every blurred clip towards black.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur params bgl"),
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
            label: Some("blur params"),
            size: INITIAL_PASS_CAPACITY * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = params_bind_group(device, &params_layout, &sampler, &uniforms);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur layout"),
            // Group 1 is the compositor's own source-texture layout, so a
            // decoded frame and a half-blurred intermediate are interchangeable
            // inputs and neither needs a second bind group.
            bind_group_layouts: &[Some(&params_layout), Some(texture_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur pipeline"),
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
                entry_point: Some("fs_blur"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Replace, not blend: this pass writes a complete picture
                    // rather than compositing onto one.
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
            pool: HashMap::new(),
        }
    }

    /// Record every job's two passes into `encoder`.
    ///
    /// Returns one pair of intermediates per job, in order. The second of each
    /// pair holds the finished blur; both must outlive the submission, so the
    /// caller holds them until the next frame.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        encoder: &mut wgpu::CommandEncoder,
        jobs: &[BlurJob<'_>],
    ) -> Result<Vec<[BlurTarget; 2]>, RenderError> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }

        // Two passes per job, each needing its own slice of the uniform buffer.
        self.ensure_capacity(device, jobs.len() as u64 * 2);

        // Acquire every texture before recording anything: the render passes
        // below borrow this struct immutably, and the pool cannot be touched
        // while they do.
        let mut pairs = Vec::with_capacity(jobs.len());
        for job in jobs {
            let first = self.acquire(device, texture_layout, format, job.width, job.height);
            let second = self.acquire(device, texture_layout, format, job.width, job.height);
            pairs.push([first, second]);
        }

        for (index, (job, pair)) in jobs.iter().zip(pairs.iter()).enumerate() {
            let horizontal = index as u64 * 2;
            let vertical = horizontal + 1;

            // Step is one texel along the axis being blurred. Both are derived
            // from this texture's own size, which is what makes the result the
            // same whether the source was a proxy or the original (§46).
            queue.write_buffer(
                &self.uniforms,
                horizontal * UNIFORM_STRIDE,
                &job.plan.write([1.0 / job.width.max(1) as f32, 0.0]),
            );
            queue.write_buffer(
                &self.uniforms,
                vertical * UNIFORM_STRIDE,
                &job.plan.write([0.0, 1.0 / job.height.max(1) as f32]),
            );

            self.record(encoder, job.source, &pair[0].view, horizontal);
            self.record(encoder, pair[0].bind_group(), &pair[1].view, vertical);
        }

        Ok(pairs)
    }

    /// One full-screen pass: sample `source`, write `target`.
    fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        pass_index: u64,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blur pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The draw covers every texel, so the previous contents are
                    // irrelevant. wgpu's safe API has no "don't care", and
                    // `Clear` is the cheaper of the two things it does offer —
                    // `Load` would pull the whole attachment back in first.
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
        pass.set_bind_group(0, &self.bind_group, &[(pass_index * UNIFORM_STRIDE) as u32]);
        pass.set_bind_group(1, source, &[]);
        pass.draw(0..6, 0..1);
    }

    /// Hand last frame's intermediates back for reuse.
    pub fn recycle(&mut self, targets: Vec<[BlurTarget; 2]>) {
        for pair in targets {
            for target in pair {
                self.pool
                    .entry((target.width, target.height))
                    .or_default()
                    .push(target);
            }
        }
    }

    fn acquire(
        &mut self,
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> BlurTarget {
        if let Some(pooled) = self
            .pool
            .get_mut(&(width, height))
            .and_then(std::vec::Vec::pop)
        {
            return pooled;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blur intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The compositor's working-space format (§21a.1). sRGB-aware, so
            // the hardware hands the shader linear values and re-encodes what
            // it writes: the averaging happens in linear light, which is what
            // an out-of-focus lens actually does. Averaging sRGB-encoded
            // numbers instead would darken every soft edge.
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur intermediate bind group"),
            layout: texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        BlurTarget {
            view,
            bind_group,
            width,
            height,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, passes: u64) {
        if passes <= self.capacity {
            return;
        }
        let capacity = passes.next_power_of_two().max(INITIAL_PASS_CAPACITY);
        self.uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur params"),
            size: capacity * UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.capacity = capacity;
        // The old bind group still points at the buffer that was just replaced.
        self.bind_group =
            params_bind_group(device, &self.params_layout, &self.sampler, &self.uniforms);
    }
}

fn params_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blur params bind group"),
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
    fn zero_blur_costs_nothing() {
        assert!(BlurPlan::new(0.0, 1080, QualityTier::Full).is_none());
        assert!(BlurPlan::new(-3.0, 1080, QualityTier::Full).is_none());
        // §50: a project file can carry anything, and a NaN sigma would divide
        // the accumulated colour by NaN and paint the clip black.
        assert!(BlurPlan::new(f32::NAN, 1080, QualityTier::Full).is_none());
    }

    /// The reason the stored amount is a fraction rather than a pixel count.
    ///
    /// §46 has preview reading a 720p proxy and export reading the 1080p
    /// original. If the same slider produced the same *texel* radius on both,
    /// the preview would show two-thirds of the blur the export renders, and
    /// the difference would only surface after a long encode.
    #[test]
    fn the_same_amount_blurs_a_proxy_and_the_original_by_the_same_fraction() {
        let proxy = BlurPlan::new(50.0, 720, QualityTier::Full).expect("blur");
        let original = BlurPlan::new(50.0, 1080, QualityTier::Full).expect("blur");

        let ratio = original.sigma / proxy.sigma;
        assert!(
            (ratio - 1080.0 / 720.0).abs() < 1e-4,
            "sigma should scale with height, got {ratio}"
        );
    }

    /// §46: the tier is a number, not a second implementation. It may change
    /// how finely the kernel is sampled, but never how wide it is — otherwise
    /// preview and export would show different amounts of blur.
    #[test]
    fn the_quality_tier_changes_taps_but_not_the_radius() {
        let preview = BlurPlan::new(100.0, 1080, QualityTier::Preview).expect("blur");
        let full = BlurPlan::new(100.0, 1080, QualityTier::Full).expect("blur");

        assert_eq!(preview.sigma, full.sigma, "the tier must not change sigma");
        assert!(full.half_taps > preview.half_taps);

        let span = |plan: BlurPlan| plan.half_taps as f32 * plan.stride;
        assert!(
            (span(preview) - span(full)).abs() < 0.5,
            "both tiers must cover the same radius: {} vs {}",
            span(preview),
            span(full)
        );
    }

    /// A kernel that fits in the budget must sample every texel; striding when
    /// there is no need to would throw away detail for nothing.
    #[test]
    fn a_narrow_blur_samples_every_texel() {
        let plan = BlurPlan::new(2.0, 1080, QualityTier::Full).expect("blur");
        assert_eq!(plan.stride, 1.0);
        assert!(plan.half_taps as f32 >= 3.0 * plan.sigma);
    }

    #[test]
    fn a_wide_blur_stays_within_the_tap_budget() {
        for tier in [QualityTier::Preview, QualityTier::Full] {
            let plan = BlurPlan::new(100.0, 2160, tier).expect("blur");
            assert!(
                plan.half_taps <= tier.blur_half_taps(),
                "{tier:?} spent {} taps",
                plan.half_taps
            );
            assert!(plan.stride > 1.0, "a 4K frame at full blur must stride");
        }
    }

    /// The Rust writer and `blur.wgsl` have to agree byte for byte. A mismatch
    /// is a wrong picture rather than a validation error, because the size is
    /// what wgpu checks and the size would still be right.
    #[test]
    fn params_match_the_shader_layout() {
        let plan = BlurPlan {
            sigma: 4.5,
            half_taps: 13,
            stride: 2.5,
        };
        let bytes = plan.write([0.25, 0.0]);

        let float = |at: usize| f32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
        let int = |at: usize| i32::from_ne_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));

        assert_eq!(float(0), 0.25, "step.x");
        assert_eq!(float(4), 0.0, "step.y");
        assert_eq!(float(8), 4.5, "sigma");
        assert_eq!(float(12), 2.5, "stride");
        assert_eq!(int(16), 13, "half_taps");
        assert_eq!(bytes.len(), PARAMS_SIZE as usize);
        const { assert!(PARAMS_SIZE <= UNIFORM_STRIDE) };
    }

    /// Two passes of N taps, not one pass of N squared. At the full tier that
    /// is the difference between 162 samples per pixel and 6561.
    #[test]
    fn separability_is_what_makes_the_tap_budget_affordable() {
        for tier in [QualityTier::Preview, QualityTier::Full] {
            let half = tier.blur_half_taps();
            let separable = 2 * (2 * half + 1);
            let naive = (2 * half + 1) * (2 * half + 1);
            assert!(separable * 10 < naive);
        }
    }
}
