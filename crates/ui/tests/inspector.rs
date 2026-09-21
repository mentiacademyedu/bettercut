//! Headless tests for the Inspector panel.
//!
//! The clip-property controls read the project immutably to draw and then
//! dispatch commands mutably, in one function. That is exactly the shape that
//! panics or fails to compile when rearranged carelessly, and nothing else
//! exercises it — the property *commands* are tested in editor-core, but not
//! the panel that drives them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_editor_core::foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_editor_core::media::{MediaAsset, MediaKind};
use bettercut_editor_core::timeline::{AudioClip, SourceRange, VideoClip};
use bettercut_editor_core::{ClipPayload, Editor};
use bettercut_ui::UiState;
use egui::{Pos2, RawInput, Rect, vec2};

fn editor_with_clips() -> (Editor, ClipId, ClipId) {
    let (mut editor, _rx) = Editor::new_project("Inspector");
    let media = editor.import_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();

    let video = VideoClip::new(media, TimelineTime::ZERO, source).unwrap();
    let video_id = video.id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Video(Box::new(video)))
        .unwrap();

    let audio = AudioClip::new(media, TimelineTime::ZERO, source).unwrap();
    let audio_id = audio.id;
    let track = editor.active_sequence().unwrap().audio_tracks[0].id;
    editor
        .add_clip(track, ClipPayload::Audio(Box::new(audio)))
        .unwrap();

    (editor, video_id, audio_id)
}

/// Draw the inspector once. Returns nothing — the assertion is that it neither
/// panics nor leaves egui in a broken state.
fn draw(editor: &mut Editor, state: &mut UiState) {
    let _ = drawn_text(editor, state);
}

/// Draw the inspector and return every word it put on screen.
///
/// Without this a panel test passes whether or not the panel it names was ever
/// reached: "did not panic" is true of the empty case too. Reading the galleys
/// back is the only way from here to assert that the right controls appeared.
fn drawn_text(editor: &mut Editor, state: &mut UiState) -> String {
    let ctx = egui::Context::default();
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 900.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        bettercut_ui::panels::inspector(ui, editor, state);
    });
    output.textures_delta.clear();

    let mut words = String::new();
    for clipped in &output.shapes {
        collect_text(&clipped.shape, &mut words);
    }
    words
}

fn collect_text(shape: &egui::Shape, into: &mut String) {
    match shape {
        egui::Shape::Text(text) => {
            into.push_str(text.galley.text());
            // A separator, so two adjacent labels cannot form a third word that
            // a `contains` check would match by accident.
            into.push(' ');
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, into);
            }
        }
        _ => {}
    }
}

#[test]
fn the_inspector_draws_with_nothing_selected() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_video_property_controls() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_the_audio_property_controls() {
    let (mut editor, _, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

#[test]
fn the_inspector_draws_with_several_clips_selected() {
    let (mut editor, video, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.selected_clips.insert(audio);
    draw(&mut editor, &mut state);
}

/// A selection pointing at a clip that no longer exists is normal — delete
/// leaves the id behind for a frame — and must not panic.
#[test]
fn a_stale_selection_does_not_panic() {
    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(ClipId::new());
    draw(&mut editor, &mut state);
}

/// Drawing must never mutate the project by itself. Only user input does.
#[test]
fn drawing_changes_nothing() {
    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);

    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    draw(&mut editor, &mut state);

    assert_eq!(
        editor.project(),
        &before,
        "drawing the inspector modified the project"
    );
}

/// A transformed clip shows the Reset button, which is a different code path
/// from the identity case.
#[test]
fn the_inspector_draws_a_transformed_clip() {
    let (mut editor, video, _) = editor_with_clips();
    editor
        .set_clip_property(
            video,
            bettercut_editor_core::ClipProperty::Scale { x: 0.5, y: 0.5 },
            false,
        )
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(video);
    draw(&mut editor, &mut state);
}

/// Every tab draws for a video clip without panicking, and the tab actually
/// changes what is shown.
#[test]
fn every_inspector_tab_draws() {
    use bettercut_ui::panels::InspectorTab;

    // Audio is not among them: video and audio are separate clips on separate
    // tracks (§8), so a video clip genuinely has no sound and the tab correctly
    // falls back — which the next test checks.
    for tab in [
        InspectorTab::Video,
        InspectorTab::Colours,
        InspectorTab::Speed,
        InspectorTab::Animation,
    ] {
        let (mut editor, video, _) = editor_with_clips();
        let mut state = UiState::default();
        state.selected_clips.insert(video);
        state.inspector_tab = tab;
        draw(&mut editor, &mut state);
        assert_eq!(
            state.inspector_tab,
            tab,
            "{} switched away from itself for a video clip",
            tab.label()
        );
    }

    // And every tab draws for an audio clip too, whichever one it lands on.
    for tab in InspectorTab::ALL {
        let (mut editor, _, audio) = editor_with_clips();
        let mut state = UiState::default();
        state.selected_clips.insert(audio);
        state.inspector_tab = tab;
        draw(&mut editor, &mut state);
    }
}

/// A tab the selection cannot offer must not strand the user on an explanation
/// with no way back. Selecting an audio clip while Video is open moves to a tab
/// that exists.
#[test]
fn an_impossible_tab_falls_back_to_one_that_works() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, _, audio) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(audio);
    state.inspector_tab = InspectorTab::Video;

    draw(&mut editor, &mut state);

    assert_eq!(
        state.inspector_tab,
        InspectorTab::Audio,
        "an audio clip left the Inspector on the Video tab"
    );
}

