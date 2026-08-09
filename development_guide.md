# Lightweight Video Editor — Development Guide

**Schema version: 2**

---

# 0. Changelog vs v1

This revision changes the following. Each item is marked `[EDIT]`, `[NEW]`, or `[CUT]` at its section.

**Structural changes**

1. **UI stack changed from Tauri/TypeScript to single-process Rust + egui + wgpu** (§4). Rationale and the conditions for reversing this decision are in §4.1. Sections §5, §53, §54, §55, §56 were rewritten to match.
2. **Milestone 0 added** (§82). A throwaway spike that validates the frame-transport path before any product code is written.
3. **Licensing decision promoted to a blocking pre-Milestone-6 task** (§0.1). H.264 export is in the MVP, so this cannot wait.

**Correctness fixes**

4. **Timeline timebase changed from 1,000,000 to 960,000 ticks/second** (§9). The old value cannot exactly represent NTSC frame boundaries.
5. **Proxy codec specified as all-intra** (§13). This is the main lever on seek/scrub latency.
6. **Audio section added** (§20a). Audio is the master clock; this was previously unspecified.
7. **Color management section added** (§21a). Without it, §46's "preview and export must match" is unachievable.
8. **Text rendering ownership assigned to the core renderer** (§26).
9. **Seek architecture section added** (§47a).

**Robustness fixes**

10. **Autosave changed to atomic write + command journal** (§38).
11. **Golden-frame tests added** (§51) to enforce §46 mechanically.
12. **Single render graph, two configurations** stated structurally (§46).
13. **FFmpeg thread caps and worker priority added** (§15).
14. **Reference benchmark machine required** (§52).
15. **MVP narrowed**: transitions and text overlays moved to Phase 2 (§59).
16. **Packaging and distribution section added** (§88a).
17. **Two rules added to the agent prohibition list** (§74).

---

## 0.1 Blocking Legal Decision

**This must be resolved before Milestone 6 (Export). It is not a Phase 3 concern.**

Three separate issues:

1. **FFmpeg licensing.** FFmpeg core is LGPL. Build and link LGPL-only, dynamically. Do not enable GPL components.
2. **x264 is GPL.** Linking it makes the entire product GPL. Do not use x264 as the default software encoder.
3. **Codec patents.** Distributing an H.264/HEVC encoder in a commercial desktop product has patent-pool implications independent of software licensing.

**Required posture:**

```text
Preferred:
OS-provided encoders
  Windows  → Media Foundation (NVENC / QuickSync / AMF)
  macOS    → VideoToolbox
  Linux    → VAAPI

Fallback:
openh264 (BSD, Cisco-sponsored binary distribution)
or
AV1 via SVT-AV1 (BSD) where playback support is acceptable
```

Using OS encoders is also faster on the target hardware, so this aligns with the performance goal.

**Action:** obtain legal review of the encoder distribution plan before Milestone 6 begins.

---

# 1. Project Goal

Build a lightweight desktop video editor focused on:

* Excellent performance on low-end and mid-range laptops.
* Fast and simple editing.
* CapCut-style convenience without requiring expensive hardware.
* Template-driven editing.
* Automated editing features.
* Fast preview and timeline interaction.
* Minimal RAM and CPU usage when idle.
* Hardware-accelerated decoding and encoding when available.
* Extensible architecture so new effects, transitions, templates, and automation can be added without rewriting the editor.

The application should prioritize responsiveness over maximum preview quality.

The editor should remain usable on machines with:

* 8 GB RAM.
* Integrated graphics.
* Older 4-core CPUs.
* SATA SSDs or average NVMe drives.
* No dedicated GPU.

The application should still scale well on more powerful systems.

---

# 2. Core Philosophy

## Performance First

Never process full-resolution media when unnecessary.

Prefer:

* Proxy media.
* Cached frames.
* Reduced-resolution playback.
* Hardware decoding.
* GPU compositing.
* Lazy loading.
* Background preprocessing.
* Incremental rendering.

The UI must never freeze because video processing is occurring.

Heavy work must run outside the UI thread.

---

## Non-Destructive Editing

Never modify original user media.

Projects should only store:

* Media references.
* Timeline positions.
* Trim points.
* Effects.
* Keyframes.
* Transformations.
* Captions.
* Audio settings.
* Template information.

The final video is generated only during export.

---

## Templates Are Data

Effects, transitions, animations, and editing templates should not generally be hard-coded.

Define them through structured configuration files.

Example:

```json
{
  "id": "quick_zoom",
  "name": "Quick Zoom",
  "type": "effect",
  "duration": 0.6,
  "parameters": {
    "scale": {
      "from": 1.0,
      "to": 1.25,
      "easing": "ease_out"
    }
  }
}
```

The engine should interpret template definitions.

This allows new templates to be installed without rebuilding the application.

---

# 3. Recommended Technology Stack

## Core Language

Use:

```text
Rust
```

Rust is responsible for:

* Project state.
* Timeline engine.
* Media management.
* Rendering coordination.
* Cache management.
* Background tasks.
* Export jobs.
* FFmpeg integration.
* Hardware detection.
* Plugin/template management.
* **The user interface** (see §4).

Avoid implementing video codecs manually.

---

## Video and Audio Backend

Use:

```text
FFmpeg
```

FFmpeg handles:

* Video decoding.
* Video encoding.
* Audio decoding.
* Audio encoding.
* Containers.
* Rescaling.
* Resampling.
* Basic filters.
* Hardware acceleration.

Recommended binding:

```text
rsmpeg
```

Create an internal Rust abstraction around FFmpeg.

Do not allow the rest of the application to depend directly on a specific FFmpeg Rust wrapper.

Example abstraction:

```rust
trait MediaDecoder {
    fn open(&mut self, source: &MediaSource) -> Result<()>;
    fn seek(&mut self, timestamp: Timestamp) -> Result<()>;
    fn decode_frame(&mut self) -> Result<VideoFrame>;
}
```

FFmpeg becomes one implementation of the application's media layer.

This makes future changes easier.

---

# 4. Desktop UI `[EDIT — replaced Tauri stack]`

Stack:

```text
Rust
eframe / egui       — application shell and widgets
wgpu                — compositor and preview rendering
cpal                — audio output
rfd                 — native file dialogs
```

**Everything runs in one process, in one language, sharing one wgpu device.**

The UI layer handles:

* Buttons.
* Panels.
* Timeline interaction.
* Menus.
* Media browser.
* Properties inspector.
* Template browser.
* User settings.
* Project management.

The editor core handles expensive processing on separate threads.

---

## 4.1 Why This Changed, and When To Reverse It `[NEW]`

**The decision that determines whether this project ships is where decoded GPU frames get composited and displayed.** Language choice is secondary. Most editor projects die on this problem.

With egui + wgpu, the compositor's output is *already* a `wgpu::Texture`. `egui-wgpu` registers it as a `TextureId` and paints it into a rect. There is no interop layer, no frame copy, no IPC, and no overlay-window geometry management. The single hardest engineering problem in the project disappears.

The previous Tauri recommendation required a native child window composited over the webview. Costs of that approach:

```text
Preview cannot be z-ordered with UI content.
  → dropdowns, modals, and panels cannot overlap the preview
Manual geometry management on resize, DPI change, monitor move
WebKitGTK path on Linux is the least reliable of the three platforms
Timeline canvas still required — DOM virtualization is insufficient
```

egui is also reactive: `eframe` idles at near-zero repaint cost and only runs at 60fps during playback and scrubbing. This directly serves §81's idle-CPU target on thermally limited laptops.

**Known cost of egui:** typography and text rendering are its weakest area. IME support is thin. There are no native menus.

