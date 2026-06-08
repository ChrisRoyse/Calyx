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

## Status

DONE / FSV-signed-off on aiwonder (2026-06-08). The aiwonder run uses one
ignored FSV test, `fsv_recall_real_corpora_aiwonder`, because the three corpus
recall checks and the grounding-gap exactness check share the same real-corpus
setup. Static third-party corpora fail closed on fixed content-address mismatch
before rows are loaded; the live Calyx code corpus is hashed as a source
readback because it intentionally changes with the repo.

Evidence at `$CALYX_HOME=/home/croyse/calyx`:

| Artifact | SHA-256 |
|---|---|
| `fsv/ph33_recall_scifact_text_20260608.json` | `c26d13fd96880b9df47d0de099dbc63638365533780a3f08ff09d4b30fbaf18c` |
| `fsv/ph33_recall_calyx_code_20260608.json` | `705cd5897dde71efdbdfe1aeee60e9088296fee3242380c32b268a02090be117` |
| `fsv/ph33_recall_cora_graph_20260608.json` | `64b2bf4654caaa98d30274bb1ec938da4fca78f58b6f5f1dd587690f27f26d9b` |
| `fsv/ph33_grounding_gaps_scifact_text_20260608.json` | `987cc4c28c757c7184e03ef19713603b8b489dce7342e770a205cce8a1405716` |
| `fsv/ph33_real_corpora_summary_20260608.json` | `b12ea6c3339cfce2dae34142d88419ffddf2371b9e9c38a85eaaa6ee4471b169` |
| `fsv/ph33_t05_fsv_20260608.log` | `618bb0c03c0a2160dc6e29a8597f511b09ff9bc331fbf5a71cf4b3b899e4268d` |

Readback ratios:

| Corpus | Modality | Rows | Final kernel members | Ratio | Exhaustive | Warning |
|---|---:|---:|---:|---:|---|---|
| `scifact_text` | text | 180 | 158 | `0.9611112` | false | none |
| `calyx_code` | code | 180 | 162 | `0.96111107` | false | none |
| `cora_graph` | graph | 2708 | 2377 | `0.9568264` | false | none |

`grounding_gaps` readback: `max_anchor_dist=0`, `expected_gap_count=4`,
`report_gap_count=4`, exact independent reachability match = `true`.

## Goal

Run the `kernel_recall_test` on at least 3 real corpora on aiwonder (one each of
text, code, and graph modality), prove that
**kernel-only recall ≥ 0.95·full** on each, and attach the evidence to the PH33
GitHub issue. Also verify that `grounding_gaps` lists exactly the unanchored kernel
members on a corpus where the anchor set is known. This is the byte-level FSV gate
for PH33 (not a unit test — a real on-device integration run).

## Build (checklist of concrete, code-level steps)

- [x] Create `tests/fsv_recall_real_corpora.rs`; gated with `#[cfg(feature = "fsv")]`
  so it does not run in CI (aiwonder-only).
- [x] Load each corpus from a verified aiwonder path (`$CALYX_HOME/datasets/<name>/`
  or `$CALYX_HOME/data/datasets/<name>/`); if a required corpus is missing,
  acquire it, record its source/checksum, and read the files back before use.
- [x] For each corpus: build or load the `Kernel` (via `build_kernel_pipeline`);
  build the `KernelIndex` and a full exact `InMemoryAnnIndex` reference; run
  `kernel_recall_test` with `rng_seed=42`, `top_k=10`, `held_out_fraction=0.10`.
- [x] Assert `ratio >= 0.95` for each corpus; print the full `RecallReport` JSON.
- [x] Run `grounding_gaps` on the text corpus with known SciFact qrel anchors;
  print the gap list; manually cross-check at least 3 reported direct-anchor gaps.
- [x] Write the three `RecallReport` JSONs and the gap list to
  `$CALYX_HOME/fsv/ph33_recall_<corpus_name>_<date>.json`; attach to GitHub issue.
- [x] Corpora to use (verified on aiwonder at run time; no synthetic substitute
  can close this issue):
  - Text: `beir-scifact/scifact`
  - Code: live Calyx repo under `/home/croyse/calyx/repo/crates`
  - Graph: `cora` citation graph -> AssocGraph nodes

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `#[test] #[cfg(feature = "fsv")] fn fsv_recall_real_corpora_aiwonder()` loads
  text/code/graph corpora, asserts each `recall_report.ratio >= 0.95`, and prints JSON.
- [x] The same ignored FSV test loads text with known anchors and asserts `gaps`
  matches an independent reachability scan.
- [x] edge: checksum mismatch on corpus file -> `CALYX_DATASET_CHECKSUM_MISMATCH`;
  corrupt input is rejected and the issue stays open until a valid corpus is read.
- [x] fail-closed: `ratio < 0.95` on any corpus -> test fails with a message including
  the exact `ratio` value and corpus name (not just `assert_eq!(true, false)`).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON report files at `$CALYX_HOME/fsv/ph33_recall_<corpus>_<date>.json`;
  printed test output.
- **Readback:**
  ```
  CALYX_HOME=/home/croyse/calyx cargo test -p calyx-lodestar --features fsv \
    fsv_recall_real_corpora_aiwonder -- --ignored --nocapture \
    2>&1 | tee /home/croyse/calyx/fsv/ph33_t05_fsv_20260608.log
  cat $CALYX_HOME/fsv/ph33_recall_*.json
  ```
- **Prove:** each JSON contains `ratio >= 0.95`; three distinct `corpus_name` values
  (text, code, graph); `grounding_gaps` JSON lists the expected gaps;
  the ignored aiwonder FSV test passes; output and JSON files attached to PH33
  GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] Three `RecallReport` JSON files at `$CALYX_HOME/fsv/ph33_recall_*.json`
      with `ratio >= 0.95` each; attached to PH33 GitHub issue
- [x] `grounding_gaps` gap list for text corpus attached, cross-checked
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
