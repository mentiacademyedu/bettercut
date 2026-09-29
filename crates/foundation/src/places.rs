//! Where bettercut keeps things for each person, in the folders each platform
//! expects: `%APPDATA%` and `%LOCALAPPDATA%` on Windows, `~/Library` on a Mac,
//! and the XDG folders on Linux.
//!
//! Two kinds of place. [`data_home`] is what a person would miss: settings,
//! their templates, crash reports and logs. [`local_home`] is what the program
//! makes and could make again — the proxy cache, bounces, renders — plus the
//! fonts imported for this machine. On Windows these are exactly the folders
//! bettercut has always used, so nothing already there is left behind.

use std::ffi::OsString;
use std::path::PathBuf;

/// Settings, templates, crash reports and logs: `…/bettercut`.
pub fn data_home() -> PathBuf {
    resolve(Platform::current(), Kind::Data, &|name| {
        std::env::var_os(name)
    })
}

/// Caches, bounces, renders, voice-overs and imported fonts: `…/bettercut`.
pub fn local_home() -> PathBuf {
    resolve(Platform::current(), Kind::Local, &|name| {
        std::env::var_os(name)
    })
}

/// The proxy and thumbnail cache: `…/bettercut/cache`.
pub fn cache_home() -> PathBuf {
    resolve(Platform::current(), Kind::Cache, &|name| {
        std::env::var_os(name)
    })
}

/// Which convention to follow. A parameter rather than a `cfg!` inside the
/// function, so every platform's answer can be tested on any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    /// Linux and the other Unixes: the XDG base directories.
    Unix,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Unix
        }
    }
}

/// Which of the three places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Data,
    Local,
    Cache,
}

/// The folder for `kind` on `platform`, reading environment variables through
/// `var`. Falls back to the system's temporary folder when nothing is set,
/// rather than failing: a session that cannot keep settings can still edit.
pub fn resolve(platform: Platform, kind: Kind, var: &dyn Fn(&str) -> Option<OsString>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = set("HOME");
    let base = match (platform, kind) {
        (Platform::Windows, Kind::Data) => set("APPDATA"),
        (Platform::Windows, Kind::Local | Kind::Cache) => set("LOCALAPPDATA"),
        (Platform::Mac, Kind::Data | Kind::Local) => {
            home.map(|h| h.join("Library").join("Application Support"))
        }
        (Platform::Mac, Kind::Cache) => home.map(|h| h.join("Library").join("Caches")),
        (Platform::Unix, Kind::Data | Kind::Local) => {
            set("XDG_DATA_HOME").or_else(|| home.map(|h| h.join(".local").join("share")))
        }
        (Platform::Unix, Kind::Cache) => {
            set("XDG_CACHE_HOME").or_else(|| home.map(|h| h.join(".cache")))
        }
    };
    let root = base.unwrap_or_else(std::env::temp_dir).join("bettercut");
    match kind {
        Kind::Cache => root.join("cache"),
        Kind::Data | Kind::Local => root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    /// Windows keeps the folders bettercut has always used.
    #[test]
    fn windows_uses_appdata_and_localappdata() {
        let vars = env(&[
            ("APPDATA", r"C:\U\Roaming"),
            ("LOCALAPPDATA", r"C:\U\Local"),
        ]);
        assert_eq!(
            resolve(Platform::Windows, Kind::Data, &vars),
            Path::new(r"C:\U\Roaming").join("bettercut")
        );
        assert_eq!(
            resolve(Platform::Windows, Kind::Local, &vars),
            Path::new(r"C:\U\Local").join("bettercut")
        );
        assert_eq!(
            resolve(Platform::Windows, Kind::Cache, &vars),
            Path::new(r"C:\U\Local").join("bettercut").join("cache")
        );
    }

    /// A Mac keeps things in ~/Library, caches apart.
    #[test]
    fn a_mac_uses_library() {
        let vars = env(&[("HOME", "/Users/ana")]);
        let support = Path::new("/Users/ana/Library/Application Support/bettercut");
        assert_eq!(resolve(Platform::Mac, Kind::Data, &vars), support);
        assert_eq!(resolve(Platform::Mac, Kind::Local, &vars), support);
        assert_eq!(
            resolve(Platform::Mac, Kind::Cache, &vars),
            Path::new("/Users/ana/Library/Caches/bettercut/cache")
        );
    }

    /// Linux follows XDG, with its documented defaults when unset.
    #[test]
    fn linux_follows_xdg() {
        let set = env(&[
            ("HOME", "/home/ana"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_CACHE_HOME", "/c"),
        ]);
        assert_eq!(
            resolve(Platform::Unix, Kind::Data, &set),
            Path::new("/d/bettercut")
        );
        assert_eq!(
            resolve(Platform::Unix, Kind::Cache, &set),
            Path::new("/c/bettercut/cache")
        );
        let unset = env(&[("HOME", "/home/ana"), ("XDG_DATA_HOME", "")]);
        assert_eq!(
            resolve(Platform::Unix, Kind::Data, &unset),
            Path::new("/home/ana/.local/share/bettercut")
        );
        assert_eq!(
            resolve(Platform::Unix, Kind::Cache, &unset),
            Path::new("/home/ana/.cache/bettercut/cache")
        );
    }

    /// With nothing set at all, somewhere writable rather than a failure.
    #[test]
    fn nothing_set_falls_back_to_temp() {
        let none = env(&[]);
        assert!(resolve(Platform::Mac, Kind::Data, &none).starts_with(std::env::temp_dir()));
    }
}
