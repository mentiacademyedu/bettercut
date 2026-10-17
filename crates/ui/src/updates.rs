//! Telling a tester a newer bettercut exists, and on Windows installing it.
//!
//! At most once a day, at start, one request to GitHub's public list of
//! releases — through the system's own `curl` (in Windows 10 and later, every
//! Mac and most Linux), so the editor carries no networking code of its own.
//! Nothing is sent but the request itself. A newer version is said in the
//! status bar with a link to its download. Off with one switch in Settings.
//!
//! Where bettercut was put in place by its installer, the bar also offers to
//! update: the new installer is downloaded only when asked, checked against
//! the SHA-256 GitHub lists for it (with Windows' own `certutil`), and run —
//! silently, and starting bettercut again when it is done — only once the
//! window is closing, after any unsaved work has been asked about. Nothing
//! installs by itself.
//!
//! On Linux the AppImage is the whole program in one file, so updating is
//! replacing that file: the new one is downloaded beside it, checked with
//! `sha256sum`, and put in its place as the window closes — then started.

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
    /// Its Windows installer, when the release has one with a digest.
    pub installer: Option<Installer>,
}

/// A release's Windows installer, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub name: String,
    pub url: String,
    /// The SHA-256 GitHub worked out for it, in lower-case hex.
    pub sha256: String,
    pub size: u64,
}

/// How the download of an update is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Download {
    /// Bytes so far, of how many.
    Fetching { got: u64, of: u64 },
    /// Downloaded and checked: the installer, waiting to be run.
    Ready(std::path::PathBuf),
    /// Why there is nothing to install.
    Failed(String),
}

/// The download, written by its thread and read by the status bar.
pub type Downloading = Arc<Mutex<Option<Download>>>;

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
            installer: installer_suffix().and_then(|suffix| installer_of(release, suffix)),
        })
    })
}

/// What this system updates itself from: the installer on Windows, the
/// AppImage on Linux. A Mac is updated from the download page.
fn installer_suffix() -> Option<&'static str> {
    if cfg!(windows) {
        Some("-setup.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("-x86_64.AppImage")
    } else {
        None
    }
}

/// The release's file ending in `suffix`, if it lists one with a SHA-256.
fn installer_of(release: &serde_json::Value, suffix: &str) -> Option<Installer> {
    release["assets"].as_array()?.iter().find_map(|asset| {
        let name = asset["name"].as_str()?;
        if !name.ends_with(suffix) {
            return None;
        }
        let sha256 = asset["digest"].as_str()?.strip_prefix("sha256:")?;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Installer {
            name: name.to_owned(),
            url: asset["browser_download_url"].as_str()?.to_owned(),
            sha256: sha256.to_ascii_lowercase(),
            size: asset["size"].as_u64().unwrap_or(0),
        })
    })
}

/// Whether this copy of bettercut can update itself: on Windows, run from
/// where the installer put it — beside its uninstaller; a copy unpacked from
/// the zip is the tester's own to replace. On Linux, run as an AppImage,
/// which says where its file is.
pub fn can_install_here() -> bool {
    if cfg!(windows) {
        return std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("unins000.exe").is_file()))
            .unwrap_or(false);
    }
    installer_suffix().is_some() && appimage().is_some()
}

/// The AppImage this copy runs from, when it runs from one.
fn appimage() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("APPIMAGE")?);
    path.is_file().then_some(path)
}

/// Whether `sha256sum <file>`'s output names `expected`: the hash, then the
/// file's name.
pub fn sha256sum_says(output: &str, expected: &str) -> bool {
    output
        .split_whitespace()
        .next()
        .is_some_and(|hex| hex.eq_ignore_ascii_case(expected))
}

/// Whether `certutil -hashfile <file> SHA256`'s output names `expected`.
/// The hash is its second line, in pairs separated by spaces on older
/// Windows and run together on newer.
pub fn certutil_says(output: &str, expected: &str) -> bool {
    output.lines().nth(1).is_some_and(|line| {
        let hex: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        hex.eq_ignore_ascii_case(expected)
    })
}