**Reverse this decision if** the spike in Milestone 0 shows egui's text quality is unacceptable for the product. In that case, in order of preference:

```text
Slint  — retained, declarative, better text, has a wgpu integration path
GPUI   — fastest, built for low-latency editors, sparse docs outside Zed
Qt/QML — most mature, proven by Kdenlive and Shotcut, but C++ and licensing
```

**Do not reverse to Tauri.** The frame-transport cost is structural, not a matter of polish.

Record whichever choice is made as `docs/adr/005-ui-shell.md`.

---

# 5. Frame Transport Rule `[EDIT — rewritten for single process]`

Decoded frames must never leave the GPU between decode and display.

Target path:

```text
Hardware decode
↓
GPU texture (no system RAM round trip)
↓
wgpu compositor
↓
egui TextureId
↓
Screen
```

Forbidden path:

```text
Decode → download to RAM → upload to GPU → display
```

A RAM round trip per frame is the exact tax the target hardware cannot afford.

**Hardware decode → texture interop is the hardest part of this project.** Budget real time for it.

```text
NVDEC / CUDA         → pleasant
VideoToolbox         → pleasant
D3D11 ↔ Vulkan/GL    → difficult
VAAPI ↔ Vulkan       → difficult
```

Implement a **software fallback** that downloads to RAM and uploads to the GPU. It is slower but correct, and it must exist so that unsupported hardware degrades rather than fails.

---

# 6. High-Level Architecture `[EDIT]`

```text
┌─────────────────────────────────────┐
│         UI Layer (egui)             │
│                                     │
│ Timeline (custom Painter canvas)    │
│ Media browser                       │
│ Inspector                           │
│ Templates                           │
│ Captions                            │
│ Export panel                        │
└──────────────────┬──────────────────┘
                   │
                   │ Commands / events
                   │ (in-process channels)
                   ▼
┌─────────────────────────────────────┐
│          Application Layer          │
│                                     │
│ Commands                            │
│ Undo / redo                         │
│ Project state                       │
│ Job scheduler                       │
└──────────────────┬──────────────────┘
                   │
                   ▼
┌─────────────────────────────────────┐
│             Editor Core             │
│                                     │
│ Timeline engine                     │
│ Project model                       │
│ Media manager                       │
│ Template engine                     │
│ Effect system                       │
│ Keyframe system                     │
└───────────┬─────────────┬───────────┘
            │             │
            ▼             ▼
┌─────────────────┐ ┌─────────────────┐
│ Preview Config  │ │ Export Config   │
│                 │ │                 │
│ Frame cache     │ │ Full quality    │
│ Proxy decode    │ │ Original media  │
│ Reduced res     │ │ Final encoding  │
└────────┬────────┘ └────────┬────────┘
         │                   │
         └─────────┬─────────┘
                   ▼
         ┌───────────────────┐
         │  ONE Render Graph │   ← see §46
         │  (wgpu)           │
         └─────────┬─────────┘
                   ▼
             ┌───────────┐
             │  FFmpeg   │
             └───────────┘
```

Note the change from v1: preview and export are **configurations of one render graph**, not two engines.

---

# 7. Repository Structure `[EDIT — frontend/ removed]`

```text
video-editor/
│
├── apps/
│   └── desktop/          # eframe entry point, window, egui shell
│
├── crates/
│   ├── editor-core/
│   ├── media/
│   ├── timeline/
│   ├── renderer/         # wgpu render graph
│   ├── effects/
│   ├── templates/
│   ├── audio/
│   ├── text/             # NEW — see §26
│   ├── captions/
│   ├── project-format/
│   ├── cache/
│   ├── export/
│   ├── automation/
│   └── ui/               # NEW — egui widgets, timeline canvas
│
├── assets/
│   ├── templates/
│   ├── transitions/
│   ├── effects/
│   └── fonts/
│
├── docs/
│   └── adr/
│
├── tests/
│   └── golden/           # NEW — reference frames, see §51
│
└── DEVELOPMENT_GUIDE.md
```

Keep modules isolated.

Avoid building a single massive Rust crate.

---

# 8. Project Data Model

A project should contain sequences.

```rust
struct Project {
    id: ProjectId,
    name: String,
    media: Vec<MediaAsset>,
    sequences: Vec<Sequence>,
    settings: ProjectSettings,
}
```

A sequence contains tracks.

```rust
struct Sequence {
    id: SequenceId,
    name: String,
    duration: TimelineTime,
    video_tracks: Vec<VideoTrack>,
    audio_tracks: Vec<AudioTrack>,
}
```

A track contains clips.

```rust
struct VideoTrack {
    id: TrackId,
    clips: Vec<VideoClip>,
    visible: bool,
    locked: bool,
}
```

A clip should contain references rather than copied media.

```rust
struct VideoClip {
    id: ClipId,

    media_id: MediaId,

    timeline_start: TimelineTime,
    timeline_end: TimelineTime,

    source_start: MediaTime,
    source_end: MediaTime,

    transform: Transform,
    opacity: f32,

    effects: Vec<EffectInstance>,
    keyframes: Vec<KeyframeTrack>,
}
```

---

# 9. Time Representation `[EDIT — timebase corrected]`

Do not use floating-point seconds as the primary timeline representation.

Bad:

```rust
time: f32
```

Use integer ticks:

```rust
struct TimelineTime {
    ticks: i64,
}
```

**Timebase:**

```text
1 second = 960,000 ticks
```

**Why not 1,000,000.** NTSC rates are everywhere — phones and screen recorders default to 29.97 and 59.94. At 29.97 fps (30000/1001), one frame is:

```text
1,000,000 × 1001 / 30000 = 33366.666…   ← not an integer
```

Every frame boundary rounds. Errors accumulate. Split-at-playhead lands off-by-one deep into a long clip.

**960,000 divides cleanly by all common rates:**

```text
23.976 fps (24000/1001) → 40040 ticks
24     fps              → 40000
25     fps              → 38400
29.97  fps (30000/1001) → 32032
30     fps              → 32000
50     fps              → 19200
59.94  fps (60000/1001) → 16016
60     fps              → 16000
120    fps              →  8000
```

It also divides evenly by 48000, so one audio sample at 48 kHz = 20 ticks exactly.

**Consequence — mandatory rule:** 960,000 does **not** divide by 44100. **All audio must be resampled to 48 kHz at import.** This is required anyway for the mixer (§20a).

Media timestamps may use their native FFmpeg timebases internally but must be converted to `TimelineTime` at the media-layer boundary.

Range check: `i64` at 960,000 ticks/sec covers roughly 304,000 years. Not a concern.

---

# 10. Essential Timeline Operations

Implement these before advanced effects.

```text
Add clip
Remove clip
Move clip
Trim left
Trim right
Split clip
Duplicate clip
Copy
Paste
Ripple delete
Track creation
Track deletion
Mute track
Hide track
Lock track
Snapping
Timeline zoom
Playhead movement
Multi-selection
Undo
Redo
```

Every editing operation must be a command.

```rust
trait EditorCommand {
    fn execute(&mut self, project: &mut Project) -> Result<()>;
    fn undo(&mut self, project: &mut Project) -> Result<()>;
}
```

Examples:

```text
MoveClipCommand
SplitClipCommand
TrimClipCommand
DeleteClipCommand
AddEffectCommand
SetKeyframeCommand
```

---

# 11. Undo/Redo Architecture

Do not snapshot the entire project after every edit.

```text
User action
↓
Command
↓
Execute
↓
Push into undo stack
```

Undo:

```text
Undo stack
↓
command.undo()
↓
move command to redo stack
```

History limit:

```text
100–500 commands
```

