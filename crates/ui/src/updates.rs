//! Telling a tester a newer bettercut exists.
//!
//! At most once a day, at start, one request to GitHub's public list of
//! releases — through the system's own `curl` (in Windows 10 and later, every
//! Mac and most Linux), so the editor carries no networking code of its own.
//! Nothing is sent but the request itself. A newer version is said in the
//! status bar with a link to its download; nothing installs by itself. Off
//! with one switch in Settings.

use std::sync::{Arc, Mutex};

/// Where the releases are listed, newest first (pre-releases included:
/// every bettercut release is one while it is a beta).
pub const RELEASES: &str =
    "https://api.github.com/repos/mentiacademyedu/bettercut/releases?per_page=5";

/// How long between checks.
pub const EVERY_SECONDS: u64 = 24 * 60 * 60;

/// A newer release: its version and the page to get it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// What a check found, written by its thread and read by the status bar.
pub type Found = Arc<Mutex<Option<Update>>>;

/// Whether a check is due, `last` and `now` in seconds since 1970.
pub fn due(last: u64, now: u64) -> bool {
    now.saturating_sub(last) >= EVERY_SECONDS
}

/// `0.5.2` as numbers; a `v` in front and anything after a `-` are ignored.
fn numbers(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split('-').next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Whether `latest` is a later version than `current`.
pub fn newer(current: &str, latest: &str) -> bool {
    match (numbers(current), numbers(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// The newest published release in GitHub's reply.
pub fn latest(reply: &str) -> Option<Update> {
    let releases: serde_json::Value = serde_json::from_str(reply).ok()?;
    releases.as_array()?.iter().find_map(|release| {
        if release["draft"].as_bool() == Some(true) {
            return None;
        }
        Some(Update {
            version: release["tag_name"]
                .as_str()?
                .trim_start_matches('v')
                .to_owned(),
            url: release["html_url"].as_str()?.to_owned(),
        })
    })
}

/// Ask GitHub on a thread of its own; `found` holds a newer release if there
/// is one, and `wake` is called so the window draws it.
pub fn check(current: &'static str, found: Found, wake: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let mut command = std::process::Command::new("curl");
        command.args([
            "--silent",
            "--fail",
            "--max-time",
            "15",
            "--header",
            "Accept: application/vnd.github+json",
            "--user-agent",
            "bettercut",
            RELEASES,
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window flashing up at start.
            command.creation_flags(0x0800_0000);
        }
        let Ok(output) = command.output() else {
            tracing::debug!("no curl to check for updates with");
            return;
        };
        if !output.status.success() {
            tracing::debug!("the update check could not reach GitHub");
            return;
        }
        let reply = String::from_utf8_lossy(&output.stdout);
        if let Some(update) = latest(&reply).filter(|u| newer(current, &u.version)) {
            tracing::info!(version = %update.version, "a newer bettercut is out");
            if let Ok(mut slot) = found.lock() {
                *slot = Some(update);
            }
            wake();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("0.5.2", "0.5.10"));
        assert!(newer("0.5.2", "v0.6.0"));
        assert!(newer("0.9.9", "1.0.0"));
        assert!(!newer("0.5.2", "0.5.2"));
        assert!(!newer("0.5.2", "0.5.1"));
        assert!(!newer("0.5.2", "nonsense"));
        assert!(!newer("0.5.2-dev", "0.5.2"), "a suffix is ignored");
    }

    #[test]
    fn the_newest_published_release_is_read() {
        let reply = r#"[
            { "tag_name": "v0.7.0", "html_url": "https://x/0.7.0", "draft": true },
            { "tag_name": "v0.6.0", "html_url": "https://x/0.6.0", "draft": false, "prerelease": true },
            { "tag_name": "v0.5.2", "html_url": "https://x/0.5.2", "draft": false }
        ]"#;
        assert_eq!(
            latest(reply),
            Some(Update {
                version: "0.6.0".to_owned(),
                url: "https://x/0.6.0".to_owned()
            })
        );
        assert_eq!(latest("{ \"message\": \"rate limited\" }"), None);
        assert_eq!(latest("not json"), None);
    }

    #[test]
    fn once_a_day() {
        assert!(due(0, EVERY_SECONDS));
        assert!(!due(1_000, 1_000 + EVERY_SECONDS - 1));
        assert!(due(1_000, 1_000 + EVERY_SECONDS));
        assert!(!due(5_000, 1_000), "a clock gone backwards is not a day");
    }
}
