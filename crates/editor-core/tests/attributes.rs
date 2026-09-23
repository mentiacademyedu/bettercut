//! Paste Attributes (`editor_core::attributes`): a copied look sorted into
//! the groups a person would name, and pasted by group.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::ClipProperty;
use bettercut_editor_core::Editor;
use bettercut_editor_core::attributes::{AttributeGroup, groups_in, only};
use bettercut_editor_core::command::ClipPayload;
use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
use bettercut_editor_core::timeline::{SourceRange, VideoClip};

#[test]
fn every_property_lands_in_a_group_a_person_would_name() {
    assert_eq!(
        AttributeGroup::of(&ClipProperty::Scale { x: 1.0, y: 1.0 }),
        AttributeGroup::Framing
    );
    assert_eq!(
        AttributeGroup::of(&ClipProperty::Temperature(0.2)),
        AttributeGroup::Colour
    );
    assert_eq!(
        AttributeGroup::of(&ClipProperty::Blur(10.0)),
        AttributeGroup::Effects
    );
    assert_eq!(
        AttributeGroup::of(&ClipProperty::LumaKey(None)),
        AttributeGroup::Keys
    );
    assert_eq!(
        AttributeGroup::of(&ClipProperty::Gain(0.5)),
        AttributeGroup::Sound
    );
    assert_eq!(
        AttributeGroup::of(&ClipProperty::KeepPitch(true)),
        AttributeGroup::Motion
    );
}

#[test]
fn only_the_ticked_groups_are_pasted() {
    let (mut editor, _events) = Editor::new_project("Attributes");
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let mut clips = Vec::new();
    for start in [0, 4] {
        let clip =
            VideoClip::new(MediaId::new(), TimelineTime::from_seconds(start), source).unwrap();
        clips.push(clip.id);
        editor
            .add_clip(track, ClipPayload::Video(Box::new(clip)))
            .unwrap();
    }
    // The first shot graded and reframed.
    editor
        .set_clip_property(clips[0], ClipProperty::Brightness(1.4), false)
        .unwrap();
    editor
        .set_clip_property(clips[0], ClipProperty::Scale { x: 1.5, y: 1.5 }, false)
        .unwrap();

    let look = editor.clip_look(clips[0]).unwrap();
    let groups = groups_in(&look);
    assert!(groups.contains(&AttributeGroup::Colour) && groups.contains(&AttributeGroup::Framing));

    let colour_only = only(&look, &[AttributeGroup::Colour]);
    assert!(!colour_only.is_empty());
    assert!(
        colour_only
            .iter()
            .all(|p| AttributeGroup::of(p) == AttributeGroup::Colour)
    );

    editor.paste_look(&colour_only, [clips[1]]).unwrap();
    let second = editor.video_clip(clips[1]).unwrap();
    assert!(
        (second.color.brightness - 1.4).abs() < 1e-6,
        "the grade did not travel"
    );
    assert!(
        (second.transform.scale.x - 1.0).abs() < 1e-6,
        "the framing travelled when it was not ticked: {}",
        second.transform.scale.x
    );

    assert!(only(&look, &[]).is_empty());
}
