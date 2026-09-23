//! The Templates window (§31, Milestone 11).
//!
//! §31's whole user flow, in one window: pick a template, drop media into its
//! slots, replace its text — then it lands on the timeline and export is the
//! only step left.
//!
//! The list on the left, the chosen template's slots on the right. A slot is a
//! dropdown of the project's media that fits it, or a text box already holding
//! the template's own wording. Nothing has to be filled: an empty media slot is
//! skipped, and the report afterwards says which — so trying a template before
//! gathering footage is possible, and the result is honest about what is
//! missing.
//!
//! Everything the window decides lives in [`TemplateDialog`]'s methods and the
//! egui code only calls them, so the behaviour is testable without driving
//! clicks through a layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bettercut_editor_core::foundation::{MediaId, TimelineTime};
use bettercut_editor_core::media::MediaKind;
use bettercut_editor_core::templates::library::{self, install, load_dir};
use bettercut_editor_core::templates::{SlotKind, Template, starters};
use bettercut_editor_core::{AppliedTemplate, Editor, SlotFill};

use crate::state::UiState;
use crate::theme;

#[derive(Debug, Default)]
pub struct TemplateDialog {
    pub open: bool,
    /// The built-in starters, then the user's own.
    templates: Vec<Template>,
    /// Where the user's templates are read from and added to.
    folder: Option<PathBuf>,
    /// Files in that folder that are not valid templates: name, and the
    /// first thing wrong with each.
    rejected: Vec<(String, String)>,
    selected: usize,
    /// `None` shows every category.
    category: Option<String>,
    /// Chosen media, by slot id. A slot absent here is left empty.
    media: HashMap<String, MediaId>,
    /// Wording, by slot id. Seeded with each text slot's default when a
    /// template is chosen, so the box shows what will appear.
    words: HashMap<String, String>,
}

impl TemplateDialog {
    pub fn open(&mut self) {
        self.open_with_folder(library::user_dir());
    }

    /// Open, reading the user's templates from `folder`.
    ///
    /// The folder is read again on every open, so a template dropped into it
    /// from outside the editor appears without a restart. What was chosen
    /// before survives, as long as that template is still there.
    pub fn open_with_folder(&mut self, folder: PathBuf) {
        let kept = self.selected().map(|t| t.id.clone());
        self.folder = Some(folder);
        self.reload();
        match kept.and_then(|id| self.index_of(&id)) {
            // Same template: its slots are still the ones filled in.
            Some(index) => self.selected = index,
            None => self.select(0),
        }
        self.open = true;
    }

    fn reload(&mut self) {
        let mut templates = starters();
        let found = self.folder.as_deref().map(load_dir).unwrap_or_default();
        templates.extend(found.templates);
        self.templates = templates;
        self.rejected = found
            .rejected
            .into_iter()
            .map(|(path, problems)| {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let why = problems
                    .first()
                    .map_or_else(|| "not a valid template".to_owned(), ToString::to_string);
                (name, why)
            })
            .collect();
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.templates.iter().position(|t| t.id == id)
    }

    /// Check a template file, add it to the user's folder, and choose it.
    pub fn add_file(&mut self, file: &Path) -> Result<String, String> {
        let folder = self.folder.clone().unwrap_or_else(library::user_dir);
        let template = install(file, &folder).map_err(|err| err.to_string())?;
        self.folder = Some(folder);
        self.reload();
        if let Some(index) = self.index_of(&template.id) {
            self.select(index);
        }
        Ok(format!("Added the template \"{}\"", template.name))
    }

    pub fn templates(&self) -> &[Template] {
        &self.templates
    }

    /// Files in the user's folder that could not be read, and why.
    pub fn rejected(&self) -> &[(String, String)] {
        &self.rejected
    }

    pub fn selected(&self) -> Option<&Template> {
        self.templates.get(self.selected)
    }

