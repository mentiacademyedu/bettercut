//! Colour wheels: a wheel to drag for each of lift, gamma and gain
//! (`bettercut_timeline::ColorWheels`).
//!
//! On a wheel the direction is the colour and the distance is how much of it,
//! with a slider beneath for how bright — which is what a grading desk offers,
//! and the reason it does: a cast is a *direction*, and a person finds a
//! direction by dragging towards it far faster than by moving three sliders
//! whose sum is the direction they meant.
//!
//! The maths that turns a place on the wheel into an RGB offset and back is
//! in the open ([`to_wheel`], [`from_wheel`]) and tested, because it is the
//! part that has to be exact: a wheel that drifts when picked up and put down
//! again is a wheel nobody trusts.

use crate::theme;

/// Half the width of a wheel, in points. Small enough for three across the
/// inspector, large enough to drag with any accuracy.
pub const WHEEL_RADIUS: f32 = 30.0;

/// The two directions across the colour plane, each with no brightness in
/// it: red against the other two, and green against blue. Orthonormal, so a
/// distance on the wheel is the same distance whichever way it points.
const U: [f32; 3] = [0.816_496_6, -0.408_248_3, -0.408_248_3];
const V: [f32; 3] = [
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
    -std::f32::consts::FRAC_1_SQRT_2,
];

/// Where an RGB offset sits: across and up the wheel (each -1..1, the middle
/// being no colour at all) and its brightness, the part a wheel cannot show.
pub fn to_wheel(offset: [f32; 3]) -> (f32, f32, f32) {
    let lum = (offset[0] + offset[1] + offset[2]) / 3.0;
    let chroma = offset.map(|c| c - lum);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    (dot(chroma, U), dot(chroma, V), lum)
}

/// The RGB offset at a place on the wheel and a brightness: the inverse of
/// [`to_wheel`], each channel held to -1..1.
pub fn from_wheel(x: f32, y: f32, lum: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| (lum + x * U[i] + y * V[i]).clamp(-1.0, 1.0))
}

/// One wheel with its brightness slider, for `value`. Drag inside the wheel
/// for the colour, double-click to put it back to nothing. The response says
/// `changed` while either is being moved.
pub fn wheel(ui: &mut egui::Ui, label: &str, value: &mut [f32; 3]) -> egui::Response {
    ui.vertical(|ui| {
        ui.spacing_mut().slider_width = WHEEL_RADIUS * 2.0;
        ui.label(egui::RichText::new(label).small().color(theme::disabled()));

        let size = egui::vec2(WHEEL_RADIUS * 2.0, WHEEL_RADIUS * 2.0);
        let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let (mut x, mut y, mut lum) = to_wheel(*value);
        let centre = rect.center();
        let painter = ui.painter_at(rect);
        paint_wheel(&painter, centre, WHEEL_RADIUS - 1.0);

        // A drag goes where the pointer is, held inside the rim: past the
        // edge is not more of a colour.
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let dx = (pos.x - centre.x) / WHEEL_RADIUS;
            let dy = -(pos.y - centre.y) / WHEEL_RADIUS;
            let reach = (dx * dx + dy * dy).sqrt();
            (x, y) = if reach > 1.0 {
                (dx / reach, dy / reach)
            } else {
                (dx, dy)
            };
            *value = from_wheel(x, y, lum);
            response.mark_changed();
        }
        if response.double_clicked() {
            *value = [0.0; 3];
            (x, y, lum) = (0.0, 0.0, 0.0);
            response.mark_changed();
        }
        response = response.on_hover_text(
            "Drag towards a colour to add it; further is more. Double-click for none",
        );

        // The marker, drawn after the drag so it sits where the hand is.
        let at = egui::pos2(centre.x + x * WHEEL_RADIUS, centre.y - y * WHEEL_RADIUS);
        painter.circle_filled(at, 4.0, theme::clip_text());
        painter.circle_stroke(at, 4.0, egui::Stroke::new(1.0, theme::background()));

        // How bright, beneath: the one direction the wheel has no room for.
        let slider = ui
            .add(egui::Slider::new(&mut lum, -1.0..=1.0).show_value(false))
            .on_hover_text("Darker to brighter, for every colour at once");
        if slider.changed() {
            *value = from_wheel(x, y, lum);
        }
        response | slider
    })
    .inner
}

