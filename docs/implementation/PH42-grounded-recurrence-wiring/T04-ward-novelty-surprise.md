# PH42 · T04 — Ward: non-recurring = novelty; surprise `−log p` (never inflates bits)

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/novelty.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH38 (τ calibration + novelty detection) · PH41 (frequency) |
| **Axioms** | A29, A12 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/09 §5b` |

## Goal

Wire recurrence frequency into Ward's novelty/anomaly classification: a
non-recurring constellation (frequency = 0 or 1) arriving in a domain full of
recurring events is the highest-information event — an anomaly, highest bits,
immediate novelty signal (A29 §5). An overdue recurrence (expected event missing
past its cadence window) is also a novelty signal. Define the surprise term
`−log p` where `p = frequency / total_domain_events` — but codify the hard
constraint: this term is used ONLY for retrieval/anomaly scoring; it MUST NOT
inflate the stored information bits of any constellation (no bit-stuffing).

## Build (checklist of concrete, code-level steps)

- [ ] Define `NoveltySignal` enum: `Recurring { frequency: u64, cadence_secs: f64 }` | `NonRecurring` | `OverdueRecurrence { expected_t: EpochSecs, overdue_by_secs: u64 }` | `Anomaly { surprise_bits: f32 }`
- [ ] Implement `classify_novelty(cx_id: CxId, vault: &Vault, clock: &dyn Clock) -> Result<NoveltySignal, CalyxError>`:
  - read `frequency` from base CF (O(1))
  - if `frequency <= 1` → `NonRecurring` (singleton = novelty/highest-info event)
  - if `frequency >= 3` AND cadence known: check `clock.now_secs() > last_occurrence_t + 2 * cadence_secs` → `OverdueRecurrence { expected_t: last_occurrence_t + cadence_secs, overdue_by_secs }`
  - else → `Recurring`
- [ ] Implement `surprise_bits(cx_id: CxId, domain: &Domain, vault: &Vault) -> Result<f32, CalyxError>`:
  - `p = frequency(cx_id) / total_domain_events(domain)` — both from base CF; `total_domain_events` = sum of frequencies for all CxIds in domain
  - if `p = 0.0` → `p = 1.0 / total_domain_events` (Laplace smoothing)
  - `surprise = -p.ln() / 2f32.ln()` (bits, base-2 logarithm)
  - return `surprise`
- [ ] Hard constraint enforcement — codify as a type-level guarantee: `surprise_bits` returns a `SurpriseScore(f32)` newtype; this newtype has NO conversion to any type that touches stored constellation bits (no `Into<LensBits>`, no `Into<InformationScore>`). Add a lint comment: `// INVARIANT: SurpriseScore is for retrieval anomaly only; MUST NOT modify stored bits`
- [ ] Implement `overdue_recurrence_scan(domain: &Domain, vault: &Vault, clock: &dyn Clock) -> Vec<(CxId, NoveltySignal)>`: scan all recurring CxIds in domain; return those that are overdue
- [ ] Integrate `classify_novelty` result into Ward's existing novelty-region logic (PH38): `NonRecurring` maps to "new region" signal; `Anomaly` maps to "guard attention required"

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `frequency = 0` → `NonRecurring`
- [ ] unit: `frequency = 1` → `NonRecurring`
- [ ] unit: `frequency = 10`, cadence=100s, `last_occurrence = clock.now_secs() - 350s` (> 2×cadence) → `OverdueRecurrence { expected_t: last+100, overdue_by: 250 }`
- [ ] unit: `surprise_bits` for `frequency=1` in a domain of 100 events: `p = 1/100 = 0.01`, `surprise = -log2(0.01) ≈ 6.64 bits`
- [ ] unit: `surprise_bits` for `frequency=50` in domain of 100: `p=0.5`, `surprise = 1.0 bit`
- [ ] unit: `SurpriseScore` newtype has no `Into<LensBits>` impl — static assertion (compile-time)
- [ ] proptest: `surprise_bits` ≥ 0.0 for all valid inputs (information is non-negative)
- [ ] edge: `total_domain_events = 0` → Laplace smoothing: `p = 1.0/1 = 1.0`, `surprise = 0.0`
- [ ] fail-closed: `frequency` field missing from base CF → `CALYX_WARD_MISSING_FREQUENCY`; not treated as `frequency=0` silently (fail-closed, A16)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `NoveltySignal` returned by `classify_novelty`; Ward's novelty-region log
- **Readback:** (1) ingest a singleton CxId in a domain of 20 recurring events; `calyx readback ward-novelty --cx-id <singleton>` → print `NoveltySignal`; (2) inject a FixedClock past the cadence window for a recurring CxId; `calyx readback ward-novelty --cx-id <recurring>` → print `OverdueRecurrence`
- **Prove:** singleton → `NonRecurring` printed; overdue → `OverdueRecurrence { expected_t: ..., overdue_by: ... }` printed; `SurpriseScore` appears in anomaly log but NOT in any stored bits field (grep CF bytes for surprise value — must be absent)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
