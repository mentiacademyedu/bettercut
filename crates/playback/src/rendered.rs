//! Whether a baked stretch of the edit is still the edit
//! (`bettercut_timeline::render`).
//!
//! A bake is a file of frames, and a file cannot notice that the clips it was
//! made from have changed. So every bake carries a hash of what it was made
//! from, and this is where that hash is taken: the clips under the stretch,
//! the lanes they are on, the master look, the shape and rate of the sequence,
//! the files behind the clips, and — through a compound — the sequence inside.
//! If the hash still matches, the file is still the picture; if it does not,
//! the plan goes back to compositing the edit.
//!
//! # What is deliberately left out
//!
//! The sound, unless the sequence has a visualizer. A bake holds picture only,
//! so re-cutting the music should not throw away ten minutes of rendering —
//! but bars that jump with the sound *are* the picture, and those move when
//! the sound does.
//!
//! # Cost
//!
//! The hash is taken once per planned frame, and only for a position a bake
//! covers — everywhere else the check is one comparison against an empty list.
//! It serializes the clips of the stretch, which is microseconds for the
//! handful of clips an instant sits on, against the milliseconds of
//! compositing it saves.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use bettercut_foundation::{TimelineTime, TrackId};
use bettercut_project_format::Project;
use bettercut_timeline::{RenderedRange, Sequence, TimelineRange};
use serde::Serialize;

/// The bake to play at `position` instead of compositing, if there is one and
/// it is still current.
pub fn usable<'a>(
    project: &Project,
    sequence: &'a Sequence,
    position: TimelineTime,
) -> Option<&'a RenderedRange> {
    let render = sequence.render_at(position)?;
    // The file has to still be in the project: a bake whose media was purged
    // would plan a layer that decodes to nothing, which draws as black — the
    // one failure worse than being slow.
    project.media_asset(render.media)?;
    (fingerprint(project, sequence, render.range) == render.fingerprint).then_some(render)
}

/// What the picture under `range` is made of, as one number.
pub fn fingerprint(project: &Project, sequence: &Sequence, range: TimelineRange) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_into(&mut hasher, project, sequence, range, 0);
    hasher.finish()
}

/// One lane's part in the picture: what it is, and the clips of it that the
/// stretch actually touches.
#[derive(Serialize)]
struct Lane<'a, C> {
    id: TrackId,
    enabled: bool,
    solo: bool,
    clips: Vec<&'a C>,
}

fn lanes<'a, C: bettercut_timeline::Clip + Serialize>(
    tracks: &'a [bettercut_timeline::Track<C>],
    range: TimelineRange,
) -> Vec<Lane<'a, C>> {
    tracks
        .iter()
        .map(|track| Lane {
            id: track.id,
            enabled: track.enabled,
            solo: track.solo,
            clips: track
                .clips()
                .iter()
                .filter(|clip| clip.timeline().overlaps(range))
                .collect(),
        })
        .collect()
}

fn hash_into(
    hasher: &mut DefaultHasher,
    project: &Project,
    sequence: &Sequence,
    range: TimelineRange,
    depth: u8,
) {
    #[derive(Serialize)]
    struct Ingredients<'a> {
        resolution: bettercut_timeline::Resolution,
        frame_rate: bettercut_foundation::FrameRate,
        master: &'a bettercut_timeline::MasterLook,
        visualizer: &'a Option<bettercut_timeline::visualizer::Visualizer>,
        watermark: &'a Option<bettercut_timeline::watermark::Watermark>,
        markers: Vec<&'a bettercut_timeline::Marker>,
        video: Vec<Lane<'a, bettercut_timeline::VideoClip>>,
        text: Vec<Lane<'a, bettercut_timeline::TextClip>>,
        adjustments: Vec<Lane<'a, bettercut_timeline::adjustment::AdjustmentClip>>,
        /// Only when something on screen is made of sound; see the module note.
        audio: Vec<Lane<'a, bettercut_timeline::AudioClip>>,
        /// The files behind the clips: a shot relinked to another take is a
        /// different picture from clips that did not change at all.
        media: Vec<&'a bettercut_media::MediaAsset>,
        /// A grade can name a lookup table, and the table lives in a file.
        luts: &'a [bettercut_project_format::LutAsset],
    }

    let video = lanes(&sequence.video_tracks, range);
    let media: Vec<&bettercut_media::MediaAsset> = video
        .iter()
        .flat_map(|lane| lane.clips.iter())
        .filter_map(|clip| project.media_asset(clip.media_id))
        .collect();

    let ingredients = Ingredients {
        resolution: sequence.resolution,
        frame_rate: sequence.frame_rate,
        master: &sequence.master,
        visualizer: &sequence.visualizer,
        watermark: &sequence.watermark,
        markers: sequence
            .markers
            .iter()
            .filter(|marker| range.contains(marker.time))
            .collect(),
        text: lanes(&sequence.text_tracks, range),
        adjustments: lanes(&sequence.adjustment_tracks, range),
        audio: if sequence.visualizer.is_some() {
            lanes(&sequence.audio_tracks, range)
        } else {
            Vec::new()
        },
        media,
        luts: &project.luts,
        video,
    };

    if let Ok(bytes) = serde_json::to_vec(&ingredients) {
        hasher.write(&bytes);
    }

    // A compound plays a sequence of its own, and that sequence is as much
    // part of this picture as the clips are.
    if depth >= crate::compound::MAX_DEPTH {
        return;
    }
    for lane in &ingredients.video {
        for clip in &lane.clips {
            let Some(inner) = project
                .media_asset(clip.media_id)
                .and_then(|asset| asset.generated)
                .and_then(|generated| generated.compound())
                .and_then(|id| project.sequence(id))
            else {
                continue;
            };
            let whole = TimelineRange {
                start: TimelineTime::ZERO,
                end: inner.duration(),
            };
            hash_into(hasher, project, inner, whole, depth + 1);
        }
    }
}
