# bettercut

A lightweight, high-performance desktop video editor. CapCut-style convenience,
built to stay fast on 8 GB RAM, integrated graphics, and an older 4-core CPU.

Built to `development_guide.md`. Section references throughout the code (`§9`,
`§53`, …) point at that document.

---

## Status

| Milestone | State |
|---|---|
| **0 — Frame transport spike** | ✅ Done — [ADR 005](docs/adr/005-ui-shell.md) |
| **1 — Application skeleton** | ✅ Done — reference machine recorded (§52.1) |
| **2 — Media import** | ✅ Probing, library, poster thumbnails, audio waveforms |
| **3 — Timeline editing** | ✅ Done — every operation in §10 |
| **4 — Playback** | 🟡 Video + audio play in sync; decode-ahead and hardware decode still open |
| **5 — Persistence** | ✅ Journal, snapshots, crash recovery, media relink |
| 6 — Export | ⛔ Blocked on §0.1 legal review |
| **7 — Proxies** | ✅ Generated on import, preferred by preview, adaptive quality recovers |

**What works today:** new/open/save projects as versioned JSON with atomic
writes; importing real video, audio, and image files with full metadata
including colour range, primaries, transfer, and matrix (§21a); and the whole
of §10's editing list —

```text
add · move (within and across tracks) · trim both edges · split at playhead
delete · ripple delete · duplicate · copy / paste · snapping
track add/remove/hide/mute/lock · zoom · scrub · multi-select · undo / redo
```

Clips are dragged and trimmed directly on the canvas, with snapping to clip
edges and the playhead (hold Alt to bypass, `N` to toggle). Every drag commits
as exactly one undo step.

Right-clicking the timeline opens a context menu, and what it offers depends on
what is under the pointer: a clip gets split/cut/copy/duplicate/delete/ripple
delete, a track header gets hide-or-mute, lock, add and remove, and empty canvas
gets paste and playhead moves. Every entry calls the same function as its
keyboard shortcut and shows that shortcut beside it.

Playback works: the preview shows composited video, audio plays through the
device, and the **audio device is the master clock** (§20a.1) — the picture
follows it, never a wall-clock timer. Transport controls sit directly above the
timeline — start, ±10 s, play/pause, end — with the playhead and sequence
duration beside them.

§16's preview scaling applies **only while playing**. Reducing resolution buys
the ability to hit a frame deadline, and a paused frame has no deadline, so the
still image you are actually looking at and scrubbing through renders at full
size. Playback still starts at quarter scale and climbs as frames arrive on
time (§17).

The sequence has a real format. Resolution and frame rate are set from the
Inspector — 16:9, 9:16, 1:1 and 4K presets, and the nine frame rates §9's
timebase divides exactly (23.976 through 120, NTSC rates included). Importing
the first video into an empty sequence adopts its format automatically, so 25
or 50 fps footage does not land on a 30 fps grid and judder. Changing the rate
later **never moves a clip**: positions are absolute ticks, not frame numbers,
so existing cuts keep their exact times rather than being silently re-snapped.

Heavy footage gets a proxy. On import, anything §13 calls demanding — 4K or
larger, HEVC/AV1/VP9, 10-bit, above 60 fps, or needing colour normalization — is
re-encoded in the background to a small **all-intra** copy (§13.1), which is what
makes scrubbing one decode per seek instead of up to 250. The preview switches to
it the moment it lands and falls back to the original if it is missing. Originals
are never touched, and export will always use them. The Inspector's **Proxies**
section turns this off or changes the quality; the status bar shows progress
while it runs.

Moved media can be found again (§66). Files that have gone are marked on open,
and the browser offers **Locate…** for one file or **Locate folder…** for all of
them — matching by name and confirming by size, so a different file that happens
to share a name is left alone rather than swapped in under existing cuts.
Relinking changes only the location: duration, resolution and colour stay as
imported, because relinking is meant to repair a project, not redefine what its
clips contain. A whole folder undoes in one step.

Work survives a crash. Every command is appended to a journal as it executes
(§38.2), so a process that dies loses at most the one edit in flight rather than
everything since the last save. On the next launch the snapshot plus the journal
are replayed and the result is *offered* — §39.5 is explicit that recovery must
never overwrite the user's file by itself, and it doesn't.

