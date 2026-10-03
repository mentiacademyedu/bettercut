//! The sound in detail: one clip's waveform, as tall as a window rather than
//! as tall as a lane.
//!
//! The timeline draws a waveform a few pixels high inside a clip, which is
//! enough to see *where* the speech is and nothing like enough to see where a
//! word starts, whether a take is clipping, or where the breath before a line
//! could be cut. Those are the questions this answers: the same cached peaks
//! (`bettercut_cache::waveform`), drawn tall, zoomable around the playhead,
//! with the level scale marked and the playhead where it really is.
//!
//! # It reads, it does not edit
//!
//! Clicking moves the playhead, and nothing else here changes the project. The
//! edit happens on the timeline; this is the magnifying glass held over it, and
//! a second place that could trim a clip would be a second place to keep in
//! step with the first.

use bettercut_cache::Waveform;
use bettercut_editor_core::Editor;
use bettercut_editor_core::foundation::{ClipId, MediaTime, TICKS_PER_SECOND, TimelineTime};
use egui::{Pos2, Sense, Stroke, vec2};

use crate::state::UiState;
use crate::theme;

/// The tallest zoom: a second of sound across the whole window, which is about
/// as far as peaks at 200 a second can honestly be stretched.
pub const MAX_ZOOM: f32 = 60.0;

/// The window's state: whether it is open, and how far in it is zoomed.
#[derive(Debug, Clone, Copy)]
pub struct WaveformView {
    pub open: bool,
    /// 1.0 shows the whole clip; 10.0 shows a tenth of it, around the
    /// playhead.
    pub zoom: f32,
}

impl Default for WaveformView {
    fn default() -> Self {
        Self {
            open: false,
            zoom: 1.0,
        }
    }
}

/// The stretch of a clip's *source* the view shows: the whole of it at zoom
/// 1.0, and a window that much smaller around `at` as it goes in.
///
/// Held inside the clip at both ends, so zooming near an edge slides the
/// window rather than showing nothing beyond it.
pub fn window_for(
    source: bettercut_editor_core::timeline::SourceRange,
    at: MediaTime,
    zoom: f32,
) -> (MediaTime, MediaTime) {
    let span = (source.end.ticks() - source.start.ticks()).max(1);
    let zoom = zoom.clamp(1.0, MAX_ZOOM);
    let shown = ((span as f64 / f64::from(zoom)) as i64).max(1);
    let half = shown / 2;
    let centre = at.ticks().clamp(source.start.ticks(), source.end.ticks());
    let mut from = centre - half;
    if from < source.start.ticks() {
        from = source.start.ticks();
    }
    if from + shown > source.end.ticks() {
        from = (source.end.ticks() - shown).max(source.start.ticks());
    }
    (
        MediaTime::from_ticks(from),
        MediaTime::from_ticks(from + shown),
    )
}

/// The peaks to draw, one pair a pixel column, each -1.0 to 1.0.
///
/// Pure arithmetic, apart from the peaks themselves: this is where an off-by-
/// one in the mapping would show as a waveform that does not line up with the
/// sound, so it is worth being able to test without a window.
pub fn columns(
    waveform: &Waveform,
    from: MediaTime,
    to: MediaTime,
    width: usize,
) -> Vec<(f32, f32)> {
    if width == 0 || waveform.peaks.is_empty() {
        return Vec::new();
    }
    let per_second = f64::from(waveform.peaks_per_second);
    let bucket = |ticks: i64| -> usize {
        ((ticks.max(0) as f64 / TICKS_PER_SECOND as f64) * per_second) as usize
    };
    let span = (to.ticks() - from.ticks()).max(1);

    (0..width)
        .map(|column| {
            let start = from.ticks() + span * column as i64 / width as i64;
            let end = from.ticks() + span * (column as i64 + 1) / width as i64;
            let (low, high) = (bucket(start), bucket(end).max(bucket(start) + 1));
            let peak = waveform.peak_over(low, high);
            (f32::from(peak.min) / 127.0, f32::from(peak.max) / 127.0)
        })
        .collect()
}

