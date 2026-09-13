//! Applying a template to the timeline (§31, §77).
//!
//! ```text
//! Template file → validator → template engine → editor commands → timeline
//! ```
//!
//! This is the "template engine" box. It reads a [`Template`] that has already
//! passed [`bettercut_templates::validate`], and what the user put in its slots,
//! and turns the two into ordinary editor commands — the same `AddClip`,
//! `AddText` and `SetTransition` a user's own clicks produce. It never touches
//! the project itself (§77), so everything it does is undone by one Ctrl+Z,
//! journalled for crash recovery, and indistinguishable afterwards from an edit
//! made by hand.

use std::collections::{BTreeSet, HashMap};

use bettercut_foundation::{
    ClipId, MediaId, MediaTime, Rational, SequenceId, TimelineTime, TrackId,
};
use bettercut_templates::{Element, SlotKind, Template};
use bettercut_timeline::{
    AudioClip, MIN_TRANSITION, SourceRange, TextClip, TimelineRange, TrackKind, Transition,
    TransitionKind, VideoClip, timeline_ticks_for,
};

use crate::command::{ClipPayload, Command, TrackKindRepr};
use crate::editor::{Editor, Stage};
use crate::error::EditorError;
use crate::ops;

/// What the user put in one slot (§31: "Drop media into slots. Replace text.").
#[derive(Debug, Clone, PartialEq)]
pub enum SlotFill {
    Media(MediaId),
    Text(String),
}

/// What applying a template did, for the interface to report.
///
/// Every one of these is something the user would otherwise discover by
/// watching the result and wondering why part of the template is missing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppliedTemplate {
    /// Everything placed, in template order.
    pub clips: Vec<ClipId>,
    /// Media slots left empty. Their elements were skipped, not filled with
    /// something the user did not choose.
    pub unfilled: Vec<String>,
    /// Slots whose media was shorter than the template's shot, and which
    /// therefore end early rather than freezing or looping.
    pub shortened: Vec<String>,
    /// Transitions the footage had no room for.
    pub dropped_transitions: usize,
}

/// A clip placed in the first pass, waiting to find out whether its
/// transition fits once its neighbour is down.
struct PendingTransition {
    clip: ClipId,
    track: TrackId,
    kind: TransitionKind,
    wanted: TimelineTime,
}

