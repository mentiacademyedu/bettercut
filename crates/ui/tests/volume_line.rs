//! The volume envelope drawn across a sound clip (§24, §53).
//!
//! Ducking is invisible without this: the music gets quieter under the voice
//! and the timeline says nothing about why. The line is sampled from the same
//! `gain_at` the mixer asks, so it draws what is *heard* rather than what the
//! keys mean — a duck that never reached the mixer would draw flat.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Color32, Pos2, RawInput, Rect, vec2};

/// Mirrors `theme::automation()`.
fn automation() -> Color32 {
    bettercut_ui::theme::automation()
}

/// Thirty seconds of sound on A1 from timeline zero.
fn setup() -> (bettercut_editor_core::Editor, UiState, ClipId) {
    let (mut editor, _events) = bettercut_editor_core::Editor::new_project("Envelope");
    let mut asset = MediaAsset::new(
        MediaKind::Audio,
        "C:/media/bed.wav",
        MediaTime::from_seconds(30),
    );
    asset.audio_codec = Some("pcm".to_owned());
    let media = editor.import_media(asset);
    let sound = editor.place_media(media).unwrap()[0];
    (editor, UiState::default(), sound)
}

/// Draw the timeline and return every point of every line drawn in the
/// automation colour.
fn envelope_points(editor: &mut bettercut_editor_core::Editor, state: &mut UiState) -> Vec<Pos2> {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 600.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::timeline::draw(ui, editor, state);
    });
    output.textures_delta.clear();

    let mut points = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut points);
    }
    points
}

fn collect(shape: &egui::Shape, points: &mut Vec<Pos2>) {
    match shape {
        egui::Shape::Path(path)
            if path.stroke.color == egui::epaint::ColorMode::Solid(automation()) =>
        {
            points.extend_from_slice(&path.points);
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, points);
            }
        }
        _ => {}
    }
}

#[test]
fn a_clip_with_no_envelope_draws_no_line() {
    let (mut editor, mut state, _sound) = setup();
    assert!(
        envelope_points(&mut editor, &mut state).is_empty(),
        "a line was drawn across a clip whose volume never moves"
    );
}

/// The shape of the duck reaches the screen: the line sits high where the
/// music is up and dips where it is ducked.
#[test]
fn the_line_dips_where_the_duck_is() {
    let (mut editor, mut state, sound) = setup();
    editor
        .set_gain_envelope(
            sound,
            &[
                (TimelineTime::from_seconds(0), 1.0),
                (TimelineTime::from_seconds(10), 1.0),
                (TimelineTime::from_seconds(12), 0.25),
                (TimelineTime::from_seconds(18), 0.25),
                (TimelineTime::from_seconds(20), 1.0),
            ],
            false,
        )
        .unwrap();

    let points = envelope_points(&mut editor, &mut state);
    assert!(!points.is_empty(), "the envelope drew nothing");

    // Lower on screen is a bigger y, so the ducked part must be *below* the
    // rest of the line.
    let highest = points.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let lowest = points.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    assert!(
        lowest - highest > 8.0,
        "the line is flat: it spans {:.1} px, so the duck is not being drawn",
        lowest - highest
    );

    // And it dips in the middle rather than at an end: the duck is at 12-18 s
    // of a thirty-second clip.
    let (left, right) = (
        points.iter().map(|p| p.x).fold(f32::MAX, f32::min),
        points.iter().map(|p| p.x).fold(f32::MIN, f32::max),
    );
    let deepest = points
        .iter()
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .expect("a point");
    let across = (deepest.x - left) / (right - left);
    assert!(
        (0.3..0.75).contains(&across),
        "the dip is {across:.2} of the way across the clip, not where the duck is"
    );
}

/// The line is drawn from what the mixer would ask, so a clip whose envelope
/// holds one level draws a straight line — not the flat *default* line, a line
/// at that level.
#[test]
fn a_held_level_draws_below_full_volume() {
    let (mut editor, mut state, sound) = setup();
    editor
        .set_gain_envelope(
            sound,
            &[
                (TimelineTime::from_seconds(0), 0.5),
                (TimelineTime::from_seconds(30), 0.5),
            ],
            false,
        )
        .unwrap();
    let half = envelope_points(&mut editor, &mut state);

    editor
        .set_gain_envelope(
            sound,
            &[
                (TimelineTime::from_seconds(0), 1.0),
                (TimelineTime::from_seconds(30), 1.0),
            ],
            false,
        )
        .unwrap();
    let full = envelope_points(&mut editor, &mut state);

    let mean = |points: &[Pos2]| points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32;
    assert!(
        mean(&half) > mean(&full) + 8.0,
        "half volume drew at {:.1} and full volume at {:.1}: the level is not \
         reaching the line",
        mean(&half),
        mean(&full)
    );
}
