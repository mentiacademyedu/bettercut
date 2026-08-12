//! Tracks: ordered, non-overlapping containers of clips (§8).
//!
//! ## Why the sort order is an invariant, not a convenience
//!
//! §53 requires drawing ~30 visible clips out of 10,000 without touching the
//! rest, and §54 requires querying by visible range rather than walking the
//! project. Both need clips sorted by start time. Enforcing that on insert makes
//! the query a binary search instead of a filter over everything.

use bettercut_foundation::{ClipId, MediaTime, TimelineTime, TrackId};
use serde::{Deserialize, Serialize};

use crate::clip::{AudioClip, Clip, SourceRange, TimelineRange, VideoClip};
use crate::error::TimelineError;

/// What a [`Track::split`] produced: the clip as it was, and the two halves.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitOutcome<C> {
    /// The original clip, kept so undo restores it exactly rather than
    /// stitching the halves back together and hoping they match.
    pub original: C,
    pub left: ClipId,
    pub right: ClipId,
}

/// Storage shared by video and audio tracks.
///
/// Generic over the clip type so the sorting and overlap invariants exist in
/// exactly one place — two copies would drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track<C> {
    pub id: TrackId,
    pub name: String,

    /// Always sorted by `timeline.start`, always non-overlapping.
    clips: Vec<C>,

    /// Hidden (video) tracks are skipped by the renderer; muted (audio) tracks
    /// are skipped by the mixer. Both still export as part of the project.
    pub enabled: bool,
    /// A locked track rejects edits (§10 "Lock track").
    pub locked: bool,
}

