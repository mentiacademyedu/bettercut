//! Headless tests for the Inspector panel.
//!
//! The clip-property controls read the project immutably to draw and then
//! dispatch commands mutably, in one function. That is exactly the shape that
//! panics or fails to compile when rearranged carelessly, and nothing else
//! exercises it — the property *commands* are tested in editor-core, but not
//! the panel that drives them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn editor_with_clips() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Inspector");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();

    let video = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let video_id = video.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(video)))
        .unwrap();

    let audio = AudioClip::new(media, TimelineTime::ZERO, source).unwrap();
    let audio_id = audio.id;
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(audio)))
        .unwrap();

    (editor, video_id, audio_id)
}

/// Draw the inspector once. Returns nothing — the assertion is that it neither
/// panics nor leaves egui in a broken state.
fn draw(editor: &mut Editor, state: &mut UiState) {
    let _ = drawn_text(editor, state);
}

/// Draw the inspector and return every word it put on screen.
///
/// Without this a panel test passes whether or not the panel it names was ever
/// reached: "did not panic" is true of the empty case too. Reading the galleys
/// back is the only way from here to assert that the right controls appeared.
fn drawn_text(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 900.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::panels::inspector(ui, editor, state);
    });
    output.textures_delta.clear();

    let mut words = String::new();
    for clipped in &output.shapes {
        collect_text(&clipped.shape, &mut words);
    }
    words
}

fn collect_text(shape: &egui::Shape, into: &mut String) {
    match shape {
        egui::Shape::Text(text) => {
            into.push_str(text.galley.text());
            // A separator, so two adjacent labels cannot form a third word that
            // a `contains` check would match by accident.
            into.push(' ');
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, into);
            }
        }
        _ => {}
    }
}

#[test]
fn the_inspector_draws_with_nothing_selected() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_video_property_controls() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_audio_property_controls() {
    let (mut editor, _, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_with_several_clips_selected() {
    let (mut editor, video, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

/// A selection pointing at a clip that no longer exists is normal — delete
/// leaves the id behind for a frame — and must not panic.
#[test]
fn a_stale_selection_does_not_panic() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(ClipId::new());
    draw(&mut editor, &mut state);
}

/// Drawing must never mutate the project by itself. Only user input does.
#[test]
fn drawing_changes_nothing() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);

    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    draw(&mut editor, &mut state);

    assert_eq!(
        editor.project(),
        &before,
        "drawing the inspector modified the project"
    );
}

/// A transformed clip shows the Reset button, which is a different code path
/// from the identity case.
#[test]
fn the_inspector_draws_a_transformed_clip() {
    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_clip_property(
            video,
            bettercut_editor_core::ClipProperty::Scale { x: 0.5, y: 0.5 },
            false,
        )
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}

/// Every tab draws for a video clip without panicking, and the tab actually
/// changes what is shown.
#[test]
fn every_inspector_tab_draws() {
    use bettercut_ui::panels::InspectorTab;

    // Audio is not among them: video and audio are separate clips on separate
    // tracks (§8), so a video clip genuinely has no sound and the tab correctly
    // falls back — which the next test checks.
    for tab in [
        InspectorTab::Video,
        InspectorTab::Colours,
        InspectorTab::Speed,
        InspectorTab::Animation,
    ] {
        let (mut editor, video, _) = editor_with_clips();
        let mut state = UiState::default();
        state.selected_clips.insert(video);
        state.inspector_tab = tab;
        draw(&mut editor, &mut state);
        assert_eq!(
            state.inspector_tab,
            tab,
            "{} switched away from itself for a video clip",
            tab.label()
        );
    }

    // And every tab draws for an audio clip too, whichever one it lands on.
    for tab in InspectorTab::ALL {
        let (mut editor, _, audio) = editor_with_clips();
        let mut state = UiState::default();
        state.selected_clips.insert(audio);
        state.inspector_tab = tab;
        draw(&mut editor, &mut state);
    }
}

/// A tab the selection cannot offer must not strand the user on an explanation
/// with no way back. Selecting an audio clip while Video is open moves to a tab
/// that exists.
#[test]
fn an_impossible_tab_falls_back_to_one_that_works() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, _, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(audio);
    state.inspector_tab = InspectorTab::Video;

    draw(&mut editor, &mut state);

    assert_eq!(
        state.inspector_tab,
        InspectorTab::Audio,
        "an audio clip left the Inspector on the Video tab"
    );
}

/// Speed has nothing behind it yet, so it must stay reachable rather than being
/// bounced away — the tab is a statement that it is coming.
#[test]
fn the_speed_tab_stays_selected_even_though_it_is_empty() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.inspector_tab = InspectorTab::Speed;

    draw(&mut editor, &mut state);
    assert_eq!(state.inspector_tab, InspectorTab::Speed);
}

/// A clip with a cut after it shows §25's transition section, which is a
/// different code path from the last clip on a track.
#[test]
fn the_inspector_draws_the_transition_section() {
    use bettercut_editor_core::timeline::TransitionKind;

    let (mut editor, video, _) = editor_with_clips();
    let media = editor.project().media[0].id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let next = VideoClip::new(
        media,
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap(),
    )
    .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(next)))
        .unwrap();
    editor
        .set_transition(video, TransitionKind::Crossfade)
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(video);

    // Twice, and asserting the project is untouched: the duration slider
    // dispatches a command when it changes, and a control that reported a
    // change every frame would rewrite the transition on every repaint —
    // filling the undo history from an idle window.
    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(
        editor.project(),
        &before,
        "drawing the transition section modified the project"
    );
}

/// §26: a title takes its own panel — words and a font, not a source range and
/// a grade. Drawing it twice must change nothing, because the text box and the
/// sliders all dispatch commands when they report a change.
#[test]
fn the_inspector_draws_a_text_clip() {
    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(
        editor.project(),
        &before,
        "drawing the text panel modified the project"
    );

    assert!(
        words.contains("Legibility"),
        "the text panel was never reached; drew: {words}"
    );
    assert!(
        !words.contains("blur"),
        "the video clip's controls appeared for a title"
    );
}

/// Every decoration draws: an outline, a shadow and a background each add
/// controls of their own, and those are the branches a smoke test misses.
#[test]
fn the_inspector_draws_a_fully_decorated_title() {
    use bettercut_editor_core::TextProperty;
    use bettercut_editor_core::text::{Background, Shadow, Stroke, TextStyle};

    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();
    editor
        .set_text_property(
            title,
            TextProperty::Style(Box::new(TextStyle {
                stroke: Some(Stroke::default()),
                shadow: Some(Shadow::default()),
                background: Some(Background::default()),
                ..TextStyle::default()
            })),
            false,
        )
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(editor.project(), &before);

    for control in ["outline", "shadow", "background", "blur"] {
        assert!(
            words.contains(control),
            "the {control} controls did not draw; drew: {words}"
        );
    }
}
