//! On a Mac, the app finds FFmpeg's libraries inside its own bundle
//! (`bettercut.app/Contents/Frameworks`, see docs/package-macos.sh): the
//! libraries name themselves `@rpath/...`, and this is where `@rpath` points.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
    }
}