    /// Choose a template, starting its slots afresh.
    ///
    /// Fresh rather than carried over: two templates' slots only share an id
    /// by coincidence, and a clip carried into a slot the user never looked at
    /// would appear in the result unasked.
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.templates.len().saturating_sub(1));
        self.media.clear();
        self.words = self
            .selected()
            .map(|t| {
                t.slots
                    .iter()
                    .filter(|s| s.kind == SlotKind::Text)
                    .map(|s| (s.id.clone(), s.default_text.clone().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();
    }

    pub fn set_media(&mut self, slot: &str, media: Option<MediaId>) {
        match media {
            Some(media) => self.media.insert(slot.to_owned(), media),
            None => self.media.remove(slot),
        };
    }

    pub fn set_words(&mut self, slot: &str, words: impl Into<String>) {
        self.words.insert(slot.to_owned(), words.into());
    }

    /// What has been chosen, as the editor takes it.
    pub fn fills(&self) -> HashMap<String, SlotFill> {
        let media = self
            .media
            .iter()
            .map(|(slot, id)| (slot.clone(), SlotFill::Media(*id)));
        let words = self
            .words
            .iter()
            .map(|(slot, text)| (slot.clone(), SlotFill::Text(text.clone())));
        media.chain(words).collect()
    }

    /// Put the chosen template at the end of the timeline, and say what
    /// happened.
    ///
    /// The end, because that is the one place a template can never collide
    /// with the user's own clips, and on an empty project it is the start.
    pub fn apply(&mut self, editor: &mut Editor) -> Result<String, String> {
        let template = self
            .selected()
            .cloned()
            .ok_or_else(|| "Choose a template first".to_owned())?;
        let at = editor
            .active_sequence()
            .map_or(TimelineTime::ZERO, |s| s.duration());

        let applied = editor
            .apply_template(&template, &self.fills(), at)
            .map_err(|err| err.to_string())?;

        // Show where it went: the preview then displays the template's first
        // frame, which is the confirmation that something happened.
        editor.set_playhead(at);
        self.open = false;
        Ok(report(&template, &applied))
    }
}

/// One status-bar line: what landed, and anything the user should know is
/// missing from it.
fn report(template: &Template, applied: &AppliedTemplate) -> String {
    let label = |id: &String| {
        template
            .slot(id)
            .map_or_else(|| id.clone(), |s| s.label.clone())
    };
    let mut notes = Vec::new();
    if !applied.unfilled.is_empty() {
        let names: Vec<String> = applied.unfilled.iter().map(label).collect();
        notes.push(format!("left empty: {}", names.join(", ")));
    }
    if !applied.shortened.is_empty() {
        let names: Vec<String> = applied.shortened.iter().map(label).collect();
        notes.push(format!("shorter than its spot: {}", names.join(", ")));
    }
    if applied.dropped_transitions > 0 {
        notes.push(format!(
            "{} transition(s) had no footage to spare and were left out",
            applied.dropped_transitions
        ));
    }
    if notes.is_empty() {
        format!("Added {}", template.name)
    } else {
        format!("Added {} — {}", template.name, notes.join("; "))
    }
}

/// Which of the project's media a slot can take, by name.
///
/// Any picture fits any picture slot: a photo in a video slot holds still for
/// the shot, and a clip in a logo slot plays. Image slots list photos first,
/// since that is what they were made for.
fn candidates(editor: &Editor, kind: SlotKind) -> Vec<(MediaId, String)> {
    let mut fits: Vec<_> = editor
        .project()
        .media
        .iter()
        .filter(|m| !m.missing && (m.is_still() || !m.duration.is_zero()))
        .filter(|m| match kind {
            SlotKind::Audio => m.audio_codec.is_some(),
            SlotKind::Text => false,
            SlotKind::Video | SlotKind::Image | SlotKind::Logo => m.kind.has_video(),
        })
        .collect();
    if matches!(kind, SlotKind::Image | SlotKind::Logo) {
        fits.sort_by_key(|m| m.kind != MediaKind::Image);
    }
    fits.into_iter()
        .map(|m| (m.id, m.file_name.clone()))
        .collect()
}

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let mut dialog = std::mem::take(&mut state.template_dialog);
    if dialog.open {
        window(ctx, editor, state, &mut dialog);
    }
    state.template_dialog = dialog;
}

fn window(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut UiState,
    dialog: &mut TemplateDialog,
) {
    let mut open = dialog.open;
    egui::Window::new("Templates")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(640.0, 420.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    list(ui, state, dialog);
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_width(360.0);
                    slots(ui, editor, state, dialog);
                });
            });
        });
    // The close button, or a successful apply, closes the window.
    dialog.open = open && dialog.open;
}

