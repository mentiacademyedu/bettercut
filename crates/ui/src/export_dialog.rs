//! The Export window (§41, §42, Milestone 6).
//!
//! A labelled form: name, destination, then the video settings, then a footer
//! saying what will come out and how big it will be. Defaults are the
//! sequence's own format, so pressing Export without touching anything gives
//! back what you were editing.
//!
//! ## What is and is not offered
//!
//! **H.265 is only offered when the machine can write it.** Its absence is not
//! a gap to fill later: the software H.265 encoder everyone reaches for is
//! x265, and §0.1 point 2 with §74 forbid linking anything GPL outright. So the
//! option is probed by actually opening an encoder, and disabled with the
//! reason when there is none. Offering a choice that fails after the user has
//! waited for an export would be worse than not offering it.
//!
//! **Frame rate is restricted to §9's nine exact rates.** The timebase divides
//! all of them exactly; a free-form field would let someone type 29.97 and get
//! a file that drifts a second an hour against its own audio.
//!
//! **Colour space is stated, not chosen.** §21a.1 fixes the working space at
//! sRGB-encoded 8-bit, and every export is tagged BT.709 limited-range to
//! match. Showing it as a fact rather than a dropdown is honest: there is no
//! second option behind it yet, and a disabled dropdown would imply there is.

use bettercut_editor_core::foundation::{FrameRate, TimelineTime};
use bettercut_editor_core::timeline::{Resolution, TimelineRange};
use bettercut_editor_core::{Editor, SHAPES, Shape};
use bettercut_export::{ExportSettings, RateControl, VideoCodec, codec_is_available};

use crate::state::UiState;
use crate::theme;

/// Width of the label column, so every control lines up.
const LABEL_WIDTH: f32 = 96.0;
/// Width of the controls.
const FIELD_WIDTH: f32 = 240.0;

/// Standard output heights, named the way everyone names them.
///
/// The number is the **short** edge, which is what "1080p" has always meant:
/// 1080 lines. For a landscape 16:9 sequence that is 1920×1080; for a vertical
/// 9:16 one it is 1080×1920. Naming the long edge instead turns a 4K sequence
/// into 1080×608, which is what this did before it was fixed.
const HEIGHTS: [(&str, u32); 5] = [
    ("2160p (4K)", 2160),
    ("1440p", 1440),
    ("1080p", 1080),
    ("720p", 720),
    ("480p", 480),
];

/// How the bitrate is decided.
///
/// The named tiers are multiples of the recommended figure rather than fixed
/// numbers, because the right bitrate depends on the size, the rate and the
/// codec — "8 Mb/s" is generous for 720p30 and thin for 4K60. Scaling the
/// recommendation keeps every tier meaningful at every setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BitrateChoice {
    Lower,
    Medium,
    #[default]
    Recommended,
    High,
    Custom,
}

impl BitrateChoice {
    /// Smallest to largest, so the list reads as a scale. `Recommended` sits in
    /// its natural place rather than at the top: it is a point on the same
    /// scale, not a separate kind of thing.
    const ALL: [Self; 5] = [
        Self::Lower,
        Self::Medium,
        Self::Recommended,
        Self::High,
        Self::Custom,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Lower => "Lower",
            Self::Medium => "Medium",
            Self::Recommended => "Recommended",
            Self::High => "High",
            Self::Custom => "Custom",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Lower => {
                "Half the recommended rate. Smallest files; soft on fine detail and motion."
            }
            Self::Medium => {
                "Three quarters. Noticeably smaller, and hard to tell apart on most footage."
            }
            Self::Recommended => {
                "Chosen from the size, frame rate and format. Use this unless you have a reason not to."
            }
            Self::High => {
                "Half again as much. For footage with heavy grain, fast motion, or another edit to come."
            }
            Self::Custom => "Type an exact rate, and choose how strictly it is held.",
        }
    }

    /// What this works out to, given the recommended figure.
    fn kbps(self, recommended: u32) -> u32 {
        let scaled = match self {
            Self::Lower => f64::from(recommended) * 0.5,
            Self::Medium => f64::from(recommended) * 0.75,
            Self::Recommended | Self::Custom => f64::from(recommended),
            Self::High => f64::from(recommended) * 1.5,
        };
        // The same floor and ceiling the encoder applies, so what is shown is
        // what gets used.
        (scaled as u32).clamp(1_000, 120_000)
    }
}

/// Containers offered. Both hold H.264 and H.265.
/// The formats offered. The last is sound only: the mix as a WAV, with no
/// picture rendered — for a voice-over to master, or the podcast cut.
const CONTAINERS: [(&str, &str); 4] = [
    ("mp4", "Plays everywhere"),
    ("mov", "QuickTime; editors"),
    ("wav", "Sound only — the mix, uncompressed"),
    (
        "gif",
        "A looping animation with no sound — for chats and web pages",
    ),
];

/// The index in [`CONTAINERS`] that means sound only.
const SOUND_ONLY: usize = 2;

/// The index in [`CONTAINERS`] that means an animated GIF.
const GIF: usize = 3;

/// Widths offered for a GIF. Small, because a GIF keeps every pixel of every
/// frame: past 640 the files stop being the quick thing a GIF is for.
const GIF_WIDTHS: [u32; 3] = [320, 480, 640];
const DEFAULT_GIF_WIDTH: u32 = 480;

/// Frame rates offered for a GIF, each dividing the timebase exactly (§9).
const GIF_RATES: [i64; 3] = [10, 15, 20];
const DEFAULT_GIF_RATE: i64 = 15;

/// 16-bit stereo at 48 kHz, in kilobits a second, for the size estimate.
const WAV_KBPS: f64 = 48_000.0 * 16.0 * 2.0 / 1_000.0;

/// A platform's upload settings, applied in one choice.
///
/// What each site actually re-encodes to, and the shape its players show,
/// rather than a guess at quality: the file is going to be compressed again on
/// arrival, so it is sent generous (`High`) to survive that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    pub label: &'static str,
    pub hint: &'static str,
    pub ratio: (u32, u32),
    /// The short edge, as the size presets name it.
    pub short_edge: u32,
    /// A fixed frame rate, or `None` to keep the sequence's.
    pub fps: Option<FrameRate>,
}

pub const PLATFORMS: [Platform; 5] = [
    Platform {
        label: "YouTube 1080p",
        hint: "Landscape at 1920×1080, the sequence's own frame rate",
        ratio: (16, 9),
        short_edge: 1080,
        fps: None,
    },
    Platform {
        label: "YouTube 4K",
        hint: "Landscape at 3840×2160, the sequence's own frame rate",
        ratio: (16, 9),
        short_edge: 2160,
        fps: None,
    },
    Platform {
        label: "TikTok · Reels · Shorts",
        hint: "Vertical at 1080×1920, 30 fps",
        ratio: (9, 16),
        short_edge: 1080,
        fps: Some(FrameRate::FPS_30),
    },
    Platform {
        label: "Instagram feed",
        hint: "Portrait at 1080×1350, 30 fps — the tallest the feed shows",
        ratio: (4, 5),
        short_edge: 1080,
        fps: Some(FrameRate::FPS_30),
    },
    Platform {
        label: "Square post",
        hint: "1080×1080, 30 fps",
        ratio: (1, 1),
        short_edge: 1080,
        fps: Some(FrameRate::FPS_30),
    },
];

/// One file the user asked for: the sequence as it is, or reshaped.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub settings: ExportSettings,
    /// `None` for the sequence's own shape. Otherwise the export reads
    /// [`Editor::export_copy`] instead of the project as it stands.
    pub shape: Option<Shape>,
}