/// The loudest sample in what is shown, in dBFS — the number that answers "is
/// this take too hot". `None` for silence, which has no level.
pub fn peak_dbfs(columns: &[(f32, f32)]) -> Option<f32> {
    let loudest = columns
        .iter()
        .map(|(min, max)| min.abs().max(max.abs()))
        .fold(0.0_f32, f32::max);
    (loudest > 0.0).then(|| 20.0 * loudest.log10())
}

/// The clip the view is about: the selected sound clip, or the one under the
/// playhead when nothing is selected.
fn subject(editor: &Editor, state: &UiState) -> Option<ClipId> {
    let selected = state
        .selected_clips
        .iter()
        .copied()
        .find(|clip| editor.audio_clip(*clip).is_some());
    selected.or_else(|| {
        let at = editor.playhead();
        editor
            .active_sequence()?
            .audio_tracks
            .iter()
            .find_map(|track| track.clip_at(at).map(|clip| clip.id))
    })
}

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut UiState) {
    if !state.waveform_view.open {
        return;
    }
    let mut open = true;
    crate::theme::placed(egui::Window::new("Sound in Detail"), ctx)
        .open(&mut open)
        .default_width(620.0)
        .resizable(true)
        .show(ctx, |ui| {
            body(ui, editor, state);
        });
    if !open {
        state.waveform_view.open = false;
    }
}

