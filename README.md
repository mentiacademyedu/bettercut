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
| **8 — Effects** | ✅ Transform, opacity, colour, blur, keyframes and the effect graph — plus masks, green screen, blend modes, clip animations, motion blur and a background colour |
| **7 — Proxies** | ✅ Generated on import, preferred by preview, adaptive quality recovers |
| **9 — Text and transitions** | ✅ Titles with entrances and exits; all seven of §25's transitions |
| **10 — Captions** | ✅ SRT and VTT in and out, on a lane of their own |
| **11 — Templates** | ✅ Format, validator, browser, slots, and adding your own |
| **12 — Automation** | 🟡 Silence removal, beat markers, scene detection and reframing done; automatic captions open |

**What works today:** new/open/save projects as versioned JSON with atomic
writes; importing real video, audio, and image files with full metadata
including colour range, primaries, transfer, and matrix (§21a); and the whole
of §10's editing list —

```text
add · move (within and across tracks) · trim both edges · split at playhead
delete · ripple delete · duplicate · copy / paste · snapping
track add/remove/hide/mute/solo/lock · zoom · scrub · undo / redo
multi-select: Ctrl+click and rubber-band box select
nudge by frame · trim to playhead · markers · jump between cuts
```

Clips are dragged and trimmed directly on the canvas, with snapping to clip
edges and the playhead (hold Alt to bypass, `N` to toggle). Every drag commits
as exactly one undo step.

Right-clicking the timeline opens a context menu, and what it offers depends on
what is under the pointer: a clip gets split/cut/copy/duplicate/delete/ripple
delete, a track header gets hide-or-mute, solo, lock, add and remove, and empty canvas
gets paste and playhead moves. Mute and solo are also buttons on the header
itself, since they are pressed dozens of times in a cut. Every entry calls the same function as its
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

Two things about the other end of that. A clean quit discards its own recovery
data, so the next launch does not offer back work the user already has on disk;
a quit with **unsaved** changes keeps it, because there is no save prompt on the
close button yet and to the work that quit is indistinguishable from a crash.
And the abandoned sessions that accumulate — one per run, and every test that
builds an editor leaves one — are pruned on a background thread after the scan
rather than before the window opens. Deleting seven thousand of them took nine
seconds of startup on this machine; housekeeping has no business holding the
window shut.

If autosave itself stops working — a full disk, a folder that cannot be written
to, neither of which announces itself — the status bar says so in the loudest
thing on it. An autosave that has quietly stopped is worse than none, because
the user believes they are protected while they are not.

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

A handle above the box turns the clip, snapping to the nearest right angle
within three degrees, because level and upright are what a turn is usually
aiming for and a hand cannot hit 0.0° on its own.

Building that handle found a bug that had been in the renderer since the first
transform: **rotation did not work**. The matrix rotated in clip space, which
runs -1..1 on both axes whatever the frame's shape, and one of its terms had the
wrong sign — so the picture's two axes turned in opposite directions. That is a
shear, not a rotation. A clip lost area as it turned, half of it at 30°, and at
45° it vanished altogether: the matrix's determinant is `-cos 2θ`, which is zero
there. Preview and export share the compositor, so exports had it too. The one
test checked a single point at 90° on a square frame, which depends only on the
two terms that happened to be right. The new tests render a solid square and
measure it — area preserved at every angle, still square on a 16:9 frame — and
put the old matrix back to confirm they fail on it. The two golden frames that
include a rotation had recorded the sheared picture and were re-recorded; the
four without one were unchanged.

§26's titles get the same box, which took one extra step. A title is drawn at
its *natural* size rather than fitted to the canvas, so the matrix the shader
ends up with is not the clip's transform but a correction of it — and how wide
"Hello" comes out is not something the model can answer, only the rasterizer. So
the render records each layer's source size and the overlay reads it back. The
box is built from the corrected transform while the *gesture* starts from the
clip's own: seeded with the corrected scale instead, the first drag of a corner
would collapse the title to a fraction of itself.

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

Those curves went unreachable for a long time: every key a user placed was
linear, because nothing offered the others. **Keyframe Easing** on the clip menu
now sets the curve of the keys under the playhead — every parameter keyed at
that instant, not one, since a scale is two parameters and easing half of it
would let the shape drift as it moved. **Previous/Next Keyframe** sits above it,
because keys land on exact ticks and the playhead is dragged in pixels: without
a way to jump, the easing was there and unusable. The timeline draws the curve
in the shape of the mark — a diamond for a straight ramp, a circle for an eased
one, a square for a hold — so it is possible to see which keys carry it.

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

The golden set covers masks, chroma key and blend modes as well as transform,
colour, blur and stacked tracks — so §46's guarantee extends to the new effects
rather than stopping at the old ones. Each effect case is also rendered a second
time with only its own effect removed, and the two have to differ: a golden case
that renders the same either way passes forever and guards nothing, while
looking in the suite like cover for a feature.

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

They guard §46 from the layer inwards, and there is a step before that: turning
a `LayerRequest` into a renderer layer. That was written out twice, once in the
preview and once in the export, and the golden frames could not see it — both
were handed the same layers to compare. Removing the mask from the export's copy
alone passed every test in the workspace, which is a silent version of exactly
the bug §46 exists to prevent.

