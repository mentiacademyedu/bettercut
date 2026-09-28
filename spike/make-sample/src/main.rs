//! Builds a sample project for the website screenshots.
//!
//! Usage: make-sample <out.vproj> <media files...>
//! Lays each file's middle section on V1 end to end, and the first file's
//! audio under the whole cut on A1.

use std::path::{Path, PathBuf};

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, SourceRange, VideoClip};

fn main() {
    let mut args = std::env::args().skip(1);
    let out: PathBuf = args.next().expect("usage: make-sample <out.vproj> <media...>").into();
    let files: Vec<String> = args.collect();
    assert!(!files.is_empty(), "give at least one media file");

    let prober = FfmpegProber;
    let mut project = Project::new("launch promo");

    let mut cursor = TimelineTime::ZERO;
    let mut first: Option<(bettercut_foundation::MediaId, MediaTime)> = None;

    for file in &files {
        let asset = prober.probe(Path::new(file)).expect("probe media");
        let duration = asset.duration;
        let id = project.add_media(asset);
        if first.is_none() {
            first = Some((id, duration));
        }

        // A slice from the middle of the file: skip the first quarter, take up
        // to six seconds (or half the file, whichever is shorter).
        let start = MediaTime::from_ticks(duration.ticks() / 4);
        let six = MediaTime::from_seconds(6);
        let length = MediaTime::from_ticks(six.ticks().min(duration.ticks() / 2).max(1));
        let source = SourceRange::new(start, MediaTime::from_ticks(start.ticks() + length.ticks()))
            .expect("source range");

        let clip = VideoClip::new(id, cursor, source).expect("video clip");
        let end = clip.timeline.end;
        project.active_mut().expect("sequence").video_tracks[0]
            .insert(clip)
            .expect("insert video clip");
        cursor = end;
    }

    // The first file's audio as a bed under the whole cut.
    if let Some((id, duration)) = first {
        let total = MediaTime::from_ticks(cursor.ticks());
        let end = MediaTime::from_ticks(duration.ticks().min(total.ticks()).max(1));
        let source = SourceRange::new(MediaTime::ZERO, end).expect("audio range");
        let clip = AudioClip::new(id, TimelineTime::ZERO, source).expect("audio clip");
        project.active_mut().expect("sequence").audio_tracks[0]
            .insert(clip)
            .expect("insert audio clip");
    }

    bettercut_project_format::save(&project, &out).expect("save project");
    println!("saved {} — {} clips", out.display(), project.clip_count());
}
