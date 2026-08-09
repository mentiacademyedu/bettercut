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
struct Layer {
    // Column-major 2x2 plus translation, mapping the unit quad into
    // normalized device coordinates.
    transform: mat3x3<f32>,
    opacity: f32,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var<uniform> layer: Layer;
@group(1) @binding(0) var source: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
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
    out.uv = vec2<f32>(corner.x, corner.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(source, samp, in.uv);
    return vec4<f32>(texel.rgb, texel.a * layer.opacity);
}
