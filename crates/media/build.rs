//! Copies the FFmpeg DLLs next to the built binaries.
//!
//! §88a: "Bundle FFmpeg shared libraries with the application. Do not require a
//! system install." This is the development-time half of that — without it,
//! `cargo run` and `cargo test` only work if the developer has put
//! `vendor/ffmpeg/bin` on `PATH`, which is exactly the kind of machine-specific
//! setup step that goes stale.
//!
//! Windows resolves DLLs from the executable's own directory first, so copying
//! them there is enough. Packaging for release does the same thing into the
//! installer payload.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=BETTERCUT_FFMPEG_BIN");

    let Ok(source) = std::env::var("BETTERCUT_FFMPEG_BIN") else {
        // Not configured — nothing to copy. The link step will fail with a
        // clearer message than anything this script could produce.
        return;
    };
    let source = PathBuf::from(source);
    if !source.is_dir() {
        // Fail here rather than warn.
        //
        // Warning and continuing produced the single worst first-run experience
        // this project has: the build carried on for another minute and then
        // died with `could not find native static library avcodec`, which names
        // neither FFmpeg nor the missing step, while the useful warning had
        // already scrolled off the screen. Nothing can link without the SDK, so
        // there is no build worth continuing.
        // ASCII only: this is printed by a Windows console that may still be on
        // a legacy code page, where box-drawing characters come out as noise.
        panic!(
            "\n\n\
             =============================================================\n\
             FFmpeg SDK not found.\n\
             \n\
             Expected it at:\n\
               {}\n\
             \n\
             It is about 250 MB and is deliberately not committed, so a\n\
             fresh clone has to fetch it once. From the repository root:\n\
             \n\
               setup.cmd\n\
             \n\
             or directly:\n\
             \n\
               powershell -ExecutionPolicy Bypass -File docs\\fetch-ffmpeg.ps1\n\
             \n\
             Run `setup.cmd check` to test every prerequisite at once.\n\
             =============================================================\n",
            source.display()
        );
    }

    let Some(target_dir) = target_dir() else {
        return;
    };

    // Both the profile root (`cargo run`) and `deps/` (`cargo test`).
    let destinations = [target_dir.clone(), target_dir.join("deps")];

    let Ok(entries) = std::fs::read_dir(&source) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_shared_lib = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "dll" | "so" | "dylib"));
        if !is_shared_lib {
            continue;
        }

        for destination in &destinations {
            if std::fs::create_dir_all(destination).is_err() {
                continue;
            }
            let Some(name) = path.file_name() else {
                continue;
            };
            let to = destination.join(name);

            // Skip if already current: copying 60 MB of DLLs on every build is
            // a needless second of I/O.
            if is_up_to_date(&path, &to) {
                continue;
            }
            // A failure here is not fatal — the DLL may be locked by a running
            // instance, in which case the existing copy is almost certainly the
            // one we were about to write.
            let _ = std::fs::copy(&path, &to);
        }
    }
}

/// Walk up from `OUT_DIR` to the profile directory (`target/debug`).
///
/// `OUT_DIR` is `target/<profile>/build/<pkg>-<hash>/out`, so the profile root
/// is three levels up. Cargo exposes no direct variable for it.
fn target_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}

fn is_up_to_date(from: &Path, to: &Path) -> bool {
    let (Ok(from), Ok(to)) = (std::fs::metadata(from), std::fs::metadata(to)) else {
        return false;
    };
    if from.len() != to.len() {
        return false;
    }
    match (from.modified(), to.modified()) {
        (Ok(a), Ok(b)) => b >= a,
        _ => false,
    }
}
