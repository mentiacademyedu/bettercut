//! Find Cuts, through the window (Milestone 12).
//!
//! The window's job is to wait for a worker without looking broken, then show
//! what was found and cut only when told. The detection itself is tested in
//! `playback`; this is about what the user sees and what happens when they say
//! yes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_jobs::JobId;
use bettercut_playback::SceneReport;
use bettercut_ui::UiState;
use bettercut_ui::scene_dialog::SceneDialog;
use egui::{Pos2, RawInput, Rect, vec2};

/// A ten-second video clip with its sound.
fn setup() -> (Editor, UiState, ClipId) {
    let (mut editor, _events) = Editor::new_project("Scenes");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/reel.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (editor, UiState::default(), placed[0])
}

/// Draw the window and return the words it drew.
fn frame(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let mut words = String::new();
    for _ in 0..2 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            bettercut_ui::scene_dialog::show(ui.ctx(), editor, state);
        });
        output.textures_delta.clear();
        words.clear();
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                words.push_str(text.galley.text());
                words.push(' ');
            }
        }
    }
    words
}

#[test]
fn the_window_waits_and_then_says_what_it_found() {
    let (mut editor, mut state, clip) = setup();
    let report = SceneReport::default();
    state.scenes = Some(SceneDialog::started(clip, JobId(1), report.clone()));

    // While the worker is still reading the file. The flag starts set — a
    // fresh interface always draws once — so clear it first, or "the window
    // asked for a repaint" is a question about the default, not the window.
    state.needs_repaint = false;
    let words = frame(&mut editor, &mut state);
    assert!(words.contains("Looking through"), "{words}");
    assert!(
        state.scenes.as_ref().is_some_and(|d| d.cuts().is_empty()),
        "cuts were offered before the job answered"
    );
    assert!(
        state.needs_repaint,
        "the window stopped asking to be drawn while waiting, so its spinner \
         would freeze until the user moved the mouse"
    );

    // The worker answers: two cuts, in source time.
    report.finish(vec![MediaTime::from_seconds(3), MediaTime::from_seconds(7)]);
    let words = frame(&mut editor, &mut state);
    assert!(words.contains("2 cuts found"), "{words}");
    assert!(words.contains("Split into 3"), "{words}");

    let dialog = state.scenes.as_ref().expect("the window closed by itself");
    assert_eq!(
        dialog.cuts(),
        [TimelineTime::from_seconds(3), TimelineTime::from_seconds(7)]
    );
}

#[test]
fn a_file_with_no_cuts_says_so_and_offers_nothing() {
    let (mut editor, mut state, clip) = setup();
    let report = SceneReport::default();
    report.finish(Vec::new());
    state.scenes = Some(SceneDialog::started(clip, JobId(1), report));

    let words = frame(&mut editor, &mut state);
    assert!(words.contains("No cuts found"), "{words}");
    assert!(words.contains("Nothing to split"), "{words}");
    assert!(state.scenes.as_ref().unwrap().cuts().is_empty());
}

/// The window reads the clip as it is *now*, not as it was when the job
/// started: detection takes seconds, and trimming during those seconds must not
/// leave a split hanging outside the clip.
#[test]
fn trimming_while_it_looks_keeps_the_cuts_inside_the_clip() {
    let (mut editor, mut state, clip) = setup();
    let report = SceneReport::default();
    state.scenes = Some(SceneDialog::started(clip, JobId(1), report.clone()));
    frame(&mut editor, &mut state);

    // The user trims the clip down to its first two seconds while waiting.
    let track = editor.track_of(clip).unwrap();
    editor
        .trim_clip(
            track,
            clip,
            bettercut_editor_core::TrimEdge::End,
            TimelineTime::from_seconds(2),
        )
        .unwrap();

    report.finish(vec![MediaTime::from_seconds(1), MediaTime::from_seconds(7)]);
    frame(&mut editor, &mut state);

    let dialog = state.scenes.as_ref().expect("the window closed by itself");
    assert_eq!(
        dialog.cuts(),
        [TimelineTime::from_seconds(1)],
        "a cut was offered past the end of the trimmed clip"
    );
}

/// What the window offers is what the editor does with it.
#[test]
fn confirming_splits_the_picture_and_its_sound() {
    let (mut editor, mut state, clip) = setup();
    let report = SceneReport::default();
    report.finish(vec![MediaTime::from_seconds(4)]);
    state.scenes = Some(SceneDialog::started(clip, JobId(1), report));
    frame(&mut editor, &mut state);

    let cuts = state.scenes.as_ref().unwrap().cuts().to_vec();
    assert_eq!(editor.split_clip_at(clip, &cuts).unwrap(), 1);

    let sequence = editor.active_sequence().unwrap();
    assert_eq!(sequence.video_tracks[0].len(), 2);
    assert_eq!(
        sequence.audio_tracks[0].len(),
        2,
        "§12: the sound stayed whole"
    );
}

/// Black at the ends is found by the same look, and comes off with the sound.
#[test]
fn black_at_the_ends_is_offered_and_trimmed_with_the_sound() {
    let (mut editor, mut state, clip) = setup();
    let report = SceneReport::default();
    report.finish_black(bettercut_playback::BlackEnds {
        head: Some(MediaTime::from_seconds(1)),
        tail: Some(MediaTime::from_seconds(8)),
    });
    report.finish(Vec::new());
    state.scenes = Some(SceneDialog::started(clip, JobId(1), report));

    let words = frame(&mut editor, &mut state);
    assert!(
        words.contains("1.0 s at the start, 2.0 s at the end"),
        "{words}"
    );
    assert!(words.contains("Trim Black"), "{words}");

    let depth = editor.undo_depth();
    editor
        .trim_ends(
            clip,
            Some(TimelineTime::from_seconds(1)),
            Some(TimelineTime::from_seconds(8)),
            "Trim Black",
        )
        .unwrap();
    assert_eq!(editor.undo_depth(), depth + 1, "one step");
    for member in [
        clip,
        editor
            .linked_with(clip)
            .into_iter()
            .find(|c| *c != clip)
            .unwrap(),
    ] {
        assert_eq!(
            editor.clip_start(member),
            Some(TimelineTime::from_seconds(1))
        );
        assert_eq!(editor.clip_end(member), Some(TimelineTime::from_seconds(8)));
    }
}