/// Speed has nothing behind it yet, so it must stay reachable rather than being
/// bounced away — the tab is a statement that it is coming.
#[test]
fn the_speed_tab_stays_selected_even_though_it_is_empty() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.inspector_tab = InspectorTab::Speed;

    draw(&mut editor, &mut state);
    assert_eq!(state.inspector_tab, InspectorTab::Speed);
}

/// A clip with a cut after it shows §25's transition section, which is a
/// different code path from the last clip on a track.
#[test]
fn the_inspector_draws_the_transition_section() {
    use bettercut_editor_core::timeline::TransitionKind;

    let (mut editor, video, _) = editor_with_clips();
    let media = editor.project().media[0].id;
    let track = editor.active_sequence().unwrap().video_tracks[0].id;
    let next = VideoClip::new(
        media,
        TimelineTime::from_seconds(4),
        SourceRange::new(MediaTime::from_seconds(10), MediaTime::from_seconds(14)).unwrap(),
    )
    .unwrap();
    editor
        .add_clip(track, ClipPayload::Video(Box::new(next)))
        .unwrap();
    editor
        .set_transition(video, TransitionKind::Crossfade)
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(video);

    // Twice, and asserting the project is untouched: the duration slider
    // dispatches a command when it changes, and a control that reported a
    // change every frame would rewrite the transition on every repaint —
    // filling the undo history from an idle window.
    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(
        editor.project(),
        &before,
        "drawing the transition section modified the project"
    );
}

/// §26: a title takes its own panel — words and a font, not a source range and
/// a grade. Drawing it twice must change nothing, because the text box and the
/// sliders all dispatch commands when they report a change.
#[test]
fn the_inspector_draws_a_text_clip() {
    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(
        editor.project(),
        &before,
        "drawing the text panel modified the project"
    );

    assert!(
        words.contains("Legibility"),
        "the text panel was never reached; drew: {words}"
    );
    // "blur" used to be the marker here, on the grounds that only footage can
    // be blurred. It is not one any more: a title smears along a fast entrance
    // exactly as a shot does, so the word now belongs on both panels. The
    // green screen is a claim about *filmed* colour, which a drawn title can
    // never have.
    assert!(
        !words.contains("green screen"),
        "the video clip's controls appeared for a title"
    );
}

/// Every decoration draws: an outline, a shadow and a background each add
/// controls of their own, and those are the branches a smoke test misses.
#[test]
fn the_inspector_draws_a_fully_decorated_title() {
    use bettercut_editor_core::TextProperty;
    use bettercut_editor_core::text::{Background, Shadow, Stroke, TextStyle};

    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();
    editor
        .set_text_property(
            title,
            TextProperty::Style(Box::new(TextStyle {
                stroke: Some(Stroke::default()),
                shadow: Some(Shadow::default()),
                background: Some(Background::default()),
                ..TextStyle::default()
            })),
            false,
        )
        .unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);
    assert_eq!(editor.project(), &before);

    for control in ["outline", "shadow", "background", "blur"] {
        assert!(
            words.contains(control),
            "the {control} controls did not draw; drew: {words}"
        );
    }
}