Commands must be serializable — the autosave journal depends on this (§38).

---

# 12. Media Import

When media is imported:

1. Read metadata.
2. Generate an internal media ID.
3. Store the original path.
4. Detect codec.
5. Detect resolution.
6. Detect frame rate.
7. Detect duration.
8. Detect audio streams.
9. **Detect color primaries, transfer function, matrix, and range** (§21a).
10. Generate thumbnail metadata.
11. Determine whether a proxy should be generated.

```rust
struct MediaAsset {
    id: MediaId,
    path: PathBuf,

    width: u32,
    height: u32,

    duration: MediaTime,

    frame_rate: Rational,

    video_codec: Option<String>,
    audio_codec: Option<String>,

    color: ColorMetadata,

    proxy: Option<ProxyAsset>,
}
```

---

# 13. Proxy System `[EDIT — codec specified]`

Proxy media is one of the highest-priority systems. It is the main reason the product can feel instant on weak hardware.

Generate proxies when media is:

```text
>= 1440p
HEVC/H.265
AV1
10-bit
high bitrate
high frame rate
difficult to seek
```

Allow the user to disable automatic proxies.

Resolutions:

```text
360p
540p
720p
```

Default: `720p`. Weak hardware: `540p`.

## 13.1 Proxy Encoding Specification `[NEW]`

**The proxy codec determines scrub latency more than anything else in the application.**

```text
Codec:        H.264
GOP length:   1  (all-intra / all-keyframe)
Profile:      High, 8-bit
Pixel format: yuv420p
Range:        limited (broadcast)
Matrix:       BT.709
Audio:        48 kHz, stereo, AAC or PCM
```

**Why all-intra.** Long-GOP media requires seeking to the preceding keyframe and decoding forward. On a 250-frame GOP that is up to 250 decodes per seek — no CPU in the target range hits §81's 150 ms target that way. With GOP=1 every frame is a keyframe, so a seek is exactly one decode.

Proxies are 3–5× larger on disk than a long-GOP equivalent at the same resolution. Accept this. Disk is cheap; scrub latency is the product.

Encode proxies with a **hardware encoder** where available — proxy generation is otherwise the heaviest background job in the app.

Proxies must also **normalize** the media:

```text
10-bit → 8-bit
HDR    → tone-mapped SDR
Any color matrix → BT.709
Any audio rate   → 48 kHz
Variable frame rate → constant frame rate
```

This means the preview pipeline only ever handles one format, which removes an entire class of bugs.

A proxy must preserve:

* Duration.
* Timing.
* Audio sync.
* Aspect ratio.

---

# 14. Proxy Editing Behavior

Editing uses:

```text
Proxy media
```

Export uses:

```text
Original media
```

```rust
struct ProxyAsset {
    path: PathBuf,
    width: u32,
    height: u32,
    status: ProxyStatus,
}

enum ProxyStatus {
    Missing,
    Queued,
    Generating,
    Ready,
    Failed,
}
```

---

# 15. Background Job System `[EDIT — thread caps added]`

Implement a central job scheduler.

```rust
enum Job {
    GenerateProxy(MediaId),
    GenerateWaveform(MediaId),
    GenerateThumbnails(MediaId),
    AnalyzeAudio(MediaId),
    Export(ExportRequest),
}
```

Never spawn unlimited threads. Use a bounded worker pool.

```text
CPU cores <= 4:
1 heavy job

CPU cores >= 8:
2–4 heavy jobs
```

## 15.1 Starvation Prevention `[NEW]`

"Pause proxy generation during playback" is necessary but not sufficient on a 4-core machine. Also required:

**Cap FFmpeg's internal threads per job.** FFmpeg defaults to using all cores. One proxy job will otherwise saturate the machine regardless of how few jobs the scheduler runs.

```rust
// per background job
codec_ctx.set_thread_count(1);   // cores <= 4
codec_ctx.set_thread_count(2);   // cores >= 8
```

**Lower worker thread OS priority below normal.**

```text
Windows → THREAD_PRIORITY_BELOW_NORMAL
Linux   → nice(+5)
macOS   → QOS_CLASS_UTILITY
```

**Never lower the priority of the playback, decode, render, or audio threads.**

Playback has priority over everything.

---

# 16. Preview Engine

```rust
enum PreviewQuality {
    Full,
    Half,
    Quarter,
    Auto,
}
```

Default: `Auto`.

---

# 17. Adaptive Preview Quality

Lower quality automatically when frames are missed.

```text
Target playback: 30 FPS

29–30 FPS → maintain quality
22–28 FPS → reduce preview resolution
<22   FPS → aggressive resolution reduction
```

When playback stops, render a higher-quality still frame.

```text
Playback    = performance optimized
Paused frame = quality optimized
```

Add hysteresis — do not change quality more than once per second, or the preview will visibly oscillate.

---

# 18. Frame Cache

LRU, byte-bounded.

```rust
struct FrameCache {
    max_bytes: usize,
}
```

```text
Default:              256 MB
Low-memory mode:      128 MB
High-memory systems:  512 MB+
```

Prioritize:

```text
Current frame
Next frames
Recent previous frames
```

---

# 19. Thumbnail Cache

Generate asynchronously. Never re-decode on timeline redraw.

```text
.cache/
└── media/
    └── MEDIA_ID/
        ├── thumbnails/
        ├── waveform.bin
        └── proxy/
```

Use hashed IDs so files with identical names do not conflict.

---

# 20. Audio Waveforms

Generate waveform summaries during import.

```rust
struct WaveformData {
    samples_per_second: u32,
    peaks: Vec<WaveformPeak>,
}

struct WaveformPeak {
    min: f32,
    max: f32,
}
```

Store multiple zoom levels (e.g. 1000, 100, 10 peaks/sec) so timeline zoom does not require recomputation.

---

# 20a. Audio Engine `[NEW SECTION]`

v1 listed an `audio/` crate but never specified it. Audio underpins A/V sync, waveforms, silence removal, beat detection, mixing, and export. It needs to be pinned down before playback is built.

## 20a.1 Audio Is The Master Clock

**Playback position is derived from the audio device, never from a video timer.**

```text
Audio callback consumes N samples
↓
Playback clock advances by N samples
↓
Video frame is selected to match the clock
↓
Late frames are dropped, not queued
```

Driving playback from a video timer produces drift that is extremely difficult to diagnose later. State this as a hard rule.

When a sequence has no audio, run a monotonic clock in its place with the same interface.

## 20a.2 Real-Time Thread Rules

The `cpal` output callback is a real-time thread. Inside it:

```text
MUST NOT allocate
MUST NOT lock a mutex
MUST NOT touch project state
MUST NOT perform I/O
MUST NOT panic
```

It reads from a lock-free ring buffer only. A separate mixer thread fills that buffer.

```text
Decode threads → resample → mixer thread → ring buffer → cpal callback
```

Ring buffer target: 100–200 ms. Underrun outputs silence and increments a counter; it never blocks.

## 20a.3 Format Normalization

**All audio is converted at import and held internally as:**

```text
Sample rate: 48 000 Hz
Format:      f32
Layout:      planar, deinterleaved
```

This is required by the §9 timebase and simplifies the mixer.

## 20a.4 Mix Graph

Summing order must be defined, because preview and export must match (§46):

```text
Clip sample
↓
clip gain  (keyframeable)
↓
clip effects
↓
track gain → track pan
↓
track sum
↓
master gain
↓
limiter (prevent clipping)
↓
output
```

Sum in f32. Clamp only at the final stage.

## 20a.5 A/V Sync Tolerance

