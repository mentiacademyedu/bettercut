# FFmpeg dependency

§88a: *"Pin FFmpeg version and build flags in a script under `docs/`. 'Which
FFmpeg build was this?' must be answerable from the repository, not from a
developer's memory."*

Run [`fetch-ffmpeg.ps1`](fetch-ffmpeg.ps1) to install it into `vendor/ffmpeg`.
That directory is gitignored; `.cargo/config.toml` points the build at it.

---

## Pinned build

```text
Version:   n8.1.2-34-g9b6c8969e0-20260809
Source:    BtbN/FFmpeg-Builds, release tag `autobuild-2026-08-09-13-03`
Asset:     ffmpeg-n8.1.2-34-g9b6c8969e0-win64-lgpl-shared-8.1.zip
SHA-256:   2936E5449886641B4279CA3FC554B678C8E9A2D20DD0C0A34FE7208B254A0905
Licence:   LGPL v3 (--enable-version3, GPL components disabled)
Linking:   shared (--enable-shared --disable-static)
Binding:   rusty_ffmpeg 0.17, prebuilt binding (no libclang needed) — ADR 002
ABI:       avcodec-62 · avformat-62 · avutil-60 · swscale-9 · swresample-6
```

> **Why a dated tag and not `latest`.** This was originally pinned against the
> rolling `latest` tag, and the hash went stale within a day — upstream rebuilds
> nightly and replaces the asset in place, so every fresh clone failed the
> integrity check with a bare exception, which reads as "the script is broken".
> `autobuild-*` tags are immutable, so this hash stays correct until someone
> changes it deliberately.
>
> A mismatch now means a corrupted or intercepted download, not a moved
> upstream. Retry before doing anything else, and **never** edit the hash to make
> the check pass — it is the only thing standing between the build and an FFmpeg
> carrying GPL or non-free components.

**Licence checklist re-run on 2026-08-09** against this build's configure line.
The relevant flags, verified from `ffmpeg -version` output rather than from the
asset's name:

```text
--enable-version3        LGPL v3
--disable-libx264        no GPL H.264 encoder
--disable-libx265        no GPL H.265 encoder
--disable-libfdk-aac     no non-free AAC encoder
--enable-shared          dynamic linking, so the LGPL relink right is preserved
(no --enable-gpl, no --enable-nonfree)
```

## Upgrading

1. Pick a newer `autobuild-*` tag from BtbN/FFmpeg-Builds and its
   `*-win64-lgpl-shared-*.zip` asset.
2. Download it and run `ffmpeg -version`; check the configure line against the
   checklist above.
3. Update `$Version`, `$Tag`, `$Asset` and `$Sha256` in `fetch-ffmpeg.ps1`
   together, and the block above.
4. If the ABI numbers change, update `docs/doctor.ps1`, which probes for
   `avcodec-62.dll`.
5. Re-run `cargo test --workspace`.

---

## Why this build — the §0.1 checklist

§0.1 makes encoder licensing a **blocking** decision before Milestone 6, with
three separate problems. This build addresses all three at the library level.
It does **not** discharge the patent question, which is not a software-licensing
matter and still needs legal review.

| §0.1 requirement | This build |
|---|---|
| FFmpeg core LGPL, linked dynamically | ✅ `--enable-version3`, no `--enable-gpl`, `--enable-shared --disable-static` |
| **Do not link x264** (GPL — would make the whole product GPL) | ✅ `--disable-libx264` |
| No other GPL components | ✅ `--disable-libx265 --disable-libxvid --disable-libxavs2 --disable-libdavs2 --disable-librubberband --disable-libvidstab` |
| No non-free components | ✅ `--disable-libfdk-aac`, `--disable-avisynth` |
| Prefer OS/hardware encoders | ✅ `--enable-ffnvcodec` (NVENC), `--enable-libvpl` (Quick Sync), `--enable-amf` (AMD), `--enable-vaapi` |
| Fallback: openh264 (BSD) | ✅ `--enable-libopenh264` |
| Fallback: AV1 via SVT-AV1 (BSD) | ✅ `--enable-libsvtav1` |

### Still outstanding

**Codec patents.** §0.1 is explicit that distributing an H.264/HEVC encoder in a
commercial desktop product has patent-pool implications *independent of software
licensing*. Nothing above changes that. Legal review of the encoder distribution
plan is still required before Milestone 6 begins, and ADR 007 stays unwritten
until it happens.

---

## Distribution (§88a)

The seven DLLs in `vendor/ffmpeg/bin` ship with the application:

```text
avcodec  avdevice  avfilter  avformat  avutil  swresample  swscale
```

Rules that follow from LGPL and from §88a:

* **Link dynamically.** LGPL requires that the user be able to replace the
  library. Dynamic linking satisfies this; static linking creates obligations we
  do not want.
* **Ship `LICENSE.txt`** from the FFmpeg build, plus the licence text of every
  other bundled dependency, in the installer.
* **Do not shell out to `ffmpeg.exe` in the hot path.** The binary in
  `vendor/ffmpeg/bin` is for local inspection only; the application links the
  libraries.
* **Do not require a system install.**

## Upgrading

1. Update the version, asset name, and SHA-256 above.
2. Re-run the licence checklist against the new `configure` line — read it from
   `ffmpeg -version`, do not assume it carried over.
3. Check `rsmpeg`/`rusty_ffmpeg` still target that FFmpeg major version.
4. Re-run `cargo test --workspace`.
