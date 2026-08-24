//! Caption errors.

#[derive(Debug, thiserror::Error)]
pub enum CaptionError {
    /// The file parsed, and there was nothing in it. Told apart from a file
    /// full of *unreadable* blocks, which parses to zero captions and a skip
    /// count — the two need different things said about them.
    #[error("no captions in that file")]
    NoCaptions,

    #[error("could not read the caption file: {0}")]
    Io(#[from] std::io::Error),

    /// Caption files are text, and a file that is not UTF-8 is either binary or
    /// in a legacy encoding this does not guess at.
    #[error("that file is not UTF-8 text")]
    NotText,
}
