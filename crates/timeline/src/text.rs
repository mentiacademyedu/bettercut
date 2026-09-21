//! Text overlays on the timeline (§26).
//!
//! ## A clip with no media
//!
//! A text clip is a *generated* source: it has a span on the timeline and it
//! produces a picture, but there is no file behind it and nothing to decode.
//! That is the only way it differs from a video clip, and it is not enough of a
//! difference to justify a second container — so it implements [`Clip`] and
//! lives in an ordinary [`Track`], which brings the sorted, non-overlapping
//! invariant and everything built on it (§8, §53): insert, move, trim, split,
//! ripple delete, all already written and already tested.
//!
//! Its source range starts at zero and runs as long as the clip does. That is
//! not a placeholder — it is what "how far into this clip are we" means for a
//! generated source, and it is what §24's keyframes anchor to, so an animated
//! title keeps its animation when it is moved.
//!
//! ## Above the picture, not in it
//!
//! Text tracks composite *after* every video track (§22). Not because the
//! ordering could not be interleaved, but because a title that can slip behind
//! a clip is a support question, and no editor of this kind offers it.

use bettercut_foundation::{ClipId, TimelineTime};
use bettercut_text::TextStyle;
use serde::{Deserialize, Serialize};

use crate::clip::{Clip, SourceRange, TimelineRange, Transform, impl_clip};
use crate::error::TimelineError;
use crate::motion::{TextAnimation, TextLook};
use crate::track::Track;

/// A piece of text on the timeline (§26).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextClip {
    pub id: ClipId,
    pub timeline: TimelineRange,
    /// Where in the clip's own span it is reading. Zero-based, and the same
    /// length as `timeline` unless the clip has been trimmed.
    pub source: SourceRange,

    pub text: String,
    pub style: TextStyle,

    /// Where the rasterized bitmap sits on the canvas, in the same normalized
    /// units every other layer uses: the same control, so it behaves the same
    /// way and the preview handles work on it unchanged.
    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub enabled: bool,
    /// How it arrives and leaves. Absent from older projects, which load with
    /// none.
    #[serde(default)]
    pub animation: TextAnimation,

    /// Smear the title along the way it is moving, as a picture clip can be.
    ///
    /// The same switch for the same reason: an entrance that spins or slides
    /// is a fast move, and a title is the thing most likely to be doing one.
    #[serde(default)]
    pub motion_blur: bool,
    /// A colour tag for organising the edit.
    #[serde(default)]
    pub color_label: crate::clip::ColorLabel,
    /// Draw a shape instead of the text (`bettercut_text::shape`). Everything
    /// else about the clip — placement, animation, the lane — is a title's.
    #[serde(default)]
    pub shape: Option<bettercut_text::Shape>,
    /// Draw a counting number instead of the text (`crate::counter`); the text
    /// stays as the clip's name.
    #[serde(default)]
    pub counter: Option<crate::counter::Counter>,
    /// Light up each word in this colour as it comes (`crate::karaoke`), or
    /// none for plain text.
    #[serde(default)]
    pub highlight: Option<bettercut_text::Rgba>,
}

fn one() -> f32 {
    1.0
}

/// How long a new title runs: three seconds.
///
/// Long enough to read a short line at a normal pace, short enough that it does
/// not have to be trimmed every time. Dropped in at the playhead and adjusted
/// from there.
pub const DEFAULT_DURATION: TimelineTime = TimelineTime::from_ticks(2_880_000);

impl TextClip {
    /// The words drawn `into_clip` after the clip starts: a counter's number
    /// when there is one, otherwise the text itself.
    pub fn shown_text(&self, into_clip: TimelineTime) -> std::borrow::Cow<'_, str> {
        match &self.counter {
            Some(counter) => std::borrow::Cow::Owned(counter.text_at(into_clip)),
            None => std::borrow::Cow::Borrowed(&self.text),
        }
    }

    /// Place `text` at `start` for [`DEFAULT_DURATION`].
    pub fn new(text: impl Into<String>, start: TimelineTime) -> Result<Self, TimelineError> {
        Self::with_duration(text, start, DEFAULT_DURATION)
    }

    pub fn with_duration(
        text: impl Into<String>,
        start: TimelineTime,
        duration: TimelineTime,
    ) -> Result<Self, TimelineError> {
        let source = SourceRange::new(
            bettercut_foundation::MediaTime::ZERO,
            bettercut_foundation::MediaTime::from_ticks(duration.ticks()),
        )?;
        Ok(Self {
            id: ClipId::new(),
            timeline: TimelineRange::new(start, start + duration)?,
            source,
            text: text.into(),
            style: TextStyle::default(),
            transform: Transform::default(),
            opacity: 1.0,
            enabled: true,
            animation: TextAnimation::default(),
            motion_blur: false,
            color_label: crate::clip::ColorLabel::None,
            shape: None,
            counter: None,
            highlight: None,
        })
    }

    /// How this title looks at `position`, its entrance and exit applied
    /// (§26, §46). Both the preview and the export ask this.
    pub fn look_at(&self, position: TimelineTime) -> TextLook {
        self.animation.look(
            self.transform,
            self.opacity,
            self.timeline,
            position,
            self.text.chars().count(),
        )
    }

    /// Whether this clip would draw anything at all.
    ///
    /// Empty text is not an error — it is what a title looks like the instant
    /// after it is added and before anything is typed — but it produces no
    /// bitmap, and the renderer needs to know that without rasterizing first.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

