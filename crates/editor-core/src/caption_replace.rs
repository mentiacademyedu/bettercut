//! Find and replace across the caption lane: the name a transcriber misheard,
//! fixed in every caption at once.

use crate::command::{Command, TextProperty};
use crate::editor::Editor;
use crate::error::EditorError;

/// `text` with every `find` replaced by `with` — ignoring case unless
/// `match_case`. An empty `find` changes nothing.
pub fn replace_text(text: &str, find: &str, with: &str, match_case: bool) -> String {
    if find.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let hit = if match_case {
            rest.find(find).map(|at| (at, find.len()))
        } else {
            find_ignoring_case(rest, find)
        };
        match hit {
            Some((at, len)) => {
                out.push_str(&rest[..at]);
                out.push_str(with);
                rest = &rest[at + len..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Where `needle` first appears in `hay` regardless of case, as a byte
/// offset and the byte length of what matched there.
fn find_ignoring_case(hay: &str, needle: &str) -> Option<(usize, usize)> {
    'start: for (at, _) in hay.char_indices() {
        let mut matched = 0;
        let mut rest = hay[at..].chars();
        for wanted in needle.chars() {
            match rest.next() {
                Some(found) if found.to_lowercase().eq(wanted.to_lowercase()) => {
                    matched += found.len_utf8();
                }
                _ => continue 'start,
            }
        }
        return Some((at, matched));
    }
    None
}

/// `text` in two at the space that leaves the halves closest in length, or
/// `None` for a single word.
pub fn halve(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let length = |part: &[&str]| part.iter().map(|w| w.chars().count()).sum::<usize>();
    let at = (1..words.len())
        .min_by_key(|&i| length(&words[..i]).abs_diff(length(&words[i..])))
        .unwrap_or(1);
    Some((words[..at].join(" "), words[at..].join(" ")))
}

/// Where on the frame the caption lane sits, as the height the captions
/// are placed at (the same measure as a clip's position: 0 is the middle).
pub const CAPTION_PLACES: [(&str, f32); 3] = [("Low", 0.35), ("Middle", 0.0), ("High", -0.35)];

/// Longer than this and a caption reads as two lines crammed into one.
pub const LONG_CAPTION: usize = 42;

/// How a caption's letters are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LetterCase {
    /// EVERY LETTER A CAPITAL: the short-video caption look.
    Upper,
    /// every letter small.
    Lower,
    /// A capital to start each sentence, the rest small.
    Sentence,
}

impl LetterCase {
    pub const ALL: [Self; 3] = [Self::Upper, Self::Lower, Self::Sentence];

    pub fn label(self) -> &'static str {
        match self {
            Self::Upper => "ABC",
            Self::Lower => "abc",
            Self::Sentence => "Abc",
        }
    }

    /// `text` set in this case.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Upper => text.to_uppercase(),
            Self::Lower => text.to_lowercase(),
            Self::Sentence => {
                let mut out = String::with_capacity(text.len());
                let mut start = true;
                for c in text.chars() {
                    // A sentence can open on a number: "3 cats" keeps
                    // the word after it small.
                    if start && c.is_alphanumeric() {
                        out.extend(c.to_uppercase());
                        start = false;
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    if matches!(c, '.' | '!' | '?') {
                        start = true;
                    }
                }
                out
            }
        }
    }
}

