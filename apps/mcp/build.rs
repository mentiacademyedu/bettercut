//! On a Mac, `bettercut-mcp` lives inside `bettercut.app` beside the app and
//! finds the same FFmpeg libraries in `Contents/Frameworks`; see
//! apps/desktop/build.rs for why the header room is needed.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg-bins=-Wl,-headerpad_max_install_names");
    }
}