The media browser shows poster thumbnails, decoded a tenth of the way into each
file — frame zero is so often black, a slate, or a fade that it makes a useless
picture. They are generated in the background on the same job pool as proxies
(§15's concurrency limit is one budget for the machine, not one per feature) and
cached as raw RGBA, which costs ~57 KB each and avoids pulling in an image codec
just to decode back to the bytes we started with.

Audio clips show waveforms, analysed once in the background and cached as peaks
— 200 buckets per second, each holding the minimum and maximum it covers, which
is what produces the familiar mirrored shape. Storing the mean instead would
draw a thin line through every loud passage, because audio is symmetric around
zero. A minute costs 24 KB. The timeline reads peaks only: a repaint never opens
a media file, and a clip whose analysis has not finished draws as a plain block.

**What does not work yet:** export, effects, and text. Multi-selection is
Ctrl+click only — there is no rubber-band box select, and no filmstrip on
timeline clips yet.

Two playback limits worth knowing before testing with your own footage:

* **No decode-ahead ring buffer yet** (§47a.3). Frames are decoded on demand and
  cached. §13.1's all-intra proxies now carry this — one decode per seek instead
  of up to 250 — so editing is smooth, but scrubbing the *original* 1080p
  long-GOP media (before its proxy finishes, or with proxies switched off) will
  still stutter.
* **Software decode only.** §5's hardware-decode-to-texture path is still open;
  this is the RAM fallback §5 requires to exist.
* **HDR is detected but not tone-mapped.** PQ and HLG sources are probed and
  correctly tagged, and they trigger a proxy — but nothing converts them into
  the SDR working space (§21a.1), at upload or in the proxy encoder. HDR
  footage will look dark and flat. SDR footage is pixel-exact.
* **No custom sequence sizes in the UI.** The Inspector offers 16:9, 9:16, 1:1
  and 4K presets; an arbitrary size round-trips through the project file but
  cannot be typed in yet.

---

## Build and run

Requires Rust 1.95+. **No LLVM, cmake, or system FFmpeg install is needed** —
see [ADR 002](docs/adr/002-ffmpeg-backend.md) for why.

There is **no prebuilt binary**: nothing is published to Releases, so
`target/release/` does not exist until you build it yourself.

```powershell
# 1. One-time setup: fetches the pinned FFmpeg SDK (~250 MB, not in the repo)
#    and then checks every build prerequisite.
.\setup.cmd

# 2. Build and run.
cargo run -p bettercut-desktop              # debug, empty project
cargo run -p bettercut-desktop -- x.vproj   # open a project file

cargo build --release -p bettercut-desktop  # -> target\release\bettercut.exe
```

In PowerShell the leading `.\` is required — PowerShell does not run programs
from the current directory without it, and bare `setup.cmd` fails with *"is not
recognized"*. `.\setup.cmd check` re-runs the checks without downloading.

**Skipping step 1 is the most common failure.** The build now stops early and
says so; before that it ran on for a minute and died with an error naming
FFmpeg only in passing:

```text
error: could not find native static library `avcodec`, perhaps an -L flag is missing?
error: could not compile `rusty_ffmpeg` (lib) due to 1 previous error
```

If anything goes wrong, `.\setup.cmd check` reports every prerequisite and names
the one that is missing. Everything runs under **Windows PowerShell 5.1**, the
version that ships with Windows — `pwsh` (PowerShell 7) is a separate install
and is not assumed.

Checks:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

The FFmpeg DLLs are copied next to the built binaries automatically, so nothing
needs to be on `PATH`.

The fetch script is the only setup step: it downloads the pinned LGPL build,
checks its SHA-256, and refuses builds carrying `--enable-gpl`, `--enable-libx264`,
`--enable-libx265`, or `--enable-nonfree` (§0.1). The SDK itself is not in the
repository; `vendor/ffmpeg-binding.rs` is, deliberately — see [ADR 002](docs/adr/002-ffmpeg-backend.md).

**Windows only for now.** The fetch script is PowerShell and pulls a win64
build, and the committed binding was generated for that target. Nothing in the
code is Windows-specific except `crates/jobs`' thread-priority call, which
already has a Unix branch.

---

## Layout

```text
apps/desktop/          eframe entry point: logging, window, event pump
crates/
  foundation/          exact time (§9), rational frame rates, IDs
  timeline/            sequences, tracks, clips, range queries (§8, §53)
  media/               FFmpeg: probe, decode, proxy encode (§3, §12, §13.1, §21a)
  audio/               master clock, ring buffer, mix graph (§20a)
  renderer/            wgpu compositor, one graph two configs (§21, §46)
  playback/            frame cache, A/V sync, proxy jobs (§18, §47a)
  jobs/                bounded worker pool, priorities, cancellation (§15, §69)
  cache/               on-disk proxies/thumbnails, LRU limit (§19, §67)
  project-format/      Project + .vproj versioned JSON, atomic save (§37, §38.1)
  editor-core/         commands, undo/redo, journal, recovery (§38.2, §39, §54–§56)
  ui/                  egui panels and the timeline canvas (§53, §58)
docs/adr/              architecture decisions
spike/frame-transport/ Milestone 0 spike — throwaway, excluded from the workspace
vendor/ffmpeg/         pinned FFmpeg SDK (~250 MB, not committed — fetch it)
vendor/ffmpeg-binding.rs   generated FFI, committed on purpose (see ADR 002)
```

Dependencies point one way only (§86). The `timeline` crate does not know egui
exists; `media` will be the only crate that ever references FFmpeg.

`crates/foundation` is not in §7's list — see
[its README](crates/foundation/README.md) for why it exists.

---

## The rules that shape this code

Four constraints from the guide account for most of the design. They are cheap to
honour now and expensive to retrofit:

**Time is integer ticks, never floats** (§9, §74). 960,000 per second, because it
divides exactly by every NTSC rate. See
[ADR 003](docs/adr/003-project-timebase.md).

**The UI never mutates the project** (§54). `Editor` exposes `project()` and no
`project_mut()`. Every change is a command, which is what makes undo, automation
(§32), and templates (§77) share one mechanism.

**The timeline is a canvas, not a widget tree** (§53). One `Painter`, one rect,
and only the clips intersecting the viewport get drawn.

**Preview and export are one render graph** (§46). Not built yet, but no code
here assumes otherwise — a second renderer implementation is on §74's prohibited
list because divergence is a certainty, not a risk.

---

## Performance targets

Every number in §81 is judged on the §52.1 **reference machine**:

```text
Intel Core i5-8250U (4C/8T) · Intel UHD Graphics 620 · 8 GB · 256 GB SATA SSD
```

A 2019 office laptop — the machine this product exists to serve.

**No such machine is currently available to benchmark on at the moment**, so §81's numbers are
design targets, not measurements. Nothing here should be described as "meets
§81" until it has been measured there; §52.1 is explicit that benchmarks from a
development machine do not count, and this one (Ryzen 5 5600X / RTX 4060) has no
integrated GPU, so it cannot even approximate the target's graphics path.

The target is not inert, though: `HardwareProfile::detect()` reads the real core
count at startup and picks the §43 mode, the §15 job limit, the §15.1 FFmpeg
thread cap, and the §18 cache size from it. See
[docs/reference-machine.md](docs/reference-machine.md) for the full consequences
and the benchmark runbook for when hardware turns up.

---

## Licensing

The project's own code is `MIT OR Apache-2.0` (declared in `Cargo.toml`). **The
`LICENSE-MIT` and `LICENSE-APACHE` files are not in the repository yet** — the
manifest currently promises terms the repo does not ship.

FFmpeg is linked **dynamically** against an **LGPL** build, which is what keeps
that obligation to "ship the DLLs and allow replacement" rather than requiring
this project to be GPL. §74 forbids linking x264 or any GPL component, and
`docs/fetch-ffmpeg.ps1` enforces it by refusing any build whose configure line
contains `--enable-gpl`, `--enable-libx264`, `--enable-libx265`, or
`--enable-nonfree`.

**§0.1 is unresolved and blocks Milestone 6.** Choosing an H.264/H.265 *encoder*
is a licensing decision, not a technical one, and the guide marks it as needing
legal review before export is built. Codec **patent** licensing is a separate
question from software licensing and is also open. Nothing in this repository
should be read as legal advice or as a resolution of either.
