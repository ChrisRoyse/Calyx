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

## Implementation note

T04 lands the Aster-level ingest facade over the storage surfaces that exist
today. `IngestInput` carries raw bytes plus measured slot vectors/scalars/anchors;
the higher panel/lens runtime still owns real embedding. `ingest_at` stamps the
caller-provided event time into `Constellation.created_at`, the
`event_time_secs` scalar, and the Ledger payload. Until T05 adds the dedicated
recurrence series store/readback, `RecurrenceSeries` writes bounded interim
`dedup:occurrence:<CxId><OccurrenceId>` records in the `online` CF; T05 replaces
that with the full series store.

## Build (checklist of concrete, code-level steps)

- [x] Implement `ingest_at(vault, input, at: EpochSecs, guard_profile) -> Result<DedupResult, CalyxError>`:
  - `IngestInput` carries raw bytes plus measured slots/anchors/scalars and builds a `Constellation` stamped with `at`
  - run `check_dedup` against `vault.dedup_policy()` without pre-store anchor side effects
  - branch on `DedupDecision`:
    - `NoMatch | AnchorConflict` → store as new, returning `DedupResult::New(cx_id)`
    - `Match + Exact` or same-CxId/same-event TctCosine → Ledger-log and return `ExactDuplicate(existing)`
    - `Match + Collapse` → no candidate base row; write collapse metadata + Ledger entry; return `DedupMerge`
    - `Match + Link` → store candidate + link metadata + Ledger entry; return `DedupMerge`
    - `Match + RecurrenceSeries` → append interim Online CF occurrence row + Ledger entry; return `DedupMerge`
  - all paths write a Ledger `Ingest` row in the same WAL group-commit as any base/online rows
- [x] `ingest` wrapper calls `ingest_at(..., at=clock.now()/1000, ...)`
- [x] `IngestInput` carries the raw embedding input; `EpochSecs = i64` (newtype)
- [x] Event time is caller-provided and stored in `created_at`, `event_time_secs`, interim occurrence rows, and Ledger payloads; full E2/E3/E4 panel-slot stamping remains with the panel runtime

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `DedupPolicy::Off` → distinct inputs with matching content vectors return `New(CxId)`
- [x] unit: `DedupPolicy::Exact` + ingest same bytes twice → second call returns `ExactDuplicate(first_id)`
- [x] unit: `TctCosine { action: RecurrenceSeries }` + content-identical input at different `at` times → later calls return `DedupMerge`
- [x] unit: `TctCosine { action: Collapse }` + match → candidate CxId absent from base CF
- [x] unit: `AnchorConflict` → second ingest returns `New(second_id)` (not merged)
- [x] unit: Ledger entry written for every call (ledger CF inspected after each ingest)
- [x] proptest: ingesting the same content N times with `RecurrenceSeries` → exactly N-1 `DedupMerge` + 1 `New`; interim series has N occurrences
- [x] edge: `at` in the far past (epoch 0) → valid, stored correctly; no clamping
- [x] edge: `at` in the future relative to `clock.now()` → allowed (event-time is caller-provided)
- [x] fail-closed: negative `EpochSecs` returns `CALYX_DEDUP_INVALID_EVENT_TIME` with no base/ledger rows

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Aster `base`, `online`, and `ledger` CF rows plus WAL/SST bytes.
- **Evidence root:** `/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`
  on aiwonder, code commit `1a0c5601134f8c36c7ce6885f047c759e9e85e25`.
- **Readback:** root was absent before trigger; `CALYX_DEDUP_INGEST_AT_FSV_ROOT`
  ran `cargo test -p calyx-cli --test dedup_ingest_at_readback -- --nocapture`
  to write the persisted vaults and `dedup-ingest-at-readback.json`
  (`BLAKE3=7f89caffebaae65958773f4db67f071acf9899db1807e9ba214015521fa13627`).
  Separate after-read used `/home/croyse/calyx/target/debug/calyx readback --cf
  base|online|ledger` against the recurrence, exact, anchor-conflict,
  event-time-edge, and negative-time vaults; `b3sum -c BLAKE3SUMS.txt` returned
  `OK` for every SST/WAL/manifest/readback artifact.
- **Prove:** exactly one logical base CxId; interim occurrence rows show
  `t_k = [100, 200, 300]`; Ledger has 3 entries with `DedupMerge` for entries
  2 and 3. Exact duplicate keeps one base row and writes a second Ledger entry;
  anchor conflict stores two base CxIds plus reciprocal `dedup:contested_with`
  rows; epoch 0 and 2100-01-01 event times persist; negative `EpochSecs` leaves
  base and ledger CFs empty. Dedicated `readback recurrence-series` remains T05.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
