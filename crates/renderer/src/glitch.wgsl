// RGB split and glitch (`crate::glitch`), applied to one layer's picture.
//
// RGB split pulls the red and blue channels apart sideways, the look of a
// cheap lens or a VHS tape. Glitch cuts the picture into horizontal bands and
// throws a few of them sideways, different bands on every frame — a digital
// signal breaking up. Both read the source at shifted positions and nothing
// else, so a picture with neither comes through unchanged.

struct GlitchParams {
    // How far red and blue are pulled apart, in UV: a share of the width.
    split: f32,
    // How much of the picture breaks up, 0–1.
    glitch: f32,
    // The frame being drawn, so the broken bands change every frame and the
    // same frame always breaks the same way (preview and export agree).
    seed: f32,
    pad0: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> params: GlitchParams;
@group(1) @binding(0) var source: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

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

// Bands the picture is cut into. A share of the height, not a pixel count, so a
// proxy and the original break along the same lines.
const BANDS: f32 = 28.0;
// The furthest a broken band is thrown, as a share of the width.
const MAX_THROW: f32 = 0.06;

// A fixed pseudo-random value in 0..1 for (band, frame, salt).
fn hash(band: f32, seed: f32, salt: u32) -> f32 {
    var v = vec3<u32>(u32(band), u32(seed), salt);
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    return f32(v.x) / 4294967295.0;
}

@fragment
fn fs_glitch(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;

    if params.glitch > 0.0 {
        let band = floor(uv.y * BANDS);
        // More of the bands break as the amount rises; at full, about half.
        if hash(band, params.seed, 1u) < params.glitch * 0.5 {
            let shove = (hash(band, params.seed, 2u) - 0.5) * 2.0 * MAX_THROW * params.glitch;
            uv.x = uv.x + shove;
        }
    }

    // Level 0 explicitly: there are no mipmaps, as the other nodes say.
    let centre = textureSampleLevel(source, samp, uv, 0.0);
    let offset = vec2<f32>(params.split, 0.0);
    let red = textureSampleLevel(source, samp, uv + offset, 0.0).r;
    let blue = textureSampleLevel(source, samp, uv - offset, 0.0).b;
    return vec4<f32>(red, centre.g, blue, centre.a);
}
