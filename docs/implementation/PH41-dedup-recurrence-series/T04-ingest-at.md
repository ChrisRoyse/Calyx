# PH41 · T04 — `ingest_at(input, at: t)` → `New | DedupMerge{into, occurrence}`

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/ingest_at.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH09 (ingest path) |
| **Axioms** | A28, A15 |
| **PRD** | `dbprdplans/25 §8`, `dbprdplans/25 §5` |

## Goal

Implement `ingest_at(vault, input, at: t) -> Result<DedupResult, CalyxError>` —
the temporal ingest entry point that extends PH09's `ingest`. The timestamp `t`
is the event time (when the event happened, not when it is being ingested). On
every call: embed input via the panel, run `check_dedup`, and branch: `NoMatch`
→ store as new constellation (returning `New(CxId)`); `Match + action=Collapse |
Link` → merge/link; `Match + action=RecurrenceSeries` → append occurrence to the
recurrence series (returning `DedupMerge { into, occurrence }`). `AnchorConflict`
→ store as new constellation. Every path writes a Ledger entry (A15).

## Build (checklist of concrete, code-level steps)

- [ ] Implement `ingest_at(vault: &mut Vault, input: &IngestInput, at: EpochSecs, clock: &dyn Clock) -> Result<DedupResult, CalyxError>`:
  - embed input via vault's panel → `Constellation` with temporal slots stamped with `at` (not `clock.now_secs()`)
  - run `check_dedup(new_cx, vault, vault.dedup_policy(), guard_profile)` (T02/T03)
  - branch on `DedupDecision`:
    - `NoMatch | AnchorConflict` → call PH09 `store_constellation` → return `DedupResult::New(cx_id)`
    - `Match + Exact` → return `DedupResult::ExactDuplicate(existing)`
    - `Match + Collapse` → write merged constellation (content from existing, occurrence appended) → Ledger entry → return `DedupResult::DedupMerge { into: existing, occurrence: new_occ_id }`
    - `Match + Link` → store both + link record → Ledger entry → return `DedupMerge`
    - `Match + RecurrenceSeries` → call `series_store.append_occurrence(existing, at, context)` (T05) → Ledger entry → return `DedupMerge { into: existing, occurrence }`
  - All paths: write `LedgerEntry::Ingest { cx_id, at, dedup_decision }` in the same WAL group-commit as the store operation (A15)
- [ ] `ingest` (PH09 entry point) becomes `ingest_at(vault, input, at=clock.now_secs(), clock)` — thin wrapper
- [ ] `IngestInput` carries the raw embedding input; `EpochSecs = i64` (newtype)
- [ ] Stamp temporal slots E2/E3/E4 with `at` (not clock.now_secs) — event-time, not ingest-time (critical for correct E2 decay scoring)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `DedupPolicy::Off` → `ingest_at` always returns `New(CxId)` regardless of content similarity
- [ ] unit: `DedupPolicy::Exact` + ingest same bytes twice → second call returns `ExactDuplicate(first_id)`
- [ ] unit: `TctCosine { action: RecurrenceSeries }` + two content-identical constellations at different `at` times → second returns `DedupMerge { into: first_id, occurrence: occ_1 }`
- [ ] unit: `TctCosine { action: Collapse }` + match → merged constellation in CF; `New` CxId absent
- [ ] unit: `AnchorConflict` → second ingest returns `New(second_id)` (not merged)
- [ ] unit: Ledger entry written for every call (inspect ledger CF after each ingest)
- [ ] proptest: ingesting the same content N times with `RecurrenceSeries` → exactly N-1 `DedupMerge` + 1 `New`; series has N-1 occurrences
- [ ] edge: `at` in the far past (epoch 0) → valid, stored correctly; no clamping
- [ ] edge: `at` in the future relative to `clock.now_secs()` → allowed (event-time is caller-provided)
- [ ] fail-closed: Ledger write fails → entire `ingest_at` returns error; no partial state (WAL atomicity)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF rows for constellations + Ledger CF + recurrence series store
- **Readback:** `calyx readback cx-list` after three `ingest_at` calls on the same content at t=100, t=200, t=300 with `RecurrenceSeries` policy; then `calyx readback recurrence-series <CxId>` to show 3 occurrences; then `calyx readback ledger --cx-id <CxId>` to show 3 Ledger entries
- **Prove:** exactly one CxId in cx-list (not three); recurrence-series shows `t_k = [100, 200, 300]`; ledger shows 3 entries with `dedup_decision = DedupMerge` for entries 2 and 3

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