impl<C: Clip> Track<C> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: TrackId::new(),
            name: name.into(),
            clips: Vec::new(),
            enabled: true,
            locked: false,
        }
    }

    pub fn clips(&self) -> &[C] {
        &self.clips
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// End of the last clip, or zero for an empty track.
    pub fn duration(&self) -> TimelineTime {
        self.clips
            .last()
            .map_or(TimelineTime::ZERO, |c| c.timeline().end)
    }

    /// Insert a clip, preserving sort order and rejecting overlaps.
    ///
    /// Returns the index it landed at.
    pub fn insert(&mut self, clip: C) -> Result<usize, TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }
        let range = clip.timeline();

        // Binary search for the insertion point by start time.
        let index = self
            .clips
            .partition_point(|c| c.timeline().start < range.start);

        // Only the neighbours can overlap, because the list is already sorted
        // and already non-overlapping.
        if let Some(prev) = index.checked_sub(1).and_then(|i| self.clips.get(i))
            && prev.timeline().overlaps(range)
        {
            return Err(TimelineError::ClipOverlap {
                track: self.id,
                new: clip.id(),
                existing: prev.id(),
                start: range.start,
                end: range.end,
            });
        }
        if let Some(next) = self.clips.get(index)
            && next.timeline().overlaps(range)
        {
            return Err(TimelineError::ClipOverlap {
                track: self.id,
                new: clip.id(),
                existing: next.id(),
                start: range.start,
                end: range.end,
            });
        }

        self.clips.insert(index, clip);
        Ok(index)
    }

    /// Remove a clip and hand it back, so an undo can put it where it was.
    pub fn remove(&mut self, id: ClipId) -> Result<C, TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }
        let index = self
            .clips
            .iter()
            .position(|c| c.id() == id)
            .ok_or(TimelineError::ClipNotFound(id))?;
        Ok(self.clips.remove(index))
    }

    pub fn get(&self, id: ClipId) -> Option<&C> {
        self.clips.iter().find(|c| c.id() == id)
    }

    /// Mutable access to one clip, for editing its **properties**.
    ///
    /// Opacity, gain and transform are safe to change here. The clip's
    /// `timeline` range is **not**: the track keeps its clips sorted by start
    /// and relies on that for the range queries §53 draws with, and moving a
    /// clip through this would break the ordering without anything noticing.
    /// Use `move_clip` / `trim` for anything that changes when a clip plays.
    pub fn get_mut(&mut self, id: ClipId) -> Option<&mut C> {
        self.clips.iter_mut().find(|c| c.id() == id)
    }

    /// The clip containing `t`, if any.
    pub fn clip_at(&self, t: TimelineTime) -> Option<&C> {
        let index = self.clips.partition_point(|c| c.timeline().end <= t);
        self.clips.get(index).filter(|c| c.timeline().contains(t))
    }

    /// Clips intersecting `range`, as a contiguous borrowed slice.
    ///
    /// This is the §53/§54 viewport query: O(log n) to locate, then O(visible).
    /// It allocates nothing and never triggers a decode (§53).
    pub fn clips_in_range(&self, range: TimelineRange) -> &[C] {
        // First clip whose end is past the range start.
        let first = self
            .clips
            .partition_point(|c| c.timeline().end <= range.start);
        // First clip that starts at or after the range end.
        let last = self
            .clips
            .partition_point(|c| c.timeline().start < range.end);
        // `last >= first` always holds: both are monotonic in the same order.
        &self.clips[first..last.max(first)]
    }

    fn index_of(&self, id: ClipId) -> Result<usize, TimelineError> {
        self.clips
            .iter()
            .position(|c| c.id() == id)
            .ok_or(TimelineError::ClipNotFound(id))
    }

    /// Would `range` fit if `ignore` were not there?
    ///
    /// `ignore` is the clip being moved or trimmed: it must not be treated as
    /// an obstacle to itself.
    fn space_is_free(&self, range: TimelineRange, ignore: ClipId) -> Option<ClipId> {
        self.clips
            .iter()
            .find(|c| c.id() != ignore && c.timeline().overlaps(range))
            .map(Clip::id)
    }

    /// Move a clip to `new_start`, keeping its duration and source range.
    ///
    /// Returns the previous start so an undo can be exact rather than
    /// recomputed — recomputing is how a move that was clamped or snapped ends
    /// up un-undoable.
    pub fn move_clip(
        &mut self,
        id: ClipId,
        new_start: TimelineTime,
    ) -> Result<TimelineTime, TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }
        if new_start.is_negative() {
            return Err(TimelineError::NegativePosition { at: new_start });
        }

        let index = self.index_of(id)?;
        let old = self.clips[index].timeline();
        if old.start == new_start {
            return Ok(old.start);
        }

        let moved = TimelineRange {
            start: new_start,
            end: new_start + old.duration(),
        };
        if let Some(existing) = self.space_is_free(moved, id) {
            return Err(TimelineError::ClipOverlap {
                track: self.id,
                new: id,
                existing,
                start: moved.start,
                end: moved.end,
            });
        }

        let mut clip = self.clips.remove(index);
        clip.set_timeline(moved);
        self.reinsert(clip);
        Ok(old.start)
    }

    /// Trim the left edge to `new_start`.
    ///
    /// The source in-point moves by the same amount, so the visible content
    /// stays put and only its extent changes. Playback speed is unaffected —
    /// the MVP has no speed change (§59).
    ///
    /// Returns the previous ranges, for an exact undo.
    pub fn trim_start(
        &mut self,
        id: ClipId,
        new_start: TimelineTime,
    ) -> Result<(TimelineRange, SourceRange), TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }
        if new_start.is_negative() {
            return Err(TimelineError::NegativePosition { at: new_start });
        }

        let index = self.index_of(id)?;
        let (old_timeline, old_source) = {
            let clip = &self.clips[index];
            (clip.timeline(), clip.source())
        };

        if new_start >= old_timeline.end {
            return Err(TimelineError::EmptyRange {
                start: new_start,
                end: old_timeline.end,
            });
        }

        // Positive when trimming inward, negative when extending outward.
        let delta = new_start.ticks() - old_timeline.start.ticks();
        let new_source_start = MediaTime::from_ticks(old_source.start.ticks() + delta);

        // Cannot extend past the beginning of the source media.
        if new_source_start.is_negative() {
            return Err(TimelineError::BeyondSourceStart {
                clip: id,
                by_ticks: -new_source_start.ticks(),
            });
        }

        let timeline = TimelineRange {
            start: new_start,
            end: old_timeline.end,
        };
        if let Some(existing) = self.space_is_free(timeline, id) {
            return Err(TimelineError::ClipOverlap {
                track: self.id,
                new: id,
                existing,
                start: timeline.start,
                end: timeline.end,
            });
        }

        let mut clip = self.clips.remove(index);
        clip.set_timeline(timeline);
        clip.set_source(SourceRange {
            start: new_source_start,
            end: old_source.end,
        });
        self.reinsert(clip);

        Ok((old_timeline, old_source))
    }

    /// Trim the right edge to `new_end`.
    ///
    /// `max_source_end` bounds the source out-point — the media's duration.
    /// Pass `None` when the caller does not know it; the structural invariants
    /// are still enforced.
    pub fn trim_end(
        &mut self,
        id: ClipId,
        new_end: TimelineTime,
        max_source_end: Option<MediaTime>,
    ) -> Result<(TimelineRange, SourceRange), TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }

        let index = self.index_of(id)?;
        let (old_timeline, old_source) = {
            let clip = &self.clips[index];
            (clip.timeline(), clip.source())
        };

        if new_end <= old_timeline.start {
            return Err(TimelineError::EmptyRange {
                start: old_timeline.start,
                end: new_end,
            });
        }

        let delta = new_end.ticks() - old_timeline.end.ticks();
        let new_source_end = MediaTime::from_ticks(old_source.end.ticks() + delta);

        if let Some(limit) = max_source_end
            && new_source_end > limit
        {
            return Err(TimelineError::BeyondSourceEnd {
                clip: id,
                by_ticks: new_source_end.ticks() - limit.ticks(),
            });
        }

        let timeline = TimelineRange {
            start: old_timeline.start,
            end: new_end,
        };
        if let Some(existing) = self.space_is_free(timeline, id) {
            return Err(TimelineError::ClipOverlap {
                track: self.id,
                new: id,
                existing,
                start: timeline.start,
                end: timeline.end,
            });
        }

        let mut clip = self.clips.remove(index);
        clip.set_timeline(timeline);
        clip.set_source(SourceRange {
            start: old_source.start,
            end: new_source_end,
        });
        self.reinsert(clip);

        Ok((old_timeline, old_source))
    }

    /// Split the clip containing `at` into two abutting clips (§76).
    ///
    /// `at` must be strictly inside the clip: splitting exactly on an edge
    /// would produce a zero-length clip, which every later stage would have to
    /// defend against.
    ///
    /// **The caller must snap `at` to a frame boundary first** (§9, §76). This
    /// method does not know the sequence's frame rate.
    ///
    /// The two halves' IDs are supplied rather than minted here, so that
    /// executing the same split twice — a redo, or a §38.2 journal replay —
    /// produces exactly the same identities. Minting them internally makes a
    /// redone split silently different from the original, which breaks
    /// selection and any later command that named those clips.
    pub fn split(
        &mut self,
        id: ClipId,
        at: TimelineTime,
        left_id: ClipId,
        right_id: ClipId,
    ) -> Result<SplitOutcome<C>, TimelineError>
    where
        C: Clone,
    {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }

        let index = self.index_of(id)?;
        let original = self.clips[index].clone();
        let timeline = original.timeline();
        let source = original.source();

        if at <= timeline.start || at >= timeline.end {
            return Err(TimelineError::SplitOutsideClip {
                clip: id,
                at,
                start: timeline.start,
                end: timeline.end,
            });
        }

        // No speed change, so a tick on the timeline is a tick in the source.
        let offset = at.ticks() - timeline.start.ticks();
        let source_split = MediaTime::from_ticks(source.start.ticks() + offset);

        let mut left = original.clone();
        left.set_id(left_id);
        left.set_timeline(TimelineRange {
            start: timeline.start,
            end: at,
        });
        left.set_source(SourceRange {
            start: source.start,
            end: source_split,
        });

        let mut right = original.clone();
        right.set_id(right_id);
        right.set_timeline(TimelineRange {
            start: at,
            end: timeline.end,
        });
        right.set_source(SourceRange {
            start: source_split,
            end: source.end,
        });

        let (left_id, right_id) = (left.id(), right.id());
        self.clips[index] = left;
        self.clips.insert(index + 1, right);

        Ok(SplitOutcome {
            original,
            left: left_id,
            right: right_id,
        })
    }

    /// Remove a clip and close the gap it leaves, pulling every later clip on
    /// **this track** left by its duration (§10 "Ripple delete").
    ///
    /// Deliberately per-track. A sequence-wide ripple would move music and
    /// overlays that the user did not touch; per-track matches what a
    /// CapCut-style editor does (§87) and is the behaviour that surprises
    /// people least.
    ///
    /// Returns the removed clip and how far the rest moved, for undo.
    pub fn ripple_remove(&mut self, id: ClipId) -> Result<(C, TimelineTime), TimelineError> {
        if self.locked {
            return Err(TimelineError::TrackLocked(self.id));
        }

        let index = self.index_of(id)?;
        let removed = self.clips.remove(index);
        let shift = removed.timeline().duration();

        // Everything at or after the removed clip is now at index..; because
        // the list stays sorted, shifting them all left preserves order.
        for clip in &mut self.clips[index..] {
            let range = clip.timeline();
            clip.set_timeline(TimelineRange {
                start: range.start - shift,
                end: range.end - shift,
            });
        }

        Ok((removed, shift))
    }

    /// Undo a ripple: push clips from `index` onward right again, then
    /// reinsert. Kept next to `ripple_remove` so the two cannot drift.
    pub fn ripple_restore(&mut self, clip: C, shift: TimelineTime) -> Result<(), TimelineError> {
        let start = clip.timeline().start;
        for existing in self
            .clips
            .iter_mut()
            .filter(|c| c.timeline().start >= start)
        {
            let range = existing.timeline();
            existing.set_timeline(TimelineRange {
                start: range.start + shift,
                end: range.end + shift,
            });
        }
        self.insert_unchecked(clip);
        Ok(())
    }

    /// Insert a clip already known to fit, preserving sort order.
    fn reinsert(&mut self, clip: C) {
        self.insert_unchecked(clip);
    }

    fn insert_unchecked(&mut self, clip: C) {
        let start = clip.timeline().start;
        let index = self.clips.partition_point(|c| c.timeline().start < start);
        self.clips.insert(index, clip);
    }

    /// Debug-only invariant check, used by tests and `debug_assert`s.
    pub fn validate(&self) -> Result<(), TimelineError> {
        for pair in self.clips.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if b.timeline().start < a.timeline().start || a.timeline().overlaps(b.timeline()) {
                return Err(TimelineError::ClipOverlap {
                    track: self.id,
                    new: b.id(),
                    existing: a.id(),
                    start: b.timeline().start,
                    end: b.timeline().end,
                });
            }
        }
        Ok(())
    }
}

