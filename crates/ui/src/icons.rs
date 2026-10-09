//! The interface's icons, drawn as lines with the painter.
//!
//! Vector rather than a font or images: no glyph can come out as an empty box
//! (the problem `tests/glyphs.rs` exists for), nothing is uploaded to the GPU,
//! and every icon is sharp at any scale the screen asks for. One stroke weight
//! and one grid — 16 units, drawn at whatever size the button is — so the set
//! reads as one family.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

/// Every icon the interface draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    // Project
    NewFile,
    Folder,
    Save,
    Undo,
    Redo,
    Export,
    QuickExport,
    Plus,
    Captions,
    Search,
    Command,
    Windows,
    Sliders,
    // Transport
    Play,
    Pause,
    ToStart,
    ToEnd,
    Back,
    Forward,
    Loop,
    Fullscreen,
    Record,
    Camera,
    More,
    ChevronDown,
    ChevronRight,
    // Editing
    Split,
    Blade,
    Trash,
    TrimStart,
    TrimEnd,
    Freeze,
    Marker,
    Snap,
    Magnet,
    ZoomIn,
    ZoomOut,
    Fit,
    // Tracks
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Speaker,
    SpeakerOff,
    // Library
    Media,
    Music,
    Text,
    Sticker,
    Effects,
    Transitions,
    Filters,
    Reset,
    Keyframe,
    Close,
}

/// A path through grid points, in the icon's 16-unit space.
fn path(points: &[(f32, f32)], map: &impl Fn(f32, f32) -> Pos2, closed: bool, stroke: Stroke) -> Shape {
    let points: Vec<Pos2> = points.iter().map(|&(x, y)| map(x, y)).collect();
    if closed {
        Shape::closed_line(points, stroke)
    } else {
        Shape::line(points, stroke)
    }
}

fn filled(points: &[(f32, f32)], map: &impl Fn(f32, f32) -> Pos2, colour: Color32) -> Shape {
    Shape::convex_polygon(
        points.iter().map(|&(x, y)| map(x, y)).collect(),
        colour,
        Stroke::NONE,
    )
}

