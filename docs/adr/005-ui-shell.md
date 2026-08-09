# ADR 005 — UI Shell

**Status:** Accepted (provisionally — see "Outstanding" below)
**Date:** 2026-08-08
**Decides:** §4, §4.1, §5
**Evidence:** Milestone 0 frame-transport spike (§82), `spike/frame-transport/`

---

## Decision

**Keep `eframe` / `egui` + `wgpu` as the application shell.** Do not reverse to
Slint, GPUI, or Qt/QML. Do not reverse to Tauri under any circumstances.

---

## What the spike tested

§82's definition of done, item by item:

| Requirement | Result |
|---|---|
| Composite two layers with a wgpu shader | ✅ two 1080p RGBA layers, alpha-blended in `composite.wgsl` |
| Display at 60 fps inside an egui window | ✅ **707 fps** sustained; see caveat on vsync below |
| Play synchronized audio via cpal | ✅ 48 kHz stereo, audio-derived clock |
| Measured: decode / upload / composite / present | ✅ table below |
| Recorded: egui text quality judgement | ✅ acceptable; see below |
| No RAM round trip per frame on ≥1 platform | ⚠️ **not tested** — needs FFmpeg |
| Hardware-decode a 1080p H.264 file | ⚠️ **not tested** — needs FFmpeg |

---

## The central result

**The §5 path works, and it is trivial.** The compositor renders into a
`wgpu::Texture`; `egui-wgpu`'s `register_native_texture()` returns a
`TextureId`; `Painter::image()` draws it. That is the entire frame transport.
No interop layer, no frame copy, no IPC, no overlay-window geometry management.

The spike also demonstrates the property §4.1 claims and Tauri could not offer:
an `egui::Window` was placed **on top of the live preview texture** and
composites correctly with it.

---

## Measurements

Two 1080p layers, 7.91 MB/frame, 30 fps video inside an uncapped UI loop.

```text
                     avg      p99    worst
ui frame            1.41     4.25     4.33   ms
frame generate      1.34     2.77     4.16   ms
texture upload      0.82     1.44     1.63   ms   <- the software fallback path
composite submit    0.09     0.19     0.22   ms
                    ui fps  707.6
                    A/V drift  -5.5 ms   (§20a.5 limit ±40)
                    dropped frames  2 (both at startup)
                    audio underruns 0
```

`upload` is the cost of the §5 **forbidden** path — RAM → GPU per frame — so it
is the pessimistic bound. 0.82 ms for 7.91 MB ≈ 9.6 GB/s. Even the software
fallback leaves ~15 ms of headroom in a 16.6 ms frame on this machine.

---

## Text quality judgement (§4.1)

**Acceptable.** At 11/13/16/22 px, both proportional and monospace, glyphs are
crisp and evenly spaced; the numeric HUD columns align. This is the criterion
§4.1 said would decide whether the shell changes, and it does not.

Two known egui weaknesses remain real and unmeasured by this spike: **IME
support** and **native menus**. Neither is on the §59 MVP path. Revisit before
Milestone 10 (captions), where non-Latin text entry starts to matter.

Note this judgement covers *UI chrome only*. §26.1 already requires that text
appearing in **output** be rasterized by the `text/` crate, never by egui.

---

## Caveats — read these before trusting the numbers

### 1. This was NOT the reference machine

```text
Spike ran on:   NVIDIA GeForce RTX 4060, discrete, Vulkan, driver 610.88
§52.1 requires: 4 cores, integrated graphics, 8 GB RAM, SATA SSD
```

§52.1 is explicit: *"Benchmarks on the development machine do not count."*
Every number above is from a machine roughly an order of magnitude faster than
the target. They prove the **path works**; they prove nothing about §81.

A game was also running on the same GPU during part of the measurement, which
inflates the p99/worst columns rather than deflating them.

### 2. Hardware decode → texture interop is untested

The hardest item in §5 is still open, because FFmpeg is not installed on this
machine. The spike validates *compositor → egui → screen*; it does not validate
*hardware decoder → texture*. §5's difficulty table still stands unverified:

```text
D3D11 ↔ Vulkan   → difficult      <- this is the Windows path we will need
VAAPI ↔ Vulkan   → difficult
```

This is deferred to the Milestone 2/4 timeframe, not cancelled. The software
fallback measured here is what protects us if interop proves impractical.

### 3. The UI loop is not frame-rate limited

707 fps means eframe is not vsync-capped in this configuration, and the spike
re-composites on *every* UI repaint — 3460 repeated composites for 208 distinct
video frames. On a thermally limited laptop that is wasted power.

**Carry into the product:** composite only when the video frame or a parameter
actually changes, and cap the repaint rate to the display refresh. This is the
same dirty-flag requirement §54 already states.

---

## Consequences

* §4's stack stands: `eframe` + `egui` + `wgpu` + `cpal` + `rfd`, one process,
  one wgpu device.
* §7's repository structure stands, including `crates/ui/`.
* The `Compositor` shape in the spike (offscreen texture + registered
  `TextureId`) is the shape `crates/renderer/` should take — but the spike code
  itself is throwaway and must not be lifted into the product.
* §46's "one render graph, two configurations" is unaffected: the spike's
  render pass writes to a texture; the export configuration will point the same
  pass at an encoder input.

---

## Outstanding

1. **Re-run the spike on the §52.1 reference machine.** Until then §81's targets
   are unvalidated. The spike is kept in `spike/frame-transport/` for exactly
   this reason — §82 says to delete it, and it will be deleted once that run is
   recorded here.
2. **Record the reference machine specification** in §52.1 (blocks Milestone 1's
   definition of done).
3. **Test hardware decode → texture interop** once FFmpeg is available.
