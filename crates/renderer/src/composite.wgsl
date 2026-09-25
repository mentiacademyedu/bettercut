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
    // Rounded corners (0–1, 1 a half circle on the shorter side) at 160 and
    // the border's width in shorter sides at 164; its colour, linear, at
    // 168–176; the picture's drawn width over height at 180. The struct
    // rounds to 192.
    border_radius: f32,
    border_width: f32,
    border_r: f32,
    border_g: f32,
    border_b: f32,
    picture_aspect: f32,
    // A drop shadow's own layer (1.0 at 188): the picture's rounded shape,
    // grown by its softness (at 184, in shorter sides) and drawn in the colour
    // the border slots carry, instead of the picture.
    shadow_softness: f32,
    shadow_mode: f32,
    // Cinematic bars, as the share of the frame's height each covers (at 192;
    // the struct rounds to 208). Only ever set on the master draw.
    bars: f32,
    // Vibrance, -1..1, at 196 — inside the padding the struct already had, so
    // it is still 208 bytes.
    vibrance: f32,
    // §45's corner pin: each corner of the quad moved, in clip-space units, in
    // the order the unit quad names them — (0,0), (1,0), (1,1), (0,1). A
    // `vec2` aligns to 8, so these start at 200, right after `vibrance`, and
    // the struct runs to 232 — rounding to 240.
    //
    // Zero for every layer that is still a rectangle, which is almost all of
    // them: the vertex shader takes the plain path when they are.
    corner_a: vec2<f32>,
    corner_b: vec2<f32>,
    corner_c: vec2<f32>,
    corner_d: vec2<f32>,
    // The colour wheels, nine scalars from 232: lift, gamma, gain, each red,
    // green, blue. Scalars rather than three `vec3`s, which would each align
    // to 16 and waste a quarter of their space; the struct ends at 268 and
    // rounds to 272.
    lift_r: f32,
    lift_g: f32,
    lift_b: f32,
    gamma_r: f32,
    gamma_g: f32,
    gamma_b: f32,
    gain_r: f32,
    gain_g: f32,
    gain_b: f32,
    // The secondary, five scalars from 268: the pick (hue, width) and the
    // three shifts. The struct ends at 288, a multiple of 16 already.
    pick_hue: f32,
    pick_width: f32,
    pick_hue_shift: f32,
    pick_saturation: f32,
    pick_luminance: f32,
    // Lens correction, one scalar at 288.
    lens: f32,
    // Posterise levels at 292, zero for none.
    posterise: f32,
    // The luma key at 296: threshold, softness, and which side is kept — 1
    // the bright, 2 the dark, 0 for no key. The struct ends at 308 and
    // rounds to 320.
    luma_threshold: f32,
    luma_softness: f32,
    luma_mode: f32,
    // Smooth skin at 308, 0-1.
    smooth_skin: f32,
}

// The weight each corner of a pinned quad carries, so the texture follows the
// shape rather than being stretched across two triangles.
//
// Where the diagonals cross divides each of them in some ratio; those ratios
// *are* the perspective. A quad whose diagonals bisect each other is a
// parallelogram and every weight comes out 1, which is the flat case.
fn corner_weights(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>) -> vec4<f32> {
    let ac = c - a;
    let bd = d - b;
    let ab = b - a;
    let denom = ac.x * bd.y - ac.y * bd.x;
    if abs(denom) < 0.000001 {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }
    // How far along each diagonal the crossing point is.
    let s = (ab.x * bd.y - ab.y * bd.x) / denom;
    let t = (ab.x * ac.y - ab.y * ac.x) / denom;
    if s <= 0.0 || s >= 1.0 || t <= 0.0 || t >= 1.0 {
        // A quad folded over itself has no crossing inside it; drawing it flat
        // is wrong but finite, which is what a dragged corner needs while it
        // is passing through.
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }
    return vec4<f32>(1.0 / (1.0 - s), 1.0 / (1.0 - t), 1.0 / s, 1.0 / t);
}

// The picture's size in units of its shorter side.
fn picture_size(layer: Layer) -> vec2<f32> {
    let aspect = max(layer.picture_aspect, 0.0001);
    return select(vec2<f32>(1.0, 1.0 / aspect), vec2<f32>(aspect, 1.0), aspect >= 1.0);
}

