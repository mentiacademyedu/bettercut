//! How a title or a shot arrives and leaves (§26).
//!
//! Something with an entrance and an exit, rather than a picture that blinks on
//! and off — the single most requested thing in a short-form editor, and one
//! that keyframes can do but nobody wants to key by hand for every caption.
//!
//! ## Presets, not curves
//!
//! A handful of motions, each a fixed recipe of opacity, offset, scale, turn or
//! reveal. A preset is one choice in a dropdown and one number for its length;
//! the keyframe editor already exists for anyone who wants something else.
//!
//! ## One set, two subjects
//!
//! [`TextAnimation`] animates a title and [`ClipMotion`] a picture clip, and
//! they share every preset and all of the arithmetic. An editor in which "slide
//! up" means one thing on a caption and another on a shot is one nobody can
//! predict. The single difference is how far a slide travels: a caption is
//! nudged and read, a shot comes in from outside the frame.
//!
//! ## Evaluated here, once
//!
//! Their `look` methods are the one place an animated subject's appearance at
//! an instant is decided, and both the preview and the export call them (§46) —
//! the same arrangement as `VideoClip::look_at` for keyframes. The progress
//! through a motion is a ratio of two integer tick distances, the one kind of
//! fraction §74 allows: it is the answer being asked for, not a position being
//! stored.

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
    /// Moves rightward while fading: in from the left, or out to the right.
    SlideRight,
    /// Moves leftward while fading: in from the right, or out to the left.
    SlideLeft,
    /// Grows from half size with a slight overshoot, or shrinks away.
    Pop,
    /// Drops in from above and bounces as it lands, or hops up and away.
    Bounce,
    /// Turns into place, or turns away.
    Spin,
    /// Letters appear one at a time, or disappear from the end.
    Typewriter,
}

impl MotionKind {
    pub const ALL: [Self; 8] = [
        Self::Fade,
        Self::SlideUp,
        Self::SlideDown,
        Self::SlideRight,
        Self::SlideLeft,
        Self::Pop,
        Self::Bounce,
        Self::Spin,
    ];

    /// The ones a title can also do. A picture has no letters to reveal, so
    /// the typewriter is a title's alone — offering it on a clip would be a
    /// preset that visibly does nothing.
    pub const FOR_TEXT: [Self; 9] = [
        Self::Fade,
        Self::SlideUp,
        Self::SlideDown,
        Self::SlideRight,
        Self::SlideLeft,
        Self::Pop,
        Self::Bounce,
        Self::Spin,
        Self::Typewriter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::SlideUp => "Slide up",
            Self::SlideDown => "Slide down",
            Self::SlideRight => "Slide right",
            Self::SlideLeft => "Slide left",
            Self::Pop => "Pop",
            Self::Bounce => "Bounce",
            Self::Spin => "Spin",
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

/// How far a *title* slides, as a fraction of the frame.
///
/// A nudge, not an arrival: a caption that flew in from off-screen would be
/// unreadable for the first half of its entrance, and the fade is what does the
/// work of announcing it.
const TITLE_TRAVEL: f32 = 0.08;

/// How far a *picture* slides.
///
/// A whole frame, because that is what puts the clip off-screen: the position
/// is doubled into clip space, so 1.0 carries the centre past the far edge and
/// the shot genuinely enters from outside the frame rather than sliding within
/// it (§59).
const CLIP_TRAVEL: f32 = 1.0;

/// How far a spin turns before settling.
const SPIN_DEGREES: f32 = 180.0;

/// A title's entrance and exit. Neither, by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextAnimation {
    #[serde(default)]
    pub intro: Option<Motion>,
    #[serde(default)]
    pub outro: Option<Motion>,
    /// A movement that repeats the whole time the title is on screen, on top
    /// of its entrance and exit.
    #[serde(default)]
    pub looping: Option<LoopMotion>,
    /// Carried across the frame for the whole time the title is up: credits
    /// rolling up, or a ticker running along.
    #[serde(default)]
    pub scroll: Option<Scroll>,
}

/// Which way a scrolling title travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scroll {
    /// Up from below the frame to above it, like end credits.
    Credits,
    /// Right to left across the frame, like a news ticker.
    Ticker,
}

impl Scroll {
    pub const ALL: [Self; 2] = [Self::Credits, Self::Ticker];

