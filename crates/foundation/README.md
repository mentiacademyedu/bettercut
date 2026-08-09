# bettercut-foundation

## Purpose

The primitives every other crate needs and nobody should redefine: exact time
representation (§9), rational frame rates, and entity IDs.

## Why this crate exists (deviation from §7)

§7 and §83 do not list a `foundation` crate. It exists because the alternatives
are worse:

* `timeline` owns `VideoClip`, which references a `MediaId`.
* `media` owns `MediaAsset`, whose durations are `MediaTime`.

Either direction of dependency between `timeline` and `media` creates a cycle,
and putting `MediaId` in `timeline` or `TimelineTime` in `media` misassigns
ownership. A leaf crate holding only shared primitives resolves it without
weakening §86's layering:

```text
foundation  ← timeline  ← project-format ← editor-core ← ui ← desktop
     ↖ media ↗
```

It contains no I/O, no threads, and no dependencies beyond `serde` and `uuid`.

## Public interfaces

| Item | Purpose |
|---|---|
| `TimelineTime` | Position/duration on the timeline. 960,000 ticks/second (§9). |
| `MediaTime` | Position within a source media file. Same timebase, distinct type. |
| `Rational` | Exact `num/den`, used for frame rates and FFmpeg timebases. |
| `FrameRate` | A `Rational` with frame-boundary helpers. |
| `ProjectId`, `SequenceId`, `TrackId`, `ClipId`, `MediaId`, `EffectId` | UUID newtypes. |

## Threading assumptions

None. Every type is `Copy` or `Clone`, `Send`, and `Sync`. No interior
mutability, no globals.

## Data ownership

Value types only. Nothing here owns a resource.

## Error behavior

Conversions that can lose precision return `Option`. There are no panics and no
silent rounding: `TimelineTime::from_frames` returns `None` when the frame rate
does not divide the timebase exactly, rather than rounding (§9).

`as_seconds_f64` is the one lossy conversion and is documented for display and
FFI only — never for timeline arithmetic (§74).

## Performance assumptions

All operations are integer arithmetic on `i64`, inlined, allocation-free.
`i64` at 960,000 ticks/s covers ~304,000 years, so overflow is not a practical
concern; intermediates use `i128` where a multiplication could exceed `i64`.