So a layer carries **one** `ClipLook` rather than a field each. Both paths pass
it straight through, the resolved transform included, and there is no list to
keep in step: adding a look field now touches the one function that builds a
look from a clip, and reaches the preview and the export without either being
edited. The compiler was the test — before, a new field needed a line in each
mapping and forgetting one said nothing.

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

All seven of §25's — crossfade, fade through black, slide, push, zoom, flash
and blur dissolve — attached to the **outgoing clip** rather than to the track. That placement is the whole design: move, trim,
split, cut and paste all carry the clip, and a transition stored beside the
timeline would have to be rewritten by every one of those operations. It is the
same argument §24 makes for anchoring keyframes to the source.

The timeline still does not overlap. §8's non-overlapping, sorted tracks are
what make the visible-range query a binary search, and a dissolve is not worth
spending that on. Clips stay where the user put them; the transition is a window
*centred on the cut* during which `layer_requests` returns **two** layers instead
of one, the incoming clip at a partial opacity. The compositor needed no changes
at all — §22's alpha-over already draws one picture over another.

The moving kinds needed none either, which is the point of adding them this
way. A slide, a push and a zoom differ from a crossfade and from each other
only in *where the two layers are drawn*, and the compositor already applies a
transform per layer — so all three are a handful of numbers in one function
beside the fade, not a shader each. The placement is applied over whatever the
clip already has, so a transition on a clip the user has moved shifts it from
where they put it. The two kinds left out, flash and blur, are the two that
would need something new: a generated white layer, and a blur across a *pair*
of layers rather than one clip. What holds all three together is one
number — that an offset of 1.0 is one whole frame width — so that number is
measured on a real device rather than assumed: a solid layer is rendered at a
known offset and its edge is found. Every other transition test would still
pass if the compositor meant something else by position, and every slide would
be wrong.

The interesting part is handles. A crossfade shows both clips at once, so before
the cut the incoming clip must supply frames from *before* its in-point, and
after it the outgoing clip must keep reading *past* its out-point. A clip
trimmed to the edge of its file has no such material, and a transition placed
anyway would render as a black flash. So the model works out what is actually
there and clamps the length to it — in the command, not in the interface, because
§38.2 replays commands after a crash and a check that only runs on the way in
comes back unchecked on the way out. Everything that shows two clips at once
needs them. Two kinds do not: a fade through black and a flash, because each
covers the cut with a colour rather than with the other shot, so both work on
any cut — including one against the very start or end of a file, which is where
a phone recording is usually trimmed to. The inspector offers whichever kinds
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

### Captions

Milestone 10, and cheap because §26 had already been built: a caption *is* a
text clip, one whose timing came from a file. So there is no caption renderer,
no caption track type and no second styling system — importing a subtitle file
produces ordinary `TextClip`s on an ordinary text track, and everything that
already works on a title works on them.

`crates/captions` owns the formats and nothing else. SubRip and WebVTT, because
between them they cover what transcription tools emit. The parsing is written
against what tools actually produce rather than against a grammar: a UTF-8 BOM,
CRLF, a missing or wrong index, `.` instead of `,` before the milliseconds, a
missing hours field, WebVTT cue settings trailing the end time. All of those are
in real files and all of them parse. What does *not* pass silently is a timing
line that cannot be read at all — that block is counted and reported (§50),
because losing a third of someone's subtitles with no word said is worse than a
warning.

Subtitle files also arrive out of order, overlapping, blank and occasionally
backwards, and §8's tracks are sorted and non-overlapping. Something has to
reconcile those, so `tidy` does it once, before the model sees anything: an
overlapping cue **shortens the earlier one** rather than moving the later one,
because a caption is anchored to the moment the words are said and sliding it
would put the subtitle after the speech. A cue left as a sub-frame flash by that
shortening is dropped rather than kept.

Two decisions worth naming. Captions get a lane of their own, because a subtitle
file is dozens of clips end to end and dropping them among someone's titles
would either collide or scatter. And a re-import **replaces** the lane rather
than appending, because importing a corrected file over an old one is the common
case — with the old captions still one undo away, since the whole import is a
single undo step.

The font list is whatever is installed on the machine — 130 families here,
against the three generic names it started with. Read once at startup from the
rasterizer that has already enumerated them, because doing it again would cost
the same startup time and the same memory for an identical answer. The three
generic names still come first in the picker: a project using "Sans" opens
correctly on a computer that has never heard of the font this one happens to
have.

