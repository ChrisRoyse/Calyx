# Stage 3 — Registry / Lenses (PH17–PH22)

> **STATUS: ✅ DONE (FSV-signed-off, current head `0ada102`).** All of PH17–PH22 are
> implemented and committed in `calyx-registry` (~4.1k LOC): the uniform
> `Registry.measure` dispatch over algorithmic / TEI-HTTP / candle-local / ONNX
> runtimes, the frozen contract + content-addressed `LensId`, hot-swap
> add/retire/park with a lazy backfill queue, capability-card profiling, and the
> default panels + closed-form temporal lenses E2/E3/E4. Stage 3 atomic-suite
> FSV root: `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752`.
> Build/test on aiwonder against the resident TEI services (:8088/:8089/:8090).
> Downstream Stage 4/5 FSV consumed Registry successfully; next active stage is
> Lodestar (`16_STAGE6_LODESTAR.md`).

The backbone (DOCTRINE §5): make plugging embedders in/out, reading their bits,
and using their associations as easy as possible. A lens is one call; its worth
is one number. Lands in `calyx-registry`. Reuses aiwonder's resident TEI lenses
(:8088/:8089/:8090). **Living-system role:** perception + growth by
differentiation.

---

## PH17 — Lens trait + algorithmic + tei-http runtimes
- **Status.** ✅ FSV-signed-off (`lens.rs`, `runtime/algorithmic.rs`,
  `runtime/tei_http.rs`, determinism probe; commit `cc322a6`).
- **Objective.** A uniform `Registry.measure(lens_id, input)` over multiple
  runtimes; ship `algorithmic` (deterministic feature encoders) and `tei-http`
  (reuse resident TEI) first.
- **Deps.** PH12, PH09.
- **Deliverables.** `lens.rs` (`Lens` impls), `runtime/algorithmic.rs`,
  `runtime/tei_http.rs` (calls :8088), batching.
- **Key tasks.** typed measure/measure_batch; HTTP client to TEI; algorithmic
  encoders (scalars, one-hot, AST-style) deterministic.
- **FSV gate.** embed a known input via :8088 **twice → identical** vector
  (determinism probe); algorithmic lens output reproducible bit-for-bit.
- **Axioms/PRD.** A4, A6, `05 §2`.

## PH18 — Frozen contract + content-addressed LensId
- **Status.** ✅ FSV-signed-off (`frozen.rs` weights-hash/dim/dtype/finite/unit-norm
  guards + `LensId` content-addressing; commit `c3b165b`).
- **Objective.** Enforce the frozen instrument at register + every measure.
- **Deps.** PH17.
- **Deliverables.** `frozen.rs` (weights_sha256 check, dim/dtype check, finite +
  unit-norm check, determinism probe), `LensId = blake3(name‖weights‖corpus‖
  shape)`.
- **Key tasks.** fail-closed codes (`CALYX_LENS_FROZEN_VIOLATION`,
  `_DIM_MISMATCH`, `_NUMERICAL_INVARIANT`, `_UNREACHABLE`); content-addressing
  so identical lens → identical id across vaults.
- **FSV gate.** mutate a weight → `FROZEN_VIOLATION`; wrong dim → `DIM_MISMATCH`;
  same lens registered in two vaults → same `LensId` (read both).
- **Axioms/PRD.** A4, A16, `05 §4`, `03 §2`.

## PH19 — candle-local + onnx runtimes
- **Status.** ✅ FSV-signed-off (`runtime/candle.rs`, `runtime/onnx.rs`, HF-cache
  resolver, dim guards; commit `4616ce7`).
- **Objective.** Run lens NNs locally (candle on sm_120, ORT CUDA EP) for
  embedded vaults / bespoke lenses.
- **Deps.** PH18.
- **Deliverables.** `runtime/candle.rs`, `runtime/onnx.rs`; weight loading from
  `CALYX_HOME/.hf-cache` (HF token from env).
- **Key tasks.** load a small real embedder from HF; produce unit-norm finite
  vectors; dim/normalize guards.
- **FSV gate.** a candle + an ONNX lens each produce finite, unit-norm vectors;
  dim guard fires on mismatch; weights pulled into `.hf-cache` (verified path).
- **Axioms/PRD.** A4, `05 §2`, `13 §2`.

## PH20 — Hot-swap add/retire/park + lazy backfill
- **Status.** ✅ FSV-signed-off (`swap.rs`: SlotSpec injection, retire-tombstone,
  park/unpark, priority `BackfillQueue`; commit `1db5ab0`).
- **Objective.** The core ergonomic: add/retire/park a lens with **no global
  re-embed**; lazy, priority-ordered backfill.
- **Deps.** PH19.
- **Deliverables.** `swap.rs` (add_lens/retire_lens/park/unpark), slot
  allocation + panel_version bump, lazy backfill scheduler (kernel/hot first,
  throttled, resumable).
- **Key tasks.** new slot CF + index placeholder; backfill queue; retire =
  tombstone (keep columns for history).
- **FSV gate.** add a lens on a populated vault → **no existing constellation
  rewritten**, new slot searchable immediately, backfilled cx fill over time
  (observe slot columns); retire tombstones, history still readable.
- **Axioms/PRD.** A5, `05 §3`, `17 §7.4` (backfill storm bounded).

## PH21 — Capability cards / profile
- **Status.** ✅ FSV-signed-off (`profile.rs`: `CapabilityCard` with spread /
  separation-silhouette / cost / coverage probes; commit `d132310`). Stage 5
  Assay now owns signal/redundancy measurements.
- **Objective.** "What is this lens good for?" in seconds, without full ingest.
- **Deps.** PH20.
- **Deliverables.** `profile.rs` → `CapabilityCard { signal, differentiation,
  spread, separation, cost, coverage }` over a probe set.
- **Key tasks.** participation-ratio/stable-rank spread; silhouette separation;
  cost (ms/input, VRAM). Signal/redundancy delegate to Assay (Stage 5) when up;
  until then spread/cost/coverage standalone.
- **FSV gate.** profile a lens → a one-JSON card with real numbers; a collapsed
  (low-spread) lens is flagged.
- **Axioms/PRD.** A6, A17, `05 §5`.

## PH22 — Default panels + temporal lenses E2/E3/E4
- **Status.** ✅ FSV-signed-off (`panels/defaults.rs` templates; `temporal/`
  E2 recency / E3 periodic / E4 positional, closed-form + retrieval-only flags;
  commit `a684b91`).
- **Objective.** Batteries-included panels (`text/code/civic/media-default`) and
  the three algorithmic temporal lenses in every panel.
- **Deps.** PH21.
- **Deliverables.** panel templates; `temporal/` E2 recency (decay), E3 periodic
  (hour/day), E4 positional — closed-form, no weights, data-oblivious.
- **Key tasks.** instantiate each default panel; E2/E3/E4 deterministic; mark
  them retrieval-only/excluded-from-dedup (used in Stage 9).
- **FSV gate.** each default panel instantiates with its slots; E2/E3/E4 produce
  deterministic closed-form scores (verified against hand-computed values).
- **Axioms/PRD.** A27, `05 §7`, `25 §2`.

---

## Stage 3 exit — ✅ achieved
A vault can add/retire/park real lenses (TEI/candle/ONNX/algorithmic) with no
re-embed, enforce the frozen contract, profile a lens in seconds, and ship with
default panels + temporal lenses — PRD `LENS`. The "nightmare every time" is one
`add_lens` call. Implemented and FSV-signed-off; downstream Stage 4/5 readbacks
on aiwonder depend on the registry/lens layer and remain green at commit
`0ada102`.
