//! Video scopes: a histogram and a waveform of the frame under the playhead.
//!
//! What a colourist reads instead of trusting a monitor: the histogram says
//! how the frame's tones are spread — a pile against the right edge is blown
//! highlights, against the left crushed shadows — and the waveform says the
//! same for each part of the picture from left to right, so a bright sky and
//! a dark foreground show as two bands rather than one blur. The parade is
//! that waveform split into its three channels, side by side: where the luma
//! trace says how bright a part of the picture is, the parade says which
//! colour is carrying it — a cast shows as one panel sitting higher than the
//! other two, which no amount of staring at the picture reliably tells you.
//!
//! Measured on the export's own render of the frame (`FrameGrabJob`), so the
//! scopes describe what the file will hold, not the preview's proxy. Values
//! are the encoded 8-bit ones, with Rec.709 luma weights, as broadcast scopes
//! read them.

use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::TimelineTime;

use crate::state::UiState;
use crate::theme;

/// Waveform columns across the picture and rows up the level scale.
pub const WAVEFORM_COLUMNS: usize = 128;
pub const WAVEFORM_ROWS: usize = 64;

/// Columns across the picture in each of the parade's three panels.
///
/// Narrower than the luma waveform because three of them share the width the
/// one had: any finer and a column would be under a pixel wide on screen.
pub const PARADE_COLUMNS: usize = 48;

/// Red, green, blue — the order a parade has always been read in.
pub const PARADE_CHANNELS: usize = 3;

/// Cells across and down the vectorscope's colour plane.
///
/// A vectorscope is read for *shape* — which way the colour leans, how far it
/// goes — so the grid only has to be fine enough that the shape is a shape.
/// 64 across is a cell every 1.5% of the plane, which at the size the window
/// draws it is smaller than a pixel.
pub const VECTOR_CELLS: usize = 64;

/// Where skin sits on that plane, as an angle from the +Cb axis: the line
/// every colourist checks a face against.
pub const SKIN_TONE_DEGREES: f32 = 123.0;

/// How long the window waits after the playhead last moved before reading
/// the new frame, so dragging the playhead does not queue a render per frame.
pub const SETTLE_SECONDS: f64 = 0.35;

/// The scopes for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Scopes {
    pub red: [u32; 256],
    pub green: [u32; 256],
    pub blue: [u32; 256],
    pub luma: [u32; 256],
    /// `WAVEFORM_COLUMNS` × `WAVEFORM_ROWS` counts, row 0 the darkest,
    /// column by column.
    pub waveform: Vec<u32>,
    /// The same, once per channel: red, then green, then blue, each
    /// `PARADE_COLUMNS` × `WAVEFORM_ROWS`, column by column.
    pub parade: Vec<u32>,
    /// `VECTOR_CELLS` × `VECTOR_CELLS` counts over the colour plane: Cb across
    /// (blue to the right), Cr up (red at the top), grey in the middle.
    ///
    /// The plane the eye reads as hue and saturation — the angle is which
    /// colour, the distance from the middle is how much of it — which is the
    /// one question a histogram cannot answer.
    pub vector: Vec<u32>,
    /// How many pixels were measured.
    pub samples: u32,
}

/// A pixel's place on the colour plane, each -0.5 to 0.5: Rec.709's own
/// colour difference, which is what the encoder stores and what a vectorscope
/// has always shown.
pub fn chroma_of(r: u8, g: u8, b: u8) -> (f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    ((b - y) / 1.8556, (r - y) / 1.5748)
}

/// Which cell of the plane a colour falls in, held inside it: Cb across, Cr
/// up, so the row is counted from the top as the counts are stored.
pub fn cell_of(cb: f32, cr: f32) -> usize {
    let last = VECTOR_CELLS - 1;
    let scale =
        |value: f32| (((value + 0.5) * last as f32).round() as i64).clamp(0, last as i64) as usize;
    let across = scale(cb);
    let down = last - scale(cr);
    down * VECTOR_CELLS + across
}

fn luma_of(r: u8, g: u8, b: u8) -> u8 {
    // Rec.709 in integers: 0.2126, 0.7152, 0.0722 of 10,000.
    ((u32::from(r) * 2126 + u32::from(g) * 7152 + u32::from(b) * 722 + 5_000) / 10_000) as u8
}

