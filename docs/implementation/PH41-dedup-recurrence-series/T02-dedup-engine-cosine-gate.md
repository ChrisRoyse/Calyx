# PH41 · T02 — Dedup engine: per-slot cosine gate (content-only, excl. E2/E3/E4)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/engine.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH37 (`Gτ` guard math — provides per-slot cosine) |
| **Axioms** | A28, A3 |
| **PRD** | `dbprdplans/25 §5` |

## Goal

Implement the dedup engine: `check_dedup(new_cx, vault, policy) -> DedupDecision`
scans the vault for any existing constellation whose required content slots all
meet `cos(new_k, existing_k) ≥ τ_k`. This reuses the `Gτ` math from PH37 —
the same guard that blocks injection is reused for dedup (A3 no-flatten means
false-merge is far harder than single-embedding dedup). Temporal lenses are
excluded by construction: only slots listed in `required_slots` (which must not
contain E2/E3/E4) participate in the cosine comparison.

## Build (checklist of concrete, code-level steps)

- [x] Define `DedupDecision` enum: `NoMatch` | `Match { existing: CxId, per_slot_cos: Vec<(SlotId, f32)> }` | `AnchorConflict { existing: CxId }` — `AnchorConflict` is checked before cosine; T03 made the branch durable and FSV-backed
- [x] Implement `resolve_tau(slot_id: SlotId, config: &TctCosineConfig, guard_profile: Option<&GuardProfile>) -> f32`:
  - `TauStrategy::PerSlot` → look up `slot_id` in the vec; missing slot → `CALYX_DEDUP_SLOT_NOT_IN_TAU`
  - `TauStrategy::Calibrated` → read threshold from `guard_profile.tau_for(&slot_id)` → `CALYX_DEDUP_MISSING_GUARD_PROFILE` if profile or threshold is missing
- [x] Implement `cosine_passes_all_required(new_cx: &Constellation, existing_cx: &Constellation, config: &TctCosineConfig, guard_profile: Option<&GuardProfile>) -> Result<Option<Vec<(SlotId, f32)>>, CalyxError>`:
  - for each `slot_id` in `config.required_slots`: compute `cos(new_slot_vec, existing_slot_vec)` using PH37/PH12 cosine; if `cos < resolve_tau(slot_id)` → return `None` (short-circuit); if all pass → `Some(per_slot_cosines)`
- [x] Implement `check_dedup(new_cx: &Constellation, vault: &Vault, policy: &DedupPolicy, guard_profile: Option<&GuardProfile>) -> Result<DedupDecision, CalyxError>`:
  - `Off` → always `NoMatch`
  - `Exact` → hash-match only (delegates to PH09 idempotent check) → `Match` or `NoMatch`
  - `TctCosine(config)` → iterate existing constellations; for each: anchor-conflict check first (T03); then `cosine_passes_all_required`; first match → `Match`; no match → `NoMatch`
- [x] Efficiency: scan bounded by `DPI` — do NOT scan `C(N,2)` beyond the DPI ceiling; once the candidate set exceeds DPI, emit `CALYX_DEDUP_DPI_EXCEEDED` and fall back to `Exact`-only
- [x] Reuse PH12/PH37 cosine implementation; never duplicate math

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `DedupPolicy::Off` → `check_dedup` always returns `NoMatch` for any input
- [x] unit: two identical content-slot vectors (cos=1.0), τ=0.9 → `Match` returned with `per_slot_cos = [(slot, 1.0)]`
- [x] unit: cos=0.88 < τ=0.9 → `NoMatch`
- [x] unit: `TauStrategy::PerSlot` with slot-A τ=0.9 (cos=0.95 passes) and slot-B τ=0.8 (cos=0.75 fails) → `NoMatch` (all-required logic, short-circuit on B)
- [x] unit: `TauStrategy::Calibrated` with missing `guard_profile` → `CALYX_DEDUP_MISSING_GUARD_PROFILE`
- [x] proptest: `check_dedup` on identical constellations always returns `Match` (cos=1.0 ≥ any reasonable τ)
- [x] edge: `required_slots` contains a slot not present in `new_cx` → `CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION`
- [x] edge: vault is empty → `NoMatch` without panic
- [x] fail-closed: candidate set exceeds DPI ceiling → `CALYX_DEDUP_DPI_EXCEEDED` (not a silent scan)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** real Aster vault bytes under
  `/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20/vault`,
  plus the saved readback artifacts in the FSV root.
- **Readback trigger:** `CALYX_DEDUP_ENGINE_FSV_ROOT=/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20 cargo test -p calyx-cli --test dedup_check_readback -- --nocapture`
  created a durable vault, inserted one source constellation, ran
  `calyx readback dedup-check`, and saved stdout/stderr, base CF bytes, and
  BLAKE3 sums.
- **Prove:** `dedup-check-readback.json` records existing CxId
  `eec6d89ce772fa6e05416733ebce870f`; near candidate (cos
  `0.949999988079071`, τ `0.9`) prints `Match` for that exact CxId; distinct
  candidate (cos `0.8500000238418579`) prints `NoMatch`; missing slot, invalid
  tau, DPI-exceeded, invalid calibrated tau (`NaN`, `+/-inf`, out-of-range),
  and constructor-bypassed empty `required_slots` edges fail closed. Separate
  `calyx readback --cf base` and `--cf slot_00` reads show the persisted source
  row and slot vector bytes.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate)
- [x] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
