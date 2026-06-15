# PH24 · T07 — Multi-lens recall FSV on real qrels (BEIR SciFact)

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/tests/stage4_real_qrels_fsv.rs` (≤500) |
| **Depends on** | T06 (this phase) · PH17–PH22 (lens runtimes, TEI :8088) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/14 §2`, `dbprdplans/19 §4` |

## Goal

The PH24 exit gate: prove that multi-lens RRF recall@10 ≥ single-lens recall@10
+ Δ where Δ ≥ 0.15 (15 percentage points) on a real labeled corpus (BEIR
SciFact subset on aiwonder). Every `Hit` returned must carry a non-zero
`LedgerRef`; real hash-chain provenance remains PH35/Stage 7.
This is also the recommended-first-demo checkpoint (`19 §2`): at this point Calyx
can answer a real vault with multiple lenses and provenance.

## Build (checklist of concrete, code-level steps)

- [x] `tests/stage4_real_qrels_fsv.rs` harness:
      1. Load the BEIR SciFact qrels subset from `CALYX_QRELS_ROOT`
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
- [x] Mark test `#[ignore]` — requires aiwonder + TEI + dataset; not a unit test
- [x] If `CALYX_QRELS_ROOT` is absent, the test
      prints `SKIP: dataset not found` and exits with code 0 (not a failure on
      dev machines without the dataset)
- [x] Companion README note: "Completing PH24 + migration shadow = recommended
      first demo (`19 §2`)"; point to PH64 for the migration shadow

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] integration (real qrels on aiwonder): `delta >= 0.15` — the primary gate
- [x] integration: all returned hits have `provenance != LedgerRef::zero()`
- [x] unit (always runs): `compute_recall_at_k(results, relevant, k=10)` correct
      for hand-crafted inputs: `results=[1,2,3], relevant={1,4} → recall=0.5`
- [x] unit: `compute_recall_at_k` with empty relevant set → 0.0 (not NaN)
- [x] unit: `compute_recall_at_k` with all results relevant → 1.0
- [x] edge: qrels file missing → `SKIP` message, exit 0 (not panic)
- [x] fail-closed: if delta < 0.15 on the real run, test fails with
      `assert!(delta >= 0.15, "multi-lens recall delta={delta} < 0.15")` — no
      silent pass

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of
  `cargo test -p calyx-sextant --test stage4_real_qrels_fsv beir_scifact_rrf_beats_single_lens_qrels -- --ignored --nocapture`
  on aiwonder with the BEIR SciFact subset under `CALYX_QRELS_ROOT`
- **Readback:** same command with output captured under
  `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`
- **Prove:** must print `single_lens_recall@10=NNN rrf_recall@10=NNN delta=NNN provenance_ok=true`
  where delta ≥ 0.15 and provenance_ok=true; screenshot or copy of this line
  plus one `LedgerRef` hex value attached to the PH24 GitHub issue as FSV evidence

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
