//! §51's integration list, end to end, against real media files.
//!
//! ```text
//! Import video · Create project · Add clip · Reload project
//! ```
//!
//! Everything except "Trim clip" (Milestone 3) and "Export sequence"
//! (Milestone 6). Unlike the unit tests, these run the real FFmpeg prober
//! against real files, so a broken binding or a mis-copied DLL fails here.

// An integration test is a separate crate, so it does not inherit the
// `cfg(test)` allowance the library crates set for themselves. A panic here is
// a test failure, which is the point.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_editor_core::foundation::{MediaTime, TimelineTime, ticks_per_frame};
use bettercut_editor_core::media::{ColorRange, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

/// The whole §60 loop that exists today: new project, import, place, save,
/// close, reopen, and confirm nothing was lost.
#[test]
fn import_place_save_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("integration.vproj");

    // 1. Create a project.
    let (mut editor, _events) = Editor::new_project("Integration");
    assert_eq!(editor.project().clip_count(), 0);

    // 2. Import a real video file.
    let media_id = editor
        .import_file(&fixture("ntsc-2997.mp4"))
        .expect("import should succeed");

    let asset = editor
        .project()
        .media_asset(media_id)
        .expect("asset in library");
    assert_eq!(asset.kind, MediaKind::Video);
    assert_eq!((asset.width, asset.height), (640, 360));
    assert!(asset.duration.ticks() > 0, "probe returned no duration");

    // §21a.2 — the colour property that silently ruins exports.
    assert_eq!(asset.color.range, ColorRange::Limited);

    // §9 — the probed rate must divide the timebase exactly, or every cut on
    // this clip would drift.
    let rate = asset.frame_rate.expect("frame rate");
    assert_eq!(
        ticks_per_frame(rate),
        Some(32_032),
        "29.97 did not survive probing as an exact rational"
    );

    let duration = asset.duration;

    // 3. Add it to the timeline.
    let track = editor.active_sequence().expect("sequence").video_tracks[0].id;
    let source = SourceRange::new(MediaTime::ZERO, duration).expect("non-empty");
    let clip = VideoClip::new(media_id, TimelineTime::ZERO, source).expect("valid clip");
    let clip_id = clip.id;

    editor
        .add_clip(track, ClipPayload::Video(Box::new(clip)))
        .expect("clip should be accepted");
    assert_eq!(editor.project().clip_count(), 1);

    // 4. Save.
    editor.save_as(&path).expect("save");
    assert!(!editor.is_dirty());

    // 5. Reopen, and check the project survived intact.
    let (reopened, _events) = Editor::open(&path).expect("reopen");
    assert_eq!(reopened.project().name, "Integration");
    assert_eq!(reopened.project().clip_count(), 1);

    let reloaded = reopened
        .project()
        .media_asset(media_id)
        .expect("media survived the round trip");
    assert_eq!(reloaded.duration, duration);
    assert_eq!(reloaded.color.range, ColorRange::Limited);
    assert_eq!(
        reloaded.frame_rate.and_then(ticks_per_frame),
        Some(32_032),
        "frame rate stopped being exact after save/load"
    );

    // The media exists on disk, so it must not be flagged missing (§66).
    assert!(!reloaded.missing, "a present file was reported missing");

    let clip = reopened.active_sequence().expect("sequence").video_tracks[0]
        .get(clip_id)
        .expect("clip survived the round trip");
    assert_eq!(clip.timeline.start, TimelineTime::ZERO);
    assert_eq!(clip.media_id, media_id);
}

#[test]
fn importing_audio_and_images_classifies_them_correctly() {
    let (mut editor, _events) = Editor::new_project("Kinds");

    let audio = editor
        .import_file(&fixture("tone-48k.wav"))
        .expect("import wav");
    let image = editor
        .import_file(&fixture("still.png"))
        .expect("import png");

    let audio = editor.project().media_asset(audio).expect("asset");
    assert_eq!(audio.kind, MediaKind::Audio);
    // §20a.3 / §9: audio is normalized to 48 kHz internally, and this source
    // already is — so the timebase can represent it exactly.
    assert_eq!(audio.audio_sample_rate, Some(48_000));

    let image = editor.project().media_asset(image).expect("asset");
    assert_eq!(image.kind, MediaKind::Image);
    assert!(
        image.frame_rate.is_none(),
        "a still image must not carry the demuxer's synthetic frame rate"
    );
}

/// §50: an unreadable file is an error the session survives, not a crash.
#[test]
fn importing_a_non_media_file_fails_without_corrupting_the_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let junk = dir.path().join("notes.txt");
    std::fs::write(&junk, b"not media").expect("write");

    let (mut editor, _events) = Editor::new_project("Robust");
    assert!(editor.import_file(&junk).is_err());

    // The project is untouched and still usable.
    assert!(editor.project().media.is_empty());
    let good = editor.import_file(&fixture("ntsc-2997.mp4"));
    assert!(good.is_ok(), "editor unusable after a failed import");
}