/// Whether the window is open, and what it has been set to.
#[derive(Debug, Default)]
pub struct ExportDialog {
    pub open: bool,
    /// The file name without its extension. Separate from the folder so the
    /// common edit — renaming the output — does not mean reopening a file
    /// picker.
    name: String,
    /// Where it goes. `None` until defaulted.
    folder: Option<std::path::PathBuf>,
    container: usize,
    codec: VideoCodec,
    /// Index into [`HEIGHTS`], or `None` for a custom size.
    height_preset: Option<usize>,
    /// Only read when `height_preset` is `None`. `Resolution` has no
    /// `Default` on purpose — there is no neutral frame size — so this holds
    /// its own placeholder until the window opens and fills it from the
    /// sequence.
    custom: Option<Resolution>,
    frame_rate: Option<FrameRate>,
    bitrate: BitrateChoice,
    /// The typed rate, kept across a switch away from Custom and back.
    ///
    /// Kilobits rather than megabits because that is the unit every delivery
    /// spec is written in — "8000 kbps" is what a platform's upload guidance
    /// says, and making the user divide it by a thousand is a small tax on the
    /// one field most likely to be copied from somewhere else.
    custom_kbps: Option<u32>,
    rate_control: RateControl,
    /// Other shapes to write alongside the main file, as width:height ratios.
    ///
    /// Kept across openings, unlike the format: someone who posts every video
    /// as a vertical cut too wants that ticked next time. A ratio the sequence
    /// has since become is skipped rather than exported twice.
    also: Vec<(u32, u32)>,
    /// The main file's shape when it is not the sequence's — set by choosing
    /// it, or by a platform preset that wants a different one. Exported from a
    /// reframed copy (`Editor::export_copy`), so the edit itself is untouched.
    main_shape: Option<Shape>,
    /// Export only the span between the in and out marks, when there is one.
    /// Set on opening whenever marks exist, since marking a range is usually
    /// done in order to export it.
    only_marked: bool,
    /// Which codecs this machine can write, probed when the window opens.
    available: Vec<(VideoCodec, bool)>,
    complaint: Option<String>,
    /// A GIF's width in pixels; 0 until chosen, meaning the default.
    gif_width: u32,
    /// A GIF's frames a second; 0 until chosen, meaning the default.
    gif_rate: i64,
}

impl ExportDialog {
    /// Open the window, defaulting everything to the sequence.
    pub fn open(&mut self, editor: &Editor) {
        self.open = true;
        self.complaint = None;

        if self.name.is_empty() {
            self.name = safe_file_name(&editor.project().name);
        }
        if self.folder.is_none() {
            self.folder = Some(default_folder(editor));
        }

        let Some(sequence) = editor.active_sequence() else {
            return;
        };
        // Reset the format to the sequence every time. Carrying last time's
        // numbers over would quietly export a different project at the wrong
        // size; the name and folder are what is worth remembering.
        self.custom = Some(sequence.resolution);
        self.height_preset = HEIGHTS
            .iter()
            .position(|(_, height)| *height == short_edge(sequence.resolution));
        self.frame_rate = Some(sequence.frame_rate);
        self.bitrate = BitrateChoice::default();
        self.custom_kbps = None;
        self.main_shape = None;
        self.only_marked = sequence.marked_range().is_some();

        // Probed once, here, rather than per frame: each check opens a real
        // encoder. `codec_is_available` caches, so reopening is free.
        self.available = VideoCodec::ALL
            .into_iter()
            .map(|codec| {
                (
                    codec,
                    codec_is_available(codec, sequence.resolution, sequence.frame_rate),
                )
            })
            .collect();

        if !self.can_write(self.codec) {
            self.codec = VideoCodec::H264;
        }
    }

    fn can_write(&self, codec: VideoCodec) -> bool {
        self.available
            .iter()
            .find(|(candidate, _)| *candidate == codec)
            .is_none_or(|(_, usable)| *usable)
    }

    /// The output size: a preset applied to the sequence's shape, or the
    /// custom numbers.
    fn resolution(&self, native: Resolution) -> Resolution {
        self.resolution_for(native, self.main_shape_for(native))
    }

    /// The size at the sequence's own shape.
    fn sequence_resolution(&self, native: Resolution) -> Resolution {
        match self.height_preset {
            Some(index) => fit_short_edge(native, HEIGHTS[index].1),
            // Falls back to the sequence: `custom` is only unset before the
            // window has ever opened, and the sequence is the honest answer.
            None => even(self.custom.unwrap_or(native)),
        }
    }

    /// The output size of an extra shape: the same size preset applied to
    /// that shape, or the custom size reshaped keeping its short edge. `None`
    /// is the sequence's own shape.
    fn resolution_for(&self, native: Resolution, shape: Option<Shape>) -> Resolution {
        let Some(shape) = shape else {
            return self.sequence_resolution(native);
        };
        match self.height_preset {
            Some(index) => fit_short_edge(shape.applied_to(native), HEIGHTS[index].1),
            None => shape.applied_to(even(self.custom.unwrap_or(native))),
        }
    }

    /// The bitrate that will actually be used, in kilobits.
    ///
    /// One place, so the dropdown, the estimate in the footer and the number
    /// handed to the encoder cannot disagree.
    fn effective_kbps(&self, native: Resolution) -> u32 {
        self.kbps_at(self.resolution(native))
    }

    /// The same choice at another output size. A tier is a multiple of what
    /// that size needs, so a square cut gets fewer bits than the landscape
    /// one it sits beside rather than the same number spread thinner.
    fn kbps_at(&self, size: Resolution) -> u32 {
        let recommended = automatic_kbps_at(self, size);
        match self.bitrate {
            BitrateChoice::Custom => self.custom_kbps.unwrap_or(recommended),
            other => other.kbps(recommended),
        }
    }

    /// The rate control that will actually be used.
    ///
    /// Every choice but `Recommended` shows the radios and carries whatever was
    /// picked. `Recommended` does not offer them — the point of it is not
    /// having to decide — so it must not silently inherit a `Constant` left
    /// over from a tier the user moved away from, which would pad the file for
    /// a reason they never asked for and could no longer see.
    fn rate_control_for_export(&self) -> RateControl {
        if self.bitrate == BitrateChoice::Recommended {
            RateControl::Variable
        } else {
            self.rate_control
        }
    }

    fn path(&self) -> Option<std::path::PathBuf> {
        self.path_for(None)
    }

    /// Where a shape's file goes: beside the main one, its name marked with
    /// the shape so the three files of one edit sort together and say which
    /// is which.
    fn path_for(&self, shape: Option<Shape>) -> Option<std::path::PathBuf> {
        let name = if self.name.trim().is_empty() {
            "video"
        } else {
            self.name.trim()
        };
        let name = match shape {
            Some(shape) => format!("{name}-{}", shape.file_suffix),
            None => name.to_owned(),
        };
        Some(
            self.folder
                .as_ref()?
                .join(format!("{name}.{}", CONTAINERS[self.container].0)),
        )
    }

    /// The extra shapes that will actually be written: ticked, and not the
    /// shape the sequence already is.
    fn extra_shapes(&self, native: Resolution) -> Vec<Shape> {
        // A shape is a picture's; sound has none, and three identical WAVs
        // under three names would be a surprise. A GIF is one small loop of
        // the edit as it is.
        if self.sound_only() || self.gif() {
            return Vec::new();
        }
        let main = self.resolution(native);
        SHAPES
            .into_iter()
            .filter(|shape| self.also.contains(&shape.ratio) && !shape.matches(main))
            .collect()
    }

    /// The main file's shape, when it differs from the sequence's.
    fn main_shape_for(&self, native: Resolution) -> Option<Shape> {
        if self.sound_only() || self.gif() {
            return None;
        }
        self.main_shape.filter(|shape| !shape.matches(native))
    }

    /// Set everything a platform wants.
    fn apply_platform(&mut self, platform: Platform, native: Resolution, native_rate: FrameRate) {
        self.height_preset = HEIGHTS
            .iter()
            .position(|(_, height)| *height == platform.short_edge);
        self.frame_rate = Some(platform.fps.unwrap_or(native_rate));
        self.codec = VideoCodec::H264;
        self.container = 0;
        self.bitrate = BitrateChoice::High;
        self.main_shape = SHAPES
            .into_iter()
            .find(|shape| shape.ratio == platform.ratio)
            .filter(|shape| !shape.matches(native));
        // Its own shape is now the main file, not an extra copy of it.
        self.also.retain(|ratio| *ratio != platform.ratio);
    }

    /// The platform the settings currently match, if any — so the menu says
    /// "TikTok" while they are what TikTok wants, and "Custom" once changed.
    fn platform_in_use(&self, native: Resolution, native_rate: FrameRate) -> Option<Platform> {
        PLATFORMS.into_iter().find(|platform| {
            let mut applied = ExportDialog {
                also: self.also.clone(),
                ..ExportDialog::default()
            };
            applied.apply_platform(*platform, native, native_rate);
            applied.height_preset == self.height_preset
                && applied.frame_rate.unwrap_or(native_rate)
                    == self.frame_rate.unwrap_or(native_rate)
                && applied.codec == self.codec
                && applied.container == self.container
                && applied.bitrate == self.bitrate
                && applied.main_shape_for(native) == self.main_shape_for(native)
        })
    }

    /// Whether the chosen format is sound only.
    fn sound_only(&self) -> bool {
        self.container == SOUND_ONLY
    }

