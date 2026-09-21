//! What changed between this edit and an earlier save
//! (`crate::versions`).
//!
//! Going back to a version is easy to offer and frightening to use: the list
//! says when each one was saved and nothing about what is in it, so choosing
//! one is a guess, and the only way to find out is to open it and look. This
//! answers the question first — *what is different* — so going back is a
//! decision rather than a gamble.
//!
//! It is a comparison of two projects, not of two files. Clips are matched by
//! their ids, which survive saving, so a clip that moved is one clip that
//! moved rather than one removed and another added; only what has no id —
//! markers, media lists, settings — is compared by value. That is what lets
//! the list say "shot 3 moved two seconds later" instead of "the file
//! differs".
//!
//! Nothing here changes anything. It reads two projects and describes the
//! difference in the words the interface uses for the same things elsewhere.

use std::collections::BTreeMap;

use bettercut_foundation::{ClipId, SequenceId, TimelineTime};
use bettercut_project_format::Project;
use bettercut_timeline::{Clip, Sequence, Track};

/// What kind of difference one line describes, for an icon or a colour beside
/// it. The text is what a person reads; this is for sorting and grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    Added,
    Removed,
    Moved,
    Trimmed,
    Retimed,
    Changed,
}

impl ChangeKind {
    /// The word for it, for a heading or a filter.
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Moved => "moved",
            Self::Trimmed => "trimmed",
            Self::Retimed => "re-timed",
            Self::Changed => "changed",
        }
    }
}

/// One difference, ready to put on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// Where it is: a lane's name, a sequence's name, or "Project".
    pub place: String,
    /// What happened, in plain words.
    pub what: String,
}

/// How many differences are listed before the rest are counted rather than
/// named.
///
/// A comparison of two edits a week apart can be thousands of lines, and
/// nobody reads the thousandth. Past this the answer a person actually needs
/// is "these are different edits", which the count says better than the list
/// would.
pub const MOST_CHANGES: usize = 300;

/// Everything that differs between two saves, `before` being the older one.
///
/// Ordered as the project is — settings, then each sequence's lanes top to
/// bottom, then its marks — rather than by kind, because that is the order
/// the person looking at the timeline already has in their head.
pub fn compare(before: &Project, after: &Project) -> Vec<Change> {
    let mut changes = Vec::new();

    compare_project(before, after, &mut changes);

    let old: BTreeMap<SequenceId, &Sequence> = before.sequences.iter().map(|s| (s.id, s)).collect();
    let new: BTreeMap<SequenceId, &Sequence> = after.sequences.iter().map(|s| (s.id, s)).collect();

    for (id, sequence) in &new {
        match old.get(id) {
            Some(was) => compare_sequence(before, after, was, sequence, &mut changes),
            None => changes.push(Change {
                kind: ChangeKind::Added,
                place: "Project".to_owned(),
                what: format!("the sequence \"{}\" was added", sequence.name),
            }),
        }
    }
    for (id, sequence) in &old {
        if !new.contains_key(id) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                place: "Project".to_owned(),
                what: format!("the sequence \"{}\" was removed", sequence.name),
            });
        }
    }

    changes.truncate(MOST_CHANGES);
    changes
}

/// One line for a whole comparison: what a menu can show without opening
/// anything.
pub fn summary(changes: &[Change]) -> String {
    if changes.is_empty() {
        return "Nothing has changed since that save".to_owned();
    }
    let mut counts: BTreeMap<ChangeKind, usize> = BTreeMap::new();
    for change in changes {
        *counts.entry(change.kind).or_default() += 1;
    }
    let parts: Vec<String> = counts
        .into_iter()
        .map(|(kind, count)| format!("{count} {}", kind.label()))
        .collect();
    let capped = if changes.len() >= MOST_CHANGES {
        "at least "
    } else {
        ""
    };
    format!("{capped}{} change(s): {}", changes.len(), parts.join(", "))
}