/// An arc of a circle around `(cx, cy)` from `from` to `to` radians, as
/// points, for the undo, loop and reset arrows.
fn arc(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let steps = 14;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Draw `icon` filling `rect` (square, centred in it) in `colour`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, colour: Color32) {
    let side = rect.width().min(rect.height());
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let unit = side / 16.0;
    let map = move |x: f32, y: f32| origin + vec2(x * unit, y * unit);
    // 1.5 px at 16, thicker as the icon grows, never thinner than a pixel.
    let stroke = Stroke::new((unit * 1.5).max(1.1), colour);
    let thin = Stroke::new((unit * 1.2).max(1.0), colour);
    let r = |x: f32| x * unit;
    let mut shapes: Vec<Shape> = Vec::new();
    let line = |pts: &[(f32, f32)]| path(pts, &map, false, stroke);
    let closed = |pts: &[(f32, f32)]| path(pts, &map, true, stroke);
    let rect_of = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(map(x0, y0), map(x1, y1));

    match icon {
        Icon::NewFile => {
            shapes.push(closed(&[(3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5)]));
            shapes.push(line(&[(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)]));
            shapes.push(line(&[(8.0, 7.5), (8.0, 12.0)]));
            shapes.push(line(&[(5.75, 9.75), (10.25, 9.75)]));
        }
        Icon::Folder => {
            shapes.push(closed(&[
                (1.5, 3.0),
                (6.0, 3.0),
                (7.5, 4.75),
                (14.5, 4.75),
                (14.5, 13.0),
                (1.5, 13.0),
            ]));
            shapes.push(line(&[(1.5, 6.75), (14.5, 6.75)]));
        }
        Icon::Save => {
            shapes.push(closed(&[(2.0, 2.0), (11.5, 2.0), (14.0, 4.5), (14.0, 14.0), (2.0, 14.0)]));
            shapes.push(line(&[(5.0, 2.0), (5.0, 5.5), (10.5, 5.5), (10.5, 2.0)]));
            shapes.push(closed(&[(4.5, 9.0), (11.5, 9.0), (11.5, 14.0), (4.5, 14.0)]));
        }
        Icon::Undo | Icon::Redo => {
            let flip = icon == Icon::Redo;
            let fx = |x: f32| if flip { 16.0 - x } else { x };
            let mut curve: Vec<(f32, f32)> = arc(9.0, 9.0, 4.5, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
            curve.insert(0, (3.0, 4.5));
            curve.push((5.0, 13.5));
            let curve: Vec<(f32, f32)> = curve.into_iter().map(|(x, y)| (fx(x), y)).collect();
            shapes.push(line(&curve));
            shapes.push(line(&[(fx(5.5), 2.0), (fx(3.0), 4.5), (fx(5.5), 7.0)]));
        }
        Icon::Export => {
            shapes.push(line(&[(8.0, 10.0), (8.0, 1.75)]));
            shapes.push(line(&[(4.75, 5.0), (8.0, 1.75), (11.25, 5.0)]));
            shapes.push(line(&[(2.5, 9.0), (2.5, 14.0), (13.5, 14.0), (13.5, 9.0)]));
        }
        Icon::QuickExport => {
            shapes.push(closed(&[(9.0, 1.5), (3.5, 9.0), (7.75, 9.0), (7.0, 14.5), (12.5, 7.0), (8.25, 7.0)]));
        }
        Icon::Plus => {
            shapes.push(line(&[(8.0, 2.5), (8.0, 13.5)]));
            shapes.push(line(&[(2.5, 8.0), (13.5, 8.0)]));
        }
        Icon::Captions => {
            shapes.push(Shape::rect_stroke(
                rect_of(1.5, 3.0, 14.5, 13.0),
                r(2.0),
                stroke,
                egui::StrokeKind::Middle,
            ));
            shapes.push(line(&[(4.0, 9.75), (8.0, 9.75)]));
            shapes.push(line(&[(9.5, 9.75), (12.0, 9.75)]));
            shapes.push(line(&[(4.0, 6.5), (6.0, 6.5)]));
            shapes.push(line(&[(7.5, 6.5), (12.0, 6.5)]));
        }
        Icon::Search => {
            shapes.push(Shape::circle_stroke(map(7.0, 7.0), r(4.75), stroke));
            shapes.push(line(&[(10.5, 10.5), (14.0, 14.0)]));
        }
        Icon::Command => {
            // A lightning-quick "find an action": a magnifier over lines.
            shapes.push(line(&[(1.75, 3.5), (8.5, 3.5)]));
            shapes.push(line(&[(1.75, 7.0), (6.0, 7.0)]));
            shapes.push(line(&[(1.75, 10.5), (5.0, 10.5)]));
            shapes.push(Shape::circle_stroke(map(10.5, 9.5), r(3.0), stroke));
            shapes.push(line(&[(12.75, 11.75), (14.75, 13.75)]));
        }
        Icon::Windows => {
            shapes.push(Shape::rect_stroke(
                rect_of(1.75, 2.25, 14.25, 13.75),
                r(1.75),
                stroke,
                egui::StrokeKind::Middle,
            ));
            shapes.push(line(&[(1.75, 5.75), (14.25, 5.75)]));
            shapes.push(line(&[(6.25, 5.75), (6.25, 13.75)]));
        }
        Icon::Sliders => {
            for (y, knob) in [(4.0, 10.5), (8.0, 5.0), (12.0, 9.0)] {
                shapes.push(line(&[(2.0, y), (14.0, y)]));
                shapes.push(Shape::circle_filled(map(knob, y), r(1.9), colour));
            }
        }
        Icon::Play => {
            shapes.push(filled(&[(4.5, 2.5), (13.0, 8.0), (4.5, 13.5)], &map, colour));
        }
        Icon::Pause => {
            shapes.push(Shape::rect_filled(rect_of(3.75, 2.75, 6.75, 13.25), r(0.75), colour));
            shapes.push(Shape::rect_filled(rect_of(9.25, 2.75, 12.25, 13.25), r(0.75), colour));
        }
        Icon::ToStart => {
            shapes.push(Shape::rect_filled(rect_of(3.0, 3.25, 4.75, 12.75), r(0.5), colour));
            shapes.push(filled(&[(13.0, 3.25), (13.0, 12.75), (5.75, 8.0)], &map, colour));
        }
        Icon::ToEnd => {
            shapes.push(Shape::rect_filled(rect_of(11.25, 3.25, 13.0, 12.75), r(0.5), colour));
            shapes.push(filled(&[(3.0, 3.25), (10.25, 8.0), (3.0, 12.75)], &map, colour));
        }
        Icon::Back => {
            shapes.push(filled(&[(8.0, 3.5), (8.0, 12.5), (1.5, 8.0)], &map, colour));
            shapes.push(filled(&[(14.5, 3.5), (14.5, 12.5), (8.0, 8.0)], &map, colour));
        }
        Icon::Forward => {
            shapes.push(filled(&[(1.5, 3.5), (8.0, 8.0), (1.5, 12.5)], &map, colour));
            shapes.push(filled(&[(8.0, 3.5), (14.5, 8.0), (8.0, 12.5)], &map, colour));
        }
        Icon::Loop => {
            shapes.push(line(&[(2.5, 9.0), (2.5, 6.5), (4.5, 4.5), (12.5, 4.5)]));
            shapes.push(line(&[(10.5, 2.5), (12.75, 4.5), (10.5, 6.5)]));
            shapes.push(line(&[(13.5, 7.0), (13.5, 9.5), (11.5, 11.5), (3.5, 11.5)]));
            shapes.push(line(&[(5.5, 9.5), (3.25, 11.5), (5.5, 13.5)]));
        }
        Icon::Fullscreen => {
            shapes.push(line(&[(2.0, 6.0), (2.0, 2.0), (6.0, 2.0)]));
            shapes.push(line(&[(10.0, 2.0), (14.0, 2.0), (14.0, 6.0)]));
            shapes.push(line(&[(14.0, 10.0), (14.0, 14.0), (10.0, 14.0)]));
            shapes.push(line(&[(6.0, 14.0), (2.0, 14.0), (2.0, 10.0)]));
        }
        Icon::Record => {
            shapes.push(Shape::circle_stroke(map(8.0, 8.0), r(6.0), stroke));
            shapes.push(Shape::circle_filled(map(8.0, 8.0), r(3.25), colour));
        }
        Icon::Camera => {
            shapes.push(closed(&[
                (1.5, 5.0),
                (4.5, 5.0),
                (5.75, 3.0),
                (10.25, 3.0),
                (11.5, 5.0),
                (14.5, 5.0),
                (14.5, 13.0),
                (1.5, 13.0),
            ]));
            shapes.push(Shape::circle_stroke(map(8.0, 8.75), r(2.6), stroke));
        }
        Icon::More => {
            for x in [3.5, 8.0, 12.5] {
                shapes.push(Shape::circle_filled(map(x, 8.0), r(1.35), colour));
            }
        }
        Icon::ChevronDown => {
            shapes.push(line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]));
        }
        Icon::ChevronRight => {
            shapes.push(line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]));
        }
        Icon::Split => {
            // Scissors.
            shapes.push(Shape::circle_stroke(map(4.25, 4.25), r(2.25), stroke));
            shapes.push(Shape::circle_stroke(map(4.25, 11.75), r(2.25), stroke));
            shapes.push(line(&[(5.9, 5.9), (14.0, 12.5)]));
            shapes.push(line(&[(5.9, 10.1), (14.0, 3.5)]));
        }
        Icon::Blade => {
            shapes.push(closed(&[(10.5, 1.75), (14.25, 5.5), (6.0, 13.75), (2.25, 13.75), (2.25, 10.0)]));
            shapes.push(line(&[(8.5, 3.75), (12.25, 7.5)]));
        }
        Icon::Trash => {
            shapes.push(line(&[(2.0, 4.0), (14.0, 4.0)]));
            shapes.push(line(&[(6.0, 4.0), (6.0, 2.0), (10.0, 2.0), (10.0, 4.0)]));
            shapes.push(closed(&[(3.75, 4.0), (12.25, 4.0), (11.5, 14.25), (4.5, 14.25)]));
            shapes.push(line(&[(6.75, 7.0), (6.75, 11.5)]));
            shapes.push(line(&[(9.25, 7.0), (9.25, 11.5)]));
        }
        Icon::TrimStart | Icon::TrimEnd => {
            let flip = icon == Icon::TrimEnd;
            let fx = |x: f32| if flip { 16.0 - x } else { x };
            shapes.push(line(&[(fx(5.0), 2.0), (fx(5.0), 14.0)]));
            shapes.push(line(&[(fx(8.0), 2.0), (fx(13.5), 2.0), (fx(13.5), 14.0), (fx(8.0), 14.0)]));
            shapes.push(line(&[(fx(1.5), 8.0), (fx(11.0), 8.0)]));
            shapes.push(line(&[(fx(8.5), 5.5), (fx(11.0), 8.0), (fx(8.5), 10.5)]));
        }
        Icon::Freeze => {
            // A snowflake: three bars through the middle, with tips.
            for k in 0..3 {
                let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 3.0;
                let (dx, dy) = (6.25 * a.cos(), 6.25 * a.sin());
                shapes.push(line(&[(8.0 - dx, 8.0 - dy), (8.0 + dx, 8.0 + dy)]));
            }
        }
        Icon::Marker => {
            shapes.push(closed(&[(4.0, 2.0), (12.0, 2.0), (12.0, 13.5), (8.0, 10.5), (4.0, 13.5)]));
        }
        Icon::Snap => {
            // Two edges meeting on a line.
            shapes.push(line(&[(8.0, 1.5), (8.0, 14.5)]));
            shapes.push(Shape::rect_stroke(
                rect_of(1.75, 4.75, 6.0, 11.25),
                r(1.0),
                thin,
                egui::StrokeKind::Middle,
            ));
            shapes.push(Shape::rect_stroke(
                rect_of(10.0, 4.75, 14.25, 11.25),
                r(1.0),
                thin,
                egui::StrokeKind::Middle,
            ));
        }
        Icon::Magnet => {
            let mut u: Vec<(f32, f32)> = vec![(3.0, 2.5), (3.0, 8.0)];
            u.extend(arc(8.0, 8.0, 5.0, std::f32::consts::PI, 0.0).into_iter().map(|(x, y)| (x, 16.0 - y)));
            u.push((13.0, 2.5));
            shapes.push(line(&u));
            shapes.push(line(&[(3.0, 5.5), (6.0, 5.5)]));
            shapes.push(line(&[(10.0, 5.5), (13.0, 5.5)]));
            shapes.push(line(&[(6.0, 2.5), (6.0, 8.0)]));
            shapes.push(line(&[(10.0, 2.5), (10.0, 8.0)]));
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            shapes.push(Shape::circle_stroke(map(7.0, 7.0), r(4.75), stroke));
            shapes.push(line(&[(10.5, 10.5), (14.0, 14.0)]));
            shapes.push(line(&[(4.75, 7.0), (9.25, 7.0)]));
            if icon == Icon::ZoomIn {
                shapes.push(line(&[(7.0, 4.75), (7.0, 9.25)]));
            }
        }
        Icon::Fit => {
            shapes.push(line(&[(1.5, 8.0), (14.5, 8.0)]));
            shapes.push(line(&[(4.0, 5.5), (1.5, 8.0), (4.0, 10.5)]));
            shapes.push(line(&[(12.0, 5.5), (14.5, 8.0), (12.0, 10.5)]));
            shapes.push(line(&[(1.5, 2.0), (1.5, 3.5)]));
            shapes.push(line(&[(14.5, 2.0), (14.5, 3.5)]));
            shapes.push(line(&[(1.5, 12.5), (1.5, 14.0)]));
            shapes.push(line(&[(14.5, 12.5), (14.5, 14.0)]));
        }
        Icon::Eye | Icon::EyeOff => {
            let mut top = arc(8.0, 13.0, 8.4, -2.45, -0.69);
            let bottom = arc(8.0, 3.0, 8.4, 0.69, 2.45);
            top.extend(bottom);
            shapes.push(path(&top, &map, true, stroke));
            shapes.push(Shape::circle_stroke(map(8.0, 8.0), r(2.1), stroke));
            if icon == Icon::EyeOff {
                shapes.push(line(&[(2.5, 2.5), (13.5, 13.5)]));
            }
        }
        Icon::Lock | Icon::Unlock => {
            shapes.push(Shape::rect_stroke(
                rect_of(3.0, 7.0, 13.0, 14.25),
                r(1.5),
                stroke,
                egui::StrokeKind::Middle,
            ));
            let mut shackle: Vec<(f32, f32)> = vec![(5.25, 7.0), (5.25, 5.0)];
            shackle.extend(arc(8.0, 5.0, 2.75, std::f32::consts::PI, 2.0 * std::f32::consts::PI));
            if icon == Icon::Lock {
                shackle.push((10.75, 7.0));
            }
            shapes.push(line(&shackle));
        }
        Icon::Speaker | Icon::SpeakerOff => {
            shapes.push(closed(&[(1.75, 6.0), (4.5, 6.0), (8.0, 2.75), (8.0, 13.25), (4.5, 10.0), (1.75, 10.0)]));
            if icon == Icon::Speaker {
                shapes.push(line(&arc(8.5, 8.0, 3.0, -0.8, 0.8)));
                shapes.push(line(&arc(8.5, 8.0, 5.5, -0.8, 0.8)));
            } else {
                shapes.push(line(&[(10.5, 5.75), (14.75, 10.25)]));
                shapes.push(line(&[(14.75, 5.75), (10.5, 10.25)]));
            }
        }
        Icon::Media => {
            shapes.push(Shape::rect_stroke(
                rect_of(1.5, 2.5, 14.5, 13.5),
                r(1.75),
                stroke,
                egui::StrokeKind::Middle,
            ));
            shapes.push(line(&[(1.75, 11.5), (5.5, 7.5), (8.5, 10.5), (10.5, 8.5), (14.25, 12.25)]));
            shapes.push(Shape::circle_filled(map(11.0, 5.5), r(1.25), colour));
        }
        Icon::Music => {
            shapes.push(line(&[(6.0, 12.0), (6.0, 2.75), (14.0, 1.5), (14.0, 10.5)]));
            shapes.push(Shape::circle_filled(map(4.0, 12.25), r(2.25), colour));
            shapes.push(Shape::circle_filled(map(12.0, 10.75), r(2.25), colour));
        }
        Icon::Text => {
            shapes.push(line(&[(2.5, 4.25), (2.5, 2.5), (13.5, 2.5), (13.5, 4.25)]));
            shapes.push(line(&[(8.0, 2.5), (8.0, 13.75)]));
            shapes.push(line(&[(5.75, 13.75), (10.25, 13.75)]));
        }
        Icon::Sticker => {
            shapes.push(Shape::circle_stroke(map(8.0, 8.0), r(6.25), stroke));
            shapes.push(Shape::circle_filled(map(5.75, 6.5), r(0.95), colour));
            shapes.push(Shape::circle_filled(map(10.25, 6.5), r(0.95), colour));
            shapes.push(line(&arc(8.0, 8.0, 3.5, 0.5, std::f32::consts::PI - 0.5)));
        }
        Icon::Effects => {
            // A four-pointed sparkle and a small one.
            shapes.push(closed(&[
                (7.0, 1.5),
                (8.5, 5.5),
                (12.5, 7.0),
                (8.5, 8.5),
                (7.0, 12.5),
                (5.5, 8.5),
                (1.5, 7.0),
                (5.5, 5.5),
            ]));
            shapes.push(line(&[(12.5, 10.5), (12.5, 14.5)]));
            shapes.push(line(&[(10.5, 12.5), (14.5, 12.5)]));
        }
        Icon::Transitions => {
            shapes.push(Shape::rect_stroke(
                rect_of(1.5, 3.5, 10.0, 12.5),
                r(1.5),
                stroke,
                egui::StrokeKind::Middle,
            ));
            shapes.push(line(&[(10.0, 5.5), (14.5, 5.5), (14.5, 14.5), (6.0, 14.5), (6.0, 12.5)]));
        }
        Icon::Filters => {
            shapes.push(Shape::circle_stroke(map(8.0, 5.75), r(3.75), stroke));
            shapes.push(Shape::circle_stroke(map(5.5, 10.25), r(3.75), stroke));
            shapes.push(Shape::circle_stroke(map(10.5, 10.25), r(3.75), stroke));
        }
        Icon::Reset => {
            let mut a = arc(8.0, 8.5, 5.5, -2.2, 3.0);
            a.reverse();
            shapes.push(line(&a));
            shapes.push(line(&[(1.25, 2.5), (2.6, 4.6), (5.0, 3.75)]));
        }
        Icon::Keyframe => {
            shapes.push(closed(&[(8.0, 2.0), (14.0, 8.0), (8.0, 14.0), (2.0, 8.0)]));
        }
        Icon::Close => {
            shapes.push(line(&[(3.5, 3.5), (12.5, 12.5)]));
            shapes.push(line(&[(12.5, 3.5), (3.5, 12.5)]));
        }
    }
    painter.extend(shapes);
}

