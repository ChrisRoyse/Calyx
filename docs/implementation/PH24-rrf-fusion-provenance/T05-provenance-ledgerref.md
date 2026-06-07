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

Every `Hit` must carry a real `LedgerRef` (input → lens → vector → answer
provenance chain, A15) and a populated `FreshnessTag`. This card wires the
top-level `search()` function that calls fusion, then enriches each `Hit` with
provenance from the Ledger (stub until PH35) and freshness from the current
MVCC seq. After this card, no `Hit` ever has a zero/default provenance.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyx-sextant/src/search.rs`:
  ```rust
  pub fn search(
      query: &Query,
      map: &SlotIndexMap,
      embedder: &dyn EmbedQuery,   // thin trait: text/anchor -> per-slot Vec<f32>
      ledger: &dyn LedgerProvider, // stub until PH35; real after
      clock: &dyn Clock,
  ) -> Result<Vec<Hit>, CalyxError>
  ```
- [ ] `EmbedQuery` trait in `search.rs`:
      `fn embed(&self, input: &QueryInput, slot: SlotId) -> Result<Vec<f32>, CalyxError>`
      (calls the registered Lens via PH20 registry, or uses a pre-supplied vector)
- [ ] `LedgerProvider` trait: `fn ref_for(&self, cx_id: CxId) -> LedgerRef`
      — in the stub, returns `LedgerRef::stub(cx_id, current_seq)`;
      real implementation after PH35
- [ ] After fusion returns raw `Hit`s, iterate and set:
      - `hit.provenance = ledger.ref_for(hit.cx_id)`
      - `hit.freshness.built_at_seq = current_seq` (from `clock` or Aster snapshot)
      - `hit.freshness.stale_by = None` (FreshDerived) or computed from lag
- [ ] `FreshnessPolicy::StaleOk { seq_lag }` in `Query` → set
      `stale_by = Some(built_at_seq + seq_lag)` on each `Hit`
- [ ] `CALYX_SEXTANT_EMBED_FAILED` if `EmbedQuery::embed` returns error
- [ ] `CALYX_SEXTANT_LEDGER_UNAVAILABLE` if `LedgerProvider` returns a fatal error

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `search()` with stub ledger → every `Hit` has `provenance ≠ LedgerRef::zero()`
- [ ] unit: `FreshnessPolicy::FreshDerived` → `hit.freshness.stale_by == None`
- [ ] unit: `FreshnessPolicy::StaleOk { seq_lag: 100 }` → `hit.freshness.stale_by == Some(built_at_seq + 100)`
- [ ] unit: two different cx_ids → two different `LedgerRef`s (stub encodes cx_id)
- [ ] proptest: for any query, all returned hits have `provenance != LedgerRef::zero()`
- [ ] edge: `EmbedQuery` returns error → `CALYX_SEXTANT_EMBED_FAILED`, no partial hits returned
- [ ] fail-closed: empty fusion result → `Ok(vec![])`, not an error (valid empty answer)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output + manual inspection of a returned `Hit`'s provenance field
- **Readback:** `cargo test -p calyx-sextant provenance -- --nocapture 2>&1`
- **Prove:** test prints each `Hit`'s `provenance` as hex; the non-zero invariant
  test prints `all_provenanced=true`; the exact hex of one provenance stub is
  captured and attached to the PH24 GitHub issue (proves the field is populated,
  not default-zeroed)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