**What does not work yet:** word-level timing is modelled (§27's `CaptionWord`)
but not filled in — SubRip has none, and WebVTT's karaoke timestamps are
stripped rather than read. That is what §27's animated subtitles will need, and
Phase 3 is where they are. There is no automatic transcription (§28).

### Speed

Every other feature so far has fitted around an assumption stated in half a
dozen comments: *a tick on the timeline is a tick in the source*. Speed is the
one that breaks it, so the mapping had to become explicit everywhere it was
implicit — `source_time_at`, both trim edges, split, the transition handles,
the filmstrip.

The rate is a `Rational`, not an `f32`. §9 and §74 forbid floating point in
position arithmetic, and this is position arithmetic: at 2× the source time for
a timeline position is scaled by the rate, and doing that through `f64` drifts
across a long clip in exactly the way the 960,000-tick timebase exists to
prevent. `Rational::scale` multiplies through `i128` and rounds to nearest, so
scaling out and back is a round trip rather than a slow march towards zero — a
test walks an hour of ticks through awkward ratios to check it.

Speed changes how *long* a clip is, which is what makes it different from every
other control: the source range is what the clip plays, and the rate decides how
long that takes. So the neighbours have to move. Both alternatives were tried
and are worse — leaving them opens a gap on every speed-up, which is the
commonest thing anyone does, and refusing for want of room makes slowing a clip
down fail on any track that is not the last one. So it ripples, per track,
exactly as ripple delete already does and for the same reason: a sequence-wide
ripple would drag music and overlays the user never touched.

One consequence worth stating, because it is not obvious. §25's crossfade
handles are measured in *source* and a transition window is measured in
*timeline*, so a clip at 2× burns two ticks of footage for every tick of
dissolve. Its handle is worth half as much, and the interface now offers half
the crossfade it used to on the same footage.

Sound re-times with the picture, which needed two things that were missing.

Importing a video used to place the **picture only** — nothing in the product
ever constructed an `AudioClip`, and every import became a video clip whatever
the file was, so an imported song arrived as a video clip on a video track. A
file now places both halves, starting together, as one undo step.

That immediately created the problem it exposed: two clips that must stay in
step. They share a `LinkId`, set when the file is placed, and re-timing either
one re-times both — from the sound side as readily as the picture side, because
selecting the audio clip is just as likely. Audio is resampled by linear
interpolation, which changes the pitch. That is deliberate: sped-up sound is
higher, exactly as it is on tape, and preserving pitch needs a phase vocoder
rather than a resampler. Claiming to do it would be worse than not offering it.

Linked clips stay in step through every edit, not just the speed control.
Moving, trimming, splitting and deleting the picture does the same to its
sound, from either side — the link is symmetric, because selecting the audio
clip is as likely as selecting the video one. Each is one undo step and all or
nothing: if the sound cannot move where the picture is going, neither does,
because a half-applied move is precisely the out-of-sync state the link exists
to prevent.

Split was the subtle one. It clones the clip, so both halves of the picture and
both halves of the sound came out sharing the original link — four clips that
would all move together where two pairs were meant. The split now hands each
side a fresh link of its own: left with left, right with right.

The *selection* is deliberately not widened to the partner. That would have
been simpler — and would have made the Inspector show "2 clips selected", with
no controls, for every imported video. Only the edits reach the partner, and
the timeline outlines it so the first a user learns of the link is not the sound
jumping when they let go of a drag. `Unlink Audio` in the clip menu detaches
them on purpose.

While a drag is in progress the partner shows as a pale ghost on its own lane,
at exactly the place the editor will put it — computed by the same delta the
editor applies, so the ghost cannot be somewhere the clip does not end up.

**What does not work yet:** speed is not keyframed, so there is no ramp from one
rate to another. Text
uses the machine's fonts rather than bundled ones — §26 wants
`assets/fonts/` for templates (Milestone 11), and until there is a font picker a
single bundled family would be the *only* family on offer. Audio does not
crossfade: mixing two sources is a different mechanism from blending two
pictures, and §25 v1 is picture only.

### Audio off the UI thread

Mixing used to run on the UI thread, once per frame, feeding a device ring
buffer that holds 150 ms. That holds up until the UI thread stops — and on
Windows it stops whenever the window is dragged or resized, because the message
loop blocks for as long as the mouse is held. The sound cut out 150 ms later.
Audio decoding ran there too, so a slow decode also cost a video frame.

It now has §20a.2's dedicated mixer thread. The thread cannot read the project —
§54 makes the editor core its only owner — so it is sent an `AudioPlan`: a
snapshot of the audio tracks and the assets they read, re-sent only when it
changes, since a slider drag on a video clip is a project change sixty times a
second and none of them concern the mixer. There is no lock anywhere between the
UI and the sound; the two talk only through a channel.

The refactor also removed a bug. Export carried its own copy of the mixing loop,
and the copies had already diverged: the preview resampled a sped-up clip and
the export did not, so a 2× clip exported with its sound at normal speed and cut
off half way. Both now call one `AudioMixer::mix_block`. A test mixes the
fixture's 440 Hz tone at 1× and 2× and counts zero crossings — with resampling
removed, the ratio comes back 0.99 instead of 2, which is exactly what the
export was producing.

### Whole-video volume

With nothing selected, the Inspector's Audio tab sets the whole video's volume —
§20a.4's master gain. It is project data rather than a monitoring level, which
reverses an earlier call of mine: §20a.4 puts master gain inside the mix graph
whose order "must be defined, because preview and export must match", so a
volume that changed what played and not what exported would be exactly the
mismatch §46 forbids, and the only whole-video control that silently did not
reach the file. A test exports at 100% and at 25% and measures the audio in each
file; with the old hard-coded unity gain the quieter one came back at 1.00 of
the louder.

Found on the way: the reset button beside any volume control could never be
enabled. Gain is not an animated parameter, so "is this the default?" was asked
of an empty list of parameters — and `all` over nothing is true.

### Templates

§31's flow, in one window: pick a template, drop media into its slots, replace
its text, and it lands on the timeline as **one undo step** (§77). Six ship
with the editor — Quick Intro, Two-Shot Promo, Picture in Picture, Outro, Top &
Bottom Meme and Photo Montage — and your own `.json` files can be added from
the same window, kept in a folder beside the editor's other per-user data.

A template is data, never code (§64), and nothing reaches the engine that has
not been through the validator. Three decisions there:

- **Every problem is reported, not just the first.** Whoever wrote the file
  fixes it in one pass instead of one error per attempt.
- **Out-of-range numbers are rejected, not clamped.** Clamping is right for a
  slider, where the user is watching the result; in a template it would quietly
  render something other than what its author wrote, and they would hear about
  it from someone else.
- **Nothing panics on any input.** A template picker that crashes on one bad
  file in a folder takes the rest of the folder down with it, so the tests feed
  it every truncation of a valid file, a few thousand single-byte corruptions,
  and every JSON type in every field. That found a real one: an authored speed
  of `1e308` saturated to `i64::MAX` on the way to a ratio and overflowed.

A template's slots are filled from the project's own media, an empty slot is
skipped rather than filled with something nobody chose, and the report says
which — "left empty: Music" beats wondering why the edit is silent.

### Photos

Stills can be placed, which is what makes montages and logos possible. A photo
has no length of its own, so it gets five seconds and can be dragged out to
anything; it never runs out of footage, so it crossfades against anything and
trims without limit.

Two things stop a photo being expensive. Every instant of it maps to the same
picture, so it is **decoded once** rather than once per frame — the frame cache
is keyed by time, and without that mapping a 12-megapixel JPEG would be decoded
sixty times a second. And a decoded still is capped at 4096 pixels on its long
edge: a phone photo at full size is 60–250 MB for one frame, past what many GPUs
accept as a single texture, and nothing is exported larger than 4K anyway.
Stills get no proxies — a proxy is a lighter *video* to decode each frame, and
there is only one frame.

### Titles that arrive and leave

A title can fade, slide up or down, pop, or type itself in — at either end, with
its own length. They are presets rather than curves: one choice and one number,
with the keyframe editor already there for anything else.

The typewriter is the one with a trap in it. Laying out only the typed part
would re-centre the line with every letter, so the words would crawl sideways as
they appeared. Instead the whole title is laid out and the letters not yet
reached are simply not drawn — same bitmap, same line breaks, every letter
already where it will finish.

Both the preview and the export ask one function what a title looks like at an
instant, so an animated title exports as the one that was watched (§46).

### Fades, track volume and pan

Sound clips fade in and out, drawn as ramps on the clip. The envelope is
quadratic rather than linear: a linear ramp of amplitude sounds like it holds
loud and then drops away at the last moment, because loudness is heard roughly
logarithmically.

The fade is a function of **where in the clip** each frame falls, not of where
the block boundaries are — so the preview's 480-frame blocks and the export's
frame-sized ones produce the same samples, which a test checks by mixing the
same span whole and in halves.

Tracks have the volume and pan §20a.4's mix graph always had a stage for, set
from the track header's menu and shown as a badge on the header, so a quiet lane
is never a mystery.

### Markers, and finding the beat

`M` drops a marker at the playhead. Clips and titles snap to them, and ↑ ↓ stop
at them as well as at cuts — which is what makes cutting to music a matter of
dropping clips onto marks.

**Mark Beats** puts one on every beat of a clip's music. It reads the waveform
the timeline already draws, so it costs no decoding at all: onset strength on a
log scale, the tempo by autocorrelation weighted toward usual tempos, then beat
by beat so a live recording that drifts is followed rather than left behind by a
rigid grid. Each of those three is held by a test that fails without it — a
softer snare being heard as half-speed, a tempo between buckets sliding off by
the end of a minute, and a drifting band.

### Removing silences

§78's flow, in order: analyse the clip's sound, **show** the suggested cuts, and
only then cut. The pauses are shaded on the timeline while the window is open,
and the three sliders — how quiet, for how long, how much breath to keep —
change the suggestion live, so "that took out a breath" is fixed by moving a
slider rather than by undoing and guessing again.

Confirming splits each stretch out at both ends and ripple-deletes it, picture
and linked sound together, from the last stretch backwards so an earlier cut
never moves a later one out from under the edit — all as one undo step.

### Normalising a level

"This one is too quiet" is the most ordinary problem in an edit, and the answer
is already in the waveform. **Normalise Volume**, on a clip with sound, sets its
gain so the loudest moment it *plays* — its own trimmed range, not the whole
recording — lands a dB under full scale.

A dB of headroom rather than none, because a peak sitting exactly at full scale
clips the moment anything is mixed with it. It works in both directions: a clip
peaking at full scale is brought down as readily as a quiet one is brought up.
A clip already there is left alone rather than given an undo step nobody can
hear, and one below the waveform's own floor is refused — multiplying a noise
floor by a thousand is not what normalising means.

It is the **peak**, not the loudness. Two clips normalised to the same peak can
still sound very different, because loudness is closer to an average than a
maximum, and measuring it properly needs the samples and a K-weighting filter
rather than an 8-bit peak envelope. The README would rather say that than have
the feature quietly claim more than it does.

### The output meter

Two bars beside the transport, showing the peak of each side of what is going
to the device right now. Amber past -3 dB, red when the limiter has had to
clamp — the one thing a meter must never be quiet about.

On a decibel scale, not a linear one: linearly everything from a whisper to a
shout crowds into the top fifth of the bar and the meter is decoration. -60 dB
to zero spreads the range the way the ear hears it, so half amplitude sits near
the top (which is what it sounds like) and a quiet passage still has somewhere
to move. Peak rather than average, because the question a meter answers is "is
this about to clip?" and an average says no right up until it does.

The mixer thread publishes the level through atomics — it must never wait on
the interface (§20a.2, §54) — and clears it when playback stops, because a
meter holding the last level of a stopped mix looks like sound that is not
there.

### Ducking music under a voice

The most common mix note in the world: *the music is too loud under the
talking*. By hand it is a keyframe either side of every sentence. **Duck Under
Voice**, on a clip with sound, dips it wherever anything else is speaking over
it — every other clip that overlaps it, on any track, using the speech detection
the captions use.

Four keys per duck: full level an attack before the words, down as they start,
held to the end, back up over the release. Speech separated by less than 1.5 s
stays one duck, because music surging back for half a second between two
sentences pumps, and pumping is worse than leaving it down. The whole shape is
one undo step, and **Clear Ducking** puts the clip back.

The dip is drawn on the clip: a pale line across the waveform, high where the
music is up and falling where it is ducked, with a dot at each key — **and the
dots can be dragged**, in both axes, because a duck is adjusted as much by
moving *when* it happens as by how deep it is. A point is held between its
neighbours: keys that crossed would be reordered as the envelope is written,
and the point under the pointer would become a different key. The whole drag is
one undo step (§11). Double-click the line to put a point in, at the level the
line already has so the shape does not jump, and double-click a point to take
it out — down to the last two, because an envelope of one point is a level
rather than a shape, and emptying it is what Clear Ducking is for. It is
sampled from the same `gain_at` the mixer asks, so it draws what is *heard*
rather than what the keys mean — an envelope that never reached the mixer would
draw flat.

Underneath it is ordinary volume automation (§24 on §20a.4's clip-gain stage),
so the same envelope is available for riding a level by hand. The audio thread
never walks a data structure or allocates (§54): the engine evaluates the
envelope at both ends of each block and hands the mixer the line between them,
which is also why a duck ramps smoothly instead of stepping at block
boundaries. Export mixes through the same `AudioMixer::mix_block`, so what is
heard is what is written (§46).

### Timing captions from speech

§28's automatic captions want a transcription engine — a model and a download
away. But transcription answers two questions, *when* and *what*, and only one
of them needs a model. **Time Captions**, on a clip's right-click menu, answers
the *when* from the waveform the timeline already keeps and puts an empty
caption on each phrase, ready to type into.

That is the tedious half. Typing a sentence takes a moment; finding the instant
it starts takes a scrub, a nudge and another scrub, thirty times over.

Three rules make the result readable rather than merely accurate: a gap shorter
than 350 ms is inside a phrase rather than between two, a burst shorter than
300 ms is a cough rather than a phrase, and a stretch longer than five seconds
is divided evenly — two captions of four seconds read better than one of five
and one of three. Each is held by a test that fails without it.

Captions with no words draw nothing and export as nothing, so a half-typed lane
is a half-finished job rather than a file full of blanks. When a real provider
lands it answers in the same shape with the words filled in, and everything
downstream of that point is already built (§46).

### Title looks

Four presets on a title — **Headline**, **Lower third**, **Quote**,
**Typewriter** — each carrying *where it sits* as well as how it looks, applied
as one undo step. A lower third in the middle of the frame is not a lower third,
so a look that changed only the style would be half a look.

Separate from the caption looks on purpose: a caption is one long thread of the
same thing, styled by the lane, while a title is placed and dressed on its own.
They keep the same readability rule, though — every look carries a box, a rim or
a shadow, and every one wraps.

### The Captions window

Timing thirty captions takes a second; typing into them by clicking each one on
the timeline and crossing to the Inspector is thirty selections and thirty trips
across the window, which would make the timing feature a way of *creating* work.

So there is a list — **Captions** in the toolbar, and it opens by itself after
timing. One row per caption, in order: the timecode plays from there, the field
takes the words, and Enter moves to the next row. The row the playhead is inside
is picked out, and the gaps between captions belong to no row, which is the
honest answer.

Four looks sit across the top — **Boxed**, **Outlined**, **Highlight**,
**Soft** — and dress the whole lane in one undo step. The look belongs to the
lane rather than to a caption, because subtitles that change style halfway
through read as a mistake, and titles are deliberately left out: they are
placed and dressed one at a time, which is the difference between a title and a
caption. Every look carries its own contrast — a box, a rim or a shadow —
because plain letters vanish over a bright shot, and a caption that cannot be
read is worse than none.

The list is a view, not a copy. Every row reads its clip each frame and every
keystroke goes through the editor, so the timeline and the list cannot disagree,
and typing a sentence is one undo step — with the caption *being typed into*
tracked, because every caption edit shares a history label and a keystroke
marked as continuing would otherwise fold into the caption typed before it,
where one undo wipes both.

### Look strength

The dial CapCut puts under every filter: the same look, applied less. It appears
once a clip is on one of the presets, and slides from nothing to the look as
written.

The clip stores **only the grade it ended up with** — which preset produced it,
and how strongly, is recovered by inverting the interpolation. Storing the
preset beside the grade would be two records of one fact, and they would
disagree the first time someone nudged a slider; recovering it also means the
strength still sits in the right place when a project is opened a week later.

Two cases had to be answered for. A grade that happens to share one number with
a preset is not that preset, so the strength has to come out the same on all
three axes or the look does not match at all. And an untouched clip is at zero
strength towards *every* look, which would light up whichever preset came first
over every ungraded clip — so ungraded is its own answer.

### Seeing what a clip is doing

A mask, a key or a blend other than normal looks exactly like an ordinary clip
in the lane, and a user left wondering why the preview disagrees with the
timeline is a user who distrusts both. Each now shows a small badge at the top
right of its clip, beside the hold and speed badges that were already there.

They stack leftwards in one row and stop rather than overrun the file name,
because a badge sitting on top of the name says less than no badge at all. The
hold and speed badges used to be drawn independently at the same anchor, which
worked only because a held frame cannot also be re-timed — one row is what lets
a clip carry three at once.

Some of it is drawn as shapes rather than words, because that is what the thing
is: a clip with an entrance gets the same shaded ramp a sound's fade gets, so a
shot that is arriving looks like it instead of looking ordinary with a badge on
it. A soloed track says **SOLO** on its header in the loudest thing in the bar,
since a solo left on is the reason every other lane has gone quiet — and the
export window says so too, because the export reads the same rule the preview
does and would otherwise quietly contain only that one track.

### Blend modes

**blend** on a video clip: Normal, Screen, Multiply, Add. Screen never darkens,
so black in an overlay disappears — which is how a light leak, a glow or a dust
plate is laid on. Multiply never lightens, so white disappears. Add is brighter
than screen and clips sooner.

A blend state lives in a wgpu pipeline and nowhere else, so four modes are four
pipelines sharing one shader — and that shader now emits **premultiplied**
colour, because each mode is a different pair of blend factors and every one of
them needs the source already weighted by its alpha. Otherwise a 50% overlay
would screen at full strength.

That change moved two golden frames, and both are recorded in the test with the
reason. One is an improvement: hardware clamps a fragment before blending, so
scaling by opacity in the shader keeps range the old clamp-then-scale threw
away. The other is a real cost of a fraction of an 8-bit step, because the
premultiplied product is quantised before the blend — accepted knowingly, since
the alternative is a float target for every composite.

### Masks

**mask** on a video clip keeps part of the picture and hides the rest: a
straight edge, a box or an oval, each with a centre, a size, an angle, a feather
and an invert. Split screens, picture-in-picture and reveals are all made of
these.

Everything is in the clip's **own** frame, 0–1 across the picture, so a mask
stays over what it was drawn on however the clip is afterwards moved, scaled or
re-timed. A mask in output coordinates would slide off its subject the moment
the clip was nudged.

The three shapes are one signed distance each — negative inside, zero on the
edge, positive outside — so a single feathering rule serves all of them, and the
feather straddles the edge rather than eating inwards from it: softening a mask
must not also shrink it.

It rides in the same composite pass as the key and the colour adjustment, and
costs nothing when there is no mask.

A masked clip shows two handles on the preview — its centre and its edge — so
it can be moved and resized there rather than typed into the Inspector. The mask is in the clip's own frame
and the preview shows that frame as a box which may be moved, scaled and turned,
so dragging goes through the box and back — which means a mask on a turned clip
follows the *picture* rather than the screen. The conversion round-trips
exactly, because a mask that drifted a little every time it was picked up would
be worse than one that could not be dragged at all. A turned mask grows along
its *own* axes: on a mask at a quarter turn, dragging down the screen widens it
rather than making it taller, because what is "down the screen" is its width.
The size stops short of nothing, since a mask dragged away to zero has no handle
left to drag it back by.

### Green screen

**green screen** on a video clip makes one colour transparent: pick the screen's
colour, then tolerance, softness and spill.

The colour is picked off the picture, not guessed on a wheel. **pick** arms an
eyedropper and the next click on the preview reads that pixel back off the GPU
and makes it the key — no two green screens are the same green once a light has
been near them, and the right answer was always already on screen. While it is
armed the picture is a colour chart rather than something to drag, so the
transform handles stand down and Escape puts it away. Re-picking keeps the
tolerance and softness already dialled in: the usual reason to pick again is
that the first point was slightly off.

The key works on **chromaticity** — the proportions of a colour with its
brightness divided out — rather than on plain colour distance, and that choice
is the whole feature. A real screen is never evenly lit, and its shadowed folds
are a long way from its lit parts in RGB; keyed on distance they survive as dark
green fringes. Keyed on proportions they are the same colour, so both go. That
is a test: a frame that is lit screen on one side and deep shadow on the other,
keyed off the lit half, has to lose both.

Black is the case that needs protecting from the other direction. It has no
proportions to speak of, so it must be answered for explicitly or the key eats
every shadow in the shot.

Two more things worth saying. The tolerance dial cannot be turned up until the
picture disappears: chromaticity distance runs past 1.0 between the furthest
colours, and the cap is deliberately short of that, because a key that can
remove everything is not a key. And spill suppression exists because a green
screen throws green onto what is in front of it — a subject keyed against one
has a green rim even where it is fully opaque.

It rides in the composite pass with the colour adjustment, so it costs nothing
when it is off and no extra pass when it is on. The key colour is converted to
linear light once per layer on the way in, because the shader compares it
against samples from an sRGB-aware texture (§21a.1).

### Clip animations, and blurring what moves

A shot that arrives gets the same presets a title does — fade, slide in any of
four directions, pop, spin — because an editor in which "slide up" means one
thing on a caption and another on a shot is one nobody can predict. They share
every preset and all of the arithmetic; the single difference is how far a slide
travels. A caption is nudged, because one that flew in from off-screen would be
unreadable while it travelled. A shot comes in from outside the frame, because
that is the effect.

An animation is applied **on top of** the clip's own placement, so a shot pushed
into a corner slides in to that corner rather than to the middle. It is decided
in the same function the export reads (§46), and it draws on the timeline as the
ramp a fade gets.

**motion blur** smears a moving shot along its path: the same picture drawn at
the places it passed through, each fainter than the last. §45 calls motion blur
expensive and it is when done properly — a velocity buffer and a directional
pass. This is what the compositor can already do without either, which is also
what a shutter records. A shot that is not moving is drawn once, so leaving it on
costs nothing until something moves, and the copies share the opacity the layer
would have had alone — otherwise turning it on looks like turning the exposure
up.

Anything that moves smears: an entrance, a keyframed move, a slow zoom. Nothing
in the smear knows which. Three places in the engine take the layer they just
pushed and rewrite it — a backdrop, a softened half of a dissolve, a shot placed
by a moving transition — and each asks for a single layer rather than a trail,
because a copy none of them rewrote is the shot drawn where the effect never put
it.

### Carrying a look to the other clips

Setting up a shot is minutes of work and the next twenty want the same
treatment. **Copy Look** takes the grade, blur, opacity, blend, mask, key,
animation, motion blur and backdrop off one clip; **Paste Look** puts them on the selection,
as one undo step. **Clear Look** is the same edit with a plain look instead of a
copied one, so clearing and pasting cannot disagree about what a look contains.

What it leaves alone matters as much. Framing is a decision about the individual
shot — a wide landscape and a close-up do not want the same crop — and keyframes
are anchored to source time, so the same keys on a clip of another length land
somewhere else entirely. A sound in the selection is skipped rather than
refusing the whole paste, because a timeline selection carries the linked sound
with it (§12).

### Filling the frame behind a shot

Fit leaves bars, and for a landscape shot in a vertical edit those bars are most
of the screen. **behind: Blur**, beside Fit and Fill, puts the clip's own
picture back there — blown up to cover and heavily softened, which is what every
phone editor does.

It is a property of the *clip*, not a second clip on a lower track: it is the
same picture, and anything that moves, trims or re-times the shot has to carry
it. A backdrop built from a duplicate would come apart the first time either was
touched — the argument §25 makes for attaching a transition to its clip.

Three details that are each a test. The backdrop does **not** inherit the clip's
own framing, so a shot pushed to one side still has a full frame behind it. It
is skipped entirely when the clip already covers the frame, where it would be a
second decode and a second draw of something nobody can see. And it is not drawn
during a transition: both clips are moving then, and a backdrop would be revealed
at the edges as they slide — a picture of the bug rather than of the shot.

It does carry the clip's opacity and its animation, which took a correction. The
backdrop used to be forced fully opaque so a half-transparent shot still had a
full frame behind it; that reasoning only holds over black, and over another
track it meant a clip at any opacity blotted out everything beneath it — a clip
faded to nothing still covered the frame with a blurred copy of itself.

Where there is no picture at all, the frame shows the **background** colour from
the whole-video tab: black by default, which is what it always was, and the
answer to black bars for anyone who would rather have white or a brand colour.
It is stated in sRGB like every other colour here and converted once on the way
to the clear, because the target encodes on write and a value handed over
unconverted comes out visibly pale.

### Changing shape for another platform

§36's resize is about the canvas: the **shape** row in the Inspector switches
between 16:9, 9:16, 1:1, 4:5 and 21:9, keeping the short edge — reshaping
1920×1080 to vertical gives 1080×1920, not something smaller in both directions
— and the source media is never touched.

What §36 leaves out is that every clip is then a different shape from the frame.
An edit cut in landscape and switched to Shorts is a sequence of pillarboxed
clips, and saying "Fill" to each one by hand is the work an editor should
absorb. **clips: Fill frame / Fit frame** says it once for all of them, as one
undo step.

Clips with a movement of their own are deliberately left alone and counted
separately in the message. A Ken Burns push is framing the user wrote by hand;
flattening it to one number would throw away work that cannot be guessed back.

### Cutting on the beat

**Cut on Beats** splits a clip at every beat of its music. Both halves of it
already existed and were tested — where the beats are, and cutting a clip at a
list of instants — so the feature is the joining, plus one piece of judgement:
more than four hundred cuts is refused with the count rather than performed. A
whole song at 120 bpm is a thousand clips and an undo step nobody can see the
end of.

The beats come from the sound and the cut lands on the clip, which carries its
linked partner (§12): cutting a music video on the beat means cutting the
picture, not the song.

### Finding the cuts in footage

A file that came off a camera is one shot. A file that came from anywhere else —
a download, a screen recording, last year's export — is usually many, and
re-cutting it by hand means scrubbing for every join. **Find Cuts**, on a clip's
right-click menu, reads the footage and offers them.

Each frame is reduced to the average brightness of a 16×9 grid, and a cut is
where the picture changes far more than it has been changing. Both halves of
that matter: a fixed threshold alone marks a cut every few frames of a whip pan,
and a relative one alone marks the noise in a locked-off shot of a wall. Tests
hold both — a sweeping bar that must *not* be cut into pieces, a camera flash
that must not become a one-frame shot.

The reading happens on a worker (§48: cancelling closes the window and stops the
decode), over only the part of the file the clip actually plays. The cuts are
drawn on the timeline as dashed lines while the window is open, nothing is cut
until you say so, and confirming makes every split as **one undo step** — §11's
rule that the history holds intentions, and "cut this at its scene changes" is
one intention.

### Looks, framing and movement

Three small things that remove arithmetic from common jobs:

- **Looks** — Punchy, Soft, Faded, Moody, Black & white — are three colour
  values that only mean something as a set, so they go on and come off in one
  step. Offered on a clip or on the whole video, and hidden where keyframes
  would override them.
- **Fit and Fill** sit beside the scale slider. Fitting leaves bars at the
  sides; filling covers the frame — the everyday need when landscape footage
  lands in a vertical edit. `fill_scale` lives beside `fit_scale`, because it is
  the same decision seen the other way round.
- **Movement** writes a slow zoom as ordinary scale keyframes, eased at both
  ends and relative to the clip's own framing, so a shot set to fill keeps
  filling while it moves. A template can ask for the same move by name, and both
  routes write the same keys from one place.

### Making room, and holding a frame

**Insert Gap** opens time at a point across every track — clips and markers
from there on move right by the same amount, and a clip the gap opens inside is
split there first, its halves keeping their own sound. It is the inverse of
removing a silence, and the thing a freeze frame needs.

**Freeze Frame** holds the frame under the playhead. It opens a gap and drops
in a clip that holds that one frame, so the shot's sound is cut and moved with
it and nothing after drifts: holding the picture *without* making room would
leave the sound running underneath and the two out of step from there on.

No image is written to disk for it. A hold is an ordinary clip with a flag, and
every instant of it asks the decoder for the same source time — so it costs one
decode however long it is held, the same trick photos use, and it saves and
recovers as plain project data. Two details follow from being a clip rather
than a mode: keyframes are evaluated against the clip's *progress* rather than
the frame it reads, so a held frame can still drift or fade; and its trim has no
end of footage to hit, because it is one frame, so it can be stretched as far as
wanted. The timeline badges it **hold** and draws no filmstrip on it — tiles
marching across a picture that never moves would be the clearest possible lie.

### Getting media in, and the keyboard

Files dragged from the desktop are imported; dropped on the timeline they are
added to its end as well, and an overlay says which will happen before the
mouse is let go.

The keyboard covers the edits a mouse is bad at: `,` and `.` nudge by a frame
(Shift: ten), `[` and `]` trim an edge to the playhead, ↑ ↓ jump between cuts
and markers, `M` marks. Every key is listed in one table that the **Shortcuts**
window (`?` or F1) reads, so the list a user sees and the keys that work are
edited side by side.

Some limits worth knowing before testing with your own footage:

* **Decode-ahead is unmeasured on the target machine.** It works — playback
  takes frames from the ring rather than decoding inline, asserted by test —
  but the half-second budget and the one-thread choice were sized by
  calculation, not measured on §52.1 hardware.
* **Software decode only.** §5's hardware-decode-to-texture path is still open;
  this is the RAM fallback §5 requires to exist.
* **HDR is tone-mapped by one chain, used twice.** PQ and HLG sources are
  converted into the SDR working space (§21a.1) by a `zscale` + `tonemap` chain
  that both the proxy encoder and the decoder run. It used to be the proxy
  alone — so an HDR clip looked right while editing, and the **export**, which
  reads originals (§14), came out at about half the brightness. A test decodes
  the HLG fixture's original directly and measures mean luma: 227 now, 122 with
  the chain switched off. SDR sources never touch it.
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