    /// Whether the chosen format is an animated GIF.
    fn gif(&self) -> bool {
        self.container == GIF
    }

    fn gif_width(&self) -> u32 {
        if self.gif_width == 0 {
            DEFAULT_GIF_WIDTH
        } else {
            self.gif_width
        }
    }

    fn gif_rate(&self) -> FrameRate {
        let rate = if self.gif_rate == 0 {
            DEFAULT_GIF_RATE
        } else {
            self.gif_rate
        };
        FrameRate::new(rate, 1).unwrap_or(FrameRate::FPS_30)
    }

    /// Every file this press of Export asks for, main file first.
    fn requests(
        &self,
        native: Resolution,
        native_rate: FrameRate,
        range: TimelineRange,
    ) -> Vec<ExportRequest> {
        std::iter::once((self.main_shape_for(native), true))
            .chain(
                self.extra_shapes(native)
                    .into_iter()
                    .map(|shape| (Some(shape), false)),
            )
            .filter_map(|(shape, main)| {
                let size = if self.gif() {
                    gif_size(native, self.gif_width())
                } else {
                    self.resolution_for(native, shape)
                };
                Some(ExportRequest {
                    settings: ExportSettings {
                        path: self.path_for(if main { None } else { shape })?,
                        resolution: size,
                        frame_rate: if self.gif() {
                            self.gif_rate()
                        } else {
                            self.frame_rate.unwrap_or(native_rate)
                        },
                        codec: self.codec,
                        bitrate: Some(i64::from(self.kbps_at(size)) * 1_000),
                        // Recommended has no rate-control control, so it must
                        // not carry one a previous selection left behind (see
                        // `bitrate_rows`).
                        rate_control: self.rate_control_for_export(),
                        range,
                        // §15.1: FFmpeg never gets every core. The renderer and
                        // the decoder are both working during an export, and
                        // leaving the machine responsive matters more than
                        // finishing a minute sooner.
                        threads: 2,
                        sound_only: self.sound_only(),
                        gif: self.gif(),
                    },
                    shape,
                })
            })
            .collect()
    }
}

/// Draw the window. Returns settings when the user pressed Export.
/// Which lanes have something soloed, named the way the user would say it.
///
/// `None` when nothing is, which is almost always — this exists to catch the
/// one time it is not.
fn soloed_lanes(sequence: &bettercut_editor_core::timeline::Sequence) -> Option<String> {
    let picture =
        sequence.video_tracks.iter().any(|t| t.solo) || sequence.text_tracks.iter().any(|t| t.solo);
    let sound = sequence.audio_tracks.iter().any(|t| t.solo);

    match (picture, sound) {
        (true, true) => Some("A picture track and a sound track are".to_owned()),
        (true, false) => Some("A picture track is".to_owned()),
        (false, true) => Some("A sound track is".to_owned()),
        (false, false) => None,
    }
}

pub fn show(
    ctx: &egui::Context,
    editor: &Editor,
    state: &mut UiState,
    dialog: &mut ExportDialog,
    exporting: bool,
) -> Vec<ExportRequest> {
    if !dialog.open {
        return Vec::new();
    }

    let Some(sequence) = editor.active_sequence() else {
        dialog.open = false;
        state.error("There is no sequence to export");
        return Vec::new();
    };

    let native = sequence.resolution;
    let native_rate = sequence.frame_rate;
    let duration = sequence.duration();
    let sequence_name = sequence.name.clone();
    // §20a.4: solo silences the rest of its lane, and the export reads the
    // same rule the preview does — so a solo left on from checking one voice
    // exports only that voice. Said here, where it is about to matter, rather
    // than left for the user to discover in the file.
    let soloed = soloed_lanes(sequence);
    let marked = sequence.marked_range();
    let has_sound = sequence.audio_tracks.iter().any(|track| track.enabled);
    let mut start = None;

    let mut open = dialog.open;
    egui::Window::new("Export")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_width(LABEL_WIDTH + FIELD_WIDTH + 60.0);
            ui.add_space(2.0);

            row(ui, "Export timeline", |ui| {
                ui.label(egui::RichText::new(&sequence_name).color(theme::DISABLED));
            });
            row(ui, "Name", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.name)
                        .desired_width(FIELD_WIDTH)
                        .hint_text("video"),
                );
            });
            destination_row(ui, dialog);
            container_row(ui, dialog);
            range_row(ui, dialog, marked, duration);

            ui.add_space(6.0);
            ui.separator();
            if dialog.sound_only() {
                // The video settings would all be ignored, so they are not
                // shown: a form of controls that do nothing is worse than a
                // shorter form.
                ui.label(egui::RichText::new("Sound").strong());
                ui.add_space(2.0);
                row(ui, "Written as", |ui| {
                    ui.label(
                        egui::RichText::new("WAV · 48 kHz · 16-bit · stereo")
                            .color(theme::DISABLED),
                    );
                });
            } else if dialog.gif() {
                // A GIF has a size and a rate, and nothing else to choose.
                ui.label(egui::RichText::new("Animation").strong());
                ui.add_space(2.0);
                gif_rows(ui, dialog, native);
            } else {
                ui.label(egui::RichText::new("Video").strong());
                ui.add_space(2.0);

                platform_row(ui, dialog, native, native_rate);
                shape_row(ui, dialog, native);
                resolution_row(ui, dialog, native);
                bitrate_rows(ui, dialog, native);
                codec_row(ui, dialog);
                frame_rate_row(ui, dialog, native_rate);
                also_row(ui, dialog, native);

                row(ui, "Colour space", |ui| {
                    ui.label(egui::RichText::new("Rec. 709 SDR").color(theme::DISABLED))
                        .on_hover_text(
                            "§21a fixes the working space and tags every export to \
                         match, so players do not have to guess.",
                        );
                });
            }

            if let Some(what) = &soloed {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{what} soloed — the export will contain only that, as the preview does"
                    ))
                    .color(theme::ERROR_TEXT),
                );
            }

            if let Some(complaint) = &dialog.complaint {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(complaint).color(theme::ERROR_TEXT));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                // The length that will be written, so the size estimate is too.
                let length = export_range(dialog.only_marked, marked, duration)
                    .map_or(duration, |range| range.duration());
                summary(ui, dialog, native, length);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        dialog.open = false;
                    }
                    let ready = dialog.folder.is_some() && !exporting;
                    let label = if exporting { "Exporting…" } else { "Export" };
                    if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                        start = Some(());
                    }
                });
            });

            if exporting {
                ui.label(
                    egui::RichText::new("An export is already running.")
                        .small()
                        .color(theme::DISABLED),
                );
            }
        });
    dialog.open &= open;

    if start.is_none() {
        return Vec::new();
    }

    // An empty sequence has nothing to write. Refusing here is the difference
    // between a message beside the button and a failure a minute later.
    let Some(range) = export_range(dialog.only_marked, marked, duration) else {
        dialog.complaint = Some("This sequence is empty — add a clip first.".to_owned());
        return Vec::new();
    };

    // Said beside the button rather than after a job fails: a WAV of nothing
    // is a file that looks broken.
    if dialog.sound_only() && !has_sound {
        dialog.complaint =
            Some("There is no sound to export — every sound track is off.".to_owned());
        return Vec::new();
    }

    let requests = dialog.requests(native, native_rate, range);
    if !requests.is_empty() {
        dialog.open = false;
    }
    requests
}

/// What an export writes: the marked span when asked for and there is one,
/// otherwise the whole sequence. `None` for an empty sequence with no marks.
fn export_range(
    only_marked: bool,
    marked: Option<TimelineRange>,
    duration: TimelineTime,
) -> Option<TimelineRange> {
    match marked {
        Some(range) if only_marked => Some(range),
        _ => TimelineRange::new(TimelineTime::ZERO, duration).ok(),
    }
}

/// Whole sequence, or just the marked span — offered only when there is one.
fn range_row(
    ui: &mut egui::Ui,
    dialog: &mut ExportDialog,
    marked: Option<TimelineRange>,
    duration: TimelineTime,
) {
    row(ui, "Range", |ui| {
        let Some(range) = marked else {
            ui.label(
                egui::RichText::new(format!("Whole sequence · {}", duration.format_timecode()))
                    .color(theme::DISABLED),
            )
            .on_hover_text("Mark in and out with I and O to export part of it");
            return;
        };
        ui.radio_value(&mut dialog.only_marked, false, "Whole sequence");
        ui.radio_value(
            &mut dialog.only_marked,
            true,
            format!("In to out · {}", range.duration().format_timecode()),
        )
        .on_hover_text(format!(
            "{} to {}",
            range.start.format_timecode(),
            range.end.format_timecode()
        ));
    });
}