```text
Video ahead of audio by  > 40 ms  → hold frame
Video behind audio by    > 40 ms  → drop frame(s)
Sustained drift          > 100 ms → log a warning
```

---

# 21. GPU Rendering

Use:

```text
wgpu
```

for:

* Video compositing.
* Scaling, rotation, cropping.
* Opacity.
* Color adjustments.
* Blur.
* Masks.
* Transitions.
* Text compositing.
* Effects.

**Change from v1:** GPU compositing is no longer deferred. Because the UI shares the wgpu device (§4), there is no integration cost to using it from the start, and the software path would have to be thrown away later.

Structure the compositor as a **render graph**: effects are nodes, not fixed pipeline stages. Templates and transitions then become data (§29) rather than code.

Keep a CPU fallback path only for machines where wgpu fails to initialize at all.

---

# 21a. Color Management `[NEW SECTION]`

§46 promises preview and export match. Without a stated color policy they will not. This is the classic "my export looks washed out" bug and it is nearly free to prevent now.

## 21a.1 Working Space

```text
v1 working space: sRGB-encoded, 8-bit RGBA
```

Adequate for the target hardware. A linear-light 16-bit float path can be added later behind the same interface; do not build it now.

## 21a.2 Conversion Happens At Upload

Every decoded frame is converted into the working space **at the texture-upload boundary**, exactly once. Nothing downstream inspects source color metadata.

Must be handled at that boundary:

| Property | Values seen in the wild | Action |
|---|---|---|
| Matrix | BT.601, BT.709, BT.2020 | Convert to sRGB primaries |
| Range | Limited (16–235), Full (0–255) | **Expand limited → full** |
| Transfer | sRGB, BT.1886, PQ, HLG | Tone-map PQ/HLG → SDR |
| Bit depth | 8, 10, 12 | Reduce to 8 with dithering |

**Range is the one that bites.** Most camera and phone H.264 is limited-range. Treating it as full range produces crushed blacks and clipped whites — and the error is small enough to be missed until a user compares against another player.

Default when metadata is absent:

```text
SD  (height <= 576)  → BT.601
HD+ (height >  576)  → BT.709
Range unspecified    → limited
```

## 21a.3 Export Path

Export uses the same conversion code as preview, then converts once more to the output color space. The conversion is a shader shared by both configurations.

Output default:

```text
BT.709, limited range, 8-bit
```

Tag the output file's color metadata explicitly. Untagged files get guessed at by players, which reintroduces the bug at the other end.

---

# 22. Rendering Pipeline `[EDIT]`

```text
Decode source frame
↓
Upload texture + color conversion   ← §21a
↓
Crop
↓
Transform
↓
Apply effects
↓
Composite track
↓
Composite overlays
↓
Composite text texture              ← §26, rasterized by core
↓
Output to preview texture OR encoder
```

Track order determines compositing order.

---

# 23. Effect System

```rust
struct EffectDefinition {
    id: EffectId,
    name: String,
    parameters: Vec<EffectParameterDefinition>,
}

enum EffectParameterType {
    Float,
    Integer,
    Boolean,
    Color,
    Enum,
    Point,
}
```

```json
{
  "id": "gaussian_blur",
  "name": "Blur",
  "parameters": [
    { "id": "amount", "type": "float", "min": 0, "max": 100, "default": 0 }
  ]
}
```

---

# 24. Keyframe System

```rust
struct Keyframe {
    time: TimelineTime,
    value: ParameterValue,
    interpolation: Interpolation,
}

enum Interpolation {
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
    Bezier,
}
```

Keyframe times use `TimelineTime` ticks (§9), never floats.

---

# 25. Transition System

```rust
struct TransitionInstance {
    id: TransitionId,
    duration: TimelineTime,
    parameters: HashMap<String, ParameterValue>,
}
```

Initial set:

```text
Crossfade
Fade to black
Slide
Push
Zoom
Blur transition
Flash
```

Avoid implementing dozens before the engine is stable.

**Moved to Phase 2** — see §59.

---

# 26. Text System `[EDIT — rendering ownership assigned]`

Required features:

```text
Font, size, weight, color
Alignment
Stroke, shadow, background
Position, scale, rotation, opacity
Letter spacing, line spacing
```

## 26.1 Text Is Rasterized By The Core Renderer `[NEW]`

**Text must be shaped and rasterized by the `text/` crate into a GPU texture, used identically by preview and export.**

Recommended:

```text
cosmic-text   — shaping, layout, line breaking, font fallback
swash         — glyph rasterization
```

**Why this rule exists.** If preview text is drawn by the UI toolkit and export text is drawn by something else, they will never match — different hinting, different subpixel positioning, different fallback fonts. §46 breaks in the most visible way possible, and users notice text before they notice anything else.

The UI layer only edits text *parameters*. It never rasterizes text that will appear in output.

Bundle fonts in `assets/fonts/`. Do not depend on system fonts for template text — a template must look the same on every machine.

---

# 27. Caption System

```rust
struct CaptionSegment {
    start: TimelineTime,
    end: TimelineTime,
    text: String,
    words: Vec<CaptionWord>,
}
```

Word-level timing enables:

```text
Karaoke captions
Word highlighting
Animated subtitles
TikTok-style captions
```

---

# 28. Automated Captions

```rust
trait TranscriptionProvider {
    fn transcribe(&self, audio: &Path) -> Result<Transcript>;
}
```

Implementations:

```text
Local model
Whisper-compatible local engine
Cloud provider
External plugin
```

On low-end laptops transcription is optional, or uses smaller models.

---

# 29. Template System

Categories:

```text
Text styles
Caption styles
Transitions
Effects
Video layouts
Intro templates
Outro templates
Social-media templates
Photo montages
Beat edits
Meme layouts
Color presets
Animations
```

---

# 30. Template Format

```json
{
  "schema_version": 1,
  "id": "social_fast_intro",
  "name": "Fast Social Intro",
  "category": "intro",
  "duration": 3.0,
  "slots": [
    { "id": "main_video", "type": "video" },
    { "id": "title", "type": "text" }
  ],
  "elements": []
}
```

Versioning is essential.

Note: template durations may use seconds as a float in the file format for authoring convenience. They are converted to `TimelineTime` ticks on load and are never used as authoritative timing (§9).

---

# 31. Template Slots

```text
Video slot
Image slot
Text slot
Audio slot
Logo slot
```

User flow:

1. Pick template.
2. Drop media into slots.
3. Replace text.
4. Export.

---

# 32. Automation Engine

Automation generates commands; it never mutates project data directly.

```text
Automation
↓
Generate commands
↓
Execute through normal editor
```

Ensures undo/redo work and that manual and automatic editing share one engine.

---

# 33. Initial Automation Features

```text
Remove silence
Detect scenes
Auto split
Auto resize
Normalize audio
Generate captions
Beat markers
Match cuts to beat
Auto slideshow
Auto crop
Create short clips from selected ranges
```

---

# 34. Silence Removal

```text
Analyze audio level
↓
Identify silent sections
↓
Apply minimum silence duration
↓
Create cut suggestions
↓
User previews
↓
Apply edits
```

Generates normal split/delete commands as one grouped command (§79).

---

# 35. Beat Detection

```rust
struct TimelineMarker {
    time: TimelineTime,
    marker_type: MarkerType,
}
```

```text
Beat
Scene
User marker
Speech
Automation marker
```

---

# 36. Auto Resize

```text
16:9   9:16   1:1   4:5
```

Presets:

```text
YouTube
YouTube Shorts
TikTok
Instagram Reels
Instagram Post
```

Updates canvas settings without modifying source media.

---

# 37. Project File Format

Human-readable JSON.

```text
.vproj
```

