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

/// Something being dragged from the left panel towards the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LibraryDrag {
    /// A file: lands where it is let go, pushing clips aside.
    Media(bettercut_editor_core::foundation::MediaId),
    /// These land at the moment they are let go at.
    Sticker(&'static str),
    Text(Option<TextPreset>),
    Design(bettercut_editor_core::text::TitleLook),
    /// An index into [ANIMATED_TEXT].
    Animated(usize),
    Sound(bettercut_editor_core::media::GeneratedSound),
    /// These land on the clip they are let go on (an index into
    /// `effects::EFFECTS`).
    Effect(usize),
    Filter(Filter),
    /// On the cut at the end of the clip it is let go on.
    Transition(TransitionKind),
}

impl LibraryDrag {
    /// What the chip following the pointer says.
    pub fn label(self, editor: &Editor) -> String {
        match self {
            Self::Media(media) => editor
                .project()
                .media_asset(media)
                .map_or_else(String::new, |m| m.display_name().to_owned()),
            Self::Sticker(sticker) => sticker.to_owned(),
            Self::Text(None) => "Text".to_owned(),
            Self::Text(Some(preset)) => format!("{} text", preset.label()),
            Self::Design(look) => look.label().to_owned(),
            Self::Animated(index) => ANIMATED_TEXT
                .get(index)
                .map_or_else(String::new, |a| a.name.to_owned()),
            Self::Sound(sound) => sound.name(),
            Self::Effect(index) => bettercut_editor_core::effects::EFFECTS
                .get(index)
                .map_or_else(String::new, |e| e.name.to_owned()),
            Self::Filter(filter) => filter.label().to_owned(),
            Self::Transition(kind) => kind.label().to_owned(),
        }
    }

    /// Whether it lands on a clip rather than at a moment.
    pub fn onto_a_clip(self) -> bool {
        matches!(
            self,
            Self::Effect(_) | Self::Filter(_) | Self::Transition(_)
        )
    }
}

/// Start dragging `item` when `response` starts being dragged.
fn draggable(response: &egui::Response, state: &mut UiState, item: LibraryDrag) {
    if response.drag_started() {
        state.dragging = Some(item);
    }
}

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
        egui::Button::new(label).sense(egui::Sense::click_and_drag()),
    )
}

/// `items` two to a row, as CapCut lays out its effects, transitions and
/// filters: one long column ran off the bottom of the panel, hiding the last
/// few. `cell` draws one, given its index and the width each gets.
fn two_to_a_row<'a, T>(
    ui: &mut egui::Ui,
    items: &'a [T],
    mut cell: impl FnMut(&mut egui::Ui, usize, &'a T, f32),
) {
    let width = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(40.0);
    for (row, pair) in items.chunks(2).enumerate() {
        ui.horizontal(|ui| {
            for (column, item) in pair.iter().enumerate() {
                cell(ui, row * 2 + column, item, width);
            }
        });
    }
}

/// A library button in a [`two_to_a_row`] cell: picked out when `on`, and
/// draggable onto the timeline.
fn cell_button(ui: &mut egui::Ui, on: bool, label: &str, width: f32) -> egui::Response {
    cell(ui, egui::Button::selectable(on, label), width)
}

/// Any button in a [`two_to_a_row`] cell, draggable.
fn cell(ui: &mut egui::Ui, button: egui::Button, width: f32) -> egui::Response {
    // Held to its half of the row: a long name wraps onto a second line
    // rather than widening the whole panel.
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.add(
            button
                .wrap_mode(egui::TextWrapMode::Wrap)
                .min_size(egui::vec2(width, 26.0))
                .sense(egui::Sense::click_and_drag()),
        )
    })
    .inner
}