#[test]
fn importing_the_same_file_twice_reuses_the_asset() {
    let (mut editor, _events) = Editor::new_project("Dedupe");
    let first = editor
        .import_file(&fixture("ntsc-2997.mp4"))
        .expect("first");
    let second = editor
        .import_file(&fixture("ntsc-2997.mp4"))
        .expect("second");

    assert_eq!(first, second, "re-import created a duplicate asset");
    assert_eq!(editor.project().media.len(), 1);
}

/// §39's whole purpose, exercised through the real editor: edits made after the
/// last save survive a process that never got to save again.
///
/// "Crashing" here means dropping the editor without saving — from the
/// filesystem's point of view that is exactly what a crash looks like.
#[test]
fn edits_made_after_the_last_save_survive_a_crash() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("crashy.vproj");

    let recovered_name;
    let track_count;
    {
        let (mut editor, _events) = Editor::new_project("Before");
        editor.save_as(&path).expect("save");

        // Everything from here on exists only in the journal.
        editor
            .dispatch(bettercut_editor_core::Command::RenameProject {
                name: "After The Save".to_owned(),
            })
            .expect("rename");
        editor.add_video_track("V2").expect("add track");
        editor.add_audio_track("A2").expect("add track");

        recovered_name = editor.project().name.clone();
        track_count = editor.active_sequence().expect("sequence").track_count();

        // No save, no shutdown: just gone.
        std::mem::forget(editor);
    }

    let session = recover(RecoveryPaths::for_project(Some(&path), "irrelevant"))
        .expect("there should be work to recover");

    assert_eq!(session.failed, 0, "some journalled edits would not replay");
    assert_eq!(session.project.name, recovered_name);
    assert_eq!(
        session.project.active().expect("sequence").track_count(),
        track_count,
        "recovered project has a different shape than the one that was lost"
    );

    // §39.5: the file on disk is untouched, still holding the pre-crash state.
    let on_disk = bettercut_project_format::load(&path).expect("load");
    assert_eq!(
        on_disk.name, "Before",
        "recovery modified the user's saved project"
    );
}

/// A snapshot that falls due partway through journalling a group must not
/// capture the rest of the group, or recovery applies those commands twice.
#[test]
fn a_group_journalled_across_a_snapshot_recovers_once() {
    use bettercut_editor_core::journal::SNAPSHOT_EVERY_COMMANDS;
    use bettercut_editor_core::{Command, RecoveryPaths, TrackKindRepr, recover};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("group.vproj");

    let expected;
    {
        let (mut editor, _events) = Editor::new_project("Group");
        editor.save_as(&path).expect("save");
        let sequence = editor.active_sequence().expect("sequence").id;

        // Two short of a snapshot, so it falls due inside the group below.
        for i in 0..SNAPSHOT_EVERY_COMMANDS - 2 {
            editor
                .dispatch(Command::RenameProject {
                    name: format!("n{i}"),
                })
                .expect("rename");
        }
        let tracks: Vec<Command> = (0..5)
            .map(|i| Command::AddTrack {
                sequence,
                kind: TrackKindRepr::Video,
                name: format!("G{i}"),
                id: bettercut_foundation::TrackId::new(),
            })
            .collect();
        editor.dispatch_group("Add Tracks", tracks).expect("group");

        expected = editor.active_sequence().expect("sequence").track_count();
        std::mem::forget(editor);
    }

    let session = recover(RecoveryPaths::for_project(Some(&path), "x")).expect("recoverable");
    assert_eq!(session.failed, 0, "some of the group replayed onto itself");
    assert_eq!(
        session.project.active().expect("sequence").track_count(),
        expected
    );
}

/// A clean save clears the recovery data: offering to restore work the user
/// already has reads as data loss even though nothing was lost.
#[test]
fn saving_clears_the_recovery_data() {
    use bettercut_editor_core::{RecoveryPaths, recover};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("clean.vproj");

    let (mut editor, _events) = Editor::new_project("Clean");
    editor.save_as(&path).expect("save");
    editor.add_video_track("V2").expect("add track");

    // There is unsaved work, so there is something to recover.
    assert!(recover(RecoveryPaths::for_project(Some(&path), "x")).is_some());

    editor.save().expect("save again");

    assert!(
        recover(RecoveryPaths::for_project(Some(&path), "x")).is_none(),
        "recovery data survived a clean save"
    );
}

/// §66: a project whose media has moved reports it rather than failing to open.
#[test]
fn a_project_with_missing_media_still_opens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let copied = dir.path().join("temporary.mp4");
    std::fs::copy(fixture("ntsc-2997.mp4"), &copied).expect("copy fixture");

    let path = dir.path().join("missing-media.vproj");
    let (mut editor, _events) = Editor::new_project("Missing");
    editor.import_file(&copied).expect("import");
    editor.save_as(&path).expect("save");
    drop(editor);

    std::fs::remove_file(&copied).expect("remove the media out from under it");

    let (reopened, _events) = Editor::open(&path).expect("project must still open");
    assert_eq!(reopened.project().media.len(), 1);
    assert!(
        reopened.project().media[0].missing,
        "a deleted file was not flagged missing"
    );
}
