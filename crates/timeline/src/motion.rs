//! How a title arrives and leaves (§26).
//!
//! A title with an entrance and an exit, rather than a picture that blinks on
//! and off — the single most requested thing in a short-form editor, and one
//! that keyframes can do but nobody wants to key by hand for every caption.
//!
//! ## Presets, not curves
//!
//! Five motions, each a fixed recipe of opacity, offset, scale or reveal. A
//! preset is one choice in a dropdown and one number for its length; the
//! keyframe editor already exists for anyone who wants something else.
//!
//! ## Evaluated here, once
//!
//! [`TextAnimation::look`] is the one place a title's appearance at an instant
//! is decided, and both the preview and the export call it (§46) — the same
//! arrangement as `VideoClip::look_at` for keyframes. The progress through a
//! motion is a ratio of two integer tick distances, the one kind of fraction
//! §74 allows: it is the answer being asked for, not a position being stored.

use bettercut_foundation::TimelineTime;
use serde::{Deserialize, Serialize};

use crate::clip::{TimelineRange, Transform};

/// One way of arriving or leaving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionKind {
    /// Opacity only.
    Fade,
    /// Moves upward while fading: in from below, or out through the top.
    SlideUp,
    /// Moves downward while fading: in from above, or out through the bottom.
    SlideDown,
    /// Grows from half size with a slight overshoot, or shrinks away.
    Pop,
    /// Letters appear one at a time, or disappear from the end.
    Typewriter,
}

impl MotionKind {
    pub const ALL: [Self; 5] = [
        Self::Fade,
        Self::SlideUp,
        Self::SlideDown,
        Self::Pop,
        Self::Typewriter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::SlideUp => "Slide up",
            Self::SlideDown => "Slide down",
            Self::Pop => "Pop",
            Self::Typewriter => "Typewriter",
        }
    }
}

/// A motion and how long it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Motion {
    pub kind: MotionKind,
    pub duration: TimelineTime,
}

impl Motion {
    pub fn new(kind: MotionKind, duration: TimelineTime) -> Self {
        Self {
            kind,
            duration: duration.clamp(MIN_MOTION, MAX_MOTION),
        }
    }
}

/// A new motion's length: 0.4 s, quick enough not to hold the words back.
pub const DEFAULT_MOTION: TimelineTime = TimelineTime::from_ticks(384_000);
/// A tenth of a second: shorter is a flicker, not a motion.
pub const MIN_MOTION: TimelineTime = TimelineTime::from_ticks(96_000);
pub const MAX_MOTION: TimelineTime = TimelineTime::from_seconds(3);

/// How far a slide travels, as a fraction of the frame's height.
const SLIDE_DISTANCE: f32 = 0.08;

/// A title's entrance and exit. Neither, by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextAnimation {
    #[serde(default)]
    pub intro: Option<Motion>,
    #[serde(default)]
    pub outro: Option<Motion>,
}

/// What a title looks like at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLook {
    pub transform: Transform,
    pub opacity: f32,
    /// How many characters are drawn, when fewer than all of them.
    pub reveal: Option<usize>,
}

impl TextAnimation {
    pub fn is_none(&self) -> bool {
        self.intro.is_none() && self.outro.is_none()
    }

    /// Every length inside the allowed range. Applied by the edit, not the
    /// slider, for §38.2's reason: the journal replays commands, and a limit
    /// that lived only in the interface would come back unapplied.
    pub fn sanitized(self) -> Self {
        let fix = |m: Option<Motion>| m.map(|m| Motion::new(m.kind, m.duration));
        Self {
            intro: fix(self.intro),
            outro: fix(self.outro),
        }
    }