fn audio_tab(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    use bettercut_editor_core::media::GeneratedSound;
    hint(
        ui,
        "Sound effects: click to put one at the playhead, or drag it onto the timeline",
    );
    let mut chosen = None;
    for sound in GeneratedSound::EFFECTS {
        let response =
            wide(ui, &sound.name()).on_hover_text(sound.description().unwrap_or_default());
        draggable(&response, state, LibraryDrag::Sound(sound));
        if response.clicked() {
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
    hint(
        ui,
        "Click to put a title at the playhead, or drag it onto the timeline",
    );
    let mut chosen: Option<Option<TextPreset>> = None;
    let response =
        wide(ui, "Default text").on_hover_text("White with a black outline, ready to type into");
    draggable(&response, state, LibraryDrag::Text(None));
    if response.clicked() {
        chosen = Some(None);
    }
    two_to_a_row(ui, &TextPreset::ALL, |ui, _, &preset, width| {
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
        let response = cell(ui, button, width).on_hover_text(preset.description());
        draggable(&response, state, LibraryDrag::Text(Some(preset)));
        if response.clicked() {
            chosen = Some(Some(preset));
        }
    });
    if let Some(preset) = chosen {
        add_styled_text(editor, state, preset);
    }

    // Whole designs: the font, the size and where the title sits.
    ui.add_space(8.0);
    hint(ui, "Designs: a title already styled and placed");
    let mut design = None;
    two_to_a_row(
        ui,
        &bettercut_editor_core::text::TitleLook::ALL,
        |ui, _, &look, width| {
            let response = cell_button(ui, false, look.label(), width).on_hover_text(match look {
                bettercut_editor_core::text::TitleLook::Headline => "Big and bold, near the middle",
                bettercut_editor_core::text::TitleLook::LowerThird => {
                    "A boxed strip low on the left, for a name or a place"
                }
                bettercut_editor_core::text::TitleLook::Quote => "Light serif with room to breathe",
                bettercut_editor_core::text::TitleLook::Typewriter => {
                    "Monospaced and spaced out, for the typewriter entrance"
                }
            });
            draggable(&response, state, LibraryDrag::Design(look));
            if response.clicked() {
                design = Some(look);
            }
        },
    );
    if let Some(look) = design {
        add_title_design(editor, state, look);
    }

    // Ready-made movement: a look and how it comes in, goes out or keeps moving.
    ui.add_space(8.0);
    hint(ui, "Animated: titles that move in, out or all along");
    let mut animated = None;
    two_to_a_row(ui, &ANIMATED_TEXT, |ui, index, which, width| {
        let response = cell_button(ui, false, which.name, width).on_hover_text(which.description);
        draggable(&response, state, LibraryDrag::Animated(index));
        if response.clicked() {
            animated = Some(which);
        }
    });
    if let Some(which) = animated {
        add_animated_text(editor, state, which);
    }
}

/// A moving title, ready made: a look, then how it comes in, goes out or
/// keeps moving — CapCut's animated text, out of what titles already do.
pub struct AnimatedText {
    pub name: &'static str,
    pub description: &'static str,
    pub look: Option<bettercut_editor_core::text::TitleLook>,
    pub preset: Option<TextPreset>,
    /// Entrance and exit: the kind and its length in milliseconds.
    pub intro: Option<(bettercut_editor_core::timeline::MotionKind, i64)>,
    pub outro: Option<(bettercut_editor_core::timeline::MotionKind, i64)>,
    pub looping: Option<bettercut_editor_core::timeline::LoopMotion>,
}

/// The animated titles the Text tab offers.
pub const ANIMATED_TEXT: [AnimatedText; 7] = {
    use bettercut_editor_core::text::TitleLook;
    use bettercut_editor_core::timeline::{LoopMotion, MotionKind};
    [
        AnimatedText {
            name: "Pop in",
            description: "Big and bright, popping in and fading away",
            look: Some(TitleLook::Headline),
            preset: Some(TextPreset::Pop),
            intro: Some((MotionKind::Pop, 400)),
            outro: Some((MotionKind::Fade, 300)),
            looping: None,
        },
        AnimatedText {
            name: "Typed out",
            description: "Letters typed one at a time",
            look: Some(TitleLook::Typewriter),
            preset: None,
            intro: Some((MotionKind::Typewriter, 1_200)),
            outro: Some((MotionKind::Fade, 300)),
            looping: None,
        },
        AnimatedText {
            name: "Bounce",
            description: "Drops in and bounces to rest",
            look: Some(TitleLook::Headline),
            preset: None,
            intro: Some((MotionKind::Bounce, 700)),
            outro: Some((MotionKind::Fade, 300)),
            looping: None,
        },
        AnimatedText {
            name: "Rise",
            description: "Slides up into place and down away",
            look: None,
            preset: Some(TextPreset::Classic),
            intro: Some((MotionKind::SlideUp, 500)),
            outro: Some((MotionKind::SlideDown, 400)),
            looping: None,
        },
        AnimatedText {
            name: "Neon pulse",
            description: "Glowing letters that gently pulse",
            look: None,
            preset: Some(TextPreset::Neon),
            intro: Some((MotionKind::Fade, 400)),
            outro: None,
            looping: Some(LoopMotion::Pulse),
        },
        AnimatedText {
            name: "Wiggle",
            description: "Black on yellow, rocking for attention",
            look: None,
            preset: Some(TextPreset::Highlight),
            intro: Some((MotionKind::Pop, 300)),
            outro: None,
            looping: Some(LoopMotion::Wiggle),
        },
        AnimatedText {
            name: "Drift",
            description: "Soft and quiet, floating gently",
            look: None,
            preset: Some(TextPreset::Soft),
            intro: Some((MotionKind::Fade, 600)),
            outro: Some((MotionKind::Fade, 600)),
            looping: Some(LoopMotion::Float),
        },
    ]
};

/// A title at the playhead as one of [`ANIMATED_TEXT`], as one undo step.
pub fn add_animated_text(editor: &mut Editor, state: &mut UiState, which: &AnimatedText) {
    use bettercut_editor_core::foundation::TimelineTime;
    use bettercut_editor_core::timeline::Motion;
    let depth = editor.undo_depth();
    let clip = match editor.add_text(which.name) {
        Ok(clip) => clip,
        Err(err) => {
            state.error(err.to_string());
            return;
        }
    };
    let mut result = Ok(());
    if let Some(look) = which.look {
        result = editor.set_title_look(clip, look);
    }
    if result.is_ok()
        && let Some(preset) = which.preset
        && let Some(style) = editor
            .text_clip(clip)
            .map(|title| preset.applied_to(&title.style))
    {
        result = editor.set_text_property(clip, TextProperty::Style(Box::new(style)), false);
    }
    if result.is_ok()
        && let Some(mut animation) = editor.text_clip(clip).map(|title| title.animation)
    {
        let motion = |(kind, ms): (_, i64)| Motion::new(kind, TimelineTime::from_millis(ms));
        animation.intro = which.intro.map(motion);
        animation.outro = which.outro.map(motion);
        animation.looping = which.looping;
        result = editor.set_text_property(clip, TextProperty::Animation(animation), false);
    }
    if let Err(err) = result {
        state.error(err.to_string());
    }
    let steps = editor.undo_depth().saturating_sub(depth);
    editor.merge_last_steps(steps, &format!("Add {} Title", which.name));
    state.select_only(clip);
    state.inspector_tab = InspectorTab::Video;
    state.needs_repaint = true;
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
    hint(
        ui,
        "Click to put one at the playhead, or drag it onto the timeline",
    );
    let mut chosen = None;
    let size = 44.0;
    let columns = ((ui.available_width() / (size + 6.0)).floor() as usize).max(1);
    egui::Grid::new("library stickers")
        .spacing(egui::vec2(6.0, 6.0))
        .show(ui, |ui| {
            for (index, (sticker, name)) in STICKERS.iter().enumerate() {
                let response = ui
                    .add_sized(
                        egui::vec2(size, size),
                        egui::Button::new(egui::RichText::new(*sticker).size(24.0))
                            .sense(egui::Sense::click_and_drag()),
                    )
                    .on_hover_text(*name);
                draggable(&response, state, LibraryDrag::Sticker(sticker));
                if response.clicked() {
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
            "Drag a transition onto a clip to put it on the cut at its end, or select the clip and click",
        );
        two_to_a_row(ui, &TransitionKind::ALL, |ui, _, &kind, width| {
            let response =
                cell_button(ui, false, kind.label(), width).on_hover_text(kind.description());
            draggable(&response, state, LibraryDrag::Transition(kind));
            if response.clicked() {
                state.info("Select the clip before the cut first, or drag the transition onto it");
            }
        });
        return;
    };
    hint(ui, "Click to put it on the cut after the selected clip");
    let existing = editor.video_clip(clip).and_then(|c| c.transition_out);
    let mut chosen = None;
    two_to_a_row(ui, &TransitionKind::ALL, |ui, _, &kind, width| {
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
        // Enabled even where this cut cannot take it: it can still be dragged
        // onto another clip.
        let response = cell_button(ui, selected, kind.label(), width).on_hover_text(hover.clone());
        draggable(&response, state, LibraryDrag::Transition(kind));
        if response.clicked() {
            if usable {
                chosen = Some(kind);
            } else {
                state.info(hover);
            }
        }
    });
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
            "Drag an effect onto a clip, or select clips and click"
        } else {
            "Click to switch an effect on or off; set how strong in the inspector's Effects tab"
        },
    );
    let mut chosen = None;
    two_to_a_row(ui, &EFFECTS, |ui, index, effect, width| {
        let on = onto
            .first()
            .and_then(|clip| editor.video_clip(*clip))
            .is_some_and(|clip| effect.amount_on(clip) > 0.0);
        let response = cell_button(ui, on, effect.name, width).on_hover_text(effect.description);
        draggable(&response, state, LibraryDrag::Effect(index));
        if response.clicked() {
            if onto.is_empty() {
                state.info("Select picture clips first, or drag the effect onto one");
            } else {
                chosen = Some((effect, if on { 0.0 } else { ONE_CLICK_AMOUNT }));
            }
        }
    });
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
            "Drag a filter onto a clip, or select clips and click"
        } else {
            "Click to give the selected clips this look"
        },
    );
    let current = onto.first().and_then(|clip| editor.filter_of(*clip));
    let mut chosen = None;
    two_to_a_row(ui, &Filter::ALL, |ui, _, &filter, width| {
        let response = cell_button(ui, current == Some(filter), filter.label(), width)
            .on_hover_text(filter.description());
        draggable(&response, state, LibraryDrag::Filter(filter));
        if response.clicked() {
            if onto.is_empty() {
                state.info("Select picture clips first, or drag the filter onto one");
            } else {
                chosen = Some(filter);
            }
        }
    });
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
