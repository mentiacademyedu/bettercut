// Sharpen: an unsharp mask (§45's cheap band, run as an effect-graph node).
//
// What is sharp about a picture is how much each pixel differs from the pixels
// around it. Taking the local average away from a pixel leaves exactly that
// difference — the detail — and adding it back at more than full strength makes
// the edges stand further out. Nothing is invented: a flat patch has no detail
// and comes out unchanged, which is what separates this from a contrast boost.
//
// One pass, and no intermediate: the average is taken from eight neighbours in
// the same fragment that uses it. A separable blur would be cheaper at a wide
// radius, but a sharpen that reaches more than a texel or two stops reading as
// sharper and starts reading as haloed.

struct SharpenParams {
    // How far away a neighbour is, in UV: the radius in texels over the
    // texture's width and height. Derived per texture, like the blur's, so a
    // proxy and the original are sharpened over the same share of the frame.
    step: vec2<f32>,
    // How much of the detail is added back, beyond the picture itself. Zero is
    // never drawn: the node declines the pass instead.
    amount: f32,
    // Explicit, so the Rust writer's bytes and this layout cannot drift apart.
    pad0: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> params: SharpenParams;
@group(1) @binding(0) var source: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// A full-screen quad in the source texture's own space, as the blur draws: the
// clip's transform is applied afterwards, by the composite pass.
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
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_sharpen(in: VsOut) -> @location(0) vec4<f32> {
    let centre = textureSampleLevel(source, samp, in.uv, 0.0);
    let s = params.step;

    // The eight around it. Level 0 explicitly, for the reason the blur gives:
    // there are no mipmaps, and implicit derivatives would ask a question this
    // does not need answered.
    var around = vec3<f32>(0.0);
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(-s.x, -s.y), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(0.0, -s.y), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(s.x, -s.y), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(-s.x, 0.0), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(s.x, 0.0), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(-s.x, s.y), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(0.0, s.y), 0.0).rgb;
    around += textureSampleLevel(source, samp, in.uv + vec2<f32>(s.x, s.y), 0.0).rgb;
    let average = around / 8.0;

    let detail = centre.rgb - average;
    // Floored at zero: pushing a dark pixel beside a bright edge further down
    // can go negative, which becomes NaN once the sRGB encode takes a root.
    let sharpened = max(centre.rgb + detail * params.amount, vec3<f32>(0.0));
    return vec4<f32>(sharpened, centre.a);
}
