//! Playing a baked stretch back, and knowing when it has stopped being the
//! edit (`bettercut_playback::rendered`, `bettercut_timeline::render`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{LayerSource, layer_requests, rendered};
use bettercut_project_format::Project;
use bettercut_timeline::{RenderedRange, SourceRange, TimelineRange, VideoClip};

fn seconds(n: i64) -> TimelineTime {
    TimelineTime::from_seconds(n)
}

fn range(from: i64, to: i64) -> TimelineRange {
    TimelineRange::new(seconds(from), seconds(to)).expect("non-empty")
}

/// Two four-second shots butted together, with the first four seconds baked
/// into a file.
fn baked_project() -> (Project, RenderedRange) {
    let mut project = Project::new("Renders");
    let media = project.add_media(MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    ));
    let mut bake = MediaAsset::new(
        MediaKind::Video,
        "C:/renders/first.mp4",
        MediaTime::from_seconds(4),
    );
    bake.baked = true;
    let baked_media = project.add_media(bake);

    let sequence = project.active_mut().expect("sequence");
    for start in [0, 4] {
        sequence.video_tracks[0]
            .insert(
                VideoClip::new(
                    media,
                    seconds(start),
                    SourceRange::new(
                        MediaTime::from_seconds(start),
                        MediaTime::from_seconds(start + 4),
                    )
                    .expect("range"),
                )
                .expect("valid"),
            )
            .expect("room");
    }

    let record = RenderedRange {
        range: range(0, 4),
        media: baked_media,
        clip: ClipId::new(),
        fingerprint: 0,
    };
    // The hash of the edit as it stands, which is what a finished bake would
    // have been recorded with.
    let fingerprint =
        rendered::fingerprint(&project, project.active().expect("sequence"), record.range);
    let record = RenderedRange {
        fingerprint,
        ..record
    };
    project.active_mut().expect("sequence").renders = vec![record.clone()];
    (project, record)
}

#[test]
fn a_baked_stretch_plays_as_one_layer_from_its_file() {
    let (project, record) = baked_project();
    let sequence = project.active().expect("sequence");

    let layers = layer_requests(&project, sequence, seconds(2));
    assert_eq!(layers.len(), 1, "the bake is the whole picture");
    assert_eq!(layers[0].source, LayerSource::Media(record.media));
    assert_eq!(layers[0].clip, record.clip, "the bake keeps one clip id");
    // Two seconds into the stretch is two seconds into the file.
    assert_eq!(layers[0].source_time, MediaTime::from_seconds(2));
}

#[test]
fn outside_the_stretch_the_edit_is_composited_as_always() {
    let (project, _record) = baked_project();
    let sequence = project.active().expect("sequence");

    let layers = layer_requests(&project, sequence, seconds(6));
    assert_eq!(layers.len(), 1);
    assert!(
        matches!(layers[0].source, LayerSource::Media(id) if id != _record.media),
        "the second shot should come from its own file"
    );
}

/// The point of the hash: change what the stretch is made of and the file
/// stops being the picture.
#[test]
fn an_edit_inside_the_stretch_makes_the_bake_stale() {
    let (mut project, record) = baked_project();
    {
        let sequence = project.active_mut().expect("sequence");
        let clip = sequence.video_tracks[0].clips()[0].id;
        let first = sequence.video_tracks[0].get_mut(clip).expect("clip");
        first.blur = 0.5;
    }
    let sequence = project.active().expect("sequence");
    assert!(rendered::usable(&project, sequence, seconds(2)).is_none());
    assert_ne!(
        rendered::fingerprint(&project, sequence, record.range),
        record.fingerprint
    );
    // And the preview goes back to compositing it.
    let layers = layer_requests(&project, sequence, seconds(2));
    assert!(matches!(layers[0].source, LayerSource::Media(id) if id != record.media));
}

/// And the other half of the point: an edit somewhere else leaves it alone,
/// or every cut made after a render would throw the render away.
#[test]
fn an_edit_outside_the_stretch_leaves_the_bake_alone() {
    let (mut project, record) = baked_project();
    {
        let sequence = project.active_mut().expect("sequence");
        let clip = sequence.video_tracks[0].clips()[1].id;
        let second = sequence.video_tracks[0].get_mut(clip).expect("clip");
        second.blur = 0.5;
    }
    let sequence = project.active().expect("sequence");
    assert_eq!(
        rendered::fingerprint(&project, sequence, record.range),
        record.fingerprint
    );
    assert!(rendered::usable(&project, sequence, seconds(2)).is_some());
}

/// The master look is in every frame of the bake, so changing it makes every
/// bake stale.
#[test]
fn a_master_change_makes_the_bake_stale() {
    let (mut project, _record) = baked_project();
    project.active_mut().expect("sequence").master.blur = 0.3;
    let sequence = project.active().expect("sequence");
    assert!(rendered::usable(&project, sequence, seconds(2)).is_none());
}

/// A bake whose file has been taken out of the project is not played: a layer
/// reading media that is not there draws black, which is worse than slow.
#[test]
fn a_bake_whose_media_has_gone_is_not_played() {
    let (mut project, record) = baked_project();
    project.remove_media(record.media).expect("not in use");
    let sequence = project.active().expect("sequence");
    assert!(rendered::usable(&project, sequence, seconds(2)).is_none());
    assert_eq!(layer_requests(&project, sequence, seconds(2)).len(), 1);
}

/// Sound is left out of the hash: a bake holds picture, and re-cutting the
/// music must not throw away ten minutes of rendering.
#[test]
fn a_sound_edit_leaves_the_bake_alone() {
    let (mut project, record) = baked_project();
    {
        let sequence = project.active_mut().expect("sequence");
        let media = sequence.video_tracks[0].clips()[0].media_id;
        sequence.audio_tracks[0]
            .insert(
                bettercut_timeline::AudioClip::new(
                    media,
                    TimelineTime::ZERO,
                    SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).expect("range"),
                )
                .expect("valid"),
            )
            .expect("room");
    }
    let sequence = project.active().expect("sequence");
    assert_eq!(
        rendered::fingerprint(&project, sequence, record.range),
        record.fingerprint
    );
}