/// Whether a project's own settings differ, and how. Media and LUTs live here
/// too: they belong to the project rather than to any one sequence.
fn compare_project(before: &Project, after: &Project, changes: &mut Vec<Change>) {
    const PLACE: &str = "Project";

    if before.name != after.name {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: PLACE.to_owned(),
            what: format!("renamed from \"{}\" to \"{}\"", before.name, after.name),
        });
    }

    let was: BTreeMap<_, _> = before.media.iter().map(|m| (m.id, m)).collect();
    let now: BTreeMap<_, _> = after.media.iter().map(|m| (m.id, m)).collect();
    for (id, asset) in &now {
        if !was.contains_key(id) {
            changes.push(Change {
                kind: ChangeKind::Added,
                place: PLACE.to_owned(),
                what: format!("{} was imported", asset.display_name()),
            });
        }
    }
    for (id, asset) in &was {
        match now.get(id) {
            None => changes.push(Change {
                kind: ChangeKind::Removed,
                place: PLACE.to_owned(),
                what: format!("{} was taken out of the project", asset.display_name()),
            }),
            // §66: the same media, read from somewhere else now.
            Some(current) if current.path != asset.path => changes.push(Change {
                kind: ChangeKind::Changed,
                place: PLACE.to_owned(),
                what: format!("{} was relinked", current.display_name()),
            }),
            Some(_) => {}
        }
    }

    let luts_before: Vec<&str> = before.luts.iter().map(|lut| lut.name.as_str()).collect();
    let luts_after: Vec<&str> = after.luts.iter().map(|lut| lut.name.as_str()).collect();
    if luts_before != luts_after {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: PLACE.to_owned(),
            what: format!(
                "the colour tables changed ({} then {})",
                luts_before.len(),
                luts_after.len()
            ),
        });
    }
}

/// One sequence against its older self.
fn compare_sequence(
    before: &Project,
    after: &Project,
    was: &Sequence,
    now: &Sequence,
    changes: &mut Vec<Change>,
) {
    let place = now.name.clone();

    if was.name != now.name {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!("renamed from \"{}\"", was.name),
        });
    }
    if was.resolution != now.resolution {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!(
                "the frame size changed from {}×{} to {}×{}",
                was.resolution.width,
                was.resolution.height,
                now.resolution.width,
                now.resolution.height
            ),
        });
    }
    if was.frame_rate != now.frame_rate {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!(
                "the frame rate changed from {} to {}",
                was.frame_rate.as_f64(),
                now.frame_rate.as_f64()
            ),
        });
    }
    if (was.master_volume - now.master_volume).abs() > 0.001 {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!(
                "the master volume changed from {:.0}% to {:.0}%",
                was.master_volume * 100.0,
                now.master_volume * 100.0
            ),
        });
    }
    if was.master != now.master {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: "the grade over the whole picture changed".to_owned(),
        });
    }
    for (label, before_on, after_on) in [
        (
            "the sound bars",
            was.visualizer.is_some(),
            now.visualizer.is_some(),
        ),
        (
            "the watermark",
            was.watermark.is_some(),
            now.watermark.is_some(),
        ),
    ] {
        if before_on != after_on {
            changes.push(Change {
                kind: if after_on {
                    ChangeKind::Added
                } else {
                    ChangeKind::Removed
                },
                place: place.clone(),
                what: format!(
                    "{label} {}",
                    if after_on {
                        "was added"
                    } else {
                        "was taken off"
                    }
                ),
            });
        }
    }

    // The lanes, top to bottom as the timeline draws them: titles, then
    // pictures, then sound.
    for (was_track, now_track) in paired(&was.text_tracks, &now.text_tracks, &place, changes) {
        compare_lane(was_track, now_track, changes, |clip| {
            let text = clip.text.lines().next().unwrap_or_default();
            if text.is_empty() {
                "a title".to_owned()
            } else {
                format!("\"{}\"", shorten(text))
            }
        });
    }
    for (was_track, now_track) in paired(&was.video_tracks, &now.video_tracks, &place, changes) {
        compare_lane(was_track, now_track, changes, |clip| {
            name_of(before, after, clip.media_id)
        });
    }
    for (was_track, now_track) in paired(&was.audio_tracks, &now.audio_tracks, &place, changes) {
        compare_lane(was_track, now_track, changes, |clip| {
            name_of(before, after, clip.media_id)
        });
    }

    compare_markers(was, now, &place, changes);
}