impl Scopes {
    /// Measure tightly packed RGBA rows, `width` by `height`. Large frames
    /// are read every other pixel each way: the shape is the same, and it is
    /// four times quicker. `None` when the rows do not match the size.
    pub fn of(rgba: &[u8], width: u32, height: u32) -> Option<Self> {
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 || rgba.len() < width * height * 4 {
            return None;
        }
        let step = if width * height > 1_000_000 { 2 } else { 1 };
        let mut scopes = Self {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
            luma: [0; 256],
            waveform: vec![0; WAVEFORM_COLUMNS * WAVEFORM_ROWS],
            parade: vec![0; PARADE_CHANNELS * PARADE_COLUMNS * WAVEFORM_ROWS],
            vector: vec![0; VECTOR_CELLS * VECTOR_CELLS],
            samples: 0,
        };
        for y in (0..height).step_by(step) {
            for x in (0..width).step_by(step) {
                let i = (y * width + x) * 4;
                let (r, g, b) = (rgba[i], rgba[i + 1], rgba[i + 2]);
                let l = luma_of(r, g, b);
                scopes.red[r as usize] += 1;
                scopes.green[g as usize] += 1;
                scopes.blue[b as usize] += 1;
                scopes.luma[l as usize] += 1;
                let column = x * WAVEFORM_COLUMNS / width;
                let row = l as usize * WAVEFORM_ROWS / 256;
                scopes.waveform[column * WAVEFORM_ROWS + row] += 1;
                let panel = x * PARADE_COLUMNS / width;
                for (channel, value) in [r, g, b].into_iter().enumerate() {
                    let row = value as usize * WAVEFORM_ROWS / 256;
                    scopes.parade[(channel * PARADE_COLUMNS + panel) * WAVEFORM_ROWS + row] += 1;
                }
                let (cb, cr) = chroma_of(r, g, b);
                scopes.vector[cell_of(cb, cr)] += 1;
                scopes.samples += 1;
            }
        }
        Some(scopes)
    }

    /// The share of pixels at pure black and at pure white: crushed shadows
    /// and blown highlights.
    pub fn clipped(&self) -> (f32, f32) {
        let total = self.samples.max(1) as f32;
        (self.luma[0] as f32 / total, self.luma[255] as f32 / total)
    }

    /// The waveform count at `column`, `row`.
    pub fn waveform_at(&self, column: usize, row: usize) -> u32 {
        self.waveform[column * WAVEFORM_ROWS + row]
    }

    /// The parade count for `channel` (0 red, 1 green, 2 blue) at `column`,
    /// `row`. Out of range reads as nothing, so a caller cannot panic the
    /// window by asking about a fourth channel.
    pub fn parade_at(&self, channel: usize, column: usize, row: usize) -> u32 {
        if channel >= PARADE_CHANNELS || column >= PARADE_COLUMNS || row >= WAVEFORM_ROWS {
            return 0;
        }
        self.parade[(channel * PARADE_COLUMNS + column) * WAVEFORM_ROWS + row]
    }

    /// The vectorscope count at `across`, `up` — `up` counted from the bottom,
    /// so it reads the way the scope is drawn.
    pub fn vector_at(&self, across: usize, up: usize) -> u32 {
        let row = VECTOR_CELLS.saturating_sub(1).saturating_sub(up);
        self.vector[row * VECTOR_CELLS + across]
    }

    /// How far the colour reaches from grey, 0–1: the longest arm of the
    /// vectorscope's shape. Zero is a picture with no colour in it at all.
    pub fn saturation_reach(&self) -> f32 {
        let middle = (VECTOR_CELLS as f32 - 1.0) / 2.0;
        let mut reach: f32 = 0.0;
        for (index, count) in self.vector.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            let (x, y) = ((index % VECTOR_CELLS) as f32, (index / VECTOR_CELLS) as f32);
            let distance = ((x - middle).powi(2) + (y - middle).powi(2)).sqrt() / middle;
            reach = reach.max(distance);
        }
        reach.min(1.0)
    }
}

