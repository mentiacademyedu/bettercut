//! The Captions window: every caption in one list, editable in place.
//!
//! Timing captions from speech (§28) produces thirty blank captions in a
//! second, and then the user has to type into them. Doing that by clicking each
//! one on the timeline and typing in the Inspector is thirty selections and
//! thirty trips across the window — which would make the timing feature a way
//! of creating work rather than saving it.
//!
//! So: a list, one row per caption, in order. Type, press Enter, and the next
//! row takes the cursor. The timecode beside each row jumps the playhead there,
//! because the word you cannot make out is the one you need to hear again.
//!
//! The list is a *view*, not a copy. Every row reads the clip on each frame and
//! every keystroke goes through the editor, so an edit made here and an edit
//! made on the timeline cannot disagree.
//!
//! # Cutting by the words
//!
//! A caption is a line of speech *and* the stretch of timeline it was said in,
//! which makes the list the fastest way to cut an interview: the line that
//! rambles is the stretch to take out. ✂ on a row does exactly that —
//! `Editor::remove_time` over the caption's span, so the picture, the sound,
//! the captions after it and the markers all close up together, as one undo
//! step. It is the edit a user would otherwise make by marking in and out on
//! the timeline and counting on having got the ends right.

use bettercut_editor_core::Editor;
use bettercut_editor_core::command::TextProperty;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};

use crate::state::UiState;
use crate::theme;

/// One caption, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionRow {
    pub clip: ClipId,
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub text: String,
}

/// Every caption in the project's caption lane, in order.
///
/// Only the caption lane: titles live on their own text tracks and are placed,
/// styled and animated individually, so listing them here would mix two
/// different jobs in one window.
pub fn rows(editor: &Editor) -> Vec<CaptionRow> {
    let Some(sequence) = editor.active_sequence() else {
        return Vec::new();
    };
    let Some(track) = sequence
        .text_tracks
        .iter()
        .find(|track| track.name == Editor::CAPTION_TRACK)
    else {
        return Vec::new();
    };
    track
        .clips()
        .iter()
        .map(|clip| CaptionRow {
            clip: clip.id,
            start: clip.timeline.start,
            end: clip.timeline.end,
            text: clip.text.clone(),
        })
        .collect()
}

/// Whether a keystroke continues the edit already in progress (§11).
///
/// `continuing` is false on the first keystroke in a field and true after it,
/// so a typed sentence is one undo step. It matters *which* field: every
/// caption edit carries the same history label, so a keystroke marked as
/// continuing is folded into whatever text edit came last — and typing into one
/// caption and then another would put both into a single step, where one undo
/// wipes a caption the user was not touching.
pub fn still_typing(typing: Option<ClipId>, clip: ClipId) -> bool {
    typing == Some(clip)
}

/// Which row the playhead is inside, for highlighting.
///
/// The gaps between captions belong to no row, which is the honest answer: a
/// highlight that clung to the last caption through ten seconds of silence
/// would say the wrong thing about where you are.
pub fn row_at(rows: &[CaptionRow], playhead: TimelineTime) -> Option<usize> {
    rows.iter()
        .position(|row| playhead >= row.start && playhead < row.end)
}

/// Which look the lane is wearing, or `None` when it is dressed some other way
/// — a style edited by hand in the Inspector is not one of the four, and
/// showing all four unselected says that better than highlighting the nearest.
pub fn lane_look(
    editor: &Editor,
    rows: &[CaptionRow],
) -> Option<bettercut_editor_core::text::CaptionLook> {
    let first = rows.first()?;
    let sequence = editor.active_sequence()?;
    let track = sequence.text_track_of(first.clip)?;
    let style = &sequence.text_track(track)?.get(first.clip)?.style;
    bettercut_editor_core::text::CaptionLook::ALL
        .into_iter()
        .find(|option| &bettercut_editor_core::text::TextStyle::look(*option) == style)
}