/// The tracks that exist in both, with a line for each that does not.
///
/// Matched by id rather than by position: adding a lane at the bottom must not
/// read as every lane above it having changed.
fn paired<'a, C>(
    was: &'a [Track<C>],
    now: &'a [Track<C>],
    place: &str,
    changes: &mut Vec<Change>,
) -> Vec<(&'a Track<C>, &'a Track<C>)> {
    let old: BTreeMap<_, _> = was.iter().map(|track| (track.id, track)).collect();
    let mut pairs = Vec::new();
    for track in now {
        match old.get(&track.id) {
            Some(before) => pairs.push((*before, track)),
            None => changes.push(Change {
                kind: ChangeKind::Added,
                place: place.to_owned(),
                what: format!("the lane {} was added", track.name),
            }),
        }
    }
    let kept: std::collections::HashSet<_> = now.iter().map(|track| track.id).collect();
    for track in was {
        if !kept.contains(&track.id) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                place: place.to_owned(),
                what: format!("the lane {} was removed", track.name),
            });
        }
    }
    pairs
}

/// One lane's clips against its older self, `name` describing a clip the way
/// that lane's clips are named.
fn compare_lane<C: Clip + PartialEq>(
    was: &Track<C>,
    now: &Track<C>,
    changes: &mut Vec<Change>,
    name: impl Fn(&C) -> String,
) {
    let place = now.name.clone();

    if was.name != now.name {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!("the lane was renamed from {}", was.name),
        });
    }
    for (what, before_on, after_on) in [
        ("muted", !was.enabled, !now.enabled),
        ("soloed", was.solo, now.solo),
        ("locked", was.locked, now.locked),
    ] {
        if before_on != after_on {
            changes.push(Change {
                kind: ChangeKind::Changed,
                place: place.clone(),
                what: format!("the lane was {}{what}", if after_on { "" } else { "un" }),
            });
        }
    }
    if (was.gain - now.gain).abs() > 0.001 {
        changes.push(Change {
            kind: ChangeKind::Changed,
            place: place.clone(),
            what: format!(
                "the lane's volume changed from {:.0}% to {:.0}%",
                was.gain * 100.0,
                now.gain * 100.0
            ),
        });
    }

    let old: BTreeMap<ClipId, &C> = was.clips().iter().map(|clip| (clip.id(), clip)).collect();
    let new: BTreeMap<ClipId, &C> = now.clips().iter().map(|clip| (clip.id(), clip)).collect();

    for (id, clip) in &new {
        let Some(before) = old.get(id) else {
            changes.push(Change {
                kind: ChangeKind::Added,
                place: place.clone(),
                what: format!("{} put at {}", name(clip), at(clip.timeline().start)),
            });
            continue;
        };
        if let Some(change) = compared(*before, clip, &place, &name) {
            changes.push(change);
        }
    }
    for (id, clip) in &old {
        if !new.contains_key(id) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                place: place.clone(),
                what: format!(
                    "{} taken out from {}",
                    name(clip),
                    at(clip.timeline().start)
                ),
            });
        }
    }
}

/// How one clip differs from its older self, or `None` if it does not.
///
/// One line per clip, choosing the thing that matters most: a clip that moved
/// *and* had its colour changed reads as moved, because that is what someone
/// scanning the list is looking for. Everything else falls through to
/// "changed", which is the honest answer for a difference this cannot name.
fn compared<C: Clip + PartialEq>(
    was: &C,
    now: &C,
    place: &str,
    name: impl Fn(&C) -> String,
) -> Option<Change> {
    if was == now {
        return None;
    }
    let (old_range, new_range) = (was.timeline(), now.timeline());
    let moved = old_range.start != new_range.start;
    let resized = old_range.duration() != new_range.duration();
    let retimed = was.speed() != now.speed() || was.reversed() != now.reversed();

    let change = |kind: ChangeKind, what: String| {
        Some(Change {
            kind,
            place: place.to_owned(),
            what,
        })
    };

    if retimed {
        return change(
            ChangeKind::Retimed,
            format!(
                "{} re-timed from {} to {}",
                name(now),
                speed_of(was),
                speed_of(now)
            ),
        );
    }
    if resized {
        let longer = new_range.duration().ticks() > old_range.duration().ticks();
        return change(
            ChangeKind::Trimmed,
            format!(
                "{} {} by {}",
                name(now),
                if longer { "lengthened" } else { "shortened" },
                length((new_range.duration().ticks() - old_range.duration().ticks()).abs())
            ),
        );
    }
    if moved {
        let later = new_range.start.ticks() > old_range.start.ticks();
        return change(
            ChangeKind::Moved,
            format!(
                "{} moved {} {} to {}",
                name(now),
                length((new_range.start.ticks() - old_range.start.ticks()).abs()),
                if later { "later" } else { "earlier" },
                at(new_range.start)
            ),
        );
    }
    change(
        ChangeKind::Changed,
        format!("{} changed at {}", name(now), at(new_range.start)),
    )
}

