//! The Remove Silences window (§78).
//!
//! §78's flow, in order: analyse the clip's sound, **show** the suggested cuts,
//! and only cut when the user says so. The suggestion is shaded on the
//! timeline while this is open, because a count of silences means nothing
//! without seeing where they fall — and the two sliders change it live, so
//! "that took out a breath" is fixed by moving a slider rather than by undoing
//! and guessing again.

use std::sync::Arc;

use bettercut_cache::Waveform;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};
use bettercut_editor_core::timeline::{AudioClip, TimelineRange};
use bettercut_playback::{SilenceSettings, silent_ranges};

use crate::state::UiState;
use crate::theme;

#[derive(Debug)]
pub struct SilenceDialog {
    /// The sound being examined. Cutting applies to it and to the picture
    /// linked to it (§12).
    clip: ClipId,
    /// As it was when the window opened; the cut is checked against the real
    /// clip when it is made.
    snapshot: AudioClip,
    waveform: Arc<Waveform>,
    threshold_db: f32,
    shortest_ms: i64,
    padding_ms: i64,
    ranges: Vec<TimelineRange>,
}

impl SilenceDialog {
    /// Examine `clip`, or `None` when its waveform has not been made yet.
    pub fn open(
        editor: &Editor,
        waveforms: &mut crate::waveforms::WaveformStore,
        clip: ClipId,
    ) -> Option<Self> {
        let snapshot = editor.audio_clip(clip)?.clone();
        let waveform = waveforms.get(snapshot.media_id)?;
        let defaults = SilenceSettings::default();
        let mut dialog = Self {
            clip,
            snapshot,
            waveform,
            threshold_db: defaults.threshold_db,
            shortest_ms: defaults.shortest.ticks() / 960,
            padding_ms: defaults.padding.ticks() / 960,
            ranges: Vec::new(),
        };
        dialog.recompute();
        Some(dialog)
    }

    fn settings(&self) -> SilenceSettings {
        SilenceSettings {
            threshold_db: self.threshold_db,
            shortest: TimelineTime::from_millis(self.shortest_ms),
            padding: TimelineTime::from_millis(self.padding_ms),
        }
    }

    fn recompute(&mut self) {
        self.ranges = silent_ranges(&self.snapshot, &self.waveform, self.settings());
    }

    /// What would be cut, for the timeline to shade.
    pub fn ranges(&self) -> &[TimelineRange] {
        &self.ranges
    }

    /// How much would come out, in total.
    pub fn total(&self) -> TimelineTime {
        TimelineTime::from_ticks(self.ranges.iter().map(|r| r.duration().ticks()).sum())
    }
}

/// Draw the window, if one is open, and act on its buttons.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(mut dialog) = state.silence.take() else {
        return;
    };
    let mut open = true;
    let mut apply = false;
    // Set by Cancel, separately from the window's own close button, which
    // holds `open` for the whole of the body below.
    let mut cancelled = false;

    egui::Window::new("Remove silences")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .show(ctx, |ui| {
            ui.set_min_width(320.0);
            let seconds = dialog.total().ticks() as f64 / 960_000.0;
            ui.label(
                egui::RichText::new(match dialog.ranges.len() {
                    0 => "No silences found in this clip.".to_owned(),
                    1 => format!("1 silence, {seconds:.1} s in all."),
                    n => format!("{n} silences, {seconds:.1} s in all."),
                })
                .strong(),
            );
            ui.label(
                egui::RichText::new("Shaded on the timeline. Nothing is cut until you say so.")
                    .small()
                    .color(theme::disabled()),
            );
            ui.add_space(8.0);

            let mut changed = false;
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.threshold_db, -60.0..=-12.0)
                        .suffix(" dB")
                        .text("quieter than"),
                )
                .on_hover_text("Anything below this counts as silence")
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.shortest_ms, 100..=3_000)
                        .suffix(" ms")
                        .text("lasting at least"),
                )
                .on_hover_text("Shorter pauses are left alone — they are part of speech")
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut dialog.padding_ms, 0..=500)
                        .suffix(" ms")
                        .text("keep either side"),
                )
                .on_hover_text("Breathing room, so words are not clipped")
                .changed();
            if changed {
                dialog.recompute();
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = match dialog.ranges.len() {
                    0 => "Nothing to remove".to_owned(),
                    n => format!("Remove {n}"),
                };
                if ui
                    .add_enabled(!dialog.ranges.is_empty(), egui::Button::new(label))
                    .clicked()
                {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    if apply {
        let ranges = dialog.ranges.clone();
        match editor.remove_ranges(dialog.clip, &ranges) {
            Ok(0) => state.info("Nothing was removed"),
            Ok(n) => state.info(format!("Removed {n} silence(s)")),
            Err(err) => state.error(err.to_string()),
        }
        state.clear_selection();
        open = false;
    }
    state.needs_repaint = true;
    if open && !cancelled {
        state.silence = Some(dialog);
    }
}
