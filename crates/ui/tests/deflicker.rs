//! Remove flicker through the interface: a measurement that finishes is put
//! on its clip as one undo step, the clip's brightness then follows the
//! gains frame by frame, and the button takes it off again.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::ClipPayload;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::deflicker::FlickerGains;
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_playback::FlickerReport;
use bettercut_ui::UiState;

fn editor_with_clip() -> (Editor, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Flicker");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/lights.mp4",
        MediaTime::from_seconds(10),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap();
    let clip = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let id = clip.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .unwrap();
    (editor, id)
}

fn frame(index: i64) -> MediaTime {
    MediaTime::from_ticks(index * MediaTime::from_seconds(1).ticks() / 30)
}

/// Gains measured from footage pulsing every other frame.
fn pulse() -> FlickerGains {
    let frames: Vec<(MediaTime, f32)> = (0..60)
        .map(|index| (frame(index), if index % 2 == 0 { 0.3 } else { 0.36 }))
        .collect();
    FlickerGains::from_brightness(&frames).unwrap()
}

fn brightness(editor: &Editor, clip: ClipId, index: i64) -> f32 {
    editor
        .video_clip(clip)
        .unwrap()
        .look_at(frame(index))
        .color
        .brightness
}

#[test]
fn a_finished_measurement_evens_the_clip_out_in_one_step() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    let ctx = egui::Context::default();

    // A job still running: nothing yet.
    let report = FlickerReport::default();
    state.flicker_jobs.push((clip, report.clone()));
    bettercut_ui::panels::collect_flicker(&ctx, &mut editor, &mut state);
    assert!(editor.video_clip(clip).unwrap().deflicker.is_none());
    assert_eq!(state.flicker_jobs.len(), 1, "a running job was dropped");

    // It finishes: the gains go on, and the job is done with.
    report.finish(Ok(pulse()));
    bettercut_ui::panels::collect_flicker(&ctx, &mut editor, &mut state);
    assert!(editor.video_clip(clip).unwrap().deflicker.is_some());
    assert!(state.flicker_jobs.is_empty());

    // The bright frames come down and the dim ones up.
    assert!(
        brightness(&editor, clip, 30) > 1.0,
        "a dim frame was not lifted"
    );
    assert!(
        brightness(&editor, clip, 31) < 1.0,
        "a bright frame was not lowered"
    );

    editor.undo().unwrap();
    assert!(
        editor.video_clip(clip).unwrap().deflicker.is_none(),
        "not one step"
    );
    assert_eq!(brightness(&editor, clip, 30), 1.0);
}

#[test]
fn the_button_takes_it_off_again() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    editor.set_flicker(clip, Some(pulse())).unwrap();

    bettercut_ui::panels::start_or_restore_flicker(&mut editor, &mut state, clip);
    assert!(editor.video_clip(clip).unwrap().deflicker.is_none());
    assert!(
        state.flicker_request.is_none(),
        "restoring started a measurement"
    );

    // And without gains, the same button asks for a measurement.
    bettercut_ui::panels::start_or_restore_flicker(&mut editor, &mut state, clip);
    assert_eq!(state.flicker_request, Some(clip));
}

#[test]
fn a_failed_measurement_says_why_and_changes_nothing() {
    let (mut editor, clip) = editor_with_clip();
    let mut state = UiState::default();
    let report = FlickerReport::default();
    report.finish(Err("the clip is too short to measure flicker in".to_owned()));
    state.flicker_jobs.push((clip, report));
    bettercut_ui::panels::collect_flicker(&egui::Context::default(), &mut editor, &mut state);
    assert!(editor.video_clip(clip).unwrap().deflicker.is_none());
    assert!(state.flicker_jobs.is_empty());
}