fn list(ui: &mut egui::Ui, state: &mut UiState, dialog: &mut TemplateDialog) {
    let mut categories: Vec<String> = dialog
        .templates
        .iter()
        .map(|t| t.category.clone())
        .collect();
    categories.sort();
    categories.dedup();

    ui.horizontal_wrapped(|ui| {
        if ui
            .selectable_label(dialog.category.is_none(), "All")
            .clicked()
        {
            dialog.category = None;
        }
        for category in categories {
            let chosen = dialog.category.as_deref() == Some(category.as_str());
            if ui.selectable_label(chosen, &category).clicked() {
                dialog.category = Some(category);
            }
        }
    });
    ui.separator();

    if ui
        .button("Add template file…")
        .on_hover_text("Check a template (.json) and add it to your templates")
        .clicked()
        && let Some(file) = rfd::FileDialog::new()
            .add_filter("template", &["json"])
            .pick_file()
    {
        match dialog.add_file(&file) {
            Ok(message) => state.info(message),
            Err(message) => state.error(message),
        }
    }
    if !dialog.rejected.is_empty() {
        let details: Vec<String> = dialog
            .rejected
            .iter()
            .map(|(name, why)| format!("{name}: {why}"))
            .collect();
        ui.label(
            egui::RichText::new(format!(
                "{} file(s) in your templates folder could not be read",
                dialog.rejected.len()
            ))
            .small()
            .color(theme::error_text()),
        )
        .on_hover_text(details.join("\n"));
    }
    ui.add_space(4.0);

    let mut choose = None;
    egui::ScrollArea::vertical()
        .id_salt("template_list")
        .show(ui, |ui| {
            for (index, template) in dialog.templates.iter().enumerate() {
                if dialog
                    .category
                    .as_ref()
                    .is_some_and(|c| *c != template.category)
                {
                    continue;
                }
                let seconds = template.duration.ticks() as f64 / 960_000.0;
                let text = format!("{}\n{} · {seconds:.0} s", template.name, template.category);
                if ui
                    .selectable_label(index == dialog.selected, text)
                    .clicked()
                {
                    choose = Some(index);
                }
            }
        });
    if let Some(index) = choose
        && index != dialog.selected
    {
        dialog.select(index);
    }
}

fn slots(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState, dialog: &mut TemplateDialog) {
    let Some(template) = dialog.selected().cloned() else {
        ui.label(egui::RichText::new("No templates available.").color(theme::disabled()));
        return;
    };

    ui.heading(&template.name);
    if !template.description.is_empty() {
        ui.label(&template.description);
    }
    ui.add_space(6.0);

    egui::Grid::new("template_slots")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            for slot in &template.slots {
                ui.label(&slot.label)
                    .on_hover_text(format!("{} slot", slot.kind.label()));
                if slot.kind == SlotKind::Text {
                    let mut words = dialog.words.get(&slot.id).cloned().unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut words).desired_width(240.0))
                        .changed()
                    {
                        dialog.set_words(&slot.id, words);
                    }
                } else {
                    let options = candidates(editor, slot.kind);
                    let current = dialog.media.get(&slot.id).copied();
                    let shown = current
                        .and_then(|id| options.iter().find(|(m, _)| *m == id))
                        .map_or("Empty — skipped", |(_, name)| name.as_str())
                        .to_owned();
                    let mut chosen = current;
                    egui::ComboBox::from_id_salt(("slot", &slot.id))
                        .width(240.0)
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut chosen, None, "Empty — skipped");
                            for (id, name) in &options {
                                ui.selectable_value(&mut chosen, Some(*id), name);
                            }
                        });
                    if chosen != current {
                        dialog.set_media(&slot.id, chosen);
                    }
                }
                ui.end_row();
            }
        });

    if candidates(editor, SlotKind::Video).is_empty() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Import some clips first — the slots list your project's media.")
                .small()
                .color(theme::disabled()),
        );
    }

    ui.add_space(10.0);
    if ui
        .button("Add to timeline")
        .on_hover_text("Placed after everything already on the timeline, as one undo step")
        .clicked()
    {
        match dialog.apply(editor) {
            Ok(message) => {
                state.info(message);
                state.needs_repaint = true;
            }
            Err(message) => state.error(message),
        }
    }
}
