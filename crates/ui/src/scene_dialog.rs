//! The Find Cuts window (Milestone 12's "Scene detection").
//!
//! Same shape as [`crate::silence_dialog`]: analyse, *show* what was found, and
//! only cut when the user says so. The difference is that this one has to wait
//! — detection decodes the footage on a worker thread — so the window opens
//! first and fills in when the answer arrives. Opening it immediately is the
//! point: a menu item that does nothing visible for four seconds looks broken.
//!
//! The cuts are drawn on the timeline while it is open, because "9 cuts" tells
//! the user nothing about whether they are the right nine.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, TimelineTime};
use bettercut_jobs::JobId;
use bettercut_playback::{
    SceneReport,
    scene_job::{timeline_black, timeline_cuts},
};

use crate::state::UiState;
use crate::theme;

#[derive(Debug)]
pub struct SceneDialog {
    /// The clip being examined. The cuts are made in it, and in the sound
    /// linked to it (§12).
    clip: ClipId,
    /// The running job, so Cancel can stop the decode rather than only closing
    /// the window (§48).
    job: JobId,
    report: SceneReport,
    /// Where the cuts fall on the timeline, once the job has answered.
    cuts: Option<Vec<TimelineTime>>,
    /// Where the picture starts and stops once black at the ends is gone.
    black: (Option<TimelineTime>, Option<TimelineTime>),
}

impl SceneDialog {
    pub fn started(clip: ClipId, job: JobId, report: SceneReport) -> Self {
        Self {
            clip,
            job,
            report,
            cuts: None,
            black: (None, None),
        }
    }

    /// Pick up the answer, once. Cheap enough to call every frame.
    ///
    /// The clip is read again here rather than snapshotted when the window
    /// opened, because the job takes seconds and the user can trim in the
    /// meantime — offering a split outside the clip's current bounds would be
    /// refused by the editor anyway.
    pub fn poll(&mut self, editor: &Editor) {
        if self.cuts.is_some() {
            return;
        }
        let Some(found) = self.report.cuts() else {
            return;
        };
        let cuts = match editor.video_clip(self.clip) {
            Some(clip) => {
                self.black = timeline_black(clip, self.report.black());
                timeline_cuts(clip, &found)
            }
            None => Vec::new(), // the clip went away while we were looking
        };
        self.cuts = Some(cuts);
    }

    /// The cuts to draw on the timeline, empty while the job is still running.
    pub fn cuts(&self) -> &[TimelineTime] {
        self.cuts.as_deref().unwrap_or_default()
    }

    /// How much black comes off the start and the end, in seconds.
    fn black_seconds(&self, editor: &Editor) -> (f64, f64) {
        let (Some(start), Some(end)) = (editor.clip_start(self.clip), editor.clip_end(self.clip))
        else {
            return (0.0, 0.0);
        };
        (
            self.black.0.map_or(0.0, |at| (at - start).as_seconds_f64()),
            self.black.1.map_or(0.0, |at| (end - at).as_seconds_f64()),
        )
    }

    pub fn job(&self) -> JobId {
        self.job
    }

    fn working(&self) -> bool {
        self.cuts.is_none()
    }
}

/// Queue a detection over everything `clip` plays, and give back the window
/// that waits for it.
///
/// Over the clip's *source range* rather than the whole file: a clip trimmed to
/// ten seconds of an hour-long recording should cost ten seconds of decoding,
/// not an hour of it.
pub fn start(
    editor: &Editor,
    jobs: &mut crate::MediaJobs,
    clip: ClipId,
    threads: u32,
) -> Result<SceneDialog, String> {
    let video = editor
        .video_clip(clip)
        .ok_or("That clip has no picture to look through")?;
    let (source, media) = (video.source, video.media_id);
    let asset = editor
        .project()
        .media_asset(media)
        .ok_or("That clip's media is missing")?;

    let (job, report) = bettercut_playback::SceneJob::new(
        asset,
        source.start,
        source.end,
        bettercut_playback::SceneSettings::default(),
        threads,
    )
    .ok_or("There is nothing in that clip to look through")?;

    let id = jobs.submit_scene(job);
    Ok(SceneDialog::started(clip, id, report))
}

