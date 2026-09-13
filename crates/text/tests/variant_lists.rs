//! Every `ALL` list in the text crate, held to the enum it belongs to.
//!
//! The companion to the timeline crate's file of the same name, for the same
//! reason: an `ALL` is a hand-written list and nothing makes it complete. A
//! weight, an alignment or a look added to the model and left out of its list
//! is a control that never appears — no error, no warning, just a choice the
//! user cannot make.
//!
//! Each test classifies its enum with an **exhaustive match**, which is the
//! only completeness check the compiler enforces, and then asserts the list is
//! the same length. A new variant stops this compiling until someone has put it
//! somewhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bettercut_text::{Alignment, CaptionLook, FontWeight, TitleLook};

#[test]
fn every_font_weight_is_offered() {
    fn offered(weight: FontWeight) -> bool {
        match weight {
            FontWeight::Light | FontWeight::Regular | FontWeight::Medium | FontWeight::Bold => true,
        }
    }

    assert_eq!(FontWeight::ALL.len(), 4, "a weight was added or removed");
    assert!(FontWeight::ALL.into_iter().all(offered));
}

#[test]
fn every_alignment_is_offered() {
    fn offered(align: Alignment) -> bool {
        match align {
            Alignment::Left | Alignment::Center | Alignment::Right => true,
        }
    }

    assert_eq!(Alignment::ALL.len(), 3, "an alignment was added or removed");
    assert!(Alignment::ALL.into_iter().all(offered));
}

#[test]
fn every_title_look_is_offered() {
    fn offered(look: TitleLook) -> bool {
        match look {
            TitleLook::Headline
            | TitleLook::LowerThird
            | TitleLook::Quote
            | TitleLook::Typewriter => true,
        }
    }

    assert_eq!(TitleLook::ALL.len(), 4, "a title look was added or removed");
    assert!(TitleLook::ALL.into_iter().all(offered));
}

#[test]
fn every_caption_look_is_offered() {
    fn offered(look: CaptionLook) -> bool {
        match look {
            CaptionLook::Boxed
            | CaptionLook::Outlined
            | CaptionLook::Highlight
            | CaptionLook::Soft => true,
        }
    }

    assert_eq!(
        CaptionLook::ALL.len(),
        4,
        "a caption look was added or removed"
    );
    assert!(CaptionLook::ALL.into_iter().all(offered));
}

/// Every look has to *say* something, or it appears in the picker as an empty
/// row. Cheap to check, and the kind of omission a new look invites.
#[test]
fn every_look_has_a_name() {
    for look in TitleLook::ALL {
        assert!(!look.label().trim().is_empty(), "a title look has no name");
    }
    for look in CaptionLook::ALL {
        assert!(
            !look.label().trim().is_empty(),
            "a caption look has no name"
        );
    }
    for weight in FontWeight::ALL {
        assert!(!weight.label().trim().is_empty(), "a weight has no name");
    }
}