// How far `local` is from the picture's rounded edge, in units of its shorter
// side: negative inside, zero on the edge. A rounded-box distance, so one
// rule gives square corners at radius zero and a circle at one.
fn picture_edge(local: vec2<f32>, layer: Layer) -> f32 {
    let size = picture_size(layer);
    let p = (local - vec2<f32>(0.5)) * size;
    let radius = clamp(layer.border_radius, 0.0, 1.0) * 0.5;
    let q = abs(p) - size * 0.5 + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
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
const MASK_STAR: u32 = 4u;
const MASK_HEART: u32 = 5u;
const MASK_MIRROR: u32 = 6u;

// Distance to a five-pointed star of outer radius 1, point up, in y-down
// units (after Inigo Quilez's star distance). Negative inside.
fn star_distance(q: vec2<f32>) -> f32 {
    let k1 = vec2<f32>(0.809016994375, -0.587785252292);
    let k2 = vec2<f32>(-k1.x, k1.y);
    // Flipped to y-up, where the formula's point is at the top.
    var p = vec2<f32>(abs(q.x), -q.y);
    p = p - 2.0 * max(dot(k1, p), 0.0) * k1;
    p = p - 2.0 * max(dot(k2, p), 0.0) * k2;
    p.x = abs(p.x);
    p.y = p.y - 1.0;
    let inner = 0.45;
    let ba = inner * vec2<f32>(-k1.y, k1.x) - vec2<f32>(0.0, 1.0);
    let h = clamp(dot(p, ba) / dot(ba, ba), 0.0, 1.0);
    return length(p - ba * h) * sign(p.y * ba.x - p.x * ba.y);
}

// Distance to a heart filling the -1..1 box, point down, in y-down units
// (after Inigo Quilez's heart distance, which spans about 1.2 across and 1.05
// up from its point). Negative inside.
fn heart_distance(q: vec2<f32>) -> f32 {
    let scale = 0.58;
    var p = vec2<f32>(abs(q.x) * scale, (1.0 - q.y) * 0.525);
    var d = 0.0;
    if p.y + p.x > 1.0 {
        let c = p - vec2<f32>(0.25, 0.75);
        d = sqrt(dot(c, c)) - sqrt(2.0) / 4.0;
    } else {
        let a = p - vec2<f32>(0.0, 1.0);
        let b = p - 0.5 * max(p.x + p.y, 0.0);
        d = sqrt(min(dot(a, a), dot(b, b))) * sign(p.x - p.y);
    }
    return d / scale;
}

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

    // Vibrance: the same mix again, but by how little colour a pixel has
    // already. A grey pixel gets the whole push, one that is already vivid
    // gets almost none — which is what keeps skin from going orange while a
    // flat sky comes back.
    if layer.vibrance != 0.0 {
        let after = dot(out, LUMA);
        let spread = max(out.r, max(out.g, out.b)) - min(out.r, min(out.g, out.b));
        let room = 1.0 - clamp(spread, 0.0, 1.0);
        out = mix(vec3<f32>(after), out, 1.0 + layer.vibrance * room);
    }

    // The wheels, last: they are a grade, built on the correction above. Lift
    // raises the darks and fades out towards white, so a shadow's colour is
    // not painted onto a highlight; gain scales the brights; gamma bends the
    // middle with a per-channel power — plus one is a root, minus one a
    // square, so the two directions are the same distance from doing nothing.
    let lift = vec3<f32>(layer.lift_r, layer.lift_g, layer.lift_b);
    let gamma = vec3<f32>(layer.gamma_r, layer.gamma_g, layer.gamma_b);
    let gain = vec3<f32>(layer.gain_r, layer.gain_g, layer.gain_b);
    if (any(lift != vec3<f32>(0.0)) || any(gamma != vec3<f32>(0.0)) || any(gain != vec3<f32>(0.0))) {
        let held = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
        out = out * (vec3<f32>(1.0) + gain) + lift * (vec3<f32>(1.0) - held);
        out = pow(max(out, vec3<f32>(0.0)), exp2(-gamma));
    }

    // The secondary, after the wheels: one range of hue picked out and
    // moved. Everything about it happens in hue, saturation and value, and
    // the weight is what keeps it a secondary — full inside the pick, fading
    // to nothing at its edge, and nothing at all for grey, which has no hue
    // to be picked by however wide the pick.
    if (layer.pick_hue_shift != 0.0 || layer.pick_saturation != 0.0 || layer.pick_luminance != 0.0) {
        let hsv = rgb_to_hsv(max(out, vec3<f32>(0.0)));
        var away = abs(hsv.x - layer.pick_hue);
        away = min(away, 1.0 - away);
        let inside = 1.0 - smoothstep(layer.pick_width * 0.5, layer.pick_width, away);
        let weight = inside * clamp(hsv.y * 4.0, 0.0, 1.0);
        let hue = fract(hsv.x + layer.pick_hue_shift * 0.5 * weight + 1.0);
        let sat = clamp(hsv.y * (1.0 + layer.pick_saturation * weight), 0.0, 1.0);
        let val = max(hsv.z * (1.0 + layer.pick_luminance * weight), 0.0);
        out = hsv_to_rgb(vec3<f32>(hue, sat, val));
    }

    // Contrast and saturation can both push a channel negative, which becomes
    // NaN once the sRGB encode takes a root of it.
    out = max(out, vec3<f32>(0.0));

    // Posterise: each channel snapped to the nearest of `levels` steps between
    // black and white, after the grade so the steps land on the graded
    // picture. Anything brighter than white is left where it is.
    if (layer.posterise >= 2.0) {
        let steps = layer.posterise - 1.0;
        let held = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
        out = mix(round(held * steps) / steps, out, step(vec3<f32>(1.0), out));
    }
    return out;
}

