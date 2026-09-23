//! Searching the History window (`history_panel::matches`).

use bettercut_ui::history_panel::matches;

#[test]
fn every_word_must_appear_in_any_order_and_case() {
    assert!(matches("Change Brightness on 3 Clips", "brightness"));
    assert!(matches("Change Brightness on 3 Clips", "clips BRIGHT"));
    assert!(!matches("Change Brightness on 3 Clips", "colour"));
    assert!(
        matches("Split Clip", ""),
        "an empty search matches everything"
    );
}