fn body(ui: &mut egui::Ui, editor: &mut Editor, state: &mut UiState) {
    let Some(clip) = subject(editor, state) else {
        ui.label(
            egui::RichText::new("Select a sound clip, or put the playhead over one.")
                .color(theme::disabled()),
        );
        return;
    };
    let Some(audio) = editor.audio_clip(clip).cloned() else {
        return;
    };
    let Some(peaks) = state.waveforms.get(audio.media_id) else {
        ui.label(
            egui::RichText::new("Reading the sound…")
                .small()
                .color(theme::disabled()),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        return;
    };

    let playhead = editor.playhead();
    let inside = playhead >= audio.timeline.start && playhead < audio.timeline.end;
    let at = audio.source_time_at(if inside {
        playhead
    } else {
        audio.timeline.start
    });
    let (from, to) = window_for(audio.source, at, state.waveform_view.zoom);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(
                editor
                    .project()
                    .media_asset(audio.media_id)
                    .map_or_else(|| "sound".to_owned(), |a| a.display_name().to_owned()),
            )
            .strong(),
        );
        ui.add(
            egui::Slider::new(&mut state.waveform_view.zoom, 1.0..=MAX_ZOOM)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{v:.0}×"))
                .text("zoom"),
        );
    });

    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 220.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2, egui::Color32::from_gray(16));

    let columns = columns(&peaks, from, to, rect.width() as usize);
    let centre = rect.center().y;
    let half = (rect.height() - 24.0) / 2.0;

    // The level scale: silence through the middle, and the two lines every
    // recordist reads a take against.
    for (amplitude, label) in [(1.0_f32, "0"), (0.5, "-6"), (0.251, "-12")] {
        for side in [-1.0_f32, 1.0] {
            let y = centre - side * amplitude * half;
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, theme::grid_line()),
            );
        }
        painter.text(
            Pos2::new(rect.left() + 4.0, centre - amplitude * half),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(10.0),
            theme::disabled(),
        );
    }

    for (index, (min, max)) in columns.iter().enumerate() {
        let x = rect.left() + index as f32;
        let top = centre - max.abs().max(0.004) * half;
        let bottom = centre + min.abs().max(0.004) * half;
        painter.line_segment(
            [Pos2::new(x, top), Pos2::new(x, bottom)],
            Stroke::new(1.0, theme::audio_clip_top()),
        );
    }

    // The volume the mixer would apply across what is shown, so a duck is
    // visible against the sound it is ducking (§24, §46).
    let level_line: Vec<Pos2> = (0..=rect.width() as usize)
        .step_by(3)
        .map(|step| {
            let x = rect.left() + step as f32;
            let source = MediaTime::from_ticks(
                from.ticks()
                    + (to.ticks() - from.ticks()) * step as i64 / rect.width().max(1.0) as i64,
            );
            let at = timeline_time_of(&audio, source);
            let gain = audio.gain_at(at).clamp(0.0, 1.0);
            Pos2::new(x, centre - gain * half)
        })
        .collect();
    if level_line.len() > 1 {
        painter.add(egui::Shape::line(
            level_line,
            Stroke::new(1.5, theme::automation()),
        ));
    }

    // The playhead, where it really is — and nothing when it is off the clip,
    // rather than a line pinned to an edge it is not at.
    if inside {
        let along = (at.ticks() - from.ticks()) as f32 / (to.ticks() - from.ticks()).max(1) as f32;
        if (0.0..=1.0).contains(&along) {
            let x = rect.left() + along * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, theme::playhead()),
            );
        }
    }

    // Clicking and dragging inside moves the playhead: the whole point of
    // looking this closely is to put it somewhere exact.
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let along = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let source = MediaTime::from_ticks(
            from.ticks() + ((to.ticks() - from.ticks()) as f32 * along) as i64,
        );
        editor.set_playhead(timeline_time_of(&audio, source));
        state.needs_repaint = true;
    }

    ui.horizontal(|ui| {
        let shown = (to.ticks() - from.ticks()) as f64 / TICKS_PER_SECOND as f64;
        ui.label(
            egui::RichText::new(format!("{shown:.2} s shown"))
                .small()
                .color(theme::disabled()),
        );
        match peak_dbfs(&columns) {
            Some(peak) => {
                let hot = peak > -1.0;
                ui.label(
                    egui::RichText::new(format!("peak {peak:.1} dBFS"))
                        .small()
                        .color(if hot {
                            theme::caution()
                        } else {
                            theme::disabled()
                        }),
                )
                .on_hover_text(if hot {
                    "This close to zero, the loudest moments may already be clipped in the file"
                } else {
                    "The loudest sample in what is shown"
                });
            }
            None => {
                ui.label(
                    egui::RichText::new("silence here")
                        .small()
                        .color(theme::disabled()),
                );
            }
        }
    });
}

