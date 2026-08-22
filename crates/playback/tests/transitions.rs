//! Transitions, resolved (§25).
//!
//! These exercise [`layer_requests`] rather than the renderer, because that is
//! where a transition actually lives: it decides *which* clips are on screen at
//! an instant and how much of each. The compositor needs no new code — a
//! crossfade is the incoming clip drawn over the outgoing one at a partial
//! alpha, which §22's alpha-over already does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use bettercut_foundation::{ClipId, MediaTime, TimelineTime};
use bettercut_media::{FfmpegProber, MediaProber};
use bettercut_playback::layer_requests;
use bettercut_project_format::Project;
use bettercut_timeline::{Clip, SourceRange, Transition, TransitionKind, VideoClip};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../media/tests/fixtures")
        .join(name)
}

fn ms(n: i64) -> TimelineTime {
    TimelineTime::from_millis(n)
}

/// Two clips cut together at 800 ms, each trimmed away from the edges of the
/// file so both have handles to spare (§25).
///
/// A: source 200–1000 ms, on the timeline 0–800 ms.
/// B: source 1200–2000 ms, on the timeline 800–1600 ms.
fn cut_project() -> (Project, ClipId, ClipId) {
    let mut project = Project::new("Transitions");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");

    let a = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_millis(200), MediaTime::from_millis(1000)).expect("range"),
    )
    .expect("valid");
    let b = VideoClip::new(
        media,
        ms(800),
        SourceRange::new(MediaTime::from_millis(1200), MediaTime::from_millis(2000))
            .expect("range"),
    )
    .expect("valid");
    let (a_id, b_id) = (a.id, b.id);

    sequence.video_tracks[0].insert(a).expect("no overlap");
    sequence.video_tracks[0].insert(b).expect("no overlap");

    (project, a_id, b_id)
}

/// Attach a transition to the outgoing clip. 400 ms, so the window is
/// 600–1000 ms around the cut at 800.
fn set_transition(project: &mut Project, clip: ClipId, kind: TransitionKind) {
    project.active_mut().expect("sequence").video_tracks[0]
        .get_mut(clip)
        .expect("clip")
        .transition_out = Some(Transition::new(kind, ms(400)));
}

/// The clips do not move: a transition is a window over an ordinary cut, not an
/// overlap. §8's non-overlapping invariant is what makes the visible-range
/// query a binary search, and it is not worth spending on a dissolve.
#[test]
fn a_transition_does_not_move_the_clips() {
    let (mut project, a, _) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);

    let track = &project.active().expect("sequence").video_tracks[0];
    assert_eq!(track.clips()[0].timeline().end, ms(800));
    assert_eq!(track.clips()[1].timeline().start, ms(800));
}

#[test]
fn outside_the_window_only_one_clip_is_visible() {
    let (mut project, a, _) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);
    let sequence = project.active().expect("sequence");

    assert_eq!(layer_requests(&project, sequence, ms(300)).len(), 1);
    assert_eq!(layer_requests(&project, sequence, ms(1400)).len(), 1);
}

/// Before the cut, the incoming clip has to supply frames from *before* its
/// in-point — the handle. Getting this wrong is invisible in a still and
/// obvious in motion: the incoming shot would freeze on its first frame for
/// half the transition.
#[test]
fn the_incoming_clip_reads_its_handle_before_the_cut() {
    let (mut project, a, b) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);
    let sequence = project.active().expect("sequence");

    // 700 ms: a quarter of the way through the 600–1000 window.
    let layers = layer_requests(&project, sequence, ms(700));
    assert_eq!(layers.len(), 2, "both clips are on screen");

    let incoming = layers.iter().find(|l| l.clip == b).expect("incoming");
    assert_eq!(
        incoming.source_time,
        MediaTime::from_millis(1100),
        "100 ms before its own in-point of 1200"
    );
    assert!(
        (incoming.look.opacity - 0.25).abs() < 1e-6,
        "a quarter faded up, was {}",
        incoming.look.opacity
    );

    let outgoing = layers.iter().find(|l| l.clip == a).expect("outgoing");
    assert_eq!(outgoing.source_time, MediaTime::from_millis(900));
    assert_eq!(outgoing.look.opacity, 1.0, "the outgoing clip stays solid");
}

/// And after the cut the outgoing clip keeps reading past its out-point.
#[test]
fn the_outgoing_clip_reads_its_handle_after_the_cut() {
    let (mut project, a, b) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);
    let sequence = project.active().expect("sequence");

    let layers = layer_requests(&project, sequence, ms(900));
    assert_eq!(layers.len(), 2);

    let outgoing = layers.iter().find(|l| l.clip == a).expect("outgoing");
    assert_eq!(
        outgoing.source_time,
        MediaTime::from_millis(1100),
        "100 ms past its own out-point of 1000"
    );

    let incoming = layers.iter().find(|l| l.clip == b).expect("incoming");
    assert!(
        (incoming.look.opacity - 0.75).abs() < 1e-6,
        "three quarters faded up, was {}",
        incoming.look.opacity
    );
}

