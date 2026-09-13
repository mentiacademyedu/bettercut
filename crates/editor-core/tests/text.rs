//! Text overlays through the editor (§26).
//!
//! The rasterizer is tested in the text crate and the resolution in the
//! playback crate. What matters here is the editing: that a title is undoable
//! in both directions, that the timeline operations built for media clips work
//! on it unchanged, and that it survives the §38.2 journal.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{SourceRange, TextClip, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor, TextProperty, TrimEdge};
use bettercut_text::{Rgba, TextStyle};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn editor() -> Editor {
    let (editor, _rx) = Editor::new_project("Text");
    editor
}

fn text_of(editor: &Editor, clip: ClipId) -> &TextClip {
    editor.text_clip(clip).expect("no such text clip")
}

#[test]
fn a_new_sequence_has_a_text_track() {
    let editor = editor();
    assert_eq!(
        editor.active_sequence().unwrap().text_tracks.len(),
        1,
        "nowhere to put a title"
    );
}

#[test]
fn a_title_is_added_at_the_playhead_and_is_undoable() {
    let mut editor = editor();
    editor.set_playhead(seconds(3));

    let clip = editor.add_text("Hello").unwrap();
    assert_eq!(text_of(&editor, clip).timeline.start, seconds(3));
    assert_eq!(text_of(&editor, clip).text, "Hello");

    editor.undo().unwrap();
    assert!(
        editor.text_clip(clip).is_none(),
        "undo left the title behind"
    );

    editor.redo().unwrap();
    assert_eq!(
        text_of(&editor, clip).text,
        "Hello",
        "redo did not put it back"
    );
}

/// Redo has to produce the *same* clip, not an equivalent one: a selection
/// holding the old id would otherwise point at nothing after an undo/redo.
#[test]
fn redo_reproduces_the_same_clip_id() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    editor.undo().unwrap();
    editor.redo().unwrap();

    assert!(
        editor.text_clip(clip).is_some(),
        "redo produced a differently-identified title"
    );
}

/// Titles do not overlap, so a second one added at the same playhead goes after
/// the first rather than being refused. Refusing would be a dead end: the user
/// asked for a title, not for a lesson in track occupancy.
#[test]
fn a_second_title_at_the_same_spot_lands_after_the_first() {
    let mut editor = editor();
    let first = editor.add_text("One").unwrap();
    let second = editor.add_text("Two").unwrap();

    let first_end = text_of(&editor, first).timeline.end;
    assert_eq!(text_of(&editor, second).timeline.start, first_end);
}

#[test]
fn removing_a_title_restores_it_on_undo() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();
    editor
        .set_text_property(
            clip,
            TextProperty::Style(Box::new(TextStyle {
                color: Rgba::opaque(255, 0, 0),
                ..TextStyle::default()
            })),
            false,
        )
        .unwrap();

    editor.remove_text(clip).unwrap();
    assert!(editor.text_clip(clip).is_none());

    editor.undo().unwrap();
    let restored = text_of(&editor, clip);
    assert_eq!(restored.text, "Hello");
    assert_eq!(
        restored.style.color,
        Rgba::opaque(255, 0, 0),
        "undo restored the title but not how it looked"
    );
}

#[test]
fn editing_the_words_is_undoable() {
    let mut editor = editor();
    let clip = editor.add_text("Before").unwrap();

    editor
        .set_text_property(clip, TextProperty::Content("After".into()), false)
        .unwrap();
    assert_eq!(text_of(&editor, clip).text, "After");

    editor.undo().unwrap();
    assert_eq!(text_of(&editor, clip).text, "Before");
}

