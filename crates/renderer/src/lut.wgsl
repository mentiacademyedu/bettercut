// A colour lookup table (`crate::lut`), applied to one layer's picture.
//
// The source arrives in linear light — the texture is sRGB-aware — but a
// creative LUT is built for display-encoded values, the numbers in an ordinary
// video file. So each pixel is encoded, looked up, decoded again, and mixed
// with the original by the strength.

struct LutParams {
    // Where the table's input range starts, per channel, and how much of the
    // grade to apply.
    domain_min: vec3<f32>,
    strength: f32,
    // Where it ends, and the table's samples along an axis.
    domain_max: vec3<f32>,
    size: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> params: LutParams;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(2) @binding(0) var table: texture_3d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// A full-screen quad in the source's own space, as the other nodes draw.
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

fn encode(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn decode(encoded: vec3<f32>) -> vec3<f32> {
    let c = clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_lut(in: VsOut) -> @location(0) vec4<f32> {
    // Level 0 explicitly: there are no mipmaps, as the other nodes say.
    let picture = textureSampleLevel(source, samp, in.uv, 0.0);
    let encoded = encode(picture.rgb);

    // Into the table's domain, then onto texel centres: the first sample sits
    // half a texel in, the last half a texel from the far edge, so the
    // hardware's trilinear filter interpolates between samples exactly as the
    // table means it to.
    let unit = clamp(
        (encoded - params.domain_min) / (params.domain_max - params.domain_min),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    let n = params.size;
    let coord = unit * ((n - 1.0) / n) + vec3<f32>(0.5 / n);
    let graded = textureSampleLevel(table, samp, coord, 0.0).rgb;

    let mixed = mix(encoded, graded, params.strength);
    return vec4<f32>(decode(mixed), picture.a);
}
