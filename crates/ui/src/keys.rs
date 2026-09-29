//! Keyboard shortcuts as each platform writes them.
//!
//! The shortcuts themselves already follow the platform: they are matched on
//! egui's "command" modifier, which is Ctrl on Windows and Linux and ⌘ on a
//! Mac. Only the words shown for them were Windows words. Text is written the
//! Windows way everywhere in the code, and passed through [`keys`] where it is
//! shown.
//!
//! On a Mac, Ctrl becomes ⌘ and Alt becomes Option. ⌘ is in egui's bundled
//! fonts; the ⌥ and ⇧ symbols are not, so Option and Shift stay words.

use std::borrow::Cow;

/// `text` with its shortcuts written for this platform.
pub fn keys(text: &str) -> Cow<'_, str> {
    keys_for(text, cfg!(target_os = "macos"))
}

/// [`keys`] for a Mac (`mac`) or anywhere else, so both can be tested.
pub fn keys_for(text: &str, mac: bool) -> Cow<'_, str> {
    if !mac || !(text.contains("Ctrl+") || text.contains("Alt+")) {
        return Cow::Borrowed(text);
    }
    // Longest first, so the modifier order comes out the Mac way round.
    Cow::Owned(
        text.replace("Ctrl+Shift+", "Shift+\u{2318}")
            .replace("Ctrl+Alt+", "Option+\u{2318}")
            .replace("Ctrl+", "\u{2318}")
            .replace("Alt+", "Option+"),
    )
}

#[cfg(test)]
mod tests {
    use super::keys_for;

    #[test]
    fn a_mac_reads_command_where_windows_reads_ctrl() {
        assert_eq!(keys_for("Ctrl+K any action", true), "\u{2318}K any action");
        assert_eq!(keys_for("Ctrl+Shift+Z", true), "Shift+\u{2318}Z");
        assert_eq!(keys_for("Ctrl+Alt+C", true), "Option+\u{2318}C");
        assert_eq!(keys_for("Alt+Left", true), "Option+Left");
        assert_eq!(keys_for("Shift+Del ripple", true), "Shift+Del ripple");
    }

    #[test]
    fn everywhere_else_is_unchanged() {
        assert_eq!(keys_for("Ctrl+Shift+Z", false), "Ctrl+Shift+Z");
    }
}