    /// The title at `position`, given how it looks when fully present.
    ///
    /// `chars` is how many characters the title has, for the typewriter.
    pub fn look(
        &self,
        transform: Transform,
        opacity: f32,
        span: TimelineRange,
        position: TimelineTime,
        chars: usize,
    ) -> TextLook {
        let mut look = TextLook {
            transform,
            opacity,
            reveal: None,
        };
        if self.is_none() {
            return look;
        }

        let length = span.duration().ticks().max(1);
        let (intro, outro) = self.fitted(length);
        let into = (position.ticks() - span.start.ticks()).clamp(0, length);
        let left = (span.end.ticks() - position.ticks()).clamp(0, length);

        if let Some((kind, ticks)) = intro {
            apply(
                &mut look,
                kind,
                presence(into, ticks),
                Edge::Entering,
                chars,
            );
        }
        if let Some((kind, ticks)) = outro {
            apply(&mut look, kind, presence(left, ticks), Edge::Leaving, chars);
        }
        look
    }

    /// Both lengths, in ticks, shrunk in proportion when together they are
    /// longer than the clip — so a short caption with a long entrance and exit
    /// still reaches full presence at its middle rather than never arriving.
    fn fitted(&self, length: i64) -> (Timed, Timed) {
        let intro = self.intro.map(|m| (m.kind, m.duration.ticks().max(1)));
        let outro = self.outro.map(|m| (m.kind, m.duration.ticks().max(1)));
        let total = intro.map_or(0, |m| m.1) + outro.map_or(0, |m| m.1);
        if total <= length {
            return (intro, outro);
        }
        let shrink = |(kind, ticks): (MotionKind, i64)| {
            // i128: a tick count times a tick count is past i64 for long clips.
            let scaled = (i128::from(ticks) * i128::from(length) / i128::from(total)) as i64;
            (kind, scaled.max(1))
        };
        (intro.map(shrink), outro.map(shrink))
    }
}

/// A motion's kind and its length in ticks, once fitted to the clip.
type Timed = Option<(MotionKind, i64)>;

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Entering,
    Leaving,
}

/// 0 when the motion has not begun to bring the title in (or has finished
/// taking it away), 1 once it is fully present.
fn presence(distance: i64, length: i64) -> f32 {
    (distance as f64 / length as f64).clamp(0.0, 1.0) as f32
}

