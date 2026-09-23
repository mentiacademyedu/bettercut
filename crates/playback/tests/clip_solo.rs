//! Clip solo in the plans (§46: the same plans the export renders): a soloed
//! picture clip is the only layer, a soloed sound clip the only one mixed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::{MediaTime, TimelineTime};
use bettercut_media::{MediaAsset, MediaKind};
use bettercut_playback::{AudioPlan, adjustments_at, layer_requests};
use bettercut_project_format::Project;
use bettercut_timeline::{AudioClip, AudioTrack, SourceRange, VideoClip, VideoTrack};

/// Two video tracks and two sound tracks, one four-second clip on each.
fn stacked() -> Project {
    let mut project = Project::new("Solo");
    let mut asset = MediaAsset::new(
        MediaKind::Video,
        "C:/media/a.mp4",
        MediaTime::from_seconds(60),
    )
    .with_audio(48_000, 2);
    asset.width = 1920;
    asset.height = 1080;
    let media = project.add_media(asset);

    let sequence = project.active_mut().unwrap();
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(4)).unwrap();
    sequence.video_tracks.push(VideoTrack::new("V2"));
    sequence.audio_tracks.push(AudioTrack::new("A2"));
    for track in &mut sequence.video_tracks {
        track
            .insert(VideoClip::new(media, TimelineTime::ZERO, source).unwrap())
            .unwrap();
    }
    for track in &mut sequence.audio_tracks {
        track
            .insert(AudioClip::new(media, TimelineTime::ZERO, source).unwrap())
            .unwrap();
    }
    project
}

#[test]
fn a_soloed_picture_clip_is_the_only_layer() {
    let mut project = stacked();
    let at = TimelineTime::from_seconds(1);
    assert_eq!(
        layer_requests(&project, project.active().unwrap(), at).len(),
        2
    );

    let lower = project.active().unwrap().video_tracks[0].clips()[0].id;
    project.active_mut().unwrap().set_clip_solo(lower, true);
    let layers = layer_requests(&project, project.active().unwrap(), at);
    assert_eq!(layers.len(), 1, "only the soloed clip is drawn");
    // Nothing soloed in the adjustment lane is not the same as nothing drawn
    // there: the rule is per kind of lane, and adjustments are picture too.
    assert!(adjustments_at(project.active().unwrap(), at).is_empty());

    project.active_mut().unwrap().set_clip_solo(lower, false);
    assert_eq!(
        layer_requests(&project, project.active().unwrap(), at).len(),
        2
    );
}

#[test]
fn a_soloed_sound_clip_is_the_only_one_in_the_mix() {
    let mut project = stacked();
    let clips = |plan: &AudioPlan| -> usize { plan.tracks.iter().map(|t| t.len()).sum() };
    assert_eq!(
        clips(&AudioPlan::of(&project, project.active().unwrap())),
        2
    );

    let upper = project.active().unwrap().audio_tracks[1].clips()[0].id;
    project.active_mut().unwrap().set_clip_solo(upper, true);
    let plan = AudioPlan::of(&project, project.active().unwrap());
    assert_eq!(clips(&plan), 1, "only the soloed clip is mixed");
    assert_eq!(plan.tracks[1].clips()[0].id, upper);

    // A picture solo alone leaves the mix as it was: the rule is per kind.
    project.active_mut().unwrap().set_clip_solo(upper, false);
    let picture = project.active().unwrap().video_tracks[0].clips()[0].id;
    project.active_mut().unwrap().set_clip_solo(picture, true);
    assert_eq!(
        clips(&AudioPlan::of(&project, project.active().unwrap())),
        2
    );
}
