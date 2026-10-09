//! Voice effects by name (`voice_effects`): one click sets the pitch, the
//! robot, the equaliser and the echo, as one undo step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;
use bettercut_editor_core::media::GeneratedSound;
use bettercut_editor_core::timeline::ClipEq;
use bettercut_editor_core::voice_effects::{VOICE_EFFECTS, VoiceEffect};

fn editor_with_a_sound() -> (Editor, bettercut_editor_core::foundation::ClipId) {
    let (mut editor, _events) = Editor::new_project("Voices");
    let clip = editor
        .add_generated_sound(GeneratedSound::LINE_UP, TimelineTime::from_seconds(2))
        .unwrap();
    (editor, clip)
}

/// The equaliser effects use the inspector's presets of the same name.
#[test]
fn the_borrowed_equalisers_are_the_presets_they_say() {
    for (effect, preset) in [("Telephone", "Phone"), ("Radio", "Radio"), ("Megaphone", "Megaphone")] {
        let eq = ClipEq::PRESETS.iter().find(|p| p.0 == preset).unwrap().2;
        assert_eq!(VoiceEffect::named(effect).unwrap().eq, eq, "{effect}");
    }
}

#[test]
fn every_voice_is_one_step_and_is_recognised() {
    let (mut editor, clip) = editor_with_a_sound();
    for effect in &VOICE_EFFECTS {
        let depth = editor.undo_depth();
        editor.apply_voice_effect(&[clip], effect).unwrap();
        assert!(editor.undo_depth() <= depth + 1, "{} took more than one step", effect.name);
        assert_eq!(
            editor.voice_effect_of(clip).map(|v| v.name),
            Some(effect.name),
            "{}",
            effect.name
        );
    }
    // Back to normal is the voice as recorded.
    editor.apply_voice_effect(&[clip], &VOICE_EFFECTS[0]).unwrap();
    let sound = editor.audio_clip(clip).unwrap();
    assert_eq!((sound.pitch, sound.robot), (0.0, 0.0));
    assert_eq!(sound.eq, ClipEq::default());
}

#[test]
fn undo_takes_the_whole_voice_off() {
    let (mut editor, clip) = editor_with_a_sound();
    editor
        .apply_voice_effect(&[clip], VoiceEffect::named("monster").unwrap())
        .unwrap();
    assert_eq!(editor.voice_effect_of(clip).unwrap().name, "Monster");
    editor.undo().unwrap();
    assert_eq!(editor.voice_effect_of(clip).unwrap().name, "Normal");
}

#[test]
fn names_are_distinct_and_nothing_to_change_is_refused() {
    for (i, a) in VOICE_EFFECTS.iter().enumerate() {
        for b in &VOICE_EFFECTS[i + 1..] {
            assert_ne!(a.name, b.name);
        }
    }
    let (mut editor, _) = editor_with_a_sound();
    assert!(editor.apply_voice_effect(&[], &VOICE_EFFECTS[1]).is_err());
}