/// A drag across a slider is one undo step, not one per frame of the drag
/// (§11). Without this an undo after nudging a title would step back through
/// hundreds of intermediate positions.
#[test]
fn dragging_a_control_collapses_into_one_undo_step() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    for step in 1..=10 {
        editor
            .set_text_property(
                clip,
                TextProperty::Opacity(1.0 - step as f32 * 0.05),
                step > 1,
            )
            .unwrap();
    }
    assert!((text_of(&editor, clip).opacity - 0.5).abs() < 1e-5);

    editor.undo().unwrap();
    assert_eq!(
        text_of(&editor, clip).opacity,
        1.0,
        "one undo did not take the whole drag back"
    );
}

/// The clamps come from the same place the video clips use, so a title and a
/// clip cannot end up with different limits on the same control.
#[test]
fn out_of_range_values_are_clamped() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    editor
        .set_text_property(clip, TextProperty::Opacity(9.0), false)
        .unwrap();
    assert_eq!(text_of(&editor, clip).opacity, 1.0);

    editor
        .set_text_property(
            clip,
            TextProperty::Style(Box::new(TextStyle {
                size: 1.0e9,
                ..TextStyle::default()
            })),
            false,
        )
        .unwrap();
    assert!(
        text_of(&editor, clip).style.size <= bettercut_text::MAX_SIZE,
        "a hand-editable size reached the model unclamped"
    );
}

/// The point of putting titles in an ordinary track: move, trim and split were
/// written once, for media clips, and work here without a second
/// implementation.
#[test]
fn a_title_moves_like_any_other_clip() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();
    let track = editor.active_sequence().unwrap().text_tracks[0].id;

    editor.move_clip(track, track, clip, seconds(10)).unwrap();
    assert_eq!(text_of(&editor, clip).timeline.start, seconds(10));

    editor.undo().unwrap();
    assert_eq!(text_of(&editor, clip).timeline.start, TimelineTime::ZERO);
}

#[test]
fn a_title_trims_like_any_other_clip() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    let track = editor.active_sequence().unwrap().text_tracks[0].id;
    editor
        .trim_clip(track, clip, TrimEdge::End, seconds(1))
        .unwrap();
    assert_eq!(text_of(&editor, clip).timeline.end, seconds(1));

    editor.undo().unwrap();
    assert_eq!(
        text_of(&editor, clip).timeline.duration(),
        bettercut_editor_core::timeline::DEFAULT_TEXT_DURATION
    );
}

#[test]
fn a_title_splits_like_any_other_clip() {
    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();
    editor.set_playhead(seconds(1));

    assert_eq!(editor.split_at_playhead(&[clip]).unwrap(), 1);

    let track = &editor.active_sequence().unwrap().text_tracks[0];
    assert_eq!(track.len(), 2, "the title did not split");
    assert!(
        track.clips().iter().all(|c| c.text == "Hello"),
        "the words did not survive the split"
    );
}

/// A text clip and a video clip can share a timeline position — they are on
/// different tracks, and a title over a shot is the entire point.
#[test]
fn a_title_and_a_video_clip_coexist() {
    let mut editor = editor();
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let video = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
    )
    .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(video)))
        .unwrap();

    let title = editor.add_text("Over the top").unwrap();
    assert_eq!(text_of(&editor, title).timeline.start, TimelineTime::ZERO);
}

/// A selection is a bare id, and the two kinds of clip take different controls.
#[test]
fn a_text_clip_is_distinguishable_from_a_media_clip() {
    let mut editor = editor();
    let title = editor.add_text("Hello").unwrap();

    assert!(editor.is_text_clip(title));
    assert!(!editor.is_text_clip(ClipId::new()));
    assert!(
        editor.video_clip(title).is_none(),
        "a title answered to the video-clip lookup"
    );
}

/// §38.2: an edit lost to a crash is worse than one never made.
#[test]
fn a_title_survives_a_save_and_load() {
    let mut editor = editor();
    let clip = editor.add_text("Persisted").unwrap();
    editor
        .set_text_property(
            clip,
            TextProperty::Style(Box::new(TextStyle {
                size: 96.0,
                color: Rgba::opaque(0, 255, 0),
                ..TextStyle::default()
            })),
            false,
        )
        .unwrap();

    let json = serde_json::to_string(editor.project()).unwrap();
    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_str(&json).unwrap();

    let restored = loaded.sequences[0].text_clip(clip).expect("title is gone");
    assert_eq!(restored.text, "Persisted");
    assert_eq!(restored.style.size, 96.0);
    assert_eq!(restored.style.color, Rgba::opaque(0, 255, 0));
}

