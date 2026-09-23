// Separable Gaussian blur (§45: "Blur -> Medium").
//
// Run twice per blurred layer, horizontally then vertically. A 2D Gaussian is
// separable, so two 1D passes of N taps produce the same picture as one 2D
// pass of N*N taps: at N=33 that is 66 samples per pixel instead of 1089.
// Nothing else here is worth optimising until that one is taken.
//
// This runs on the layer's *source* texture, before the composite pass
// transforms it, so a blurred clip still scales and rotates normally.

struct BlurParams {
    // Distance between neighbouring texels in UV space: (1/width, 0) for the
    // horizontal pass, (0, 1/height) for the vertical one. The direction of
    // the pass is carried entirely by this vector, which is why one shader
    // covers both.
    step: vec2<f32>,
    // Standard deviation, in texels of the texture being sampled.
    sigma: f32,
    // Texels between taps. 1.0 samples every texel; above that the kernel is
    // sparse, which is how a wide blur stays within a tap budget (§45).
    stride: f32,
    // Taps either side of the centre. The kernel is 2 * half_taps + 1 wide.
    half_taps: i32,
    // The tilt-shift band, in texture v: its centre, its half height (zero
    // for no band) and how far past it the blur fades in. In what was the
    // struct's padding; the Rust writer agrees byte for byte.
    band_centre: f32,
    band_half: f32,
    band_soft: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> params: BlurParams;
@group(1) @binding(0) var source: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// A full-screen quad. Unlike the composite pass there is no transform here:
// blur happens in the source texture's own space, and the geometry is applied
// afterwards.
@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let corner = corners[index];
    var out: VsOut;
    // Texture v runs top-down, clip-space y runs bottom-up.
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let inv_two_sigma_squared = 1.0 / (2.0 * params.sigma * params.sigma);
    // Tilt-shift: inside the band every tap lands on the pixel itself, so it
    // is sharp; past the soft edge the taps reach their full distance. Both
    // passes scale the same way, so the blur stays round.
    var reach = 1.0;
    if params.band_half > 0.0 {
        let away = abs(in.uv.y - params.band_centre);
        reach = smoothstep(params.band_half, params.band_half + max(params.band_soft, 0.0001), away);
    }

    var sum = vec4<f32>(0.0);
    var weight_total = 0.0;

    for (var i = -params.half_taps; i <= params.half_taps; i++) {
        let distance = f32(i) * params.stride;
        let weight = exp(-distance * distance * inv_two_sigma_squared);
        // `textureSampleLevel` rather than `textureSample`: this is inside a
        // loop, and sampling with implicit derivatives carries a uniform
        // control flow requirement that a loop bound read from a uniform
        // satisfies only by analysis. There are no mipmaps here anyway, so
        // asking for level 0 explicitly costs nothing and removes the question.
        sum += textureSampleLevel(source, samp, in.uv + params.step * distance * reach, 0.0) * weight;
        weight_total += weight;
    }

    // Normalised by the weights actually used, not by the analytic integral of
    // the Gaussian. The kernel here is both truncated and, at large radii,
    // sparse, so its weights do not sum to 1. Dividing by the true total keeps
    // average brightness exact; using the analytic constant would darken every
    // blurred clip, and darken it more the wider the blur.
    return sum / weight_total;
}
