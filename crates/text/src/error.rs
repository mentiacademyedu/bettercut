//! Text errors.

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// The text laid out to nothing at all — every character was whitespace,
    /// or the string was empty. Not a failure the user needs telling about;
    /// the caller draws no layer.
    #[error("text laid out to an empty bitmap")]
    Empty,

    /// A bitmap larger than any sensible frame. Reachable only from a project
    /// file that was edited by hand, but it would be an enormous allocation.
    #[error("text would rasterize to {width}×{height}, which is past the limit")]
    TooLarge { width: u32, height: u32 },
}