impl Editor {
    /// Put `template` on the timeline starting at `at`, as one undo step.
    ///
    /// `fills` maps slot ids to what the user supplied. Every fill is checked
    /// against its slot before anything changes; a text slot left out keeps
    /// its default wording, and a media slot left out skips its elements.
    ///
    /// Element track numbers count from the bottom of the sequence's existing
    /// tracks, and tracks are added when the template needs more. Titles find
    /// their own lane, adding one when every existing lane is busy — a template
    /// cannot know what titles the user already has.
    ///
    /// Fails without changing anything if a fill does not fit its slot, or if
    /// the template's space on a track is already occupied.
    pub fn apply_template(
        &mut self,
        template: &Template,
        fills: &HashMap<String, SlotFill>,
        at: TimelineTime,
    ) -> Result<AppliedTemplate, EditorError> {
        let sequence = self.active_sequence_id()?;
        self.check_fills(template, fills)?;
        let at = self.snap(at);

        self.staged(format!("Apply {}", template.name), |editor, stage| {
            let mut applied = AppliedTemplate::default();
            let mut unfilled = BTreeSet::new();
            let mut shortened = BTreeSet::new();
            let mut pending = Vec::new();

            for element in &template.elements {
                let (start, end) = editor.span(at, element.start(), element.duration());
                match element {
                    Element::Clip {
                        slot,
                        track,
                        transform,
                        crop,
                        opacity,
                        speed,
                        transition_out,
                        movement,
                        motion,
                        ..
                    } => {
                        let Some(SlotFill::Media(media)) = fills.get(slot) else {
                            unfilled.insert(slot.clone());
                            continue;
                        };
                        let track_id =
                            editor.template_track(stage, sequence, TrackKind::Video, *track)?;
                        editor.require_room(sequence, track_id, start, end)?;

                        let (source, short) = editor.source_for(*media, end - start, *speed)?;
                        if short {
                            shortened.insert(slot.clone());
                        }
                        let mut clip = VideoClip::new(*media, start, source)?;
                        clip.speed = *speed;
                        clip.timeline = TimelineRange::new(
                            start,
                            start
                                + TimelineTime::from_ticks(timeline_ticks_for(
                                    source.duration(),
                                    *speed,
                                )),
                        )?;
                        clip.crop = *crop;
                        clip.transform = *transform;
                        clip.opacity = *opacity;
                        // The same entrance the Inspector sets, so a
                        // template's animation and a user's are one thing.
                        clip.motion = *motion;
                        // The same keys the Movement buttons write (§24), from
                        // the same place, so a template's zoom and a user's are
                        // one thing.
                        for (parameter, key) in movement.keyframes(clip.transform.scale, source) {
                            clip.keyframes.set(parameter, key);
                        }
                        let id = clip.id;

                        editor.stage(
                            stage,
                            Command::AddClip {
                                sequence,
                                track: track_id,
                                clip: ClipPayload::Video(Box::new(clip)),
                            },
                        )?;
                        applied.clips.push(id);
                        if let Some((kind, wanted)) = transition_out {
                            pending.push(PendingTransition {
                                clip: id,
                                track: track_id,
                                kind: *kind,
                                wanted: *wanted,
                            });
                        }
                    }

                    Element::Audio {
                        slot,
                        track,
                        volume,
                        fade_in,
                        fade_out,
                        ..
                    } => {
                        let Some(SlotFill::Media(media)) = fills.get(slot) else {
                            unfilled.insert(slot.clone());
                            continue;
                        };
                        let track_id =
                            editor.template_track(stage, sequence, TrackKind::Audio, *track)?;
                        editor.require_room(sequence, track_id, start, end)?;

                        let (source, short) =
                            editor.source_for(*media, end - start, Rational::ONE)?;
                        if short {
                            shortened.insert(slot.clone());
                        }
                        let mut clip = AudioClip::new(*media, start, source)?;
                        clip.gain = *volume;
                        clip.fade_in = *fade_in;
                        clip.fade_out = *fade_out;
                        let id = clip.id;

                        editor.stage(
                            stage,
                            Command::AddClip {
                                sequence,
                                track: track_id,
                                clip: ClipPayload::Audio(Box::new(clip)),
                            },
                        )?;
                        applied.clips.push(id);
                    }

                    Element::Text {
                        slot,
                        text,
                        transform,
                        style,
                        animation,
                        ..
                    } => {
                        let words = match slot.as_ref().and_then(|s| fills.get(s)) {
                            Some(SlotFill::Text(words)) => words.clone(),
                            _ => text.clone(),
                        };
                        // Nothing to draw. Not an error: clearing a title's
                        // box is a reasonable way to say "no title here".
                        if words.trim().is_empty() {
                            continue;
                        }
                        let track_id = editor.text_lane(stage, sequence, start, end)?;

                        let mut clip = TextClip::with_duration(words, start, end - start)?;
                        clip.style = style.clone();
                        clip.animation = *animation;
                        clip.transform = *transform;
                        let id = clip.id;

                        editor.stage(
                            stage,
                            Command::AddText {
                                sequence,
                                track: track_id,
                                clip: Box::new(clip),
                            },
                        )?;
                        applied.clips.push(id);
                    }
                }
            }

            // Second pass: whether a transition fits depends on the clip after
            // it, which may not have been placed when this one was.
            for pending in pending {
                let room = ops::transition_room(
                    editor.project(),
                    sequence,
                    pending.track,
                    pending.clip,
                    pending.kind,
                );
                let length = room.map(|room| room.min(pending.wanted));
                match length {
                    Some(length) if length >= MIN_TRANSITION => editor.stage(
                        stage,
                        Command::SetTransition {
                            sequence,
                            track: pending.track,
                            clip: pending.clip,
                            transition: Some(Transition::new(pending.kind, length)),
                        },
                    )?,
                    _ => applied.dropped_transitions += 1,
                }
            }

            applied.unfilled = unfilled.into_iter().collect();
            applied.shortened = shortened.into_iter().collect();
            Ok(applied)
        })
    }

    /// Check every fill against its slot before anything is changed.
    fn check_fills(
        &self,
        template: &Template,
        fills: &HashMap<String, SlotFill>,
    ) -> Result<(), EditorError> {
        let refuse = |slot: &str, reason| EditorError::TemplateFill {
            slot: slot.to_owned(),
            reason,
        };

        for (id, fill) in fills {
            let slot = template
                .slot(id)
                .ok_or_else(|| refuse(id, "this template has no such slot"))?;
            let label = slot.label.as_str();

            match (slot.kind, fill) {
                (SlotKind::Text, SlotFill::Text(words)) => {
                    if words.chars().count() > bettercut_templates::validate::MAX_TEXT_CHARS {
                        return Err(refuse(label, "that is too much text for one title"));
                    }
                }
                (SlotKind::Text, SlotFill::Media(_)) => {
                    return Err(refuse(label, "this slot takes words, not a file"));
                }
                (_, SlotFill::Text(_)) => {
                    return Err(refuse(label, "this slot takes a file, not words"));
                }
                (kind, SlotFill::Media(media)) => {
                    let asset = self
                        .project()
                        .media_asset(*media)
                        .ok_or(EditorError::MediaNotFound(*media))?;
                    if kind == SlotKind::Audio {
                        if asset.audio_codec.is_none() {
                            return Err(refuse(label, "that file has no sound"));
                        }
                    } else if !asset.kind.has_video() {
                        return Err(refuse(label, "that file has no picture"));
                    }
                    // A photo fills any picture slot for as long as it asks.
                    if !asset.is_still() && asset.duration <= MediaTime::ZERO {
                        return Err(refuse(label, "that file has no duration"));
                    }
                }
            }
        }
        Ok(())
    }