```json
{
  "version": 1,
  "project": {},
  "media": [],
  "sequences": []
}
```

Add migrations when schemas change.

Acceptable at MVP scale. Revisit if §52's 10,000-clip benchmark shows serialization exceeding ~100 ms; the migration path is a binary format behind the same `project-format` interface.

---

# 38. Autosave `[EDIT — atomic write + command journal]`

Autosave is mandatory.

## 38.1 Atomic Writes

**Never truncate the project file in place.** A crash mid-write destroys the project.

```text
1. Write to project.vproj.tmp
2. fsync
3. Rename project.vproj.tmp → project.vproj   (atomic on all target platforms)
```

## 38.2 Command Journal `[NEW]`

Rewriting the whole project on every autosave does not scale to §52's 10,000-clip benchmark.

Every edit is already a command (§10). Journal commands between full saves:

```text
Edit → append serialized command to journal (small, fast, append-only)
Every N commands or T seconds → full atomic snapshot, truncate journal
```

Recovery = last snapshot + replay journal. This gives near-zero-cost autosave *and* better crash recovery than debounced full writes: at most one command is lost instead of everything since the last debounce.

```text
project.vproj
recovery/
    snapshot.vproj
    journal.log
```

Commands must therefore be serializable (§11).

Still debounce full snapshots — do not snapshot on every mouse movement.

---

# 39. Crash Recovery

On launch:

1. Detect incomplete previous session.
2. Locate most recent snapshot.
3. Replay journal.
4. Offer project recovery.
5. Never overwrite the original project automatically.

---

# 40. Export Engine

Export uses original media, not proxies.

```rust
struct ExportRequest {
    sequence_id: SequenceId,
    output: PathBuf,
    preset: ExportPreset,
}
```

Presets:

```text
720p   1080p   1440p   4K
H.264  H.265   AV1 when supported
```

See §0.1 for encoder licensing constraints.

---

# 41. Hardware Encoding

Detect at runtime:

```text
NVIDIA NVENC
Intel Quick Sync
AMD AMF
Apple VideoToolbox
```

Always provide a software fallback (see §0.1 — **not x264**).

Do not fail export solely because hardware acceleration is unavailable.

---

# 42. Export Progress

```text
Frames completed
Total frames
Percentage
Estimated processing speed
Current encoding stage
```

Allow cancellation. Runs in a dedicated background task.

---

# 43. Performance Modes

```text
Performance | Balanced | Quality
```

Performance:

```text
540p proxies
Quarter preview
Small frame cache
Aggressive background throttling
```

Balanced:

```text
720p proxies
Adaptive preview
Medium cache
```

Quality:

```text
Higher preview resolution
Larger cache
Less aggressive proxy usage
```

---

# 44. Low-End Device Behavior

Automatically:

```text
Lower preview resolution
Limit background workers
Use proxies aggressively
Reduce thumbnail generation priority
Reduce memory cache size
Pause proxy generation during playback
Cap FFmpeg threads per job        ← §15.1
Avoid expensive realtime effects
```

Playback has priority over everything.

---

# 45. Effect Performance Classification

```rust
enum EffectCost {
    Cheap,
    Medium,
    Expensive,
}
```

```text
Opacity            → Cheap
Color adjustment   → Cheap
Blur               → Medium
Motion blur        → Expensive
AI segmentation    → Very expensive
```

In performance mode, expensive effects may use simplified preview implementations. Full quality is used during export.

---

# 46. Preview vs Export Rendering `[EDIT — one graph, two configs]`

**There is exactly one render graph implementation.** Preview and export are two *configurations* of it, differing only in:

```text
Resolution
Source media (proxy vs original)
Effect quality tier (§45)
Output sink (screen texture vs encoder)
```

Two separate renderer implementations will diverge. This is not a risk, it is a certainty.

```rust
struct RenderConfig {
    resolution: Resolution,
    media_source: MediaSourceMode,   // Proxy | Original
    effect_quality: QualityTier,
    output: RenderTarget,            // Texture | Encoder
}
```

Enforced mechanically by golden-frame tests (§51).

---

# 47. Lazy Processing

Prioritize media near the playhead.

```text
Playhead at 00:30 → prioritize 00:25–00:40
```

Prefetch nearby frames. Cancel obsolete work after large seeks.

---

# 47a. Seek Architecture `[NEW SECTION]`

§81 targets <150 ms seek but v1 never specified how. This is the single most visible performance characteristic in the product.

## 47a.1 Why Seeking Is Slow

Long-GOP media cannot be randomly accessed. Seeking to frame N means seeking to the preceding keyframe and decoding forward. On a 250-frame GOP that is up to 250 decodes.

**The primary fix is all-intra proxies (§13.1)** — one decode per seek instead of up to 250.

## 47a.2 Seek Types

```rust
enum SeekMode {
    Scrub,     // dragging playhead — lowest latency wins
    Precise,   // frame-accurate — correctness wins
    Playback,  // sequential — prefetch wins
}
```

**Scrub:** serve from frame cache if present. Otherwise decode at quarter resolution. Coalesce — if a newer scrub arrives while one is in flight, cancel the in-flight one and take the newest position. Never queue scrub requests; users drag faster than any decoder.

**Precise:** decode exact frame at current preview quality. Used for stop, arrow-key stepping, and split.

**Playback:** never seeks; reads sequentially from the decode-ahead ring buffer.

## 47a.3 Decode-Ahead Ring Buffer

```text
Target: 0.5–1.0 seconds of decoded frames ahead of the playhead
Bounded by byte count, not frame count
Backpressure: decode thread blocks when full — never grows unbounded
```

## 47a.4 Drop-Frame Policy

Playback must degrade, not stutter.

```text
Frame late by < 1 frame interval  → present anyway
Frame late by 1–3 intervals       → drop it, present the next
Sustained lateness                → reduce preview quality (§17)
Buffer underrun                   → hold last frame, never block audio
```

**Audio never stops.** Video drops to stay with audio (§20a.5).

## 47a.5 Cancellation

Every decode job carries a `CancellationToken`. Seeking from 00:30 to 12:00 cancels all decode work around 00:30 immediately — checked between frames, not only between jobs.

---

# 48. Cancellation

All heavy jobs support cancellation.

```rust
CancellationToken
```

Cancellation must be checked frequently enough to be responsive — inside decode loops, not only at job boundaries.

---

# 49. Logging

Structured logs (`tracing`).

```text
Timestamp
Module
Severity
Project ID
Media ID when relevant
Error message
```

Do not log sensitive file contents. Log file *paths* only at DEBUG.

```text
ERROR WARN INFO DEBUG TRACE
```

Production default: `INFO`.

Never log from the audio callback (§20a.2).

---

# 50. Error Handling

Avoid panics in application code. Use `Result` and structured error types.

A media decoding failure must not crash the editor:

```text
Mark clip unavailable
Show error message
Continue project session
```

---

# 51. Testing Strategy `[EDIT — golden frames added]`

Test the editor core independently of the UI.

Unit tests:

```text
Clip movement
Clip trimming
Clip splitting
Timeline snapping
Undo / Redo
Keyframe interpolation
Template parsing
Project save/load
Project migration
Proxy mapping
NTSC tick conversion round-trip   ← §9
Color range expansion             ← §21a
Command journal replay            ← §38
```

Integration tests:

```text
Import video
Create project
Add clip
Trim clip
Export sequence
Reload project
Crash recovery from journal
```

## 51.1 Golden-Frame Tests `[NEW]`

§46 is enforced by test, not by discipline.

```text
For a fixture project:
  Render frame N via preview config at full quality
  Render frame N via export config
  Assert per-pixel difference within tolerance
```