/// A platform's settings in one choice, and which one the settings match now.
fn platform_row(
    ui: &mut egui::Ui,
    dialog: &mut ExportDialog,
    native: Resolution,
    native_rate: FrameRate,
) {
    row(ui, "Made for", |ui| {
        let current = dialog.platform_in_use(native, native_rate);
        let mut chosen = None;
        egui::ComboBox::from_id_salt("export_platform")
            .selected_text(current.map_or("Custom settings", |p| p.label))
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for platform in PLATFORMS {
                    if ui
                        .selectable_label(current == Some(platform), platform.label)
                        .on_hover_text(platform.hint)
                        .clicked()
                    {
                        chosen = Some(platform);
                    }
                }
            });
        if let Some(platform) = chosen {
            dialog.apply_platform(platform, native, native_rate);
        }
    });
}

/// The main file's shape: the sequence's, or another cropped from it.
fn shape_row(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: Resolution) {
    row(ui, "Shape", |ui| {
        let main = dialog.main_shape_for(native);
        let sequence_label = SHAPES
            .into_iter()
            .find(|shape| shape.matches(native))
            .map_or_else(
                || "As the sequence".to_owned(),
                |shape| format!("{} (as the sequence)", shape.label),
            );
        egui::ComboBox::from_id_salt("export_main_shape")
            .selected_text(main.map_or(sequence_label.clone(), |shape| {
                format!("{} — cropped from the edit", shape.label)
            }))
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(main.is_none(), sequence_label.as_str())
                    .clicked()
                {
                    dialog.main_shape = None;
                }
                for shape in SHAPES.into_iter().filter(|shape| !shape.matches(native)) {
                    if ui
                        .selectable_label(main == Some(shape), shape.label)
                        .on_hover_text(format!(
                            "{} — shots are cropped to fill it; your edit is not changed",
                            shape.hint
                        ))
                        .clicked()
                    {
                        dialog.main_shape = Some(shape);
                        dialog.also.retain(|ratio| *ratio != shape.ratio);
                    }
                }
            });
    });
}

/// Other shapes to write in the same go: the edit as it is, plus a vertical
/// or square cut, without reshaping the sequence and exporting again by hand.
fn also_row(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: Resolution) {
    row(ui, "Also export", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(FIELD_WIDTH + 40.0);
            for shape in SHAPES {
                // The main file's shape is the main file already.
                if shape.matches(dialog.resolution(native)) {
                    continue;
                }
                let mut ticked = dialog.also.contains(&shape.ratio);
                let size = dialog.resolution_for(native, Some(shape));
                if ui
                    .checkbox(&mut ticked, shape.label)
                    .on_hover_text(format!(
                        "{} — a second file at {}×{}. Shots are cropped to fill it; \
                         clips you framed by hand stay as they are.",
                        shape.hint, size.width, size.height
                    ))
                    .changed()
                {
                    dialog.also.retain(|ratio| *ratio != shape.ratio);
                    if ticked {
                        dialog.also.push(shape.ratio);
                    }
                }
            }
        });
    });

    let extra = dialog.extra_shapes(native);
    if !extra.is_empty() {
        let names: Vec<String> = extra
            .iter()
            .filter_map(|shape| {
                dialog
                    .path_for(Some(*shape))
                    .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            })
            .collect();
        ui.label(
            egui::RichText::new(format!(
                "Also writes {}, one after another. Your edit is not changed.",
                names.join(", ")
            ))
            .small()
            .color(theme::DISABLED),
        );
    }
}

/// One labelled line of the form.
fn row<R>(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Sense::hover(),
        );
        ui.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        control(ui)
    })
    .inner
}

/// Where the file goes: the folder, and a button that changes it.
///
/// The whole path *was* the button, and clicking a file name to change where it
/// saves reads as nothing at all — a label that happens to be clickable is not
/// a control. `🗁` is in egui's documented emoji set, and `tests/glyphs.rs`
/// checks it against the family that draws it.
fn destination_row(ui: &mut egui::Ui, dialog: &mut ExportDialog) {
    row(ui, "Export to", |ui| {
        let mut shown = dialog.path().map_or_else(
            || "(no folder chosen)".to_owned(),
            |p| p.display().to_string(),
        );
        ui.add(
            egui::TextEdit::singleline(&mut shown)
                .desired_width(FIELD_WIDTH - 32.0)
                // Read-only, not disabled: the text stays legible and can be
                // selected and copied, which is half of what a path is for.
                .interactive(false),
        );

        if ui
            .button("🗁")
            .on_hover_text("Choose the folder to save into")
            .clicked()
            && let Some(chosen) = rfd::FileDialog::new()
                .add_filter("Video", &["mp4", "mov"])
                .add_filter("Sound", &["wav"])
                .set_file_name(format!(
                    "{}.{}",
                    dialog.name, CONTAINERS[dialog.container].0
                ))
                .save_file()
        {
            // The picker returns a full path; split it back into the two things
            // this dialog edits separately.
            if let Some(parent) = chosen.parent() {
                dialog.folder = Some(parent.to_path_buf());
            }
            if let Some(stem) = chosen.file_stem() {
                dialog.name = stem.to_string_lossy().into_owned();
            }
            if let Some(index) = chosen
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|e| CONTAINERS.iter().position(|(name, _)| *name == e))
            {
                dialog.container = index;
            }
            dialog.complaint = None;
        }
    });
}

fn resolution_row(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: Resolution) {
    row(ui, "Resolution", |ui| {
        let size = dialog.resolution(native);
        let selected = match dialog.height_preset {
            Some(index) => HEIGHTS[index].0.to_owned(),
            None => format!("Custom ({}×{})", size.width, size.height),
        };

        egui::ComboBox::from_id_salt("export_resolution")
            .selected_text(selected)
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for (index, (label, height)) in HEIGHTS.iter().enumerate() {
                    let at = fit_short_edge(native, *height);
                    if ui
                        .selectable_label(
                            dialog.height_preset == Some(index),
                            format!("{label}  —  {}×{}", at.width, at.height),
                        )
                        .clicked()
                    {
                        dialog.height_preset = Some(index);
                    }
                }
                if ui
                    .selectable_label(dialog.height_preset.is_none(), "Custom…")
                    .clicked()
                {
                    // Start from whatever the preset was showing, so switching
                    // to custom does not jump the picture to something else.
                    dialog.custom = Some(size);
                    dialog.height_preset = None;
                }
            });
    });

    if dialog.height_preset.is_none() {
        row(ui, "", |ui| {
            let current = dialog.custom.unwrap_or(native);
            let mut width = current.width;
            let mut height = current.height;
            let aspect = aspect_of(native);

            let w = ui.add(
                egui::DragValue::new(&mut width)
                    .speed(2)
                    .range(16..=7680)
                    .prefix("w "),
            );
            let h = ui.add(
                egui::DragValue::new(&mut height)
                    .speed(2)
                    .range(16..=4320)
                    .prefix("h "),
            );
            // Both follow the sequence's shape. A squashed export is almost
            // always a mistake; changing the *sequence* aspect is the Inspector's
            // job, and doing it there keeps the preview honest about the result.
            if w.changed() {
                dialog.custom = Some(even(Resolution::new(
                    width,
                    (f64::from(width) / aspect).round() as u32,
                )));
            }
            if h.changed() {
                dialog.custom = Some(even(Resolution::new(
                    (f64::from(height) * aspect).round() as u32,
                    height,
                )));
            }
            ui.label(
                egui::RichText::new("keeps the sequence's shape")
                    .small()
                    .color(theme::DISABLED),
            );
        });
    }
}

