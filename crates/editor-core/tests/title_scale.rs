//! Title and caption designs are drawn for 1080 lines and sized for the
//! frame they go in, so a look chosen in 4K is the size it is in HD.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::text::{CaptionLook, TextStyle, TitleLook};
use bettercut_editor_core::timeline::Resolution;

fn editor_at(width: u32, height: u32) -> Editor {
    let (mut editor, _events) = Editor::new_project("Scale");
    let rate = editor.active_sequence().unwrap().frame_rate;
    editor
        .set_sequence_format(Resolution { width, height }, rate)
        .unwrap();
    editor
}

#[test]
fn a_look_chosen_in_4k_is_twice_the_pixels() {
    let mut editor = editor_at(3840, 2160);
    assert!((editor.frame_scale() - 2.0).abs() < 1e-6);
    let title = editor.add_text("Hello").unwrap();
    editor.set_title_look(title, TitleLook::Headline).unwrap();
    let style = &editor.text_clip(title).unwrap().style;
    let designed = TextStyle::title(TitleLook::Headline);
    assert!(
        (style.size - designed.size * 2.0).abs() < 0.01,
        "{}",
        style.size
    );
}

#[test]
fn captions_dressed_in_4k_are_twice_the_pixels() {
    let mut editor = editor_at(3840, 2160);
    editor
        .replace_captions(
            vec![bettercut_captions::CaptionSegment {
                start: TimelineTime::ZERO,
                end: TimelineTime::from_seconds(2),
                text: "Hello".to_owned(),
                words: Vec::new(),
            }],
            "Captions",
        )
        .expect("a caption");
    editor.set_caption_look(CaptionLook::Boxed).unwrap();
    let sequence = editor.active_sequence().unwrap();
    let caption = sequence
        .text_tracks
        .iter()
        .flat_map(|t| t.clips())
        .next()
        .expect("the caption");
    let designed = TextStyle::look(CaptionLook::Boxed);
    assert!((caption.style.size - designed.size * 2.0).abs() < 0.01);
}

#[test]
fn hd_is_as_designed() {
    let mut editor = editor_at(1920, 1080);
    let title = editor.add_text("Hello").unwrap();
    editor.set_title_look(title, TitleLook::Quote).unwrap();
    assert_eq!(
        editor.text_clip(title).unwrap().style,
        TextStyle::title(TitleLook::Quote)
    );
}
