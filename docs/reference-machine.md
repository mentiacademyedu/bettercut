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
table describes without the user configuring anything (§44).

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
