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

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{FrameRate, TimelineTime};
use bettercut_editor_core::timeline::{Resolution, TimelineRange};
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
const CONTAINERS: [(&str, &str); 2] = [("mp4", "Plays everywhere"), ("mov", "QuickTime; editors")];

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
    /// Which codecs this machine can write, probed when the window opens.
    available: Vec<(VideoCodec, bool)>,
    complaint: Option<String>,
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
        match self.height_preset {
            Some(index) => fit_short_edge(native, HEIGHTS[index].1),
            // Falls back to the sequence: `custom` is only unset before the
            // window has ever opened, and the sequence is the honest answer.
            None => even(self.custom.unwrap_or(native)),
        }
    }

    /// The bitrate that will actually be used, in kilobits.
    ///
    /// One place, so the dropdown, the estimate in the footer and the number
    /// handed to the encoder cannot disagree.
    fn effective_kbps(&self, native: Resolution) -> u32 {
        let recommended = automatic_kbps(self, native);
        match self.bitrate {
            BitrateChoice::Custom => self.custom_kbps.unwrap_or(recommended),
            other => other.kbps(recommended),
        }
    }

    fn path(&self) -> Option<std::path::PathBuf> {
        let name = if self.name.trim().is_empty() {
            "video"
        } else {
            self.name.trim()
        };
        Some(
            self.folder
                .as_ref()?
                .join(format!("{name}.{}", CONTAINERS[self.container].0)),
        )
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

    let native = sequence.resolution;
    let native_rate = sequence.frame_rate;
    let duration = sequence.duration();
    let sequence_name = sequence.name.clone();
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

            ui.add_space(6.0);
            ui.separator();
            ui.label(egui::RichText::new("Video").strong());
            ui.add_space(2.0);

            resolution_row(ui, dialog, native);
            bitrate_rows(ui, dialog, native);
            codec_row(ui, dialog);
            container_row(ui, dialog);
            frame_rate_row(ui, dialog, native_rate);

            row(ui, "Colour space", |ui| {
                ui.label(egui::RichText::new("Rec. 709 SDR").color(theme::DISABLED))
                    .on_hover_text(
                        "§21a fixes the working space and tags every export to \
                     match, so players do not have to guess.",
                    );
            });

            if let Some(complaint) = &dialog.complaint {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(complaint).color(theme::ERROR_TEXT));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                summary(ui, dialog, native, duration);

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

    start?;
    let path = dialog.path()?;

    // An empty sequence has nothing to write. Refusing here is the difference
    // between a message beside the button and a failure a minute later.
    let Ok(range) = TimelineRange::new(TimelineTime::ZERO, duration) else {
        dialog.complaint = Some("This sequence is empty — add a clip first.".to_owned());
        return None;
    };

    dialog.open = false;
    Some(ExportSettings {
        path,
        resolution: dialog.resolution(native),
        frame_rate: dialog.frame_rate.unwrap_or(native_rate),
        codec: dialog.codec,
        bitrate: Some(i64::from(dialog.effective_kbps(native)) * 1_000),
        // Only a typed rate carries a rate-control choice; see `bitrate_rows`.
        rate_control: if dialog.bitrate == BitrateChoice::Custom {
            dialog.rate_control
        } else {
            RateControl::Variable
        },
        range,
        // §15.1: FFmpeg never gets every core. The renderer and the decoder are
        // both working during an export, and leaving the machine responsive
        // matters more than finishing a minute sooner.
        threads: 2,
    })
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

        // Rate control is offered only here. The named tiers all mean "spend
        // about this much", which is variable by definition; someone who needs
        // a rate held exactly — a broadcast or ingest spec — has an exact
        // number to type, and types it.
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
                        // the thing §41 is against.
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

fn summary(ui: &mut egui::Ui, dialog: &ExportDialog, native: Resolution, duration: TimelineTime) {
    let size = dialog.resolution(native);
    let kbps = dialog.effective_kbps(native);
    let seconds =
        duration.ticks() as f64 / bettercut_editor_core::foundation::TICKS_PER_SECOND as f64;
    // Video plus 192 kb/s of audio, in megabytes.
    let megabytes = (f64::from(kbps) + 192.0) * seconds / 8_000.0;

    ui.label(
        egui::RichText::new(format!(
            "{} · {}×{} · about {megabytes:.0} MB",
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
    let size = dialog.resolution(native);
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

    #[test]
    fn a_project_name_that_cannot_be_a_file_name_is_cleaned() {
        assert_eq!(safe_file_name("Trip 6/7"), "Trip 6-7");
        assert_eq!(safe_file_name("  "), "video");
        assert_eq!(safe_file_name("a:b*c?"), "a-b-c-");
    }
}
