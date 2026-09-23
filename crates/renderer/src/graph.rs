//! The effect graph: effects are nodes, not fixed pipeline stages (§20, §46).
//!
//! > Structure the compositor as a **render graph**: effects are nodes, not
//! > fixed pipeline stages. Templates and transitions then become data (§29)
//! > rather than code.
//!
//! ## What is a node and what is not
//!
//! A node is an effect that needs **its own pass** — one that reads more than
//! the texel it is writing. Blur is the first: it samples a neighbourhood, so
//! it cannot be folded into the draw that composites the layer.
//!
//! Opacity, colour and the transform are deliberately *not* nodes. They are
//! per-texel functions evaluated in `composite.wgsl` during the draw that was
//! going to happen anyway, so making them nodes would add a full-frame pass and
//! a full-frame texture each, for arithmetic that costs nothing where it is.
//! "Effects are nodes" is about the ones that need a pass, not about moving
//! free work into passes to look uniform.
//!
//! ## One chain per layer, run before compositing
//!
//! Each layer's chain runs on that layer's own texture, and the composite pass
//! then draws the result exactly as it would have drawn the decoded frame. A
//! blurred clip therefore still scales, rotates and blends like any other: the
//! geometry stays in one place instead of being duplicated into every effect
//! shader.
//!
//! ## Why the intermediates are pooled here rather than per node
//!
//! Every node wants a full-resolution scratch texture, and at 1080p that is
//! 8 MB. Allocating one per node per layer per frame is 30 allocations a second
//! on a two-effect chain, which §68 and §73 both rule out. One pool, keyed by
//! size, shared by every node: a chain of blur-then-something reuses the same
//! textures the frame before finished with.
//!
//! Textures are held for a **frame** after their last use, not returned
//! immediately. The GPU is still reading them when `composite` returns; handing
//! one straight back to the pool would let the next frame's first pass
//! overwrite pixels the previous frame is still sampling.

use std::collections::HashMap;

use eframe::wgpu;

use crate::config::QualityTier;
use crate::error::RenderError;

/// A texture an effect wrote, ready to be read by the next one.
///
/// Its bind group is built from the compositor's *source* texture layout, which
/// is what makes a node's output and a decoded frame interchangeable — the next
/// node, and the composite pass, cannot tell which they were given.
pub struct EffectTexture {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl EffectTexture {
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// The view to render into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn as_input(&self) -> EffectInput<'_> {
        EffectInput {
            bind_group: &self.bind_group,
            width: self.width,
            height: self.height,
        }
    }
}

/// The settings a node needs, without the clip they came from.
///
/// A node is given a picture and the parameters for it, not a `Layer`. The
/// distinction earns its keep the moment something other than a clip needs
/// effects — the sequence-wide adjustment applies the same chain to the
/// *composited* image, which has no clip and no decoded frame behind it.
#[derive(Debug, Clone, Copy, Default)]
pub struct EffectParams {
    /// §45's blur amount, 0–100.
    pub blur: f32,
    /// Sharpening, 0–100 (`crate::sharpen`).
    pub sharpen: f32,
    /// A colour lookup table and its strength (`crate::lut`).
    pub lut: Option<bettercut_timeline::ClipLut>,
    /// RGB split and glitch, each 0–100 (`crate::glitch`).
    pub rgb_split: f32,
    pub glitch: f32,
    /// Pixelate, 0–100 (`crate::glitch`): the picture in square blocks.
    pub pixelate: f32,
    /// Zoom blur, 0–100 (`crate::glitch`): streaks out from the middle.
    pub zoom_blur: f32,
    /// Tilt-shift (`crate::blur`): a sharp band this tall, 0 for none, at
    /// this centre, with the blur growing away from it.
    pub tilt_band: f32,
    pub tilt_centre: f32,
    /// Glow, 0–100 (`crate::glitch`): bright parts bleed light.
    pub glow: f32,
    /// Old film, 0–100 (`crate::glitch`): scratches, dust, flicker.
    pub old_film: f32,
    /// A reflection (`crate::reflect`).
    pub reflection: bettercut_timeline::Reflection,
    /// The frame being drawn, for effects that change every frame.
    pub seed: u32,
}

impl EffectParams {
    /// True when no node would do anything, so the caller can skip the chain
    /// and the intermediate textures it would need.
    pub fn is_identity(self) -> bool {
        self.blur <= 0.0
            && self.sharpen <= 0.0
            && self.lut.is_none_or(|lut| lut.clamped().strength <= 0.0)
            && self.rgb_split <= 0.0
            && self.glitch <= 0.0
            && self.pixelate <= 0.0
            && self.zoom_blur <= 0.0
            && self.glow <= 0.0
            && self.old_film <= 0.0
            && self.reflection == bettercut_timeline::Reflection::None
    }
}

/// What a node reads: either the decoded frame or the previous node's output.
#[derive(Clone, Copy)]
pub struct EffectInput<'a> {
    pub bind_group: &'a wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
}

/// Intermediate textures, reused between frames.
pub struct TargetPool {
    format: wgpu::TextureFormat,
    free: HashMap<(u32, u32), Vec<EffectTexture>>,
    /// Used this frame or last, and not yet safe to hand out again.
    in_flight: Vec<EffectTexture>,
}

