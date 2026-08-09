# ADR 003 — Project Timebase

**Status:** Accepted
**Date:** 2026-08-08
**Decides:** §9
**Implemented by:** `crates/foundation/src/time.rs`

---

## Decision

Timeline positions are `i64` ticks at **960,000 ticks per second**. Floating-point
seconds never appear in timeline position arithmetic (§74).

---

## Why not floating point

A `f32` has 24 bits of mantissa. At one hour it cannot resolve a millisecond.
Positions drift, two clips that should butt-join develop a sub-frame gap, and the
gap shows up as a one-frame flash at the cut. `f64` postpones the problem without
removing it: repeated add/subtract during trimming still accumulates error, and
equality comparisons stop meaning anything.

## Why not 1,000,000 ticks/second

Microseconds look like the obvious choice and are wrong for NTSC.

```text
29.97 fps = 30000/1001
one frame = 1,000,000 × 1001 / 30000 = 33366.666…   ← not an integer
```

Every frame boundary rounds. The error accumulates, and split-at-playhead lands
off-by-one deep into a long clip. NTSC rates are not an edge case — phones and
screen recorders default to 29.97 and 59.94.

## Why 960,000

It divides exactly by every rate we support:

| Rate | Ticks per frame |
|---|---|
| 23.976 (24000/1001) | 40,040 |
| 24 | 40,000 |
| 25 | 38,400 |
| 29.97 (30000/1001) | 32,032 |
| 30 | 32,000 |
| 50 | 19,200 |
| 59.94 (60000/1001) | 16,016 |
| 60 | 16,000 |
| 120 | 8,000 |

It also divides by 48,000, so one audio sample at 48 kHz is exactly 20 ticks.

`i64` at this rate covers roughly ±304,000 years.

---

## Consequences

### Mandatory: all audio resamples to 48 kHz at import

960,000 does **not** divide by 44,100. This is not a limitation to work around —
it is a requirement the mixer has anyway (§20a.3). Recorded here because a future
reader will otherwise try to "fix" the timebase to accommodate 44.1 kHz.

### Conversions refuse rather than round

`TimelineTime::from_frames` returns `None` when a frame rate does not divide the
timebase, and `Sequence::new` rejects such a rate outright. Rounding silently is
the failure this ADR exists to prevent, so the API does not offer it.

### FFmpeg timestamps convert through a rational, never a float

`from_timebase`/`to_timebase` take a `Rational` and use an `i128` intermediate so
a fine timebase over a long duration cannot overflow before the division.

**Known precision limit:** a timebase that does not divide 960,000 evenly is
representable only on a stride. MPEG-TS (1/90000) has a stride of 3, because one
90 kHz unit is 10⅔ ticks. Real frame timestamps land on the stride — a 29.97 fps
frame is 3003 units at 90 kHz, and 3003 is a multiple of 3 — so this does not
bite in practice. Sub-stride timestamps truncate toward zero; the finest
representable step is ~1.04 µs. Asserted in
`time::tests::timestamps_between_ticks_truncate`.

---

## Enforcement

* `foundation::time` tests assert the whole table above, plus frame round-trips
  at every supported rate over a million frames.
* `ntsc_does_not_drift_over_an_hour` asserts an exact tick count an hour in.
* `forty_four_one_khz_does_not_divide_the_timebase` fails loudly if the timebase
  is ever changed without revisiting this decision.
* `ntsc_frame_rate_survives_persistence_exactly` (project-format) asserts a rate
  still divides the timebase after a save/load round trip — the check that would
  catch someone changing `FrameRate` to serialize as a float.
