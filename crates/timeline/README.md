# bettercut-timeline

## Purpose

The sequence/track/clip data model and the queries the renderer and UI run
against it. Milestone 1 scope: the model, its invariants, and range queries.
Editing operations (move, trim, split, ripple) arrive in Milestone 3 per §85.

## Public interfaces

| Item | Purpose |
|---|---|
| `Sequence` | Tracks, canvas resolution, frame rate (§8). |
| `VideoTrack`, `AudioTrack` | Ordered clip containers with visible/locked/muted state. |
| `VideoClip`, `AudioClip` | Media references with timeline and source ranges (§8). |
| `Transform` | Position, scale, rotation, anchor (§59 "Basic transform"). |
| `Sequence::clips_in_range` | Viewport query for §53/§54. |

## Threading assumptions

Plain data. `Send + Sync`, no interior mutability, no locks. The editor core owns
the only instance and hands out `&` borrows per UI frame (§54).

## Data ownership

Clips own nothing but IDs and numbers. A clip references media by `MediaId`; it
never owns or copies media (§2, non-destructive editing).

Tracks keep clips sorted by `timeline_start` and non-overlapping. Both invariants
are enforced on insert and asserted by `Track::validate`.

## Error behavior

`TimelineError` for every rejected operation. No panics: an overlapping insert,
an inverted range, or an unknown ID is an error value, not a crash (§50).

## Performance assumptions

* Clips are stored sorted, so `clips_in_range` is a binary search plus a scan of
  the visible span — **not** a walk of the whole project. §53 requires drawing
  ~30 visible clips out of 10,000 without touching the other 9,970.
* All timing is `i64` ticks. **No floating point appears in timeline position
  arithmetic** (§74). Floats appear only in `Transform`, which is spatial.
* No allocation in query paths; `clips_in_range` returns a borrowed slice.