impl TargetPool {
    pub fn new(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            free: HashMap::new(),
            in_flight: Vec::new(),
        }
    }

    /// Release the previous frame's intermediates. Called once per frame,
    /// before anything is recorded.
    pub fn recycle(&mut self) {
        for texture in self.in_flight.drain(..) {
            self.free
                .entry((texture.width, texture.height))
                .or_default()
                .push(texture);
        }
    }

    /// Hand a texture back. It stays out of circulation until the next
    /// `recycle`, because the GPU has not finished with it yet.
    pub fn retire(&mut self, texture: EffectTexture) {
        self.in_flight.push(texture);
    }

    /// How many textures are held, for the memory diagnostics §73 asks for.
    pub fn len(&self) -> usize {
        self.in_flight.len() + self.free.values().map(Vec::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn acquire(
        &mut self,
        device: &wgpu::Device,
        texture_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
    ) -> EffectTexture {
        if let Some(pooled) = self
            .free
            .get_mut(&(width, height))
            .and_then(std::vec::Vec::pop)
        {
            return pooled;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effect intermediate"),
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
            // it writes — an effect that averages texels averages light, which
            // is what averaging is supposed to mean.
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect intermediate bind group"),
            layout: texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        EffectTexture {
            view,
            bind_group,
            width,
            height,
        }
    }
}

/// Everything a node is allowed to reach while recording.
///
/// Deliberately narrow. A node gets the device, the queue, a way to ask for a
/// scratch texture, and the quality tier — and nothing else. It cannot see the
/// output resolution, the media source, or the sink, which are the other three
/// axes §46 says preview and export differ on: a node that read them could make
/// the two configurations render differently, which is the exact failure §46
/// exists to prevent.
pub struct EffectContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    texture_layout: &'a wgpu::BindGroupLayout,
    tier: QualityTier,
    targets: &'a mut TargetPool,
}

impl<'a> EffectContext<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        texture_layout: &'a wgpu::BindGroupLayout,
        tier: QualityTier,
        targets: &'a mut TargetPool,
    ) -> Self {
        Self {
            device,
            queue,
            texture_layout,
            tier,
            targets,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        self.queue
    }

    /// §45's quality tier. The only thing a node may vary between preview and
    /// export, and it must change *cost*, never the picture (§46).
    pub fn tier(&self) -> QualityTier {
        self.tier
    }

    /// A scratch texture to render into.
    pub fn acquire(&mut self, width: u32, height: u32) -> EffectTexture {
        self.targets
            .acquire(self.device, self.texture_layout, width, height)
    }

    /// Give back a texture the node is finished with mid-chain.
    pub fn retire(&mut self, texture: EffectTexture) {
        self.targets.retire(texture);
    }
}

/// One effect in a layer's chain.
///
/// Implementors record their own passes and hand back what they wrote. The
/// compositor does not know what any of them do, which is the point: adding an
/// effect is writing one of these, not editing the compositor.
pub trait EffectNode {
    /// For pass labels and diagnostics.
    fn name(&self) -> &'static str;

    /// Reserve whatever scales with the number of layers, once per frame.
    ///
    /// Nodes that write per-pass uniforms need a distinct slice per layer —
    /// every layer's parameters have to survive until submission, so they
    /// cannot share one slot. Growing a buffer mid-chain would orphan the
    /// bind group the already-recorded passes are holding, so the size is
    /// settled here, before anything is recorded.
    fn begin_frame(&mut self, _device: &wgpu::Device, _layers: usize) {}

    /// Record this node's passes for one layer.
    ///
    /// `None` means the node does nothing for this layer — an effect at its
    /// default costs no passes, no textures, and no time. That is the normal
    /// case: most clips have no effects at all.
    fn apply(
        &mut self,
        ctx: &mut EffectContext<'_>,
        encoder: &mut wgpu::CommandEncoder,
        params: EffectParams,
        input: EffectInput<'_>,
    ) -> Result<Option<EffectTexture>, RenderError>;
}

/// Run one layer through every node, in order.
///
/// Returns the last node's output, or `None` when no node did anything — in
/// which case the composite pass draws the decoded frame directly and the whole
/// graph cost one comparison per node.
pub fn run_chain(
    nodes: &mut [Box<dyn EffectNode>],
    ctx: &mut EffectContext<'_>,
    encoder: &mut wgpu::CommandEncoder,
    params: EffectParams,
    source: EffectInput<'_>,
) -> Result<Option<EffectTexture>, RenderError> {
    let mut current: Option<EffectTexture> = None;

    for node in nodes.iter_mut() {
        let input = current.as_ref().map_or(source, EffectTexture::as_input);
        if let Some(output) = node.apply(ctx, encoder, params, input)?
            && let Some(spent) = current.replace(output)
        {
            // The node that produced it has been read; it is not needed for the
            // rest of the chain. Retired rather than reused immediately,
            // because the GPU has not run any of this yet.
            ctx.retire(spent);
        }
    }

    Ok(current)
}

// Everything here holds GPU handles, which have no constructor outside wgpu, so
// there is nothing to unit-test that would not be a mock asserting itself. The
// behaviour that matters — a declining node costing no textures, and
// intermediates being reused rather than reallocated every frame — is asserted
// against a real device in `tests/effect_graph.rs`.
