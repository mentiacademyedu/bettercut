//! §33's auto slideshow: a pile of photos, turned into an edit.
//!
//! # Why this is a template
//!
//! A slideshow is a run of shots, each held for a while, with a transition at
//! every cut and a slow move across each picture so it does not sit dead on
//! screen. That is exactly what a template is, and the template engine already
//! knows how to lay one down: it finds or makes the tracks, refuses to write
//! over what is already there, fits each transition to the handles the footage
//! actually has, and does the whole thing as one undo step (§77, §79).
//!
//! So this builds a [`Template`] in memory and hands it to
//! [`Editor::apply_template`] rather than placing clips itself. Writing a second
//! placement path would mean two answers to "what happens when the transition
//! does not fit" — and they would not stay the same.
//!
//! # What it does not do
//!
//! It does not choose the pictures or their order. §33 lists the automation,
//! not a judgement about which photo is best; the user selected these, in this
//! order, and the slideshow respects that.

use bettercut_foundation::{MediaId, TimelineTime};
use bettercut_templates::{Element, Slot, SlotKind, Template};
use bettercut_timeline::{ClipMotion, Movement, Transform, TransitionKind};

use crate::editor::Editor;
use crate::error::EditorError;
use crate::template_apply::{AppliedTemplate, SlotFill};

/// The shortest a picture may be held.
///
/// Below about a second a slideshow stops reading as one and starts reading as
/// a flicker, and the transition at each end has nowhere to live.
pub const MIN_HOLD: TimelineTime = TimelineTime::from_seconds(1);

/// The longest, which is where a slideshow becomes a set of stills that happen
/// to follow each other.
pub const MAX_HOLD: TimelineTime = TimelineTime::from_seconds(30);

/// How each picture is held and left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slideshow {
    /// How long each picture is on screen, before transitions overlap it.
    pub hold: TimelineTime,
    /// The cut between one picture and the next. `None` for hard cuts.
    pub transition: Option<TransitionKind>,
    /// How long that transition runs.
    pub transition_length: TimelineTime,
    /// A slow move across each picture (§24's keyframes, via `Movement`).
    pub movement: Movement,
}

impl Default for Slideshow {
    fn default() -> Self {
        Self {
            hold: TimelineTime::from_seconds(3),
            transition: Some(TransitionKind::Crossfade),
            transition_length: bettercut_timeline::transition::DEFAULT_DURATION,
            movement: Movement::ZoomIn,
        }
    }
}

impl Slideshow {
    /// Brought into range, so a value from an interface or a script cannot ask
    /// for something the editor's own controls could not produce.
    ///
    /// The transition is held to *half* the hold as well as to its own minimum:
    /// a transition longer than the picture it joins would overlap the one
    /// beyond it, and §25's handles are measured from a clip that is still
    /// there when the next one starts.
    pub fn clamped(self) -> Self {
        let hold =
            TimelineTime::from_ticks(self.hold.ticks().clamp(MIN_HOLD.ticks(), MAX_HOLD.ticks()));
        let longest = TimelineTime::from_ticks(hold.ticks() / 2);
        let length = TimelineTime::from_ticks(
            self.transition_length.ticks().clamp(
                bettercut_timeline::transition::MIN_DURATION.ticks(),
                longest
                    .ticks()
                    .max(bettercut_timeline::transition::MIN_DURATION.ticks()),
            ),
        );
        Self {
            hold,
            transition: self.transition,
            transition_length: length,
            movement: self.movement,
        }
    }

    /// How long a slideshow of `pictures` runs.
    ///
    /// The transitions do not add to it: §25 overlaps the two clips rather than
    /// inserting between them, so the whole is the sum of the holds.
    pub fn total(self, pictures: usize) -> TimelineTime {
        TimelineTime::from_ticks(self.clamped().hold.ticks() * pictures as i64)
    }

    /// The template this slideshow is.
    ///
    /// Slots are named `picture_1` upwards, which is what
    /// [`Editor::build_slideshow`] fills them by.
    pub fn template(self, pictures: usize) -> Template {
        let plan = self.clamped();
        let mut slots = Vec::with_capacity(pictures);
        let mut elements = Vec::with_capacity(pictures);

        for index in 0..pictures {
            let id = slot_id(index);
            slots.push(Slot {
                id: id.clone(),
                // `Image` rather than `Video`: the slideshow is offered for
                // photos, and a slot's kind is checked against the fill.
                kind: SlotKind::Image,
                label: format!("Picture {}", index + 1),
                default_text: None,
            });

            elements.push(Element::Clip {
                slot: id,
                start: TimelineTime::from_ticks(plan.hold.ticks() * index as i64),
                duration: plan.hold,
                track: 0,
                transform: Transform::default(),
                crop: bettercut_timeline::Crop::NONE,
                opacity: 1.0,
                speed: bettercut_foundation::Rational::ONE,
                // The last picture has nothing to cut to.
                transition_out: plan
                    .transition
                    .filter(|_| index + 1 < pictures)
                    .map(|kind| (kind, plan.transition_length)),
                movement: plan.movement,
                motion: ClipMotion::default(),
            });
        }

        Template {
            id: "auto-slideshow".to_string(),
            name: "Slideshow".to_string(),
            category: "Automatic".to_string(),
            description: format!("{pictures} pictures, {} seconds each", seconds(plan.hold)),
            duration: plan.total(pictures),
            slots,
            elements,
        }
    }
}

fn slot_id(index: usize) -> String {
    format!("picture_{}", index + 1)
}

/// Whole seconds, for the description only — never for timing (§9, §74).
fn seconds(time: TimelineTime) -> i64 {
    time.ticks() / bettercut_foundation::TICKS_PER_SECOND
}

impl Editor {
    /// §33: build a slideshow from `pictures`, in the order given, at `at`.
    ///
    /// One undo step, because it goes through the template engine (§77).
    /// Fails without changing anything if the space is occupied or a picture is
    /// not a picture — the same failures applying any template has.
    pub fn build_slideshow(
        &mut self,
        pictures: &[MediaId],
        plan: Slideshow,
        at: TimelineTime,
    ) -> Result<AppliedTemplate, EditorError> {
        if pictures.is_empty() {
            return Err(EditorError::NoPicturesForSlideshow);
        }

        let template = plan.template(pictures.len());
        let fills = pictures
            .iter()
            .enumerate()
            .map(|(index, media)| (slot_id(index), SlotFill::Media(*media)))
            .collect();

        self.apply_template(&template, &fills, at)
    }
}