/// What the window keeps between frames.
#[derive(Debug, Default)]
pub struct ScopesState {
    pub open: bool,
    /// The scopes last measured, and the time they were measured at.
    pub current: Option<(TimelineTime, Scopes)>,
    /// A frame asked for and not yet back.
    pub pending: bool,
    /// When the playhead was last seen to move, and where it was.
    pub settling: Option<(TimelineTime, f64)>,
    /// A read to start, picked up by the desktop shell.
    pub request: Option<TimelineTime>,
    /// Show the three channels side by side instead of the luma trace.
    pub parade: bool,
}

impl ScopesState {
    /// Take a rendered frame for the scopes.
    pub fn arrived(&mut self, at: TimelineTime, width: u32, height: u32, rgba: &[u8]) {
        self.pending = false;
        if let Some(scopes) = Scopes::of(rgba, width, height) {
            self.current = Some((at, scopes));
        }
    }

    /// Whether to ask for the frame at `playhead` now, `now` seconds into the
    /// session: the window is open, nothing is in flight, the scopes are not
    /// already for this frame, and the playhead has stayed put for
    /// [`SETTLE_SECONDS`].
    pub fn wants(&mut self, playhead: TimelineTime, now: f64) -> bool {
        if !self.open || self.pending {
            return false;
        }
        if self.current.as_ref().is_some_and(|(at, _)| *at == playhead) {
            self.settling = None;
            return false;
        }
        match self.settling {
            // A hair of tolerance: seconds are floats, and 10.35 - 10.0 is not
            // quite 0.35.
            Some((at, since)) if at == playhead => now - since >= SETTLE_SECONDS - 1e-9,
            _ => {
                self.settling = Some((playhead, now));
                false
            }
        }
    }
}