/// The marks, which have no ids: compared by where they are.
fn compare_markers(was: &Sequence, now: &Sequence, place: &str, changes: &mut Vec<Change>) {
    let old: BTreeMap<i64, &bettercut_timeline::Marker> =
        was.markers.iter().map(|m| (m.time.ticks(), m)).collect();
    let new: BTreeMap<i64, &bettercut_timeline::Marker> =
        now.markers.iter().map(|m| (m.time.ticks(), m)).collect();

    for (ticks, marker) in &new {
        match old.get(ticks) {
            None => changes.push(Change {
                kind: ChangeKind::Added,
                place: place.to_owned(),
                what: format!("a mark added at {}", at(marker.time)),
            }),
            Some(before) if before.label != marker.label || before.span != marker.span => {
                changes.push(Change {
                    kind: ChangeKind::Changed,
                    place: place.to_owned(),
                    what: format!("the mark at {} changed", at(marker.time)),
                });
            }
            Some(_) => {}
        }
    }
    for (ticks, marker) in &old {
        if !new.contains_key(ticks) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                place: place.to_owned(),
                what: format!("the mark at {} was removed", at(marker.time)),
            });
        }
    }
}

/// A file's name in whichever project still has it — a clip removed from the
/// newer save is named from the older one.
fn name_of(before: &Project, after: &Project, media: bettercut_foundation::MediaId) -> String {
    after
        .media_asset(media)
        .or_else(|| before.media_asset(media))
        .map_or_else(
            || "a clip".to_owned(),
            |asset| asset.display_name().to_owned(),
        )
}

fn at(time: TimelineTime) -> String {
    time.format_timecode()
}

/// A stretch of time in the words a person would use, from ticks.
fn length(ticks: i64) -> String {
    let seconds = ticks as f64 / bettercut_foundation::TICKS_PER_SECOND as f64;
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        format!("{:.0} m {:02.0} s", seconds / 60.0, seconds % 60.0)
    }
}

fn speed_of<C: Clip>(clip: &C) -> String {
    let speed = clip.speed();
    let factor = speed.num() as f64 / speed.den() as f64;
    if clip.reversed() {
        format!("{factor:.2}× backwards")
    } else {
        format!("{factor:.2}×")
    }
}

