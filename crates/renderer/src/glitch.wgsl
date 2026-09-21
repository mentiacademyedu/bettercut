// RGB split, glitch and pixelate (`crate::glitch`), applied to one layer's
// picture. Pixelate reads each square block's picture from its middle, so the
// frame comes through as coarse tiles: the look of a censored face.
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
    // Pixelate block size as a share of the longer side; zero is off.
    block: f32,
    // Zoom blur: the share of each point's distance to the middle it streaks.
    zoom: f32,
    // Glow: how much of the gathered bright light is added back; zero is off.
    glow: f32,
    // Old film: how worn the print is, 0–1; zero is off. The struct is 32.
    film: f32,
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
// Samples along a zoom blur's streak.
const ZOOM_TAPS: i32 = 24;
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

// Taps gathered for a glow: two rings around the point.
const GLOW_TAPS: i32 = 12;
// Only light above this (linear) blooms, so a dim picture does not wash out.
const GLOW_THRESHOLD: f32 = 0.45;

// The bright light around `uv`: what is above the threshold, gathered from two
// rings a few percent of the shorter side out. Rings in pixels made square, so
// the glow is round on any frame shape.
fn gathered_glow(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let square = vec2<f32>(min(size.x, size.y)) / size;
    var sum = vec3<f32>(0.0);
    for (var ring = 1; ring <= 2; ring = ring + 1) {
        let radius = 0.018 * f32(ring * ring);
        for (var i = 0; i < GLOW_TAPS; i = i + 1) {
            let angle = (f32(i) + 0.5 * f32(ring)) / f32(GLOW_TAPS) * 6.2831853;
            let offset = vec2<f32>(cos(angle), sin(angle)) * radius * square;
            let tap = textureSampleLevel(source, samp, uv + offset, 0.0).rgb;
            sum = sum + max(tap - vec3<f32>(GLOW_THRESHOLD), vec3<f32>(0.0));
        }
    }
    return sum / f32(GLOW_TAPS * 2);
}

@fragment
fn fs_glitch(in: VsOut) -> @location(0) vec4<f32> {
    var picture = broken_up(in.uv);
    if params.glow > 0.0 {
        let light = gathered_glow(in.uv) * params.glow;
        picture = vec4<f32>(picture.rgb + light * picture.a, picture.a);
    }
    if params.film > 0.0 {
        picture = vec4<f32>(worn(picture.rgb, in.uv) , picture.a);
    }
    return picture;
}

// Scratches that come and go, a few frames at a time.
const SCRATCHES: u32 = 3u;
// Dust is scattered over a grid of this many cells across the shorter side.
const DUST_CELLS: f32 = 48.0;

// A picture as an old print: its exposure wavering from frame to frame, a few
// thin vertical scratches, and specks of dust — all from the frame number, so
// preview and export wear the same way (§46).
fn worn(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let film = params.film;
    let size = vec2<f32>(textureDimensions(source));
    var out = rgb;

    // Flicker: the whole frame a little brighter or darker.
    out = out * (1.0 + (hash(0.0, params.seed, 11u) - 0.5) * 0.25 * film);

    // Scratches hold for a few frames, then jump somewhere else.
    let held = floor(params.seed / 3.0);
    for (var i = 0u; i < SCRATCHES; i = i + 1u) {
        if hash(f32(i), held, 12u) < film * 0.8 {
            let x = hash(f32(i), held, 13u);
            let wobble = (hash(floor(uv.y * 40.0), held + f32(i), 14u) - 0.5) * 0.0015;
            // At least a pixel and a half, so a small proxy still shows it.
            let width = max(0.0008 + 0.0012 * hash(f32(i), held, 15u), 1.5 / size.x);
            let line = 1.0 - smoothstep(width * 0.5, width, abs(uv.x - x - wobble));
            // Mostly dark lines, now and then a bright one.
            let bright = hash(f32(i), held, 16u) > 0.7;
            let mark = select(vec3<f32>(0.05), vec3<f32>(0.9), bright);
            out = mix(out, mark, line * 0.7);
        }
    }

    // Dust: a speck in a few cells, different cells every frame.
    let cells = size / min(size.x, size.y) * DUST_CELLS;
    let cell = floor(uv * cells);
    let index = cell.x * 1000.0 + cell.y;
    if hash(index, params.seed, 17u) < film * 0.01 {
        let centre = vec2<f32>(hash(index, params.seed, 18u), hash(index, params.seed, 19u));
        let into = fract(uv * cells) - centre;
        let radius = 0.12 + 0.2 * hash(index, params.seed, 20u);
        let speck = 1.0 - smoothstep(radius * 0.6, radius, length(into));
        out = mix(out, vec3<f32>(0.02), speck * 0.85);
    }
    return max(out, vec3<f32>(0.0));
}

// The picture with the split, glitch, pixelate and zoom blur applied.
fn broken_up(start: vec2<f32>) -> vec4<f32> {
    var uv = start;

    if params.glitch > 0.0 {
        let band = floor(uv.y * BANDS);
        // More of the bands break as the amount rises; at full, about half.
        if hash(band, params.seed, 1u) < params.glitch * 0.5 {
            let shove = (hash(band, params.seed, 2u) - 0.5) * 2.0 * MAX_THROW * params.glitch;
            uv.x = uv.x + shove;
        }
    }

    // Into blocks after the bands are thrown, so a broken band still breaks
    // along block edges. Square in pixels: the share is of the longer side.
    if params.block > 0.0 {
        let size = vec2<f32>(textureDimensions(source));
        let cell = max(params.block * max(size.x, size.y), 1.0);
        uv = (floor(uv * size / cell) + 0.5) * cell / size;
    }

    // Zoom blur: the average of the picture along the line from this point
    // towards the middle, a rush forward. Evenly spaced taps, so the streak
    // is the same length at any resolution.
    if params.zoom > 0.0 {
        var sum = vec4<f32>(0.0);
        let towards = vec2<f32>(0.5, 0.5) - uv;
        for (var i = 0; i < ZOOM_TAPS; i = i + 1) {
            let t = f32(i) / f32(ZOOM_TAPS - 1) * params.zoom;
            sum = sum + textureSampleLevel(source, samp, uv + towards * t, 0.0);
        }
        let streaked = sum / f32(ZOOM_TAPS);
        if params.split <= 0.0 {
            return streaked;
        }
        let offset = vec2<f32>(params.split, 0.0);
        let red = textureSampleLevel(source, samp, uv + offset, 0.0).r;
        let blue = textureSampleLevel(source, samp, uv - offset, 0.0).b;
        return vec4<f32>(mix(streaked.r, red, 0.5), streaked.g, mix(streaked.b, blue, 0.5), streaked.a);
    }

    // Level 0 explicitly: there are no mipmaps, as the other nodes say.
    let centre = textureSampleLevel(source, samp, uv, 0.0);
    let offset = vec2<f32>(params.split, 0.0);
    let red = textureSampleLevel(source, samp, uv + offset, 0.0).r;
    let blue = textureSampleLevel(source, samp, uv - offset, 0.0).b;
    return vec4<f32>(red, centre.g, blue, centre.a);
}