    fn snap(&self, at: TimelineTime) -> TimelineTime {
        let at = at.max(TimelineTime::ZERO);
        self.active_sequence().map_or(at, |s| s.snap_to_frame(at))
    }

    /// Where an element lands, both ends on the frame grid (§9, §76).
    ///
    /// Snapped end and start separately, rather than snapping the start and
    /// keeping the length, so two elements the author butted together stay
    /// butted together. Never shorter than one frame.
    fn span(
        &self,
        at: TimelineTime,
        start: TimelineTime,
        duration: TimelineTime,
    ) -> (TimelineTime, TimelineTime) {
        let begin = self.snap(at + start);
        let end = self.snap(at + start + duration);
        let frame = self
            .active_sequence()
            .map_or(1, |s| s.ticks_per_frame().max(1));
        (begin, end.max(begin + TimelineTime::from_ticks(frame)))
    }

    /// The source range that fills `length` of timeline at `speed`, and
    /// whether the media ran out first.
    fn source_for(
        &self,
        media: MediaId,
        length: TimelineTime,
        speed: Rational,
    ) -> Result<(SourceRange, bool), EditorError> {
        let limit = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?
            .source_limit();
        let wanted = MediaTime::from_ticks(speed.scale(length.ticks()));
        // A still never runs out.
        let used = limit.map_or(wanted, |available| wanted.min(available));
        Ok((SourceRange::new(MediaTime::ZERO, used)?, used < wanted))
    }

    /// The `index`th track of `kind`, adding tracks until there is one.
    fn template_track(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        kind: TrackKind,
        index: usize,
    ) -> Result<TrackId, EditorError> {
        loop {
            let tracks: Vec<TrackId> = match (self.active_sequence(), kind) {
                (Some(s), TrackKind::Video) => s.video_tracks.iter().map(|t| t.id).collect(),
                (Some(s), TrackKind::Audio) => s.audio_tracks.iter().map(|t| t.id).collect(),
                (Some(s), TrackKind::Text) => s.text_tracks.iter().map(|t| t.id).collect(),
                (Some(s), TrackKind::Adjustment) => {
                    s.adjustment_tracks.iter().map(|t| t.id).collect()
                }
                (None, _) => return Err(EditorError::SequenceNotFound(sequence)),
            };
            if let Some(id) = tracks.get(index) {
                return Ok(*id);
            }
            self.add_track_staged(stage, sequence, kind, tracks.len() + 1)?;
        }
    }

    /// The first text lane free over `[start, end)`, adding one if none is.
    fn text_lane(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        start: TimelineTime,
        end: TimelineTime,
    ) -> Result<TrackId, EditorError> {
        let (free, count) = {
            let s = self
                .active_sequence()
                .ok_or(EditorError::SequenceNotFound(sequence))?;
            let free = s
                .text_tracks
                .iter()
                // Captions have a lane of their own, and a template's title
                // dropped among them would be replaced by the next import.
                .filter(|t| t.name != Self::CAPTION_TRACK)
                .find(|t| {
                    !t.clips()
                        .iter()
                        .any(|c| c.timeline.start < end && c.timeline.end > start)
                })
                .map(|t| t.id);
            (free, s.text_tracks.len())
        };
        match free {
            Some(id) => Ok(id),
            None => self.add_track_staged(stage, sequence, TrackKind::Text, count + 1),
        }
    }

    fn add_track_staged(
        &mut self,
        stage: &mut Stage,
        sequence: SequenceId,
        kind: TrackKind,
        number: usize,
    ) -> Result<TrackId, EditorError> {
        let (repr, letter) = match kind {
            TrackKind::Video => (TrackKindRepr::Video, 'V'),
            TrackKind::Audio => (TrackKindRepr::Audio, 'A'),
            TrackKind::Text => (TrackKindRepr::Text, 'T'),
            TrackKind::Adjustment => (TrackKindRepr::Adjustment, 'F'),
        };
        let id = TrackId::new();
        self.stage(
            stage,
            Command::AddTrack {
                sequence,
                kind: repr,
                name: format!("{letter}{number}"),
                id,
            },
        )?;
        Ok(id)
    }

    /// Refuse, rather than shuffle the user's clips, when the template's space
    /// on a track is taken.
    fn require_room(
        &self,
        sequence: SequenceId,
        track: TrackId,
        start: TimelineTime,
        end: TimelineTime,
    ) -> Result<(), EditorError> {
        let s = self
            .active_sequence()
            .ok_or(EditorError::SequenceNotFound(sequence))?;
        let busy = |r: TimelineRange| r.start < end && r.end > start;
        let occupied = if let Some(t) = s.video_track(track) {
            t.clips().iter().any(|c| busy(c.timeline))
        } else if let Some(t) = s.audio_track(track) {
            t.clips().iter().any(|c| busy(c.timeline))
        } else {
            return Err(EditorError::TrackNotFound(track));
        };
        if occupied {
            Err(EditorError::NoRoomForTemplate)
        } else {
            Ok(())
        }
    }
}
