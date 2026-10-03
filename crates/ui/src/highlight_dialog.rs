//! "Find the good bits": the loud moments of a long clip, shaded on the
//! timeline, to keep or to mark.
//!
//! The same shape as the silence window — look before anything is cut — but
//! the other way round: there, the stretches shown are what goes; here they
//! are what stays.

use std::sync::Arc;

use bettercut_cache::Waveform;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, TimelineRange};
use bettercut_playback::highlights::{HighlightSettings, highlights};

use crate::state::UiState;
use crate::theme;

#[derive(Debug)]
pub struct HighlightDialog {
    clip: ClipId,
    snapshot: AudioClip,
    waveform: Arc<Waveform>,
    /// How far above the clip's own middle a moment must be, as a
    /// percentage for the slider.
    strictness: f32,
    least_ms: i64,
    padding_ms: i64,
    ranges: Vec<TimelineRange>,
}

impl HighlightDialog {
    /// Look at `clip`'s waveform, or `None` when there is none to read yet.
    pub fn open(
        editor: &Editor,
        waveforms: &mut crate::waveforms::WaveformStore,
        clip: ClipId,
    ) -> Option<Self> {
        let snapshot = editor.audio_clip(clip)?.clone();
        let waveform = waveforms.get(snapshot.media_id)?;
        let defaults = HighlightSettings::default();
        let mut dialog = Self {
            clip,
            snapshot,
            waveform,
            strictness: defaults.above,
            least_ms: defaults.least.ticks() / 960,
            padding_ms: defaults.padding.ticks() / 960,
            ranges: Vec::new(),
        };
        dialog.recompute();
        Some(dialog)
    }

    fn settings(&self) -> HighlightSettings {
        HighlightSettings {
            least: TimelineTime::from_millis(self.least_ms),
            padding: TimelineTime::from_millis(self.padding_ms),
            above: self.strictness,
            ..HighlightSettings::default()
        }
    }

    fn recompute(&mut self) {
        self.ranges = highlights(&self.snapshot, &self.waveform, self.settings());
    }

    /// The moments found, for the timeline to shade.
    pub fn ranges(&self) -> &[TimelineRange] {
        &self.ranges
    }

    /// How much would be kept, in total.
    pub fn total(&self) -> TimelineTime {
        TimelineTime::from_ticks(self.ranges.iter().map(|r| r.duration().ticks()).sum())
    }
}

/// Draw the window, if one is open, and do what it asks.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(mut dialog) = state.highlights.take() else {
        return;
    };
    let mut open = true;
    let mut keep = false;
    let mut mark = false;
    let mut cancelled = false;

    crate::theme::placed(egui::Window::new("Find the good bits"), ctx)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "The loudest moments of this clip — a cheer, a chorus, a laugh — shaded on the timeline. Nothing is cut until you say so.",
                )
                .small()
                .color(theme::disabled()),
            );
            ui.add_space(6.0);

            let mut changed = false;
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.strictness, 0.15..=0.85)
                        .text("how picky")
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                )
                .on_hover_text("Higher finds fewer, surer moments")
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.least_ms, 500..=6_000)
                        .text("at least")
                        .suffix(" ms"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.padding_ms, 0..=2_000)
                        .text("room either side")
                        .suffix(" ms"),
                )
                .changed();
            if changed {
                dialog.recompute();
            }

            ui.add_space(6.0);
            let found = dialog.ranges.len();
            ui.label(match found {
                0 => "Nothing stands out — try being less picky.".to_owned(),
                1 => format!("1 moment · {}", dialog.total().format_timecode()),
                n => format!("{n} moments · {} in total", dialog.total().format_timecode()),
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(found > 0, egui::Button::new("Keep Only These"))
                    .on_hover_text("Cut everything else out of this clip and close the gaps. Undoable.")
                    .clicked()
                {
                    keep = true;
                }
                if ui
                    .add_enabled(found > 0, egui::Button::new("Mark Them"))
                    .on_hover_text("Leave the clip alone and put a marker at each moment")
                    .clicked()
                {
                    mark = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    if keep {
        let ranges = dialog.ranges.clone();
        match editor.keep_only(dialog.clip, &ranges) {
            Ok(0) => state.info("Nothing was cut"),
            Ok(n) => state.info(format!(
                "Kept {} moment(s), cut {n} stretch(es)",
                ranges.len()
            )),
            Err(err) => state.error(err.to_string()),
        }
        state.clear_selection();
        open = false;
    }
    if mark {
        let starts: Vec<TimelineTime> = dialog.ranges.iter().map(|range| range.start).collect();
        match editor.add_markers(&starts) {
            Ok(n) => state.info(format!("Marked {n} moment(s)")),
            Err(err) => state.error(err.to_string()),
        }
        open = false;
    }
    state.needs_repaint = true;
    if open && !cancelled {
        state.highlights = Some(dialog);
    }
}