Cover at minimum:

```text
Plain clip
Clip with transform
Clip with color adjustment
Two composited tracks
Clip with text overlay          ← §26
Limited-range source            ← §21a
BT.601 source
NTSC frame-rate source          ← §9
```

Store references in `tests/golden/`. A golden-frame failure is a release blocker.

Also add golden **audio** tests: render 1 second through preview mix and export mix, assert sample-level match (§20a.4).

---

# 52. Performance Tests `[EDIT — reference machine required]`

Benchmarks:

```text
Timeline with 100 clips
Timeline with 1,000 clips
Timeline with 10,000 clips

Seek latency (scrub / precise)
Frame decode latency
Project serialization
Journal replay
Waveform generation
Thumbnail generation
Cold app startup
Idle CPU over 60 seconds
```

## 52.1 Reference Machine `[NEW]`

"Performance regressions are bugs" is unenforceable without a named machine. Record its exact specification here before Milestone 1:

```text
REFERENCE MACHINE — recorded 2026-08-08

CPU:              Intel Core i5-8250U (4 cores / 8 threads, 1.6 GHz base)
GPU:              Intel UHD Graphics 620 (integrated, no discrete GPU)
RAM:              8 GB DDR4-2400, dual channel
Storage:          256 GB SATA SSD
OS:               Windows 11
Display scaling:  125% @ 1920x1080

Status:           TARGET SPECIFIED, NOT YET VALIDATED
                  No machine of this class is currently available to benchmark
                  on. Every §81 number is therefore a design target, not a
                  measurement. See docs/reference-machine.md.
```

Requirements: 4 cores, integrated graphics, 8 GB RAM, SATA SSD. This machine is the arbiter for every number in §81. Benchmarks on the development machine do not count.

The development machine is a Ryzen 5 5600X / RTX 4060 / 32 GB — roughly an order of magnitude faster, and with no integrated GPU it cannot even approximate the reference machine's graphics path. Numbers from it are recorded as "dev machine" and never as §81 results.

---

# 53. Large Timeline Performance `[EDIT — canvas, not DOM]`

**The timeline is a custom-drawn canvas, not a widget tree.**

Draw with egui's `Painter` inside a single allocated response rect:

```text
Clip rectangles
Waveforms (from cached peaks, §20)
Thumbnails (from disk cache, §19)
Playhead
Track headers
Selection
Snap guides
```

Virtualize by viewport — compute the visible time range and draw only what intersects it.

```text
Project:  10,000 clips
Visible:  30 clips
Drawn:    ~30 clips + decorations
```

At the highest zoom-out level, do not draw per-clip detail. Draw a density summary instead; individual clips are sub-pixel and invisible.

Waveform and thumbnail draws must read from cache only. **A timeline repaint must never trigger a decode.**

---

# 54. State Management `[EDIT — rewritten for single process]`

The editor core owns the authoritative project state. This does not change in a single-process design — it is what makes undo, automation, and templates coherent.

```text
UI reads   → borrowed snapshot or query, per frame
UI writes  → commands only, never direct mutation
```

The UI must **never** mutate `Project` directly, even though Rust would allow it. Direct mutation bypasses undo and breaks §32.

Query by visible range rather than walking the whole project each frame:

```rust
fn clips_in_range(&self, track: TrackId, range: Range<TimelineTime>) -> &[VideoClip];
```

Keep a per-frame dirty-flag so egui repaints only when something changed. Idle CPU near zero is a §81 target.

---

# 55. Editor Commands API `[EDIT — in-process]`

The command set is unchanged; it is now an in-process Rust API rather than an IPC surface.

```text
project.create / open / save

media.import

timeline.add_clip / move_clip / trim_clip / split_clip / delete_clip

playback.play / pause / seek

effect.add / remove / set_parameter

export.start / cancel
```

Typed request/response structs. Keep this boundary clean even though it is in-process — it is what allows the UI to be replaced (§4.1) and what a future plugin API (§63) will build on.

---

# 56. Events `[EDIT — in-process channels]`

```text
playback.position_changed
playback.state_changed

job.started / progress / finished / failed

proxy.ready
media.imported
project.changed

export.progress / completed
```

Delivered over bounded channels. Throttle high-frequency events — playhead position updates at most once per UI frame, not per decoded frame.

---

# 57. Keyboard Shortcuts

```text
Space              Play/Pause
Ctrl/Cmd + Z       Undo
Ctrl/Cmd + Shift+Z Redo
Ctrl/Cmd + S       Save
Ctrl/Cmd + C       Copy
Ctrl/Cmd + V       Paste
Delete             Delete clip
S                  Split at playhead
Left / Right       Frame step
```

Eventually customizable.

---

# 58. Initial UI Layout

```text
┌────────────────────────────────────────────────────┐
│ Toolbar                                            │
├──────────────┬──────────────────────┬──────────────┤
│ Media        │                      │ Inspector    │
│              │       Preview        │              │
│ Templates    │  (wgpu TextureId)    │ Properties   │
│              │                      │              │
├──────────────┴──────────────────────┴──────────────┤
│                                                    │
│            Timeline (Painter canvas)               │
│                                                    │
└────────────────────────────────────────────────────┘
```

Keep the interface simple. Avoid exposing hundreds of controls simultaneously.

Because the preview is an egui texture rather than an overlay window, panels and modals may freely overlap it.

---

# 59. MVP Scope `[EDIT — narrowed]`

```text
Create / Open / Save project

Import video, image, audio

Timeline
  Add / Move / Trim / Split / Delete clips
  Multiple video tracks
  Multiple audio tracks

Playback / Pause / Seek
Audio playback with A/V sync

Basic transform
  Position, Scale, Rotation, Opacity

Audio volume

Proxy generation
Timeline thumbnails
Waveforms

Undo / Redo

1080p H.264 export
```

## Moved out of MVP `[CUT]`

```text
Text overlays   → Phase 2
Transitions     → Phase 2
```

**Why.** Each pulls in a real subsystem — text requires full shaping, fallback, and rasterization (§26); transitions require a timing model that overlaps two clips. Neither is required by the §60 success criteria. Cutting both gets to a validated core loop meaningfully sooner.

Do not implement AI-heavy features before this foundation works.

---

# 60. MVP Success Criteria `[EDIT]`

A user can:

1. Launch the application.
2. Import a video.
3. Drag it into the timeline.
4. Play it smoothly, with audio in sync.
5. Scrub the timeline without the UI stalling.
6. Split it.
7. Remove sections.
8. Add music.
9. Adjust volume.
10. Export a valid video.
11. Close the application.
12. Reopen the project.
13. Continue editing without project corruption.

All of this on the §52.1 reference machine.

---

# 61. Phase 2

```text
Text overlays          ← moved from MVP
Transitions            ← moved from MVP
Keyframes
More effects
Caption editor
Automatic transcription
Template system
Proxy improvements
Hardware encoding
Auto resize
Silence removal
Scene detection
Beat detection
```

Note: GPU compositing is no longer here — it is in the MVP (§21).

---

# 62. Phase 3

```text
Template marketplace
Plugin system
Advanced captions
Animated subtitles
Motion tracking
Background removal
AI segmentation
Auto highlights
Smart reframing
Object tracking
Advanced color tools
Nested sequences
Adjustment layers
Compound clips
```

---

# 63. Plugin Architecture

Do not build plugins immediately. Avoid architecture that makes them impossible.

Eventually:

```text
Effects, Transitions, Templates, Automation, Exporters, AI providers
```

Plugins must never get unrestricted system access by default.

---

# 64. Security

Treat downloaded templates as untrusted.

Templates contain data, not executable code.

