// The finished frame's raw sRGB bytes, drawn into a plain (not sRGB-aware)
// texture for egui — on a GPU that cannot view one texture in two formats
// (WebGL2), where a second view of the target is not on offer.
//
// Reading the sRGB target decodes it to linear; encoding it again here gives
// back the stored bytes exactly, which is what egui expects to be handed.

@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the whole target.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn encode(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        return linear * 12.92;
    }
    return 1.055 * pow(linear, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = textureLoad(source, vec2<i32>(position.xy), 0);
    return vec4<f32>(encode(colour.r), encode(colour.g), encode(colour.b), colour.a);
}
