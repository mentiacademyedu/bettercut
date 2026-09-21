//! Word-by-word highlight: which word of a caption is being said.
//!
//! The captions in a project are timed by the line — a subtitle file says when
//! each line shows, not when each word in it is spoken. So the words share the
//! line's time by how long they are: a long word takes longer to say than a
//! short one, and counting its letters (plus the space after it) is a good
//! enough stand-in that the highlight keeps pace with ordinary speech.
//!
//! Integer arithmetic on ticks (§74), so the preview and the export light the
//! same word on the same frame.

use std::ops::Range;

use bettercut_foundation::TimelineTime;

/// Every word in `text`: runs of characters between whitespace, as character
/// indices (newlines counted), end exclusive.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = None;
    let mut count = 0;
    for (index, c) in text.chars().enumerate() {
        count = index + 1;
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(index),
            (true, Some(from)) => {
                words.push(from..index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push(from..count);
    }
    words
}

/// The word being said `into_clip` after a caption `duration` long starts,
/// each word given time in proportion to its length plus one. `None` for
/// text with no words, before the start, or at or after the end.
pub fn word_at(
    text: &str,
    into_clip: TimelineTime,
    duration: TimelineTime,
) -> Option<Range<usize>> {
    let words = words(text);
    if words.is_empty() || into_clip < TimelineTime::ZERO || into_clip >= duration {
        return None;
    }
    let weight = |word: &Range<usize>| (word.end - word.start + 1) as i128;
    let total: i128 = words.iter().map(weight).sum();
    // How far through the weights this instant is, exactly.
    let reached = i128::from(into_clip.ticks()) * total / i128::from(duration.ticks().max(1));
    let mut end = 0;
    for word in &words {
        end += weight(word);
        if reached < end {
            return Some(word.clone());
        }
    }
    words.last().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: i64) -> TimelineTime {
        TimelineTime::from_millis(n)
    }

    #[test]
    fn words_are_runs_between_whitespace() {
        assert_eq!(words("  hello  big\nworld "), vec![2..7, 9..12, 13..18]);
        assert_eq!(
            words("é à"),
            vec![0..1, 2..3],
            "counted in characters, not bytes"
        );
        assert!(words(" \n ").is_empty());
    }

    /// "a" weighs 2 and "long" 5, over 7 s: "a" for the first 2 s.
    #[test]
    fn each_word_gets_time_by_its_length() {
        let text = "a long";
        let duration = ms(7_000);
        assert_eq!(word_at(text, ms(0), duration), Some(0..1));
        assert_eq!(word_at(text, ms(1_999), duration), Some(0..1));
        assert_eq!(word_at(text, ms(2_000), duration), Some(2..6));
        assert_eq!(word_at(text, ms(6_999), duration), Some(2..6));
    }

    #[test]
    fn nothing_is_lit_outside_the_clip_or_without_words() {
        assert_eq!(word_at("hi there", ms(-1), ms(1_000)), None);
        assert_eq!(word_at("hi there", ms(1_000), ms(1_000)), None);
        assert_eq!(word_at("   ", ms(10), ms(1_000)), None);
    }

    /// Every word is lit at some point, in order.
    #[test]
    fn every_word_takes_its_turn() {
        let text = "the quick brown fox jumps over the lazy dog";
        let duration = ms(3_000);
        let mut seen: Vec<Range<usize>> = Vec::new();
        for at in (0..3_000).step_by(10) {
            let word = word_at(text, ms(at), duration).unwrap();
            if seen.last() != Some(&word) {
                seen.push(word);
            }
        }
        assert_eq!(seen, words(text));
    }
}
