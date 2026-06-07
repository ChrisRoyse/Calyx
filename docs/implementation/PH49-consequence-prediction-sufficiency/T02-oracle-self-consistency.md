# PH49 · T02 — `oracle_self_consistency` from grounded recurrence

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/self_consistency.rs` (≤500) |
| **Depends on** | T01 (types), PH42 (grounded recurrence wiring), PH28 (KSG MI) |
| **Axioms** | A20, A29, A2 |
| **PRD** | `dbprdplans/21 §1`, `dbprdplans/21 §2` (`07 §3b` cited in stage file) |

## Goal

Implement `oracle_self_consistency(domain) -> OracleSelfConsistency` that measures
**flakiness** (does the oracle reproduce its verdict when run twice on the same input?)
and **validity** (does the verdict track the property of interest?) from grounded
recurrence streams. The ceiling `= validity * (1.0 - flakiness)` is the hard cap on
every downstream confidence value. "Knows what will happen = extrapolating measured
recurrence, never a fabricated rate" (`21 §2`).

## Build (checklist of concrete, code-level steps)

- [ ] `fn oracle_self_consistency(vault: &Vault, domain: DomainId, clock: &dyn Clock) -> Result<OracleSelfConsistency, OracleError>` — takes an injected `Clock` (never `SystemTime::now()` in logic)
- [ ] Query grounded recurrence series for `domain` via PH42 recurrence API: retrieve pairs of oracle verdicts for the same item observed at different times
- [ ] **Flakiness:** for each item with ≥2 oracle observations, compare verdict at time t1 vs t2; `flakiness = 1.0 - (# agreement pairs / # total pairs)`; if fewer than `MIN_QUORUM` (= 10) pairs, emit `CALYX_ORACLE_NO_RECURRENCE` and return `Err`
- [ ] **Validity:** compute `I(oracle_verdict; ground_truth_anchor)` using the Assay KSG estimator (PH28); normalize to `[0.0, 1.0]` as `validity = I / H(outcome)`; if no ground-truth anchor available, `validity = 0.0` with `provisional: true` flag
- [ ] **Ceiling:** `ceiling = validity * (1.0 - flakiness)` — two independently-violable axes (`21 §1`)
- [ ] Write a `LedgerRef` entry for the consistency measurement (A15: every mutation writes provenance); return it in an associated struct
- [ ] Respect `MIN_QUORUM = 10` pairs for flakiness; `MIN_QUORUM = 50` for KSG validity (A16 fail-closed below quorum, same as PH28)
- [ ] Result is deterministic given the same recurrence data and seeded RNG for KSG; inject seed via `Clock` context

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: synthetic recurrence series with 20 pairs, 18 agreements → `flakiness = 0.10 ± 0.001`; `ceiling ≤ validity`
- [ ] unit: 50-pair synthetic where oracle always agrees with ground truth → `validity ≈ 1.0`; `ceiling ≈ 0.9` (accounting for `flakiness = 0.1`)
- [ ] proptest: `ceiling ≤ min(validity, 1.0 - flakiness)` for all valid input combinations; `ceiling ∈ [0.0, 1.0]`
- [ ] edge (≥3): 0 recurrence pairs → `CALYX_ORACLE_NO_RECURRENCE`; 9 pairs (below quorum) → same error; all pairs agree but validity = 0 (no anchor) → `ceiling = 0.0`
- [ ] fail-closed: below flakiness quorum → `Err(OracleError::NoRecurrence { domain })` with correct code; no silent zero-fill

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/self_consistency.rs`; recurrence CF in the Aster vault; Ledger CF entry for the measurement
- **Readback:** `calyx readback oracle_self_consistency <domain>` prints `{flakiness, validity, ceiling}` JSON; `xxd` the Ledger CF row to confirm provenance written
- **Prove:** on a real domain with known recurrence data, ceiling matches hand-computed `validity * (1 - flakiness)`; below-quorum domain emits `CALYX_ORACLE_NO_RECURRENCE` in the readback output

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