/// the Speed tab is a real control now, not a note saying it is not built.
#[test]
fn the_inspector_draws_the_speed_controls() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.selected_clips.insert(video);
    state.inspector_tab = InspectorTab::Speed;

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);

    assert_eq!(
        editor.project(),
        &before,
        "drawing the speed controls modified the project"
    );
    assert!(
        words.contains("2×") && words.contains("0.5×"),
        "the presets did not draw; drew: {words}"
    );
    assert!(
        !words.contains("not built yet"),
        "the placeholder is still there"
    );
}

/// §26's family picker offers what is installed, not three generic names. The
/// list is read once at startup and kept in `UiState`, so a test supplies it
/// the same way the application does.
#[test]
fn the_family_picker_offers_the_installed_fonts() {
    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);
    state.font_families = vec![
        "Bodoni Ornamental".to_owned(),
        "Caslon Antique".to_owned(),
        "Zapfino Extra".to_owned(),
    ];

    // The picker's contents live inside a combo box, which egui only lays out
    // once opened — so what is asserted here is that the panel draws with a
    // list present and changes nothing by itself.
    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    draw(&mut editor, &mut state);

    assert_eq!(editor.project(), &before);
    assert!(
        words.contains("family"),
        "the font row did not draw: {words}"
    );
}

/// A machine with no font list still gets a usable picker — the three generic
/// names work anywhere, which is also why they come first.
#[test]
fn the_family_picker_works_with_no_list() {
    let (mut editor, _, _) = editor_with_clips();
    let title = editor.add_text("Hello").unwrap();

    let mut state = UiState::default();
    state.selected_clips.clear();
    state.selected_clips.insert(title);
    assert!(state.font_families.is_empty());

    let before = editor.project().clone();
    draw(&mut editor, &mut state);
    assert_eq!(editor.project(), &before);
}

/// With nothing selected the Audio tab is the whole video's volume — not a
/// greyed-out tab and not a note saying it does not exist yet.
#[test]
fn the_whole_video_audio_tab_offers_a_volume() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, _, _) = editor_with_clips();
    let mut state = UiState::default();
    state.inspector_tab = InspectorTab::Audio;

    let before = editor.project().clone();
    let words = drawn_text(&mut editor, &mut state);
    assert_eq!(editor.project(), &before, "drawing changed the project");

    assert_eq!(
        state.inspector_tab,
        InspectorTab::Audio,
        "the Audio tab was not available for the whole video"
    );
    assert!(words.contains("volume"), "no volume control drawn: {words}");
    assert!(
        !words.contains("no whole-video volume yet"),
        "the old placeholder is still there"
    );
}

/// A picture placed with its sound: the Audio tab, with the picture selected,
/// adjusts the sound linked to it rather than saying there is none (§12).
#[test]
fn a_pictures_audio_tab_adjusts_its_linked_sound() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, _rx) = Editor::new_project("Inspector");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(10),
    );
    asset.audio_codec = Some("aac".to_owned());
    let media = editor.import_media(asset);
    let placed = editor.place_media(media).unwrap();

    let mut state = UiState::default();
    state.selected_clips.insert(placed[0]);
    state.inspector_tab = InspectorTab::Audio;

    let words = drawn_text(&mut editor, &mut state);
    assert_eq!(state.inspector_tab, InspectorTab::Audio);
    assert!(
        words.contains("The sound that came with this clip."),
        "the linked sound was not offered: {words}"
    );
    assert!(
        words.contains("fade in") && words.contains("fade out"),
        "{words}"
    );
    assert!(!words.contains("This clip has no sound."), "{words}");
}

/// The Colours tab offers the one-click looks, on a clip and on the whole
/// video.
#[test]
fn the_colours_tab_offers_looks() {
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, video, _) = editor_with_clips();
    let mut state = UiState::default();
    state.inspector_tab = InspectorTab::Colours;
    state.selected_clips.insert(video);

    let words = drawn_text(&mut editor, &mut state);
    assert!(words.contains("Looks"), "{words}");
    for (name, _) in bettercut_ui::panels::LOOKS {
        assert!(words.contains(name), "{name} missing: {words}");
    }

    state.clear_selection();
    let whole = drawn_text(&mut editor, &mut state);
    assert!(
        whole.contains("Faded"),
        "the whole video has no looks: {whole}"
    );
}

