//! The interface's typeface: the operating system's own.
//!
//! egui draws with its bundled Ubuntu Light unless told otherwise, which on
//! a dense editing screen reads thin and a little foreign. Every desktop the
//! editor runs on ships a well-made interface face — Segoe UI on Windows,
//! the system face on a Mac, a common sans on Linux — so the first of those
//! found is put in front of egui's own, which stay behind it for every
//! character the system face lacks (the symbols and emoji the interface
//! uses, `tests/glyphs.rs`). Nothing is bundled and nothing downloaded; with
//! none of them found, egui's fonts are used as before.
//!
//! The file is read once at start-up and kept by egui for the life of the
//! app: one font, a few hundred kilobytes to a couple of megabytes.

use std::path::PathBuf;

/// Where each platform keeps the faces worth trying, best first. The first
/// that exists is used for regular text; a bold face beside it, when found,
/// is used for strong text.
fn candidates() -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut out = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let fonts = std::env::var_os("WINDIR")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        out.push((fonts.join("segoeui.ttf"), Some(fonts.join("segoeuisb.ttf"))));
        out.push((fonts.join("arial.ttf"), Some(fonts.join("arialbd.ttf"))));
    }
    #[cfg(target_os = "macos")]
    {
        out.push((PathBuf::from("/System/Library/Fonts/SFNS.ttf"), None));
        out.push((
            PathBuf::from("/System/Library/Fonts/Helvetica.ttc"),
            None,
        ));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (regular, bold) in [
            (
                "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
                "/usr/share/fonts/truetype/inter/Inter-SemiBold.ttf",
            ),
            (
                "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
                "/usr/share/fonts/opentype/inter/Inter-SemiBold.otf",
            ),
            (
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
            ),
            (
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
            ),
            (
                "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf",
            ),
            (
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            ),
        ] {
            out.push((PathBuf::from(regular), Some(PathBuf::from(bold))));
        }
    }
    out
}

/// The family name strong text is drawn in.
pub const STRONG: &str = "bettercut-strong";

static STRONG_BOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether [`STRONG`] has been given fonts. Asking egui to draw in a family
/// it has never heard of panics, so until [`install`] has run — in a test,
/// say — strong text is drawn in the ordinary family instead.
pub fn strong_bound() -> bool {
    STRONG_BOUND.load(std::sync::atomic::Ordering::Relaxed)
}

/// Put the system's interface face in front of egui's. Returns whether one
/// was found; the interface works the same either way.
pub fn install(ctx: &egui::Context) -> bool {
    let mut fonts = egui::FontDefinitions::default();
    let Some((regular, bold)) = candidates().into_iter().find_map(|(regular, bold)| {
        let bytes = std::fs::read(&regular).ok()?;
        let bold = bold.and_then(|b| std::fs::read(b).ok());
        Some((bytes, bold))
    }) else {
        // No system face: strong text in egui's own, under the same name.
        let ordinary = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        fonts
            .families
            .insert(egui::FontFamily::Name(STRONG.into()), ordinary);
        ctx.set_fonts(fonts);
        STRONG_BOUND.store(true, std::sync::atomic::Ordering::Relaxed);
        return false;
    };

    fonts.font_data.insert(
        "system-ui".to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(regular)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "system-ui".to_owned());

    // Strong text in a real bold where there is one, falling back through the
    // regular face to egui's for anything it lacks.
    let mut strong = Vec::new();
    if let Some(bold) = bold {
        fonts.font_data.insert(
            "system-ui-bold".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bold)),
        );
        strong.push("system-ui-bold".to_owned());
    }
    strong.extend(
        fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name(STRONG.into()), strong);

    ctx.set_fonts(fonts);
    STRONG_BOUND.store(true, std::sync::atomic::Ordering::Relaxed);
    true
}