/// Draw the window, if it is open, and ask for a frame when one is due.
pub fn show(ctx: &egui::Context, editor: &Editor, state: &mut UiState) {
    let now = ctx.input(|i| i.time);
    let playhead = editor.playhead();
    if state.scopes.wants(playhead, now) {
        state.scopes.request = Some(playhead);
        state.scopes.pending = true;
    }
    if !state.scopes.open {
        return;
    }
    if state.scopes.settling.is_some() || state.scopes.pending {
        // Come back to read the frame once the playhead has settled.
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
    let mut open = true;
    let mut parade = state.scopes.parade;
    crate::theme::placed(egui::Window::new("Scopes"), ctx)
        .open(&mut open)
        .default_width(300.0)
        .resizable(false)
        .show(ctx, |ui| {
            let Some((at, scopes)) = &state.scopes.current else {
                ui.label(
                    egui::RichText::new("Reading the frame under the playhead…")
                        .small()
                        .color(theme::disabled()),
                );
                return;
            };
            let stale = *at != playhead;
            ui.label(
                egui::RichText::new(format!(
                    "{}{}",
                    at.format_timecode(),
                    if stale { " · updating" } else { "" }
                ))
                .small()
                .color(theme::disabled()),
            );

            ui.label(egui::RichText::new("Histogram").small().strong());
            let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 90.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2, egui::Color32::from_gray(18));
            // Scaled to the tallest bin away from the two ends, so a frame
            // with a lot of pure black does not flatten everything else.
            let peak = [&scopes.red, &scopes.green, &scopes.blue, &scopes.luma]
                .iter()
                .flat_map(|bins| bins[1..255].iter().copied())
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            for (bins, colour) in [
                (
                    &scopes.red,
                    egui::Color32::from_rgba_unmultiplied(230, 70, 70, 150),
                ),
                (
                    &scopes.green,
                    egui::Color32::from_rgba_unmultiplied(70, 210, 90, 150),
                ),
                (
                    &scopes.blue,
                    egui::Color32::from_rgba_unmultiplied(80, 120, 240, 150),
                ),
                (
                    &scopes.luma,
                    egui::Color32::from_rgba_unmultiplied(235, 235, 235, 200),
                ),
            ] {
                let points: Vec<egui::Pos2> = bins
                    .iter()
                    .enumerate()
                    .map(|(i, &count)| {
                        let x = rect.left() + rect.width() * i as f32 / 255.0;
                        let h = (count as f32 / peak).min(1.0) * rect.height();
                        egui::pos2(x, rect.bottom() - h)
                    })
                    .collect();
                painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, colour)));
            }
            let (crushed, blown) = scopes.clipped();
            if crushed > 0.01 || blown > 0.01 {
                ui.label(
                    egui::RichText::new(format!(
                        "{:.0}% pure black · {:.0}% pure white",
                        crushed * 100.0,
                        blown * 100.0
                    ))
                    .small()
                    .color(theme::error_text()),
                );
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Waveform").small().strong());
                // The same measurement read two ways, so they are one choice
                // rather than two panels fighting for the window.
                if ui
                    .add(egui::Button::selectable(!parade, "Luma"))
                    .on_hover_text("One trace: how bright each part of the picture is")
                    .clicked()
                {
                    parade = false;
                }
                if ui
                    .add(egui::Button::selectable(parade, "RGB parade"))
                    .on_hover_text(
                        "The three channels side by side. A panel sitting higher than the \
                         others is a cast of that colour",
                    )
                    .clicked()
                {
                    parade = true;
                }
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 110.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2, egui::Color32::from_gray(18));
            if parade {
                draw_parade(&painter, rect, scopes);
            } else {
                let rows_per_column = (scopes.samples as f32 / WAVEFORM_COLUMNS as f32).max(1.0);
                let (cell_w, cell_h) = (
                    rect.width() / WAVEFORM_COLUMNS as f32,
                    rect.height() / WAVEFORM_ROWS as f32,
                );
                for column in 0..WAVEFORM_COLUMNS {
                    for row in 0..WAVEFORM_ROWS {
                        let count = scopes.waveform_at(column, row);
                        if count == 0 {
                            continue;
                        }
                        // Brighter where more of the column sits at that level;
                        // a square root so a thin trace still shows.
                        let share = (count as f32 / rows_per_column * 8.0).min(1.0).sqrt();
                        let alpha = (40.0 + 215.0 * share) as u8;
                        let x = rect.left() + column as f32 * cell_w;
                        let y = rect.bottom() - (row + 1) as f32 * cell_h;
                        painter.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell_w, cell_h)),
                            0,
                            egui::Color32::from_rgba_unmultiplied(120, 230, 140, alpha),
                        );
                    }
                }
            }
            // The colour plane: which way the picture leans, and how far.
            // Drawn round, as every vectorscope is, because the reading is an
            // angle and a distance rather than a pair of numbers.
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Vectorscope").small().strong());
                ui.label(
                    egui::RichText::new(format!("reach {:.0}%", scopes.saturation_reach() * 100.0))
                        .small()
                        .color(crate::theme::disabled()),
                )
                .on_hover_text(
                    "How far the strongest colour sits from grey. Near nothing is a flat, \
                     unsaturated picture; near the edge is colour the encoder will struggle with",
                );
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 280.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2, egui::Color32::from_gray(18));

            let middle = rect.center();
            let radius = rect.width().min(rect.height()) / 2.0 - 4.0;
            let graticule = egui::Stroke::new(1.0, egui::Color32::from_gray(60));
            painter.circle_stroke(middle, radius, graticule);
            painter.circle_stroke(middle, radius / 2.0, graticule);
            painter.line_segment(
                [
                    egui::pos2(middle.x - radius, middle.y),
                    egui::pos2(middle.x + radius, middle.y),
                ],
                graticule,
            );
            painter.line_segment(
                [
                    egui::pos2(middle.x, middle.y - radius),
                    egui::pos2(middle.x, middle.y + radius),
                ],
                graticule,
            );
            // The skin-tone line: faces sit along it whatever their colour,
            // which is what makes a white balance checkable rather than a
            // matter of opinion.
            let skin = SKIN_TONE_DEGREES.to_radians();
            painter.line_segment(
                [
                    middle,
                    egui::pos2(
                        middle.x + radius * skin.cos(),
                        middle.y - radius * skin.sin(),
                    ),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 140, 110)),
            );

            let per_cell = (scopes.samples as f32 / 64.0).max(1.0);
            let cell = rect.width() / VECTOR_CELLS as f32;
            for across in 0..VECTOR_CELLS {
                for up in 0..VECTOR_CELLS {
                    let count = scopes.vector_at(across, up);
                    if count == 0 {
                        continue;
                    }
                    let share = (count as f32 / per_cell).min(1.0).sqrt();
                    let alpha = (40.0 + 215.0 * share) as u8;
                    let x = rect.left() + across as f32 * cell;
                    let y = rect.bottom() - (up + 1) as f32 * cell;
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell, cell)),
                        0,
                        egui::Color32::from_rgba_unmultiplied(230, 230, 140, alpha),
                    );
                }
            }
        });
    state.scopes.parade = parade;
    if !open {
        state.scopes.open = false;
    }
}

