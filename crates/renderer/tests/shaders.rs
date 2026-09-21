//! The WGSL has to compile, and nothing else in the suite proves it.
//!
//! Every other renderer test works on the Rust side of the boundary — uniform
//! packing, blur planning, configuration. The shaders themselves are only
//! handed to `create_shader_module` when a real device exists, which means a
//! typo or a type error in a `.wgsl` file survives `cargo test`, survives
//! `cargo clippy`, and appears as a panic on the first frame the user draws.
//!
//! naga is what wgpu compiles shaders with, so parsing and validating with it
//! here is the same check the driver would do, minus the GPU. That matters
//! beyond convenience: §52.1's low-end machine and the CI box may have no
//! usable adapter at all.
//!
//! This does not prove the shaders produce the *right* picture — that is
//! §51.1's golden-frame job, which needs a device. It proves they are
//! well-formed, which is the failure that would otherwise be found by hand.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use naga::valid::{Capabilities, ValidationFlags, Validator};

fn check(name: &str, source: &str) {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(module) => module,
        Err(err) => panic!("{name} does not parse:\n{}", err.emit_to_string(source)),
    };

    // The default capability set, deliberately: anything beyond it would not
    // run on §52.1's integrated hardware.
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::default());
    if let Err(err) = validator.validate(&module) {
        panic!("{name} does not validate:\n{}", err.emit_to_string(source));
    }
}

#[test]
fn composite_shader_is_valid() {
    check("composite.wgsl", include_str!("../src/composite.wgsl"));
}

#[test]
fn blur_shader_is_valid() {
    check("blur.wgsl", include_str!("../src/blur.wgsl"));
}

/// **Every** shader in the crate, found by listing the directory rather than
/// by a list someone has to remember to extend. The two named tests above were
/// the whole of this file while three more shaders were added beside them —
/// one with a reserved word for a variable name that only a GPU run caught.
#[test]
fn every_shader_in_the_crate_is_valid() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "wgsl") {
            let source = std::fs::read_to_string(&path).expect("read");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            check(&name, &source);
            checked.push(name);
        }
    }
    for expected in [
        "composite.wgsl",
        "blur.wgsl",
        "sharpen.wgsl",
        "lut.wgsl",
        "glitch.wgsl",
        "reflect.wgsl",
    ] {
        assert!(
            checked.iter().any(|n| n == expected),
            "{expected} was not found to check"
        );
    }
}

/// The uniform structs are written by hand on the Rust side, at byte offsets
/// the shaders have to agree with. naga knows the layout the GPU will use, so
/// it can be asked directly rather than reasoned about in a comment.
///
/// This is the check that would have caught a `vec3` pad silently rounding the
/// composite struct up to 80 bytes.
#[test]
fn the_uniform_structs_are_the_sizes_the_rust_side_writes() {
    let expected = [
        (
            "composite.wgsl",
            include_str!("../src/composite.wgsl"),
            "Layer",
            // 48 for the transform's three columns, 16 for opacity and the
            // colour values in its padding, the chroma key's `vec3` at 64 with
            // tolerance in its tail, the mask's numbers from 88, the white
            // balance in the tail that left at 120, §22's crop as two `vec2`s
            // from 128, the vignette and grain from 144, and the corners and
            // border and shadow from 160 to 192, and the bars at 192 — rounded
            // to the struct's 16-byte alignment — and §45's four pinned corners
            // from 208, which take it to 240.
            240,
        ),
        (
            "blur.wgsl",
            include_str!("../src/blur.wgsl"),
            "BlurParams",
            32,
        ),
    ];

    for (file, source, name, size) in expected {
        let module = naga::front::wgsl::parse_str(source).expect("parses");
        let mut validator = Validator::new(ValidationFlags::all(), Capabilities::default());
        let info = validator.validate(&module).expect("validates");
        let _ = info;

        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{file} has no struct named {name}"));

        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("{name} is not a struct");
        };
        assert_eq!(
            span, size,
            "{file}'s {name} is {span} bytes; the Rust writer packs {size}"
        );
    }
}