/// Fetch `installer` on a thread of its own into the system's temporary
/// folder, then check it; `state` follows along, and `wake` is called each
/// time it changes so the bar redraws.
pub fn download(installer: Installer, state: Downloading, wake: impl Fn() + Send + 'static) {
    let set = move |value: Download, state: &Downloading| {
        if let Ok(mut slot) = state.lock() {
            *slot = Some(value);
        }
    };
    set(
        Download::Fetching {
            got: 0,
            of: installer.size,
        },
        &state,
    );
    std::thread::spawn(move || {
        // On Linux beside the AppImage it will replace, so the swap is a
        // rename on one disk; elsewhere in the temporary folder.
        let folder = appimage()
            .filter(|_| !cfg!(windows))
            .and_then(|image| image.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| std::env::temp_dir().join("bettercut-update"));
        let path = folder.join(format!(".{}.download", installer.name));
        let result = (|| -> Result<(), String> {
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&path);
            let mut command = std::process::Command::new("curl");
            command
                .args([
                    "--silent",
                    "--fail",
                    "--location",
                    "--user-agent",
                    "bettercut",
                    "--output",
                ])
                .arg(&path)
                .arg(&installer.url);
            hide_console(&mut command);
            let mut child = command
                .spawn()
                .map_err(|_| "there is no curl to download with".to_owned())?;
            loop {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    if !status.success() {
                        return Err("the download did not finish".to_owned());
                    }
                    break;
                }
                let got = std::fs::metadata(&path).map_or(0, |m| m.len());
                set(
                    Download::Fetching {
                        got,
                        of: installer.size,
                    },
                    &state,
                );
                wake();
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            let (tool, says): (&str, fn(&str, &str) -> bool) = if cfg!(windows) {
                ("certutil", certutil_says)
            } else {
                ("sha256sum", sha256sum_says)
            };
            let mut command = std::process::Command::new(tool);
            if cfg!(windows) {
                command.arg("-hashfile").arg(&path).arg("SHA256");
            } else {
                command.arg(&path);
            }
            hide_console(&mut command);
            let output = command
                .output()
                .map_err(|_| "the download could not be checked".to_owned())?;
            if !says(&String::from_utf8_lossy(&output.stdout), &installer.sha256) {
                return Err("the download is not the file GitHub lists".to_owned());
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                tracing::info!(file = %path.display(), "update downloaded and checked");
                set(Download::Ready(path), &state);
            }
            Err(reason) => {
                tracing::warn!(%reason, "update download failed");
                let _ = std::fs::remove_file(&path);
                set(Download::Failed(reason), &state);
            }
        }
        wake();
    });
}

