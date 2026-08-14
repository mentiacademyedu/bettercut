//! The Export window (§41, §42, Milestone 6).
//!
//! Deliberately short. §41 asks for an interface that explains itself, and the
//! honest reading of that for export is *fewer decisions*, not more: the
//! defaults are the sequence's own format, and the only thing a user has to
//! supply is where to put the file.
//!
//! What is offered is the one choice with a real trade-off — output size — plus
//! a plain statement of what will be written and which encoder will do it.
//! Bitrate, profile, keyframe interval and colour tags are all decided from the
//! sequence, because a user who wanted to set those would not be using this
//! program.

use bettercut_editor_core::Editor;
use bettercut_editor_core::timeline::{Resolution, TimelineRange};
use bettercut_export::ExportSettings;

use crate::state::UiState;
use crate::theme;

/// Whether the window is open, and what it has been set to.
#[derive(Debug, Default)]
pub struct ExportDialog {
    pub open: bool,
    /// `None` until the user picks a file.
    path: Option<std::path::PathBuf>,
    /// Index into [`SCALES`].
    scale: usize,
    /// Set when the user has been told why they cannot export yet.
    complaint: Option<String>,
}

/// Output sizes, as fractions of the sequence resolution.
///
/// Fractions rather than a list of pixel sizes: a 9:16 sequence and a 16:9 one
/// want completely different numbers, and "half" means the same useful thing to
/// both. Halving is also exactly representable, so an even sequence stays even
/// and H.264's 4:2:0 has nothing to complain about.
const SCALES: [(&str, u32); 3] = [("Full", 1), ("Half", 2), ("Quarter", 4)];

impl ExportDialog {
    /// Open the window, suggesting a file name from the project.
    pub fn open(&mut self, editor: &Editor) {
        self.open = true;
        self.complaint = None;
        if self.path.is_none() {
            self.path = Some(suggested_path(editor));
        }
    }
}

/// Draw the window. Returns settings when the user pressed Export.
pub fn show(
    ctx: &egui::Context,
    editor: &Editor,
    state: &mut UiState,
    dialog: &mut ExportDialog,
    exporting: bool,
) -> Option<ExportSettings> {
    if !dialog.open {
        return None;
    }

    let Some(sequence) = editor.active_sequence() else {
        dialog.open = false;
        state.error("There is no sequence to export");
        return None;
    };

    let full = sequence.resolution;
    let rate = sequence.frame_rate;
    let duration = sequence.duration();
    let mut start = None;

    let mut open = dialog.open;
    egui::Window::new("Export")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_width(360.0);

            ui.horizontal(|ui| {
                ui.label("Save to");
                let shown = dialog.path.as_ref().map_or_else(
                    || "(choose a file)".to_owned(),
                    |p| {
                        p.file_name().map_or_else(
                            || p.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        )
                    },
                );
                if ui
                    .button(shown)
                    .on_hover_text(
                        dialog
                            .path
                            .as_ref()
                            .map_or_else(String::new, |p| p.display().to_string()),
                    )
                    .clicked()
                    && let Some(chosen) = rfd::FileDialog::new()
                        .add_filter("MP4 video", &["mp4"])
                        .set_file_name(
                            dialog
                                .path
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map_or_else(
                                    || "video.mp4".to_owned(),
                                    |n| n.to_string_lossy().into_owned(),
                                ),
                        )
                        .save_file()
                {
                    dialog.path = Some(chosen);
                    dialog.complaint = None;
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Size");
                for (index, (label, divisor)) in SCALES.iter().enumerate() {
                    let size = scaled(full, *divisor);
                    if ui
                        .selectable_label(dialog.scale == index, *label)
                        .on_hover_text(format!("{}×{}", size.width, size.height))
                        .clicked()
                    {
                        dialog.scale = index;
                    }
                }
            });

            let size = scaled(full, SCALES[dialog.scale].1);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}×{} · {rate} fps · {} · H.264 with AAC audio",
                    size.width,
                    size.height,
                    duration.format_timecode()
                ))
                .small()
                .color(theme::DISABLED),
            );
            // §14, and worth saying out loud: people expect an editor that has
            // been showing them a proxy to export the proxy.
            ui.label(
                egui::RichText::new(
                    "Export always reads your original files, never the proxies \
                     used for editing.",
                )
                .small()
                .color(theme::DISABLED),
            );

            if let Some(complaint) = &dialog.complaint {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(complaint).color(theme::ERROR_TEXT));
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = dialog.path.is_some() && !exporting;
                let button = egui::Button::new(if exporting { "Exporting…" } else { "Export" });
                if ui.add_enabled(ready, button).clicked() {
                    start = Some(());
                }
                if ui.button("Cancel").clicked() {
                    dialog.open = false;
                }
                if exporting {
                    ui.label(
                        egui::RichText::new("An export is already running.")
                            .small()
                            .color(theme::DISABLED),
                    );
                }
            });
        });
    dialog.open &= open;

    start?;
    let path = dialog.path.clone()?;
    let size = scaled(full, SCALES[dialog.scale].1);

    // An empty sequence has nothing to write, and the export crate would refuse
    // it — but refusing here is the difference between a message beside the
    // button and a failure in the status bar a minute later.
    let Ok(range) = TimelineRange::new(
        bettercut_editor_core::foundation::TimelineTime::ZERO,
        duration,
    ) else {
        dialog.complaint = Some("This sequence is empty — add a clip first.".to_owned());
        return None;
    };

    dialog.open = false;
    Some(ExportSettings {
        path,
        resolution: size,
        range,
        // §15.1: FFmpeg never gets every core. The renderer and the decoder are
        // both working during an export, and leaving the machine responsive
        // matters more than finishing a minute sooner.
        threads: 2,
    })
}