/// A project written before §26 has no text tracks in it, and must still load.
#[test]
fn an_older_project_loads_without_text_tracks() {
    let editor = editor();
    let mut json: serde_json::Value = serde_json::to_value(editor.project()).unwrap();
    strip(&mut json, "text_tracks");

    let loaded: bettercut_editor_core::project_format::Project =
        serde_json::from_value(json).unwrap();
    assert!(loaded.sequences[0].text_tracks.is_empty());
}

fn strip(value: &mut serde_json::Value, field: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(field);
            for (_, v) in map.iter_mut() {
                strip(v, field);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| strip(v, field)),
        _ => {}
    }
}

/// The preview's drag handles produce a [`ClipProperty`] — they do not know
/// what kind of layer they are moving. A title has to accept one, or dragging a
/// title in the picture silently fails while the Inspector's number box works.
#[test]
fn a_title_accepts_the_preview_s_drag_properties() {
    use bettercut_editor_core::ClipProperty;

    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    editor
        .set_clip_value(clip, ClipProperty::Position { x: 0.25, y: -0.1 }, false)
        .unwrap();
    assert_eq!(text_of(&editor, clip).transform.position.x, 0.25);
    assert_eq!(text_of(&editor, clip).transform.position.y, -0.1);

    editor
        .set_clip_value(clip, ClipProperty::Scale { x: 2.0, y: 2.0 }, false)
        .unwrap();
    assert_eq!(text_of(&editor, clip).transform.scale.x, 2.0);

    editor.undo().unwrap();
    assert_eq!(
        text_of(&editor, clip).transform.scale.x,
        1.0,
        "the drag was not undoable"
    );
}

/// And a drag is still one undo step, exactly as it is for a video clip.
#[test]
fn dragging_a_title_across_the_picture_is_one_undo_step() {
    use bettercut_editor_core::ClipProperty;

    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    for step in 0..12 {
        editor
            .set_clip_value(
                clip,
                ClipProperty::Position {
                    x: step as f32 * 0.02,
                    y: 0.0,
                },
                step > 0,
            )
            .unwrap();
    }

    editor.undo().unwrap();
    assert_eq!(
        text_of(&editor, clip).transform.position.x,
        0.0,
        "one undo did not take the whole drag back"
    );
}

/// The properties a title does not have are refused rather than silently
/// dropped — a grade on a text layer would be a control that appears to work.
#[test]
fn a_title_refuses_the_properties_it_does_not_have() {
    use bettercut_editor_core::ClipProperty;

    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();

    assert!(
        editor
            .set_clip_value(clip, ClipProperty::Brightness(0.5), false)
            .is_err()
    );
    assert!(
        editor
            .set_clip_value(clip, ClipProperty::Gain(2.0), false)
            .is_err()
    );
}

/// A title's entrance and exit are one undoable property, kept in range by the
/// edit itself so a replayed journal cannot bring back a length the panel
/// would never offer.
#[test]
fn a_titles_animation_is_set_clamped_and_undone() {
    use bettercut_editor_core::timeline::{MIN_MOTION, Motion, MotionKind, TextAnimation};

    let mut editor = editor();
    let clip = editor.add_text("Hello").unwrap();
    let wanted = TextAnimation {
        intro: Some(Motion {
            kind: MotionKind::Pop,
            duration: TimelineTime::ZERO,
        }),
        outro: None,
    };

    editor
        .set_text_property(clip, TextProperty::Animation(wanted), false)
        .unwrap();
    let intro = text_of(&editor, clip).animation.intro.expect("an entrance");
    assert_eq!(intro.kind, MotionKind::Pop);
    assert_eq!(intro.duration, MIN_MOTION, "a zero length was let through");

    editor.undo().unwrap();
    assert!(text_of(&editor, clip).animation.is_none());
}

