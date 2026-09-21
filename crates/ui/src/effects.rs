//! What a clip is having done to it, named once.
//!
//! Two places ask the same question and used to answer it separately: the
//! timeline draws a badge for every effect on a clip, and the Inspector's
//! Effects tab opens the sections that are in use. Two lists of the same four
//! things drift — an effect added to one and not the other is a clip that looks
//! plain on the timeline while its controls sit there set.
//!
//! So the list lives here, the predicates with it, and both callers walk it.

use bettercut_editor_core::timeline::{BlendMode, Reflection, VideoClip};

/// One thing on the Effects tab.
///
/// Ordered as the tab is: the animation first, because it is the one a phone
/// editor puts on nearly every clip, then the three that are occasional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Animation,
    Blend,
    Mask,
    ChromaKey,
    Glitch,
    Mirror,
}

impl Effect {
    pub const ALL: [Self; 6] = [
        Self::Animation,
        Self::Blend,
        Self::Mask,
        Self::ChromaKey,
        Self::Glitch,
        Self::Mirror,
    ];

    /// The section's heading in the Inspector.
    pub fn title(self) -> &'static str {
        match self {
            Self::Animation => "Animation",
            Self::Blend => "Blend",
            Self::Mask => "Mask",
            Self::ChromaKey => "Green screen",
            Self::Glitch => "Glitch",
            Self::Mirror => "Mirror & kaleidoscope",
        }
    }

    /// The short name on the timeline badge, or `None` when the clip is not
    /// using this effect.
    ///
    /// Short because it is drawn inside the clip: a clip narrower than its own
    /// labels tells the user nothing, and three of these can be on at once.
    pub fn badge(self, clip: &VideoClip) -> Option<&'static str> {
        match self {
            // The animation section holds the motion-blur switch too, so it
            // is in use when either is.
            Self::Animation => {
                if !clip.motion.is_none() {
                    Some("anim")
                } else {
                    clip.motion_blur.then_some("blur")
                }
            }
            // Normal is what a clip does without being asked, so it is not
            // something that has been *done* to it.
            Self::Blend => (clip.blend != BlendMode::Normal).then(|| clip.blend.label()),
            Self::Mask => clip.mask.is_some().then_some("mask"),
            Self::ChromaKey => clip.chroma_key.is_some().then_some("key"),
            Self::Glitch => (clip.rgb_split > 0.0
                || clip.glitch > 0.0
                || clip.pixelate > 0.0
                || clip.zoom_blur > 0.0
                || clip.glow > 0.0
                || clip.old_film > 0.0
                || clip.light_leak > 0.0
                || clip.beat_pulse > 0.0)
                .then_some("glitch"),
            Self::Mirror => (clip.reflection != Reflection::None).then_some("mirror"),
        }
    }

    /// Whether this clip is using the effect — the same question the badge
    /// answers, asked when the name is not wanted.
    pub fn active(self, clip: &VideoClip) -> bool {
        self.badge(clip).is_some()
    }
}

/// Every effect on a clip, in tab order. Empty for an ordinary clip.
pub fn badges(clip: &VideoClip) -> Vec<&'static str> {
    Effect::ALL
        .iter()
        .filter_map(|effect| effect.badge(clip))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_editor_core::foundation::{MediaId, MediaTime, TimelineTime};
    use bettercut_editor_core::timeline::{
        ChromaKey, ClipMotion, Mask, Motion, MotionKind, SourceRange,
    };

    fn clip() -> VideoClip {
        VideoClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).expect("range"),
        )
        .expect("clip")
    }

    #[test]
    fn an_ordinary_clip_has_nothing_to_say() {
        assert!(badges(&clip()).is_empty());
        assert!(Effect::ALL.iter().all(|effect| !effect.active(&clip())));
    }

    /// The animation section carries the motion-blur switch as well, so a clip
    /// using only that still counts as using the section — otherwise the tab
    /// opens folded over a setting that is on.
    #[test]
    fn motion_blur_alone_counts_as_an_effect() {
        let mut clip = clip();
        clip.motion_blur = true;

        assert_eq!(Effect::Animation.badge(&clip), Some("blur"));
        assert!(Effect::Animation.active(&clip));
    }

    /// Normal is what a clip does anyway. Badging it would put a badge on every
    /// clip on the timeline, which is the same as badging none of them.
    #[test]
    fn the_ordinary_blend_mode_is_not_an_effect() {
        let mut clip = clip();
        clip.blend = BlendMode::Normal;
        assert_eq!(Effect::Blend.badge(&clip), None);

        clip.blend = BlendMode::Screen;
        assert_eq!(Effect::Blend.badge(&clip), Some(BlendMode::Screen.label()));
    }

    #[test]
    fn every_effect_is_named_when_it_is_on() {
        let mut clip = clip();
        clip.motion = ClipMotion {
            intro: Some(Motion::new(MotionKind::Fade, TimelineTime::from_seconds(1))),
            outro: None,
        };
        clip.blend = BlendMode::Multiply;
        clip.mask = Some(Mask::default());
        clip.chroma_key = Some(ChromaKey::default());
        clip.rgb_split = 20.0;
        clip.reflection = Reflection::FourWay;

        assert_eq!(
            badges(&clip),
            vec![
                "anim",
                BlendMode::Multiply.label(),
                "mask",
                "key",
                "glitch",
                "mirror"
            ],
            "an effect went unnamed, or they came out in the wrong order"
        );
        assert!(Effect::ALL.iter().all(|effect| effect.active(&clip)));
    }

    /// The badges are drawn inside a clip on the timeline, which can be forty
    /// pixels wide. A long name there is a name nobody reads.
    #[test]
    fn the_names_stay_short_enough_to_draw_on_a_clip() {
        let mut clip = clip();
        clip.mask = Some(Mask::default());
        clip.chroma_key = Some(ChromaKey::default());
        clip.motion = ClipMotion {
            intro: Some(Motion::new(MotionKind::Spin, TimelineTime::from_seconds(1))),
            outro: None,
        };
        for mode in BlendMode::ALL {
            clip.blend = mode;
            for name in badges(&clip) {
                assert!(name.len() <= 8, "badge {name:?} is too long to draw");
            }
        }
    }
}
