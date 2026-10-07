//! The tabs along the top of the left panel, as in CapCut: the media, and the
//! things that can be put on it — text, stickers, transitions and filters —
//! laid out to be seen and clicked, rather than found in menus. Each runs the
//! same editor action the Add menu, the clip menu or the inspector runs.

use bettercut_editor_core::filters::Filter;
use bettercut_editor_core::foundation::ClipId;
use bettercut_editor_core::stickers::STICKERS;
use bettercut_editor_core::text::TextPreset;
use bettercut_editor_core::timeline::{MIN_TRANSITION, TransitionKind};
use bettercut_editor_core::{Editor, TextProperty};

use crate::panels::InspectorTab;
use crate::state::UiState;
use crate::theme;

/// Which tab the left panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryTab {
    #[default]
    Media,
    Audio,
    Text,
    Stickers,
    Effects,
    Transitions,
    Filters,
}

impl LibraryTab {
    pub const ALL: [Self; 7] = [
        Self::Media,
        Self::Audio,
        Self::Text,
        Self::Stickers,
        Self::Effects,
        Self::Transitions,
        Self::Filters,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Stickers => "Stickers",
            Self::Effects => "Effects",
            Self::Transitions => "Transitions",
            Self::Filters => "Filters",
        }
    }
}

/// The row of tabs.
pub fn tabs(ui: &mut egui::Ui, state: &mut UiState) {
    ui.horizontal_wrapped(|ui| {
        // Tight, so the seven fit on two rows at the panel's usual width.
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 2.0);
        ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
        for tab in LibraryTab::ALL {
            if ui
                .selectable_label(state.library_tab == tab, tab.label())
                .clicked()
            {
                state.library_tab = tab;
            }
        }
    });
    ui.separator();
}

/// The tab's contents, for every tab but Media (which is the media browser).
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    egui::ScrollArea::vertical().show(ui, |ui| match state.library_tab {
        LibraryTab::Media => {}
        LibraryTab::Audio => audio_tab(ui, editor, state),
        LibraryTab::Text => text_tab(ui, editor, state),
        LibraryTab::Stickers => sticker_tab(ui, editor, state),
        LibraryTab::Effects => effect_tab(ui, editor, state),
        LibraryTab::Transitions => transition_tab(ui, editor, state),
        LibraryTab::Filters => filter_tab(ui, editor, state),
    });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(theme::disabled()));
    ui.add_space(4.0);
}

/// A button as wide as the panel, for a list of things to pick.
fn wide(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add_sized(
        egui::vec2(ui.available_width(), 26.0),
        egui::Button::new(label),
    )
}