/// The three panels of the parade across `rect`, each in its own colour.
///
/// Read across a panel the way the picture reads left to right, and up it the
/// way levels go: the three together are one picture taken apart, so what is
/// worth seeing is where they *differ* \u2014 a sky pinning the blue panel to the
/// top while red and green sit low is a cast, not a bright sky.
fn draw_parade(painter: &egui::Painter, rect: egui::Rect, scopes: &Scopes) {
    const GAP: f32 = 4.0;
    const COLOURS: [(u8, u8, u8); PARADE_CHANNELS] =
        [(240, 80, 80), (90, 220, 110), (100, 140, 250)];

    let panel_w = (rect.width() - GAP * (PARADE_CHANNELS as f32 - 1.0)) / PARADE_CHANNELS as f32;
    let rows_per_column = (scopes.samples as f32 / PARADE_COLUMNS as f32).max(1.0);
    let cell_w = panel_w / PARADE_COLUMNS as f32;
    let cell_h = rect.height() / WAVEFORM_ROWS as f32;

    for (channel, (r, g, b)) in COLOURS.into_iter().enumerate() {
        let left = rect.left() + channel as f32 * (panel_w + GAP);
        for column in 0..PARADE_COLUMNS {
            for row in 0..WAVEFORM_ROWS {
                let count = scopes.parade_at(channel, column, row);
                if count == 0 {
                    continue;
                }
                let share = (count as f32 / rows_per_column * 8.0).min(1.0).sqrt();
                let alpha = (40.0 + 215.0 * share) as u8;
                let x = left + column as f32 * cell_w;
                let y = rect.bottom() - (row + 1) as f32 * cell_h;
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell_w, cell_h)),
                    0,
                    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        (0..w * h)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    #[test]
    fn a_solid_frame_is_one_bin() {
        let scopes = Scopes::of(&solid(16, 8, [200, 100, 50]), 16, 8).unwrap();
        assert_eq!(scopes.samples, 128);
        assert_eq!(scopes.red[200], 128);
        assert_eq!(scopes.green[100], 128);
        assert_eq!(scopes.blue[50], 128);
        assert_eq!(scopes.luma.iter().filter(|c| **c > 0).count(), 1);
    }

    /// Each panel measures its own channel: a flat orange frame puts red high,
    /// green in the middle and blue low, in every column.
    #[test]
    fn the_parade_splits_the_frame_into_its_channels() {
        let scopes = Scopes::of(&solid(96, 4, [240, 128, 16]), 96, 4).unwrap();
        let row_of = |value: usize| value * WAVEFORM_ROWS / 256;
        for column in 0..PARADE_COLUMNS {
            assert!(scopes.parade_at(0, column, row_of(240)) > 0, "red {column}");
            assert!(
                scopes.parade_at(1, column, row_of(128)) > 0,
                "green {column}"
            );
            assert!(scopes.parade_at(2, column, row_of(16)) > 0, "blue {column}");
        }
        // Every measured pixel is counted once in each of the three panels.
        assert_eq!(
            scopes.parade.iter().sum::<u32>(),
            scopes.samples * PARADE_CHANNELS as u32
        );
    }

    /// And each panel reads left to right like the picture: red on the left of
    /// the frame stays on the left of the red panel.
    #[test]
    fn a_parade_panel_follows_the_picture_left_to_right() {
        let (w, h) = (192_u32, 4_u32);
        let mut rgba = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let left = x < w / 2;
                rgba.extend_from_slice(&[
                    if left { 250 } else { 10 },
                    10,
                    if left { 10 } else { 250 },
                    255,
                ]);
            }
        }
        let scopes = Scopes::of(&rgba, w, h).unwrap();
        let (low, high) = (10 * WAVEFORM_ROWS / 256, 250 * WAVEFORM_ROWS / 256);
        let last = PARADE_COLUMNS - 1;

        assert!(scopes.parade_at(0, 0, high) > 0 && scopes.parade_at(0, 0, low) == 0);
        assert!(scopes.parade_at(0, last, low) > 0 && scopes.parade_at(0, last, high) == 0);
        assert!(scopes.parade_at(2, 0, low) > 0 && scopes.parade_at(2, 0, high) == 0);
        assert!(scopes.parade_at(2, last, high) > 0);
        // Green never moved, so its panel is one level the whole way across.
        assert!(scopes.parade_at(1, 0, low) > 0 && scopes.parade_at(1, last, low) > 0);
    }

    /// Grey is the one frame where the three panels agree, which is what makes
    /// a parade a white-balance check.
    #[test]
    fn grey_puts_the_three_panels_at_the_same_height() {
        let scopes = Scopes::of(&solid(64, 4, [180, 180, 180]), 64, 4).unwrap();
        let row = 180 * WAVEFORM_ROWS / 256;
        for column in 0..PARADE_COLUMNS {
            let heights: Vec<u32> = (0..PARADE_CHANNELS)
                .map(|channel| scopes.parade_at(channel, column, row))
                .collect();
            assert!(
                heights
                    .iter()
                    .all(|count| *count == heights[0] && *count > 0),
                "column {column} read {heights:?}"
            );
        }
    }

    /// Asking about a channel or a column that does not exist reads as empty
    /// rather than panicking the window.
    #[test]
    fn the_parade_is_safe_to_ask_out_of_range() {
        let scopes = Scopes::of(&solid(8, 8, [100, 100, 100]), 8, 8).unwrap();
        assert_eq!(scopes.parade_at(PARADE_CHANNELS, 0, 0), 0);
        assert_eq!(scopes.parade_at(0, PARADE_COLUMNS, 0), 0);
        assert_eq!(scopes.parade_at(0, 0, WAVEFORM_ROWS), 0);
    }

    #[test]
    fn luma_uses_rec709_weights() {
        assert_eq!(luma_of(255, 255, 255), 255);
        assert_eq!(luma_of(0, 0, 0), 0);
        assert_eq!(luma_of(255, 0, 0), 54);
        assert_eq!(luma_of(0, 255, 0), 182);
        assert_eq!(luma_of(0, 0, 255), 18);
    }

    /// Dark on the left, bright on the right: the waveform's left columns sit
    /// low and its right columns high.
    #[test]
    fn the_waveform_follows_the_picture_left_to_right() {
        let (w, h) = (256_u32, 10_u32);
        let mut rgba = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 20 } else { 230 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let scopes = Scopes::of(&rgba, w, h).unwrap();
        let low = 20 * WAVEFORM_ROWS / 256;
        let high = 230 * WAVEFORM_ROWS / 256;
        assert!(scopes.waveform_at(0, low) > 0 && scopes.waveform_at(0, high) == 0);
        let last = WAVEFORM_COLUMNS - 1;
        assert!(scopes.waveform_at(last, high) > 0 && scopes.waveform_at(last, low) == 0);
        assert_eq!(scopes.waveform.iter().sum::<u32>(), scopes.samples);
    }

    /// Grey has no colour in it, so it lands dead centre — whatever its
    /// brightness. The one reading a vectorscope has to get right.
    #[test]
    fn grey_sits_in_the_middle_at_any_brightness() {
        let middle = VECTOR_CELLS / 2;
        for level in [0_u8, 64, 128, 200, 255] {
            let scopes = Scopes::of(&solid(8, 8, [level, level, level]), 8, 8).unwrap();
            assert_eq!(
                scopes.vector_at(middle, middle),
                64,
                "grey at {level} left the middle"
            );
            assert!(
                scopes.saturation_reach() < 0.05,
                "grey at {level} reads as coloured"
            );
        }
    }

    /// Red is up, blue is right, and green is the other way from both: the
    /// axes are Cb across and Cr up, as the plane has always been drawn.
    #[test]
    fn the_primaries_land_on_their_own_sides() {
        let middle = (VECTOR_CELLS / 2) as i64;
        let place = |rgb: [u8; 3]| {
            let (cb, cr) = chroma_of(rgb[0], rgb[1], rgb[2]);
            let index = cell_of(cb, cr);
            (
                (index % VECTOR_CELLS) as i64 - middle,
                middle - (index / VECTOR_CELLS) as i64,
            )
        };

        let (across, up) = place([255, 0, 0]);
        assert!(up > 10 && across < 0, "red landed at {across},{up}");
        let (across, up) = place([0, 0, 255]);
        assert!(across > 10 && up.abs() < 10, "blue landed at {across},{up}");
        let (across, up) = place([0, 255, 0]);
        assert!(across < -5 && up < -5, "green landed at {across},{up}");
    }

    /// The further the colour, the further out the scope draws it — which is
    /// what "reach" reports.
    #[test]
    fn stronger_colour_reaches_further_from_the_middle() {
        let flat = Scopes::of(&solid(8, 8, [140, 130, 130]), 8, 8).unwrap();
        let strong = Scopes::of(&solid(8, 8, [255, 40, 40]), 8, 8).unwrap();

        assert!(
            flat.saturation_reach() < strong.saturation_reach(),
            "{} then {}",
            flat.saturation_reach(),
            strong.saturation_reach()
        );
        assert!(strong.saturation_reach() > 0.3);
    }

    /// Skin sits along the line the scope draws for it, whatever the face.
    #[test]
    fn skin_tones_land_along_the_skin_line() {
        for skin in [[255_u8, 205, 170], [190, 140, 110], [120, 80, 60]] {
            let (cb, cr) = chroma_of(skin[0], skin[1], skin[2]);
            let degrees = cr.atan2(cb).to_degrees();
            assert!(
                (degrees - SKIN_TONE_DEGREES).abs() < 20.0,
                "{skin:?} sits at {degrees:.0}°, not near {SKIN_TONE_DEGREES}°"
            );
        }
    }

    /// Every pixel is counted exactly once, as the waveform's are.
    #[test]
    fn the_plane_counts_every_pixel() {
        let (w, h) = (32_u32, 16_u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[(x * 8) as u8, (y * 16) as u8, 90, 255]);
            }
        }
        let scopes = Scopes::of(&rgba, w, h).unwrap();
        assert_eq!(scopes.vector.iter().sum::<u32>(), scopes.samples);
    }

    /// Colour past what the plane holds is held to its edge rather than
    /// landing somewhere else on it.
    #[test]
    fn colour_past_the_edge_stays_at_the_edge() {
        let last = VECTOR_CELLS - 1;
        assert_eq!(cell_of(9.0, 0.0) % VECTOR_CELLS, last);
        assert_eq!(cell_of(-9.0, 0.0) % VECTOR_CELLS, 0);
        assert_eq!(cell_of(0.0, 9.0) / VECTOR_CELLS, 0);
        assert_eq!(cell_of(0.0, -9.0) / VECTOR_CELLS, last);
    }

    #[test]
    fn clipping_is_counted_at_the_ends() {
        let mut rgba = solid(10, 10, [0, 0, 0]);
        rgba.extend(solid(10, 10, [255, 255, 255]));
        let scopes = Scopes::of(&rgba, 10, 20).unwrap();
        assert_eq!(scopes.clipped(), (0.5, 0.5));
        assert!(Scopes::of(&rgba[..10], 10, 20).is_none());
    }

    #[test]
    fn a_big_frame_is_read_every_other_pixel() {
        let scopes = Scopes::of(&solid(1920, 1080, [9, 9, 9]), 1920, 1080).unwrap();
        assert_eq!(scopes.samples, 960 * 540);
    }

    /// A read waits for the playhead to settle, is asked for once, and not
    /// again while it is in flight or once it is current.
    #[test]
    fn reads_wait_for_the_playhead_to_settle() {
        let mut state = ScopesState {
            open: true,
            ..ScopesState::default()
        };
        let at = TimelineTime::from_seconds(2);
        assert!(!state.wants(at, 10.0), "read before settling");
        assert!(!state.wants(at, 10.1));
        assert!(state.wants(at, 10.0 + SETTLE_SECONDS));
        state.pending = true;
        assert!(!state.wants(at, 11.0), "asked again while in flight");
        state.arrived(at, 4, 4, &solid(4, 4, [1, 2, 3]));
        assert!(!state.pending);
        assert!(!state.wants(at, 12.0), "asked again for the frame it has");
        // Moving on starts the wait again.
        let later = TimelineTime::from_seconds(3);
        assert!(!state.wants(later, 12.0));
        assert!(state.wants(later, 12.0 + SETTLE_SECONDS));

        state.open = false;
        assert!(
            !state.wants(TimelineTime::from_seconds(9), 99.0),
            "read while closed"
        );
    }
}
