# ADR 002 — FFmpeg Backend and Binding

**Status:** Accepted
**Date:** 2026-08-08
**Decides:** §3, §84, §86, §88a
**Implemented by:** `crates/media/src/ffmpeg/`, `docs/ffmpeg.md`

---

## Decision

FFmpeg is the media backend, bundled as **LGPL shared libraries** and linked
dynamically. The Rust binding is **`rusty_ffmpeg`** (raw FFI) with its
`use_prebuilt_binding` feature — **not** `rsmpeg`, which §3 recommends.

---

## Why not rsmpeg, which the guide names

§3 recommends `rsmpeg`, and it was tried first. It does not work here:

1. **`rsmpeg` 0.18 pins FFmpeg 8.0 exactly.** It fails to compile against
   FFmpeg 8.1 (`AVFormatContext` field changes) and against 7.1.
   FFmpeg 8.0 binaries are not offered by the LGPL build we use.
2. **It requires LLVM/libclang on every build machine.** `rsmpeg` 0.18 depends
   on `rusty_ffmpeg` ^0.16.7, which has no prebuilt binding and always runs
   bindgen. Its header whitelist is not version-gated, so it also demands
   `avfft.h` and `xvmc.h` — headers FFmpeg 8.1 no longer ships.
3. The version of libclang matters. bindgen 0.71 against LLVM 22 silently
   produces **opaque** structs rather than failing, so `AVFormatContext` comes
   out as `{ _address: u8 }` and every field access breaks. Pinning LLVM 19
   means an 806 MB download or an installer requiring administrator rights.

`rusty_ffmpeg` 0.17's `use_prebuilt_binding` removes all of it: a binding
generated against FFmpeg 8.1 is used as-is, and **no LLVM is needed at all**. A
contributor runs `docs/fetch-ffmpeg.ps1` and builds.

The binding is supplied by us, not by the crate: `.cargo/config.toml` sets
`FFMPEG_BINDING_PATH` to `vendor/ffmpeg-binding.rs`, which is **committed** and
excepted from `.gitignore`'s `/vendor/*` rule. Leaving the variable unset makes
the build script run bindgen and fail with *"Unable to find libclang"*, so that
one generated file is what this whole decision rests on. It is regenerated only
when the pinned FFmpeg version changes, which needs LLVM once, on one machine.

## What this costs

`rusty_ffmpeg` is raw FFI, so the media crate writes `unsafe` where `rsmpeg`
would have provided safe wrappers.

That cost is smaller than it looks, and it is contained:

* §3 already requires an internal abstraction: *"Do not allow the rest of the
  application to depend directly on a specific FFmpeg Rust wrapper."* The traits
  in `crates/media/src/decoder.rs` were written before either binding was
  chosen, so nothing outside `crates/media/src/ffmpeg/` changes.
* The workspace sets `unsafe_code = "forbid"`. **`crates/media` is the single
  documented exception**, declared in its own `Cargo.toml` so the exception is
  visible in review rather than implicit.
* Hardware decode → texture interop (§5, the hardest part of the project) needs
  raw FFI regardless. `rsmpeg`'s safe wrappers do not cover it.

Rules every `unsafe` block here follows are listed at the top of
`crates/media/src/ffmpeg/mod.rs`. The one that matters most: allocation is
paired with its free via `Drop`, not by hand, so an early return cannot leak a
demuxer handle — on Windows a leaked handle keeps the user's file locked.

## Reversing this

If `rsmpeg` gains prebuilt bindings, or pins a FFmpeg version we can obtain as
an LGPL shared build, switching back is contained to
`crates/media/src/ffmpeg/`. Nothing else in the project names an FFmpeg type
(§86), which is the whole point of the abstraction.

---

## Distribution

Covered in [`docs/ffmpeg.md`](../ffmpeg.md): the pinned version, its SHA-256,
the full configure line, and the §0.1 licence checklist it satisfies
(LGPL v3, no x264/x265/xvid, openh264 and SVT-AV1 available as BSD fallbacks,
NVENC/Quick Sync/AMF/VAAPI for the OS encoders §0.1 prefers).

`crates/media/build.rs` copies the DLLs next to the built binaries so
`cargo run` and `cargo test` need no `PATH` setup (§88a).

**§0.1 is not discharged by this ADR.** Codec patent licensing is a separate
question from software licensing and still blocks Milestone 6. ADR 007 stays
unwritten until legal review happens.