    pub fn label(self) -> &'static str {
        match self {
            Self::Credits => "Credits",
            Self::Ticker => "Ticker",
        }
    }

    /// Carry `look` to where the scroll has it `progress` (0–1) through the
    /// title. The text enters wholly off one edge and leaves wholly off the
    /// other, whatever its size: the anchor slides from the text's leading
    /// edge to its trailing one as the position crosses the frame, so no
    /// measurement of the text is needed here.
    pub fn apply(self, look: &mut TextLook, progress: f32) {
        let p = progress.clamp(0.0, 1.0);
        match self {
            Self::Credits => {
                look.transform.anchor.y = p;
                look.transform.position.y = 0.5 - p;
            }
            Self::Ticker => {
                look.transform.anchor.x = p;
                look.transform.position.x = 0.5 - p;
            }
        }
    }
}

/// A movement a title or sticker keeps making while it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMotion {
    /// Swells and settles, like a heartbeat.
    Pulse,
    /// Rocks side to side.
    Wiggle,
    /// Turns round and round.
    Spin,
    /// Bobs gently up and down.
    Float,
}

impl LoopMotion {
    pub const ALL: [Self; 4] = [Self::Pulse, Self::Wiggle, Self::Spin, Self::Float];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pulse => "Pulse",
            Self::Wiggle => "Wiggle",
            Self::Spin => "Spin",
            Self::Float => "Float",
        }
    }

    /// Move `look` to where the loop has it `seconds` after the title starts.
    /// A function of time alone, so preview and export agree (§46), and at
    /// the first instant every loop is at rest, so nothing jumps on arrival.
    pub fn apply(self, look: &mut TextLook, seconds: f32) {
        use std::f32::consts::TAU;
        let t = seconds.max(0.0);
        match self {
            Self::Pulse => {
                let swell = 1.0 + 0.08 * (TAU * t / 0.9).sin().max(0.0);
                look.transform.scale.x *= swell;
                look.transform.scale.y *= swell;
            }
            Self::Wiggle => {
                look.transform.rotation_degrees += 7.0 * (TAU * t / 0.6).sin();
            }
            Self::Spin => {
                look.transform.rotation_degrees += 180.0 * t;
            }
            Self::Float => {
                look.transform.position.y -= 0.02 * (TAU * t / 2.4).sin();
            }
        }
    }
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
        self.intro.is_none()
            && self.outro.is_none()
            && self.looping.is_none()
            && self.scroll.is_none()
    }

    /// Every length inside the allowed range. Applied by the edit, not the
    /// slider, for §38.2's reason: the journal replays commands, and a limit
    /// that lived only in the interface would come back unapplied.
    pub fn sanitized(self) -> Self {
        let fix = |m: Option<Motion>| m.map(|m| Motion::new(m.kind, m.duration));
        Self {
            intro: fix(self.intro),
            outro: fix(self.outro),
            looping: self.looping,
            scroll: self.scroll,
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
        let mut look = evaluate(
            self.intro,
            self.outro,
            transform,
            opacity,
            span,
            position,
            chars,
            TITLE_TRAVEL,
        );
        if let Some(looping) = self.looping {
            let seconds = (position.ticks() - span.start.ticks()) as f32
                / bettercut_foundation::TICKS_PER_SECOND as f32;
            looping.apply(&mut look, seconds);
        }
        if let Some(scroll) = self.scroll {
            let progress = (position.ticks() - span.start.ticks()) as f32
                / span.duration().ticks().max(1) as f32;
            scroll.apply(&mut look, progress);
        }
        look
    }
}

/// A picture clip's entrance and exit (Clip animations).
///
/// The same presets a title gets, and deliberately the same code: an editor in
/// which "slide up" means one thing on a caption and another on a shot is an
/// editor nobody can predict. What differs is how far a slide travels — a
/// caption is nudged, a shot arrives from outside the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClipMotion {
    #[serde(default)]
    pub intro: Option<Motion>,
    #[serde(default)]
    pub outro: Option<Motion>,
}

impl ClipMotion {
    pub fn is_none(&self) -> bool {
        self.intro.is_none() && self.outro.is_none()
    }

    /// Every length inside the allowed range, as [`TextAnimation::sanitized`].
    pub fn sanitized(self) -> Self {
        let fix = |m: Option<Motion>| m.map(|m| Motion::new(m.kind, m.duration));
        Self {
            intro: fix(self.intro),
            outro: fix(self.outro),
        }
    }

    /// How long the entrance and the exit take, in ticks.
    ///
    /// For drawing them on the timeline, where they are the same ramps a
    /// sound's fades get: a clip that is arriving should look like it, not like
    /// an ordinary clip with a badge on it.
    pub fn ramps(&self) -> (i64, i64) {
        (
            self.intro.map_or(0, |m| m.duration.ticks()),
            self.outro.map_or(0, |m| m.duration.ticks()),
        )
    }