// Hue (a turn, 0 red), saturation and value of a linear colour. Value may run
// past one for a bright pixel; that is kept, so a secondary never dims what it
// only meant to tint.
fn rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let high = max(rgb.r, max(rgb.g, rgb.b));
    let low = min(rgb.r, min(rgb.g, rgb.b));
    let spread = high - low;
    var hue = 0.0;
    if (spread > 1e-6) {
        if (high == rgb.r) {
            hue = (rgb.g - rgb.b) / spread;
        } else if (high == rgb.g) {
            hue = 2.0 + (rgb.b - rgb.r) / spread;
        } else {
            hue = 4.0 + (rgb.r - rgb.g) / spread;
        }
        hue = fract(hue / 6.0 + 1.0);
    }
    let sat = select(0.0, spread / high, high > 1e-6);
    return vec3<f32>(hue, sat, high);
}

// The way back from `rgb_to_hsv`.
fn hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let h = hsv.x * 6.0;
    let c = hsv.z * hsv.y;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    let m = hsv.z - c;
    var rgb = vec3<f32>(0.0);
    if (h < 1.0) {
        rgb = vec3<f32>(c, x, 0.0);
    } else if (h < 2.0) {
        rgb = vec3<f32>(x, c, 0.0);
    } else if (h < 3.0) {
        rgb = vec3<f32>(0.0, c, x);
    } else if (h < 4.0) {
        rgb = vec3<f32>(0.0, x, c);
    } else if (h < 5.0) {
        rgb = vec3<f32>(x, 0.0, c);
    } else {
        rgb = vec3<f32>(c, 0.0, x);
    }
    return rgb + vec3<f32>(m);
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

// Smooth skin: a colour-aware blur, where the pixel is skin-coloured. Two
// rings of eight samples; each counts as much as its colour is close to the
// centre's, so an edge — an eye, a lip, a strand of hair — keeps its line
// while the small differences across skin average out. The radius grows
// with the amount and with the picture's own height.
fn smooth_skin(rgb: vec3<f32>, uv: vec2<f32>, amount: f32) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(source, 0));
    let texel = 1.0 / size;
    // Half-pixel steps, so the taps fall between pixels and each one is
    // itself an average: whole even steps would land on the same
    // pixel pattern and average nothing.
    let radius = (1.5 + 2.0 * amount) * max(size.y / 720.0, 1.0);
    var total = rgb;
    var weight = 1.0;
    for (var i = 0; i < 16; i = i + 1) {
        let angle = f32(i % 8) * 0.785398;
        let reach = select(radius, radius * 2.0, i >= 8);
        let offset = vec2<f32>(cos(angle), sin(angle)) * reach * texel;
        let s = textureSampleLevel(source, samp, uv + offset, 0.0).rgb;
        let d = s - rgb;
        // Blotches and pores differ from the skin around them by a few
        // percent and count almost fully; an eye or a strand of hair differs
        // by a third or more and counts for nothing.
        let w = exp(-dot(d, d) * 30.0);
        total = total + s * w;
        weight = weight + w;
    }
    let smoothed = total / weight;
    // Skin, judged in gamma-like terms: red over green over blue, warm but
    // not saturated, neither black nor blown out.
    let g = sqrt(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    let warm = smoothstep(0.04, 0.12, g.r - g.b) * (1.0 - smoothstep(0.45, 0.6, g.r - g.b));
    let order = smoothstep(-0.02, 0.03, g.r - g.g) * smoothstep(-0.02, 0.03, g.g - g.b);
    let lit = smoothstep(0.15, 0.3, g.r) * (1.0 - smoothstep(0.97, 1.0, g.b));
    let skin = warm * order * lit;
    return mix(rgb, smoothed, clamp(amount * skin, 0.0, 1.0));
}