impl Editor {
    /// Join caption `clip` to the caption after it on the caption lane: the
    /// words run together, and it lasts until the second one ended. One undo
    /// step; `false` when it is the last caption.
    pub fn merge_caption_with_next(
        &mut self,
        clip: bettercut_foundation::ClipId,
    ) -> Result<bool, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, first, next)) = self.active_sequence().and_then(|s| {
            let lane = s
                .text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)?;
            let first = lane.clips().iter().find(|c| c.id == clip)?;
            let next = lane
                .clips()
                .iter()
                .filter(|c| c.timeline.start >= first.timeline.end && c.id != clip)
                .min_by_key(|c| c.timeline.start)?;
            Some((
                lane.id,
                first.text.clone(),
                (next.id, next.text.clone(), next.timeline.end),
            ))
        }) else {
            return Ok(false);
        };
        let (next_id, next_text, end) = next;
        let joined = match (first.trim(), next_text.trim()) {
            ("", other) | (other, "") => other.to_owned(),
            (a, b) => format!("{a} {b}"),
        };
        self.dispatch_group(
            "Merge Captions",
            vec![
                // Text lanes have their own remove: `RemoveClip` is for
                // picture and sound.
                Command::RemoveText {
                    sequence,
                    track,
                    clip: next_id,
                },
                Command::TrimClip {
                    sequence,
                    track,
                    clip,
                    edge: crate::ops::TrimEdge::End,
                    to: end,
                },
                Command::SetTextProperty {
                    sequence,
                    track,
                    clip,
                    property: TextProperty::Content(joined),
                },
            ],
        )?;
        Ok(true)
    }

    /// Split caption `clip` in two at the word nearest its middle, the time
    /// shared by how much each half says. One undo step; `false` for a
    /// single word or a caption too short to share.
    pub fn split_caption(
        &mut self,
        clip: bettercut_foundation::ClipId,
    ) -> Result<bool, EditorError> {
        let Some(commands) = self.split_caption_commands(clip)? else {
            return Ok(false);
        };
        self.dispatch_group("Split Caption", commands)?;
        Ok(true)
    }

    /// Split every caption longer than `longest` characters, once each, as
    /// one undo step. Returns how many were split.
    pub fn split_long_captions(&mut self, longest: usize) -> Result<usize, EditorError> {
        let long: Vec<_> = self
            .active_sequence()
            .and_then(|s| s.text_tracks.iter().find(|t| t.name == Self::CAPTION_TRACK))
            .map(|lane| {
                lane.clips()
                    .iter()
                    .filter(|c| c.text.chars().count() > longest)
                    .map(|c| c.id)
                    .collect()
            })
            .unwrap_or_default();
        let mut commands = Vec::new();
        let mut split = 0;
        for clip in long {
            if let Some(more) = self.split_caption_commands(clip)? {
                commands.extend(more);
                split += 1;
            }
        }
        if split > 0 {
            self.dispatch_group("Split Long Captions", commands)?;
        }
        Ok(split)
    }

    /// The commands that split caption `clip`, or `None` when it cannot be.
    fn split_caption_commands(
        &self,
        clip: bettercut_foundation::ClipId,
    ) -> Result<Option<Vec<Command>>, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some(active) = self.active_sequence() else {
            return Ok(None);
        };
        let Some(lane) = active
            .text_tracks
            .iter()
            .find(|t| t.name == Self::CAPTION_TRACK)
        else {
            return Ok(None);
        };
        let Some(original) = lane.clips().iter().find(|c| c.id == clip) else {
            return Ok(None);
        };
        let Some((first, second)) = halve(&original.text) else {
            return Ok(None);
        };
        let (a, b) = (first.chars().count() as i64, second.chars().count() as i64);
        let span = original.timeline;
        let length = span.duration().ticks();
        let at = active.snap_to_frame(bettercut_foundation::TimelineTime::from_ticks(
            span.start.ticks() + length * a / (a + b).max(1),
        ));
        if at <= span.start || at >= span.end {
            return Ok(None);
        }
        let mut after = original.clone();
        after.id = bettercut_foundation::ClipId::new();
        after.timeline = bettercut_timeline::TimelineRange {
            start: at,
            end: span.end,
        };
        after.source = bettercut_timeline::SourceRange::new(
            bettercut_foundation::MediaTime::ZERO,
            bettercut_foundation::MediaTime::from_ticks((span.end - at).ticks()),
        )?;
        after.text = second;
        Ok(Some(vec![
            Command::TrimClip {
                sequence,
                track: lane.id,
                clip,
                edge: crate::ops::TrimEdge::End,
                to: at,
            },
            Command::SetTextProperty {
                sequence,
                track: lane.id,
                clip,
                property: TextProperty::Content(first),
            },
            Command::AddText {
                sequence,
                track: lane.id,
                clip: Box::new(after),
            },
        ]))
    }

    /// Hold each caption until the next one starts when the gap between them
    /// is shorter than `shorter_than` — subtitles that blink off and on
    /// between lines are harder to read than ones that stay. One undo step;
    /// returns how many were held.
    pub fn close_caption_gaps(
        &mut self,
        shorter_than: bettercut_foundation::TimelineTime,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, mut spans)) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| {
                    (
                        t.id,
                        t.clips()
                            .iter()
                            .map(|c| (c.id, c.timeline))
                            .collect::<Vec<_>>(),
                    )
                })
        }) else {
            return Ok(0);
        };
        spans.sort_by_key(|(_, span)| span.start);
        let commands: Vec<Command> = spans
            .windows(2)
            .filter_map(|pair| {
                let ((clip, span), (_, next)) = (pair[0], pair[1]);
                let gap = next.start - span.end;
                (gap > bettercut_foundation::TimelineTime::ZERO && gap < shorter_than).then_some(
                    Command::TrimClip {
                        sequence,
                        track,
                        clip,
                        edge: crate::ops::TrimEdge::End,
                        to: next.start,
                    },
                )
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Close Caption Gaps", commands)?;
        }
        Ok(count)
    }

    /// Remove every caption with nothing but spaces in it — the blanks timing
    /// leaves over pauses nobody typed into. One undo step; returns how many.
    pub fn remove_empty_captions(&mut self) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, empty)) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| {
                    (
                        t.id,
                        t.clips()
                            .iter()
                            .filter(|c| c.text.trim().is_empty())
                            .map(|c| c.id)
                            .collect::<Vec<_>>(),
                    )
                })
        }) else {
            return Ok(0);
        };
        let count = empty.len();
        if count > 0 {
            let commands = empty
                .into_iter()
                .map(|clip| Command::RemoveText {
                    sequence,
                    track,
                    clip,
                })
                .collect();
            self.dispatch_group("Remove Empty Captions", commands)?;
        }
        Ok(count)
    }

    /// Put every caption on the caption lane at height `y`, keeping each one's
    /// sideways place — up out of the way of a lower third. One undo step;
    /// returns how many moved.
    pub fn place_captions(&mut self, y: f32) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, placed)) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| {
                    (
                        t.id,
                        t.clips()
                            .iter()
                            .map(|c| (c.id, c.transform.position))
                            .collect::<Vec<_>>(),
                    )
                })
        }) else {
            return Ok(0);
        };
        let commands: Vec<Command> = placed
            .into_iter()
            .filter(|(_, at)| (at.y - y).abs() > 1e-4)
            .map(|(clip, at)| Command::SetTextProperty {
                sequence,
                track,
                clip,
                property: TextProperty::Position { x: at.x, y },
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group("Place Captions", commands)?;
        }
        Ok(count)
    }

    /// Set every caption on the caption lane in `case`, as one undo step.
    /// Returns how many changed.
    pub fn set_caption_case(&mut self, case: LetterCase) -> Result<usize, EditorError> {
        self.rewrite_captions("Caption Case", |text| case.apply(text))
    }

    /// Rewrite every caption's words with `change`, as one undo step labelled
    /// `label`. Returns how many changed.
    fn rewrite_captions(
        &mut self,
        label: &str,
        change: impl Fn(&str) -> String,
    ) -> Result<usize, EditorError> {
        let sequence = self.active_sequence_id()?;
        let Some((track, texts)) = self.active_sequence().and_then(|s| {
            s.text_tracks
                .iter()
                .find(|t| t.name == Self::CAPTION_TRACK)
                .map(|t| {
                    (
                        t.id,
                        t.clips()
                            .iter()
                            .map(|c| (c.id, c.text.clone()))
                            .collect::<Vec<_>>(),
                    )
                })
        }) else {
            return Ok(0);
        };
        let commands: Vec<Command> = texts
            .into_iter()
            .filter_map(|(clip, text)| {
                let changed = change(&text);
                (changed != text).then_some(Command::SetTextProperty {
                    sequence,
                    track,
                    clip,
                    property: TextProperty::Content(changed),
                })
            })
            .collect();
        let count = commands.len();
        if count > 0 {
            self.dispatch_group(label, commands)?;
        }
        Ok(count)
    }

    /// Replace `find` with `with` in every caption on the caption lane, as
    /// one undo step. Returns how many captions changed.
    pub fn replace_in_captions(
        &mut self,
        find: &str,
        with: &str,
        match_case: bool,
    ) -> Result<usize, EditorError> {
        self.rewrite_captions("Replace in Captions", |text| {
            replace_text(text, find, with, match_case)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::replace_text;

    #[test]
    fn a_line_is_halved_at_the_most_even_space() {
        use super::halve;
        assert_eq!(
            halve("we went down to the sea"),
            Some(("we went down".to_owned(), "to the sea".to_owned()))
        );
        assert_eq!(halve("one"), None);
        assert_eq!(halve("a b"), Some(("a".to_owned(), "b".to_owned())));
    }

    #[test]
    fn letter_cases() {
        use super::LetterCase;
        assert_eq!(LetterCase::Upper.apply("hi there"), "HI THERE");
        assert_eq!(LetterCase::Lower.apply("Hi THERE"), "hi there");
        assert_eq!(
            LetterCase::Sentence.apply("WELL. OK then! who? 3 cats"),
            "Well. Ok then! Who? 3 cats"
        );
    }

    #[test]
    fn every_match_is_replaced_with_or_without_case() {
        assert_eq!(
            replace_text("Jon and jon", "jon", "John", false),
            "John and John"
        );
        assert_eq!(
            replace_text("Jon and jon", "jon", "John", true),
            "Jon and John"
        );
        assert_eq!(
            replace_text("Straße", "SSE", "x", false),
            "Straße",
            "no false match"
        );
        assert_eq!(
            replace_text("Ünïcode ünïcode", "ÜNÏ", "U", false),
            "Ucode Ucode"
        );
        assert_eq!(replace_text("anything", "", "x", false), "anything");
    }
}
