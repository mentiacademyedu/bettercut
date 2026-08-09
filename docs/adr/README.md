# Architecture Decision Records

§72 lists the ADRs this project keeps. Current state:

| ADR | Subject | Status |
|---|---|---|
| [002](002-ffmpeg-backend.md) | FFmpeg backend and binding | **Accepted** — implemented |
| [003](003-project-timebase.md) | 960,000-tick timebase | **Accepted** — implemented |
| [004](004-proxy-system.md) | All-intra proxy encoding | **Accepted** — spec only, Milestone 7 |
| [005](005-ui-shell.md) | egui + wgpu shell | **Accepted** — validated by the Milestone 0 spike |

## Not yet written

These are listed in §72 but have nothing to record until the decision is actually
made. Writing them now would be fabricating a rationale.

| ADR | Subject | Blocked on |
|---|---|---|
| 001 | Rust core | Nothing — §3 states it; write when there is a trade-off worth recording |
| 006 | Colour management | Milestone 4. §21a states the policy; the ADR records what the shader actually does once it exists |
| 007 | Encoder licensing | **Blocks Milestone 6.** See §0.1 — needs legal review, not an engineering decision |

## Open questions (§89)

1. Encoder licensing posture — blocking Milestone 6 (§0.1). ADR 002 settles the
   *software licence* half (LGPL, no GPL components); the **patent** half is
   untouched and still needs legal review
2. Reference machine (§52.1) — **specified** (i5-8250U / UHD 620 / 8 GB / SATA
   SSD, see [reference-machine.md](../reference-machine.md)) but **not
   validated**: no such machine is available to benchmark on, so every §81
   number remains a design target rather than a measurement
3. ~~egui text quality acceptable?~~ — answered in ADR 005: yes
4. Hardware decode interop coverage per platform — still open; the Milestone 0
   spike validated the compositor path but not decoder→texture interop
5. Binary project format threshold — revisit after §52 benchmarks
6. Whether to support variable frame rate source without proxying