/// Back from a source instant to where it falls on the timeline, through the
/// clip's speed — the inverse of [`AudioClip::source_time_at`].
fn timeline_time_of(
    clip: &bettercut_editor_core::timeline::AudioClip,
    source: MediaTime,
) -> TimelineTime {
    let into_source = source.ticks() - clip.source.start.ticks();
    let into_clip = bettercut_editor_core::timeline::timeline_ticks_for(
        MediaTime::from_ticks(into_source.max(0)),
        clip.speed,
    );
    let ticks = if clip.reversed {
        clip.timeline.end.ticks() - into_clip
    } else {
        clip.timeline.start.ticks() + into_clip
    };
    TimelineTime::from_ticks(ticks.clamp(clip.timeline.start.ticks(), clip.timeline.end.ticks()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_cache::Peak;
    use bettercut_editor_core::timeline::SourceRange;

    fn source(from: i64, to: i64) -> SourceRange {
        SourceRange::new(MediaTime::from_seconds(from), MediaTime::from_seconds(to))
            .expect("a range")
    }

    /// Ten seconds of peaks: silence, then a loud second in the middle.
    fn peaks() -> Waveform {
        let per_second = 200;
        let mut peaks = vec![Peak { min: -4, max: 4 }; 10 * per_second as usize];
        for bucket in peaks
            .iter_mut()
            .skip(5 * per_second as usize)
            .take(per_second as usize)
        {
            *bucket = Peak {
                min: -120,
                max: 120,
            };
        }
        Waveform::new(per_second, peaks).expect("a waveform")
    }

    #[test]
    fn at_zoom_one_the_whole_clip_is_shown() {
        let (from, to) = window_for(source(0, 10), MediaTime::from_seconds(3), 1.0);
        assert_eq!(from, MediaTime::ZERO);
        assert_eq!(to, MediaTime::from_seconds(10));
    }

    #[test]
    fn zooming_in_centres_on_where_the_playhead_is() {
        let (from, to) = window_for(source(0, 10), MediaTime::from_seconds(5), 10.0);
        // A tenth of ten seconds, around five.
        assert!((to.ticks() - from.ticks() - MediaTime::from_seconds(1).ticks()).abs() < 2);
        let centre = (from.ticks() + to.ticks()) / 2;
        assert!(
            (centre - MediaTime::from_seconds(5).ticks()).abs() < MediaTime::from_millis(1).ticks(),
            "the window is not around the playhead"
        );
    }

    /// Near an edge the window slides rather than hanging off the clip: there
    /// is nothing out there to draw.
    #[test]
    fn zooming_near_an_edge_keeps_the_window_inside_the_clip() {
        let clip = source(2, 12);
        let (from, to) = window_for(clip, MediaTime::from_seconds(2), 10.0);
        assert_eq!(from, MediaTime::from_seconds(2));
        assert!(to <= MediaTime::from_seconds(12));

        let (from, to) = window_for(clip, MediaTime::from_seconds(12), 10.0);
        assert!(from >= MediaTime::from_seconds(2));
        assert_eq!(to, MediaTime::from_seconds(12));
    }

    /// The loud second lands in the middle of the picture, which is the one
    /// thing a waveform has to get right.
    #[test]
    fn the_columns_put_the_sound_where_it_happens() {
        let width = 100;
        let drawn = columns(
            &peaks(),
            MediaTime::ZERO,
            MediaTime::from_seconds(10),
            width,
        );
        assert_eq!(drawn.len(), width);

        let loudest = drawn
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
            .expect("a column")
            .0;
        assert!(
            (50..60).contains(&loudest),
            "the loud second drew at column {loudest} of {width}"
        );
        // And the quiet part is quiet, not silent: a flat gap reads as missing
        // data rather than as a quiet passage.
        assert!(drawn[0].1 > 0.0 && drawn[0].1 < 0.1, "{:?}", drawn[0]);
    }

    /// Zoomed into the loud second, every column is loud.
    #[test]
    fn zooming_in_draws_what_is_under_the_window() {
        let drawn = columns(
            &peaks(),
            MediaTime::from_seconds(5),
            MediaTime::from_seconds(6),
            40,
        );
        assert!(
            drawn.iter().all(|(_, max)| *max > 0.9),
            "a column inside the loud second came out quiet: {drawn:?}"
        );
    }

    #[test]
    fn the_peak_is_read_in_decibels() {
        let full = peak_dbfs(&[(-1.0, 1.0)]).expect("a level");
        assert!(full.abs() < 0.1, "{full}");
        let half = peak_dbfs(&[(-0.5, 0.5)]).expect("a level");
        assert!((half + 6.02).abs() < 0.1, "{half}");
        assert_eq!(peak_dbfs(&[(0.0, 0.0)]), None, "silence has no level");
        assert_eq!(peak_dbfs(&[]), None);
    }

    /// An empty window draws nothing rather than panicking on a zero width.
    #[test]
    fn nothing_to_draw_is_not_a_failure() {
        assert!(columns(&peaks(), MediaTime::ZERO, MediaTime::from_seconds(1), 0).is_empty());
        let empty = Waveform::new(200, Vec::new()).expect("a waveform");
        assert!(columns(&empty, MediaTime::ZERO, MediaTime::from_seconds(1), 10).is_empty());
    }
}
