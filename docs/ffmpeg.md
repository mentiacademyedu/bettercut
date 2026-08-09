# FFmpeg dependency

§88a: *"Pin FFmpeg version and build flags in a script under `docs/`. 'Which
FFmpeg build was this?' must be answerable from the repository, not from a
developer's memory."*

Run [`fetch-ffmpeg.ps1`](fetch-ffmpeg.ps1) to install it into `vendor/ffmpeg`.
That directory is gitignored; `.cargo/config.toml` points the build at it.

---

## Pinned build

```text
Version:   n8.1.2-34-g9b6c8969e0-20260808
Source:    BtbN/FFmpeg-Builds, release tag `latest`
Asset:     ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip
SHA-256:   96326847B2CDCE6A97C2703B1F487C3A5ED5E56C9D19180A080276B095BE95D3
Licence:   LGPL v3 (--enable-version3, GPL components disabled)
Linking:   shared (--enable-shared --disable-static)
Binding:   rsmpeg 0.18 / rusty_ffmpeg 0.17, prebuilt bindings (no libclang needed)
```

> **Note.** The upstream `latest` tag is a rolling release, so the asset behind
> that URL changes over time. The SHA-256 above is what this project was built
> and tested against. If `fetch-ffmpeg.ps1` reports a hash mismatch, that is the
> upstream build having moved — verify the new build still satisfies the licence
> checklist below before updating the hash here.

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
