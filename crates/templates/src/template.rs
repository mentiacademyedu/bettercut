//! A template that has passed validation (§65).
//!
//! Everything here is known-good: times are ticks, every element's slot exists
//! and is the right kind, every number is inside the range the editor's own
//! controls allow. The only way to get one is [`crate::validate`].

use bettercut_foundation::{Rational, TimelineTime};
use bettercut_text::TextStyle;
use bettercut_timeline::{Movement, TextAnimation, Transform, TransitionKind};

/// What kind of media a slot takes (§31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Video,
    Image,
    Text,
    Audio,
    /// An image, placed where a brand mark goes. The same media as `Image`,
    /// kept distinct because the interface asks for it differently.
    Logo,
}

impl SlotKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "video" => Self::Video,
            "image" => Self::Image,
            "text" => Self::Text,
            "audio" => Self::Audio,
            "logo" => Self::Logo,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Image => "Image",
            Self::Text => "Text",
            Self::Audio => "Audio",
            Self::Logo => "Logo",
        }
    }

    /// Whether this slot's media is a picture that goes on a video track.
    pub fn is_picture(self) -> bool {
        matches!(self, Self::Video | Self::Image | Self::Logo)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub id: String,
    pub kind: SlotKind,
    pub label: String,
    pub default_text: Option<String>,
}

/// One validated element.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    Clip {
        slot: String,
        start: TimelineTime,
        duration: TimelineTime,
        track: usize,
        transform: Transform,
        /// §22's crop. `Crop::NONE` for a template that asks for none.
        crop: bettercut_timeline::Crop,
        opacity: f32,
        speed: Rational,
        transition_out: Option<(TransitionKind, TimelineTime)>,
        movement: Movement,
        /// The entrance and exit. Neither, for a template that asks for none.
        motion: bettercut_timeline::ClipMotion,
    },
    Text {
        slot: Option<String>,
        text: String,
        start: TimelineTime,
        duration: TimelineTime,
        transform: Transform,
        style: TextStyle,
        animation: TextAnimation,
    },
    Audio {
        slot: String,
        start: TimelineTime,
        duration: TimelineTime,
        track: usize,
        volume: f32,
        fade_in: TimelineTime,
        fade_out: TimelineTime,
    },
}

impl Element {
    pub fn start(&self) -> TimelineTime {
        match self {
            Self::Clip { start, .. } | Self::Text { start, .. } | Self::Audio { start, .. } => {
                *start
            }
        }
    }

    pub fn duration(&self) -> TimelineTime {
        match self {
            Self::Clip { duration, .. }
            | Self::Text { duration, .. }
            | Self::Audio { duration, .. } => *duration,
        }
    }

    pub fn slot(&self) -> Option<&str> {
        match self {
            Self::Clip { slot, .. } | Self::Audio { slot, .. } => Some(slot),
            Self::Text { slot, .. } => slot.as_deref(),
        }
    }
}

/// A template, ready to apply (§29–§31).
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub duration: TimelineTime,
    pub slots: Vec<Slot>,
    pub elements: Vec<Element>,
}

impl Template {
    pub fn slot(&self, id: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.id == id)
    }
}
