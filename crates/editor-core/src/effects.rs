//! The picture effects by name — glitch, glow, blur and the rest — with what
//! each is for, how to set it and how to read it off a clip. One list for the
//! left panel's Effects tab and for an assistant's `set_effect`, so the two
//! cannot disagree about what an effect is called or how strong "50" is.

use bettercut_timeline::VideoClip;

use crate::command::ClipProperty;

/// One effect.
#[derive(Debug, Clone, Copy)]
pub struct NamedEffect {
    pub name: &'static str,
    pub description: &'static str,
    /// The property, at an amount in the clip's own units.
    pub make: fn(f32) -> ClipProperty,
    /// The amount on the clip, in the same units.
    pub read: fn(&VideoClip) -> f32,
    /// Clip units per point on a 0–100 scale: most effects are 0–100 already,
    /// a few (vignette, smooth skin) are 0–1.
    pub scale: f32,
}

impl NamedEffect {
    /// How strong it is on `clip`, 0–100.
    pub fn amount_on(&self, clip: &VideoClip) -> f32 {
        (self.read)(clip) / self.scale
    }

    /// The property that sets it to `amount`, 0–100.
    pub fn at(&self, amount: f32) -> ClipProperty {
        (self.make)(amount.clamp(0.0, 100.0) * self.scale)
    }

    /// Found by name as anyone might write it: any case, spaces or none.
    pub fn named(name: &str) -> Option<&'static Self> {
        let plain = |s: &str| {
            s.chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        };
        EFFECTS.iter().find(|e| plain(e.name) == plain(name))
    }
}

/// The strength an effect is switched on at with one click: clearly there,
/// with room to go either way in the inspector.
pub const ONE_CLICK_AMOUNT: f32 = 50.0;

/// Every effect, in the order they are offered.
pub const EFFECTS: [NamedEffect; 13] = [
    NamedEffect {
        name: "Glitch",
        description: "Blocks of the picture torn sideways, flickering",
        make: ClipProperty::Glitch,
        read: |c| c.glitch,
        scale: 1.0,
    },
    NamedEffect {
        name: "RGB split",
        description: "Red, green and blue pulled apart at the edges",
        make: ClipProperty::RgbSplit,
        read: |c| c.rgb_split,
        scale: 1.0,
    },
    NamedEffect {
        name: "Glow",
        description: "Bright parts bloom softly into what is around them",
        make: ClipProperty::Glow,
        read: |c| c.glow,
        scale: 1.0,
    },
    NamedEffect {
        name: "Old film",
        description: "Scratches, dust and a flickering, faded print",
        make: ClipProperty::OldFilm,
        read: |c| c.old_film,
        scale: 1.0,
    },
    NamedEffect {
        name: "Light leak",
        description: "Warm light drifting in from the edge, like film fogged by the sun",
        make: ClipProperty::LightLeak,
        read: |c| c.light_leak,
        scale: 1.0,
    },
    NamedEffect {
        name: "Lens flare",
        description: "A streak and rings of light across the lens",
        make: ClipProperty::LensFlare,
        read: |c| c.lens_flare,
        scale: 1.0,
    },
    NamedEffect {
        name: "Zoom blur",
        description: "Streaks rushing out from the middle, a sense of speed",
        make: ClipProperty::ZoomBlur,
        read: |c| c.zoom_blur,
        scale: 1.0,
    },
    NamedEffect {
        name: "Beat pulse",
        description: "The picture punches in on every marker: put markers on the beat",
        make: ClipProperty::BeatPulse,
        read: |c| c.beat_pulse,
        scale: 1.0,
    },
    NamedEffect {
        name: "Pixelate",
        description: "Big square blocks, as for hiding something",
        make: ClipProperty::Pixelate,
        read: |c| c.pixelate,
        scale: 1.0,
    },
    NamedEffect {
        name: "Blur",
        description: "The whole picture soft",
        make: ClipProperty::Blur,
        read: |c| c.blur,
        scale: 1.0,
    },
    NamedEffect {
        name: "Sharpen",
        description: "Edges crisper, for soft footage",
        make: ClipProperty::Sharpen,
        read: |c| c.sharpen,
        scale: 1.0,
    },
    NamedEffect {
        name: "Vignette",
        description: "The corners darkened, drawing the eye in",
        make: ClipProperty::Vignette,
        read: |c| c.vignette,
        scale: 0.01,
    },
    NamedEffect {
        name: "Smooth skin",
        description: "Skin softened, the rest left sharp",
        make: ClipProperty::SmoothSkin,
        read: |c| c.smooth_skin,
        scale: 0.01,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaId, MediaTime, TimelineTime};
    use bettercut_timeline::SourceRange;

    fn clip() -> VideoClip {
        VideoClip::new(
            MediaId::new(),
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(2)).unwrap(),
        )
        .unwrap()
    }

    /// What an effect is set to is what reading it back says, on the same
    /// 0–100 scale, for every effect — the scales cannot drift apart.
    #[test]
    fn each_effect_reads_back_what_it_set() {
        let (mut editor, _events) = crate::Editor::new_project("Effects");
        let media = editor.import_media(bettercut_media::MediaAsset::new(
            bettercut_media::MediaKind::Video,
            "C:/media/a.mp4",
            MediaTime::from_seconds(4),
        ));
        editor.place_media(media).unwrap();
        let id = editor.active_sequence().unwrap().video_tracks[0].clips()[0].id;
        for effect in &EFFECTS {
            assert_eq!(
                effect.amount_on(&clip()),
                0.0,
                "{} is on by default",
                effect.name
            );
            editor
                .set_clip_property(id, effect.at(40.0), false)
                .unwrap();
            let on = editor.video_clip(id).unwrap();
            assert!(
                (effect.amount_on(on) - 40.0).abs() < 0.01,
                "{}: set 40, read {}",
                effect.name,
                effect.amount_on(on)
            );
            editor.set_clip_property(id, effect.at(0.0), false).unwrap();
        }
    }

    #[test]
    fn effects_are_found_however_they_are_written() {
        assert_eq!(NamedEffect::named("rgb split").unwrap().name, "RGB split");
        assert_eq!(NamedEffect::named("RGBSplit").unwrap().name, "RGB split");
        assert_eq!(NamedEffect::named("old-film").unwrap().name, "Old film");
        assert!(NamedEffect::named("sparkles").is_none());
    }
}
