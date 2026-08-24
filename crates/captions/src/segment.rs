//! What a caption is (§27).

use bettercut_foundation::TimelineTime;
use serde::{Deserialize, Serialize};

/// One word, timed (§27).
///
/// Word-level timing is what karaoke captions, word highlighting and
/// TikTok-style animated subtitles are built on — §27 lists all three. None of
/// them are rendered yet, but the timing is carried from import so that
/// building them later does not mean re-transcribing everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionWord {
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub text: String,
}

/// One caption: a span of time and what is said in it (§27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionSegment {
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub text: String,
    /// Empty for a subtitle file, which times whole lines. Populated by a
    /// transcriber (§28), which knows where each word fell.
    #[serde(default)]
    pub words: Vec<CaptionWord>,
}

impl CaptionSegment {
    pub fn new(start: TimelineTime, end: TimelineTime, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
            words: Vec::new(),
        }
    }

    pub fn duration(&self) -> TimelineTime {
        TimelineTime::from_ticks((self.end.ticks() - self.start.ticks()).max(0))
    }

    /// Whether this segment says nothing.
    ///
    /// Subtitle files are full of these — a blank cue used as a spacer, or a
    /// stray index with no text under it. They are not errors, and they are not
    /// captions either.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// The shortest caption worth showing.
///
/// A cue below about a fifth of a second is a flash rather than something read.
/// Files converted between frame rates routinely contain a few.
pub const MIN_DURATION: TimelineTime = TimelineTime::from_ticks(192_000);

/// Put a list of captions into a state a track will accept.
///
/// Subtitle files are written by hand and by a dozen different tools, and they
/// arrive out of order, overlapping, empty, and occasionally backwards. §8's
/// tracks are sorted and non-overlapping, so *something* has to reconcile the
/// two — and doing it here, once, beats every caller discovering the same
/// surprises.
///
/// The rules, in order:
///
/// * blank cues are dropped — there is nothing to show;
/// * a cue that ends before it starts is dropped, not silently reversed: it is
///   a corrupt file, and guessing which end was meant would be worse;
/// * the rest are sorted by start time;
/// * a cue overlapping the next one is **shortened**, never moved. A caption is
///   anchored to the moment the words are said, and sliding it later would put
///   the subtitle after the speech;
/// * a cue left shorter than [`MIN_DURATION`] by that shortening is dropped
///   rather than left as a flash.
pub fn tidy(mut segments: Vec<CaptionSegment>) -> Vec<CaptionSegment> {
    segments.retain(|s| !s.is_blank() && s.end.ticks() > s.start.ticks());
    segments.sort_by_key(|s| s.start.ticks());

    let mut out: Vec<CaptionSegment> = Vec::with_capacity(segments.len());
    for mut segment in segments {
        if let Some(previous) = out.last_mut()
            && previous.end.ticks() > segment.start.ticks()
        {
            previous.end = segment.start;
            if previous.duration().ticks() < MIN_DURATION.ticks() {
                out.pop();
            }
        }
        // A cue starting before the previous one ended *and* ending before it
        // did — one caption entirely inside another — has nothing left after
        // the shortening above. Dropping it beats emitting a zero-length clip.
        if let Some(previous) = out.last()
            && segment.start.ticks() < previous.end.ticks()
        {
            segment.start = previous.end;
        }
        if segment.duration().ticks() >= MIN_DURATION.ticks() {
            out.push(segment);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: i64) -> TimelineTime {
        TimelineTime::from_millis(n)
    }

    fn cue(start: i64, end: i64, text: &str) -> CaptionSegment {
        CaptionSegment::new(ms(start), ms(end), text)
    }

    #[test]
    fn tidying_sorts_by_start_time() {
        let out = tidy(vec![cue(2000, 3000, "second"), cue(0, 1000, "first")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "first");
        assert_eq!(out[1].text, "second");
    }

    #[test]
    fn blank_cues_are_dropped() {
        let out = tidy(vec![cue(0, 1000, "  "), cue(2000, 3000, "words")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "words");
    }

    /// A cue that ends before it begins is a corrupt file. Reversing it would
    /// be a guess about which end was meant.
    #[test]
    fn a_backwards_cue_is_dropped() {
        let out = tidy(vec![cue(3000, 1000, "backwards")]);
        assert!(out.is_empty());
    }

    /// The overlapping cue keeps its start: a caption is anchored to the moment
    /// the words are said, and sliding it later would put the subtitle after
    /// the speech.
    #[test]
    fn an_overlap_shortens_the_earlier_cue() {
        let out = tidy(vec![cue(0, 3000, "first"), cue(2000, 4000, "second")]);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].end, ms(2000), "the earlier cue was not shortened");
        assert_eq!(out[1].start, ms(2000), "the later cue moved");
        assert_eq!(out[1].end, ms(4000));
    }

    /// Shortening can leave nothing worth showing.
    #[test]
    fn a_cue_shortened_to_a_flash_is_dropped() {
        let out = tidy(vec![cue(0, 3000, "first"), cue(50, 4000, "second")]);
        assert_eq!(out.len(), 1, "a 50 ms flash survived");
        assert_eq!(out[0].text, "second");
    }

    /// One caption entirely inside another leaves the inner one with no room.
    #[test]
    fn a_nested_cue_does_not_produce_a_zero_length_clip() {
        let out = tidy(vec![cue(0, 5000, "outer"), cue(1000, 2000, "inner")]);
        for segment in &out {
            assert!(
                segment.duration().ticks() >= MIN_DURATION.ticks(),
                "{} ran for {} ticks",
                segment.text,
                segment.duration().ticks()
            );
        }
        // And whatever survived is still in order and does not overlap.
        for pair in out.windows(2) {
            assert!(pair[0].end.ticks() <= pair[1].start.ticks());
        }
    }

    /// The invariant a track needs, over a deliberately awful list.
    #[test]
    fn the_result_is_always_sorted_and_non_overlapping() {
        let out = tidy(vec![
            cue(5000, 6000, "e"),
            cue(0, 2500, "a"),
            cue(2000, 2100, "b"),
            cue(2000, 4000, "c"),
            cue(0, 0, "zero"),
            cue(9000, 1000, "backwards"),
            cue(4000, 5000, "   "),
            cue(4500, 7000, "d"),
        ]);

        for pair in out.windows(2) {
            assert!(
                pair[0].end.ticks() <= pair[1].start.ticks(),
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
        for segment in &out {
            assert!(!segment.is_blank());
            assert!(segment.duration().ticks() >= MIN_DURATION.ticks());
        }
    }
}
