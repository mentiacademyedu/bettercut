// Milestone 0 spike — two-layer compositor.
//
// Layer A: "decoded video frame" (uploaded per frame, or resident on the GPU)
// Layer B: overlay layer, transformed and blended over A.
//
// This is deliberately the shape §22 describes (transform -> composite), so the
// spike measures the real pipeline's cost, not a triangle demo.

struct Uniforms {
    // Overlay placement, in normalized composite space.
    overlay_offset: vec2<f32>,
    overlay_scale: vec2<f32>,
    overlay_opacity: f32,
    // 0.0 = source is full range, 1.0 = expand limited (16-235) -> full. §21a.2
    expand_limited_range: f32,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var layer_a: texture_2d<f32>;
@group(0) @binding(2) var layer_b: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle. No vertex buffer, no index buffer.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    out.uv = vec2<f32>(x, y);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

// §21a.2: limited-range (16-235) -> full-range expansion. Getting this wrong is
// the "crushed blacks" bug. Included here so the spike exercises the real path.
fn expand_range(c: vec3<f32>) -> vec3<f32> {
    return clamp((c - 16.0 / 255.0) * (255.0 / 219.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var base = textureSample(layer_a, samp, in.uv).rgb;
    base = mix(base, expand_range(base), u.expand_limited_range);

    // Transform the overlay: uv -> overlay-local uv.
    let ov_uv = (in.uv - u.overlay_offset) / u.overlay_scale;
    var out_rgb = base;

    if (ov_uv.x >= 0.0 && ov_uv.x <= 1.0 && ov_uv.y >= 0.0 && ov_uv.y <= 1.0) {
        let ov = textureSample(layer_b, samp, ov_uv);
        out_rgb = mix(base, ov.rgb, ov.a * u.overlay_opacity);
    }

    return vec4<f32>(out_rgb, 1.0);
}