/// Landscape footage in a vertical sequence: the Video tab offers Fill, and it
/// is not offered when the clip already matches the frame.
#[test]
fn the_video_tab_offers_fit_and_fill() {
    use bettercut_editor_core::timeline::Resolution;
    use bettercut_ui::panels::InspectorTab;

    let (mut editor, _rx) = Editor::new_project("Reframe");
    let media = editor.import_media(
        MediaAsset::new(
            MediaKind::Video,
            "C:/media/wide.mp4",
            MediaTime::from_seconds(10),
        )
        .with_video(
            1920,
            1080,
            bettercut_editor_core::foundation::FrameRate::FPS_30,
        ),
    );
    let clip = editor.place_media(media).unwrap()[0];
    editor
        .set_sequence_format(
            Resolution::new(1080, 1920),
            bettercut_editor_core::foundation::FrameRate::FPS_30,
        )
        .unwrap();

    let mut state = UiState::default();
    state.inspector_tab = InspectorTab::Video;
    state.selected_clips.insert(clip);

    let words = drawn_text(&mut editor, &mut state);
    assert!(words.contains("Fill"), "no fill button: {words}");
    assert!(words.contains("Fit"), "no fit button: {words}");
    assert!(words.contains("movement"), "no movement row: {words}");
    assert!(
        words.contains("Zoom in") && words.contains("Zoom out"),
        "{words}"
    );
}

/// The meter's scale (§20a).
///
/// A meter is only useful if the range it shows matches the range the ear
/// hears. These pin the decibel scale: half a bar is around -30 dB, not around
/// half amplitude.
mod meter {
    use bettercut_ui::panels::meter_fraction;

    #[test]
    fn silence_is_empty_and_full_scale_is_full() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_fraction(1.0), 1.0);
        // Past full scale the bar cannot say more than "everything".
        assert_eq!(meter_fraction(4.0), 1.0);
    }

    /// Linearly, -6 dB (half the amplitude) would draw a half-empty bar and
    /// every ordinary mix would look quiet. On a decibel scale it is nearly
    /// full, which is what it sounds like.
    #[test]
    fn half_amplitude_is_near_the_top() {
        let half = meter_fraction(0.5);
        assert!(half > 0.85, "-6 dB drew at {half:.2} of the bar");
    }

    /// And a quiet passage still has somewhere to go: -40 dB is a third of the
    /// way up rather than pinned to nothing.
    #[test]
    fn a_quiet_passage_still_moves_the_bar() {
        let quiet = meter_fraction(0.01); // -40 dB
        assert!(
            (0.25..0.45).contains(&quiet),
            "-40 dB drew at {quiet:.2} of the bar"
        );
    }

    /// Below the floor there is nothing to show: -60 dB is the bottom of the
    /// scale, and a bar that never quite empties would suggest sound in a
    /// silent passage.
    #[test]
    fn the_floor_is_the_floor() {
        assert_eq!(meter_fraction(0.001), 0.0, "-60 dB is the bottom");
        assert_eq!(meter_fraction(0.0001), 0.0, "-80 dB is below it");
    }

    #[test]
    fn the_bar_rises_with_the_level() {
        let mut previous = 0.0;
        for peak in [0.002_f32, 0.01, 0.1, 0.4, 0.9, 1.0] {
            let fraction = meter_fraction(peak);
            assert!(
                fraction > previous,
                "{peak} drew no higher than the one below"
            );
            previous = fraction;
        }
    }

    #[test]
    fn a_broken_level_draws_nothing() {
        assert_eq!(meter_fraction(f32::NAN), 0.0);
        assert_eq!(meter_fraction(f32::INFINITY), 0.0);
        assert_eq!(meter_fraction(-1.0), 0.0);
    }
}

/// The Looks row, and the strength that CapCut puts under every filter (§45).
mod look_strength {
    use bettercut_editor_core::timeline::ColorAdjust;
    use bettercut_ui::panels::LOOKS;

    fn punchy() -> ColorAdjust {
        LOOKS
            .iter()
            .find_map(|(name, look)| (*name == "Punchy").then_some(*look))
            .expect("Punchy is one of the looks")
    }

    /// Every preset is reachable and distinct: a row of buttons where two do
    /// the same thing is a row with a mistake in it.
    #[test]
    fn the_looks_differ_from_each_other() {
        for (index, (name, look)) in LOOKS.iter().enumerate() {
            for (other_name, other) in LOOKS.iter().skip(index + 1) {
                assert_ne!(
                    (look.brightness, look.contrast, look.saturation),
                    (other.brightness, other.contrast, other.saturation),
                    "{name} and {other_name} are the same grade"
                );
            }
        }
    }

    /// The first look is the way back: a row of grades with no way off them
    /// would be a trap.
    #[test]
    fn the_first_look_is_the_way_back_to_nothing() {
        let (name, look) = LOOKS[0];
        assert_eq!(name, "None");
        assert!(look.is_identity());
    }