    /// The clip's transform and opacity at `position`, given how it looks when
    /// fully present.
    ///
    /// Applied on top of the clip's own transform rather than replacing it, so
    /// a shot that has been scaled and moved slides in from off-frame to where
    /// the user put it — not to the middle.
    pub fn look(
        &self,
        transform: Transform,
        opacity: f32,
        span: TimelineRange,
        position: TimelineTime,
    ) -> (Transform, f32) {
        let look = evaluate(
            self.intro,
            self.outro,
            transform,
            opacity,
            span,
            position,
            0,
            CLIP_TRAVEL,
        );
        (look.transform, look.opacity)
    }
}

/// One instant of an entrance and an exit, whichever subject they belong to.
#[expect(
    clippy::too_many_arguments,
    reason = "the subject's whole state at an instant; splitting it would only \
              move the arguments into a struct nobody else has a use for"
)]
fn evaluate(
    intro: Option<Motion>,
    outro: Option<Motion>,
    transform: Transform,
    opacity: f32,
    span: TimelineRange,
    position: TimelineTime,
    chars: usize,
    travel: f32,
) -> TextLook {
    let mut look = TextLook {
        transform,
        opacity,
        reveal: None,
    };
    if intro.is_none() && outro.is_none() {
        return look;
    }

    let length = span.duration().ticks().max(1);
    let (intro, outro) = fitted(intro, outro, length);
    let into = (position.ticks() - span.start.ticks()).clamp(0, length);
    let left = (span.end.ticks() - position.ticks()).clamp(0, length);

    if let Some((kind, ticks)) = intro {
        apply(
            &mut look,
            kind,
            presence(into, ticks),
            Edge::Entering,
            chars,
            travel,
        );
    }
    if let Some((kind, ticks)) = outro {
        apply(
            &mut look,
            kind,
            presence(left, ticks),
            Edge::Leaving,
            chars,
            travel,
        );
    }
    look
}

