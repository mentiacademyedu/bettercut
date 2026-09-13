//! The template file, as written (§30).
//!
//! What an author puts in a `.json` file, before anything has been checked.
//! Kept separate from [`crate::Template`] — the validated form — because the
//! two have different jobs: this one has to accept whatever an author typed so
//! the validator can say *what* is wrong with it, and that one has to be
//! impossible to hold in an invalid state.
//!
//! Times are seconds as `f64`. §30 allows it "for authoring convenience", on
//! the condition that they are converted to ticks on load and never used as
//! authoritative timing — so they exist only here, and the validated form holds
//! `TimelineTime`.
//!
//! **Unknown fields are refused**, deliberately. A template written for a
//! newer version may carry an element this one does not understand; accepting
//! it and ignoring the part it does not recognise would render something other
//! than what its author made, silently. §65 lists "unsupported operations" as
//! something to reject.

use serde::Deserialize;

/// A whole template file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateFile {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub category: String,
    /// Seconds.
    pub duration: f64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub slots: Vec<SlotFile>,
    #[serde(default)]
    pub elements: Vec<ElementFile>,
}

/// Something the user fills in (§31).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotFile {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// What the interface shows beside the slot: "Your main shot", "Headline".
    #[serde(default)]
    pub label: String,
    /// For a text slot: what it says until the user replaces it.
    #[serde(default)]
    pub default_text: Option<String>,
}

/// One thing the template places on the timeline.
///
/// Tagged by `type`, and the set is closed: an element type this version has
/// never heard of fails to parse rather than being skipped.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ElementFile {
    /// A video or image slot's media, placed on a video track.
    Clip {
        slot: String,
        start: f64,
        duration: f64,
        #[serde(default)]
        track: usize,
        #[serde(default)]
        transform: TransformFile,
        /// §22's crop, as `[left, top, right, bottom]` in fractions of the
        /// source. Beside the transform rather than inside it, because that is
        /// where it sits on a clip — §22 runs it *before* the transform, and
        /// nesting it would suggest otherwise. A title has none, which is why
        /// this is on the clip element alone.
        #[serde(default)]
        crop: Option<[f32; 4]>,
        #[serde(default)]
        opacity: Option<f32>,
        #[serde(default)]
        speed: Option<f64>,
        #[serde(default)]
        transition_out: Option<TransitionFile>,
        /// A slow move across the shot: `zoom_in`, `zoom_out` or `none`.
        #[serde(default)]
        movement: Option<String>,
        /// How the shot arrives and leaves, in the same shape a title's
        /// animation takes — the presets are the same ones.
        #[serde(default)]
        animation: Option<AnimationFile>,
    },
    /// Text: from a text slot, or fixed wording the user cannot change.
    Text {
        #[serde(default)]
        slot: Option<String>,
        #[serde(default)]
        text: Option<String>,
        start: f64,
        duration: f64,
        #[serde(default)]
        transform: TransformFile,
        /// Only the settings that differ from a new title's; see
        /// `validate::style_of`. Raw JSON here so a partial style parses.
        #[serde(default)]
        style: Option<serde_json::Value>,
        /// How it arrives and leaves.
        #[serde(default)]
        animation: Option<AnimationFile>,
    },
    /// An audio slot's media, placed on an audio track.
    Audio {
        slot: String,
        start: f64,
        duration: f64,
        #[serde(default)]
        track: usize,
        #[serde(default)]
        volume: Option<f32>,
        /// Seconds.
        #[serde(default)]
        fade_in: Option<f64>,
        /// Seconds.
        #[serde(default)]
        fade_out: Option<f64>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformFile {
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    /// Uniform: a template that squashes the user's footage is a mistake.
    #[serde(default)]
    pub scale: Option<f32>,
    /// Degrees, clockwise on screen.
    #[serde(default)]
    pub rotation: Option<f32>,
    /// Mirror left-to-right. No range to check: a flag is already in range.
    #[serde(default)]
    pub flip_h: bool,
    /// Mirror top-to-bottom.
    #[serde(default)]
    pub flip_v: bool,
}

/// A title's entrance and exit: `{ "in": {...}, "out": {...} }`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationFile {
    #[serde(default, rename = "in")]
    pub intro: Option<MotionFile>,
    #[serde(default, rename = "out")]
    pub outro: Option<MotionFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionFile {
    /// One of `MotionKind`'s names: `fade`, `slide_up`, `pop`, `spin` and so
    /// on. A title may also use `typewriter`; a picture has no letters to
    /// reveal, so it may not.
    pub kind: String,
    /// Seconds.
    pub duration: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionFile {
    pub kind: String,
    /// Seconds.
    pub duration: f64,
}