impl_clip!(
    TextClip,
    // A title's exit belongs to its end and its entrance to its start, so a
    // split keeps each on the half that still has that edge — the same rule
    // as a transition (§25), for the same reason.
    fn clear_transition_out(&mut self) {
        self.animation.outro = None;
    },
    fn clear_transition_in(&mut self) {
        self.animation.intro = None;
    }
);

/// A lane of text clips.
///
/// The same container the video and audio tracks use, so the invariants and the
/// editing operations are the same ones — see the module note.
pub type TextTrack = Track<TextClip>;

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(n: i64) -> TimelineTime {
        TimelineTime::from_seconds(n)
    }

    #[test]
    fn a_new_clip_runs_from_the_position_given() {
        let clip = TextClip::new("Hello", seconds(2)).expect("valid");
        assert_eq!(clip.timeline.start, seconds(2));
        assert_eq!(clip.timeline.duration(), DEFAULT_DURATION);
    }

    /// The source is zero-based and as long as the clip, which is what makes
    /// §24's keyframes land in the right place on a generated source.
    #[test]
    fn the_source_starts_at_zero_and_matches_the_duration() {
        let clip = TextClip::new("Hello", seconds(5)).expect("valid");
        assert_eq!(clip.source.start.ticks(), 0);
        assert_eq!(clip.source.duration().ticks(), DEFAULT_DURATION.ticks());
    }

    #[test]
    fn blank_text_is_recognised() {
        let mut clip = TextClip::new("Hello", TimelineTime::ZERO).expect("valid");
        assert!(!clip.is_blank());
        clip.text = "   \n ".to_string();
        assert!(clip.is_blank());
    }

    /// The whole reason for reusing [`Track`]: a text lane holds the same
    /// invariant every other lane does, without a second implementation of it.
    #[test]
    fn a_text_track_refuses_overlapping_clips() {
        let mut track = TextTrack::new("T1");
        track
            .insert(TextClip::with_duration("first", TimelineTime::ZERO, seconds(4)).expect("ok"))
            .expect("empty track");

        let overlapping = TextClip::with_duration("second", seconds(2), seconds(4)).expect("ok");
        assert!(track.insert(overlapping).is_err(), "an overlap was allowed");

        let after = TextClip::with_duration("second", seconds(4), seconds(4)).expect("ok");
        assert!(track.insert(after).is_ok(), "an abutting clip was refused");
    }

    /// And the editing operations come with it.
    #[test]
    fn a_text_clip_can_be_split_like_any_other() {
        let mut track = TextTrack::new("T1");
        let clip = TextClip::with_duration("Hello", TimelineTime::ZERO, seconds(4)).expect("ok");
        let id = clip.id;
        track.insert(clip).expect("empty track");

        let outcome = track
            .split(id, seconds(1), ClipId::new(), ClipId::new())
            .expect("split");

        assert_eq!(track.len(), 2);
        let left = track.get(outcome.left).expect("left half");
        let right = track.get(outcome.right).expect("right half");
        assert_eq!(left.timeline().end, seconds(1));
        assert_eq!(right.timeline().start, seconds(1));
        assert_eq!(left.text, "Hello", "the text did not survive the split");
        assert_eq!(right.text, "Hello");
    }

    /// The entrance stays on the half that still starts where the title
    /// started, and the exit on the half that still ends where it ended — a
    /// split must not add a second entrance in the middle.
    #[test]
    fn a_split_title_keeps_its_entrance_left_and_its_exit_right() {
        use crate::motion::{Motion, MotionKind};

        let mut track = TextTrack::new("T1");
        let mut clip =
            TextClip::with_duration("Hello", TimelineTime::ZERO, seconds(4)).expect("ok");
        clip.animation = TextAnimation {
            scroll: None,
            intro: Some(Motion::new(MotionKind::Pop, seconds(1))),
            outro: Some(Motion::new(MotionKind::Fade, seconds(1))),
            looping: None,
        };
        let id = clip.id;
        track.insert(clip).expect("empty track");

        let outcome = track
            .split(id, seconds(2), ClipId::new(), ClipId::new())
            .expect("split");

        let left = track.get(outcome.left).expect("left half");
        let right = track.get(outcome.right).expect("right half");
        assert!(left.animation.intro.is_some() && left.animation.outro.is_none());
        assert!(right.animation.intro.is_none() && right.animation.outro.is_some());
    }
}