/// The icon's own drawing size inside a button, in points.
pub const ICON_SIZE: f32 = 16.0;

/// A square button showing only `icon`, with `tip` on hover. Quiet until the
/// pointer is on it; `selected` shows it as switched on.
pub fn button(ui: &mut egui::Ui, icon: Icon, selected: bool, tip: &str) -> egui::Response {
    button_sized(ui, icon, selected, tip, crate::theme::ICON_BUTTON)
}

/// [`button`] at another size: a larger one for Play.
pub fn button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    selected: bool,
    tip: &str,
    side: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::click());
    let response = response.on_hover_text(tip);
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let selected = selected || menu_open(ui, &response);
        let (fill, colour) = crate::theme::icon_button_colours(&response, selected, enabled);
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, crate::theme::RADIUS_SMALL, fill);
        }
        let inner = (side * 0.55).clamp(12.0, 22.0);
        paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(inner)),
            icon,
            colour,
        );
    }
    response
}

/// The app's mark at the start of the toolbar: the icon's rounded square in
/// the accent, with the play triangle cut through, at toolbar size.
pub fn app_mark(ui: &mut egui::Ui) {
    let side = 20.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
    response.on_hover_text("bettercut");
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, crate::theme::accent());
    let unit = side / 16.0;
    let at = |x: f32, y: f32| rect.min + vec2(x * unit, y * unit);
    painter.add(Shape::convex_polygon(
        vec![at(6.0, 4.25), at(12.25, 8.0), at(6.0, 11.75)],
        Color32::WHITE,
        Stroke::NONE,
    ));
    // The cut: a diagonal gap through the triangle, in the accent.
    painter.line_segment(
        [at(6.5, 12.0), at(11.0, 4.0)],
        Stroke::new(unit * 1.1, crate::theme::accent()),
    );
}

