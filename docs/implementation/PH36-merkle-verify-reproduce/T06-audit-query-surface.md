# PH36 · T06 — Audit query surface: `get_provenance`, `get_answer_trace`, `audit(filter)`

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/lib.rs` (≤500, re-exports + thin wrappers) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §5`, `11 §7` |

## Goal

Expose the full public audit API defined in `11 §5` so that calyx-mcp, calyx-cli,
and downstream crates (PH61, PH62, PH63) have a stable, typed surface to query
ledger provenance. Every function that returns results from a quarantined range
must return `CALYX_LEDGER_CHAIN_BROKEN` (fail-closed). Every "trusted" result
that cannot be traced to a ledger entry must be tagged `unprovenanced`.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn get_provenance(cf_reader, cx_id: CxId) -> Result<Vec<LedgerEntry>>` —
  returns all entries whose `subject` matches `cx_id` (Ingest → Measure →
  Assay → Guard → Answer chain); checks quarantine before returning.
- [ ] `pub fn get_answer_trace(cf_reader, answer_id: QueryId) -> Result<AnswerTrace>` —
  `AnswerTrace = { answer_entry, path: Vec<(CxId, hop: u32, score: f32, lens_id)>, fusion_weights, guard_result, freshness_ts }`;
  decoded from the `Answer` and linked `Guard`/`Kernel` entries; checks quarantine.
- [ ] `pub fn verify_chain(cf_reader, manifest, range: KeyRange) -> Result<VerifyResult>` —
  re-export from `verify.rs` (T02); included here for the stable public API.
- [ ] `pub fn merkle_root(cf_reader, range: KeyRange) -> Result<[u8;32]>` —
  re-export from `merkle.rs` (T01).
- [ ] `pub fn reproduce(cf_reader, registry, forge, answer_id) -> Result<ReproduceResult>` —
  re-export from `reproduce.rs` (T05).
- [ ] `pub fn audit(cf_reader, filter: AuditFilter) -> Result<Vec<LedgerEntry>>` —
  `AuditFilter = { kind: Option<EntryKind>, actor: Option<ActorId>, ts_range: Option<(u64,u64)>, seq_range: Option<(u64,u64)> }`;
  iterates ledger CF with filter applied; skips quarantined ranges with a log
  warning (does not silently include them).
- [ ] All six functions check quarantine via `is_quarantined` before serving
  results; return `CALYX_LEDGER_CHAIN_BROKEN` if any requested seq falls in a
  quarantined range.
- [ ] `unprovenanced` tagging: add `CalyxWarning::Unprovenanced { surface: String }`
  (non-fatal) to `calyx-core` for callers that need to label results lacking
  a ledger entry.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: ingest 3 constellations → `get_provenance(cx_id[0])` returns
  exactly the Ingest entry for `cx_id[0]`; no entries from other cx_ids.
- [ ] unit: write an Answer entry with a known path → `get_answer_trace(answer_id)`
  returns the correct hop count and fusion_weights byte-exact.
- [ ] unit: `audit(AuditFilter { kind: Some(Ingest), .. })` over 10 entries
  (5 Ingest, 5 Measure) returns exactly 5 entries.
- [ ] edge (≥3): `get_provenance` for a cx_id with no entries → `Ok(vec![])`;
  `get_answer_trace` for a quarantined seq → `CALYX_LEDGER_CHAIN_BROKEN`;
  `audit` with `ts_range` that excludes all entries → `Ok(vec![])`.
- [ ] fail-closed: `get_provenance` with any requested seq in a quarantined range
  → `CALYX_LEDGER_CHAIN_BROKEN` (not a partial result); `audit` with a
  quarantined range in `seq_range` → same error, not a silent skip.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx` CLI commands on aiwonder after a real ingest + answer run
- **Readback:**
  1. `calyx get-provenance --vault test --cx <cx_id>` → prints the Ingest
     entry and any Measure/Assay/Guard entries for that cx_id.
  2. `calyx get-answer-trace --vault test --answer <answer_id>` → prints the
     ordered path with cx_ids, hops, scores, lens_ids, and fusion_weights.
  3. `calyx audit --vault test --kind Ingest | wc -l` → matches the number of
     ingested constellations.
- **Prove:** provenance chain printed covers ingest → measure → answer with no
  gaps; quarantined-range query returns `CALYX_LEDGER_CHAIN_BROKEN` immediately.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