/// Within a track, later layers draw over earlier ones (§22). A crossfade is
/// the incoming picture coming up over the outgoing one, so the order is not
/// arbitrary — reversed, the fade would run backwards.
#[test]
fn the_incoming_clip_is_drawn_on_top() {
    let (mut project, a, b) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);
    let sequence = project.active().expect("sequence");

    let layers = layer_requests(&project, sequence, ms(700));
    assert_eq!(layers[0].clip, a, "outgoing first");
    assert_eq!(layers[1].clip, b, "incoming over it");
}

/// A fade through black never shows both clips, which is what lets it work
/// without handles.
#[test]
fn a_fade_through_black_shows_one_clip_at_a_time() {
    let (mut project, a, _) = cut_project();
    set_transition(&mut project, a, TransitionKind::FadeThroughBlack);
    let sequence = project.active().expect("sequence");

    for at in [ms(650), ms(800), ms(950)] {
        let layers = layer_requests(&project, sequence, at);
        assert_eq!(layers.len(), 1, "two clips at once at {at:?}");
    }
}

/// It reaches black at the cut and full brightness at both edges. A fade that
/// only ever reached half would look like a dip, not a fade.
#[test]
fn a_fade_through_black_dips_to_nothing_at_the_cut() {
    let (mut project, a, _) = cut_project();
    set_transition(&mut project, a, TransitionKind::FadeThroughBlack);
    let sequence = project.active().expect("sequence");

    let opacity = |at| layer_requests(&project, sequence, at)[0].look.opacity;

    assert!(opacity(ms(800)).abs() < 1e-6, "not black at the cut");
    assert!((opacity(ms(700)) - 0.5).abs() < 1e-6, "half way out");
    assert!((opacity(ms(900)) - 0.5).abs() < 1e-6, "half way in");
    assert!(
        (opacity(ms(600)) - 1.0).abs() < 1e-6,
        "full at the window start"
    );
    assert!(
        (opacity(ms(999)) - 1.0).abs() < 0.01,
        "full at the window end"
    );
}

/// A gap is not a cut. There is nothing on the other side to fade to, and
/// fading anyway would surprise whoever put the gap there.
#[test]
fn a_transition_with_no_neighbour_does_nothing() {
    let mut project = Project::new("Lonely");
    let asset = FfmpegProber
        .probe(&fixture("ntsc-2997.mp4"))
        .expect("probe");
    let media = project.add_media(asset);
    let sequence = project.active_mut().expect("sequence");

    let clip = VideoClip::new(
        media,
        TimelineTime::ZERO,
        SourceRange::new(MediaTime::from_millis(200), MediaTime::from_millis(1000)).expect("range"),
    )
    .expect("valid");
    let id = clip.id;
    sequence.video_tracks[0].insert(clip).expect("empty track");
    set_transition(&mut project, id, TransitionKind::Crossfade);

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, ms(700));
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].look.opacity, 1.0, "no partner, no fade");
}

/// The fade multiplies the opacity the clip already has. A clip the user set to
/// 50% that also crossfades should end up at half of half — not snapped back to
/// full by the transition.
#[test]
fn a_crossfade_multiplies_the_clip_s_own_opacity() {
    let (mut project, a, b) = cut_project();
    set_transition(&mut project, a, TransitionKind::Crossfade);
    project.active_mut().expect("sequence").video_tracks[0]
        .get_mut(b)
        .expect("clip")
        .opacity = 0.5;

    let sequence = project.active().expect("sequence");
    let layers = layer_requests(&project, sequence, ms(700));
    let incoming = layers.iter().find(|l| l.clip == b).expect("incoming");
    assert!(
        (incoming.look.opacity - 0.125).abs() < 1e-6,
        "half of a quarter, was {}",
        incoming.look.opacity
    );
}

/// Handles are bounded by what is actually in the file, and a clip trimmed to
/// the very start of its media has none.
#[test]
fn the_available_handle_bounds_the_length() {
    let at_the_start = Transition::max_duration(
        TransitionKind::Crossfade,
        ms(4000),
        ms(4000),
        MediaTime::from_millis(1000),
        MediaTime::ZERO,
    );
    assert_eq!(at_the_start, TimelineTime::ZERO);

    let with_room = Transition::max_duration(
        TransitionKind::Crossfade,
        ms(4000),
        ms(4000),
        MediaTime::from_millis(1000),
        MediaTime::from_millis(300),
    );
    assert_eq!(with_room, ms(600), "twice the smaller handle");
}
