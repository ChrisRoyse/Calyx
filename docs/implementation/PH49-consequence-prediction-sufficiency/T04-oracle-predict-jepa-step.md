# PH49 · T04 — `oracle_predict` JEPA step + confidence ceiling

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/predict.rs` (≤500) |
| **Depends on** | T03 (honesty gate), T02 (self-consistency), PH42 (grounded recurrence), PH37 (Gτ guard) |
| **Axioms** | A20, A2, A8, A29 |
| **PRD** | `dbprdplans/21 §2`, `dbprdplans/21 §1` |

## Goal

Implement `oracle_predict(vault, action, domain) -> Result<Prediction, OracleError>`,
the main Oracle API. The call executes a JEPA-style step: given `(panel_t, action)`,
traverse grounded recurrence edges to predict `panel_{t+1}` and the outcome
(`AnchorValue`). Confidence is computed from recurrence cadence and capped at
`oracle_self_consistency.ceiling`. The honesty gate (`check_sufficiency`) is called
first; if `sufficient: false`, the call returns `Err(OracleError::Insufficient)`
immediately — no prediction proceeds. Provenance and guard are always populated.
This is a forward-association query over stored transition constellations, not a
learned neural forward pass (A24, strictly Royse corpus `21 §2`).

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn oracle_predict(vault: &Vault, action: &Action, domain: DomainId, clock: &dyn Clock) -> Result<Prediction, OracleError>` — public API matching `21 §9`
- [ ] **Step 0 — honesty gate:** call `check_sufficiency(vault, panel, domain)` first; if `Err(Insufficient)`, return immediately with the bound populated
- [ ] **Step 1 — JEPA step:** retrieve panel snapshot `panel_t` (current constellation for the domain's action anchor); traverse grounded recurrence edges where `action` matches (PH42 recurrence wiring); collect matching outcomes from prior occurrences
- [ ] **Step 2 — outcome prediction:** weighted vote over recurring outcomes using Bayesian posterior (Gamma-Poisson rate prior, `26 §6`); predicted `outcome: AnchorValue` = highest-posterior outcome
- [ ] **Step 3 — raw confidence:** `raw_confidence = posterior_mode / (posterior_mode + posterior_uncertainty)` from the Gamma-Poisson posterior; normalized to `[0.0, 1.0]`
- [ ] **Step 4 — ceiling cap:** call `oracle_self_consistency(vault, domain, clock)`; `confidence = raw_confidence.min(ceiling)` — confidence **never** exceeds the ceiling (`21 §2`)
- [ ] **Step 5 — guard:** call `calyx-ward` Gτ guard for the action; populate `guard: GuardVerdict`
- [ ] **Step 6 — first-order consequences:** populate `consequences: Vec<Consequence>` with one-hop downstream outcomes from the recurrence graph (PH42 edges); each consequence's confidence = `confidence * hop_attenuation_factor` (default 0.7 per hop)
- [ ] **Step 7 — provenance:** write `LedgerRef` entry recording which constellations/kernel/edges grounded the prediction (A15); attach to `Prediction`
- [ ] Fail-closed: zero grounded recurrence edges → `Err(OracleError::NoRecurrence)` (not a zero-confidence prediction); `CALYX_ORACLE_NO_RECURRENCE`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: synthetic vault with 20 recurrence observations of `action_A→Pass`; `oracle_predict` returns `outcome = Pass`, `confidence ≤ ceiling = 0.95` (known self-consistency); confidence > 0.5
- [ ] unit: raw confidence 0.9, ceiling 0.7 → returned `confidence = 0.7` exactly (ceiling enforced)
- [ ] unit: `sufficient: false` panel → `oracle_predict` returns `Err(OracleError::Insufficient)` before any recurrence query
- [ ] proptest: for all inputs, `prediction.confidence ≤ prediction.bound.dpi_ceiling` always holds
- [ ] edge (≥3): no recurrence data → `CALYX_ORACLE_NO_RECURRENCE`; action with 1 observation → returns prediction with wide credible interval; all observations disagree (uniform) → confidence near 0
- [ ] fail-closed: ledger write failure → `Err(OracleError::LedgerWriteFailure)` with code; never silently omits provenance

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `Prediction` JSON returned by `oracle_predict`; the Ledger CF entry; the vault's recurrence edge CF
- **Readback:** `calyx readback oracle_predict --domain <real_domain> --action <action_id>` prints `Prediction` JSON; verify `confidence <= bound.dpi_ceiling` field-by-field; `xxd` Ledger CF row confirms provenance
- **Prove:** on SWE-bench Lite (real deterministic oracle domain), `confidence ≤ oracle_self_consistency.ceiling`; ceiling reads correctly from the grounded recurrence data; `guard` field is populated (not null/default)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
