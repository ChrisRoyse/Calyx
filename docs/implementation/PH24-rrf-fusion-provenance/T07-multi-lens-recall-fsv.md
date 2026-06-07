# PH24 · T07 — Multi-lens recall FSV on real qrels (BEIR/MS MARCO)

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/tests/fusion_recall.rs` (≤500) |
| **Depends on** | T06 (this phase) · PH17–PH22 (lens runtimes, TEI :8088) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/14 §2`, `dbprdplans/19 §4` |

## Goal

The PH24 exit gate: prove that multi-lens RRF recall@10 ≥ single-lens recall@10
+ Δ where Δ ≥ 0.15 (15 percentage points) on a real labeled corpus (BEIR/MS MARCO
subset on aiwonder). Every `Hit` returned must carry a non-zero `LedgerRef`.
This is also the recommended-first-demo checkpoint (`19 §2`): at this point Calyx
can answer a real vault with multiple lenses and provenance.

## Build (checklist of concrete, code-level steps)

- [ ] `tests/fusion_recall.rs` harness:
      1. Load the BEIR/MS MARCO qrels subset from
         `$CALYX_HOME/datasets/msmarco_qrels_1k.jsonl` (1000 queries + relevant
         doc IDs; created by PH69 or a synthetic stand-in for dev)
      2. Ingest the document set into an in-process vault using `calyx-aster`
         (two slots: dense GTE-small via :8088 + sparse BM25 placeholder via
         a no-op slot for this phase, real sparse in PH25)
      3. For each query: run `SingleLens(dense_slot)` → compute recall@10 vs qrels;
         run `Rrf` with both slots → compute recall@10 vs qrels
      4. Compute `delta = rrf_recall_mean - single_recall_mean`
      5. Assert `delta >= 0.15`
      6. Spot-check: for the first 5 hits of the first query, assert
         `hit.provenance != LedgerRef::zero()`
      7. Print:
         ```
         single_lens_recall@10=NNN rrf_recall@10=NNN delta=NNN provenance_ok=true
         ```
- [ ] Mark test `#[ignore]` — requires aiwonder + TEI + dataset; not a unit test
- [ ] If `$CALYX_HOME/datasets/msmarco_qrels_1k.jsonl` is absent, the test
      prints `SKIP: dataset not found` and exits with code 0 (not a failure on
      dev machines without the dataset)
- [ ] Companion README note: "Completing PH24 + migration shadow = recommended
      first demo (`19 §2`)"; point to PH64 for the migration shadow

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration (real qrels on aiwonder): `delta >= 0.15` — the primary gate
- [ ] integration: all returned hits have `provenance != LedgerRef::zero()`
- [ ] unit (always runs): `compute_recall_at_k(results, relevant, k=10)` correct
      for hand-crafted inputs: `results=[1,2,3], relevant={1,4} → recall=0.5`
- [ ] unit: `compute_recall_at_k` with empty relevant set → 0.0 (not NaN)
- [ ] unit: `compute_recall_at_k` with all results relevant → 1.0
- [ ] edge: qrels file missing → `SKIP` message, exit 0 (not panic)
- [ ] fail-closed: if delta < 0.15 on the real run, test fails with
      `assert!(delta >= 0.15, "multi-lens recall delta={delta} < 0.15")` — no
      silent pass

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-sextant fusion_recall -- --nocapture --ignored`
  on aiwonder with the BEIR/MS MARCO subset in `$CALYX_HOME/datasets/`
- **Readback:** `cargo test -p calyx-sextant fusion_recall -- --nocapture --ignored 2>&1 | grep -E 'recall|delta|provenance'`
- **Prove:** must print `single_lens_recall@10=NNN rrf_recall@10=NNN delta=NNN provenance_ok=true`
  where delta ≥ 0.15 and provenance_ok=true; screenshot or copy of this line
  plus one `LedgerRef` hex value attached to the PH24 GitHub issue as FSV evidence

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
