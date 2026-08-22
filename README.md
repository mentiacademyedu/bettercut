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
| **4 — Playback** | 🟡 Video + audio in sync, decode-ahead ring; hardware decode still open |
| **5 — Persistence** | ✅ Journal, snapshots, crash recovery, media relink |
| **6 — Export** | ✅ Export… in the toolbar, on the job pool, with progress and a stop button |
| **8 — Effects** | ✅ Transform, opacity, colour, blur, keyframes, and the effect graph |
| **7 — Proxies** | ✅ Generated on import, preferred by preview, adaptive quality recovers |

**What works today:** new/open/save projects as versioned JSON with atomic
writes; importing real video, audio, and image files with full metadata
including colour range, primaries, transfer, and matrix (§21a); and the whole
of §10's editing list —

```text
add · move (within and across tracks) · trim both edges · split at playhead
delete · ripple delete · duplicate · copy / paste · snapping
track add/remove/hide/mute/lock · zoom · scrub · undo / redo
multi-select: Ctrl+click and rubber-band box select
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

Playback reads **sequentially** — §47a.2's `Playback` mode, "never seeks". This
matters more than it sounds. A container seek only reaches the preceding
keyframe, so a frame-accurate seek costs up to a whole GOP of decodes, and §47a.1
puts that at 250 on ordinary long-GOP footage. Doing that *per displayed frame*
made playback quadratic: measured at 27 ms to read 50 frames forward against
377 ms to seek to each one, on a small 640×360 fixture. All-intra proxies (§13.1)
hide it, because there the preceding keyframe *is* the frame — which is exactly
why it survived so long, and why it appeared the moment footage played before its
proxy had finished building. A jump still seeks; the next frame along does not.

While playing, a decode thread runs half a second ahead of the playhead
(§47a.3), filling a byte-bounded ring so the frame the clock asks for is
usually already decoded. The ring pushes back rather than evicting — a full
buffer stops the decoder instead of throwing away frames it is about to need —
and a seek discards the work queued for the old position at the next frame
boundary (§47a.5). It runs only while playing: scrubbing jumps around rather
than moving forward, so there is nothing to predict.

§16's preview scaling applies **only while playing**. Reducing resolution buys
the ability to hit a frame deadline, and a paused frame has no deadline, so the
still image you are actually looking at and scrubbing through renders at full
size. Playback still starts at quarter scale and climbs as frames arrive on
time (§17).

The composited picture is cached against the playhead position, which is the
right key for scrubbing and the wrong one for everything else. For a long time
**no property change updated the preview at all** — sliders, colour, blur,
keyframes, a hidden track — because the cache stayed valid while the playhead
sat still, and `ProjectChanged` only asked egui to repaint the panels around an
unchanged frame. It surfaced when the drag handles landed: the box moved and the
video did not follow. Edits that change how a clip looks now mark the picture
stale, and playhead events deliberately do not, because the picture is already
keyed on those.

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

Media can also be taken back **out** of the project. It is refused while a clip
still uses the file — the button is disabled with the reason on hover rather
than failing after the click, because removing it anyway would leave cuts
pointing at nothing and removing the clips too would throw away an edit nobody
asked to lose. §2 holds throughout: this removes the file from the *project*,
never from the disk.

Audio clips show waveforms, analysed once in the background and cached as peaks
— 200 buckets per second, each holding the minimum and maximum it covers, which
is what produces the familiar mirrored shape. Storing the mean instead would
draw a thin line through every loud passage, because audio is symmetric around
zero. A minute costs 24 KB. The timeline reads peaks only: a repaint never opens
a media file, and a clip whose analysis has not finished draws as a plain block.

Video clips show a filmstrip: 32 frames sampled across each file, packed into
one cached sheet so a clip costs one texture rather than dozens. Tiles are drawn
at the screen position of the source time they came from, so trimming and
zooming keep them aligned with the footage instead of stretching to fit. The
strip is deliberately **coarse** — on a one-hour file the tiles are minutes
apart, so zooming in repeats a frame across a stretch of timeline. Rendering
more tiles as you zoom needs the zoom level to drive cache keys; this is the
version that works everywhere first.

The Inspector is a set of tabs — **Video**, **Colours**, **Audio**, **Speed**,
**Animation** — and it is always showing something. With nothing selected the
controls adjust the **whole video**; click a clip, on the timeline or in the
picture, and the same controls narrow to that clip. Speed is a tab with nothing
behind it yet, and says so rather than pretending.

Video holds opacity, scale, position, rotation and blur; Colours holds
brightness, contrast and saturation; Audio holds volume. Every row has a
keyframe button and its own reset, and the whole clip has one above the tabs.

A whole-video adjustment is a pass over the **composited** picture, not the same
setting applied to each clip. Those are different images: blurring two stacked
clips and then combining them is not blurring the combination. So when there is
an adjustment the layers composite into a scratch texture and one more draw
applies the master's transform, opacity, colour and blur. When there is none —
almost every frame — nothing is allocated and nothing changes, which is asserted
by test.

Colour is three multiplies in the fragment shader (§45 rates it *Cheap*), applied
in linear light: contrast pivots on 0.18 rather than 0.5, because the source is
sampled through an sRGB texture and perceptual mid-grey is 0.18 before the curve.
Pivoting at 0.5 would darken the picture every time you added contrast.

Dragging a slider lands as **one** undo step, not sixty: §11's history holds
intentions, not mouse samples, and undo returns to the value from before the drag
began.

The picture can also be framed directly. A selected clip gets a box with four
corner circles: drag inside to move it, drag a corner to scale it about its
centre. The box is derived by inverting the shader's own transform, which is the
kind of derivation that is plausible and wrong — so a test composites a real
frame through the real renderer, reads back where the picture actually landed,
and compares, across square, letterboxed and pillarboxed sources.

Two bugs there are worth recording, because both looked like features that had
never worked. The corner was captured on egui's `drag_started`, which only fires
*after* the pointer has passed the drag threshold — by then it has left the
handle, so every corner drag silently became a move. And the timeline's ruler
scrubbed whenever the mouse button was down *anywhere*, so a drag in the preview
moved the playhead as soon as the cursor crossed it. Both are now decided on the
press, and on whether the press landed on the widget at all.

**Blur** is the first effect that could not ride along in the composite pass —
it reads a neighbourhood rather than one texel — so it runs as a separable
Gaussian: two 1D passes instead of one 2D kernel, which at the export tier is 162
samples per pixel rather than 6561. Two decisions are worth knowing about:

- The stored amount is a **fraction of frame height, not a pixel radius.** §46
  has the preview reading a 720p proxy while export reads the 1080p original; a
  radius in pixels would mean the preview showed two-thirds of the blur that
  actually got encoded, and you would only find out after the export finished.
- The kernel is normalised by **the weights it actually used**, not by the
  Gaussian's analytic constant. Truncated and sparse kernels do not sum to one,
  and dividing by the wrong number darkens every blurred clip — more the wider
  the blur, so it reads as a vignette rather than a bug. A GPU test composites a
  white square and checks total brightness survives to within 0.5%.

§45 rates blur *Medium*, so it is the first effect where the quality tier does
anything: preview spends 16 taps a side, export 40. The tier changes **only that
number** — both cover the same radius, and a GPU test asserts the two agree. At
wide radii the kernel becomes sparse rather than narrower, which can alias on
fine detail; downsampling first is the fix, and is worth doing when export lands.

**Keyframes** (§24) make any of those parameters move over time. Every control
in the Inspector has a diamond next to it: click it once and the parameter
starts animating from wherever it is, with a key at the playhead. Move the
playhead, drag the slider, and you get a second key — the same slider, no mode
to be in. A filled diamond means there is a key on this exact frame and clicking
removes it. Keys show as marks along the bottom of the clip, and ◀ ▶ jump
between them.

Three decisions carry most of the weight:

- **Keys are anchored to the source media, not to the timeline.** Everything
  else about a clip already moves — dragging it, trimming either edge,
  splitting, cut and paste — and timeline-anchored keys would have to be
  rewritten by every one of those operations, with every undo putting them back.
  A fade that drifts off its shot after an unrelated ripple delete is easy to
  write and hard to notice. Anchored to the source, none of those operations has
  to do anything, and a test moves a clip 90 000 ticks to prove the animation
  stays on the picture.
- **The limits live on the parameter, not at the point of editing.** There are
  now two ways to set a value — a slider and an interpolated curve — and §24's
  Bézier is allowed to overshoot on purpose, because that is how you get a
  bounce. Both go through the same clamp, so an animated opacity cannot reach
  1.4 where the slider stops at 1.0.
- **One evaluator for every curve.** Linear, the four named easings and a custom
  Bézier are all cubic Béziers over the unit square, using the same control
  points CSS does. The curve parameter is *not* the horizontal position, so
  reading the curve at `t = x` gives an easing that is subtly wrong everywhere
  except its endpoints; solving for x is what the tests check, by asserting that
  ease-in lags the linear midpoint and ease-out leads it.

Animating a parameter takes its slider away from the static value — the
renderer stops reading it — so editing one with the playhead off the clip is
refused with a message rather than silently changing a number nothing reads.

A control's own reset clears its keys along with its value, in one undo step.
Removing *all* keyframes, from the Animation tab, deliberately does not touch
values: each control simply goes back to reading its own. That distinction was
a bug first — one button did both behind a label that mentioned only keyframes,
so a graded clip lost its grade to something that said nothing about colour.

Behind both of those is the **effect graph** (§20): effects are nodes in a
per-layer chain, not fixed stages in the compositor. Adding one is writing an
`EffectNode` — reserve per-frame resources, record your passes, hand back what
you wrote — rather than editing the compositor to make room for it. Blur is the
first and currently only node.

Opacity, colour and the transform are deliberately *not* nodes. They are
per-texel functions evaluated during the composite draw that was going to happen
anyway, so making them nodes would buy a full-frame pass and a full-frame
texture each, for arithmetic that costs nothing where it is. "Effects are nodes"
is about the ones that need a pass.

The refactor introduced a hazard worth naming, because it is the kind that only
shows up on a stacked composite: nodes write per-pass uniforms, and
`write_buffer` stages its writes until the submit, so two layers sharing one
slot would both render with whichever wrote *last*. A single-layer preview looks
perfect. The graph tests composite two layers blurred by amounts five times
apart and check the two halves differ — and both layers have to be blurred for
the test to bite, since a layer at zero declines before it writes anything. I
verified that by reintroducing the bug and watching the test fail.

**Golden-frame tests** (§51.1) are what keep §46 honest — one render graph, two
configurations — and §51.1 calls a failure a release blocker. Two checks, because
there are two different questions:

- *Does export match preview?* Render the same layers under both configs and
  compare per pixel. Driver-independent, since both sides run on whatever GPU is
  present. The tiers genuinely differ — blur spends 16 taps a side under preview
  and 40 under export — so this is not comparing a thing to itself.
- *Did the picture change at all?* The check above cannot answer that: a
  regression hitting both configs equally keeps it green. So each case also has
  a stored signature under `crates/renderer/tests/golden/`.

The signatures are not reference PNGs. A byte-exact image would need
regenerating per GPU — an NVIDIA card and an Intel iGPU disagree in the last bit
of 8-bit rounding — and a reference that fails for reasons that aren't
regressions is a reference that gets deleted. Each is a 4×4 grid holding, per
cell, mean linear RGB and the sharpest step between neighbouring texels: where
the light is, and how crisp it is.

That second term took three attempts, and the reason is the interesting part.
Blur preserves total brightness by design, so a mean-only signature reads a
blurred frame and a sharp one as *the same picture*. Standard deviation didn't
fix it either — over a 64-texel cell it's dominated by the fixture's gradient,
which a small blur barely touches. Nor did mean neighbour difference, which
divides a 64-texel edge away across 4096 smooth ones. The sharpest step works:
1.0 at a hard edge, 0.14 once blurred. A test asserting the signatures can tell
the cases apart is what caught each of those, rather than my noticing.

I verified the pair is complementary by changing the contrast pivot from 0.18 to
0.5 — the exact mistake described above. The signature check failed; the
preview/export check stayed green, because the bug hits both equally.

Dragging on empty timeline space draws a rubber band and selects every clip it
covers; Ctrl adds to the selection instead of replacing it. Selection is
resolved in time and track space rather than against screen rectangles, so a
clip scrolled past the left edge is still selected when the band covers its
span. Dragging the ruler still scrubs, dragging a clip still moves it, and a
press that never moves is still an ordinary click.

**Export** is in the toolbar: format, size, frame rate, bitrate, and where the
file goes, all defaulting to the sequence. the job runs on the same scheduler as proxies and thumbnails, the status
bar shows a bar and a Stop button, and the finished message names the encoder
that ran. §74 is blunt that FFmpeg must never block the UI, and an export is the
longest FFmpeg run the program does.

The job takes a **snapshot** of the project rather than borrowing it. Editing
during an export is the obvious thing to do with the minutes it takes, and a job
reading live state would render half its frames from before an edit and half
from after. A project is references and numbers, never media, so the copy is
nearly free.

The path underneath is tested against real footage: a project goes in, an
H.264/AAC file comes out, and tests decode it back to check the frame count, the
duration, and that the *edit* is in it rather than a copy of the source. A clip
at quarter opacity exports darker; a keyframed fade exports as a fade. Cancelling
deletes the partial file — something that looks finished is worse than nothing.

There is no export renderer. §46 says there is exactly one render graph and that
preview and export are configurations of it, so export builds the same
`Compositor` with `RenderConfig::export_to_texture` and feeds it from the same
`layer_requests` the preview calls — one function that decides which clips are
visible, where in their source they are reading, and what they look like there.
What actually differs is §46's four axes: resolution, original media instead of
proxies (§14), the full effect tier, and where the pixels go.

Export walks the timeline forward, which is §47a.2's `Playback` mode — never
seek, read the next frame. An export is the longest forward walk the program
ever does, so the per-frame seek that made playback quadratic would have cost
the most here.

§0.1 requires OS-provided encoders and forbids linking anything GPL. So the
encoder is chosen by **opening** it, not by looking up its name: a stock Windows
FFmpeg contains `h264_nvenc` on a machine with an AMD card and `h264_qsv` on one
with no Intel graphics, and both are *found*. Candidates are tried in §0.1's
order — NVENC, Quick Sync, AMF, Media Foundation, then openh264 — each opened at
the real resolution and frame rate, and the first that survives is used. On this
machine that is NVENC; Quick Sync is rejected for wanting `nv12`, and AMF for a
missing driver DLL. Nothing GPL is a candidate, and a test asserts it.

Choosing an OS encoder also means we distribute no encoder at all, which keeps
§0.1's patent-pool question away from the binary. `libopenh264` is the fallback
for machines with no hardware encoder — BSD, and already linked because proxies
use it.

One finding worth recording. The first end-to-end test wrote 48 frames and read
47 back — on NVENC only, while openh264 returned all 48. B-frames are coded
after the frames they reference, so the first packet's decode timestamp is
*negative*, and MP4 cannot store that without an edit list; the muxer resolves
it by dropping the packet. Disabling B-frames fixes it, costs little at these
bitrates, and makes all four encoders behave alike. The export tests run against
each encoder the machine has.

### Transitions

Two of them, crossfade and fade through black (§25), attached to the **outgoing
clip** rather than to the track. That placement is the whole design: move, trim,
split, cut and paste all carry the clip, and a transition stored beside the
timeline would have to be rewritten by every one of those operations. It is the
same argument §24 makes for anchoring keyframes to the source.

The timeline still does not overlap. §8's non-overlapping, sorted tracks are
what make the visible-range query a binary search, and a dissolve is not worth
spending that on. Clips stay where the user put them; the transition is a window
*centred on the cut* during which `layer_requests` returns **two** layers instead
of one, the incoming clip at a partial opacity. The compositor needed no changes
at all — §22's alpha-over already draws one picture over another.

The interesting part is handles. A crossfade shows both clips at once, so before
the cut the incoming clip must supply frames from *before* its in-point, and
after it the outgoing clip must keep reading *past* its out-point. A clip
trimmed to the edge of its file has no such material, and a transition placed
anyway would render as a black flash. So the model works out what is actually
there and clamps the length to it — in the command, not in the interface, because
§38.2 replays commands after a crash and a check that only runs on the way in
comes back unchecked on the way out. A fade through black needs no handles at
all: each clip fades within its own range, so it works on any cut, including one
against the very start or end of a file. The inspector offers whichever kinds
the cut can support and says why the other is unavailable.

### Text

§26.1 is a one-line rule with a large consequence:

> Text must be shaped and rasterized by the `text/` crate into a GPU texture,
> used identically by preview and export.

If the preview drew text with egui and the export drew it with something else,
the two would never match — different hinting, different subpixel positioning,
different fallback fonts — and §46 would break in the place users notice first.
So there is one rasterizer (`crates/text`, built on cosmic-text), it produces a
plain RGBA bitmap, and both configurations upload that same bitmap. A test
rasterizes the same title through two independently constructed renderers and
compares the bytes.

The bitmap is produced from a coverage mask rather than by drawing the glyphs
three times, which is what lets the outline, the shadow and the fill all derive
from the *same* shaped glyphs. The outline uses a distance transform: maxing
over a disc costs O(radius²) per pixel and a 16-pixel outline on a title-sized
bitmap runs into hundreds of millions of operations, which the preview would pay
on every keystroke. Two sweeps cost the same whatever the radius, and the
fractional distance gives the outline a soft edge for free.

A title is an ordinary clip on an ordinary track. `TextClip` implements the same
`Clip` trait as video and audio, so it lives in the same `Track<C>` and inherits
the sorted, non-overlapping invariant along with move, trim, split and ripple
delete — all already written and already tested. What it does not have is media:
its source range starts at zero and runs as long as the clip does, which is what
"how far into this clip are we" means for a generated source, and what §24's
keyframes will anchor to.

One thing was worth getting wrong first. Every layer is *fitted* to the canvas —
a 640×360 frame fills a 1920×1080 one — which is right for footage and wrong for
a title: text sizes are in sequence pixels, so a bitmap 400 pixels wide must
cover 400/1920 of the canvas whatever else it says. Fitted instead, a longer
sentence comes out *smaller*, which is the opposite of a size control. The fix
undoes the fit for generated layers rather than adding a mode to the shader, and
GPU tests composite real titles and count pixels to check it.

**What does not work yet:** speed changes, which have a tab that says so. Text
uses the machine's fonts rather than bundled ones — §26 wants
`assets/fonts/` for templates (Milestone 11), and until there is a font picker a
single bundled family would be the *only* family on offer. Audio does not
crossfade: mixing two sources is a different mechanism from blending two
pictures, and §25 v1 is picture only.

Some limits worth knowing before testing with your own footage:

* **Decode-ahead is unmeasured on the target machine.** It works — playback
  takes frames from the ring rather than decoding inline, asserted by test —
  but the half-second budget and the one-thread choice were sized by
  calculation, not measured on §52.1 hardware.
* **Software decode only.** §5's hardware-decode-to-texture path is still open;
  this is the RAM fallback §5 requires to exist.
* **HDR is tone-mapped in the proxy, not at upload.** PQ and HLG sources are
  probed, tagged, and converted into the SDR working space (§21a.1) by the proxy
  encoder — a real conversion, not a retag, asserted by a test that measures mean
  luma. But the direct-decode path has no equivalent, so an HDR file looks dark
  and flat for as long as it takes its proxy to build. SDR is pixel-exact
  throughout.
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

**Preview and export are one render graph** (§46). Export is not built yet, but
no code here assumes otherwise — a second renderer implementation is on §74's
prohibited list because divergence is a certainty, not a risk. Blur is the first
effect where the two configurations could have drifted, so it is also the first
one a test pins: the preview and export tiers must render the same picture.

Two dev-dependencies were added for that, both already in the tree via wgpu, so
neither adds anything to a build (§74 asks for justification, not abstinence):
`naga` parses and type-checks the WGSL during `cargo test`, which previously
happened only when a real device was created — a shader typo used to survive the
entire suite and surface as a panic on the first frame drawn. `pollster` drives
wgpu's async setup from a synchronous test. The GPU tests skip themselves when no
adapter is available, since §52.1's target and a headless CI box may have none.

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
