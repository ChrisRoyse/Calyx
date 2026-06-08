# PH33 · T05 — FSV: kernel-only recall ≥ 0.95·full on ≥3 real corpora (aiwonder)

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/tests/fsv_recall_real_corpora.rs` (≤500) |
| **Depends on** | T04 (recall test harness complete), real corpus paths verified on aiwonder |
| **Axioms** | A10, A11 |
| **PRD** | `dbprdplans/08 §3` (Stage 5), `08 §7` |

## Goal

Run the `kernel_recall_test` on at least 3 real corpora on aiwonder (one each of
text, code, and graph modality), prove that
**kernel-only recall ≥ 0.95·full** on each, and attach the evidence to the PH33
GitHub issue. Also verify that `grounding_gaps` lists exactly the unanchored kernel
members on a corpus where the anchor set is known. This is the byte-level FSV gate
for PH33 (not a unit test — a real on-device integration run).

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests/fsv_recall_real_corpora.rs`; gated with `#[cfg(feature = "fsv")]`
  so it does not run in CI (aiwonder-only).
- [ ] Load each corpus from a verified aiwonder path (`$CALYX_HOME/datasets/<name>/`
  or `$CALYX_HOME/data/datasets/<name>/`); if a required corpus is missing,
  acquire it, record its source/checksum, and read the files back before use.
- [ ] For each corpus: build or load the `Kernel` (via `build_kernel_pipeline`);
  build the `KernelIndex` and a full HNSW reference index; run `kernel_recall_test`
  with `rng_seed=42`, `top_k=10`, `held_out_fraction=0.10`.
- [ ] Assert `ratio >= 0.95` for each corpus; print the full `RecallReport` JSON.
- [ ] Run `grounding_gaps` on the text corpus (which has known anchors from PH09);
  print the gap list; manually cross-check at least 3 reported gaps.
- [ ] Write the three `RecallReport` JSONs and the gap list to
  `$CALYX_HOME/fsv/ph33_recall_<corpus_name>_<date>.json`; attach to GitHub issue.
- [ ] Corpora to use (verified on aiwonder at run time; no synthetic substitute
  can close this issue):
  - Text: e.g. `nq-open` or `wikipedia-sections`
  - Code: e.g. `codeparrot-clean` or `the-stack-python`
  - Graph: e.g. `ogbn-arxiv` or `cora` (citation graph → AssocGraph nodes)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_text_corpus_recall()` — loads text corpus,
  asserts `recall_report.ratio >= 0.95`, prints JSON.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_code_corpus_recall()` — same for code corpus.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_graph_corpus_recall()` — same for graph corpus.
- [ ] `#[test] #[cfg(feature = "fsv")] fn fsv_grounding_gaps_text()` — loads text corpus
  with known anchors; asserts `gaps` list matches hand-verified set.
- [ ] edge: checksum mismatch on corpus file → `CALYX_DATASET_CHECKSUM_MISMATCH`;
  corrupt input is rejected and the issue stays open until a valid corpus is read.
- [ ] fail-closed: `ratio < 0.95` on any corpus → test fails with a message including
  the exact `ratio` value and corpus name (not just `assert_eq!(true, false)`).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON report files at `$CALYX_HOME/fsv/ph33_recall_<corpus>_<date>.json`;
  printed test output.
- **Readback:**
  ```
  cargo test -p calyx-lodestar --features fsv fsv_recall_real_corpora 2>&1 | tee /tmp/ph33_t05_fsv.txt
  cat $CALYX_HOME/fsv/ph33_recall_*.json
  ```
- **Prove:** each JSON contains `ratio >= 0.95`; three distinct `corpus_name` values
  (text, code, graph); `grounding_gaps` JSON lists the expected gaps;
  all four test functions pass; output and JSON files attached to PH33 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [ ] Three `RecallReport` JSON files at `$CALYX_HOME/fsv/ph33_recall_*.json`
      with `ratio >= 0.95` each; attached to PH33 GitHub issue
- [ ] `grounding_gaps` gap list for text corpus attached, cross-checked
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