fn bitrate_rows(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: Resolution) {
    let recommended = automatic_kbps(dialog, native);

    row(ui, "Bit rate", |ui| {
        let chosen = dialog.bitrate;
        egui::ComboBox::from_id_salt("export_bitrate_mode")
            .selected_text(match chosen {
                BitrateChoice::Custom => "Custom".to_owned(),
                other => format!("{}  —  {} kb/s", other.label(), other.kbps(recommended)),
            })
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                // Listed smallest to largest, with the number each one works
                // out to, so the names are anchored to something real rather
                // than being four words the user has to guess between.
                for choice in BitrateChoice::ALL {
                    let label = match choice {
                        BitrateChoice::Custom => "Custom…".to_owned(),
                        other => {
                            format!("{}  —  {} kb/s", other.label(), other.kbps(recommended))
                        }
                    };
                    if ui
                        .selectable_label(chosen == choice, label)
                        .on_hover_text(choice.description())
                        .clicked()
                    {
                        dialog.bitrate = choice;
                        if choice == BitrateChoice::Custom && dialog.custom_kbps.is_none() {
                            // Start from where they were, not from a number
                            // unrelated to what they had selected.
                            dialog.custom_kbps = Some(chosen.kbps(recommended));
                        }
                    }
                }
            });
    });

    if dialog.bitrate == BitrateChoice::Custom {
        let mut kbps = dialog.custom_kbps.unwrap_or(recommended);
        row(ui, "", |ui| {
            let response = ui.add(
                egui::DragValue::new(&mut kbps)
                    .speed(100)
                    .range(200..=200_000)
                    .suffix(" kb/s"),
            );
            response.on_hover_text("Higher is better looking and larger. 8000 suits 1080p.");
        });
        dialog.custom_kbps = Some(kbps);
    }

    // Offered for every rate the user has taken a view on, and hidden only for
    // Recommended — where the whole point is not having to decide, and where
    // constant rate would pad a file for no reason the user asked for.
    if dialog.bitrate != BitrateChoice::Recommended {
        row(ui, "", |ui| {
            ui.vertical(|ui| {
                for mode in [RateControl::Constant, RateControl::Variable] {
                    if ui
                        .radio(dialog.rate_control == mode, mode.label())
                        .on_hover_text(mode.description())
                        .clicked()
                    {
                        dialog.rate_control = mode;
                    }
                }
            });
        });
    }
}

fn codec_row(ui: &mut egui::Ui, dialog: &mut ExportDialog) {
    row(ui, "Codec", |ui| {
        egui::ComboBox::from_id_salt("export_codec")
            .selected_text(dialog.codec.label())
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for codec in VideoCodec::ALL {
                    let usable = dialog.can_write(codec);
                    let selected = dialog.codec == codec;
                    let response = ui
                        .add_enabled_ui(usable, |ui| ui.selectable_label(selected, codec.label()))
                        .inner;
                    let response = if usable {
                        response.on_hover_text(codec.description())
                    } else {
                        // The reason matters. Greyed out with no explanation is
                        // the thing to avoid.
                        response.on_disabled_hover_text(
                            "This computer has no H.265 encoder. H.265 needs a \
                             recent graphics card, and there is no software \
                             alternative we can ship — the usual one is licensed \
                             in a way that would apply to this whole program.",
                        )
                    };
                    if response.clicked() {
                        dialog.codec = codec;
                    }
                }
            });
    });
}

fn container_row(ui: &mut egui::Ui, dialog: &mut ExportDialog) {
    row(ui, "Format", |ui| {
        egui::ComboBox::from_id_salt("export_container")
            .selected_text(CONTAINERS[dialog.container].0)
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for (index, (name, hint)) in CONTAINERS.iter().enumerate() {
                    if ui
                        .selectable_label(dialog.container == index, *name)
                        .on_hover_text(*hint)
                        .clicked()
                    {
                        dialog.container = index;
                    }
                }
            });
    });
}

fn frame_rate_row(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: FrameRate) {
    row(ui, "Frame rate", |ui| {
        let current = dialog.frame_rate.unwrap_or(native);
        egui::ComboBox::from_id_salt("export_frame_rate")
            .selected_text(format!("{current} fps"))
            .width(FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for rate in FrameRate::SUPPORTED {
                    let label = if rate == native {
                        format!("{rate} fps  (sequence)")
                    } else {
                        format!("{rate} fps")
                    };
                    if ui
                        .selectable_label(current == rate, label)
                        .on_hover_text(
                            "Only rates the timeline divides exactly are offered, \
                             so the picture and the sound cannot drift apart (§9).",
                        )
                        .clicked()
                    {
                        dialog.frame_rate = Some(rate);
                    }
                }
            });
    });
}

/// A GIF's size: `width` wide, or the sequence's own width if that is
/// smaller, keeping the sequence's shape.
fn gif_size(native: Resolution, width: u32) -> Resolution {
    let width = width.min(native.width).max(1);
    let height = (u64::from(native.height) * u64::from(width) + u64::from(native.width) / 2)
        / u64::from(native.width.max(1));
    Resolution {
        width,
        height: (height as u32).max(1),
    }
}

/// Roughly how many bytes a GIF of this size, rate and length comes to. Rough
/// on purpose: how well a frame compresses depends on the picture, and a third
/// of a byte a pixel is where ordinary footage lands.
fn gif_bytes(size: Resolution, rate: FrameRate, duration: TimelineTime) -> f64 {
    let seconds =
        duration.ticks() as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
    f64::from(size.width) * f64::from(size.height) * rate.as_f64() * seconds * 0.35
}

fn gif_rows(ui: &mut egui::Ui, dialog: &mut ExportDialog, native: Resolution) {
    row(ui, "Width", |ui| {
        for width in GIF_WIDTHS {
            let size = gif_size(native, width);
            if ui
                .selectable_label(dialog.gif_width() == width, format!("{width}"))
                .on_hover_text(format!("{} × {}", size.width, size.height))
                .clicked()
            {
                dialog.gif_width = width;
            }
        }
    });
    row(ui, "Frames a second", |ui| {
        for rate in GIF_RATES {
            let current = dialog.gif_rate() == FrameRate::new(rate, 1).unwrap_or(FrameRate::FPS_30);
            if ui
                .selectable_label(current, format!("{rate}"))
                .on_hover_text("Fewer frames make a smaller file and a choppier loop")
                .clicked()
            {
                dialog.gif_rate = rate;
            }
        }
    });
    row(ui, "Written as", |ui| {
        ui.label(
            egui::RichText::new("GIF · 256 colours a frame · loops · no sound")
                .color(theme::DISABLED),
        );
    });
}

fn summary(ui: &mut egui::Ui, dialog: &ExportDialog, native: Resolution, duration: TimelineTime) {
    if dialog.gif() {
        let size = gif_size(native, dialog.gif_width());
        let megabytes = gif_bytes(size, dialog.gif_rate(), duration) / 1_000_000.0;
        ui.label(
            egui::RichText::new(format!(
                "{} · {}×{} GIF · about {megabytes:.0} MB",
                duration.format_timecode(),
                size.width,
                size.height
            ))
            .small()
            .color(theme::DISABLED),
        );
        return;
    }
    if dialog.sound_only() {
        let seconds =
            duration.ticks() as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
        let megabytes = WAV_KBPS * seconds / 8_000.0;
        ui.label(
            egui::RichText::new(format!(
                "{} · sound only · about {megabytes:.0} MB",
                duration.format_timecode()
            ))
            .small()
            .color(theme::DISABLED),
        );
        return;
    }
    let size = dialog.resolution(native);
    let kbps = dialog.effective_kbps(native);
    let seconds =
        duration.ticks() as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
    // Video plus 192 kb/s of audio, in megabytes.
    let megabytes = (f64::from(kbps) + 192.0) * seconds / 8_000.0;

    let extra = dialog.extra_shapes(native).len();
    let files = match extra {
        0 => String::new(),
        n => format!(" · {} files", n + 1),
    };
    ui.label(
        egui::RichText::new(format!(
            "{} · {}×{} · about {megabytes:.0} MB{files}",
            duration.format_timecode(),
            size.width,
            size.height,
        ))
        .small()
        .color(theme::DISABLED),
    )
    .on_hover_text("Export always reads your original files, never the proxies used for editing.");
}

/// The automatic bitrate, in kilobits, mirroring what the encoder would pick.
///
/// Shown rather than left implicit so "Recommended" is a number the user can
/// judge and then override, instead of a black box. It has to stay in step with
/// `encoders::bitrate_for`, and a test asserts it does.
fn automatic_kbps(dialog: &ExportDialog, native: Resolution) -> u32 {
    automatic_kbps_at(dialog, dialog.resolution(native))
}

/// The same, at an output size given directly.
fn automatic_kbps_at(dialog: &ExportDialog, size: Resolution) -> u32 {
    let pixels = f64::from(size.width) * f64::from(size.height);
    let fps = dialog
        .frame_rate
        .map_or(30.0, bettercut_editor_core::foundation::FrameRate::as_f64);
    let scale = match dialog.codec {
        VideoCodec::H264 => 1.0,
        VideoCodec::H265 => 0.55,
    };
    // The same shape as `encoders::bitrate_for`: 0.2 bits per pixel at 30 fps.
    let bits = pixels * fps.clamp(1.0, 120.0) / 5.0 * scale;
    ((bits / 1_000.0) as u32).clamp(1_000, 120_000)
}