/// Draw the window, if one is open, and act on its buttons.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    let Some(mut dialog) = state.scenes.take() else {
        return;
    };
    dialog.poll(editor);

    let mut open = true;
    let mut apply = false;
    let mut mark = false;
    let mut trim = false;
    let mut cancelled = false;
    let (head, tail) = dialog.black_seconds(editor);

    egui::Window::new("Find cuts")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            if dialog.working() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Looking through the footage…");
                });
                ui.label(
                    egui::RichText::new("This reads the whole clip, so it takes a moment.")
                        .small()
                        .color(theme::disabled()),
                );
            } else {
                let found = dialog.cuts().len();
                ui.label(
                    egui::RichText::new(match found {
                        0 => "No cuts found — this looks like one continuous shot.".to_owned(),
                        1 => "1 cut found.".to_owned(),
                        n => format!("{n} cuts found."),
                    })
                    .strong(),
                );
                ui.label(
                    egui::RichText::new(if found == 0 {
                        "Nothing to split."
                    } else {
                        "Marked on the timeline. Nothing is cut until you say so."
                    })
                    .small()
                    .color(theme::disabled()),
                );
                if head > 0.0 || tail > 0.0 {
                    ui.label(match (head > 0.0, tail > 0.0) {
                        (true, true) => {
                            format!("Black: {head:.1} s at the start, {tail:.1} s at the end.")
                        }
                        (true, false) => format!("Black: {head:.1} s at the start."),
                        _ => format!("Black: {tail:.1} s at the end."),
                    });
                }
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let count = dialog.cuts().len();
                if ui
                    .add_enabled(
                        count > 0,
                        egui::Button::new(format!("Split into {}", count + 1)),
                    )
                    .clicked()
                {
                    apply = true;
                }
                // Or just a mark at each cut: chapters, or cuts to make later.
                if ui
                    .add_enabled(count > 0, egui::Button::new("Mark Cuts"))
                    .on_hover_text("Put a marker at every cut found and leave the clip whole")
                    .clicked()
                {
                    mark = true;
                }
                // The lens cap, the camera waking up: off the ends, with
                // the sound, as one step.
                if (head > 0.0 || tail > 0.0)
                    && ui
                        .button("Trim Black")
                        .on_hover_text(
                            "Take the black off the start and end of the clip, and its sound",
                        )
                        .clicked()
                {
                    trim = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    if mark {
        let cuts: Vec<TimelineTime> = dialog.cuts().to_vec();
        match editor.add_markers(&cuts) {
            Ok(n) => state.info(format!("Marked {n} cuts")),
            Err(err) => state.error(err.to_string()),
        }
        // The dialog was taken out of the state above; not putting it back
        // is what closes it — and a decode still running is stopped.
        if dialog.working() {
            state.scene_cancel = Some(dialog.job());
        }
        state.needs_repaint = true;
        return;
    }

    if trim {
        let (from, to) = dialog.black;
        match editor.trim_ends(dialog.clip, from, to, "Trim Black") {
            Ok(true) => state.info(format!("Trimmed {:.1} s of black", head + tail)),
            Ok(false) => state.info("Nothing to trim"),
            Err(err) => state.error(err.to_string()),
        }
        state.needs_repaint = true;
        return;
    }

    if apply {
        let cuts: Vec<TimelineTime> = dialog.cuts().to_vec();
        match editor.split_clip_at(dialog.clip, &cuts) {
            Ok(0) => state.info("Nothing was split"),
            Ok(n) => state.info(format!("Split into {} clips", n + 1)),
            Err(err) => state.error(err.to_string()),
        }
        state.clear_selection();
        open = false;
    }

    // Closing while the decode is still running stops it: §48's rule about
    // cancellation being responsive is worth nothing if nothing calls it.
    if !open || cancelled {
        if dialog.working() {
            state.scene_cancel = Some(dialog.job());
        }
        state.needs_repaint = true;
        return;
    }

    // A window waiting on a worker has to keep asking to be drawn, or it sits
    // on its spinner until the user happens to move the mouse.
    state.needs_repaint = true;
    state.scenes = Some(dialog);
}