/// A title's first line, short enough for a list.
fn shorten(text: &str) -> String {
    const MOST: usize = 24;
    if text.chars().count() <= MOST {
        return text.to_owned();
    }
    let kept: String = text.chars().take(MOST).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bettercut_foundation::{MediaTime, Rational};
    use bettercut_media::{MediaAsset, MediaKind};
    use bettercut_timeline::{SourceRange, VideoClip};

    /// A project with one ten-second shot on the first picture lane.
    fn project() -> Project {
        let mut project = Project::new("Cut");
        let media = project.add_media(MediaAsset::new(
            MediaKind::Video,
            "C:/media/shot.mp4",
            MediaTime::from_seconds(60),
        ));
        let clip = VideoClip::new(
            media,
            TimelineTime::ZERO,
            SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(10)).unwrap(),
        )
        .unwrap();
        project.active_mut().unwrap().video_tracks[0]
            .insert(clip)
            .unwrap();
        project
    }

    fn only_clip(project: &mut Project) -> &mut VideoClip {
        let sequence = project.active_mut().unwrap();
        let id = sequence.video_tracks[0].clips()[0].id;
        sequence.video_tracks[0].get_mut(id).unwrap()
    }

    #[test]
    fn a_project_compared_with_itself_has_nothing_to_say() {
        let project = project();
        assert!(compare(&project, &project).is_empty());
        assert_eq!(
            summary(&[]),
            "Nothing has changed since that save".to_owned()
        );
    }

    /// The reason ids matter: a clip that moved is one clip that moved, not
    /// one removed and another added.
    #[test]
    fn a_moved_clip_reads_as_one_move() {
        let before = project();
        let mut after = before.clone();
        {
            let clip = only_clip(&mut after);
            let range = clip.timeline;
            clip.set_timeline(bettercut_timeline::TimelineRange {
                start: range.start + TimelineTime::from_seconds(2),
                end: range.end + TimelineTime::from_seconds(2),
            });
        }

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Moved);
        assert!(changes[0].what.contains("2.0 s later"), "{:?}", changes[0]);
    }

    /// A trim says which way it went and by how much.
    #[test]
    fn a_trimmed_clip_says_how_much_shorter() {
        let before = project();
        let mut after = before.clone();
        {
            let clip = only_clip(&mut after);
            let range = clip.timeline;
            clip.set_timeline(bettercut_timeline::TimelineRange {
                start: range.start,
                end: range.end - TimelineTime::from_seconds(3),
            });
        }

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Trimmed);
        assert!(
            changes[0].what.contains("shortened by 3.0 s"),
            "{:?}",
            changes[0]
        );
    }

    /// Speed beats length: a slowed clip is longer *because* it is slower, and
    /// reading "lengthened" would describe the symptom.
    #[test]
    fn a_retimed_clip_reads_as_retimed_not_trimmed() {
        let before = project();
        let mut after = before.clone();
        {
            let clip = only_clip(&mut after);
            clip.speed = Rational::new(1, 2).unwrap();
            let range = clip.timeline;
            clip.set_timeline(bettercut_timeline::TimelineRange {
                start: range.start,
                end: range.start + TimelineTime::from_seconds(20),
            });
        }

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Retimed);
        assert!(changes[0].what.contains("0.50×"), "{:?}", changes[0]);
    }

    #[test]
    fn adding_and_removing_a_clip_are_both_seen() {
        let before = project();
        let mut after = before.clone();
        {
            let sequence = after.active_mut().unwrap();
            let media = sequence.video_tracks[0].clips()[0].media_id;
            let clip = VideoClip::new(
                media,
                TimelineTime::from_seconds(20),
                SourceRange::new(MediaTime::ZERO, MediaTime::from_seconds(5)).unwrap(),
            )
            .unwrap();
            sequence.video_tracks[0].insert(clip).unwrap();
        }
        let added = compare(&before, &after);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].kind, ChangeKind::Added);

        // And the other way round, which is the same comparison reversed.
        let removed = compare(&after, &before);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].kind, ChangeKind::Removed);
    }

    /// A clip whose look changed but whose place did not still shows up, or a
    /// grading pass would compare as no change at all.
    #[test]
    fn a_clip_changed_in_place_is_still_a_change() {
        let before = project();
        let mut after = before.clone();
        only_clip(&mut after).opacity = 0.5;

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Changed);
    }

    /// Lanes are matched by id, so a lane added at the bottom is one line
    /// rather than every lane reading as different.
    #[test]
    fn a_new_lane_is_one_line() {
        let before = project();
        let mut after = before.clone();
        after
            .active_mut()
            .unwrap()
            .video_tracks
            .push(bettercut_timeline::VideoTrack::new("V2"));

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Added);
        assert!(changes[0].what.contains("V2"), "{:?}", changes[0]);
    }

    /// Marks have no ids, so they are compared by where they are.
    #[test]
    fn marks_are_compared_by_where_they_are() {
        let before = project();
        let mut after = before.clone();
        after
            .active_mut()
            .unwrap()
            .markers
            .push(bettercut_timeline::Marker::at(TimelineTime::from_seconds(
                4,
            )));

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Added);
        assert!(
            changes[0].what.starts_with("a mark added"),
            "{:?}",
            changes[0]
        );
    }

    /// The project's own settings are compared too, not only what is on the
    /// timeline.
    #[test]
    fn settings_and_media_are_compared() {
        let before = project();
        let mut after = before.clone();
        after.active_mut().unwrap().master_volume = 0.5;
        after.add_media(MediaAsset::new(
            MediaKind::Audio,
            "C:/media/music.wav",
            MediaTime::from_seconds(30),
        ));

        let changes = compare(&before, &after);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes.iter().any(|c| c.what.contains("music.wav")));
        assert!(changes.iter().any(|c| c.what.contains("master volume")));
    }

    /// A long list is cut off rather than handed over whole: nobody reads the
    /// three hundredth line, and the count says it better.
    #[test]
    fn a_huge_difference_is_capped() {
        let before = Project::new("Empty");
        let mut after = before.clone();
        {
            let sequence = after.active_mut().unwrap();
            for index in 0..(MOST_CHANGES + 50) {
                sequence
                    .markers
                    .push(bettercut_timeline::Marker::at(TimelineTime::from_seconds(
                        index as i64,
                    )));
            }
        }
        let changes = compare(&before, &after);
        assert_eq!(changes.len(), MOST_CHANGES);
        assert!(
            summary(&changes).starts_with("at least"),
            "{}",
            summary(&changes)
        );
    }
}