/// Both lengths, in ticks, shrunk in proportion when together they are longer
/// than the clip — so a short caption with a long entrance and exit still
/// reaches full presence at its middle rather than never arriving.
fn fitted(intro: Option<Motion>, outro: Option<Motion>, length: i64) -> (Timed, Timed) {
    let intro = intro.map(|m| (m.kind, m.duration.ticks().max(1)));
    let outro = outro.map(|m| (m.kind, m.duration.ticks().max(1)));
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

fn apply(
    look: &mut TextLook,
    kind: MotionKind,
    presence: f32,
    edge: Edge,
    chars: usize,
    travel: f32,
) {
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
            let away = travel * (1.0 - eased);
            look.transform.position.y += if below { away } else { -away };
            look.opacity *= eased;
        }
        MotionKind::SlideRight | MotionKind::SlideLeft => {
            // Positive x is rightward. Sliding right means arriving from the
            // left and leaving off the right.
            let rightward = kind == MotionKind::SlideRight;
            let left_of_home = match edge {
                Edge::Entering => rightward,
                Edge::Leaving => !rightward,
            };
            let away = travel * (1.0 - eased);
            look.transform.position.x += if left_of_home { -away } else { away };
            look.opacity *= eased;
        }
        MotionKind::Spin => {
            // Anticlockwise into place, clockwise away — the same direction of
            // travel through the whole clip, so an intro and an outro read as
            // one movement rather than a wobble.
            let turn = SPIN_DEGREES * (1.0 - eased);
            look.transform.rotation_degrees += match edge {
                Edge::Entering => -turn,
                Edge::Leaving => turn,
            };
            look.opacity *= eased;
        }
        MotionKind::Pop => {
            let grow = 0.5 + 0.5 * ease_out_back(presence);
            look.transform.scale.x *= grow;
            look.transform.scale.y *= grow;
            // Visible quickly, so the overshoot is seen rather than faded.
            look.opacity *= (presence * 3.0).min(1.0);
        }
        MotionKind::Bounce => {
            // Falls from above and lands with two smaller bounces; leaving,
            // the same played backwards, so it hops up and out. Solid almost
            // at once, so the bounce is seen rather than faded.
            let away = travel * (1.0 - ease_out_bounce(presence));
            look.transform.position.y -= away;
            look.opacity *= (presence * 4.0).min(1.0);
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

/// A ball dropped onto a floor: lands at a third of the way, then two
/// smaller bounces, each touching 1 again.
fn ease_out_bounce(t: f32) -> f32 {
    const N1: f32 = 7.5625;
    const D1: f32 = 2.75;
    if t < 1.0 / D1 {
        N1 * t * t
    } else if t < 2.0 / D1 {
        let t = t - 1.5 / D1;
        N1 * t * t + 0.75
    } else if t < 2.5 / D1 {
        let t = t - 2.25 / D1;
        N1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / D1;
        N1 * t * t + 0.984375
    }
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

    /// The bounce starts above, lands, rises again and settles home.
    #[test]
    fn bounce_lands_and_rises_again_before_settling() {
        assert_eq!(ease_out_bounce(0.0), 0.0);
        assert!(
            (ease_out_bounce(1.0 / 2.75) - 1.0).abs() < 1e-4,
            "no first landing"
        );
        assert!(ease_out_bounce(1.5 / 2.75) < 0.8, "no bounce after landing");
        assert!((ease_out_bounce(1.0) - 1.0).abs() < 1e-4, "does not settle");
    }

    fn animated(intro: MotionKind, outro: MotionKind) -> TextAnimation {
        TextAnimation {
            scroll: None,
            intro: Some(Motion::new(intro, secs(1))),
            outro: Some(Motion::new(outro, secs(1))),
            looping: None,
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
            scroll: None,
            intro: Some(Motion::new(MotionKind::Fade, secs(3))),
            outro: Some(Motion::new(MotionKind::Fade, secs(3))),
            looping: None,
        };
        assert_eq!(at(long, 2000).opacity, 1.0);
        assert!((at(long, 1000).opacity - 0.5).abs() < 1e-6);
    }

    fn clip_motion(intro: MotionKind, outro: MotionKind) -> ClipMotion {
        ClipMotion {
            intro: Some(Motion::new(intro, secs(1))),
            outro: Some(Motion::new(outro, secs(1))),
        }
    }

    /// `(transform, opacity)` for a picture clip `ms` into the same span.
    fn clip_at(motion: ClipMotion, ms: i64) -> (Transform, f32) {
        motion.look(Transform::default(), 1.0, span(), millis(10_000 + ms))
    }

    /// The difference between a caption and a shot. A title is nudged, because
    /// a caption that flew in from off-screen would be unreadable while it
    /// travelled; a shot arrives from outside the frame, which is what the
    /// animation is *for*.
    #[test]
    fn a_picture_arrives_from_outside_the_frame_and_a_title_only_leans_in() {
        let picture = clip_at(clip_motion(MotionKind::SlideRight, MotionKind::Fade), 0);
        assert!(
            picture.0.position.x <= -1.0,
            "the shot started inside the frame at {}",
            picture.0.position.x
        );

        let title = at(animated(MotionKind::SlideUp, MotionKind::Fade), 0);
        assert!(
            title.transform.position.y.abs() < 0.2,
            "a caption was thrown off-screen: {}",
            title.transform.position.y
        );
    }

    #[test]
    fn sliding_right_enters_from_the_left_and_leaves_to_the_right() {
        let right = clip_motion(MotionKind::SlideRight, MotionKind::SlideRight);
        assert!(clip_at(right, 0).0.position.x < 0.0, "did not start left");
        assert_eq!(clip_at(right, 2000).0.position.x, 0.0, "did not settle");
        assert!(
            clip_at(right, 3900).0.position.x > 0.0,
            "did not leave right"
        );

        let left = clip_motion(MotionKind::SlideLeft, MotionKind::SlideLeft);
        assert!(clip_at(left, 0).0.position.x > 0.0, "did not start right");
        assert!(clip_at(left, 3900).0.position.x < 0.0, "did not leave left");
    }

    /// Anticlockwise in, clockwise out — one direction of travel through the
    /// clip rather than a turn and a turn back, which reads as a wobble.
    #[test]
    fn a_spin_turns_one_way_through_the_whole_clip() {
        let spin = clip_motion(MotionKind::Spin, MotionKind::Spin);
        assert!(
            clip_at(spin, 0).0.rotation_degrees < -90.0,
            "did not turn in"
        );
        assert_eq!(clip_at(spin, 2000).0.rotation_degrees, 0.0, "never settled");
        assert!(
            clip_at(spin, 3900).0.rotation_degrees > 90.0,
            "turned back the way it came"
        );
    }

    /// An animation is *on top of* the clip's own placement. A shot the user
    /// scaled and moved into a corner must slide in to that corner, not to the
    /// middle of the frame.
    #[test]
    fn an_animation_leaves_the_clips_own_placement_alone() {
        let placed = Transform {
            position: crate::clip::Vec2::new(0.3, -0.2),
            scale: crate::clip::Vec2::new(0.5, 0.5),
            ..Transform::default()
        };
        let motion = clip_motion(MotionKind::SlideRight, MotionKind::Fade);

        let middle = motion.look(placed, 1.0, span(), millis(12_000));
        assert_eq!(middle.0, placed, "the clip did not settle where it was put");

        let start = motion.look(placed, 1.0, span(), millis(10_000));
        assert_eq!(
            start.0.scale, placed.scale,
            "the entrance rescaled the shot"
        );
        assert!(start.0.position.x < placed.position.x);
        assert_eq!(start.0.position.y, placed.position.y);
    }

    /// A clip that is arriving should look like it on the timeline, the same
    /// way a sound's fades and a title's entrance do — not like an ordinary
    /// clip with a badge on it.
    #[test]
    fn a_motion_reports_its_length_for_drawing() {
        assert_eq!(ClipMotion::default().ramps(), (0, 0), "nothing to draw");

        let one = clip_motion(MotionKind::Fade, MotionKind::Spin);
        assert_eq!(one.ramps(), (secs(1).ticks(), secs(1).ticks()));

        let intro_only = ClipMotion {
            intro: Some(Motion::new(MotionKind::Pop, millis(400))),
            outro: None,
        };
        assert_eq!(intro_only.ramps(), (millis(400).ticks(), 0));
    }

    #[test]
    fn lengths_are_kept_in_range() {
        let wild = TextAnimation {
            scroll: None,
            intro: Some(Motion {
                kind: MotionKind::Fade,
                duration: TimelineTime::ZERO,
            }),
            outro: Some(Motion {
                kind: MotionKind::Fade,
                duration: secs(600),
            }),
            looping: None,
        }
        .sanitized();
        assert_eq!(wild.intro.unwrap().duration, MIN_MOTION);
        assert_eq!(wild.outro.unwrap().duration, MAX_MOTION);
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use bettercut_foundation::TICKS_PER_SECOND;

    fn at(looping: LoopMotion, seconds: f32) -> TextLook {
        let animation = TextAnimation {
            scroll: None,
            intro: None,
            outro: None,
            looping: Some(looping),
        };
        let span = TimelineRange::new(TimelineTime::ZERO, TimelineTime::from_seconds(10)).unwrap();
        let position = TimelineTime::from_ticks((seconds * TICKS_PER_SECOND as f32) as i64);
        animation.look(Transform::default(), 1.0, span, position, 5)
    }

    /// Every loop is at rest the instant the title starts, and moves after.
    #[test]
    fn loops_start_at_rest_and_move() {
        let rest = Transform::default();
        for kind in LoopMotion::ALL {
            let start = at(kind, 0.0).transform;
            assert!((start.scale.x - rest.scale.x).abs() < 1e-4, "{kind:?}");
            assert!(start.rotation_degrees.abs() < 1e-3, "{kind:?}");
            assert!(
                (start.position.y - rest.position.y).abs() < 1e-4,
                "{kind:?}"
            );
            let moved = (0..40).any(|i| {
                let later = at(kind, i as f32 * 0.07).transform;
                (later.scale.x - 1.0).abs() > 0.01
                    || later.rotation_degrees.abs() > 0.5
                    || (later.position.y - rest.position.y).abs() > 0.005
            });
            assert!(moved, "{kind:?} never moved");
        }
    }

    /// A pulse only ever swells; a spin keeps turning the same way.
    #[test]
    fn pulse_swells_and_spin_turns() {
        for i in 0..50 {
            assert!(at(LoopMotion::Pulse, i as f32 * 0.05).transform.scale.x >= 1.0 - 1e-6);
        }
        assert!(
            at(LoopMotion::Spin, 2.0).transform.rotation_degrees
                > at(LoopMotion::Spin, 1.0).transform.rotation_degrees
        );
    }

    /// A loop is kept through sanitizing, and a title with only a loop is
    /// not "no animation".
    #[test]
    fn a_loop_survives_sanitizing() {
        let animation = TextAnimation {
            scroll: None,
            intro: None,
            outro: None,
            looping: Some(LoopMotion::Float),
        };
        assert_eq!(animation.sanitized().looping, Some(LoopMotion::Float));
        assert!(!animation.is_none());
    }
}
