//! Stickers: an emoji dropped on the picture as its own layer.
//!
//! A title clip underneath, because that is what an emoji is — a character,
//! shaped and drawn by the same renderer — and because it then trims, fades,
//! animates, follows a motion track and exports exactly as a title does,
//! rather than needing any of that written a second time.
//!
//! What a sticker is not is a *title*: no outline, no bold, nothing that makes
//! words readable over footage, and big enough to read as a picture rather
//! than as punctuation.

use bettercut_foundation::ClipId;
use bettercut_text::TextStyle;

use crate::command::Command;
use crate::editor::Editor;
use crate::error::EditorError;

/// The emoji the sticker picker offers, in the order it shows them.
///
/// A short list on purpose: these are the ones that get used, and a grid of
/// two thousand is a worse way to find any of them than typing one into a
/// title. Every one of them is in the font the interface draws with — there is
/// a test that holds this list to that.
pub const STICKERS: [(&str, &str); 40] = [
    ("😀", "Grin"),
    ("😂", "Tears of joy"),
    ("😍", "Heart eyes"),
    ("😎", "Sunglasses"),
    ("😮", "Wow"),
    ("😢", "Sad"),
    ("😡", "Angry"),
    ("👍", "Thumbs up"),
    ("👎", "Thumbs down"),
    ("👏", "Clap"),
    ("🙏", "Thank you"),
    ("💪", "Strong"),
    ("👀", "Eyes"),
    ("🔥", "Fire"),
    ("🎉", "Party"),
    ("💯", "Hundred"),
    ("✨", "Sparkles"),
    ("💥", "Boom"),
    ("💡", "Idea"),
    ("🚀", "Rocket"),
    ("🌈", "Rainbow"),
    ("💬", "Speech"),
    ("✅", "Done"),
    ("❌", "No"),
    ("❤", "Heart"),
    ("★", "Star"),
    ("✔", "Tick"),
    ("✖", "Cross"),
    ("☺", "Smile"),
    ("☹", "Frown"),
    ("☀", "Sun"),
    ("☁", "Cloud"),
    ("☂", "Umbrella"),
    ("☃", "Snowman"),
    ("⚡", "Lightning"),
    ("⏰", "Alarm clock"),
    ("♪", "Music note"),
    ("☎", "Telephone"),
    ("✈", "Aeroplane"),
    ("⚽", "Football"),
];

/// How tall a sticker is drawn, in the same units a title's size is: a good
/// deal larger, because a symbol carries no words to read.
pub const STICKER_SIZE: f32 = 160.0;

impl Editor {
    /// Put `sticker` on the title lane at the playhead, as one undo step.
    /// Returns the clip.
    ///
    /// Sized and styled as a sticker rather than a title: see the module note.
    pub fn add_sticker(&mut self, sticker: &str) -> Result<ClipId, EditorError> {
        let sticker = sticker.trim();
        if sticker.is_empty() {
            return Err(EditorError::NoSticker);
        }
        let sequence_id = self.active_sequence_id()?;
        let track = self
            .active_sequence()
            .and_then(|s| s.text_tracks.first().map(|t| t.id))
            .ok_or(EditorError::NoTextTrack)?;
        let start = self.free_sticker_slot(track);
        let mut clip = bettercut_timeline::TextClip::new(sticker, start)?;
        clip.style = TextStyle {
            size: STICKER_SIZE,
            // An outline round an emoji traces the shape of the character's
            // box, not of the picture inside it, which looks like a mistake.
            stroke: None,
            ..TextStyle::default()
        };
        self.fit_to_frame(&mut clip);
        let id = clip.id;
        self.dispatch(Command::AddText {
            sequence: sequence_id,
            track,
            clip: Box::new(clip),
        })?;
        Ok(id)
    }

    /// Whether a clip is a sticker: one symbol and nothing else on the title
    /// lane. What the inspector uses to call it a sticker rather than a title.
    pub fn is_sticker(&self, clip: ClipId) -> bool {
        self.text_clip(clip).is_some_and(|text| {
            let trimmed = text.text.trim();
            !trimmed.is_empty()
                && trimmed.chars().count() <= 2
                && trimmed.chars().all(|c| !c.is_alphanumeric())
        })
    }
}