/// Halve or quarter a resolution, keeping both dimensions even.
///
/// H.264 4:2:0 stores chroma at half resolution in each direction, so an odd
/// dimension has no representation and the writer refuses it. Rounding down to
/// even here means the choice is always offered rather than sometimes failing.
fn scaled(full: Resolution, divisor: u32) -> Resolution {
    let even = |value: u32| (value / divisor).max(2) & !1;
    Resolution::new(even(full.width), even(full.height))
}

/// A file beside the project, named after it.
fn suggested_path(editor: &Editor) -> std::path::PathBuf {
    let name = {
        let raw = editor.project().name.trim();
        if raw.is_empty() { "video" } else { raw }
    };
    // Strip what a file name cannot hold, rather than handing the OS something
    // it will reject: a project called "Trip 6/7" is a perfectly good name.
    let safe: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect();

    let directory = editor
        .path()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .or_else(dirs_videos)
        .unwrap_or_else(std::env::temp_dir);

    directory.join(format!("{safe}.mp4"))
}

/// The user's Videos folder, when there is an obvious one.
fn dirs_videos() -> Option<std::path::PathBuf> {
    // No extra dependency for one path: the home directory plus the
    // conventional name covers Windows and macOS, and is a reasonable guess
    // elsewhere. When it does not exist, the caller falls back again.
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let videos = std::path::PathBuf::from(home).join("Videos");
    videos.is_dir().then_some(videos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_both_dimensions_even() {
        // 1080 / 4 is 270, which is even; 1079 / 2 is 539, which is not.
        for (width, height) in [(1920, 1080), (1079, 721), (640, 361), (3840, 2160)] {
            let full = Resolution::new(width, height);
            for (_, divisor) in SCALES {
                let size = scaled(full, divisor);
                assert!(
                    size.width.is_multiple_of(2) && size.height.is_multiple_of(2),
                    "{width}x{height} / {divisor} produced {}x{}",
                    size.width,
                    size.height
                );
                assert!(size.width >= 2 && size.height >= 2);
            }
        }
    }

    /// Full size must be exactly the sequence, not the sequence rounded.
    #[test]
    fn full_size_is_the_sequence_itself() {
        let full = Resolution::new(1920, 1080);
        assert_eq!(scaled(full, 1), full);
    }

    /// A tiny sequence must still produce something encodable rather than a
    /// zero-pixel frame.
    #[test]
    fn a_tiny_sequence_does_not_scale_to_nothing() {
        let size = scaled(Resolution::new(4, 4), 4);
        assert_eq!((size.width, size.height), (2, 2));
    }
}