fn apply(look: &mut TextLook, kind: MotionKind, presence: f32, edge: Edge, chars: usize) {
    if presence >= 1.0 {
        return;
    }
    let eased = ease_out_cubic(presence);
    match kind {
        MotionKind::Fade => look.opacity *= presence,
        MotionKind::SlideUp | MotionKind::SlideDown => {
            // Positive y is down the frame. Sliding up means arriving from
            // below and leaving through the top.
            let upward = kind == MotionKind::SlideUp;
            let below = match edge {
                Edge::Entering => upward,
                Edge::Leaving => !upward,
            };
            let away = SLIDE_DISTANCE * (1.0 - eased);
            look.transform.position.y += if below { away } else { -away };
            look.opacity *= eased;
        }
        MotionKind::Pop => {
            let grow = 0.5 + 0.5 * ease_out_back(presence);
            look.transform.scale.x *= grow;
            look.transform.scale.y *= grow;
            // Visible quickly, so the overshoot is seen rather than faded.
            look.opacity *= (presence * 3.0).min(1.0);
        }
        MotionKind::Typewriter => {
            // Rounded up, so the first letter shows as soon as the motion
            // starts and the last goes only as it ends.
            let shown = (chars as f32 * presence).ceil() as usize;
            look.reveal = Some(look.reveal.map_or(shown, |r| r.min(shown)).min(chars));
        }
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Past 1 briefly before settling: the "pop".
fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: i64) -> TimelineTime {
        TimelineTime::from_seconds(s)
    }

    fn millis(ms: i64) -> TimelineTime {
        TimelineTime::from_ticks(ms * 960)
    }

    fn span() -> TimelineRange {
        TimelineRange::new(secs(10), secs(14)).unwrap()
    }

    fn animated(intro: MotionKind, outro: MotionKind) -> TextAnimation {
        TextAnimation {
            intro: Some(Motion::new(intro, secs(1))),
            outro: Some(Motion::new(outro, secs(1))),
        }
    }

    fn at(animation: TextAnimation, ms: i64) -> TextLook {
        animation.look(Transform::default(), 1.0, span(), millis(10_000 + ms), 10)
    }

    #[test]
    fn no_animation_changes_nothing() {
        let look = at(TextAnimation::default(), 0);
        assert_eq!(look.opacity, 1.0);
        assert_eq!(look.transform, Transform::default());
        assert_eq!(look.reveal, None);
    }

    #[test]
    fn a_fade_rises_in_and_falls_out() {
        let fade = animated(MotionKind::Fade, MotionKind::Fade);
        assert_eq!(at(fade, 0).opacity, 0.0, "invisible at its first instant");
        assert!((at(fade, 500).opacity - 0.5).abs() < 1e-6);
        assert_eq!(at(fade, 2000).opacity, 1.0, "fully present in the middle");
        assert!((at(fade, 3500).opacity - 0.5).abs() < 1e-6);
        assert_eq!(at(fade, 4000).opacity, 0.0);
    }

    #[test]
    fn slide_up_arrives_from_below_and_leaves_through_the_top() {
        let slide = animated(MotionKind::SlideUp, MotionKind::SlideUp);
        assert!(at(slide, 0).transform.position.y > 0.0, "starts below");
        assert_eq!(at(slide, 2000).transform.position.y, 0.0);
        assert!(at(slide, 3900).transform.position.y < 0.0, "leaves upward");

        let down = animated(MotionKind::SlideDown, MotionKind::SlideDown);
        assert!(at(down, 0).transform.position.y < 0.0, "starts above");
        assert!(at(down, 3900).transform.position.y > 0.0, "leaves downward");
    }

    #[test]
    fn pop_overshoots_before_settling() {
        let pop = animated(MotionKind::Pop, MotionKind::Fade);
        assert!(at(pop, 0).transform.scale.x < 0.6, "starts small");
        let peak = (1..10)
            .map(|i| at(pop, i * 100).transform.scale.x)
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.0, "never overshot: peak {peak}");
        assert_eq!(at(pop, 2000).transform.scale.x, 1.0);
    }

    #[test]
    fn typewriter_reveals_letters_in_and_takes_them_away_out() {
        let typing = animated(MotionKind::Typewriter, MotionKind::Typewriter);
        assert_eq!(at(typing, 0).reveal, Some(0));
        assert_eq!(at(typing, 50).reveal, Some(1), "the first letter at once");
        assert_eq!(at(typing, 500).reveal, Some(5));
        assert_eq!(at(typing, 2000).reveal, None, "whole in the middle");
        assert_eq!(at(typing, 3500).reveal, Some(5));
        assert_eq!(at(typing, 3999).reveal, Some(1));
        assert_eq!(at(typing, 0).opacity, 1.0, "typing is not also a fade");
    }

    #[test]
    fn motions_longer_than_the_clip_share_it() {
        // Two 3 s motions on a 4 s clip: each shrinks to 2 s, so the title is
        // fully present exactly at the middle rather than never.
        let long = TextAnimation {
            intro: Some(Motion::new(MotionKind::Fade, secs(3))),
            outro: Some(Motion::new(MotionKind::Fade, secs(3))),
        };
        assert_eq!(at(long, 2000).opacity, 1.0);
        assert!((at(long, 1000).opacity - 0.5).abs() < 1e-6);
    }

    #[test]
    fn lengths_are_kept_in_range() {
        let wild = TextAnimation {
            intro: Some(Motion {
                kind: MotionKind::Fade,
                duration: TimelineTime::ZERO,
            }),
            outro: Some(Motion {
                kind: MotionKind::Fade,
                duration: secs(600),
            }),
        }
        .sanitized();
        assert_eq!(wild.intro.unwrap().duration, MIN_MOTION);
        assert_eq!(wild.outro.unwrap().duration, MAX_MOTION);
    }
}
