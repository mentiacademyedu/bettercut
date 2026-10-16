//! The Keyframe Curve window (§24): how the keys under the playhead ease into
//! the next ones, picked from named curves or drawn by dragging two handles.
//!
//! A row of words — "ease in", "ease out" — says little about the move it
//! makes; the curve itself says it at a glance, and dragging its handles is
//! how a move gets the overshoot or the pull-back the presets do not have.
//! What the window shows is always the curve of the keys under the playhead,
//! read afresh each frame, so moving the playhead onto another key shows that
//! key's curve; every pick, and every drag when it is let go, is one step to
//! undo.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::ClipId;
use bettercut_editor_core::timeline::Interpolation;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::state::UiState;
use crate::theme;

/// The window, for one clip's keys.
#[derive(Debug)]
pub struct CurveEditor {
    clip: ClipId,
    /// The handles while one is being dragged: `[x1, y1, x2, y2]`, and which.
    dragging: Option<([f32; 4], usize)>,
}

impl CurveEditor {
    pub fn open(clip: ClipId) -> Self {
        Self {
            clip,
            dragging: None,
        }
    }
}

/// How far a handle may be dragged above and below the two key values: room
/// for an overshoot or a pull-back, not for a curve that leaves the screen.
const LOW: f32 = -0.8;
const HIGH: f32 = 1.8;

/// The curve of the keys under the playhead on `clip`, or `None` when no key
/// is there.
fn curve_here(editor: &Editor, clip: ClipId) -> Option<Interpolation> {
    let at = editor.source_time_at_playhead(clip)?;
    editor.video_clip(clip)?.keyframes.interpolation_at(at)
}

