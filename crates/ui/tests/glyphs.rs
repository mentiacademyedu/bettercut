//! Every character the interface draws must exist in the font family that will
//! actually be asked to draw it.
//!
//! egui ships its own fonts and does **not** fall back to the system's. A
//! character no font in the relevant family covers renders as an empty box —
//! silently, at runtime, on whatever screen the user is looking at. The toolbar
//! carries a comment about this because it already happened, with `↶` and `↷`
//! for undo and redo, and the fix was to go back to words.
//!
//! ## Why the family matters, not just "is it bundled"
//!
//! Writing this test turned up something the original comment did not say.
//! `↶` **is** bundled — it is in Hack. But Hack is the *monospace* font, and
//! egui's proportional family is Ubuntu-Light plus the two emoji fonts. A
//! button draws proportional, so it never sees Hack, and the arrow boxes. The
//! comment's conclusion was right and its reason was incomplete.
//!
//! So the check is per family. Asking only "is this character in any bundled
//! font" would have called `↶` fine and let the bug straight back in.
//!
//! Adding a symbol to the interface means adding it here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ab_glyph::{Font, FontRef};
use egui::FontFamily;

/// Characters the interface draws, and the family that draws them.
///
/// Plain ASCII is not listed — every font covers it, and the list is for the
/// ones worth doubting.
const USED: &[(char, FontFamily, &str)] = &[
    (
        '🗁',
        FontFamily::Proportional,
        "export dialog: the Browse button",
    ),
    (
        '◀',
        FontFamily::Proportional,
        "inspector: previous keyframe",
    ),
    ('▶', FontFamily::Proportional, "inspector: next keyframe"),
    (
        '♦',
        FontFamily::Proportional,
        "inspector: a keyframe is on this frame",
    ),
    (
        '◊',
        FontFamily::Proportional,
        "inspector: the parameter is animated",
    ),
    (
        '○',
        FontFamily::Proportional,
        "inspector: the parameter is not animated",
    ),
    (
        '·',
        FontFamily::Proportional,
        "export dialog: separators in the summary line",
    ),
    ('×', FontFamily::Proportional, "export dialog: 1920×1080"),
    ('—', FontFamily::Proportional, "status and hint text"),
    (
        '…',
        FontFamily::Proportional,
        "every truncated label, and every button that opens a dialog",
    ),
    (
        '↺',
        FontFamily::Proportional,
        "inspector: reset one control to its default",
    ),
    (
        '·',
        FontFamily::Monospace,
        "inspector: the System diagnostics block",
    ),
];

/// Whether any font in `family` can draw `c`.
///
/// This is the question egui itself answers when it lays text out: it walks the
/// family in order and takes the first font with a glyph.
fn is_drawable(c: char, family: &FontFamily) -> bool {
    let definitions = egui::FontDefinitions::default();
    let Some(names) = definitions.families.get(family) else {
        panic!("egui has no {family:?} family, which cannot be right");
    };

    names.iter().any(|name| {
        definitions.font_data.get(name).is_some_and(|data| {
            FontRef::try_from_slice_and_index(&data.font, data.index)
                .is_ok_and(|font| font.glyph_id(c).0 != 0)
        })
    })
}

#[test]
fn every_symbol_the_interface_uses_has_a_glyph() {
    let missing: Vec<String> = USED
        .iter()
        .filter(|(c, family, _)| !is_drawable(*c, family))
        .map(|(c, family, used)| format!("U+{:04X} in {family:?} ({used})", *c as u32))
        .collect();

    assert!(
        missing.is_empty(),
        "these render as empty boxes, because egui bundles its own fonts and \
         does not fall back to the system's:\n  {}",
        missing.join("\n  ")
    );
}

/// Proof the check can fail. Without this, a `glyph_id` that always returned
/// non-zero — or a family list that silently went empty — would make every
/// character look fine forever.
#[test]
fn a_character_no_bundled_font_covers_is_reported_missing() {
    // U+10FFFD: the last private-use code point. Nothing sane maps it.
    assert!(
        !is_drawable('\u{10FFFD}', &FontFamily::Proportional),
        "the check passes characters that cannot possibly be in a font, so it \
         would pass anything"
    );
}

/// The characters that started this, pinned so the toolbar's decision to spell
/// its buttons out stays explicable.
///
/// The fullwidth plus and minus are in no bundled font at all. The undo arrows
/// are in Hack only — which is why they must not appear on a proportional
/// button, and why this test asks per family rather than per font.
#[test]
fn the_toolbar_symbols_are_still_unavailable_where_it_matters() {
    for (symbol, note) in [
        ('↶', "undo arrow: Hack only, and buttons draw proportional"),
        ('↷', "redo arrow: Hack only"),
        ('＋', "fullwidth plus: in no bundled font"),
        ('－', "fullwidth minus: in no bundled font"),
        (
            '◆',
            "geometric black diamond: Hack only, so it boxed on a button",
        ),
        ('◇', "geometric white diamond: Hack only"),
    ] {
        assert!(
            !is_drawable(symbol, &FontFamily::Proportional),
            "U+{:04X} is drawable in the proportional family now ({note}). The \
             toolbar could use it — but confirm it renders before trusting this.",
            symbol as u32
        );
    }

    // And the halves of that claim that are *not* true, so the reason stays on
    // the record rather than being rediscovered.
    assert!(
        is_drawable('↶', &FontFamily::Monospace),
        "the undo arrow was in Hack when this was written; if it no longer is, \
         the explanation above needs revisiting"
    );
}