fn audio_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    use bettercut_editor_core::media::GeneratedSound;
    hint(
        ui,
        "Sound effects: click to put one at the playhead, on a free sound lane",
    );
    let mut chosen = None;
    for sound in GeneratedSound::EFFECTS {
        if wide(ui, &sound.name())
            .on_hover_text(sound.description().unwrap_or_default())
            .clicked()
        {
            chosen = Some(sound);
        }
    }
    if let Some(sound) = chosen {
        let seconds = sound.natural_length().unwrap_or(1.0);
        let length = bettercut_editor_core::foundation::TimelineTime::from_millis(
            (seconds * 1000.0).ceil() as i64,
        );
        match editor.add_generated_sound(sound, length) {
            Ok(clip) => {
                state.select_only(clip);
                state.inspector_tab = InspectorTab::Audio;
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

fn text_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    hint(ui, "Click to put a title at the playhead");
    let mut chosen: Option<Option<TextPreset>> = None;
    if wide(ui, "Default text")
        .on_hover_text("White with a black outline, ready to type into")
        .clicked()
    {
        chosen = Some(None);
    }
    for preset in TextPreset::ALL {
        // Each in its own colours, so the list shows what it gives.
        let look = preset.applied_to(&bettercut_editor_core::text::TextStyle::default());
        let rgb = |c: bettercut_editor_core::text::Rgba| egui::Color32::from_rgb(c.r, c.g, c.b);
        let mut button = egui::Button::new(
            egui::RichText::new(preset.label())
                .strong()
                .color(rgb(look.color)),
        );
        if let Some(background) = look.background {
            button = button.fill(rgb(background.color));
        }
        if ui
            .add_sized(egui::vec2(ui.available_width(), 26.0), button)
            .on_hover_text(preset.description())
            .clicked()
        {
            chosen = Some(Some(preset));
        }
    }
    if let Some(preset) = chosen {
        add_styled_text(editor, state, preset);
    }

    // Whole designs: the font, the size and where the title sits.
    ui.add_space(8.0);
    hint(ui, "Designs: a title already styled and placed");
    let mut design = None;
    for look in bettercut_editor_core::text::TitleLook::ALL {
        if wide(ui, look.label())
            .on_hover_text(match look {
                bettercut_editor_core::text::TitleLook::Headline => "Big and bold, near the middle",
                bettercut_editor_core::text::TitleLook::LowerThird => {
                    "A boxed strip low on the left, for a name or a place"
                }
                bettercut_editor_core::text::TitleLook::Quote => "Light serif with room to breathe",
                bettercut_editor_core::text::TitleLook::Typewriter => {
                    "Monospaced and spaced out, for the typewriter entrance"
                }
            })
            .clicked()
        {
            design = Some(look);
        }
    }
    if let Some(look) = design {
        add_title_design(editor, state, look);
    }
}

/// A title at the playhead in one of the whole designs, as one undo step.
pub fn add_title_design(
    editor: &mut Editor,
    state: &mut UiState,
    look: bettercut_editor_core::text::TitleLook,
) {
    let depth = editor.undo_depth();
    let clip = match editor.add_text(look.label()) {
        Ok(clip) => clip,
        Err(err) => {
            state.error(err.to_string());
            return;
        }
    };
    if let Err(err) = editor.set_title_look(clip, look) {
        state.error(err.to_string());
    }
    let steps = editor.undo_depth().saturating_sub(depth);
    editor.merge_last_steps(steps, &format!("Add {} Title", look.label()));
    state.select_only(clip);
    state.inspector_tab = InspectorTab::Video;
    state.needs_repaint = true;
}

/// A title at the playhead, wearing `preset`, as one undo step; selected so
/// the inspector is showing the box to type in.
pub fn add_styled_text(editor: &mut Editor, state: &mut UiState, preset: Option<TextPreset>) {
    let depth = editor.undo_depth();
    let clip = match editor.add_text("Text") {
        Ok(clip) => clip,
        Err(err) => {
            state.error(err.to_string());
            return;
        }
    };
    if let Some(preset) = preset {
        if let Some(style) = editor
            .text_clip(clip)
            .map(|title| preset.applied_to(&title.style))
            && let Err(err) =
                editor.set_text_property(clip, TextProperty::Style(Box::new(style)), false)
        {
            state.error(err.to_string());
        }
        let steps = editor.undo_depth().saturating_sub(depth);
        editor.merge_last_steps(steps, &format!("Add {} Text", preset.label()));
    }
    state.select_only(clip);
    state.inspector_tab = InspectorTab::Video;
    state.needs_repaint = true;
}

fn sticker_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    hint(ui, "Click to put one at the playhead");
    let mut chosen = None;
    let size = 44.0;
    let columns = ((ui.available_width() / (size + 6.0)).floor() as usize).max(1);
    egui::Grid::new("library stickers")
        .spacing(egui::vec2(6.0, 6.0))
        .show(ui, |ui| {
            for (index, (sticker, name)) in STICKERS.iter().enumerate() {
                if ui
                    .add_sized(
                        egui::vec2(size, size),
                        egui::Button::new(egui::RichText::new(*sticker).size(24.0)),
                    )
                    .on_hover_text(*name)
                    .clicked()
                {
                    chosen = Some(*sticker);
                }
                if index % columns == columns - 1 {
                    ui.end_row();
                }
            }
        });
    if let Some(sticker) = chosen {
        match editor.add_sticker(sticker) {
            Ok(clip) => {
                state.select_only(clip);
                state.inspector_tab = InspectorTab::Video;
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
}

/// The one selected picture clip, if exactly one is selected.
fn selected_picture(editor: &Editor, state: &UiState) -> Option<ClipId> {
    let mut pictures = state
        .selected_clips
        .iter()
        .copied()
        .filter(|clip| editor.video_clip(*clip).is_some());
    let first = pictures.next()?;
    pictures.next().is_none().then_some(first)
}

fn transition_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let Some(clip) = selected_picture(editor, state) else {
        hint(
            ui,
            "Select the clip before a cut, then click a transition to put it on that cut",
        );
        for kind in TransitionKind::ALL {
            ui.add_enabled(false, egui::Button::new(kind.label()))
                .on_disabled_hover_text(kind.description());
        }
        return;
    };
    hint(ui, "Click to put it on the cut after the selected clip");
    let existing = editor.video_clip(clip).and_then(|c| c.transition_out);
    let mut chosen = None;
    for kind in TransitionKind::ALL {
        let room = editor.transition_room(clip, kind);
        let overlap = editor.transition_overlap(clip, kind);
        let usable = room.is_some_and(|r| r >= MIN_TRANSITION) || overlap.is_some();
        let hover = match overlap {
            Some(shorter) => crate::panels::overlap_hint(kind, shorter),
            None if room.is_none() => {
                "There is no clip straight after this one to blend into".to_owned()
            }
            None if !usable => "These clips are too short to blend into each other".to_owned(),
            None => kind.description().to_owned(),
        };
        let selected = existing.is_some_and(|t| t.kind == kind);
        let response = ui
            .add_enabled(
                usable,
                egui::Button::selectable(selected, kind.label())
                    .min_size(egui::vec2(ui.available_width(), 26.0)),
            )
            .on_hover_text(hover.clone())
            .on_disabled_hover_text(hover);
        if response.clicked() {
            chosen = Some(kind);
        }
    }
    if let Some(kind) = chosen {
        match editor.set_transition(clip, kind) {
            Ok(()) => state.needs_repaint = true,
            Err(err) => state.error(err.to_string()),
        }
    }

    // CapCut's "Apply to all": the same transition on every cut of the lane.
    if let Some(transition) = existing {
        ui.add_space(6.0);
        if ui
            .add_sized(
                egui::vec2(ui.available_width(), 26.0),
                egui::Button::new(format!("{} on every cut", transition.kind.label())),
            )
            .on_hover_text(
                "Put this transition on every cut of this clip's lane, as one undo step. \
                 Clips with no footage to spare overlap to make room",
            )
            .clicked()
            && let Some(track) = editor.track_of(clip)
        {
            match editor.transition_every_cut(track, transition.kind) {
                Ok((applied, 0)) => {
                    state.info(format!("{} on {applied} cuts", transition.kind.label()))
                }
                Ok((applied, skipped)) => state.info(format!(
                    "{} on {applied} cuts; {skipped} too short to take one",
                    transition.kind.label()
                )),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    }
}

fn effect_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    use bettercut_editor_core::effects::{EFFECTS, ONE_CLICK_AMOUNT};
    let onto: Vec<ClipId> = state
        .selected_clips
        .iter()
        .copied()
        .filter(|clip| editor.video_clip(*clip).is_some())
        .collect();
    hint(
        ui,
        if onto.is_empty() {
            "Select picture clips, then click an effect to put it on them"
        } else {
            "Click to switch an effect on or off; set how strong in the inspector's Effects tab"
        },
    );
    let mut chosen = None;
    for effect in &EFFECTS {
        let on = onto
            .first()
            .and_then(|clip| editor.video_clip(*clip))
            .is_some_and(|clip| effect.amount_on(clip) > 0.0);
        let response = ui
            .add_enabled(
                !onto.is_empty(),
                egui::Button::selectable(on, effect.name)
                    .min_size(egui::vec2(ui.available_width(), 26.0)),
            )
            .on_hover_text(effect.description)
            .on_disabled_hover_text(effect.description);
        if response.clicked() {
            chosen = Some((effect, if on { 0.0 } else { ONE_CLICK_AMOUNT }));
        }
    }
    if let Some((effect, amount)) = chosen {
        let depth = editor.undo_depth();
        for clip in &onto {
            if let Err(err) = editor.set_clip_property(*clip, effect.at(amount), false) {
                state.error(err.to_string());
                break;
            }
        }
        let steps = editor.undo_depth().saturating_sub(depth);
        editor.merge_last_steps(
            steps,
            &if amount > 0.0 {
                format!("Add {}", effect.name)
            } else {
                format!("Remove {}", effect.name)
            },
        );
        if amount > 0.0 {
            state.inspector_tab = InspectorTab::Effects;
        }
        state.needs_repaint = true;
    }
}

fn filter_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let onto: Vec<ClipId> = state
        .selected_clips
        .iter()
        .copied()
        .filter(|clip| editor.video_clip(*clip).is_some())
        .collect();
    hint(
        ui,
        if onto.is_empty() {
            "Select picture clips, then click a filter to give them that look"
        } else {
            "Click to give the selected clips this look"
        },
    );
    let current = onto.first().and_then(|clip| editor.filter_of(*clip));
    let mut chosen = None;
    for filter in Filter::ALL {
        let response = ui
            .add_enabled(
                !onto.is_empty(),
                egui::Button::selectable(current == Some(filter), filter.label())
                    .min_size(egui::vec2(ui.available_width(), 26.0)),
            )
            .on_hover_text(filter.description())
            .on_disabled_hover_text(filter.description());
        if response.clicked() {
            chosen = Some(filter);
        }
    }
    if let Some(filter) = chosen {
        match editor.apply_filter(filter, onto) {
            Ok(n) if n > 1 => state.info(format!("{} on {n} clips", filter.label())),
            Ok(_) => {}
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
        return;
    }

    // CapCut's "Apply to all": the selected clip's look on every picture clip.
    if let Some(filter) = current {
        ui.add_space(6.0);
        if ui
            .add_sized(
                egui::vec2(ui.available_width(), 26.0),
                egui::Button::new(if filter == Filter::Original {
                    "No look on any clip".to_owned()
                } else {
                    format!("{} on every clip", filter.label())
                }),
            )
            .on_hover_text("Give every picture clip in the edit this look, as one undo step")
            .clicked()
        {
            let every: Vec<ClipId> = editor
                .active_sequence()
                .map(|sequence| {
                    sequence
                        .video_tracks
                        .iter()
                        .flat_map(|track| track.clips().iter().map(|clip| clip.id))
                        .collect()
                })
                .unwrap_or_default();
            match editor.apply_filter(filter, every) {
                Ok(n) => state.info(format!("{} on {n} clips", filter.label())),
                Err(err) => state.error(err.to_string()),
            }
            state.needs_repaint = true;
        }
    }
}
