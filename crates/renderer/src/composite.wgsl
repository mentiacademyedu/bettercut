// The compositor (§22).
//
// One layer per draw, back to front, alpha-blended into the target. Track
// order is compositing order (§22), so the caller submits them in that order.
//
// The frames arriving here have already been converted into the working space
// (§21a.1: sRGB-encoded 8-bit RGBA). §21a.2 requires that conversion to happen
// exactly once, at the upload boundary, and nothing here re-examines source
// colour metadata — by the time a pixel reaches this shader its provenance is
// no longer a question anyone should be asking.

// Layout matters: this must be exactly 64 bytes to match `UNIFORM_SIZE` and
// the bytes `layer_uniform` writes.
//
// `mat3x3<f32>` occupies 48 bytes (three 16-byte-aligned columns), and
// `opacity` follows at offset 48, rounding the struct to 64. Adding an explicit
// `vec3` pad would *not* help — a vec3 aligns to 16, so it would land at offset
// 64 and push the struct to 80, which is a pipeline validation error rather
// than a silent mismatch.
// The three colour values ride in the padding `opacity` leaves behind: it sits
// at offset 48, so 52, 56 and 60 were already reserved and the struct is still
// exactly 64 bytes.
struct Layer {
    // Column-major 2x2 plus translation, mapping the unit quad into
    // normalized device coordinates.
    transform: mat3x3<f32>,
    opacity: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    // The chroma key (§45). A `vec3` aligns to 16, so it starts at 64 and its
    // own tail padding carries `tolerance` at 76 — the struct then rounds to
    // 96, which is `UNIFORM_SIZE`.
    //
    // Already converted to linear light on the way in: the source texture is
    // sRGB-aware, so what `textureSample` hands back is linear, and comparing
    // it against an sRGB-encoded key would key the wrong colour (§21a.1).
    key_color: vec3<f32>,
    // Zero means no key at all — the whole thing is skipped.
    tolerance: f32,
    softness: f32,
    spill: f32,
    // The mask (§45). 0 is none, and then linear, rectangle, ellipse in the
    // order `MaskShape` declares them. A `vec2` aligns to 8, so the two that
    // follow sit at 96 and 104 and the struct rounds to 128.
    mask_shape: u32,
    mask_feather: f32,
    mask_center: vec2<f32>,
    mask_size: vec2<f32>,
    mask_rotation: f32,
    // 1.0 keeps the outside instead. A float rather than a bool because a
    // WGSL bool has no defined size in a uniform.
    mask_invert: f32,
    // White balance (§45). Both -1..1, both zero for no change. They sit in the
    // tail the struct already had: `mask_invert` ends at 120 and a 16-aligned
    // struct rounds to 128 regardless, so these two cost nothing.
    temperature: f32,
    tint: f32,
    // §22's crop, as where in the source the visible rectangle starts and how
    // big it is — both in 0..1 source units. `vec2` aligns to 8, so these sit
    // at 128 and 136 and the struct grows to 144.
    //
    // Pre-resolved to an origin and a size rather than four edges, because that
    // is what sampling needs and the shader should not be re-deriving it per
    // pixel.
    crop_origin: vec2<f32>,
    crop_size: vec2<f32>,
    // How much the edges are darkened, 0–1. Only ever set on a full-frame draw
    // — the master grade or an adjustment — where `local` runs across the frame
    // itself, so the darkening follows the frame's shape. At 144.
    vignette: f32,
    // Film grain, 0–1, full-frame draws only, at 148. Then the frame number it
    // is drawn for (152), so it moves every frame and the same frame always
    // gets the same grain, and the size of one grain in output pixels (156).
    // These fill the tail the struct already had: it still rounds to 160.
    grain: f32,
    grain_seed: f32,
    grain_cell: f32,
}

// How far the heaviest grain moves a mid-grey, in linear light.
const GRAIN_STRENGTH: f32 = 0.2;

// A fixed pseudo-random value in -0.5..0.5 for one grain cell on one frame.
// Integer hashing, so it is the same on every GPU: preview and export agree
// (§46) and a golden frame is reproducible.
fn grain_noise(cell: vec2<f32>, seed: f32) -> f32 {
    var v = vec3<u32>(u32(cell.x), u32(cell.y), u32(seed));
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return f32(v.x) / 4294967295.0 - 0.5;
}

// Grain laid on a finished colour. Strongest in the mid-tones and fading to
// nothing in pure black and pure white, as film grain does — noise added
// evenly would lift the blacks into a grey haze.
fn add_grain(colour: vec3<f32>, pixel: vec2<f32>, layer: Layer) -> vec3<f32> {
    if layer.grain <= 0.0 {
        return colour;
    }
    let cell = floor(pixel / max(layer.grain_cell, 1.0));
    let noise = grain_noise(cell, layer.grain_seed);
    let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
    let perceptual = sqrt(clamp(luma, 0.0, 1.0));
    let weight = 4.0 * perceptual * (1.0 - perceptual);
    return max(colour + vec3<f32>(noise * layer.grain * GRAIN_STRENGTH * weight), vec3<f32>(0.0));
}