/// Run the downloaded installer: silently, closing what is in its way, and
/// starting bettercut again once it is done — or on Linux, put the new
/// AppImage where the old one was and start it. Called as the window closes.
pub fn install(installer: &std::path::Path) -> std::io::Result<()> {
    if !cfg!(windows) {
        let image = appimage().ok_or_else(|| std::io::Error::other("not run as an AppImage"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(installer, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(installer, &image)?;
        return std::process::Command::new(&image).spawn().map(|_| ());
    }
    let mut command = std::process::Command::new(installer);
    command.args([
        "/SILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/CLOSEAPPLICATIONS",
        "/relaunch=1",
    ]);
    command.spawn().map(|_| ())
}

/// No console window flashing up for a tool run in the background.
fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
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
                url: "https://x/0.6.0".to_owned(),
                installer: None,
            })
        );
        assert_eq!(latest("{ \"message\": \"rate limited\" }"), None);
        assert_eq!(latest("not json"), None);
    }

    #[test]
    fn the_installer_is_found_with_its_digest() {
        let digest = "BAF786EB1E7DEAA8F4130FFAD3ABF2A25597051BC6E43E10EACD41C6AD854620";
        let reply = format!(
            r#"[{{ "tag_name": "v0.7.0", "html_url": "https://x/0.7.0", "draft": false,
                "assets": [
                    {{ "name": "bettercut-0.7.0-macos.dmg", "digest": "sha256:{digest}",
                       "browser_download_url": "https://x/dmg", "size": 3 }},
                    {{ "name": "bettercut-0.7.0-setup.exe", "digest": "sha256:{digest}",
                       "browser_download_url": "https://x/exe", "size": 57000000 }}
                ] }}]"#
        );
        let release = &serde_json::from_str::<serde_json::Value>(&reply).unwrap()[0];
        let installer = installer_of(release, "-setup.exe").expect("the setup.exe");
        assert_eq!(installer.name, "bettercut-0.7.0-setup.exe");
        assert_eq!(installer.url, "https://x/exe");
        assert_eq!(installer.sha256, digest.to_ascii_lowercase());
        assert_eq!(installer.size, 57_000_000);

        // A system wanting another kind of file finds none here.
        assert_eq!(installer_of(release, "-x86_64.AppImage"), None);

        // No digest, nothing to check a download against: no installer.
        let unchecked: serde_json::Value = serde_json::from_str(
            r#"{ "assets": [{ "name": "bettercut-0.7.0-setup.exe",
                              "browser_download_url": "https://x/exe", "size": 5 }] }"#,
        )
        .unwrap();
        assert_eq!(installer_of(&unchecked, "-setup.exe"), None);
    }

    #[test]
    fn certutil_is_read_either_way_it_prints() {
        let hash = "baf786eb1e7deaa8f4130ffad3abf2a25597051bc6e43e10eacd41c6ad854620";
        let newer = format!(
            "SHA256 hash of C:\\x.exe:\r\n{hash}\r\nCertUtil: -hashfile command completed successfully.\r\n"
        );
        assert!(certutil_says(&newer, hash));
        let spaced: String = hash
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let older = format!("SHA256 hash of file x.exe:\n{spaced}\nCertUtil: done.\n");
        assert!(certutil_says(&older, hash));
        assert!(!certutil_says(&newer, &hash.replace('b', "c")));
        assert!(!certutil_says("CertUtil: error", hash));
    }

    #[test]
    fn sha256sum_is_read() {
        let hash = "baf786eb1e7deaa8f4130ffad3abf2a25597051bc6e43e10eacd41c6ad854620";
        assert!(sha256sum_says(
            &format!("{hash}  /home/me/bettercut.AppImage\n"),
            hash
        ));
        assert!(!sha256sum_says(
            &format!("{hash}  x\n"),
            &hash.replace('b', "c")
        ));
        assert!(!sha256sum_says("", hash));
    }

    #[test]
    fn a_real_hash_is_checked_by_certutil() {
        if !cfg!(windows) {
            return;
        }
        let path = std::env::temp_dir().join("bettercut-certutil-test.bin");
        std::fs::write(&path, b"abc").unwrap();
        let output = std::process::Command::new("certutil")
            .arg("-hashfile")
            .arg(&path)
            .arg("SHA256")
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        // SHA-256 of "abc".
        assert!(certutil_says(
            &String::from_utf8_lossy(&output.stdout),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
    }

    #[test]
    fn once_a_day() {
        assert!(due(0, EVERY_SECONDS));
        assert!(!due(1_000, 1_000 + EVERY_SECONDS - 1));
        assert!(due(1_000, 1_000 + EVERY_SECONDS));
        assert!(!due(5_000, 1_000), "a clock gone backwards is not a day");
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;

    /// Wait for the download thread to settle on an answer.
    fn settled(state: &Downloading) -> Download {
        for _ in 0..200 {
            if let Some(done) = state.lock().unwrap().clone()
                && !matches!(done, Download::Fetching { .. })
            {
                return done;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the download never finished");
    }

    fn fetch(sha256: &str, name: &str) -> Download {
        let source = std::env::temp_dir().join(format!("bettercut-update-source-{name}"));
        std::fs::write(&source, b"abc").unwrap();
        let shown = source.display().to_string().replace('\\', "/");
        let url = format!(
            "file://{}{shown}",
            if shown.starts_with('/') { "" } else { "/" }
        );
        let state = Downloading::default();
        download(
            Installer {
                name: name.to_owned(),
                url,
                sha256: sha256.to_owned(),
                size: 3,
            },
            state.clone(),
            || {},
        );
        let done = settled(&state);
        let _ = std::fs::remove_file(&source);
        done
    }

    /// Fetched from a file rather than GitHub, so the test needs no network:
    /// what is checked is the fetch, the hash and what the bar is told.
    #[test]
    fn a_download_is_ready_only_when_its_hash_matches() {
        // Checked with certutil on Windows and sha256sum on Linux; a Mac
        // does not update itself.
        if installer_suffix().is_none() {
            return;
        }
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let Download::Ready(path) = fetch(abc, "bettercut-test-ok-setup.exe") else {
            panic!("a matching download was not ready");
        };
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        let _ = std::fs::remove_file(&path);

        let wrong = abc.replace('a', "b");
        let name = "bettercut-test-bad-setup.exe";
        assert!(matches!(fetch(&wrong, name), Download::Failed(_)));
        assert!(
            !std::env::temp_dir()
                .join("bettercut-update")
                .join(format!(".{name}.download"))
                .exists(),
            "a file that failed its check was left to run"
        );
    }
}
