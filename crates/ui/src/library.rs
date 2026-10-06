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
    Transitions,
    Filters,
}

impl LibraryTab {
    pub const ALL: [Self; 6] = [
        Self::Media,
        Self::Audio,
        Self::Text,
        Self::Stickers,
        Self::Transitions,
        Self::Filters,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Stickers => "Stickers",
            Self::Transitions => "Transitions",
            Self::Filters => "Filters",
        }
    }
}

/// The row of tabs.
pub fn tabs(ui: &mut egui::Ui, state: &mut UiState) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
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
    }
}
