//! Replacing a clip's media: new footage in the old clip's place, with
//! everything done to the clip kept.
//!
//! A placeholder shot, a better take, the client's corrected logo: the clip is
//! already cut to length, graded, framed, animated and faded, and doing that
//! again on a freshly placed file is the work this saves. So the clip stays —
//! its place and length on the timeline, its look, its keys, its speed — and
//! only what it reads from changes.
//!
//! # Which part of the new file
//!
//! The same stretch as before when the new file is long enough to have it (a
//! better take of the same length lines up exactly); otherwise the stretch
//! nearest to it that fits. A file shorter than the footage the clip uses is
//! refused — stretching or looping it would be a different edit, and silently
//! shortening the clip would move everything after it.
//!
//! A photo has no stretch: it shows for the clip's whole length, at normal
//! speed and forwards, since it has no motion to re-time.
//!
//! # The sound
//!
//! A picture's linked sound (§12) came from the old file, and sound from
//! footage that is no longer on screen is never what anyone wants. When the new
//! file has sound, the linked sound switches with the picture. When it has
//! none — a photo, a silent clip — the old sound is taken away. A picture that
//! had no sound gains the new file's, when an audio track has room for it.
//!
//! Replacing a sound clip on its own replaces just the sound, and unties it
//! from its picture: it no longer belongs to it.

use bettercut_foundation::{ClipId, LinkId, MediaId, MediaTime, Rational, TrackId};
use bettercut_media::MediaAsset;
use bettercut_timeline::{SourceRange, TimelineRange};

use crate::command::{ClipPayload, Command, MediaSwap};
use crate::editor::Editor;
use crate::error::EditorError;

/// What happened to the sound, for the status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundChange {
    /// There was none, and the new file brought none (or there was no room).
    None,
    /// The linked sound now plays the new file's.
    Replaced,
    /// The new file has no sound, so the old file's was taken away.
    Removed,
    /// The picture had no sound, and now has the new file's.
    Added,
    /// A sound clip was replaced on its own and untied from its picture.
    Unlinked,
}

/// The stretch of `asset` a clip reads after a replacement, with the speed and
/// direction it plays it at.
///
/// `source`, `speed` and `reversed` are the clip's now; `timeline` is its span,
/// which does not change. A photo clip's source range grows and shrinks with
/// the clip, so it already says how much footage the clip needs.
pub fn replacement_source(
    source: SourceRange,
    speed: Rational,
    reversed: bool,
    timeline: TimelineRange,
    asset: &MediaAsset,
) -> Result<(SourceRange, Rational, bool), EditorError> {
    let length = MediaTime::from_ticks(timeline.duration().ticks());
    if asset.is_still() {
        return Ok((
            SourceRange::new(MediaTime::ZERO, length)?,
            Rational::ONE,
            false,
        ));
    }
    let (start, needed) = (source.start, source.duration());
    if needed > asset.duration {
        return Err(EditorError::ReplacementTooShort {
            needed: needed.as_seconds_f64(),
            available: asset.duration.as_seconds_f64(),
        });
    }
    // The same in-point when it fits; otherwise as close to it as the file
    // allows, which is the stretch ending at the file's end.
    let start = start.min(asset.duration - needed);
    Ok((SourceRange::new(start, start + needed)?, speed, reversed))
}

impl Editor {
    /// Media that `clip` could be replaced with: files with a picture for a
    /// picture clip, files with sound for a sound clip. Not the file it
    /// already shows.
    pub fn replacement_candidates(&self, clip: ClipId) -> Vec<&MediaAsset> {
        let (current, wants_picture) = if let Some(video) = self.video_clip(clip) {
            (video.media_id, true)
        } else if let Some(audio) = self.audio_clip(clip) {
            (audio.media_id, false)
        } else {
            return Vec::new();
        };
        self.project()
            .media
            .iter()
            .filter(|asset| asset.id != current)
            .filter(|asset| {
                if wants_picture {
                    asset.kind.has_video()
                } else {
                    has_sound(asset)
                }
            })
            .collect()
    }