```text
Bad:  template contains JavaScript
Bad:  template executes shell command
Good: template describes approved animation/effect operations
```

Validate every template before loading.

Additionally: templates must not reference absolute paths outside the template bundle, and bundled assets must be size-capped before decode.

---

# 65. Template Validation

```text
Schema version
Required fields
Effect IDs
Parameter ranges
Media paths (must be bundle-relative)
Resource sizes
Unsupported operations
```

Reject malformed templates without crashing.

---

# 66. File Paths

Do not rely solely on absolute paths.

Store:

```text
absolute path
filename
size
optional hash
metadata
```

When media is missing:

```text
Locate file
Locate folder
Auto relink matching media
```

---

# 67. Cache Management

Configurable maximum size.

```text
5 GB   10 GB   20 GB
```

Contents:

```text
Proxies, Thumbnails, Waveforms, Preview renders, Analysis results
```

LRU cleanup. Never delete source media.

Warn before generating proxies that would exceed the cache limit — proxies are large (§13.1).

---

# 68. Memory Rules

Avoid holding full videos in memory. Frames have clear ownership and short lifetimes. Use bounded queues.

```text
Bad:  Decode hundreds of frames and keep all of them.
Good: Decode a small frame window. Cache relevant frames. Evict old frames.
```

---

# 69. Threading Rules `[EDIT]`

Separate:

```text
UI (egui)
Audio callback (real-time — §20a.2)
Audio mixer
Playback coordination
Decode
Render submission
Background jobs
Export
```

Priorities:

```text
Priority 0   Audio callback (real-time, never preempted)
Priority 1   Playback / decode / render
Priority 2   User-triggered operations
Priority 3   Export
Priority 4   Proxy generation
Priority 5   Thumbnail / background analysis
```

Export priority may be configurable.

Do not allow proxy generation to starve playback (§15.1).

---

# 70. Avoid Premature Complexity

Do not initially implement:

```text
Custom video codec
Custom audio codec
Custom scripting language
Distributed rendering
Cloud projects
Collaborative editing
Marketplace backend
Advanced node compositor
Full After Effects competitor
Linear-light float color pipeline    ← see §21a.1
```

Focus on the core editing loop first.

---

# 71. Coding Standards

Rust:

```text
rustfmt
clippy  (deny warnings in CI)
cargo test
```

Avoid `unwrap()` / `expect()` in production paths unless failure is truly impossible and documented.

Deny `unwrap()`, allocation, and locking in the audio callback — enforce by review (§20a.2).

---

# 72. Documentation Rules

Every major crate contains `README.md` documenting:

```text
Purpose
Public interfaces
Threading assumptions
Data ownership
Error behavior
Performance assumptions
```

ADRs:

```text
docs/adr/
001-rust-core.md
002-ffmpeg-backend.md
003-project-timebase.md          ← record the 960,000 rationale
004-proxy-system.md              ← record the all-intra rationale
005-ui-shell.md                  ← NEW
006-color-management.md          ← NEW
007-encoder-licensing.md         ← NEW
```

---

# 73. Coding Agent Rules

## Before Coding

1. Read this document.
2. Inspect existing project structure.
3. Reuse existing abstractions.
4. Avoid introducing redundant frameworks.
5. Determine which module owns the requested behavior.

## While Coding

* Keep changes focused.
* Avoid unrelated refactoring.
* Maintain backwards compatibility when practical.
* Write tests for logic-heavy changes.
* Avoid blocking the UI thread.
* Avoid unnecessary allocations in hot paths.
* Avoid copying large frame buffers.
* Prefer streaming and references.
* Preserve non-destructive editing.
* Use typed interfaces.
* Handle errors explicitly.

## After Coding

1. Run formatting.
2. Run linting.
3. Run unit tests.
4. Run relevant integration tests.
5. **Run golden-frame tests if the renderer, effects, color, or text changed.**
6. Report changed files.
7. Explain architectural changes.
8. Mention known limitations.
9. Mention any performance implications.

---

# 74. Agent Must Not `[EDIT — two rules added]`

```text
Rewrite the entire architecture to implement one feature.

Introduce a second renderer implementation.               ← NEW (§46)

Use floating-point in timeline position arithmetic.       ← NEW (§9)

Process raw frames outside the renderer crate.

Download frames to system RAM in the hot path.

Use Python for realtime video rendering.

Block the UI while FFmpeg runs.

Allocate, lock, or log inside the audio callback.

Store full videos in RAM.

Modify source media.

Hard-code hundreds of templates.

Create unlimited background threads.

Let FFmpeg use all CPU cores in a background job.

Silently ignore FFmpeg failures.

Link x264 or any GPL component.

Rasterize output text in the UI layer.

Add large dependencies without justification.
```

---

# 75. Feature Implementation Template

```text
Requirement   — what the user should be able to do
Data Model    — which structures change
Core Logic    — editor behavior independent of UI
Commands      — reversible commands where project data changes
Core API      — expose required core commands
UI            — interface controls
Persistence   — save/load support (and migration if schema changed)
Undo/Redo     — edits are reversible
Performance   — does this belong in a worker?
Tests         — unit, integration, golden-frame where visual
```

---

# 76. Example: Implement Split Clip

```text
User places playhead inside clip and presses S.
```

```text
UI → timeline.split_clip → SplitClipCommand → Timeline core
```

Original:

```text
[------------- clip -------------]
                ^
             playhead
```

Result:

```text
[---- clip A ----][---- clip B ----]
```

Media is not duplicated:

```text
Clip A source: 0 → split point
Clip B source: split point → original end
```

The split point must be snapped to a frame boundary in `TimelineTime` ticks (§9) before the command is constructed. With the 960,000 timebase this is exact for all supported frame rates.

Undo restores the original clip.

---

# 77. Example: Implement Template Application

```text
Template file
↓
Template validator
↓
Template engine
↓
Generate editor commands
↓
Timeline engine
```

The template engine must not directly mutate project internals. Applied as one `CommandGroup` (§79) so it is a single undo.

---

# 78. Example: Auto Silence Removal

```text
User selects clip
↓
Extract/analyze audio
↓
Detect silent ranges
↓
Show suggested cuts
↓
User confirms
↓
Generate split/delete commands as one group
↓
Timeline updates
```

---

# 79. Grouped Commands

```rust
struct CommandGroup {
    commands: Vec<Box<dyn EditorCommand>>,
}
```

For:

```text
Applying template
Auto silence removal
Beat edits
Multi-delete
Paste multiple clips
```

Undo reverses the entire operation.

---

# 80. Feature Priority

```text
1. Stability
2. Timeline responsiveness
3. Playback performance
4. Project reliability
5. Export reliability
6. Basic editing
7. Templates
8. Automation
9. Effects
10. AI features
```

Do not reverse this priority.

A video editor with 300 effects but unreliable playback is not acceptable.

---

# 81. Performance Targets

Measured on the §52.1 reference machine.

```text
Application startup            < 3 s
Timeline interaction           60 FPS
Scrub response (proxied)       < 100 ms
Precise seek (proxied)         < 150 ms
Play/pause                     near-instant
1080p proxy playback           stable real-time
A/V sync drift                 < 40 ms sustained
Idle CPU                       < 1%
Idle repaint rate              ~0 FPS (reactive)
Memory, small project          < 500 MB
```

Goals rather than absolute guarantees — but tracked as benchmarks (§52), and regressions are bugs.

---

# 82. Development Roadmap `[EDIT — Milestone 0 added]`

## Milestone 0 — Frame Transport Spike `[NEW]`

**Throwaway code. No product architecture. Do this first.**