/// Play and Pause: round, a size up from the other transport buttons, and
/// filled, because it is the one pressed most.
pub fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let side = 34.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::click());
    let response = response.on_hover_text(if playing { "Pause (Space)" } else { "Play (Space)" });
    if ui.is_rect_visible(rect) {
        let p = crate::theme::palette();
        let fill = if response.is_pointer_button_down_on() {
            p.pressed
        } else if response.hovered() {
            p.hover
        } else {
            p.control
        };
        ui.painter().circle_filled(rect.center(), side / 2.0, fill);
        let icon = Rect::from_center_size(
            rect.center() + vec2(if playing { 0.0 } else { 1.0 }, 0.0),
            Vec2::splat(15.0),
        );
        paint(
            ui.painter(),
            icon,
            if playing { Icon::Pause } else { Icon::Play },
            p.text_strong,
        );
    }
    response
}

/// Whether the menu `response`'s button opens is open now.
fn menu_open(ui: &egui::Ui, response: &egui::Response) -> bool {
    egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(response))
}

/// An icon button that opens a menu of `contents` when clicked, drawn
/// switched on while the menu is open.
pub fn menu<R>(
    ui: &mut egui::Ui,
    icon: Icon,
    tip: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let response = button(ui, icon, false, tip);
    egui::Popup::menu(&response).show(contents);
    response
}

