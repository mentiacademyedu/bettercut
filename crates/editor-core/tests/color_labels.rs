//! Colour labels on clips (`Editor::set_color_label`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::ColorLabel;

fn video_with_sound(
    editor: &mut Editor,
) -> (
    bettercut_editor_core::foundation::ClipId,
    bettercut_editor_core::foundation::ClipId,
) {
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(6),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();
    (placed[0], placed[1])
}

/// A picture and its sound are one shot: tagging either tags both, in one
/// undo step named for the colour.
#[test]
fn a_linked_pair_is_tagged_together() {
    let (mut editor, _events) = Editor::new_project("Labels");
    let (picture, sound) = video_with_sound(&mut editor);
    let depth = editor.undo_depth();

    let tagged = editor.set_color_label(&[sound], ColorLabel::Blue).unwrap();

    assert_eq!(tagged, 2);
    assert_eq!(editor.color_label(picture), Some(ColorLabel::Blue));
    assert_eq!(editor.color_label(sound), Some(ColorLabel::Blue));
    assert_eq!(editor.undo_depth(), depth + 1);
    assert_eq!(editor.undo_label().as_deref(), Some("Colour Label: Blue"));

    editor.undo().unwrap();
    assert_eq!(editor.color_label(picture), Some(ColorLabel::None));
}

/// Every kind of lane takes a label: titles and adjustments are organised
/// like everything else.
#[test]
fn titles_and_adjustments_take_labels() {
    let (mut editor, _events) = Editor::new_project("Labels");
    let title = editor.add_text("Title").unwrap();
    let adjustment = editor.add_adjustment().unwrap();

    editor
        .set_color_label(&[title, adjustment], ColorLabel::Orange)
        .unwrap();

    assert_eq!(editor.color_label(title), Some(ColorLabel::Orange));
    assert_eq!(editor.color_label(adjustment), Some(ColorLabel::Orange));
}

/// A split clip keeps its tag on both halves, and a label survives a save and
/// a load.
#[test]
fn a_label_survives_a_split_and_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, _events) = Editor::new_project("Labels");
    let (picture, _) = video_with_sound(&mut editor);
    editor.set_color_label(&[picture], ColorLabel::Red).unwrap();

    editor
        .split_clip_at(picture, &[TimelineTime::from_seconds(3)])
        .unwrap();
    let halves: Vec<_> = editor.active_sequence().unwrap().video_tracks[0]
        .clips()
        .iter()
        .map(|c| c.color_label)
        .collect();
    assert_eq!(halves, vec![ColorLabel::Red, ColorLabel::Red]);

    let file = dir.path().join("labels.vproj");
    editor.save_as(&file).unwrap();
    let (opened, _events) = Editor::open(&file).unwrap();
    assert!(
        opened.active_sequence().unwrap().video_tracks[0]
            .clips()
            .iter()
            .all(|c| c.color_label == ColorLabel::Red)
    );
}

/// Clearing a label is the same edit with no colour, and nothing selected is
/// no step at all.
#[test]
fn clearing_and_tagging_nothing() {
    let (mut editor, _events) = Editor::new_project("Labels");
    let (picture, _) = video_with_sound(&mut editor);
    editor
        .set_color_label(&[picture], ColorLabel::Green)
        .unwrap();

    editor
        .set_color_label(&[picture], ColorLabel::None)
        .unwrap();
    assert_eq!(editor.color_label(picture), Some(ColorLabel::None));
    assert_eq!(editor.undo_label().as_deref(), Some("Clear Colour Label"));

    let depth = editor.undo_depth();
    assert_eq!(editor.set_color_label(&[], ColorLabel::Red).unwrap(), 0);
    assert_eq!(editor.undo_depth(), depth);
}

/// Every label has a distinct name and colour, and only None has no colour.
#[test]
fn every_label_is_distinct() {
    let mut names: Vec<_> = ColorLabel::ALL.iter().map(|l| l.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ColorLabel::ALL.len());
    let colours: Vec<_> = ColorLabel::ALL.iter().filter_map(|l| l.rgb()).collect();
    assert_eq!(colours.len(), ColorLabel::ALL.len() - 1);
    let mut unique = colours.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), colours.len());
}
