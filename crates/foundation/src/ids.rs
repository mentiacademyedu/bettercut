//! Entity identifiers.
//!
//! Every ID is a distinct newtype over a UUID. A `TrackId` cannot be passed
//! where a `ClipId` is expected, which matters more than it sounds: most of the
//! editing commands in §10 take two or three IDs of different kinds.
//!
//! UUIDs rather than indices, because §37 stores projects as JSON that outlives
//! the session and §66 must relink media across machines. An index into a `Vec`
//! is invalidated by the first delete.

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(uuid::Uuid);

        impl $name {
            /// Mint a fresh, random ID.
            #[allow(clippy::new_without_default)] // an ID has no meaningful default
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }

            /// Reconstruct a known ID — deserialization and tests.
            pub const fn from_uuid(uuid: uuid::Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(self) -> uuid::Uuid {
                self.0
            }

            /// Short form for logs and UI (§49 logs IDs, not file contents).
            pub fn short(self) -> String {
                self.0.simple().to_string()[..8].to_owned()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(ProjectId, "Identifies a project (§8).");
define_id!(SequenceId, "Identifies a sequence within a project (§8).");
define_id!(TrackId, "Identifies a video or audio track (§8).");
define_id!(ClipId, "Identifies a clip on a track (§8).");
define_id!(
    MediaId,
    "Identifies an imported media asset (§12).\n\nStable across sessions; §84 will derive it from path, size, and hash so re-importing the same file reuses its proxies and thumbnails."
);
define_id!(EffectId, "Identifies an effect instance on a clip (§23).");
define_id!(MarkerId, "Identifies a timeline marker (§35).");
define_id!(
    LinkId,
    "Ties a video clip to the sound that came from the same file (§12, §51). Picture and sound live on separate tracks (§8), so a video file is two clips; what makes them one *thing* is a shared link, so that re-timing the picture re-times the sound with it."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = ClipId::new();
        let b = ClipId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_round_trip_through_json() {
        let id = MediaId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        // #[serde(transparent)] means the JSON is a bare string, not an object.
        assert!(json.starts_with('"'), "unexpected encoding: {json}");
        let back: MediaId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn short_form_is_eight_chars() {
        assert_eq!(TrackId::new().short().len(), 8);
    }
}