/// Draw the window, if one is open, and act on what is picked.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(mut window) = state.curve_editor.take() else {
        return;
    };
    if editor.video_clip(window.clip).is_none() {
        return;
    }
    let here = curve_here(editor, window.clip);
    let mut open = true;
    let mut chosen = None;

    egui::Window::new("Keyframe Curve")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - vec2(160.0, 220.0))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            let Some(current) = here else {
                ui.label("Move the playhead onto one of this clip's keyframes.");
                ui.label(
                    egui::RichText::new("The curve is how the move leaves that key.")
                        .small()
                        .color(theme::text_muted()),
                );
                return;
            };

            // The named curves, each drawn small.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (name, curve) in Interpolation::CURVES {
                    if curve_tile(ui, name, curve, current == curve).clicked() {
                        chosen = Some(curve);
                    }
                }
            });
            ui.add_space(8.0);

            // The curve itself, with its handles to drag.
            if let Some(dropped) = graph(ui, current, &mut window.dragging) {
                chosen = Some(dropped);
            }
            ui.label(
                egui::RichText::new(match (window.dragging, current) {
                    (Some(_), _) => "Let go to apply.".to_owned(),
                    (None, Interpolation::Hold) => {
                        "Hold — the value waits, then jumps. Pick a curve above to ease it."
                            .to_owned()
                    }
                    (None, _) => format!(
                        "{} — drag a handle to shape it; past the lines it overshoots.",
                        capitalised(current.label())
                    ),
                })
                .small()
                .color(theme::text_muted()),
            );
        });

    if let Some(curve) = chosen {
        match editor.set_keyframe_easing(window.clip, curve) {
            Ok(_) => state.info(format!("Keys here are now {}", curve.label())),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    if open {
        state.curve_editor = Some(window);
    }
}

fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

/// One named curve as a small picture with its name under it.
fn curve_tile(ui: &mut egui::Ui, name: &str, curve: Interpolation, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
    let painter = ui.painter_at(rect);
    let fill = if on {
        theme::accent().gamma_multiply(0.35)
    } else if response.hovered() {
        theme::hover()
    } else {
        theme::control()
    };
    painter.rect_filled(rect, theme::RADIUS_SMALL, fill);
    let plot = Rect::from_min_max(rect.min + vec2(10.0, 9.0), rect.max - vec2(10.0, 21.0));
    let colour = if on {
        theme::text_strong()
    } else {
        theme::text()
    };
    draw_curve(&painter, plot, curve, Stroke::new(1.5, colour), 0.25);
    painter.text(
        pos2(rect.center().x, rect.bottom() - 9.0),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(9.5),
        colour,
    );
    response.on_hover_text(capitalised(name))
}

/// Where a point of the curve's square lands in `plot`: progress `t` along,
/// value `y` up, with `margin` (in squares) of room above and below.
fn place(plot: Rect, t: f32, y: f32, margin: f32) -> Pos2 {
    let span = 1.0 + 2.0 * margin;
    pos2(
        plot.left() + t * plot.width(),
        plot.bottom() - (y + margin) / span * plot.height(),
    )
}

fn draw_curve(
    painter: &egui::Painter,
    plot: Rect,
    curve: Interpolation,
    stroke: Stroke,
    margin: f32,
) {
    let points: Vec<Pos2> = if curve == Interpolation::Hold {
        // A step: the value held, then the jump at the next key.
        vec![
            place(plot, 0.0, 0.0, margin),
            place(plot, 1.0, 0.0, margin),
            place(plot, 1.0, 1.0, margin),
        ]
    } else {
        (0..=40)
            .map(|step| {
                let t = step as f32 / 40.0;
                place(plot, t, curve.ease(t), margin)
            })
            .collect()
    };
    painter.add(egui::Shape::line(points, stroke));
}

/// The big curve: the two key values as lines, the curve between them, and
/// the handles. Returns the curve when a dragged handle is let go.
fn graph(
    ui: &mut egui::Ui,
    current: Interpolation,
    dragging: &mut Option<([f32; 4], usize)>,
) -> Option<Interpolation> {
    let (rect, _) = ui.allocate_exact_size(vec2(320.0, 230.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::RADIUS_SMALL, theme::canvas());
    let plot = Rect::from_min_max(rect.min + vec2(38.0, 12.0), rect.max - vec2(16.0, 12.0));
    // The square from key to key spans the middle of the plot; above and
    // below is the room handles may be dragged into.
    let margin = -LOW;
    debug_assert!((HIGH - 1.0 - margin).abs() < 1e-6);
    let at = |t: f32, y: f32| place(plot, t, y, margin);
    let line = Stroke::new(1.0, theme::border());
    for y in [0.0, 1.0] {
        painter.line_segment([at(0.0, y), at(1.0, y)], line);
    }
    let small = egui::FontId::proportional(10.0);
    painter.text(
        at(0.0, 1.0) + vec2(-4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "to",
        small.clone(),
        theme::text_faint(),
    );
    painter.text(
        at(0.0, 0.0) + vec2(-4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "from",
        small,
        theme::text_faint(),
    );

    let handles = match dragging {
        Some((handles, _)) => Some(*handles),
        None => current.handles(),
    };
    let Some(mut handles) = handles else {
        // Hold has no handles: a step, and a word on how to get a curve.
        draw_curve(
            &painter,
            plot,
            current,
            Stroke::new(2.0, theme::keyframe()),
            margin,
        );
        return None;
    };
    let shown = Interpolation::Bezier {
        x1: handles[0],
        y1: handles[1],
        x2: handles[2],
        y2: handles[3],
    };

    // Each handle hangs from the key it belongs to.
    let ends = [at(0.0, 0.0), at(1.0, 1.0)];
    let mut released = None;
    for which in 0..2 {
        let point = at(handles[which * 2], handles[which * 2 + 1]);
        painter.line_segment([ends[which], point], Stroke::new(1.0, theme::text_muted()));
        let id = ui.id().with(("curve handle", which));
        let response = ui.interact(
            Rect::from_center_size(point, Vec2::splat(18.0)),
            id,
            Sense::drag(),
        );
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let t = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            let span = 1.0 + 2.0 * margin;
            let y = ((plot.bottom() - pointer.y) / plot.height() * span - margin).clamp(LOW, HIGH);
            handles[which * 2] = t;
            handles[which * 2 + 1] = y;
            *dragging = Some((handles, which));
        }
        if response.drag_stopped() {
            released = Some(handles);
            *dragging = None;
        }
        let active = response.hovered() || response.dragged();
        painter.circle(
            point,
            if active { 6.0 } else { 5.0 },
            if active {
                theme::accent()
            } else {
                theme::text_strong()
            },
            Stroke::new(1.5, Color32::from_black_alpha(160)),
        );
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    draw_curve(
        &painter,
        plot,
        shown,
        Stroke::new(2.0, theme::keyframe()),
        margin,
    );
    for end in ends {
        painter.circle_filled(end, 3.5, theme::keyframe());
    }

    released.map(|[x1, y1, x2, y2]| {
        let drawn = Interpolation::Bezier { x1, y1, x2, y2 };
        // Dragged back onto a named curve: called by its name.
        Interpolation::CURVES
            .into_iter()
            .map(|(_, curve)| curve)
            .find(|curve| curve.handles() == Some([x1, y1, x2, y2]))
            .unwrap_or(drawn)
    })
}
