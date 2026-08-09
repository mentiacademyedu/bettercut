# ADR 004 — Proxy Encoding

**Status:** Accepted (specification); **not yet implemented** — Milestone 7
**Date:** 2026-08-08
**Decides:** §13, §13.1, §47a
**Partially implemented by:** `crates/media/src/proxy.rs` (`ProxySpec::V1`)

---

## Decision

Preview edits proxy media. Proxies are **H.264, GOP length 1 (all-intra)**,
High profile, 8-bit, yuv420p, limited range, BT.709, audio 48 kHz stereo.
Default 720p; 540p in Performance mode.

---

## Why all-intra is the decision that matters

Long-GOP media cannot be randomly accessed. Seeking to frame N means seeking to
the preceding keyframe and decoding forward. On a 250-frame GOP that is up to
**250 decodes per seek**.

§81 targets <100 ms scrub and <150 ms precise seek on a 4-core machine with
integrated graphics. No CPU in that range reaches those numbers by decoding 250
frames. With GOP=1, every frame is a keyframe and a seek is exactly one decode.

This single choice is worth more to perceived speed than any other optimization
in the application.

## The cost, accepted deliberately

All-intra files are **3–5× larger** on disk than a long-GOP equivalent at the
same resolution. That is the trade: disk is cheap, scrub latency is the product.

Consequences that follow from accepting it:
* §67's cache limit matters more, and the user must be warned before a proxy run
  would exceed it.
* Proxy generation must use a hardware encoder where available — it is otherwise
  the heaviest background job in the app.

## Proxies normalize, not just shrink

```text
10-bit              → 8-bit
HDR (PQ/HLG)        → tone-mapped SDR
any colour matrix   → BT.709
any audio rate      → 48 kHz          (required by ADR 003)
variable frame rate → constant frame rate
```

This is the quiet benefit: **the preview pipeline only ever handles one format.**
An entire class of colour and timing bugs cannot occur in preview because the
inputs cannot vary. It also means §21a's conversion path has one well-tested
case and a rarely-taken general case, rather than only a general case.

---

## Consequences

* `ProxySpec::V1` encodes this decision as data so the Milestone 7 encoder cannot
  silently disagree with it. `the_proxy_spec_is_all_intra` fails if `gop_length`
  is ever raised — which is exactly how someone would "optimize" disk usage
  without realising they had traded away the seek target.
* Proxy widths are rounded to even numbers: yuv420p chroma subsampling requires
  it, and an odd width fails at encode time rather than at spec time.
* `MediaAsset::should_generate_proxy` implements §13's trigger list (≥1440p,
  HEVC/AV1/VP9, 10-bit, >60 fps, or anything needing normalization).

---

## Outstanding

Nothing here is implemented beyond the specification and the trigger rules.
Milestone 7 must:

1. Generate proxies through the job scheduler with §15.1's thread caps.
2. Switch preview to the proxy when `ProxyStatus::Ready`.
3. Warn before exceeding §67's cache limit.
4. Measure the resulting seek latency on the §52.1 reference machine and record
   it here. If the numbers do not hit §81, this ADR is wrong and needs revisiting
   — not the target.