// How far from the centre, as a share of the way to a corner, the darkening
// begins. Inside this the picture is untouched: a vignette frames a shot, and
// one that dimmed the middle would be dimming the subject.
const VIGNETTE_START: f32 = 0.35;

// How much of a pixel a vignette leaves. `local` is 0..1 across the quad; the
// distance is normalised so a corner is 1 whatever the frame's shape, which
// makes the falloff an ellipse that fits the frame rather than a circle that
// darkens a wide frame's sides before its top.
fn vignette_keep(local: vec2<f32>, amount: f32) -> f32 {
    if amount <= 0.0 {
        return 1.0;
    }
    let from_centre = length((local - vec2<f32>(0.5)) / vec2<f32>(0.5)) / sqrt(2.0);
    return 1.0 - amount * smoothstep(VIGNETTE_START, 1.0, from_centre);
}

const MASK_NONE: u32 = 0u;
const MASK_LINEAR: u32 = 1u;
const MASK_RECTANGLE: u32 = 2u;
const MASK_ELLIPSE: u32 = 3u;

// Linear mid-grey.
//
// Contrast expands around a pivot, and everything here is in **linear light**:
// the source is an sRGB-aware texture, so sampling already decoded it. Perceptual
// mid-grey is 0.5 *after* the sRGB curve, which is ~0.18 before it. Pivoting at
// 0.5 in linear would treat a bright grey as neutral and make every contrast
// increase darken the picture.
const MID_GREY: f32 = 0.18;

// Rec.709 luma weights, correct for linear RGB (§21a's working space).
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// Warm/cool and green/magenta, as a per-channel gain in linear light.
//
// A gain rather than an add, for the same reason brightness is: multiplying is
// what putting a filter in front of a lens does, and it leaves black black. An
// additive warm shift would lift the blacks orange.
//
// The coefficients are deliberately gentle. This is a correction, not an
// effect: a slider at its end should look like a different white balance, not
// like a colour cast laid over the picture.
fn white_balance(rgb: vec3<f32>, layer: Layer) -> vec3<f32> {
    let warm = layer.temperature;
    let magenta = layer.tint;
    return rgb * vec3<f32>(
        1.0 + 0.30 * warm + 0.15 * magenta,
        1.0 - 0.30 * magenta,
        1.0 - 0.30 * warm + 0.15 * magenta,
    );
}

fn adjust_colour(rgb: vec3<f32>, layer: Layer) -> vec3<f32> {
    // Brightness is a multiply rather than an add: in linear light that is an
    // exposure change, which is what a camera does and what looks natural. An
    // additive lift washes blacks out into fog.
    var out = rgb * layer.brightness;
    // Before the contrast, because a white balance is what the camera should
    // have done. Correcting after the contrast has already expanded around
    // mid-grey means correcting a cast the contrast has itself exaggerated.
    out = white_balance(out, layer);
    out = (out - MID_GREY) * layer.contrast + MID_GREY;

    let luma = dot(out, LUMA);
    out = mix(vec3<f32>(luma), out, layer.saturation);

    // Contrast and saturation can both push a channel negative, which becomes
    // NaN once the sRGB encode takes a root of it.
    return max(out, vec3<f32>(0.0));
}

// Where a colour sits on the chromaticity plane: its proportions, with its
// brightness divided out.
//
// Keying on plain RGB distance fails on exactly the pixels that matter — the
// shadowed folds of the screen are the same colour as the lit parts and a long
// way from them in RGB, so they survive as dark green fringes. Proportions do
// not move with the light.
fn chromaticity(rgb: vec3<f32>) -> vec2<f32> {
    let total = rgb.r + rgb.g + rgb.b;
    // Black has no proportions to speak of. Returning the key's own
    // chromaticity would key it; a third each keeps it neutral and opaque.
    if total < 0.0001 {
        return vec2<f32>(1.0 / 3.0, 1.0 / 3.0);
    }
    return rgb.rg / total;
}

// How much of a pixel survives the key: 1 keeps it, 0 removes it.
fn key_alpha(rgb: vec3<f32>, layer: Layer) -> f32 {
    let distance = length(chromaticity(rgb) - chromaticity(layer.key_color));
    // `smoothstep` needs a non-empty range; with no softness this is a hard
    // edge at the tolerance.
    let edge = layer.tolerance + max(layer.softness, 0.0001);
    return smoothstep(layer.tolerance, edge, distance);
}

// Take the screen's colour back out of what was kept.
//
// A green screen throws green onto everything in front of it, so a subject
// keyed against one has a green rim even where it is fully opaque. Pulling the
// keyed hue towards the pixel's own luma removes the cast without touching
// colours that are nothing like it.
fn suppress_spill(rgb: vec3<f32>, layer: Layer, alpha: f32) -> vec3<f32> {
    if layer.spill <= 0.0 {
        return rgb;
    }
    let distance = length(chromaticity(rgb) - chromaticity(layer.key_color));
    // Only near the key, and only where something was kept: how much cast a
    // pixel has is how close to the screen's colour it still is.
    let nearness = 1.0 - smoothstep(layer.tolerance, layer.tolerance * 3.0 + 0.05, distance);
    let luma = dot(rgb, LUMA);
    return mix(rgb, vec3<f32>(luma), nearness * layer.spill * alpha);
}

