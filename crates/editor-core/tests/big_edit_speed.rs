//! Big edits stay quick: a thousand clips cut, undone, redone and deleted,
//! each in well under the time a person would notice. Times are printed, so a
//! slowdown shows before a bound fails.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};

fn timed<T>(what: &str, work: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    let out = work();
    let took = began.elapsed();
    eprintln!("{what}: {took:?}");
    assert!(took < Duration::from_secs(5), "{what} took {took:?}");
    out
}

#[test]
fn a_thousand_clips_cut_undo_redo_and_delete_quickly() {
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
    editor.place_media(media).unwrap();
    let clip = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
    let cuts: Vec<_> = (1..1000)
        .map(|i| TimelineTime::from_millis(i * 600))
        .collect();

    timed("split 999 ways", || {
        editor.split_clip_at(clip, &cuts).unwrap()
    });
    let clips: Vec<_> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(clips.len(), 1000);

    timed("undo it", || editor.undo().unwrap());
    timed("redo it", || editor.redo().unwrap());

    // Five hundred chosen and deleted at once, as the interface does it: one
    // step. (One by one they cost a durable disk write each — right for an
    // edit a person makes, and nobody makes five hundred in a second.)
    let sequence = editor.active_sequence().unwrap().id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let removals: Vec<_> = clips
        .iter()
        .step_by(2)
        .map(|clip| bettercut_editor_core::Command::RemoveClip {
            sequence,
            track,
            clip: *clip,
        })
        .collect();
    timed("delete 500 at once", || {
        editor.dispatch_group("Delete Selection", removals).unwrap();
    });
    timed("undo the delete", || editor.undo().unwrap());
    timed("save", || {
        let file = std::env::temp_dir().join(format!("bettercut-big-{}.vproj", std::process::id()));
        editor.save_as(&file).unwrap();
        let _ = std::fs::remove_file(&file);
    });
}
