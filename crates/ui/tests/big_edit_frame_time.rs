//! A big edit is still quick: cut into a thousand pieces in one go, and drawn, a thousand clips on the timeline, the
//! whole interface laid out headless, frame after frame.
//!
//! The bound is generous — it is a guard against a frame that walks every
//! clip several times over, not a benchmark — and the time is printed, so a
//! slower build shows before it fails.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::MediaTime;
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

#[test]
fn a_thousand_clips_draw_quickly() {
    let (mut editor, _events) = Editor::new_project("Big");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(600),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.audio_codec = Some("aac".to_owned());
    asset.audio_channels = Some(2);
    asset.audio_sample_rate = Some(48_000);
    let media = editor.import_media(asset);
    // One long clip, cut into a thousand pieces: picture and sound each.
    editor.place_media(media).unwrap();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    let cuts: Vec<_> = (1..1000)
        .map(|i| bettercut_editor_core::foundation::TimelineTime::from_millis(i * 600))
        .collect();
    let split_began = std::time::Instant::now();
    editor.split_clip_at(clip, &cuts).unwrap();
    let split = split_began.elapsed();
    eprintln!("splitting at {} points took {split:?}", cuts.len());
    // It took forty-five seconds when the journal synced every command.
    assert!(
        split < std::time::Duration::from_secs(5),
        "a {}-way split took {split:?}",
        cuts.len()
    );
    let count = editor.active_sequence().unwrap().clip_count();
    assert!(count >= 1000, "{count} clips");

    let mut state = UiState::default();
    let ctx = egui::Context::default();
    let frame = |editor: &mut Editor, state: &mut UiState| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| bettercut_ui::draw(ui, editor, state, None));
        output.textures_delta.clear();
    };
    // Warm up: fonts, layout caches.
    for _ in 0..3 {
        frame(&mut editor, &mut state);
    }
    // Zoomed out to the whole edit: every clip on screen at once.
    let length = editor.active_sequence().unwrap().duration();
    state.zoom_to_fit(length, 1200.0);
    let frames = 10;
    let began = std::time::Instant::now();
    for _ in 0..frames {
        frame(&mut editor, &mut state);
    }
    let each = began.elapsed() / frames;
    eprintln!("{count} clips: {each:?} a frame");
    assert!(
        each < std::time::Duration::from_millis(100),
        "{count} clips take {each:?} a frame"
    );
}
