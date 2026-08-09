# Reference machine (§52.1)

**Status: target specified, not yet validated.**

```text
CPU:              Intel Core i5-8250U (4 cores / 8 threads, 1.6 GHz base)
GPU:              Intel UHD Graphics 620 (integrated, no discrete GPU)
RAM:              8 GB DDR4-2400, dual channel
Storage:          256 GB SATA SSD
OS:               Windows 11
Display scaling:  125% @ 1920x1080
```

A typical 2018 office laptop. This is the §1 target — *"8 GB RAM, integrated
graphics, older 4-core CPUs, SATA SSDs"* — and the class of machine the product
exists to serve. Hitting §81 here is the differentiator; hitting it on a gaming
PC proves nothing.

---

## What "not yet validated" means

No machine of this class is available to benchmark on. So:

* **Every number in §81 is a design target, not a measurement.** Nothing may be
  reported as "meets §81" until it has been measured here.
* **§52's benchmarks are not yet meaningful.** They can run on the development
  machine to catch gross regressions, but their absolute values are noise.
* **The Milestone 0 spike is deliberately kept** in `spike/frame-transport/`.
  §82 says to delete it after ADR 005 is written; it stays until it has been run
  on real reference hardware, because it is the only thing that measures the
  frame-transport path end to end.

## The development machine, for contrast

```text
Ryzen 5 5600X (6C/12T) · RTX 4060 (discrete) · 32 GB · NVMe
```

Roughly an order of magnitude faster. It also has **no integrated GPU**, so it
cannot approximate the reference machine's graphics path even with cores and
storage throttled — and the GPU is where the risk actually lives (wgpu
compositing, hardware decode → texture interop, memory bandwidth).

Partial approximation is possible for CPU and I/O — 4-core affinity, media on a
SATA SSD, a capped frame cache — and is worth doing to catch gross regressions.
It is not a substitute, and results from it are labelled "dev machine".

## The integrated-GPU machine

```text
Intel Core i7 (13th gen) · integrated graphics · 16 GB · Windows
```

**The first machine with integrated graphics**, and therefore the first that
exercises the half of the risk the dev machine cannot reach at all. Everything
graphical this project has ever measured came from a discrete RTX 4060.

What it **does** validate:

* wgpu initialises and picks a sane adapter and backend on Intel graphics.
* §21's compositing path — including the sRGB view distinction, which is a
  driver-visible detail and was wrong until recently.
* egui text rendering and panel layout at this display's real scaling (ADR 005
  accepted these on the dev machine only).
* §50's behaviour if wgpu fails: the editor must start anyway and say why.

What it **does not** validate:

* **Anything CPU-bound.** A 13th-gen i7 has roughly three times the reference
  machine's multi-core throughput. Proxy generation, journal replay and
  timeline operations will all look far better than they will on the target.
* **Memory pressure.** 16 GB against §81's budget on an 8 GB machine.
* **Storage.** Almost certainly NVMe, not the target's SATA SSD.

One trap worth knowing before reading any result from it: `HardwareProfile` is
**CPU-only**, so this machine reports enough cores to land in `Quality` mode —
720p proxies, the largest cache, least throttling — while the reference machine
lands in `Performance`. Left alone it therefore exercises *different defaults*
than the target does. To test the target's configuration, set **Inspector →
Proxies → Quality** to *Smoothest* by hand.

That CPU-only profile is deliberate rather than an oversight: proxy resolution,
job counts and FFmpeg thread caps are all decisions about CPU work, and §16/§17
handle the GPU adaptively at run time — preview starts at quarter resolution and
climbs only if frames arrive on time. If this machine shows the preview stuck at
a low tier, that is the adaptive system working, and the number worth reporting.

### What to capture

1. `RUST_LOG=info cargo run -p bettercut-desktop` — the first few lines record
   the detected profile and the chosen adapter, backend and driver.
2. The **Inspector → System** section, which shows the same thing on screen:
   processors, mode, job and thread caps, cache size, GPU name, kind, backend
   and driver.
3. Whether import, playback, scrubbing and proxy generation complete at all —
   correctness before speed. Timings from here are indicative, not §81 results.

---

## What the target already changes in the code

Naming the machine is not paperwork; several defaults derive from it directly.

| Consequence | Where | Rule |
|---|---|---|
| 1 heavy background job, never more | `PerformanceMode::max_heavy_jobs` | §15: cores ≤ 4 → 1 |
| FFmpeg capped at 1 thread per background job | `PerformanceMode::ffmpeg_threads_per_job` | §15.1 — one uncapped job saturates 4 cores regardless of scheduler limits |
| 540p proxies in Performance mode | `PerformanceMode::proxy_resolution` | §13 |
| 128 MB frame cache in Performance mode | `PerformanceMode::frame_cache_bytes` | §18, against §81's <500 MB budget |
| All-intra proxies | ADR 004 | §13.1 — the only way one decode per seek is possible on this CPU |
| Software frame-transport fallback must exist | ADR 005 | §5 — UHD 620 interop is the "difficult" column |

`HardwareProfile::detect()` reads the actual core count at startup and picks
defaults from it, so running on a 4-core machine automatically behaves as this
table describes without the user configuring anything (§44). It does **not**
read the GPU — see the note in the integrated-GPU section above for why, and for
what that means when reading results from a machine with a fast CPU and slow
graphics.

---

## Runbook — when hardware becomes available

Do these in order and record the results here.

1. **Milestone 0 spike.** Build and run `spike/frame-transport/`. It prints a
   timing table every 2 s. Capture at least 60 s of steady state.
   Definition of done (§82): sustained 60 fps, and measured decode / upload /
   composite / present times.
2. **Judge egui text quality on this display at 125% scaling.** ADR 005 accepted
   it based on the dev machine; scaling and panel type both affect it.
3. **§52 benchmarks:** cold startup, idle CPU over 60 s, timeline with
   100 / 1,000 / 10,000 clips, project serialization, journal replay.
4. **§81 targets:** startup < 3 s, timeline 60 fps, scrub < 100 ms, precise seek
   < 150 ms, A/V drift < 40 ms, idle CPU < 1%, memory < 500 MB.
5. Update ADR 005's "Outstanding" section, then **delete the spike**.

If the numbers miss §81, the response is to fix the code or revisit the ADRs —
not to move the target. §80 puts timeline responsiveness and playback
performance above every feature, and §88 says the editor must feel fast before
it feels powerful.
