// Reflections (`crate::reflect`), applied to one layer's picture.
//
// A transcription of `bettercut_timeline::Reflection::source_uv`: for each
// texel drawn, where in the source to read it. Keep the two in step — the GPU
// test compares them.

struct ReflectParams {
    // 1 left & right, 2 top & bottom, 3 four-way, 4 kaleidoscope.
    kind: f32,
    // The picture's width over its height, so the kaleidoscope's wedges are
    // true angles.
    aspect: f32,
    pad0: f32,
    pad1: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> params: ReflectParams;
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

// Wedges in the kaleidoscope: `KALEIDOSCOPE_SEGMENTS`.
const SEGMENTS: f32 = 6.0;
const TAU: f32 = 6.283185307179586;

// The first half of 0..1 mirrored onto the second.
fn fold_half(t: f32) -> f32 {
    return 0.5 - abs(t - 0.5);
}

// Any coordinate brought back into 0..1 by mirroring at each edge.
fn fold_edges(t: f32) -> f32 {
    let wrapped = t - 2.0 * floor(t / 2.0);
    return select(wrapped, 2.0 - wrapped, wrapped > 1.0);
}

@fragment
fn fs_reflect(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    let kind = u32(params.kind + 0.5);

    if kind == 1u {
        uv.x = fold_half(uv.x);
    } else if kind == 2u {
        uv.y = fold_half(uv.y);
    } else if kind == 3u {
        uv = vec2<f32>(fold_half(uv.x), fold_half(uv.y));
    } else if kind == 4u {
        let aspect = select(1.0, params.aspect, params.aspect > 0.0);
        let p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
        let radius = length(p);
        let wedge = TAU / SEGMENTS;
        var angle = atan2(p.y, p.x);
        angle = angle - wedge * floor(angle / wedge);
        if angle > wedge / 2.0 {
            angle = wedge - angle;
        }
        uv = vec2<f32>(
            fold_edges(radius * cos(angle) / aspect + 0.5),
            fold_edges(radius * sin(angle) + 0.5),
        );
    }

    // Level 0 explicitly: there are no mipmaps, as the other nodes say.
    return textureSampleLevel(source, samp, uv, 0.0);
}
