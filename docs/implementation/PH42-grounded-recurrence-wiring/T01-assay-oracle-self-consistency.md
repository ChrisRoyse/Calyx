# PH42 · T01 — Assay: frequency as grounded anchor + `oracle_self_consistency`

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/recurrence_anchor.rs` (≤500) |
| **Depends on** | PH41 (recurrence series + frequency) · PH28 (KSG MI / NMI) |
| **Axioms** | A29, A2, A20 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/07 §3b` |

## Goal

Expose frequency as a grounded anchor in Assay — a count of what actually
happened is reality, not a learned vector (A2). Implement
`oracle_self_consistency(domain: &Domain, vault: &Vault) -> Result<f32, CalyxError>`
which measures whether recurring events within a domain produce agreeing or
differing outcomes: events with the same recurrence signature whose observed
outcome anchors agree → consistent (score → 1.0); differing outcomes → flaky
(score → 0.0). This scalar is the ceiling `τ_corr` used by the Oracle (PH49) and
must be measured natively from the recurrence series.

## Build (checklist of concrete, code-level steps)

- [ ] Define `RecurrenceAnchor { cx_id: CxId, frequency: u64, cadence_secs: Option<f64> }` — read from base CF `frequency` field (O(1)); never recomputed from series
- [ ] Implement `frequency_anchor_for(cx_id: CxId, vault: &Vault) -> Result<RecurrenceAnchor, CalyxError>`: read `frequency` from base CF; return `RecurrenceAnchor`
- [ ] Define `OutcomeAgreement` enum: `Consistent { agreement_rate: f32 }` | `Flaky { agreement_rate: f32 }` | `Insufficient { n: usize }` (n < 3 recurring occurrences → insufficient data)
- [ ] Implement `measure_outcome_agreement(cx_id: CxId, vault: &Vault) -> Result<OutcomeAgreement, CalyxError>`:
  - read `RecurrenceSeries` for `cx_id`; if `occurrences.len() < 3` → `Insufficient { n }`
  - for each pair of occurrences: compare the outcome anchor (a specific named anchor slot, e.g., `OutcomeAnchor` slot); count agreeing pairs (same anchor value) vs total pairs
  - `agreement_rate = agreeing_pairs / total_pairs`
  - `agreement_rate ≥ 0.75` → `Consistent`; else → `Flaky`
- [ ] Implement `oracle_self_consistency(domain: &Domain, vault: &Vault) -> Result<f32, CalyxError>`:
  - collect all CxIds in the domain that have `frequency ≥ 3` (recurring)
  - for each: call `measure_outcome_agreement`; collect `agreement_rate` values
  - if none → return `1.0` (unknown → permissive); if some → return `mean(agreement_rates)`
  - this scalar is the floor of the Oracle's confidence ceiling: `oracle_conf ≤ self_consistency`
- [ ] `agree → consistent, differ → flaky/ceiling drops` — codify: `Flaky` outcome lowers the Oracle ceiling for that domain to `agreement_rate`
- [ ] Expose `oracle_self_consistency` from `calyx-assay` lib root

## Implementation notes

- Issue #387 implements this as a PH42-local Assay surface in `crates/calyx-assay/src/recurrence_anchor.rs`.
- `frequency_anchor_for` reads the latest persisted base-CF `recurrence.frequency` scalar in O(1); `cadence_secs` remains `None` because cadence is derived from the recurrence series elsewhere.
- Outcome evidence is encoded in bounded `OccurrenceContext.bytes` JSON under `outcome_anchor`; the default wire shape for the common text case is `{"outcome_anchor":{"kind":{"label":"OutcomeAnchor"},"value":{"text":"agree"}}}`.
- Legacy non-JSON occurrence contexts are treated as no observed outcome; malformed or wrong-anchor evidence fails closed with `CALYX_ASSAY_MISSING_OUTCOME_SLOT`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: CxId with 5 occurrences, all outcome anchors identical → `Consistent { agreement_rate: 1.0 }`
- [ ] unit: CxId with 6 occurrences, 3 agree/3 disagree → `agreement_rate = C(3,2)/C(6,2) = 3/15 = 0.20` → `Flaky`
- [ ] unit: CxId with 2 occurrences → `Insufficient { n: 2 }`
- [ ] unit: `oracle_self_consistency` on a domain with 3 CxIds: rates [1.0, 0.9, 0.8] → mean = 0.90
- [ ] unit: `oracle_self_consistency` on a domain with no recurring CxIds → `1.0`
- [ ] unit: `frequency_anchor_for` reads from base CF, not series scan → O(1) (mock CF, assert no series read)
- [ ] proptest: `agreement_rate ∈ [0.0, 1.0]` for all valid inputs
- [ ] edge: all occurrences have no `OutcomeAnchor` slot → `Consistent { 1.0 }` (absence of disagreement is agreement)
- [ ] fail-closed: `OutcomeAnchor` slot missing from panel → `CALYX_ASSAY_MISSING_OUTCOME_SLOT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `oracle_self_consistency` scalar in persisted Assay report JSON, readable via `calyx readback assay-report --artifact <assay-report.json>`
- **Readback:** create a domain with 5 recurring CxIds (all same action, 3 occurrences each), split: 3 CxIds have agreeing outcomes, 2 have differing. Persist the Assay report JSON, then run `calyx readback assay-report --artifact <assay-report.json> --field oracle_self_consistency`
- **Prove:** `oracle_self_consistency` value printed is between 0.6 and 0.9 (mixed agreeing/flaky corpus); `Consistent` CxIds show `agreement_rate ≥ 0.75`; `Flaky` CxIds show `agreement_rate < 0.75`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
