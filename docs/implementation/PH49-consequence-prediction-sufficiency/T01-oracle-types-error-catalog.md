# PH49 · T01 — Oracle types + error catalog

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/types.rs` (≤500), `crates/calyx-oracle/src/error.rs` (≤500), `crates/calyx-oracle/src/lib.rs` (≤500) |
| **Depends on** | — |
| **Axioms** | A20, A2, A16 |
| **PRD** | `dbprdplans/21 §2`, `dbprdplans/21 §9` |

## Goal

Define the complete set of types that the Oracle API operates over — `Prediction`,
`Consequence`, `SufficiencyBound`, `OracleSelfConsistency`, `ConsequenceTree`,
`OracleError` — and the error catalog with every `CALYX_ORACLE_*` code. These types
are the contract between all PH49 modules and downstream phases (PH50, PH51).

## Build (checklist of concrete, code-level steps)

- [ ] `struct Prediction { outcome: AnchorValue, confidence: f32, consequences: Vec<Consequence>, bound: SufficiencyBound, provenance: LedgerRef, guard: GuardVerdict }` — verbatim from `21 §2`
- [ ] `struct SufficiencyBound { I_panel_oracle: f32, dpi_ceiling: f32, sufficient: bool, per_sensor_deficit: Vec<(LensId, f32)> }` — `per_sensor_deficit` lists each lens's contribution gap
- [ ] `struct OracleSelfConsistency { flakiness: f32, validity: f32, ceiling: f32 }` — `ceiling = validity * (1.0 - flakiness)`; both components independently measured (`21 §1`)
- [ ] `struct Consequence { action_or_event: String, domain: DomainId, outcome: AnchorValue, confidence: f32, hop: u8, provenance: LedgerRef }` — `hop` = depth in butterfly tree; confidence attenuated per hop
- [ ] `struct ConsequenceTree { root: Consequence, children: Vec<ConsequenceTree>, max_depth: u8 }` — bounded tree; `max_depth` default 4
- [ ] `enum OracleError` with variants: `Insufficient { bound: SufficiencyBound }`, `FlakyAnchor { self_consistency: f32 }`, `NoRecurrence { domain: DomainId }`, `DomainNotFound`, `LedgerWriteFailure`
- [ ] Error catalog constants: `CALYX_ORACLE_INSUFFICIENT`, `CALYX_ORACLE_FLAKY_ANCHOR`, `CALYX_ORACLE_NO_RECURRENCE` as `&'static str` codes following the `CALYX_*` convention (A16)
- [ ] `impl Display + std::error::Error` for `OracleError`; each variant includes remediation text (A16)
- [ ] All types `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`; `Prediction` implements `PartialEq` for test assertions
- [ ] Crate root `lib.rs` declares all submodules; re-exports `Prediction`, `OracleError`, `SufficiencyBound`, `OracleSelfConsistency`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct a `Prediction` with known fields; `serde_json::to_string` + round-trip `from_str` yields byte-identical struct (A2: every value a claim, readback confirms)
- [ ] unit: `OracleSelfConsistency::ceiling` = `validity * (1.0 - flakiness)` for three representative (validity, flakiness) pairs: `(1.0, 0.0)→1.0`, `(0.8, 0.1)→0.72`, `(0.5, 0.5)→0.25`
- [ ] proptest: `ceiling` ∈ `[0.0, 1.0]` for any `validity, flakiness ∈ [0.0, 1.0]`
- [ ] edge (≥3): `per_sensor_deficit` empty → `SufficiencyBound` still serializes; `hop = 0` on root `Consequence`; `max_depth = 0` on `ConsequenceTree` → children empty
- [ ] fail-closed: `OracleError::Insufficient` Display contains `CALYX_ORACLE_INSUFFICIENT` code; `OracleError::FlakyAnchor` contains `CALYX_ORACLE_FLAKY_ANCHOR`; remediation text non-empty on each variant

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/types.rs` + `crates/calyx-oracle/src/error.rs`
- **Readback:** `cargo test -p calyx-oracle -- types` prints test results; `grep CALYX_ORACLE_ crates/calyx-oracle/src/error.rs` shows all three codes
- **Prove:** all three `CALYX_ORACLE_*` codes are present in source; round-trip serde test passes; `OracleSelfConsistency::ceiling` arithmetic correct on three known inputs

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
