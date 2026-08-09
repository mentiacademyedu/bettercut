# bettercut-media

## Purpose

The media asset model and the traits that FFmpeg will implement behind
(§3, §12, §84). This is the **only** crate that will ever reference an FFmpeg
binding (§86).

Milestone 1 scope: types and traits only. There is no FFmpeg dependency yet —
§83 says "Do not implement FFmpeg, playback, or advanced UI yet."

## Public interfaces

| Item | Purpose |
|---|---|
| `MediaAsset` | Path, dimensions, duration, codecs, colour metadata, proxy (§12). |
| `ColorMetadata` | Primaries, transfer, matrix, range — the §21a inputs. |
| `MediaKind` | Video / Audio / Image. |
| `ProxyAsset`, `ProxyStatus` | §13/§14 proxy state machine. |
| `MediaDecoder` | The §3 abstraction. FFmpeg becomes one implementation. |
| `VideoFrame`, `AudioBuffer` | What a decoder hands back. |

## Threading assumptions

`MediaAsset` and friends are plain `Send + Sync` data.

`MediaDecoder` implementations are **not** `Sync` and are owned by exactly one
decode thread. A decoder holds mutable codec state; sharing one across threads is
a bug, not a performance opportunity.

## Data ownership

`MediaAsset` owns metadata only. It never owns pixels, samples, or file handles,
and it never modifies source media (§2, §74).

## Error behavior

`MediaError` for every failure. §50: a decode failure marks the clip unavailable
and the session continues — it never crashes the editor. §74 forbids silently
ignoring an FFmpeg failure, so every fallible call returns a `Result` that names
what failed.

## Performance assumptions

* Colour metadata is read once at import and resolved to explicit values, so
  nothing downstream inspects source metadata per frame (§21a.2).
* `MediaDecoder::decode_frame` is expected to hand back a GPU-resident frame
  where hardware decode is available, and a RAM frame only on the fallback path
  (§5). The trait is deliberately agnostic so the fallback can exist.
* Decoders are cheap to seek and expensive to open; the media layer is expected
  to pool them, not open one per request.