/// The shorter of the two dimensions — what "1080p" names.
fn short_edge(size: Resolution) -> u32 {
    size.width.min(size.height)
}

fn aspect_of(size: Resolution) -> f64 {
    if size.height == 0 {
        return 16.0 / 9.0;
    }
    f64::from(size.width) / f64::from(size.height)
}

/// Scale a resolution so its **short** edge is `short`, keeping the shape.
///
/// Short, not long. "1080p" means 1080 lines: 1920×1080 landscape, 1080×1920
/// vertical, 1080×1080 square. Scaling the long edge instead turns a 4K
/// sequence into 1080×608 — which is exactly what this did before, and what a
/// test asserted, because the test was written from the same misreading.
fn fit_short_edge(native: Resolution, short: u32) -> Resolution {
    let (w, h) = (native.width.max(1), native.height.max(1));
    let scaled = if w <= h {
        Resolution::new(
            short,
            (u64::from(short) * u64::from(h) / u64::from(w)) as u32,
        )
    } else {
        Resolution::new(
            (u64::from(short) * u64::from(w) / u64::from(h)) as u32,
            short,
        )
    };
    even(scaled)
}

/// Round both dimensions down to even.
///
/// H.264 and H.265 both store chroma at half resolution in each direction, so
/// an odd dimension has no representation and the writer refuses it. Rounding
/// here means any number the user picks is accepted rather than sometimes
/// failing at the last moment.
fn even(size: Resolution) -> Resolution {
    Resolution::new(size.width.max(2) & !1, size.height.max(2) & !1)
}

/// Strip what a file name cannot hold, rather than handing the OS something it
/// will reject: a project called "Trip 6/7" is a perfectly good name.
fn safe_file_name(name: &str) -> String {
    let trimmed = name.trim();
    let source = if trimmed.is_empty() { "video" } else { trimmed };
    source
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect()
}

/// Beside the project, or the user's Videos folder.
fn default_folder(editor: &Editor) -> std::path::PathBuf {
    editor
        .path()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .or_else(videos_directory)
        .unwrap_or_else(std::env::temp_dir)
}

