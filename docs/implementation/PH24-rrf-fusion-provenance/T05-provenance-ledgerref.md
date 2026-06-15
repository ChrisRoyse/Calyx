# PH24 · T05 — Provenance: attach `LedgerRef` + freshness to every `Hit`

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/search.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH09 (Aster MVCC seq reads) · PH35 stub (`LedgerRef`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §5`, `dbprdplans/11` |

## Goal

Every `Hit` must carry a non-zero `LedgerRef` and a populated `FreshnessTag`.
Current Stage 4 code uses stored `Constellation.provenance` when present and a
deterministic `stub_ledger` fallback for rows without stored provenance. The
real input-to-lens-to-vector-to-answer hash-chain remains PH35/Stage 7. After
this card, no `Hit` ever has a zero/default provenance.

## Build (checklist of concrete, code-level steps)

- [x] `crates/calyx-sextant/src/search.rs`:
  ```rust
  pub fn search(
      query: &Query,
      map: &SlotIndexMap,
      embedder: &dyn EmbedQuery,   // thin trait: text/anchor -> per-slot Vec<f32>
      ledger: &dyn LedgerProvider, // stub until PH35; real after
      clock: &dyn Clock,
  ) -> Result<Vec<Hit>, CalyxError>
  ```
- [x] `EmbedQuery` trait in `search.rs`:
      `fn embed(&self, input: &QueryInput, slot: SlotId) -> Result<Vec<f32>, CalyxError>`
      (calls the registered Lens via PH20 registry, or uses a pre-supplied vector)
- [x] `LedgerProvider` trait: `fn ref_for(&self, cx_id: CxId) -> LedgerRef`
      — in the stub, returns `LedgerRef::stub(cx_id, current_seq)`;
      real implementation after PH35
- [x] After fusion returns raw `Hit`s, iterate and set:
      - `hit.provenance = ledger.ref_for(hit.cx_id)`
      - `hit.freshness.built_at_seq = current_seq` (from `clock` or Aster snapshot)
      - `hit.freshness.stale_by = None` (FreshDerived) or computed from lag
- [x] `FreshnessPolicy::StaleOk { seq_lag }` in `Query` → set
      `stale_by = Some(built_at_seq + seq_lag)` on each `Hit`
- [x] `CALYX_SEXTANT_EMBED_FAILED` if `EmbedQuery::embed` returns error
- [x] `CALYX_SEXTANT_LEDGER_UNAVAILABLE` if `LedgerProvider` returns a fatal error

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `search()` with stub ledger → every `Hit` has `provenance ≠ LedgerRef::zero()`
- [x] unit: `FreshnessPolicy::FreshDerived` → `hit.freshness.stale_by == None`
- [x] unit: `FreshnessPolicy::StaleOk { seq_lag: 100 }` → `hit.freshness.stale_by == Some(built_at_seq + 100)`
- [x] unit: two different cx_ids → two different `LedgerRef`s (stub encodes cx_id)
- [x] proptest: for any query, all returned hits have `provenance != LedgerRef::zero()`
- [x] edge: `EmbedQuery` returns error → `CALYX_SEXTANT_EMBED_FAILED`, no partial hits returned
- [x] fail-closed: empty fusion result → `Ok(vec![])`, not an error (valid empty answer)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output + manual inspection of a returned `Hit`'s provenance field
- **Readback:** `cargo test -p calyx-sextant provenance -- --nocapture 2>&1`
- **Prove:** test prints each `Hit`'s `provenance` as hex; the non-zero invariant
  test prints `all_provenanced=true`; the exact hex of one provenance stub is
  captured and attached to the PH24 GitHub issue (proves the field is populated,
  not default-zeroed)

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
