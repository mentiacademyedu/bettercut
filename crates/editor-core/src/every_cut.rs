//! One transition on every cut of a picture lane.
//!
//! A montage often wants the same dissolve between every shot. Placing it cut
//! by cut is a dozen menus; this is one. Each cut gets the transition at the
//! usual length, or as long as the footage either side allows when that is
//! shorter; a cut with no room at all is left as a plain cut and counted, so
//! the result can say so. One undo step, as is taking them all away again.

use bettercut_foundation::{ClipId, TimelineTime, TrackId};
use bettercut_timeline::{MIN_TRANSITION, Transition, TransitionKind};

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

impl Editor {
    /// Put a `kind` transition on every cut of picture lane `track`. Returns
    /// how many cuts took one, and how many had no room for it.
    /// Put a crossfade of `length` on every join between sound clips on
    /// `track`, as one undo step. Returns how many took one and how many had
    /// no room — the same shape of answer as the picture's version, and for
    /// the same reason: a join with nothing to fade through is skipped
    /// rather than stopping the rest.
    ///
    /// A join is two clips touching. A gap is not a cut, and two clips that
    /// overlap already are left as they are.
    pub fn crossfade_every_cut(
        &mut self,
        track: TrackId,
        length: TimelineTime,
    ) -> Result<(usize, usize), EditorError> {
        let clips: Vec<ClipId> = self
            .active_sequence()
            .and_then(|s| s.audio_tracks.iter().find(|t| t.id == track))
            .map(|t| t.clips().iter().map(|c| c.id).collect())
            .unwrap_or_default();
        if clips.is_empty() {
            return Err(EditorError::TrackNotFound(track));
        }

        let mut done = 0;
        let mut skipped = 0;
        for clip in clips {
            // Each clip's own join with the one after it; the last has none.
            if self.next_touching_sound(clip).is_none() {
                continue;
            }
            match self.set_audio_crossfade(clip, length, false) {
                Ok(given) if given > TimelineTime::ZERO => done += 1,
                Ok(_) => skipped += 1,
                Err(_) => skipped += 1,
            }
        }
        Ok((done, skipped))
    }

    pub fn transition_every_cut(
        &mut self,
        track: TrackId,
        kind: TransitionKind,
    ) -> Result<(usize, usize), EditorError> {
        let house = self.project().settings.transition_length;
        let sequence = self.active_sequence_id()?;
        let clips: Vec<_> = self
            .active_sequence()
            .and_then(|s| s.video_track(track))
            .ok_or(EditorError::TrackNotFound(track))?
            .clips()
            .iter()
            .map(|clip| clip.id)
            .collect();

        let mut commands = Vec::new();
        let mut skipped = 0;
        for clip in clips {
            // `None`: no clip straight after — not a cut, nothing to count.
            let Some(room) = self.transition_room(clip, kind) else {
                continue;
            };
            if room < MIN_TRANSITION {
                skipped += 1;
                continue;
            }
            commands.push(Command::SetTransition {
                sequence,
                track,
                clip,
                transition: Some(Transition::new(kind, house.min(room))),
            });
        }
        let applied = commands.len();
        if applied > 0 {
            self.dispatch_group(format!("{} on Every Cut", kind.label()), commands)?;
        }
        Ok((applied, skipped))
    }

    /// The transitions a mixed pass takes in turn: lively, and each reading
    /// as a cut to somewhere new.
    pub const MIXED_TRANSITIONS: [TransitionKind; 6] = [
        TransitionKind::Crossfade,
        TransitionKind::Slide,
        TransitionKind::Zoom,
        TransitionKind::Push,
        TransitionKind::Wipe,
        TransitionKind::Flash,
    ];

    /// A different transition on each cut of picture lane `track`, taking
    /// [`Self::MIXED_TRANSITIONS`] in turn, at the house length (held to the
    /// footage each has). One undo step. Returns (applied, skipped).
    pub fn mixed_transition_every_cut(
        &mut self,
        track: TrackId,
    ) -> Result<(usize, usize), EditorError> {
        let house = self.project().settings.transition_length;
        let sequence = self.active_sequence_id()?;
        let clips: Vec<_> = self
            .active_sequence()
            .and_then(|s| s.video_track(track))
            .ok_or(EditorError::TrackNotFound(track))?
            .clips()
            .iter()
            .map(|clip| clip.id)
            .collect();
        let mut commands = Vec::new();
        let mut skipped = 0;
        let mut turn = 0;
        for clip in clips {
            let kind = Self::MIXED_TRANSITIONS[turn % Self::MIXED_TRANSITIONS.len()];
            let Some(room) = self.transition_room(clip, kind) else {
                continue; // not a cut
            };
            turn += 1;
            if room < MIN_TRANSITION {
                skipped += 1;
                continue;
            }
            commands.push(Command::SetTransition {
                sequence,
                track,
                clip,
                transition: Some(Transition::new(kind, house.min(room))),
            });
        }
        let applied = commands.len();
        if applied > 0 {
            self.dispatch_group("Mixed Transitions on Every Cut", commands)?;
        }
        Ok((applied, skipped))
    }

    /// Make every transition already on picture lane `track` last `length`,
    /// keeping its kind — each held to the footage it has to fade through.
    /// One undo step. Returns how many changed.
    pub fn retime_every_transition(
        &mut self,
        track: TrackId,
        length: TimelineTime,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let existing: Vec<(ClipId, Transition)> = self
            .active_sequence()
            .and_then(|s| s.video_track(track))
            .ok_or(EditorError::TrackNotFound(track))?
            .clips()
            .iter()
            .filter_map(|clip| clip.transition_out.map(|t| (clip.id, t)))
            .collect();
        let commands: Vec<Command> = existing
            .into_iter()
            .filter_map(|(clip, transition)| {
                let room = self.transition_room(clip, transition.kind)?;
                let wanted = length.max(MIN_TRANSITION).min(room);
                (wanted != transition.duration && wanted >= MIN_TRANSITION).then_some(
                    Command::SetTransition {
                        sequence,
                        track,
                        clip,
                        transition: Some(Transition::new(transition.kind, wanted)),
                    },
                )
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Transition Lengths", commands)?;
        }
        Ok(count)
    }

    /// Take every transition off picture lane `track`, as one undo step.
    /// Returns how many were removed.
    pub fn remove_every_transition(&mut self, track: TrackId) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let commands: Vec<Command> = self
            .active_sequence()
            .and_then(|s| s.video_track(track))
            .ok_or(EditorError::TrackNotFound(track))?
            .clips()
            .iter()
            .filter(|clip| clip.transition_out.is_some())
            .map(|clip| Command::SetTransition {
                sequence,
                track,
                clip: clip.id,
                transition: None,
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Remove Transitions".to_owned(), commands)?;
        }
        Ok(count)
    }
}