// How much of a pixel the mask keeps: 1 inside, 0 outside, feathered between.
//
// Everything is in the clip's own frame — `uv` runs 0..1 across the picture —
// so the mask stays over the part of the shot it was drawn on however the clip
// is afterwards moved or scaled. A mask in output coordinates would slide off
// its subject the moment the clip was nudged.
fn mask_alpha(uv: vec2<f32>, layer: Layer) -> f32 {
    if layer.mask_shape == MASK_NONE {
        return 1.0;
    }

    // Into the mask's own frame: relative to its centre, turned by its own
    // rotation. The picture is not square, but the mask's size is given in the
    // same 0..1 units as its position, so no aspect term belongs here — a
    // "square" mask on a 16:9 shot is a 16:9 rectangle, which is what dragging
    // its corners in those units means.
    let radians = radians(layer.mask_rotation);
    let c = cos(radians);
    let sn = sin(radians);
    let offset = uv - layer.mask_center;
    let local = vec2<f32>(
        offset.x * c + offset.y * sn,
        -offset.x * sn + offset.y * c,
    );

    // `distance` is 0 on the shape's edge, negative inside, positive outside —
    // a signed distance, so one feathering rule serves all three shapes.
    var distance = 0.0;
    if layer.mask_shape == MASK_LINEAR {
        // A straight edge through the centre: everything above it is kept.
        distance = local.y;
    } else if layer.mask_shape == MASK_RECTANGLE {
        let half = max(layer.mask_size, vec2<f32>(0.0001, 0.0001));
        // The larger of the two axis overshoots: inside only where both are.
        let over = abs(local) - half;
        distance = max(over.x, over.y);
    } else {
        let half = max(layer.mask_size, vec2<f32>(0.0001, 0.0001));
        // Scaled into a circle, so an ellipse needs no special case.
        distance = length(local / half) - 1.0;
    }

    let feather = max(layer.mask_feather, 0.0001);
    // Centred on the edge: half the softness falls either side of it, so
    // feathering does not also shrink the shape.
    let kept = 1.0 - smoothstep(-feather * 0.5, feather * 0.5, distance);
    return select(kept, 1.0 - kept, layer.mask_invert > 0.5);
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> layer: Layer;
@group(1) @binding(0) var source: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    /// Where to sample the source: inside the crop.
    @location(0) uv: vec2<f32>,
    /// Where this pixel is in the *visible* picture, 0..1 across whatever the
    /// crop left. The mask uses this rather than `uv`, so a mask drawn on a
    /// cropped shot stays where it was drawn instead of shrinking into the
    /// corner along with the sampling window.
    @location(1) local: vec2<f32>,
}

// A unit quad from the vertex index. No vertex buffer: for a full-screen-ish
// quad the index arithmetic is cheaper than binding one.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // Two triangles: (0,0) (1,0) (0,1) / (1,0) (1,1) (0,1)
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = corners[index];
    let placed = layer.transform * vec3<f32>(corner, 1.0);

    var out: VsOut;
    out.position = vec4<f32>(placed.xy, 0.0, 1.0);
    // Texture v runs top-down while clip space y runs bottom-up.
    out.local = vec2<f32>(corner.x, corner.y);
    // §22's crop is here, ahead of everything else the fragment does: the quad
    // is unchanged and the window it reads through is not.
    out.uv = layer.crop_origin + out.local * layer.crop_size;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(source, samp, in.uv);
    var rgb = texel.rgb;
    var alpha = texel.a * layer.opacity;

    // Keyed before grading, because the key is about what the *camera* saw. A
    // grade that shifted the screen's colour would otherwise have to be undone
    // in the head to choose a key that works.
    if layer.tolerance > 0.0 || layer.softness > 0.0 {
        let kept = key_alpha(rgb, layer);
        rgb = suppress_spill(rgb, layer, kept);
        alpha = alpha * kept;
    }

    // The mask is geometry, not colour: it decides what of this layer exists
    // at all, so it multiplies the alpha after everything else has decided
    // what the pixel looks like.
    alpha = alpha * mask_alpha(in.local, layer);

    // **Premultiplied**: the colour is scaled by its own alpha before it
    // leaves the shader.
    //
    // Straight alpha would need `SrcAlpha, OneMinusSrcAlpha` blending, which
    // only expresses alpha-over. §22's other three modes — screen, multiply,
    // add — are each a different pair of blend factors, and every one of them
    // needs the source already weighted by its alpha or a half-transparent
    // overlay would screen at full strength. One shader, four blend states.
    // In linear light, like every other grade here: a gain, so black stays
    // black and the darkening looks like light falling off rather than a grey
    // wash laid over the corners.
    let colour = add_grain(
        adjust_colour(rgb, layer) * vignette_keep(in.local, layer.vignette),
        in.position.xy,
        layer,
    );
    return vec4<f32>(colour * alpha, alpha);
}