```text
Hardware-decode a 1080p H.264 file
Composite two layers with a wgpu shader
Display at 60 fps inside an egui window
Play synchronized audio via cpal
```

Run it on the §52.1 reference machine.

Definition of done:

```text
Sustained 60 fps
No RAM round trip per frame on at least one platform
Measured: decode time, upload time, composite time, present time
Recorded: egui text quality judgement (§4.1)
```

**This spike can invalidate the rest of the plan.** If frame transport cannot be made to work in the chosen shell, the shell changes (§4.1) — and it is far cheaper to learn that now than at Milestone 8.

Record results in `docs/adr/005-ui-shell.md`. Then delete the spike.

---

## Milestone 1 — Application Skeleton

```text
Rust workspace
eframe application shell
Command / event plumbing
Logging (tracing)
Settings
Basic project model
Reference machine recorded (§52.1)
```

Done when:

```text
App launches.
UI can dispatch commands to core.
Core can emit events to UI.
New / Save / Open project works with versioned JSON.
```

---

## Milestone 2 — Media Import

```text
FFmpeg discovery
Metadata extraction (including color metadata, §12)
Media library
Thumbnails
```

Done when: user can import common video/audio/image files.

---

## Milestone 3 — Timeline

```text
Tracks, Clips
Move, Trim, Split, Delete
Selection, Zoom, Snapping
Canvas-rendered timeline (§53)
```

Done when: basic editing works without playback.

---

## Milestone 4 — Playback

```text
Decoder
Audio engine (§20a) — master clock, ring buffer, mixer
Seek architecture (§47a)
Frame cache
Color conversion at upload (§21a)
A/V synchronization
```

Done when: timeline plays correctly with audio in sync, and scrubbing is responsive.

---

## Milestone 5 — Persistence

```text
Save / Open
Atomic writes (§38.1)
Command journal (§38.2)
Crash recovery
Missing media handling
```

Done when: projects survive application restarts and simulated crashes.

---

## Milestone 6 — Export

```text
[BLOCKED ON §0.1 LEGAL REVIEW]

Timeline render via export config (§46)
Audio mix
Encoding (OS encoders preferred)
Progress, Cancellation
Golden-frame tests (§51.1)
```

Done when: edited timeline exports correctly and golden-frame tests pass.

---

## Milestone 7 — Proxy Workflow

```text
All-intra proxy generation (§13.1)
Proxy selection
Adaptive preview (§17)
Cache management
Thread caps (§15.1)
```

Done when: high-resolution source media becomes usable on the reference machine.

---

## Milestone 8 — Effects and Keyframes

```text
Transform, Opacity, Basic color, Blur
Keyframes
Render graph node structure
```

---

## Milestone 9 — Text and Transitions

```text
Text rasterization in core (§26.1)
Text overlays
Transitions
```

---

## Milestone 10 — Captions

```text
Subtitle tracks
Caption styling
Caption import/export
```

---

## Milestone 11 — Templates

```text
Template schema
Template validator
Template browser
Template slots
Template installation
```

---

## Milestone 12 — Automation

```text
Silence removal
Scene detection
Beat markers
Auto resize
Automatic captions
```

---

# 83. First Coding-Agent Assignment `[EDIT]`

**Milestone 0 (§82) must be completed and its ADR written before this assignment begins.**

```text
Create the initial application architecture.

Requirements:

1. Create a Rust workspace.

2. Create separate crates for:
   - editor-core
   - timeline
   - media
   - project-format
   - ui

3. Create an eframe desktop application in apps/desktop.

4. Establish a typed command/event boundary between the UI
   crate and editor-core. The UI must not mutate Project directly.

5. Implement application logging with tracing.

6. Implement a minimal Project structure.

7. Implement TimelineTime with a 960,000 tick/second timebase,
   with conversion helpers to and from FFmpeg rationals.

8. Implement:
   - New Project
   - Save Project (atomic write, §38.1)
   - Open Project

9. Store projects as versioned JSON.

10. Add unit tests for:
    - project serialization/deserialization
    - TimelineTime conversion round-trips at
      23.976, 24, 25, 29.97, 30, 50, 59.94, 60 fps

Do not implement FFmpeg, playback, or advanced UI yet.

Keep all video-specific interfaces extensible for later integration.
```

---

# 84. Second Coding-Agent Assignment

```text
Implement media importing and metadata discovery using FFmpeg.

Requirements:

- Wrap FFmpeg behind the media crate's own traits.
  Nothing outside the media crate references rsmpeg types.
- Import video, audio, images.
- Read metadata including color primaries, transfer,
  matrix, and range (§12).
- Generate stable media IDs.
- Store source paths.
- Detect missing media.
- Add media assets to project.
- Build media browser UI.
- Do not implement video playback yet.
```

---

# 85. Third Coding-Agent Assignment

```text
Implement the timeline data model and editing commands.

Required operations:
- Create / delete track
- Add / Move / Trim / Split / Delete clip
- Undo / Redo

The implementation must be independent of the UI crate.

All timing in TimelineTime ticks. No floats.

Add comprehensive unit tests, including frame-boundary
snapping at NTSC rates.
```

---

# 86. Architecture Rule `[EDIT]`

```text
UI (egui)
↓
Application
↓
Editor Core
↓
Media / Timeline / Effects / Text / Audio
↓
FFmpeg / wgpu / OS
```

Lower layers must never depend on the UI.

For example, the `timeline` crate must not know that egui exists.

The `media` crate is the only crate that references the FFmpeg binding.

---

# 87. Final Product Direction

```text
CapCut simplicity
+
desktop performance
+
template-driven editing
+
low-end hardware optimization
+
open/extensible architecture
```

The competitive advantage is not:

```text
We have the most effects.
```

It is:

```text
Editing feels instant.
Templates save time.
Automation eliminates repetitive work.
The editor still works well on inexpensive hardware.
```

---

# 88. Final Engineering Principle

When trading between:

```text
more features
```

and:

```text
smooth timeline + reliable editing
```

choose:

```text
smooth timeline + reliable editing
```

The editor must feel fast before it feels powerful.

---

# 88a. Packaging and Distribution `[NEW SECTION]`

v1 had no section on this, but it affects Milestone 2 — FFmpeg has to reach the user's machine somehow.

## FFmpeg Distribution

```text
Bundle FFmpeg shared libraries with the application.
Do not require a system install.
Do not shell out to an ffmpeg binary in the hot path.
```

Link dynamically against LGPL FFmpeg. LGPL requires that the user be able to replace the library — dynamic linking satisfies this; static linking creates obligations you do not want (§0.1).

Ship the license texts of every bundled dependency in the installer.

## Per-Platform

```text
Windows   MSI or NSIS. Code signing certificate required or
          SmartScreen will block downloads.
macOS     .app in a signed, notarized DMG. Notarization is
          mandatory on current macOS.
Linux     AppImage as primary (bundles FFmpeg cleanly).
          Flatpak second.
```

## Updates

Defer to Phase 2, but do not architect against it. A delta updater over signed releases is sufficient; do not build an update server before the product works.

## Build Reproducibility

Pin FFmpeg version and build flags in a script under `docs/`. "Which FFmpeg build was this?" must be answerable from the repository, not from a developer's memory.

---

# 89. Open Questions `[NEW]`

Items deliberately unresolved. Record decisions as ADRs when made.

```text
1. Encoder licensing posture — blocking Milestone 6 (§0.1)
2. Reference machine specification (§52.1)
3. egui text quality acceptable? — answered by Milestone 0 (§4.1)
4. Hardware decode interop coverage per platform — Milestone 0
5. Binary project format threshold — revisit after §52 benchmarks
6. Whether to support variable frame rate source without proxying
```