    /// What the strength slider rests on: a look applied at a fraction is still
    /// recognisably that look, at that fraction, with nothing remembered
    /// anywhere but the grade itself.
    #[test]
    fn a_weakened_look_is_still_recognisably_itself() {
        for strength in [0.25_f32, 0.5, 0.75, 1.0] {
            let graded = ColorAdjust::IDENTITY.lerp(punchy(), strength);
            let (name, at) = bettercut_ui::panels::look_of(graded).expect("still a look");
            assert_eq!(name, "Punchy");
            assert!(
                (at - strength).abs() < 0.01,
                "applied at {strength}, read back at {at}"
            );
        }
    }

    /// A grade the user made by hand is nobody's preset, and the row shows
    /// none of them selected rather than the nearest.
    #[test]
    fn a_hand_made_grade_matches_no_look() {
        let hand_made = ColorAdjust {
            brightness: 1.3,
            contrast: 0.7,
            saturation: 1.4,
            temperature: 0.0,
            tint: 0.0,
            vibrance: 0.0,
        };
        assert_eq!(bettercut_ui::panels::look_of(hand_made), None);
    }

    /// Every look in the row is recognised as itself, at full strength.
    ///
    /// `look_of` returns the *first* preset a grade lies on, so two looks that
    /// a grade satisfies at once would make the row light up whichever comes
    /// earlier in the list. Warm and Cool are the pair most at risk of that:
    /// they differ only in the sign of the white balance, and everything else
    /// about them is close.
    #[test]
    fn every_look_is_recognised_as_itself() {
        for (name, look) in bettercut_ui::panels::LOOKS.iter().skip(1) {
            let (found, at) = bettercut_ui::panels::look_of(*look)
                .unwrap_or_else(|| panic!("{name} is not recognised as a look at all"));
            assert_eq!(found, *name, "{name} was reported as {found}");
            assert!((at - 1.0).abs() < 0.01, "{name} read back at strength {at}");
        }
    }

    /// And an untouched clip is not reported as a weak version of everything.
    #[test]
    fn an_untouched_clip_is_on_no_look() {
        assert_eq!(bettercut_ui::panels::look_of(ColorAdjust::IDENTITY), None);
    }
}

/// The Inspector's tabs (§59).
mod tabs {
    use bettercut_ui::panels::InspectorTab;

    /// Every tab is reachable and named: a tab missing from `ALL` exists in the
    /// enum and nowhere a user can click.
    #[test]
    fn every_tab_is_offered_and_named() {
        assert_eq!(InspectorTab::ALL.len(), 6);
        for tab in InspectorTab::ALL {
            assert!(!tab.label().is_empty(), "{tab:?} has no label");
        }
        for (index, tab) in InspectorTab::ALL.iter().enumerate() {
            for other in InspectorTab::ALL.iter().skip(index + 1) {
                assert_ne!(tab.label(), other.label(), "two tabs share a label");
            }
        }
    }

    /// Video is what opens on a fresh selection: placement is what a user
    /// reaches for constantly, and effects are a choice made once.
    #[test]
    fn video_is_the_default_tab() {
        assert_eq!(InspectorTab::default(), InspectorTab::Video);
        assert_eq!(InspectorTab::ALL[0], InspectorTab::Video);
    }

    /// Effects sits beside Video rather than at the end: the two are the halves
    /// of one question — where the picture is, and what is done to it.
    #[test]
    fn effects_sits_next_to_video() {
        assert_eq!(InspectorTab::ALL[1], InspectorTab::Effects);
    }
}

/// The curves editor grabs the handle nearest the pointer, and none when the
/// pointer is far from every handle.
#[test]
fn a_curve_handle_is_grabbed_where_it_is_drawn() {
    use bettercut_editor_core::timeline::curves::STRAIGHT;
    use bettercut_ui::panels::curve_handle_at;
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
    // The middle handle of a straight line sits in the middle of the square.
    assert_eq!(
        curve_handle_at(&STRAIGHT, rect, egui::pos2(100.0, 100.0)),
        Some(2)
    );
    assert_eq!(
        curve_handle_at(&STRAIGHT, rect, egui::pos2(3.0, 197.0)),
        Some(0)
    );
    assert_eq!(
        curve_handle_at(&STRAIGHT, rect, egui::pos2(100.0, 20.0)),
        None
    );
}