pub type VideoTrack = Track<VideoClip>;
pub type AudioTrack = Track<AudioClip>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::SourceRange;
    use bettercut_foundation::{MediaId, MediaTime};

    fn clip(start: i64, len: i64) -> VideoClip {
        let source =
            SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(len)).expect("len > 0");
        VideoClip::new(MediaId::new(), TimelineTime::from_ticks(start), source).expect("valid")
    }

    #[test]
    fn insert_keeps_clips_sorted_regardless_of_insert_order() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(500, 100)).expect("no overlap");
        track.insert(clip(0, 100)).expect("no overlap");
        track.insert(clip(200, 100)).expect("no overlap");

        let starts: Vec<i64> = track
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        assert_eq!(starts, vec![0, 200, 500]);
        track.validate().expect("invariants hold");
    }

    #[test]
    fn insert_rejects_an_overlap_on_either_side() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(100, 100)).expect("no overlap");

        // Overlaps from the left.
        assert!(matches!(
            track.insert(clip(50, 100)),
            Err(TimelineError::ClipOverlap { .. })
        ));
        // Overlaps from the right.
        assert!(matches!(
            track.insert(clip(150, 100)),
            Err(TimelineError::ClipOverlap { .. })
        ));
        // Fully contained.
        assert!(matches!(
            track.insert(clip(120, 10)),
            Err(TimelineError::ClipOverlap { .. })
        ));
        assert_eq!(track.len(), 1);
    }

    #[test]
    fn butt_joined_clips_are_accepted() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");
        track
            .insert(clip(100, 100))
            .expect("touching is not overlapping");
        assert_eq!(track.len(), 2);
        track.validate().expect("invariants hold");
    }

    #[test]
    fn range_query_returns_only_intersecting_clips() {
        let mut track = VideoTrack::new("V1");
        for i in 0..10 {
            track.insert(clip(i * 1000, 500)).expect("no overlap");
        }

        let range = TimelineRange::new(
            TimelineTime::from_ticks(2_500),
            TimelineTime::from_ticks(5_200),
        )
        .expect("non-empty");

        let visible = track.clips_in_range(range);
        let starts: Vec<i64> = visible.iter().map(|c| c.timeline.start.ticks()).collect();
        // 2000..2500 ends exactly at the range start, so it is excluded.
        assert_eq!(starts, vec![3000, 4000, 5000]);
    }

    #[test]
    fn range_query_is_empty_in_a_gap() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");
        track.insert(clip(1000, 100)).expect("no overlap");

        let gap = TimelineRange::new(TimelineTime::from_ticks(200), TimelineTime::from_ticks(900))
            .expect("non-empty");
        assert!(track.clips_in_range(gap).is_empty());
    }

    #[test]
    fn clip_at_finds_the_containing_clip() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");
        track.insert(clip(500, 100)).expect("no overlap");

        assert!(track.clip_at(TimelineTime::from_ticks(50)).is_some());
        assert!(track.clip_at(TimelineTime::from_ticks(100)).is_none());
        assert!(track.clip_at(TimelineTime::from_ticks(550)).is_some());
    }

    #[test]
    fn removing_returns_the_clip_so_undo_can_restore_it() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c.clone()).expect("no overlap");

        let removed = track.remove(id).expect("present");
        assert_eq!(removed, c);
        assert!(track.is_empty());
        assert!(matches!(
            track.remove(id),
            Err(TimelineError::ClipNotFound(_))
        ));
    }

    #[test]
    fn a_locked_track_rejects_edits() {
        let mut track = VideoTrack::new("V1");
        track.locked = true;
        assert!(matches!(
            track.insert(clip(0, 100)),
            Err(TimelineError::TrackLocked(_))
        ));
    }

    // ---- editing primitives (§85) ----

    #[test]
    fn moving_a_clip_keeps_its_duration_and_source() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let (id, source) = (c.id, c.source);
        track.insert(c).expect("no overlap");

        let old_start = track
            .move_clip(id, TimelineTime::from_ticks(500))
            .expect("ok");
        assert_eq!(old_start.ticks(), 0);

        let moved = track.get(id).expect("present");
        assert_eq!(moved.timeline.start.ticks(), 500);
        assert_eq!(moved.timeline.duration().ticks(), 100);
        assert_eq!(moved.source, source, "moving must not retime the source");
        track.validate().expect("invariants hold");
    }

    #[test]
    fn moving_keeps_the_track_sorted() {
        let mut track = VideoTrack::new("V1");
        let first = clip(0, 100);
        let id = first.id;
        track.insert(first).expect("no overlap");
        track.insert(clip(200, 100)).expect("no overlap");
        track.insert(clip(400, 100)).expect("no overlap");

        // Move the first clip past the other two.
        track
            .move_clip(id, TimelineTime::from_ticks(600))
            .expect("ok");

        let starts: Vec<i64> = track
            .clips()
            .iter()
            .map(|c| c.timeline.start.ticks())
            .collect();
        assert_eq!(starts, vec![200, 400, 600]);
        track.validate().expect("invariants hold");
    }

    #[test]
    fn moving_onto_another_clip_is_rejected_and_changes_nothing() {
        let mut track = VideoTrack::new("V1");
        let a = clip(0, 100);
        let id = a.id;
        track.insert(a).expect("no overlap");
        track.insert(clip(200, 100)).expect("no overlap");

        assert!(matches!(
            track.move_clip(id, TimelineTime::from_ticks(150)),
            Err(TimelineError::ClipOverlap { .. })
        ));
        assert_eq!(
            track.get(id).expect("still there").timeline.start.ticks(),
            0
        );
        track.validate().expect("invariants hold");
    }

    #[test]
    fn a_clip_cannot_be_moved_before_zero() {
        let mut track = VideoTrack::new("V1");
        let c = clip(100, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");

        assert!(matches!(
            track.move_clip(id, TimelineTime::from_ticks(-1)),
            Err(TimelineError::NegativePosition { .. })
        ));
    }

    /// Trimming the left edge moves the source in-point by the same amount, so
    /// the visible content stays put instead of sliding.
    #[test]
    fn trimming_the_start_advances_the_source_in_point() {
        let mut track = VideoTrack::new("V1");
        let source = SourceRange::new(MediaTime::from_ticks(1000), MediaTime::from_ticks(1100))
            .expect("valid");
        let c = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).expect("valid");
        let id = c.id;
        track.insert(c).expect("no overlap");

        track
            .trim_start(id, TimelineTime::from_ticks(30))
            .expect("ok");

        let trimmed = track.get(id).expect("present");
        assert_eq!(trimmed.timeline.start.ticks(), 30);
        assert_eq!(trimmed.timeline.end.ticks(), 100);
        assert_eq!(trimmed.source.start.ticks(), 1030);
        assert_eq!(trimmed.source.end.ticks(), 1100);
        assert_eq!(
            trimmed.timeline.duration().ticks(),
            trimmed.source.duration().ticks(),
            "trim changed playback speed"
        );
    }

    #[test]
    fn trimming_the_end_retracts_the_source_out_point() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");

        track
            .trim_end(id, TimelineTime::from_ticks(70), None)
            .expect("ok");

        let trimmed = track.get(id).expect("present");
        assert_eq!(trimmed.timeline.end.ticks(), 70);
        assert_eq!(trimmed.source.end.ticks(), 70);
        assert_eq!(
            trimmed.timeline.duration().ticks(),
            trimmed.source.duration().ticks()
        );
    }

    /// A clip cannot show media from before the file starts.
    #[test]
    fn trimming_the_start_outward_stops_at_the_media_start() {
        let mut track = VideoTrack::new("V1");
        let source = SourceRange::new(MediaTime::ZERO, MediaTime::from_ticks(100)).expect("valid");
        let c =
            VideoClip::new(MediaId::new(), TimelineTime::from_ticks(500), source).expect("valid");
        let id = c.id;
        track.insert(c).expect("no overlap");

        // Extending left by 10 would need source content at -10.
        assert!(matches!(
            track.trim_start(id, TimelineTime::from_ticks(490)),
            Err(TimelineError::BeyondSourceStart { .. })
        ));
    }

    #[test]
    fn trimming_the_end_outward_stops_at_the_media_end() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");

        // The media is only 150 ticks long, so 200 is past its end.
        assert!(matches!(
            track.trim_end(
                id,
                TimelineTime::from_ticks(200),
                Some(MediaTime::from_ticks(150))
            ),
            Err(TimelineError::BeyondSourceEnd { .. })
        ));
        // Within the media it is fine.
        assert!(
            track
                .trim_end(
                    id,
                    TimelineTime::from_ticks(140),
                    Some(MediaTime::from_ticks(150))
                )
                .is_ok()
        );
    }

    #[test]
    fn trimming_cannot_invert_a_clip() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");

        assert!(matches!(
            track.trim_start(id, TimelineTime::from_ticks(100)),
            Err(TimelineError::EmptyRange { .. })
        ));
        assert!(matches!(
            track.trim_end(id, TimelineTime::ZERO, None),
            Err(TimelineError::EmptyRange { .. })
        ));
    }

    #[test]
    fn trimming_into_a_neighbour_is_rejected() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");

        // Source starts well inside the media, so there is room to extend left
        // and the *overlap* is what stops it — not the media-start limit.
        let source = SourceRange::new(MediaTime::from_ticks(1000), MediaTime::from_ticks(1100))
            .expect("valid");
        let c =
            VideoClip::new(MediaId::new(), TimelineTime::from_ticks(200), source).expect("valid");
        let id = c.id;
        track.insert(c).expect("no overlap");

        assert!(matches!(
            track.trim_start(id, TimelineTime::from_ticks(50)),
            Err(TimelineError::ClipOverlap { .. })
        ));
        // Unchanged.
        assert_eq!(track.get(id).expect("present").timeline.start.ticks(), 200);
    }

    /// §76: the two halves must abut exactly, with no gap and no overlap, and
    /// the source must be divided at the matching point.
    #[test]
    fn splitting_produces_two_abutting_clips() {
        let mut track = VideoTrack::new("V1");
        let source = SourceRange::new(MediaTime::from_ticks(500), MediaTime::from_ticks(600))
            .expect("valid");
        let c = VideoClip::new(MediaId::new(), TimelineTime::ZERO, source).expect("valid");
        let id = c.id;
        track.insert(c).expect("no overlap");

        let (left_id, right_id) = (ClipId::new(), ClipId::new());
        let outcome = track
            .split(id, TimelineTime::from_ticks(40), left_id, right_id)
            .expect("ok");
        assert_eq!(track.len(), 2);
        assert_eq!(outcome.left, left_id, "split did not use the supplied ids");
        assert_eq!(outcome.right, right_id);

        let left = track.get(outcome.left).expect("left");
        let right = track.get(outcome.right).expect("right");

        assert_eq!(left.timeline.start.ticks(), 0);
        assert_eq!(left.timeline.end.ticks(), 40);
        assert_eq!(right.timeline.start.ticks(), 40);
        assert_eq!(right.timeline.end.ticks(), 100);
        assert_eq!(left.timeline.end, right.timeline.start, "gap at the cut");

        // §76: "Media is not duplicated" — the source range is divided.
        assert_eq!(left.source.start.ticks(), 500);
        assert_eq!(left.source.end.ticks(), 540);
        assert_eq!(right.source.start.ticks(), 540);
        assert_eq!(right.source.end.ticks(), 600);

        assert_ne!(outcome.left, outcome.right, "halves share an id");
        assert_eq!(outcome.original.id, id);
        track.validate().expect("invariants hold");
    }

    #[test]
    fn splitting_on_an_edge_is_refused() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");

        for at in [0, 100, 200, -50] {
            assert!(
                matches!(
                    track.split(
                        id,
                        TimelineTime::from_ticks(at),
                        ClipId::new(),
                        ClipId::new()
                    ),
                    Err(TimelineError::SplitOutsideClip { .. })
                ),
                "split at {at} should be refused"
            );
        }
        assert_eq!(track.len(), 1);
    }

    /// §10 "Ripple delete": the gap closes behind the removed clip.
    #[test]
    fn ripple_delete_closes_the_gap() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");
        let middle = clip(100, 50);
        let id = middle.id;
        track.insert(middle).expect("no overlap");
        track.insert(clip(150, 100)).expect("no overlap");

        let (removed, shift) = track.ripple_remove(id).expect("ok");
        assert_eq!(shift.ticks(), 50);
        assert_eq!(removed.id, id);

        let spans: Vec<(i64, i64)> = track
            .clips()
            .iter()
            .map(|c| (c.timeline.start.ticks(), c.timeline.end.ticks()))
            .collect();
        assert_eq!(spans, vec![(0, 100), (100, 200)]);
        track.validate().expect("invariants hold");
    }

    #[test]
    fn ripple_delete_round_trips_through_restore() {
        let mut track = VideoTrack::new("V1");
        track.insert(clip(0, 100)).expect("no overlap");
        let middle = clip(100, 50);
        let id = middle.id;
        track.insert(middle).expect("no overlap");
        track.insert(clip(150, 100)).expect("no overlap");

        let before: Vec<(i64, i64)> = track
            .clips()
            .iter()
            .map(|c| (c.timeline.start.ticks(), c.timeline.end.ticks()))
            .collect();

        let (removed, shift) = track.ripple_remove(id).expect("ok");
        track.ripple_restore(removed, shift).expect("ok");

        let after: Vec<(i64, i64)> = track
            .clips()
            .iter()
            .map(|c| (c.timeline.start.ticks(), c.timeline.end.ticks()))
            .collect();
        assert_eq!(after, before, "ripple undo did not restore the timeline");
        track.validate().expect("invariants hold");
    }

    #[test]
    fn every_edit_is_refused_on_a_locked_track() {
        let mut track = VideoTrack::new("V1");
        let c = clip(0, 100);
        let id = c.id;
        track.insert(c).expect("no overlap");
        track.locked = true;

        assert!(track.move_clip(id, TimelineTime::from_ticks(5)).is_err());
        assert!(track.trim_start(id, TimelineTime::from_ticks(5)).is_err());
        assert!(
            track
                .trim_end(id, TimelineTime::from_ticks(50), None)
                .is_err()
        );
        assert!(
            track
                .split(
                    id,
                    TimelineTime::from_ticks(50),
                    ClipId::new(),
                    ClipId::new()
                )
                .is_err()
        );
        assert!(track.ripple_remove(id).is_err());
        assert_eq!(track.len(), 1);
    }

    #[test]
    fn duration_is_the_end_of_the_last_clip() {
        let mut track = VideoTrack::new("V1");
        assert_eq!(track.duration(), TimelineTime::ZERO);
        track.insert(clip(0, 100)).expect("no overlap");
        track.insert(clip(900, 100)).expect("no overlap");
        assert_eq!(track.duration().ticks(), 1000);
    }
}
