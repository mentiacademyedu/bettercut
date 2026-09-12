//! Templates (§29–§31, §64, §65, Milestone 11).
//!
//! A template is *data*: a list of elements to place on the timeline, some of
//! them waiting on media or words the user supplies (slots). It is never code
//! (§64), and the engine never sees one that has not been through
//! [`validate`] — the only way to obtain a [`Template`].

pub mod format;
pub mod library;
pub mod starters;
pub mod template;
pub mod validate;

pub use starters::starters;
pub use template::{Element, Slot, SlotKind, Template};
pub use validate::{Problem, SCHEMA_VERSION, is_bundle_relative, parse};
