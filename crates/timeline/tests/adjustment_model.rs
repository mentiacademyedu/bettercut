//! The adjustment clip as data: what it defaults to, and what a file can say.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_foundation::TimelineTime;
use bettercut_timeline::{
    AdjustmentClip, AdjustmentLook, AdjustmentTrack, Clip, ColorAdjust, Sequence,
};

/// A project written before adjustments existed loads with none, and so draws
/// exactly as it did.
#[test]
fn an_older_project_has_no_adjustments() {
    let mut value = serde_json::to_value(Sequence::default_hd()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("adjustment_tracks")
        .expect("the field is written today");
    let loaded: Sequence = serde_json::from_value(value).expect("an older sequence still loads");
    assert!(loaded.adjustment_tracks.is_empty());
}

/// A new adjustment changes nothing until it is told to — adding one must not
/// visibly alter the edit on its own.
#[test]
fn a_new_adjustment_changes_nothing() {
    let clip = AdjustmentClip::new(TimelineTime::from_seconds(4)).unwrap();
    assert!(clip.look.is_identity());
    assert_eq!(clip.timeline.start, TimelineTime::from_seconds(4));
    assert_eq!(
        clip.timeline.end,
        TimelineTime::from_seconds(4) + bettercut_timeline::DEFAULT_ADJUSTMENT_DURATION
    );
}

/// Its source runs from zero for as long as it lasts, like a title's, so the
/// container's trims and splits mean the same thing for it.
#[test]
fn its_source_is_its_own_span() {
    let clip = AdjustmentClip::with_duration(
        TimelineTime::from_seconds(10),
        TimelineTime::from_seconds(2),
    )
    .unwrap();
    assert_eq!(clip.source().start, bettercut_foundation::MediaTime::ZERO);
    assert_eq!(
        clip.source().duration().ticks(),
        TimelineTime::from_seconds(2).ticks()
    );
}

/// The container's editing works on it: the point of it being an ordinary
/// clip in an ordinary track.
#[test]
fn it_splits_like_any_clip() {
    let mut lane = AdjustmentTrack::new("Adj 1");
    let clip =
        AdjustmentClip::with_duration(TimelineTime::ZERO, TimelineTime::from_seconds(4)).unwrap();
    let id = clip.id;
    lane.insert(clip).unwrap();

    lane.split(
        id,
        TimelineTime::from_seconds(1),
        bettercut_foundation::ClipId::new(),
        bettercut_foundation::ClipId::new(),
    )
    .unwrap();
    assert_eq!(lane.len(), 2, "the adjustment did not split in two");
    lane.validate()
        .expect("the lane is still consistent after the split");
}

/// A look saved and loaded comes back whole, and a look written before one of
/// its fields existed takes that field's identity.
#[test]
fn a_look_round_trips_and_fills_in_what_it_lacks() {
    let look = AdjustmentLook {
        color: ColorAdjust {
            brightness: 0.9,
            contrast: 1.2,
            saturation: 0.6,
            temperature: -0.4,
            tint: 0.1,
            vibrance: 0.25,
            wheels: Default::default(),
            secondary: bettercut_timeline::HslSecondary::IDENTITY,
        },
        blur: 15.0,
        strength: 0.7,
        vignette: 0.45,
        grain: 0.3,
    };
    let back: AdjustmentLook =
        serde_json::from_str(&serde_json::to_string(&look).unwrap()).unwrap();
    assert_eq!(back, look);

    let sparse: AdjustmentLook = serde_json::from_str("{}").unwrap();
    assert_eq!(
        sparse,
        AdjustmentLook::default(),
        "an empty look should be the one that changes nothing, at full strength"
    );
}

/// `Sequence::clip_spans` is the one list of lanes, so it has to hold every
/// kind of lane there is.
///
/// A clip goes on one lane of each kind and every one must be reported. The
/// exhaustive match is the part that makes this last: add a lane kind and this
/// stops compiling until someone has put a clip on it here — which is the
/// moment they find out `clip_spans` needs it too. Before `clip_spans` existed
/// the same omission was made in five separate lookups, each reading as "not
/// found" for a clip that was there.
#[test]
fn every_kind_of_lane_is_in_the_one_list() {
    use bettercut_foundation::{MediaId, MediaTime};
    use bettercut_timeline::{
        AudioClip, AudioTrack, SourceRange, TextClip, TextTrack, TrackKind, VideoClip, VideoTrack,
    };

    let mut sequence = Sequence::default_hd();
    let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap();

    let mut placed = Vec::new();
    for kind in [
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Text,
        TrackKind::Adjustment,
    ] {
        let id = match kind {
            TrackKind::Video => {
                let clip = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
                let id = clip.id;
                let mut track = VideoTrack::new("V");
                track.insert(clip).unwrap();
                sequence.video_tracks.push(track);
                id
            }
            TrackKind::Audio => {
                let clip = AudioClip::new(MediaId::new(), TimelineTime::ZERO, source).unwrap();
                let id = clip.id;
                let mut track = AudioTrack::new("A");
                track.insert(clip).unwrap();
                sequence.audio_tracks.push(track);
                id
            }
            TrackKind::Text => {
                let clip = TextClip::new("title", TimelineTime::ZERO).unwrap();
                let id = clip.id;
                let mut track = TextTrack::new("T");
                track.insert(clip).unwrap();
                sequence.text_tracks.push(track);
                id
            }
            TrackKind::Adjustment => {
                let clip = AdjustmentClip::new(TimelineTime::ZERO).unwrap();
                let id = clip.id;
                let mut track = AdjustmentTrack::new("F");
                track.insert(clip).unwrap();
                sequence.adjustment_tracks.push(track);
                id
            }
        };
        placed.push((kind, id));
    }

    for (kind, id) in placed {
        let span = sequence
            .clip_span(id)
            .unwrap_or_else(|| panic!("a clip on a {kind:?} lane is missing from clip_spans"));
        assert_eq!(
            span.kind, kind,
            "a {kind:?} clip was reported as {:?}",
            span.kind
        );
    }
}
