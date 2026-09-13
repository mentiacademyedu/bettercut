//! Chapters from markers: the "0:00 Intro" lines a video description needs.
//!
//! Video sites turn timestamps at the start of description lines into
//! chapters, and the rules are strict enough that a list that breaks one is
//! silently ignored: the first must be at 0:00, there must be at least three,
//! and none may be shorter than ten seconds. So the list is built to follow
//! them — a chapter at 0:00 is added when no marker sits there — and anything
//! it cannot fix is said, rather than left for the upload to discover.
//!
//! A marker's name is the chapter's title; an unnamed one is "Chapter N".
//! Timestamps are whole seconds, rounded down, as the sites read them.

use bettercut_foundation::{TICKS_PER_SECOND, TimelineTime};

use crate::marker::Marker;

/// The fewest chapters a site accepts.
pub const MIN_CHAPTERS: usize = 3;

/// The shortest a chapter may be.
pub const MIN_CHAPTER: TimelineTime = TimelineTime::from_ticks(10 * TICKS_PER_SECOND);

/// Something about the list a site would reject it for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterProblem {
    /// Fewer than [`MIN_CHAPTERS`].
    TooFew(usize),
    /// A chapter shorter than [`MIN_CHAPTER`], by its title.
    TooShort(String),
}

impl ChapterProblem {
    pub fn describe(&self) -> String {
        match self {
            Self::TooFew(count) => format!(
                "{count} chapter{} — video sites need at least {MIN_CHAPTERS}",
                if *count == 1 { "" } else { "s" }
            ),
            Self::TooShort(title) => {
                format!("\"{title}\" is under 10 seconds — video sites skip chapters that short")
            }
        }
    }
}

/// A chapter list, ready to paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapters {
    /// One line a chapter: `0:00 Intro`.
    pub text: String,
    pub count: usize,
    pub problems: Vec<ChapterProblem>,
}

/// The chapter list for `markers` on a sequence `duration` long.
///
/// Markers at or past the end are left out: a chapter that starts when the
/// video has finished is not one.
pub fn chapter_list(markers: &[Marker], duration: TimelineTime) -> Chapters {
    let mut sorted: Vec<&Marker> = markers
        .iter()
        .filter(|m| m.time >= TimelineTime::ZERO && m.time < duration)
        .collect();
    sorted.sort_by_key(|m| m.time);

    // Timestamps are whole seconds, so two markers in the same second would
    // be two chapters at one time: the first of them stands. A title stays
    // unset until the list is final, so numbering counts an added intro.
    let second = |t: TimelineTime| t.ticks() / TICKS_PER_SECOND;
    let mut untitled: Vec<(TimelineTime, Option<String>)> = Vec::new();
    for marker in sorted {
        if untitled
            .last()
            .is_some_and(|(at, _)| second(*at) == second(marker.time))
        {
            continue;
        }
        let name = marker.label.trim();
        untitled.push((marker.time, (!name.is_empty()).then(|| name.to_owned())));
    }
    if untitled.first().is_none_or(|(at, _)| second(*at) != 0) {
        untitled.insert(0, (TimelineTime::ZERO, Some("Intro".to_owned())));
    }
    let starts: Vec<(TimelineTime, String)> = untitled
        .into_iter()
        .enumerate()
        .map(|(index, (at, title))| {
            (
                at,
                title.unwrap_or_else(|| format!("Chapter {}", index + 1)),
            )
        })
        .collect();

    let long_form = duration >= TimelineTime::from_seconds(3_600);
    let mut problems = Vec::new();
    let mut text = String::new();
    for (index, (at, title)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(duration, |(next, _)| *next);
        if end - *at < MIN_CHAPTER {
            problems.push(ChapterProblem::TooShort(title.clone()));
        }
        text.push_str(&timestamp(*at, long_form));
        text.push(' ');
        text.push_str(title);
        text.push('\n');
    }
    if starts.len() < MIN_CHAPTERS {
        problems.insert(0, ChapterProblem::TooFew(starts.len()));
    }
    Chapters {
        text,
        count: starts.len(),
        problems,
    }
}

/// `m:ss`, or `h:mm:ss` when the video runs an hour or more — every line in
/// the same form, as the sites expect.
fn timestamp(at: TimelineTime, long_form: bool) -> String {
    let total = at.ticks().max(0) / TICKS_PER_SECOND;
    let (hours, minutes, seconds) = (total / 3_600, total / 60 % 60, total % 60);
    if long_form {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{}:{seconds:02}", total / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(seconds: i64, label: &str) -> Marker {
        Marker {
            label: label.to_owned(),
            ..Marker::at(TimelineTime::from_seconds(seconds))
        }
    }

    #[test]
    fn named_markers_become_lines_and_an_intro_is_added() {
        let chapters = chapter_list(
            &[marker(95, "The build"), marker(30, "Setup")],
            TimelineTime::from_seconds(300),
        );
        assert_eq!(chapters.text, "0:00 Intro\n0:30 Setup\n1:35 The build\n");
        assert_eq!(chapters.count, 3);
        assert!(chapters.problems.is_empty(), "{:?}", chapters.problems);
    }

    #[test]
    fn a_marker_at_zero_is_the_first_chapter_and_unnamed_ones_are_numbered() {
        let chapters = chapter_list(
            &[marker(0, "Cold open"), marker(40, ""), marker(90, "")],
            TimelineTime::from_seconds(200),
        );
        assert_eq!(
            chapters.text,
            "0:00 Cold open\n0:40 Chapter 2\n1:30 Chapter 3\n"
        );
    }

    #[test]
    fn numbering_counts_the_added_intro() {
        let chapters = chapter_list(
            &[marker(40, ""), marker(90, "")],
            TimelineTime::from_seconds(200),
        );
        assert_eq!(
            chapters.text,
            "0:00 Intro\n0:40 Chapter 2\n1:30 Chapter 3\n"
        );
    }

    #[test]
    fn what_a_site_would_reject_is_said() {
        let chapters = chapter_list(&[marker(5, "Too soon")], TimelineTime::from_seconds(60));
        assert_eq!(
            chapters.problems,
            vec![
                ChapterProblem::TooFew(2),
                ChapterProblem::TooShort("Intro".to_owned()),
            ]
        );
        assert!(chapters.problems[0].describe().contains("at least 3"));
    }

    #[test]
    fn long_videos_use_hours_and_markers_past_the_end_are_dropped() {
        let chapters = chapter_list(
            &[
                marker(600, "Part two"),
                marker(3_700, "Part three"),
                marker(9_999, "After"),
            ],
            TimelineTime::from_seconds(7_200),
        );
        assert_eq!(
            chapters.text,
            "0:00:00 Intro\n0:10:00 Part two\n1:01:40 Part three\n"
        );
    }

    #[test]
    fn two_markers_in_one_second_are_one_chapter() {
        let chapters = chapter_list(
            &[
                Marker {
                    label: "First".to_owned(),
                    ..Marker::at(TimelineTime::from_ticks(30 * TICKS_PER_SECOND + 100))
                },
                Marker {
                    label: "Same second".to_owned(),
                    ..Marker::at(TimelineTime::from_ticks(30 * TICKS_PER_SECOND + 900_000))
                },
                marker(80, "Later"),
            ],
            TimelineTime::from_seconds(200),
        );
        assert_eq!(chapters.text, "0:00 Intro\n0:30 First\n1:20 Later\n");
    }
}
