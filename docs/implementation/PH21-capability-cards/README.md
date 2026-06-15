# PH21 — Capability cards / profile

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** (stage P2 completion)  ·  **Axioms:** A6, A17

## Objective

Implement `Registry.profile(lens_id, probe_set?) -> CapabilityCard` so an
agent can answer "what is this lens good for?" in seconds without full
ingestion. The `CapabilityCard` provides six metrics: `signal`, `differentiation`,
`spread` (participation-ratio / stable-rank), `separation` (silhouette),
`cost` (ms/input, VRAM), and `coverage`. A collapsed (low-spread) lens is
flagged automatically. Signal and redundancy delegate to Assay (Stage 5);
without scoped Assay rows, spread/cost/coverage and explicit proxy metrics are
standalone.

## Dependencies

- **Phases:** PH20 (Registry with slots and lifecycle), PH12 (Forge CPU for
  matrix ops — participation ratio / stable rank)
- **Provides for:** PH22 (default panels select their lenses based on
  capability cards), PH29 (Assay differentiation check compares against the
  card's differentiation field), PH47 (Anneal lens-proposal checks coverage
  and signal)

## Current state (build off what exists)

`calyx-registry` has PH17-PH20. Forge PH12 provides CPU GEMM/cosine. Assay
(Stage 5) owns grounded `signal` and `differentiation`; Registry profile keeps
those fields as `None`/JSON `null` for no-Assay callers, and
`profile_slot_with_assay` attaches scoped `AssayStore` lens signal and pair-gain
rows when they are available. `list_panel` surfaces stored `Slot.bits_about`,
and `list_panel_with_assay` overlays scoped Assay rows. The fast probe-derived
estimates remain available only as explicitly labeled `proxy_signal` and
`proxy_differentiation`, so Stage 6+ callers cannot mistake Registry estimates
for grounded Assay quality.

#334 FSV evidence:
`/home/croyse/calyx/data/fsv-issue334-ph21-assay-registry-20260608`.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/profile.rs` | `CapabilityCard`, `ProbeSet`, `profile_lens()`, spread metrics, silhouette, cost measurement |
| `crates/calyx-registry/src/profile/assay.rs` | Assay-backed capability attachment from scoped `AssayStore` rows |
| `crates/calyx-registry/src/panel_ops.rs` | `list_panel`/`list_panel_with_assay` slot listing with `bits_about` readback |
| `crates/calyx-registry/src/profile/spread.rs` | participation-ratio and stable-rank computations over probe embeddings |
| `crates/calyx-registry/src/profile/separation.rs` | silhouette score over labeled probes |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | CapabilityCard struct + ProbeSet | PH20 |
| T02 | Spread metrics (participation-ratio + stable-rank) | T01 |
| T03 | Separation metric (silhouette) | T01 |
| T04 | Cost measurement (ms/input, VRAM) | T01 |
| T05 | Registry.profile() + collapsed-lens flag | T02, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. `profile_lens(gte_lens_id, probe_set)` returns a one-JSON `CapabilityCard`
   with `signal:null`, `differentiation:null`, explicit proxy estimates, and
   real numbers for spread/separation/cost/coverage.
2. `profile_slot_with_assay(slot, store, scoped_key)` returns a one-JSON
   `CapabilityCard` with `signal_source:"assay_store"` and
   `differentiation_source:"assay_store"` read from persisted Assay CF bytes.
3. `list_panel_with_assay(panel, store, scoped_key)` returns `bits_about` from
   the same scoped Assay lens row.
4. A collapsed lens (probe embeddings that are nearly identical — manually
   constructed mock) is flagged in the card: `collapsed: true`.
5. Print the card/listing JSON to stdout and attach to the PH21 GitHub issue;
   readback must prove missing Assay fields are null and present Assay fields
   come from stored rows, not proxy estimates.

Readback: `CALYX_FSV_ROOT=<root> cargo test -p calyx-registry ph21_profile_card_aiwonder_fsv -- --ignored --nocapture`
on aiwonder; JSON card/listing, Assay CF JSON, and Assay SST hex are attached to
#334.

## Risks / landmines

- **Signal stub:** `signal` and `differentiation` fields must not be silently
  zero-filled. They carry `Option<f32>` with `None` until scoped Assay rows are
  provided, so callers know they are uncomputed.
- **Probe set size:** participation ratio and stable rank require at least
  N=50 vectors for meaningful results; enforce a minimum and return a
  `coverage` flag if the probe set is too small.
- **VRAM measurement:** cost.vram_mb is approximate (pre-/post-inference
  CUDA memory query); mark it as `estimated: true` in the card.
