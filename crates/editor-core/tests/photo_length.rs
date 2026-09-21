//! The project's photo length (`ProjectSettings::photo_length`): how long a
//! photo or colour clip runs when it is put on the timeline.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::command::SettingChange;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{Generated, MediaAsset, MediaKind};
use bettercut_editor_core::{Command, Editor};

fn set(editor: &mut Editor, length: MediaTime) {
    editor
        .dispatch(Command::ChangeSetting {
            change: SettingChange::PhotoLength(length),
        })
        .unwrap();
}

fn photo(editor: &mut Editor) -> bettercut_editor_core::foundation::MediaId {
    editor.import_media(MediaAsset::new(
        MediaKind::Image,
        "C:/media/beach.jpg",
        MediaTime::ZERO,
    ))
}

fn length_of(editor: &Editor, clip: bettercut_editor_core::foundation::ClipId) -> TimelineTime {
    editor.video_clip(clip).unwrap().timeline.duration()
}

#[test]
fn photos_run_for_the_projects_length() {
    let (mut editor, _events) = Editor::new_project("Photos");
    assert_eq!(
        editor.project().settings.photo_length,
        MediaTime::from_seconds(5)
    );
    let media = photo(&mut editor);

    set(&mut editor, MediaTime::from_seconds(3));
    let clip = editor.place_media(media).unwrap()[0];
    assert_eq!(length_of(&editor, clip), TimelineTime::from_seconds(3));

    // Colour clips too.
    editor.set_playhead(TimelineTime::from_seconds(20));
    let colour = editor.add_colour_clip(Generated::solid([0, 0, 0])).unwrap();
    assert_eq!(length_of(&editor, colour), TimelineTime::from_seconds(3));

    // A video is still its own length.
    let video = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/walk.mp4",
        MediaTime::from_seconds(8),
    ));
    let walk = editor.place_media(video).unwrap()[0];
    assert_eq!(length_of(&editor, walk), TimelineTime::from_seconds(8));
}

#[test]
fn the_length_is_kept_in_range_undone_and_saved() {
    let (mut editor, _events) = Editor::new_project("Photos");
    set(&mut editor, MediaTime::from_seconds(600));
    assert_eq!(
        editor.project().settings.photo_length,
        MediaTime::from_seconds(60)
    );
    set(&mut editor, MediaTime::from_millis(1));
    assert_eq!(
        editor.project().settings.photo_length,
        MediaTime::from_millis(500)
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.project().settings.photo_length,
        MediaTime::from_seconds(60)
    );

    set(&mut editor, MediaTime::from_seconds(2));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photos.vproj");
    editor.save_as(&path).unwrap();
    let (opened, _) = Editor::open(&path).unwrap();
    assert_eq!(
        opened.project().settings.photo_length,
        MediaTime::from_seconds(2)
    );
}
