# PH22 — Default panels + temporal lenses E2/E3/E4

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** (LENS predicate completion)  ·  **Axioms:** A27

## Objective

Ship batteries-included panels (`text-default`, `code-default`, `civic-default`,
`media-default`) so a new vault is multi-lens on day one. Additionally implement
the three algorithmic temporal lenses — **E2 Temporal-Recent** (recency decay),
**E3 Temporal-Periodic** (hour-of-day / day-of-week), and **E4 Temporal-Positional**
(sequence order) — as closed-form, no-weights, data-oblivious `AlgorithmicLens`
instances. Mark temporal lenses as `retrieval_only = true` and
`excluded_from_dedup = true` (`25 §2`). All three are deterministic;
verified against hand-computed reference values.

## Dependencies

- **Phases:** PH21 (capability cards + profile; Registry fully operational)
- **Provides for:** PH37 (Ward guard uses panels), PH40 (Sextant temporal
  fusion — E2/E3/E4 AP-60 post-retrieval boost), PH41 (dedup excludes E2/E3/E4
  from `Gτ` agreement)

## Current state (build off what exists)

`calyx-registry` has PH17–PH22 in-tree. `AlgorithmicLens` includes temporal
closed-form lenses, `panels/` exposes default panel templates, and the temporal
flags are persisted onto core `Slot` rows so downstream search and dedup can read
`retrieval_only` and `excluded_from_dedup` without guessing.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env. E2/E3/E4
are closed-form (no HTTP, no HF cache needed).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/temporal/mod.rs` | module declarations, `TemporalLensFlags` |
| `crates/calyx-registry/src/temporal/e2_recency.rs` | E2: Linear/Exponential/Step decay |
| `crates/calyx-registry/src/temporal/e3_periodic.rs` | E3: hour-of-day / day-of-week matching |
| `crates/calyx-registry/src/temporal/e4_positional.rs` | E4: positional encoding over event sequence |
| `crates/calyx-registry/src/panels/mod.rs` | `PanelTemplate`, `instantiate_panel` |
| `crates/calyx-registry/src/panels/defaults.rs` | four default panel constructors |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | E2 Temporal-Recent lens (Linear/Exponential/Step decay) | PH17 T02 |
| T02 | E3 Temporal-Periodic lens (hour-of-day, day-of-week) | PH17 T02 |
| T03 | E4 Temporal-Positional lens (sequence order encoding) | PH17 T02 |
| T04 | TemporalLensFlags: retrieval_only + excluded_from_dedup | T01, T02, T03 |
| T05 | Panel templates + default panel constructors | T04 |
| T06 | FSV: panels instantiate + E2/E3/E4 deterministic hand-verified | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. Each default panel (`text-default`, `code-default`, `civic-default`,
   `media-default`) instantiates via `instantiate_panel` with its full slot
   list; print each panel's slot count and slot names.
2. E2, E3, E4 each produce deterministic closed-form scores verified against
   hand-computed reference values (see T01–T03).
3. All three temporal lenses have `retrieval_only = true` and
   `excluded_from_dedup = true` in their `TemporalLensFlags`.

Readback: `cargo test -p calyx-registry panels_temporal -- --nocapture`
on aiwonder; panel slot counts and E2/E3/E4 reference values attached to
PH22 GitHub issue.

## Risks / landmines

- **AP-60 invariant:** temporal lenses must never dominate primary retrieval;
  the `retrieval_only` flag is the enforcer. Any code that uses E2/E3/E4 in
  primary ranking must check this flag — assert in `Registry::measure` that
  temporal lenses are only used via the post-retrieval boost path (PH40 wires
  this; for now, log a warning if `retrieval_only` lens is called from a
  primary-ranking path).
- **excluded_from_dedup:** the `excluded_from_dedup` flag must be readable by
  PH41 (dedup) without importing `calyx-registry`; expose it via `LensSpec`
  public field (already in the struct plan).
- **Closed-form determinism:** E2/E3/E4 must not call any external service,
  load any weights, or use `SystemTime::now()` in their `measure` implementations;
  they consume the timestamp from `input.bytes` (interpreted as a Unix timestamp
  i64 serialized as little-endian 8 bytes).