/// §26's title looks: a whole preset in one step.
mod looks {
    use bettercut_editor_core::foundation::TimelineTime;
    use bettercut_editor_core::text::{TextStyle, TitleLook};
    use bettercut_editor_core::{Editor, EditorError};

    fn editor_with_a_title() -> (Editor, bettercut_editor_core::foundation::ClipId) {
        let (mut editor, _events) = Editor::new_project("Titles");
        editor.set_playhead(TimelineTime::from_seconds(1));
        let title = editor.add_text("A NAME").expect("a title");
        (editor, title)
    }

    fn title(
        editor: &Editor,
        clip: bettercut_editor_core::foundation::ClipId,
    ) -> (TextStyle, f32, f32) {
        let sequence = editor.active_sequence().expect("sequence");
        let clip = sequence
            .text_tracks
            .iter()
            .find_map(|track| track.get(clip))
            .expect("the title");
        (
            clip.style.clone(),
            clip.transform.position.x,
            clip.transform.position.y,
        )
    }

    /// The style *and* the position: a lower third in the middle of the frame
    /// is not a lower third.
    #[test]
    fn a_look_dresses_a_title_and_places_it() {
        let (mut editor, clip) = editor_with_a_title();
        let (_, x, y) = title(&editor, clip);
        assert_eq!((x, y), (0.0, 0.0), "a new title starts centred");

        editor
            .set_title_look(clip, TitleLook::LowerThird)
            .expect("a title takes a look");

        let (style, x, y) = title(&editor, clip);
        assert_eq!(style, TextStyle::title(TitleLook::LowerThird));
        assert_eq!((x, y), TitleLook::LowerThird.anchor());
        assert!(y > 0.15 && x < 0.0, "it was not put low and left: {x},{y}");
    }

    /// One decision, one step — and undo puts back both halves of it.
    #[test]
    fn a_look_is_one_undo_step() {
        let (mut editor, clip) = editor_with_a_title();
        let (before_style, before_x, before_y) = title(&editor, clip);
        let depth = editor.undo_depth();

        editor.set_title_look(clip, TitleLook::Headline).unwrap();
        assert_eq!(editor.undo_depth(), depth + 1);

        editor.undo().unwrap();
        let (style, x, y) = title(&editor, clip);
        assert_eq!(style, before_style, "the style stayed changed");
        assert_eq!(
            (x, y),
            (before_x, before_y),
            "undo put back the style but left the title where the look moved it"
        );
    }

    /// Trying the looks does not have to be done in order: each replaces the
    /// last completely, position included.
    #[test]
    fn a_second_look_replaces_the_first() {
        let (mut editor, clip) = editor_with_a_title();
        editor.set_title_look(clip, TitleLook::LowerThird).unwrap();
        editor.set_title_look(clip, TitleLook::Quote).unwrap();

        let (style, x, y) = title(&editor, clip);
        assert_eq!(style, TextStyle::title(TitleLook::Quote));
        assert_eq!(
            (x, y),
            TitleLook::Quote.anchor(),
            "it kept the old position"
        );
    }

    /// Footage is not a title.
    #[test]
    fn a_video_clip_has_no_title_look() {
        let (mut editor, _events) = Editor::new_project("Titles");
        let media = editor.import_media(bettercut_editor_core::media::MediaAsset::new(
            bettercut_editor_core::media::MediaKind::Video,
            "C:/media/shot.mp4",
            bettercut_editor_core::foundation::MediaTime::from_seconds(10),
        ));
        let clip = editor.place_media(media).unwrap()[0];

        let refused = editor.set_title_look(clip, TitleLook::Headline);
        assert!(
            matches!(refused, Err(EditorError::ClipNotFound(_))),
            "{refused:?}"
        );
    }
}
