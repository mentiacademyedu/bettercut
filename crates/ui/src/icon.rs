//! The app's icon, drawn in code: a rounded square going from blue to violet,
//! with a white play triangle cut through by a diagonal gap — a video, cut.
//!
//! Drawn rather than loaded so there is no image file to keep in step with
//! the name or the colours, and so any size can be asked for.

/// The icon as `size`×`size` RGBA bytes, row by row, edges anti-aliased.
pub fn rgba(size: u32) -> Vec<u8> {
    let n = size.max(8) as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    // Four samples a pixel, so the curves and the diagonal are smooth.
    let offsets = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    for y in 0..size {
        for x in 0..size {
            let (mut body, mut mark) = (0.0_f32, 0.0_f32);
            for (dx, dy) in offsets {
                let (u, v) = ((x as f32 + dx) / n, (y as f32 + dy) / n);
                if in_rounded_square(u, v) {
                    body += 0.25;
                    if in_play(u, v) && !in_cut(u, v) {
                        mark += 0.25;
                    }
                }
            }
            // Blue at the top left to violet at the bottom right.
            let t = (x + y) as f32 / (2.0 * n);
            let base = [
                lerp(56.0, 132.0, t),
                lerp(112.0, 70.0, t),
                lerp(240.0, 220.0, t),
            ];
            let colour = |c: f32| {
                let mixed = if body > 0.0 {
                    c + (255.0 - c) * (mark / body)
                } else {
                    c
                };
                mixed.round().clamp(0.0, 255.0) as u8
            };
            out.extend_from_slice(&[
                colour(base[0]),
                colour(base[1]),
                colour(base[2]),
                (body * 255.0).round() as u8,
            ]);
        }
    }
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Inside a square with rounded corners, filling most of the tile.
fn in_rounded_square(u: f32, v: f32) -> bool {
    let (lo, hi, r) = (0.04, 0.96, 0.22);
    if !(lo..=hi).contains(&u) || !(lo..=hi).contains(&v) {
        return false;
    }
    let cx = u.clamp(lo + r, hi - r);
    let cy = v.clamp(lo + r, hi - r);
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

/// Inside the play triangle, pointing right, a little right of centre.
fn in_play(u: f32, v: f32) -> bool {
    let (left, right, top, bottom) = (0.34, 0.74, 0.26, 0.74);
    if u < left || u > right {
        return false;
    }
    let half = (bottom - top) / 2.0 * (right - u) / (right - left);
    let middle = (top + bottom) / 2.0;
    (v - middle).abs() <= half
}

/// The cut: a thin diagonal gap across the triangle.
fn in_cut(u: f32, v: f32) -> bool {
    ((u - 0.5) - (v - 0.5) * 0.6).abs() < 0.035
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_a_see_through_corner_a_coloured_body_and_a_white_mark() {
        let size = 64;
        let px = rgba(size);
        assert_eq!(px.len(), (size * size * 4) as usize);
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(0, 0)[3], 0, "the corner is rounded off");
        let edge = at(8, 32);
        assert_eq!(edge[3], 255, "the body is solid");
        assert!(edge[2] > 200 && edge[0] < 150, "the body is blue: {edge:?}");
        let mark = at(28, 32);
        assert!(
            mark[0] > 240 && mark[1] > 240,
            "the play mark is white: {mark:?}"
        );
    }
}