fn videos_directory() -> Option<std::path::PathBuf> {
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

    /// The bug this replaced: "1080p" set the *long* edge, so a 4K sequence
    /// exported at 1080×608 instead of 1920×1080.
    #[test]
    fn a_preset_names_the_short_edge() {
        let uhd = fit_short_edge(Resolution::new(3840, 2160), 1080);
        assert_eq!((uhd.width, uhd.height), (1920, 1080));

        let already = fit_short_edge(Resolution::new(1920, 1080), 1080);
        assert_eq!((already.width, already.height), (1920, 1080));

        // Vertical: 1080p means 1080 across, 1920 tall.
        let portrait = fit_short_edge(Resolution::new(1080, 1920), 1080);
        assert_eq!((portrait.width, portrait.height), (1080, 1920));

        let square = fit_short_edge(Resolution::new(1000, 1000), 720);
        assert_eq!((square.width, square.height), (720, 720));

        // And an unusual shape keeps its shape.
        let wide = fit_short_edge(Resolution::new(2560, 1080), 720);
        assert_eq!((wide.width, wide.height), (1706, 720));
    }

    #[test]
    fn every_preset_of_every_shape_is_even() {
        let shapes = [
            Resolution::new(3840, 2160),
            Resolution::new(1080, 1920),
            Resolution::new(1000, 1000),
            Resolution::new(2560, 1080),
            Resolution::new(1439, 1079),
        ];
        for shape in shapes {
            for (label, height) in HEIGHTS {
                let size = fit_short_edge(shape, height);
                assert!(
                    size.width.is_multiple_of(2) && size.height.is_multiple_of(2),
                    "{label} of {}×{} gave {}×{}",
                    shape.width,
                    shape.height,
                    size.width,
                    size.height
                );
            }
        }
    }

    fn dialog_at(width: u32, height: u32, rate: FrameRate, codec: VideoCodec) -> ExportDialog {
        ExportDialog {
            height_preset: None,
            custom: Some(Resolution::new(width, height)),
            frame_rate: Some(rate),
            codec,
            ..ExportDialog::default()
        }
    }

    /// The number shown next to "Recommended" has to be the number the encoder
    /// will actually use, or it is worse than showing nothing.
    #[test]
    fn the_shown_automatic_bitrate_matches_the_encoders_own() {
        let native = Resolution::new(1920, 1080);
        let hd = dialog_at(1920, 1080, FrameRate::FPS_30, VideoCodec::H264);
        let shown = automatic_kbps(&hd, native);
        // encoders::bitrate_for: 1920*1080*30/5 = 12,441,600 bits.
        assert!((12_200..=12_600).contains(&shown), "shown {shown} kb/s");

        let hevc = automatic_kbps(
            &dialog_at(1920, 1080, FrameRate::FPS_30, VideoCodec::H265),
            native,
        );
        assert!(hevc < shown, "H.265 should ask for less: {hevc} vs {shown}");
        assert!(hevc > shown / 2, "and not so much less that it looks worse");
    }

    #[test]
    fn the_automatic_bitrate_follows_the_settings() {
        let native = Resolution::new(1920, 1080);
        let full = automatic_kbps(
            &dialog_at(1920, 1080, FrameRate::FPS_30, VideoCodec::H264),
            native,
        );
        let half = automatic_kbps(
            &dialog_at(960, 540, FrameRate::FPS_30, VideoCodec::H264),
            native,
        );
        assert!(
            (full / 5..=full / 3).contains(&half),
            "half the size should be about a quarter the bitrate: {half} vs {full}"
        );

        let faster = automatic_kbps(
            &dialog_at(1920, 1080, FrameRate::FPS_60, VideoCodec::H264),
            native,
        );
        assert!(faster > full * 3 / 2, "60 fps should cost more: {faster}");
    }

    /// A codec the machine cannot write must not be selectable. `can_write`
    /// answers optimistically before the probe has run, because the window has
    /// not opened yet and refusing everything would be worse.
    #[test]
    fn an_unavailable_codec_is_not_writable() {
        let dialog = ExportDialog {
            available: vec![(VideoCodec::H264, true), (VideoCodec::H265, false)],
            ..ExportDialog::default()
        };
        assert!(dialog.can_write(VideoCodec::H264));
        assert!(!dialog.can_write(VideoCodec::H265));
        assert!(ExportDialog::default().can_write(VideoCodec::H265));
    }

    /// The name and the container are edited separately, and the path is built
    /// from them — so changing the format must change the extension, not leave
    /// an `.mp4` holding a QuickTime file.
    #[test]
    fn the_path_follows_the_name_and_the_container() {
        let mut dialog = ExportDialog {
            name: "holiday".to_owned(),
            folder: Some(std::path::PathBuf::from("/tmp")),
            ..ExportDialog::default()
        };
        assert!(dialog.path().unwrap().ends_with("holiday.mp4"));

        dialog.container = 1;
        assert!(dialog.path().unwrap().ends_with("holiday.mov"));

        // An empty name is a slip, not a request for a file called "".
        dialog.name = "   ".to_owned();
        assert!(dialog.path().unwrap().ends_with("video.mov"));
    }

    /// The tiers have to be a scale: each one strictly larger than the one
    /// before, and Recommended sitting where its name says it does.
    #[test]
    fn the_bitrate_tiers_are_an_ascending_scale() {
        let recommended = 12_000;
        let values: Vec<u32> = BitrateChoice::ALL
            .iter()
            .filter(|c| **c != BitrateChoice::Custom)
            .map(|c| c.kbps(recommended))
            .collect();

        assert!(
            values.windows(2).all(|pair| pair[1] > pair[0]),
            "the tiers are not ascending: {values:?}"
        );
        assert_eq!(BitrateChoice::Recommended.kbps(recommended), recommended);
        assert_eq!(BitrateChoice::Lower.kbps(recommended), 6_000);
        assert_eq!(BitrateChoice::High.kbps(recommended), 18_000);
        // Custom starts from the recommendation, so switching to it does not
        // move the number under the user.
        assert_eq!(BitrateChoice::Custom.kbps(recommended), recommended);
    }

    /// Every tier stays inside the range the encoder will accept, even from a
    /// recommendation already at one end of it.
    #[test]
    fn no_tier_escapes_the_encoders_range() {
        for recommended in [1_000, 12_000, 120_000] {
            for choice in BitrateChoice::ALL {
                let kbps = choice.kbps(recommended);
                assert!(
                    (1_000..=120_000).contains(&kbps),
                    "{} of {recommended} gave {kbps}",
                    choice.label()
                );
            }
        }
    }

    /// What the footer estimates, what the dropdown shows and what the encoder
    /// is handed all come from one place, so they cannot disagree.
    #[test]
    fn the_effective_rate_follows_the_chosen_tier() {
        let native = Resolution::new(1920, 1080);
        let mut dialog = dialog_at(1920, 1080, FrameRate::FPS_30, VideoCodec::H264);
        let recommended = automatic_kbps(&dialog, native);

        dialog.bitrate = BitrateChoice::Recommended;
        assert_eq!(dialog.effective_kbps(native), recommended);

        dialog.bitrate = BitrateChoice::Lower;
        assert_eq!(dialog.effective_kbps(native), recommended / 2);

        dialog.bitrate = BitrateChoice::High;
        assert!(dialog.effective_kbps(native) > recommended);

        // A typed rate wins over every tier.
        dialog.bitrate = BitrateChoice::Custom;
        dialog.custom_kbps = Some(4_321);
        assert_eq!(dialog.effective_kbps(native), 4_321);
    }

    /// A tier scales with the format, not just with itself: "Lower" at 4K must
    /// still be more than "High" at 480p, or the names would mean nothing
    /// across settings.
    #[test]
    fn the_tiers_track_the_output_format() {
        let native = Resolution::new(3840, 2160);
        let mut big = dialog_at(3840, 2160, FrameRate::FPS_30, VideoCodec::H264);
        big.bitrate = BitrateChoice::Lower;

        let mut small = dialog_at(854, 480, FrameRate::FPS_30, VideoCodec::H264);
        small.bitrate = BitrateChoice::High;

        assert!(
            big.effective_kbps(native) > small.effective_kbps(native),
            "Lower at 4K ({}) should exceed High at 480p ({})",
            big.effective_kbps(native),
            small.effective_kbps(native)
        );
    }

    /// Rate control travels with every tier the user has taken a view on, and
    /// Recommended stays variable however it was reached — including after the
    /// user set Constant on another tier and switched back, where the control
    /// is no longer on screen to explain itself.
    #[test]
    fn only_recommended_forces_variable_rate_control() {
        let mut dialog = ExportDialog {
            rate_control: RateControl::Constant,
            ..ExportDialog::default()
        };

        for choice in [
            BitrateChoice::Lower,
            BitrateChoice::Medium,
            BitrateChoice::High,
            BitrateChoice::Custom,
        ] {
            dialog.bitrate = choice;
            assert_eq!(
                dialog.rate_control_for_export(),
                RateControl::Constant,
                "{} should carry the chosen rate control",
                choice.label()
            );
        }

        dialog.bitrate = BitrateChoice::Recommended;
        assert_eq!(
            dialog.rate_control_for_export(),
            RateControl::Variable,
            "Recommended has no rate-control control, so it must not inherit one"
        );
    }

    #[test]
    fn a_project_name_that_cannot_be_a_file_name_is_cleaned() {
        assert_eq!(safe_file_name("Trip 6/7"), "Trip 6-7");
        assert_eq!(safe_file_name("  "), "video");
        assert_eq!(safe_file_name("a:b*c?"), "a-b-c-");
    }

    /// §20a.4: the export reads the same rule the preview does, so a solo left
    /// on from checking one voice exports only that voice. The warning names
    /// which lane, because "something is soloed" sends the user hunting.
    #[test]
    fn the_export_says_when_a_solo_would_cut_the_film_down() {
        use bettercut_editor_core::timeline::Sequence;

        let mut sequence = Sequence::default_hd();
        assert_eq!(soloed_lanes(&sequence), None, "nothing is soloed");

        sequence.audio_tracks[0].solo = true;
        assert_eq!(soloed_lanes(&sequence).as_deref(), Some("A sound track is"));

        sequence.video_tracks[0].solo = true;
        assert_eq!(
            soloed_lanes(&sequence).as_deref(),
            Some("A picture track and a sound track are"),
            "both lanes soloed should say both"
        );

        sequence.audio_tracks[0].solo = false;
        assert_eq!(
            soloed_lanes(&sequence).as_deref(),
            Some("A picture track is")
        );
    }

    fn shape(label: &str) -> Shape {
        SHAPES.into_iter().find(|s| s.label == label).unwrap()
    }

    fn everything() -> TimelineRange {
        TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(10)).unwrap()
    }

    /// One press of Export with two extra shapes ticked asks for three files:
    /// the main one first, then each shape, named so they sort together.
    #[test]
    fn extra_shapes_become_extra_files_beside_the_main_one() {
        let native = Resolution::HD_1080;
        let dialog = ExportDialog {
            name: "trip".to_owned(),
            folder: Some(std::path::PathBuf::from("/out")),
            height_preset: Some(2), // 1080p
            also: vec![(9, 16), (1, 1)],
            ..ExportDialog::default()
        };

        let requests = dialog.requests(native, FrameRate::FPS_30, everything());

        let files: Vec<(Option<&str>, String, Resolution)> = requests
            .iter()
            .map(|r| {
                (
                    r.shape.map(|s| s.label),
                    r.settings
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    r.settings.resolution,
                )
            })
            .collect();
        assert_eq!(
            files,
            vec![
                (None, "trip.mp4".to_owned(), Resolution::HD_1080),
                (
                    Some("9:16"),
                    "trip-9x16.mp4".to_owned(),
                    Resolution::new(1080, 1920)
                ),
                (
                    Some("1:1"),
                    "trip-1x1.mp4".to_owned(),
                    Resolution::new(1080, 1080)
                ),
            ]
        );
    }

    /// A ticked shape the sequence already is would write the same video twice
    /// under two names; it is the main file already.
    #[test]
    fn the_sequences_own_shape_is_not_exported_twice() {
        let dialog = ExportDialog {
            folder: Some(std::path::PathBuf::from("/out")),
            also: vec![(16, 9), (9, 16)],
            ..ExportDialog::default()
        };

        let requests = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything());

        let shapes: Vec<_> = requests.iter().map(|r| r.shape.map(|s| s.label)).collect();
        assert_eq!(shapes, vec![None, Some("9:16")]);
    }

    /// Each file gets the bitrate its own size needs at the chosen tier: a
    /// square cut has fewer pixels than the landscape one and needs fewer bits.
    #[test]
    fn each_shape_gets_the_bitrate_its_size_needs() {
        let native = Resolution::HD_1080;
        let mut dialog = dialog_at(1920, 1080, FrameRate::FPS_30, VideoCodec::H264);
        dialog.folder = Some(std::path::PathBuf::from("/out"));
        dialog.height_preset = Some(2);
        dialog.also = vec![(1, 1)];

        let requests = dialog.requests(native, FrameRate::FPS_30, everything());

        let (main, square) = (&requests[0].settings, &requests[1].settings);
        assert_eq!(
            main.bitrate,
            Some(i64::from(dialog.effective_kbps(native)) * 1_000)
        );
        assert!(
            square.bitrate < main.bitrate,
            "the square cut was given the landscape bitrate: {:?} vs {:?}",
            square.bitrate,
            main.bitrate
        );
        assert_eq!(square.codec, main.codec);
        assert_eq!(square.frame_rate, main.frame_rate);
        assert_eq!(square.range, main.range);
    }

    /// A custom size carries over to the shape at the same short edge, rather
    /// than falling back to the sequence's size.
    #[test]
    fn a_custom_size_is_reshaped_at_its_own_detail() {
        let dialog = dialog_at(1280, 720, FrameRate::FPS_30, VideoCodec::H264);
        assert_eq!(
            dialog.resolution_for(Resolution::HD_1080, Some(shape("9:16"))),
            Resolution::new(720, 1280)
        );
    }

    /// Sound only asks for one WAV, marked as sound only, and ignores extra
    /// shapes left ticked from a video export — sound has no shape.
    #[test]
    fn sound_only_is_one_wav_whatever_shapes_are_ticked() {
        let dialog = ExportDialog {
            name: "podcast".to_owned(),
            folder: Some(std::path::PathBuf::from("/out")),
            container: SOUND_ONLY,
            also: vec![(9, 16), (1, 1)],
            ..ExportDialog::default()
        };

        let requests = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything());

        assert_eq!(requests.len(), 1);
        assert!(requests[0].settings.sound_only);
        assert!(requests[0].settings.path.ends_with("podcast.wav"));
        assert_eq!(CONTAINERS[SOUND_ONLY].0, "wav");
    }

    /// A GIF is one file at the GIF size and rate, keeping the sequence's
    /// shape, with no extra shapes and no sound.
    #[test]
    fn a_gif_is_one_small_looping_file() {
        let dialog = ExportDialog {
            name: "loop".to_owned(),
            folder: Some(std::path::PathBuf::from("/out")),
            container: GIF,
            also: vec![(9, 16)],
            main_shape: SHAPES.into_iter().find(|shape| shape.ratio == (1, 1)),
            ..ExportDialog::default()
        };

        let requests = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything());

        assert_eq!(requests.len(), 1, "extra shapes were written for a GIF");
        let settings = &requests[0].settings;
        assert!(settings.gif && !settings.sound_only);
        assert!(settings.path.ends_with("loop.gif"));
        assert_eq!(requests[0].shape, None);
        assert_eq!(
            (settings.resolution.width, settings.resolution.height),
            (480, 270)
        );
        assert_eq!(settings.frame_rate, FrameRate::new(15, 1).unwrap());
        assert_eq!(CONTAINERS[GIF].0, "gif");
    }

    /// A GIF never comes out wider than the sequence, and keeps its shape.
    #[test]
    fn a_gif_is_never_bigger_than_the_sequence() {
        let small = Resolution {
            width: 400,
            height: 400,
        };
        let size = gif_size(small, 640);
        assert_eq!((size.width, size.height), (400, 400));
        let vertical = gif_size(Resolution::VERTICAL_1080, 320);
        assert_eq!((vertical.width, vertical.height), (320, 569));
        assert!(
            gif_bytes(size, FrameRate::FPS_30, TimelineTime::from_seconds(2))
                > gif_bytes(
                    size,
                    FrameRate::new(10, 1).unwrap(),
                    TimelineTime::from_seconds(2)
                )
        );
    }

    /// And a video format is never sound only, nor a GIF.
    #[test]
    fn video_formats_write_pictures() {
        for container in (0..CONTAINERS.len()).filter(|c| *c != SOUND_ONLY && *c != GIF) {
            let dialog = ExportDialog {
                folder: Some(std::path::PathBuf::from("/out")),
                container,
                ..ExportDialog::default()
            };
            let requests = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything());
            assert!(
                !requests[0].settings.sound_only && !requests[0].settings.gif,
                "{}",
                CONTAINERS[container].0
            );
        }
    }

    fn platform(label: &str) -> Platform {
        PLATFORMS.into_iter().find(|p| p.label == label).unwrap()
    }

    /// TikTok from a landscape edit: one vertical file, 1080×1920 at 30 fps,
    /// H.264 in an MP4, cropped from the edit — under the plain name.
    #[test]
    fn a_vertical_platform_reshapes_the_main_file() {
        let native = Resolution::HD_1080;
        let mut dialog = ExportDialog {
            name: "trip".to_owned(),
            folder: Some(std::path::PathBuf::from("/out")),
            container: 1,
            codec: VideoCodec::H265,
            ..ExportDialog::default()
        };

        dialog.apply_platform(
            platform("TikTok · Reels · Shorts"),
            native,
            FrameRate::FPS_60,
        );
        let requests = dialog.requests(native, FrameRate::FPS_60, everything());

        assert_eq!(requests.len(), 1);
        let main = &requests[0];
        assert_eq!(main.shape.map(|s| s.label), Some("9:16"));
        assert_eq!(main.settings.resolution, Resolution::new(1080, 1920));
        assert_eq!(main.settings.frame_rate, FrameRate::FPS_30);
        assert_eq!(main.settings.codec, VideoCodec::H264);
        assert!(
            main.settings.path.ends_with("trip.mp4"),
            "{:?}",
            main.settings.path
        );
    }

    /// YouTube 4K on a landscape sequence keeps the sequence's shape and rate
    /// and exports at 3840×2160.
    #[test]
    fn a_platform_in_the_sequences_shape_changes_only_the_format() {
        let native = Resolution::HD_1080;
        let mut dialog = ExportDialog {
            folder: Some(std::path::PathBuf::from("/out")),
            ..ExportDialog::default()
        };

        dialog.apply_platform(platform("YouTube 4K"), native, FrameRate::FILM_24);
        let requests = dialog.requests(native, FrameRate::FILM_24, everything());

        assert!(requests[0].shape.is_none());
        assert_eq!(requests[0].settings.resolution, Resolution::new(3840, 2160));
        assert_eq!(requests[0].settings.frame_rate, FrameRate::FILM_24);
    }

    /// The menu names the platform while the settings are what it wants, and
    /// says Custom as soon as one of them is changed by hand.
    #[test]
    fn the_platform_is_shown_until_a_setting_is_changed() {
        let native = Resolution::HD_1080;
        let mut dialog = ExportDialog::default();
        assert_eq!(dialog.platform_in_use(native, FrameRate::FPS_30), None);

        dialog.apply_platform(platform("Instagram feed"), native, FrameRate::FPS_30);
        assert_eq!(
            dialog
                .platform_in_use(native, FrameRate::FPS_30)
                .map(|p| p.label),
            Some("Instagram feed")
        );

        dialog.bitrate = BitrateChoice::Lower;
        assert_eq!(dialog.platform_in_use(native, FrameRate::FPS_30), None);
    }

    /// With the main file reshaped, its shape is not offered as an extra, and
    /// the sequence's own shape can be — a vertical upload and the landscape
    /// original in one go.
    #[test]
    fn the_sequences_shape_can_ride_along_a_reshaped_main_file() {
        let native = Resolution::HD_1080;
        let mut dialog = ExportDialog {
            name: "trip".to_owned(),
            folder: Some(std::path::PathBuf::from("/out")),
            also: vec![(16, 9), (9, 16)],
            ..ExportDialog::default()
        };

        dialog.apply_platform(
            platform("TikTok · Reels · Shorts"),
            native,
            FrameRate::FPS_30,
        );
        let requests = dialog.requests(native, FrameRate::FPS_30, everything());

        let shapes: Vec<_> = requests.iter().map(|r| r.shape.map(|s| s.label)).collect();
        assert_eq!(shapes, vec![Some("9:16"), Some("16:9")]);
        assert!(requests[1].settings.path.ends_with("trip-16x9.mp4"));
    }

    /// Every platform lands on an even size its menu row describes.
    #[test]
    fn every_platform_produces_its_stated_size() {
        for platform in PLATFORMS {
            let mut dialog = ExportDialog {
                folder: Some(std::path::PathBuf::from("/out")),
                ..ExportDialog::default()
            };
            dialog.apply_platform(platform, Resolution::HD_1080, FrameRate::FPS_30);
            let size = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything())[0]
                .settings
                .resolution;
            assert!(
                platform
                    .hint
                    .contains(&format!("{}×{}", size.width, size.height)),
                "{} says {:?} but exports {}×{}",
                platform.label,
                platform.hint,
                size.width,
                size.height
            );
        }
    }

    /// The marked span is exported when chosen and present; otherwise the
    /// whole sequence, and nothing at all for an empty one.
    #[test]
    fn the_export_range_follows_the_marks() {
        let whole = TimelineTime::from_seconds(20);
        let marked =
            TimelineRange::new(TimelineTime::from_seconds(3), TimelineTime::from_seconds(8))
                .unwrap();
        let everything = TimelineRange::new(TimelineTime::ZERO, whole).unwrap();

        assert_eq!(export_range(true, Some(marked), whole), Some(marked));
        assert_eq!(export_range(false, Some(marked), whole), Some(everything));
        assert_eq!(export_range(true, None, whole), Some(everything));
        assert_eq!(export_range(true, None, TimelineTime::ZERO), None);
        // Marks on an empty sequence still make something to export.
        assert_eq!(
            export_range(true, Some(marked), TimelineTime::ZERO),
            Some(marked)
        );
    }

    /// Nothing ticked is exactly the export there was before.
    #[test]
    fn with_nothing_ticked_there_is_one_file() {
        let dialog = ExportDialog {
            folder: Some(std::path::PathBuf::from("/out")),
            ..ExportDialog::default()
        };
        let requests = dialog.requests(Resolution::HD_1080, FrameRate::FPS_30, everything());
        assert_eq!(requests.len(), 1);
        assert!(requests[0].shape.is_none());
        assert_eq!(requests[0].settings.path, dialog.path().unwrap());
    }
}