/// [`labelled_button`] that opens a menu of `contents`.
pub fn labelled_menu<R>(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    tip: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let response = labelled_button(ui, icon, text, false, tip);
    egui::Popup::menu(&response).show(contents);
    response
}

/// An icon and a word together, for the few actions whose icon alone would
/// be a guess: drawn like a button, icon first.
pub fn labelled_button(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    selected: bool,
    tip: &str,
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let height = crate::theme::ICON_BUTTON;
    let width = 8.0 + ICON_SIZE + 6.0 + galley.size().x + 10.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::click());
    let response = response.on_hover_text(tip);
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let selected = selected || menu_open(ui, &response);
        let (fill, colour) = crate::theme::icon_button_colours(&response, selected, enabled);
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, crate::theme::RADIUS_SMALL, fill);
        }
        let icon_rect = Rect::from_min_size(
            pos2(rect.left() + 8.0, rect.center().y - ICON_SIZE / 2.0),
            Vec2::splat(ICON_SIZE),
        );
        paint(ui.painter(), icon_rect.shrink(1.0), icon, colour);
        ui.painter().galley_with_override_text_color(
            pos2(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0),
            galley,
            colour,
        );
    }
    response
}

/// Every icon, for the test that draws them all.
pub const ALL: [Icon; 54] = [
    Icon::NewFile,
    Icon::Folder,
    Icon::Save,
    Icon::Undo,
    Icon::Redo,
    Icon::Export,
    Icon::QuickExport,
    Icon::Plus,
    Icon::Captions,
    Icon::Search,
    Icon::Command,
    Icon::Windows,
    Icon::Sliders,
    Icon::Play,
    Icon::Pause,
    Icon::ToStart,
    Icon::ToEnd,
    Icon::Back,
    Icon::Forward,
    Icon::Loop,
    Icon::Fullscreen,
    Icon::Record,
    Icon::Camera,
    Icon::More,
    Icon::ChevronDown,
    Icon::ChevronRight,
    Icon::Split,
    Icon::Blade,
    Icon::Trash,
    Icon::TrimStart,
    Icon::TrimEnd,
    Icon::Freeze,
    Icon::Marker,
    Icon::Snap,
    Icon::Magnet,
    Icon::ZoomIn,
    Icon::ZoomOut,
    Icon::Fit,
    Icon::Eye,
    Icon::EyeOff,
    Icon::Lock,
    Icon::Unlock,
    Icon::Speaker,
    Icon::SpeakerOff,
    Icon::Media,
    Icon::Music,
    Icon::Text,
    Icon::Sticker,
    Icon::Effects,
    Icon::Transitions,
    Icon::Filters,
    Icon::Reset,
    Icon::Keyframe,
    Icon::Close,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon draws something, inside its box, without panicking.
    #[test]
    fn every_icon_draws_inside_its_box() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(100.0, 100.0), Vec2::splat(16.0));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for icon in ALL {
                paint(ui.painter(), rect, icon, Color32::WHITE);
            }
        });
        output.textures_delta.clear();
        // Every shape lands within the box, give or take a stroke's width.
        let room = rect.expand(2.0);
        for clipped in &output.shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            if bounds.is_positive() {
                assert!(room.contains_rect(bounds), "{bounds:?} spills out of {rect:?}");
            }
        }
    }
}
