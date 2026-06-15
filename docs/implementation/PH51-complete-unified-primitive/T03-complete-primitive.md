# PH51 · T03 — `complete(cx, clamp, free)` — full primitive with sufficiency gate

| Field | Value |
|---|---|
| **Phase** | PH51 — `complete()` unified primitive |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/complete.rs` (≤500) |
| **Depends on** | T01 (energy/descent), T02 (types), PH49 T03 (honesty gate — sufficiency check), PH49 T02 (self-consistency ceiling) |
| **Axioms** | A2, A16, A20 |
| **PRD** | `dbprdplans/26 §3`, `dbprdplans/26 §11.1` |

## Goal

Implement `complete(cx, clamp, free) -> Result<CompletionResult, OracleError>` — the
unified primitive from `26 §11.1`. Forward/reverse/lateral inference are clamp-direction
choices over a single energy descent. The function:
1. Validates `clamp ∩ free = ∅` (T02 invariant).
2. Runs `check_sufficiency` — refuses if panel insufficient (`CALYX_ORACLE_INSUFFICIENT`).
3. Retrieves region members from the Gτ region (PH37) for the domain.
4. For each free slot: runs `descend()` using clamped slots as anchor context.
5. Tags each filled slot: `Inferred` if converged, `Provisional` if not (or near-insufficient).
6. Caps confidence at `oracle_self_consistency.ceiling`.
7. Writes a Ledger entry (A15).

```
complete(cx, clamp: SlotSet, free: SlotSet) -> filled cx + confidence
  clamp present, free future    → PREDICTION   (Oracle / consequence)
  clamp outcome, free cause     → ABDUCTION    (reverse_query / root cause)
  clamp some lenses, free rest  → IMPUTATION   (slot completion / repair)
```

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn complete(vault: &Vault, cx: &Constellation, clamp: SlotSet, free: SlotSet, clock: &dyn Clock) -> Result<CompletionResult, OracleError>`
- [ ] **Validation:** check `clamp ∩ free = ∅`; check all IDs in `clamp ∪ free` exist in `cx`; else `CALYX_ORACLE_SLOT_CONFLICT`
- [ ] **Sufficiency gate:** call `check_sufficiency(vault, panel, domain)` — if `Err(Insufficient)`, return immediately; never start descent on insufficient panel (A20)
- [ ] **Region members:** query Gτ region (PH37 `GuardProfile`) for constellations in the same region as `cx`; these are the attractors in the energy sum
- [ ] **Descent loop:** for each slot in `free`, initialize its vector from `cx` if present (warm start) or from the region mean (cold start); run `descend(free_slot_vec, region_members_for_that_lens, beta, MAX_STEPS, EPS)`
- [ ] **Tagging:** `SlotTag::Inferred` if `DescentResult.converged && I_panel_oracle >= H(outcome)`; `SlotTag::Provisional` otherwise; clamped slots always `SlotTag::Measured`
- [ ] **Confidence:** `raw_confidence = 1.0 - mean_final_energy / ln(n_members)`; cap at `oracle_self_consistency.ceiling`; result is `CompletionResult.confidence`
- [ ] **Clamped slot immutability:** slots in `clamp` are copied unchanged from `cx` into `CompletionResult`; descent never touches them (assert in debug mode)
- [ ] **Ledger:** write one `LedgerRef` entry recording `cx.id`, `clamp`, `free`, and `confidence` (A15)
- [ ] β retrieved from Anneal config per `(domain, lens_id)` pair; default `1.0`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 3 clamped + 4 free slots on a 7-lens constellation; `complete` returns `CompletionResult` with 3 `Measured` + 4 `Inferred` tagged slots; `confidence ≤ ceiling`
- [ ] unit: abduction mode (clamp outcome slot, free cause slot); `complete` returns a cause slot value within cosine 0.9 of known planted cause ± 1e-2
- [ ] unit: imputation mode (clamp 5/7 slots, free 2/7); free slots converge to known values from synthetic attractor ± 1e-2
- [ ] unit: `confidence ≤ oracle_self_consistency.ceiling` for all three modes
- [ ] proptest: all free slots in `CompletionResult.filled_cx` have tag ∈ {`Inferred`, `Provisional`}; all clamped slots have tag = `Measured`
- [ ] edge (≥3): all slots clamped → `complete` returns `cx` unchanged, `confidence = 1.0` (trivially complete); all slots free → valid but wide `confidence`; zero region members → `Err` (no attractors to descend toward)
- [ ] fail-closed: insufficient panel → `CALYX_ORACLE_INSUFFICIENT` before descent starts; `clamp ∩ free ≠ ∅` → `CALYX_ORACLE_SLOT_CONFLICT`; Ledger write failure → `CALYX_ORACLE_LEDGER_WRITE_FAILURE`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `CompletionResult` JSON from `calyx readback complete --cx <id> --clamp <lens_ids> --free <lens_ids>`; Ledger CF entry
- **Readback:**
  ```
  calyx readback complete --cx <partial_cx_id> --clamp lens_1,lens_2,lens_3 --free lens_4,lens_5,lens_6,lens_7
  # Shows CompletionResult JSON; verify tag fields
  jq '.filled_cx[] | {lens_id, tag}' complete_output.json
  # All free slots: tag = "inferred" or "provisional"
  # All clamped slots: tag = "measured"
  ```
- **Prove:** free slots tagged `inferred` or `provisional`; clamped slots tagged `measured`; `confidence ≤ ceiling` field-by-field; Ledger entry written (xxd confirms)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH51 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
