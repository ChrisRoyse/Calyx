# Stage 3 — Registry / Lenses (PH17–PH22)

> **STATUS: ✅ DONE (FSV-signed-off; latest pushed main tracked in #23).** All of PH17–PH22 are
> implemented and committed in `calyx-registry` (~4.1k LOC): the uniform
> `Registry.measure` dispatch over algorithmic / TEI-HTTP / candle-local / ONNX
> runtimes, the frozen contract + content-addressed `LensId`, hot-swap
> add/retire/park with a lazy durable backfill scheduler, capability-card profiling, and the
> default panels + closed-form temporal lenses E2/E3/E4. Stage 3 atomic-suite
> FSV root: `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752`.
> Build/test on aiwonder against the resident TEI services (:8088/:8089/:8090).
> Downstream Stage 4/5/6 FSV consumed Registry successfully; current active
> frontier is tracked in `03_PHASE_MAP.md`.
> Post-sweep hardening #288 carries PH22 temporal `retrieval_only` and
> `excluded_from_dedup` flags into core `Slot` rows, not only template specs.
> Post-sweep hardening #289 makes PH19 ONNX CUDA execution-provider
> registration fail loud instead of silently falling back to CPU.
> FSV root for #289:
> `/home/croyse/calyx/data/fsv-issue289-onnx-provider-20260608`.
> Post-sweep hardening #300 replaces the PH20 synthetic queue-only FSV with
> durable scheduler watermarks/throttle/restart-resume state. FSV root for #300:
> `/home/croyse/calyx/data/fsv-issue300-backfill-scheduler-20260608`.
> Post-sweep hardening #311 wires durable backfill enqueue into the hot-swap
> API via `SwapController::add_lens_durable`; callers no longer need a separate
> scheduler enqueue after add. FSV root for #311:
> `/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608`.
> Post-sweep hardening #310 closes the PH18 unfrozen-registration bypass:
> `Registry::register` and `register_with_spec` now fail with
> `CALYX_LENS_FROZEN_VIOLATION`; runtime callers must use `register_frozen*`.
> FSV root for #310:
> `/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608`.
> Post-sweep hardening #314 makes hot-swap add paths require a frozen registered
> lens before panel, queue, or durable scheduler mutation. FSV root for #314:
> `/home/croyse/calyx/data/fsv-issue314-registered-hot-swap-20260608`.
> Post-sweep hardening #315 persists backfill scheduler JSON through
> temp-file/fsync/rename and fails closed on corrupt scheduler state. FSV root
> for #315:
> `/home/croyse/calyx/data/fsv-issue315-backfill-atomic-persist-20260608`.
> Post-sweep hardening #321 makes scheduler mutations transactional: a persist
> failure after the scheduler rename restores the previous scheduler JSON before
> `add_lens_durable` rolls back panel and queue state. FSV root for #321:
> `/home/croyse/calyx/data/fsv-issue321-durable-rollback-20260608`.
> Post-sweep hardening #327 makes PH20 lifecycle operations idempotent where
> appropriate: exact duplicate adds return the existing slot without mutation,
> repeated park/unpark/retire calls do not keep bumping panel versions, and
> park/retire cancels pending in-memory backfill for that slot.
> Post-sweep hardening #339 records whether frozen registration used a
> deterministic probe or an explicit contract-only exemption, and adds a
> Registry -> Aster slot backfill -> Sextant index/search integration FSV.
> FSV root for #339:
> `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608`
> (`registry-sextant-readback.json`
> `2163eeb8397de004a8a1c39e04631ccc7aa3f68836a7aa713bca7a6911cf6708`).
> Default panel instantiation remains template-only: `instantiate_panel` creates
> core `Panel` rows from `PanelTemplate`; registry/store-backed activation of
> those templates is a later product surface owned by PH62 T02/T03/T08, PH63
> T02/T03/T08, and PH71 T05/T06, not a hidden Stage 3 claim.

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
  so identical lens → identical id across vaults. Plain `register` /
  `register_with_spec` are fail-closed compatibility stubs; successful
  registration requires `register_frozen*` with a `FrozenLensContract`.
  `Registry::determinism_proof` reports `probe_verified` for
  `register_frozen_with_probe` and `contract_only_exemption` for the explicit
  no-probe path.
- **FSV gate.** mutate a weight → `FROZEN_VIOLATION`; wrong dim → `DIM_MISMATCH`;
  plain registration → `FROZEN_VIOLATION` and no insert; same lens registered
  in two vaults → same `LensId` (read both).
- **Axioms/PRD.** A4, A16, `05 §4`, `03 §2`.

## PH19 — candle-local + onnx runtimes
- **Status.** ✅ FSV-signed-off (`runtime/candle.rs`, `runtime/onnx.rs`, HF-cache
  resolver, dim guards; commit `4616ce7`; post-sweep Candle device-policy
  truth #301 FSV-backed on aiwonder).
- **Objective.** Run lens NNs locally (Candle CPU-explicit by default, optional
  fail-loud Candle CUDA behind `calyx-registry/candle-cuda`, ORT CUDA EP
  fail-loud by default) for embedded vaults / bespoke lenses.
- **Deps.** PH18.
- **Deliverables.** `runtime/candle.rs`, `runtime/onnx.rs`; weight loading from
  `CALYX_HOME/.hf-cache` (HF token from env).
- **Key tasks.** load a small real embedder from HF; produce unit-norm finite
  vectors; dim/normalize guards.
- **Post-sweep note.** The ONNX runtime now uses CUDA device 0 with
  `error_on_failure` and no implicit CPU fallback; a CPU-only path must be
  explicit and separately reported (#289). Candle now reports device policy
  explicitly: default `cpu_explicit,no_cuda`; requesting Candle CUDA without the
  optional `candle-cuda` feature fails loud instead of silently claiming GPU;
  the optional `candle-cuda` build was separately verified on aiwonder device 0
  (RTX 5090, compute capability 12.0).
- **FSV gate.** a Candle CPU-explicit + an ONNX CUDA lens each produce finite,
  unit-norm vectors; dim guard fires on mismatch; weights pulled into
  `.hf-cache` (verified path). Optional Candle CUDA must be separately run with
  `--features candle-cuda` before it can be claimed; #301 readback root:
  `/home/croyse/calyx/data/fsv-issue301-candle-device-policy-20260608`.
- **Axioms/PRD.** A4, `05 §2`, `13 §2`.

## PH20 — Hot-swap add/retire/park + lazy backfill
- **Status.** ✅ FSV-signed-off (`swap.rs`: SlotSpec injection, retire-tombstone,
  park/unpark, priority `BackfillQueue`; `add_lens_durable` persists
  `BackfillScheduler` requests; #311 FSV root
  `/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608`).
- **Objective.** The core ergonomic: add/retire/park a lens with **no global
  re-embed**; lazy, priority-ordered backfill.
- **Deps.** PH19.
- **Deliverables.** `swap.rs` (add_lens/retire_lens/park/unpark), slot
  allocation + panel_version bump, `backfill.rs` lazy scheduler (kernel/hot
  first, persisted watermarks, throttle, restart resume).
- **Key tasks.** new slot CF + index placeholder; backfill queue; retire =
  tombstone (keep columns for history).
- **FSV gate.** add a lens on a populated vault → **no existing constellation
  rewritten**, persisted scheduler JSON shows ordered/throttled/resumed
  backfill, reopened Aster slot CF reads show both backfilled vectors, and
  retire tombstones while history stays readable. #327 adds lifecycle regression
  coverage for idempotent duplicate add, no-op repeated lifecycle calls, and
  pending backfill cancellation on park/retire. #339 adds the cross-stage
  product path: `add_lens_durable` -> durable scheduler batch -> Registry
  measurement -> `AsterVault::put_slot_vector` -> Sextant `SlotIndexMap` insert
  -> stored-provenance search readback.
- **Axioms/PRD.** A5, `05 §3`, `17 §7.4` (backfill storm bounded).

## PH21 — Capability cards / profile
- **Status.** ✅ FSV-signed-off (`profile.rs`: `CapabilityCard` with spread /
  separation-silhouette / cost / coverage probes; commit `d132310`). Stage 5
  Assay-backed metric attachment is now wired through #334.
- **Objective.** "What is this lens good for?" in seconds, without full ingest.
- **Deps.** PH20.
- **Deliverables.** `profile.rs` → `CapabilityCard { signal,
  differentiation, proxy_signal, proxy_differentiation, spread, separation,
  cost, coverage }` over a probe set. Without scoped Assay evidence, grounded
  `signal`/`differentiation` remain JSON `null` with `assay_pending`; with a
  scoped `AssayStore`, `profile_slot_with_assay` attaches stored lens signal
  bits and pair-gain differentiation from Assay rows. Registry estimates stay
  explicitly labeled as proxies.
- **Key tasks.** participation-ratio/stable-rank spread; silhouette separation;
  cost (ms/input, VRAM). Signal/redundancy delegate to Assay (Stage 5) when up;
  until then spread/cost/coverage standalone.
- **FSV gate.** profile a lens → a one-JSON card where no-Assay callers show
  `signal`/`differentiation` as JSON `null`, scoped Assay callers read stored
  `assay_store` values, `list_panel_with_assay` returns `bits_about`, proxy
  estimates and spread/separation/cost/coverage read back as numbers, and a
  collapsed (low-spread) lens is flagged. #334 evidence:
  `/home/croyse/calyx/data/fsv-issue334-ph21-assay-registry-20260608`.
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
- **Post-sweep note.** Temporal flags now persist on instantiated core
  `Panel.slots` so downstream consumers do not need the original template spec
  to enforce AP-60 retrieval/dedup boundaries (#288).
- **Post-sweep note.** Default panel instantiation is template-only in Stage 3:
  it does not register/store/activate runtime lenses. Registry/store-backed
  activation remains a later product workflow; docs and FSV must not claim it
  from `instantiate_panel` alone (#339; owners PH62 T02/T03/T08, PH63
  T02/T03/T08, PH71 T05/T06).
- **FSV gate.** each default panel instantiates with its slots; E2/E3/E4 produce
  deterministic closed-form scores (verified against hand-computed values).
- **Axioms/PRD.** A27, `05 §7`, `25 §2`.

---

## Stage 3 exit — ✅ achieved
A vault can add/retire/park real lenses (TEI/candle/ONNX/algorithmic) with no
re-embed, enforce the frozen contract, profile a lens in seconds, and ship with
default panels + temporal lenses — PRD `LENS`. The "nightmare every time" is one
`add_lens` call. Implemented and FSV-signed-off; downstream Stage 4/5 readbacks
on aiwonder depend on the registry/lens layer, and PH20's durable add-lens
scheduler path is FSV-backed by #311.