    /// Put `media` in place of what `clip` shows or plays, keeping every edit
    /// made to the clip. One undo step, sound included. See the module docs.
    pub fn replace_media(
        &mut self,
        clip: ClipId,
        media: MediaId,
    ) -> Result<SoundChange, EditorError> {
        let sequence = self.active_sequence_id()?;
        let asset = self
            .project()
            .media_asset(media)
            .ok_or(EditorError::MediaNotFound(media))?
            .clone();
        let track = self.track_of(clip).ok_or(EditorError::ClipNotFound(clip))?;

        if let Some(video) = self.video_clip(clip).cloned() {
            if !asset.kind.has_video() {
                return Err(EditorError::ReplacementLacks("picture"));
            }
            let (source, speed, reversed) = replacement_source(
                video.source,
                video.speed,
                video.reversed,
                video.timeline,
                &asset,
            )?;
            let sounds: Vec<(TrackId, bettercut_timeline::AudioClip)> = self
                .linked_with(clip)
                .into_iter()
                .filter_map(|id| Some((self.track_of(id)?, self.audio_clip(id)?.clone())))
                .collect();

            let mut commands = Vec::new();
            let mut link = None;
            let change = if !has_sound(&asset) {
                for (track, sound) in &sounds {
                    commands.push(Command::RemoveClip {
                        sequence,
                        track: *track,
                        clip: sound.id,
                    });
                }
                if sounds.is_empty() {
                    SoundChange::None
                } else {
                    SoundChange::Removed
                }
            } else if !sounds.is_empty() {
                link = video.link;
                for (track, sound) in &sounds {
                    let (source, speed, reversed) = replacement_source(
                        sound.source,
                        sound.speed,
                        sound.reversed,
                        sound.timeline,
                        &asset,
                    )?;
                    commands.push(Command::ReplaceClipMedia {
                        sequence,
                        track: *track,
                        clip: sound.id,
                        swap: MediaSwap {
                            media,
                            source,
                            speed,
                            reversed,
                            link,
                        },
                    });
                }
                SoundChange::Replaced
            } else if let Some(sound_track) = self.free_audio_track(video.timeline) {
                let fresh = LinkId::new();
                link = Some(fresh);
                let mut sound =
                    bettercut_timeline::AudioClip::new(media, video.timeline.start, source)?;
                sound.speed = speed;
                sound.reversed = reversed;
                sound.timeline = video.timeline;
                sound.link = link;
                commands.push(Command::AddClip {
                    sequence,
                    track: sound_track,
                    clip: ClipPayload::Audio(Box::new(sound)),
                });
                SoundChange::Added
            } else {
                SoundChange::None
            };

            // The picture first, so an undo restores it last.
            commands.insert(
                0,
                Command::ReplaceClipMedia {
                    sequence,
                    track,
                    clip,
                    swap: MediaSwap {
                        media,
                        source,
                        speed,
                        reversed,
                        link,
                    },
                },
            );
            self.dispatch_group("Replace Media".to_owned(), commands)?;
            return Ok(change);
        }

        let sound = self
            .audio_clip(clip)
            .cloned()
            .ok_or(EditorError::ClipKindMismatch)?;
        if !has_sound(&asset) {
            return Err(EditorError::ReplacementLacks("sound"));
        }
        let (source, speed, reversed) = replacement_source(
            sound.source,
            sound.speed,
            sound.reversed,
            sound.timeline,
            &asset,
        )?;
        let mut commands = vec![Command::ReplaceClipMedia {
            sequence,
            track,
            clip,
            swap: MediaSwap {
                media,
                source,
                speed,
                reversed,
                link: None,
            },
        }];
        // Its picture keeps its own file, but is no longer tied to this sound.
        let pictures: Vec<ClipId> = self
            .linked_with(clip)
            .into_iter()
            .filter(|id| *id != clip)
            .collect();
        for picture in &pictures {
            if let (Some(track), Some(swap)) =
                (self.track_of(*picture), self.unlinked_swap(*picture))
            {
                commands.push(Command::ReplaceClipMedia {
                    sequence,
                    track,
                    clip: *picture,
                    swap,
                });
            }
        }
        self.dispatch_group("Replace Media".to_owned(), commands)?;
        Ok(if pictures.is_empty() {
            SoundChange::None
        } else {
            SoundChange::Unlinked
        })
    }

    /// A clip as it is, minus its link: a swap that changes nothing else.
    fn unlinked_swap(&self, clip: ClipId) -> Option<MediaSwap> {
        if let Some(video) = self.video_clip(clip) {
            return Some(MediaSwap {
                media: video.media_id,
                source: video.source,
                speed: video.speed,
                reversed: video.reversed,
                link: None,
            });
        }
        self.audio_clip(clip).map(|audio| MediaSwap {
            media: audio.media_id,
            source: audio.source,
            speed: audio.speed,
            reversed: audio.reversed,
            link: None,
        })
    }

    /// The first audio track with nothing in `span`.
    fn free_audio_track(&self, span: TimelineRange) -> Option<TrackId> {
        self.active_sequence()?
            .audio_tracks
            .iter()
            .find(|track| !track.locked && track.clips().iter().all(|c| !c.timeline.overlaps(span)))
            .map(|track| track.id)
    }
}

/// Whether a file has sound a clip can play. A photo never does.
fn has_sound(asset: &MediaAsset) -> bool {
    asset.audio_codec.is_some() && !asset.is_still()
}
