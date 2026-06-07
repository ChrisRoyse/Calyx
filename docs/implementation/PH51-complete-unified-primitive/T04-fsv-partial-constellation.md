# PH51 · T04 — FSV: partial constellation completes to known full; slots tagged `inferred`

| Field | Value |
|---|---|
| **Phase** | PH51 — `complete()` unified primitive |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/tests/complete_tests.rs` (≤500) |
| **Depends on** | T03 (`complete`), T01 (energy), T02 (types) |
| **Axioms** | A2, A16, A20 |
| **PRD** | `dbprdplans/26 §3`, `dbprdplans/26 §11.1`, `dbprdplans/21` (FSV gate: "partial constellation completes to known full within tolerance; completed slots tagged `inferred`") |

## Goal

Prove the PH51 FSV exit gate on aiwonder: a partial constellation (some slots missing)
completes to the known full one within tolerance, and all completed (free) slots are
tagged `inferred`, never confused with `measured`. This is the byte-level proof that the
energy-descent machinery works and that the epistemic-tag discipline (`26 §11.1`, A2/A16)
holds in practice.

## Build (checklist of concrete, code-level steps)

- [ ] **FSV test 1 — imputation mode (slot completion):** create synthetic vault with a fully-known 7-lens constellation `cx_full`; create `cx_partial` with lenses 1–3 populated, lenses 4–7 zeroed/missing; call `complete(cx_partial, clamp={1,2,3}, free={4,5,6,7})`; assert each free slot's cosine similarity to the corresponding slot in `cx_full` is ≥ `0.90` (tolerance); assert all free slots have `tag = SlotTag::Inferred`; assert all clamped slots have `tag = SlotTag::Measured`
- [ ] **FSV test 2 — prediction mode:** create `cx` with current-state lenses clamped; free = future-outcome slot; call `complete`; assert `confidence ≤ oracle_self_consistency.ceiling`; assert free slot tagged `inferred`
- [ ] **FSV test 3 — abduction mode:** create `cx` with outcome slot clamped; free = cause slot; call `complete`; assert returned cause slot ≥ `0.85` cosine to planted cause ± 1e-2
- [ ] **FSV test 4 — insufficient panel refused:** construct a panel where `I_panel_oracle = 0.3 < H(outcome) = 1.0`; call `complete`; assert `Err(OracleError::Insufficient)` returned before any descent starts
- [ ] **FSV test 5 — tag discipline scan:** run all three modes; iterate over `CompletionResult.filled_cx`; assert no free slot has `tag = SlotTag::Measured`; assert no clamped slot has `tag ≠ SlotTag::Measured`
- [ ] Write all five test outputs to `/tmp/ph51_complete_fsv.json` (one JSON object per test); each object includes `test_name`, `cosine_similarities`, `tags`, `confidence`, `converged`
- [ ] Use `calyx-testkit` `MockClock` with seed `42`; all RNG deterministic; region members constructed from seeded synthetic vectors

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] FSV test 1: all 4 free slots ≥ 0.90 cosine to `cx_full`; all tagged `inferred`
- [ ] FSV test 2: `confidence ≤ ceiling`; future slot tagged `inferred`
- [ ] FSV test 3: cause slot ≥ 0.85 cosine to planted cause
- [ ] FSV test 4: `CALYX_ORACLE_INSUFFICIENT` fires; no descent log entries
- [ ] FSV test 5: tag discipline holds for all 3 modes in one pass
- [ ] edge: `complete` with fully-known `cx` (all slots clamped) → `CompletionResult` identical to `cx`; all `Measured`
- [ ] fail-closed: region has 0 members → structured error; not a NaN descent

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/tmp/ph51_complete_fsv.json`; stdout of `cargo test -p calyx-oracle -- complete_tests --nocapture`
- **Readback:**
  ```
  cargo test -p calyx-oracle -- complete_tests --nocapture 2>&1 | tee /tmp/ph51_fsv.log
  cat /tmp/ph51_complete_fsv.json | jq '.[] | {test_name, all_inferred: ([.tags[] | select(. == "inferred")] | length), all_measured_clamped: ([.tags[] | select(. == "measured")] | length)}'
  grep "cosine_similarities" /tmp/ph51_complete_fsv.json | python3 -c "import sys,json; data=json.load(sys.stdin); assert all(s >= 0.90 for s in data['cosine_similarities'])"
  ```
- **Prove:** free slots show `tag = "inferred"` in JSON; cosine ≥ 0.90 for all 4 imputed slots; `CALYX_ORACLE_INSUFFICIENT` present in log for test 4; no slot shows `tag = "measured"` unless clamped

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH51 GitHub issue
- [ ] `/tmp/ph51_complete_fsv.json` screenshot showing tag and cosine fields attached
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
