//! On a Mac, the app finds FFmpeg's libraries inside its own bundle
//! (`bettercut.app/Contents/Frameworks`, see docs/package-macos.sh): the
//! libraries name themselves `@rpath/...`, and this is where `@rpath` points.
//!
//! Packaging rewrites where the binary looks for each library, and longer
//! paths only fit if the linker left room for them: without
//! `-headerpad_max_install_names` the Intel build's packaging failed with
//! "larger updated load commands do not fit".

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg-bins=-Wl,-headerpad_max_install_names");
    }
}