/// Draw the window, if it is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.captions_open {
        return;
    }
    let mut open = true;
    let rows = rows(editor);
    let current = row_at(&rows, editor.playhead());

    // What the user typed, and whether they are still typing it: the edit is
    // applied after the loop, because the rows borrow the editor.
    let mut edit: Option<(ClipId, String, bool)> = None;
    let mut jump: Option<TimelineTime> = None;
    // The stretch a row asked to have taken out of the cut.
    let mut cut: Option<bettercut_editor_core::timeline::TimelineRange> = None;
    let mut focus_next = false;
    let mut typing = state.caption_typing;
    let mut look: Option<bettercut_editor_core::text::CaptionLook> = None;
    let current_look = lane_look(editor, &rows);
    // A change to the lane's word highlight: `Some(None)` turns it off.
    let mut highlight: Option<Option<bettercut_editor_core::text::Rgba>> = None;
    let current_highlight = editor.caption_highlight();

    egui::Window::new("Captions")
        .open(&mut open)
        .default_width(380.0)
        .default_height(420.0)
        .resizable(true)
        .show(ctx, |ui| {
            if rows.is_empty() {
                ui.label("No captions yet.");
                ui.label(
                    egui::RichText::new(
                        "Import a subtitle file, or right-click a clip with sound and choose \
                         Time Captions to put an empty caption on every phrase.",
                    )
                    .small()
                    .color(theme::DISABLED),
                );
                return;
            }

            ui.label(
                egui::RichText::new(format!("{} captions · Enter moves to the next", rows.len()))
                    .small()
                    .color(theme::DISABLED),
            );
            ui.add_space(4.0);

            // The look belongs to the lane: subtitles that changed style
            // halfway through read as a mistake.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("look").small().color(theme::DISABLED));
                for option in bettercut_editor_core::text::CaptionLook::ALL {
                    if ui
                        .selectable_label(current_look == Some(option), option.label())
                        .clicked()
                    {
                        look = Some(option);
                    }
                }
            });
            // Karaoke-style: the word being said lit in its own colour.
            ui.horizontal(|ui| {
                let mut on = current_highlight.is_some();
                if ui
                    .checkbox(&mut on, "light each word")
                    .on_hover_text("Colour each word as it is said, the way karaoke captions do")
                    .changed()
                {
                    highlight = Some(on.then_some(Editor::DEFAULT_HIGHLIGHT));
                }
                if let Some(colour) = current_highlight {
                    let mut rgba = egui::Color32::from_rgba_unmultiplied(
                        colour.r, colour.g, colour.b, colour.a,
                    );
                    if ui.color_edit_button_srgba(&mut rgba).changed() {
                        let [r, g, b, a] = rgba.to_srgba_unmultiplied();
                        highlight = Some(Some(bettercut_editor_core::text::Rgba::new(r, g, b, a)));
                    }
                }
            });
            ui.add_space(6.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, row) in rows.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let label = egui::RichText::new(row.start.format_timecode()).monospace();
                        let label = if current == Some(index) {
                            label.color(theme::PLAYHEAD)
                        } else {
                            label.color(theme::DISABLED)
                        };
                        if ui
                            .add(egui::Button::new(label).frame(false))
                            .on_hover_text("Play from here")
                            .clicked()
                        {
                            jump = Some(row.start);
                        }

                        // Taking the line out takes the stretch of the cut
                        // it was said in: the point of editing by the words.
                        if ui
                            .add(egui::Button::new("✂").frame(false))
                            .on_hover_text("Cut this line out of the video and close the gap")
                            .clicked()
                        {
                            cut = Some(bettercut_editor_core::timeline::TimelineRange {
                                start: row.start,
                                end: row.end,
                            });
                        }

                        let id = egui::Id::new(("caption", row.clip));
                        let mut text = row.text.clone();
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .id(id)
                                .desired_width(f32::INFINITY)
                                // A blank caption is the normal state right
                                // after timing, and an empty box with no hint
                                // looks like a bug rather than an invitation.
                                .hint_text("type what is said here"),
                        );
                        // One undo step for the whole sentence, not one per
                        // letter (§11).
                        if field.changed() {
                            edit = Some((row.clip, text, still_typing(typing, row.clip)));
                            typing = Some(row.clip);
                        }
                        if field.lost_focus() {
                            // The next keystroke, wherever it lands, starts its
                            // own step.
                            typing = None;
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                focus_next = true;
                            }
                        }
                        if focus_next && index + 1 < rows.len() {
                            let next = egui::Id::new(("caption", rows[index + 1].clip));
                            ui.ctx().memory_mut(|m| m.request_focus(next));
                            focus_next = false;
                        }
                    });
                }
            });
        });

    if let Some(range) = cut {
        match editor.remove_time(range) {
            Ok(0) => state.info("That line covers nothing to cut"),
            Ok(_) => {
                // The playhead follows the join, which is the next thing to
                // watch.
                editor.set_playhead(range.start);
                state.needs_repaint = true;
            }
            Err(err) => state.error(err.to_string()),
        }
    }
    if let Some((clip, text, continuing)) = edit
        && let Err(err) = editor.set_text_property(clip, TextProperty::Content(text), continuing)
    {
        state.error(err.to_string());
    }
    if let Some(colour) = highlight {
        match editor.set_caption_highlight(colour) {
            Ok(0) => {}
            Ok(n) => state.info(if colour.is_some() {
                format!("{n} captions light each word")
            } else {
                format!("{n} captions back to plain")
            }),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    if let Some(look) = look {
        match editor.set_caption_look(look) {
            Ok(0) => {}
            Ok(n) => state.info(format!("{n} captions restyled")),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
    }
    state.caption_typing = typing;
    if let Some(at) = jump {
        editor.set_playhead(at);
        state.needs_repaint = true;
    }
    state.captions_open = open && state.captions_open;
}