/// The wheel's face: grey in the middle, every colour round the rim, as a
/// fan of triangles coloured at their corners.
fn paint_wheel(painter: &egui::Painter, centre: egui::Pos2, radius: f32) {
    const SEGMENTS: usize = 48;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, egui::Color32::from_gray(60));
    for step in 0..=SEGMENTS {
        let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let rgb = from_wheel(angle.cos(), angle.sin(), 0.0);
        let byte = |c: f32| ((0.5 + 0.5 * c).clamp(0.0, 1.0) * 255.0) as u8;
        mesh.colored_vertex(
            egui::pos2(
                centre.x + radius * angle.cos(),
                centre.y - radius * angle.sin(),
            ),
            egui::Color32::from_rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2])),
        );
    }
    for step in 0..SEGMENTS {
        mesh.add_triangle(0, (step + 1) as u32, (step + 2) as u32);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(
        centre,
        radius,
        egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// Picked up and put down again is the same offset: the wheel does not
    /// drift.
    #[test]
    fn the_wheel_and_the_offset_round_trip() {
        for offset in [
            [0.3, -0.1, 0.2],
            [-0.5, -0.5, 0.4],
            [0.0, 0.0, 0.0],
            [0.25, 0.25, 0.25],
        ] {
            let (x, y, lum) = to_wheel(offset);
            assert!(
                close(from_wheel(x, y, lum), offset),
                "{offset:?} came back as {:?}",
                from_wheel(x, y, lum)
            );
        }
    }

    /// Grey has no direction: it sits in the middle at any brightness, which
    /// is what makes the brightness slider a separate thing from the wheel.
    #[test]
    fn grey_sits_in_the_middle() {
        for level in [-0.8, -0.2, 0.0, 0.4, 1.0] {
            let (x, y, lum) = to_wheel([level; 3]);
            assert!(
                x.abs() < 1e-6 && y.abs() < 1e-6,
                "grey {level} left the middle: {x},{y}"
            );
            assert!((lum - level).abs() < 1e-6);
        }
    }

    /// Red is one way and its opposite the other: dragging towards a colour
    /// adds that colour, and away from it takes it out.
    #[test]
    fn opposite_colours_are_opposite_ways() {
        let (rx, ry, _) = to_wheel([1.0, 0.0, 0.0]);
        let (cx, cy, _) = to_wheel([0.0, 1.0, 1.0]);
        assert!(rx > 0.5 && ry.abs() < 1e-6, "red at {rx},{ry}");
        assert!(cx < -0.5 && cy.abs() < 1e-6, "cyan at {cx},{cy}");
        let (gx, gy, _) = to_wheel([0.0, 1.0, 0.0]);
        let (bx, by, _) = to_wheel([0.0, 0.0, 1.0]);
        assert!(
            (gx - bx).abs() < 1e-6 && gy > 0.0 && by < 0.0,
            "green {gx},{gy} blue {bx},{by}"
        );
    }

    /// A drag to the rim is a full colour and nothing past it.
    #[test]
    fn the_rim_stays_within_range() {
        for step in 0..36 {
            let angle = step as f32 / 36.0 * std::f32::consts::TAU;
            let rgb = from_wheel(angle.cos(), angle.sin(), 0.0);
            assert!(rgb.iter().all(|c| (-1.0..=1.0).contains(c)), "{rgb:?}");
            assert!(
                rgb.iter().sum::<f32>().abs() < 1e-5,
                "a rim colour changed the brightness"
            );
        }
    }
}