// How much of a pixel survives the luma key: what is brighter than the
// threshold (mode 1) or darker (mode 2), fading over the softness. Judged on
// perceptual brightness, so the threshold means what the eye sees.
fn luma_key_alpha(rgb: vec3<f32>, layer: Layer) -> f32 {
    let level = sqrt(clamp(dot(rgb, LUMA), 0.0, 1.0));
    let soft = max(layer.luma_softness, 0.0001);
    let bright = smoothstep(layer.luma_threshold - soft, layer.luma_threshold + soft, level);
    return select(1.0 - bright, bright, layer.luma_mode < 1.5);
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
    } else if layer.mask_shape == MASK_MIRROR {
        // A band across the centre, its height the size's second half: both
        // edges straight and both feathered.
        distance = abs(local.y) - max(layer.mask_size.y, 0.0001);
    } else if layer.mask_shape == MASK_RECTANGLE {
        let half = max(layer.mask_size, vec2<f32>(0.0001, 0.0001));
        // The larger of the two axis overshoots: inside only where both are.
        let over = abs(local) - half;
        distance = max(over.x, over.y);
    } else if layer.mask_shape == MASK_STAR {
        let half = max(layer.mask_size, vec2<f32>(0.0001, 0.0001));
        distance = star_distance(local / half) * min(half.x, half.y);
    } else if layer.mask_shape == MASK_HEART {
        let half = max(layer.mask_size, vec2<f32>(0.0001, 0.0001));
        distance = heart_distance(local / half) * min(half.x, half.y);
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
    /// Where this pixel is in the *visible* picture, 0..1 across whatever the
    /// crop left, times its corner's weight — and that weight as `z`.
    ///
    /// Divided back out in the fragment shader, which is what makes a pinned
    /// quad's texture follow its shape (`corner_weights`). Every other layer
    /// carries a weight of one, so the divide changes nothing.
    ///
    /// The mask and the border read this rather than the sampling position, so
    /// a mask drawn on a cropped shot stays where it was drawn instead of
    /// shrinking into the corner along with the window.
    @location(0) uvw: vec3<f32>,
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

    // Which of the quad's four corners this vertex is, in the order the pin
    // names them.
    var which = array<u32, 6>(0u, 1u, 3u, 1u, 2u, 3u);
    var corner = corners[index];
    // A shadow's edge fades outwards past the picture, so its quad is grown by
    // the softness on every side; `local` then runs past 0..1 there, which
    // `picture_edge` measures as outside.
    if layer.shadow_mode > 0.5 {
        let size = picture_size(layer);
        let grow = (size + vec2<f32>(2.0 * max(layer.shadow_softness, 0.0))) / size;
        corner = (corner - vec2<f32>(0.5)) * grow + vec2<f32>(0.5);
    }
    let pinned = any(layer.corner_a != vec2<f32>(0.0))
        || any(layer.corner_b != vec2<f32>(0.0))
        || any(layer.corner_c != vec2<f32>(0.0))
        || any(layer.corner_d != vec2<f32>(0.0));

    var out: VsOut;
    if pinned {
        // §45's corner pin: the quad's own four corners, each moved.
        let a = (layer.transform * vec3<f32>(0.0, 0.0, 1.0)).xy + layer.corner_a;
        let b = (layer.transform * vec3<f32>(1.0, 0.0, 1.0)).xy + layer.corner_b;
        let c = (layer.transform * vec3<f32>(1.0, 1.0, 1.0)).xy + layer.corner_c;
        let d = (layer.transform * vec3<f32>(0.0, 1.0, 1.0)).xy + layer.corner_d;
        var quad = array<vec2<f32>, 4>(a, b, c, d);
        var uvs = array<vec2<f32>, 4>(
            vec2<f32>(0.0, 0.0),
            vec2<f32>(1.0, 0.0),
            vec2<f32>(1.0, 1.0),
            vec2<f32>(0.0, 1.0),
        );
        let weights = corner_weights(a, b, c, d);
        let at = which[index];
        let w = weights[at];
        out.position = vec4<f32>(quad[at], 0.0, 1.0);
        out.uvw = vec3<f32>(uvs[at] * w, w);
        return out;
    }

    let placed = layer.transform * vec3<f32>(corner, 1.0);
    out.position = vec4<f32>(placed.xy, 0.0, 1.0);
    // Texture v runs top-down while clip space y runs bottom-up.
    out.uvw = vec3<f32>(corner.x, corner.y, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // The corner weight divided back out: for every layer but a pinned one it
    // is 1 and this is the position the vertex shader already had.
    var local = in.uvw.xy / max(in.uvw.z, 0.000001);
    // The lens: where this pixel reads from is pushed out or pulled in by
    // how far it is from the middle, squared — the shape a real lens bows
    // straight lines into. Measured in square units of the picture's own
    // height so the bowing is round rather than stretched with the frame.
    // Past the picture's edge the read lands outside, and the edge test
    // below on the same `local` makes it see-through rather than smeared.
    var uncovered = false;
    if (layer.lens != 0.0) {
        let stretch = vec2<f32>(layer.picture_aspect, 1.0);
        let away = (local - vec2<f32>(0.5)) * stretch;
        let bowed = away * (1.0 + layer.lens * dot(away, away));
        local = vec2<f32>(0.5) + bowed / stretch;
        // Reading past the picture is reading nothing: the sampler would
        // otherwise smear the last pixel outwards, which is a stretch, not a
        // correction.
        uncovered = any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0));
    }
    // §22's crop: the quad is unchanged and the window it reads through is not.
    let uv = layer.crop_origin + local * layer.crop_size;
    let texel = textureSample(source, samp, uv);
    // Up here, outside every branch: a derivative is only defined in uniform
    // control flow. One pixel's worth of distance, for smooth edges.
    let edge = picture_edge(local, layer);
    let pixel = max(fwidth(edge), 0.00001);

    // A shadow is only its shape: solid inside, fading across its softness
    // centred on the picture's edge.
    if layer.shadow_mode > 0.5 {
        let soft = max(layer.shadow_softness, pixel);
        let shade = layer.opacity * (1.0 - smoothstep(-soft, soft, edge));
        return vec4<f32>(vec3<f32>(layer.border_r, layer.border_g, layer.border_b) * shade, shade);
    }
    var rgb = texel.rgb;
    var alpha = texel.a * layer.opacity;
    // Before the keys and the grade: skin is judged on what the camera saw.
    if layer.smooth_skin > 0.0 {
        rgb = smooth_skin(rgb, uv, layer.smooth_skin);
    }
    if (uncovered) {
        alpha = 0.0;
    }

    // Keyed before grading, because the key is about what the *camera* saw. A
    // grade that shifted the screen's colour would otherwise have to be undone
    // in the head to choose a key that works.
    if layer.tolerance > 0.0 || layer.softness > 0.0 {
        let kept = key_alpha(rgb, layer);
        rgb = suppress_spill(rgb, layer, kept);
        alpha = alpha * kept;
    }
    // And by brightness, for the same reason, on the same camera picture.
    if layer.luma_mode > 0.5 {
        alpha = alpha * luma_key_alpha(rgb, layer);
    }


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
    var colour = add_grain(
        adjust_colour(rgb, layer) * vignette_keep(local, layer.vignette),
        in.position.xy,
        layer,
    );

    // The border is laid on the finished picture — a frame is not graded
    // with the shot — and is solid even where a key took the picture away.
    // Then everything outside the rounded corners is dropped.
    if layer.border_radius > 0.0 || layer.border_width > 0.0 {
        if layer.border_width > 0.0 {
            let ring = smoothstep(
                -layer.border_width - pixel * 0.5,
                -layer.border_width + pixel * 0.5,
                edge,
            );
            colour = mix(colour, vec3<f32>(layer.border_r, layer.border_g, layer.border_b), ring);
            alpha = mix(alpha, layer.opacity, ring);
        }
        alpha = alpha * (1.0 - smoothstep(-pixel * 0.5, pixel * 0.5, edge));
    }

    // The mask is geometry, not colour: it decides what of this layer exists
    // at all, so it multiplies the alpha after everything else has decided
    // what the pixel looks like.
    alpha = alpha * mask_alpha(local, layer);

    // Cinematic bars: solid black over whatever the frame holds there.
    if layer.bars > 0.0 && (local.y < layer.bars || local.y > 1.0 - layer.bars) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(colour * alpha, alpha);
}
