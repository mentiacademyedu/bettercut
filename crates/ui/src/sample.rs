//! A sample edit, made from nothing: three coloured scenes with a title on
//! each — the titles are the first tips — crossfades between them, a whoosh
//! at the start and a marker. Something to cut, move and export before a
//! single file has been imported.

use bettercut_editor_core::command::TextProperty;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::media::{Generated, GeneratedSound};
use bettercut_editor_core::text::{TextStyle, TitleLook};
use bettercut_editor_core::timeline::TransitionKind;
use bettercut_editor_core::{Editor, EditorError};

use crate::state::UiState;

/// The scenes: what each title says, and the colour behind it, top to bottom.
pub const SCENES: [(&str, [u8; 3], [u8; 3]); 3] = [
    ("Your first edit", [52, 72, 168], [12, 12, 40]),
    (
        "Press S to split, Delete to remove",
        [196, 92, 60],
        [44, 12, 12],
    ),
    ("Ctrl+K finds anything", [32, 140, 120], [6, 30, 30]),
];

/// Lay the sample out in `editor`, which should be a fresh project.
pub fn build(editor: &mut Editor) -> Result<(), EditorError> {
    let mut at = TimelineTime::ZERO;
    let mut previous = None;
    for (words, top, bottom) in SCENES {
        editor.set_playhead(at);
        let scene = editor.add_colour_clip(Generated::Colour { top, bottom })?;
        let title = editor.add_text(words)?;
        editor.set_text_property(
            title,
            TextProperty::Style(Box::new(TextStyle::title(TitleLook::Headline))),
            false,
        )?;
        // Colour clips have no end to their material, so there is always
        // room for the dissolve.
        if let Some(before) = previous {
            editor.set_transition(before, TransitionKind::Crossfade)?;
        }
        previous = Some(scene);
        at = editor.clip_end(scene).unwrap_or(at);
    }
    editor.set_playhead(TimelineTime::ZERO);
    editor.add_generated_sound(GeneratedSound::Whoosh, TimelineTime::from_seconds(1))?;
    let middle = TimelineTime::from_ticks(at.ticks() / 2);
    editor.add_markers(&[middle])?;
    editor.set_marker_label(middle, "Try splitting here")?;
    editor.set_playhead(TimelineTime::ZERO);
    Ok(())
}

/// Replace the open project with the sample — asking first when there is
/// unsaved work, as a new project does.
pub fn open(editor: &mut Editor, state: &mut UiState) {
    if editor.is_dirty() && !state.discard_ok {
        state.pending_switch = Some(crate::save_prompt::Switch::Sample);
        return;
    }
    let (mut fresh, _events) = Editor::new_project("Sample");
    match build(&mut fresh) {
        Ok(()) => {
            *editor = fresh;
            crate::panels::reset_state(state);
            state.welcome_open = false;
            state.info("A sample edit to try things on — nothing in it is yours to lose");
        }
        Err(err) => state.error(format!("Could not make the sample: {err}")),
    }
}
