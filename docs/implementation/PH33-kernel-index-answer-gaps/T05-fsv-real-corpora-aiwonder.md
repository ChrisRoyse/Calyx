# PH33 T05 - FSV: kernel recall gate on real corpora

| Field | Value |
|---|---|
| **Phase** | PH33 - Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/tests/fsv_recall_real_corpora.rs` (<=500) |
| **Depends on** | T04 recall harness and gate API |
| **Axioms** | A10, A11 |
| **PRD** | `dbprdplans/08`, `dbprdplans/28` |

## Status

DONE / FSV-signed-off on aiwonder. Issue #330 tightened the final acceptance
path so the real-corpora FSV uses `kernel_recall_gate`, not warning-only
`kernel_recall_test`. The report-only function remains available only for
tuning and diagnostics before the final gate.

Evidence at `$CALYX_HOME=/home/croyse/calyx`:

| Artifact | SHA-256 |
|---|---|
| `fsv/ph33_recall_scifact_text_20260608.json` | `c26d13fd96880b9df47d0de099dbc63638365533780a3f08ff09d4b30fbaf18c` |
| `fsv/ph33_recall_calyx_code_20260608.json` | `b7114058e50b99cd8b49f79516fd8b4eb1b161e6a5b534416bb8944f90cd7fe5` |
| `fsv/ph33_recall_cora_graph_20260608.json` | `64b2bf4654caaa98d30274bb1ec938da4fca78f58b6f5f1dd587690f27f26d9b` |
| `fsv/ph33_grounding_gaps_scifact_text_20260608.json` | `987cc4c28c757c7184e03ef19713603b8b489dce7342e770a205cce8a1405716` |
| `fsv/ph33_real_corpora_summary_20260608.json` | `1b0a6c0e1045de2a3230b326dd782f5767772dd6b5a9f4138543e65c5cdbe714` |
| `data/fsv-issue330-recall-gate-fail-closed-20260608/10-real-corpora-gate-test.out` | `74394affa43b2481a013151dbfd9a47a7e95ec9427792a29de7a747501f0b2b8` |

Additional #331 raw-vs-tuned acceptance evidence at
`/home/croyse/calyx/data/fsv-issue331-raw-vs-tuned-recall-20260608`:

| Artifact | SHA-256 |
|---|---|
| `real-corpora/ph33_recall_scifact_text_20260608.json` | `6bf0f2989930996b44d3201b2075d3d911e0fff1a12399434db68b38927cabab` |
| `real-corpora/ph33_recall_calyx_code_20260608.json` | `84571e9fe745cef519ab7c036510c5903872ef699ddd31be1b273de61b3ce7ba` |
| `real-corpora/ph33_recall_cora_graph_20260608.json` | `e0d41fd76bbbaea7fb1bf28c05d214ce783de7c787365a7592368b5da20bb41a` |
| `real-corpora/ph33_real_corpora_summary_20260608.json` | `3091ef593f6ab9c062097420c6fd21dfc84a1c5b9a9c831aa7c577b9730f595b` |
| `root_manifest.sha256` | `acd84fcf82e08a43788ad7331bbc7290ebe783cde0b8b880548bb060464494df` |

Readback ratios:

| Corpus | Modality | Rows | Final kernel members | Ratio | Warning |
|---|---:|---:|---:|---:|---|
| `scifact_text` | text | 180 | 158 | `0.9611112` | none |
| `calyx_code` | code | 180 | 169 | `0.9777778` | none |
| `cora_graph` | graph | 2708 | 2377 | `0.9568264` | none |

#331 tuned-recall readback:

| Corpus | Raw ratio | Tuned ratio | Added members | Pass mode |
|---|---:|---:|---:|---|
| `scifact_text` | `0.08333334` | `0.9611112` | 154 | tuned |
| `calyx_code` | `0.09444446` | `0.96666664` | 153 | tuned |
| `cora_graph` | `0.064206704` | `0.9568264` | 2242 | tuned |

These rows are the caveat for Stage 6 summaries: the raw compact-kernel target
is not the acceptance result. The signed PH33 gate is the final/tuned report
with `pass_mode`, and all current real-corpus passes are `pass_mode=tuned`.

`grounding_gaps` readback: `max_anchor_dist=0`, `expected_gap_count=4`,
`report_gap_count=4`, exact independent reachability match = `true`.

## Goal

Run the recall gate on at least three real corpora on aiwonder: text, code, and
graph. Each corpus must pass `ratio >= 0.95` through `kernel_recall_gate`. Any
below-gate corpus must fail closed with `CALYX_KERNEL_RECALL_BELOW_GATE`.

## Build

- [x] `tests/fsv_recall_real_corpora.rs` is gated behind the `fsv` feature and
  runs only on aiwonder.
- [x] Static corpora are content-address checked before rows load.
- [x] The live Calyx code corpus is hashed at run time because it changes with
  the repository.
- [x] Each corpus builds a kernel index and exact full reference index.
- [x] Tuning may call `kernel_recall_test` to inspect report warnings.
- [x] Final acceptance calls `kernel_recall_gate` so a below-gate ratio returns
  an error instead of a warning-only report.
- [x] `grounding_gaps` is verified against an independent reachability scan.

## FSV

- **SoT:** JSON report files at
  `$CALYX_HOME/fsv/ph33_recall_<corpus>_20260608.json`, the summary JSON, and
  captured stdout.
- **Command:**

  ```bash
  CALYX_HOME=/home/croyse/calyx cargo test -p calyx-lodestar --features fsv \
    fsv_recall_real_corpora_aiwonder -- --ignored --nocapture \
    2>&1 | tee /home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608/10-real-corpora-gate-test.out
  ```

- **Issue #330 readback root:**
  `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
- **Prove:** three distinct corpora pass gate mode with no warning, and the
  PH33 synthetic degraded corpus proves below-gate recall now errors.

## Done when

- [x] `cargo check`, `clippy -D warnings`, and `test` pass on aiwonder.
- [x] `.rs` files remain <=500 lines.
- [x] Three real-corpus recall JSON files show `ratio >= 0.95`.
- [x] Below-gate synthetic FSV proves the gate is fail-closed.
- [x] `grounding_gaps` output matches the independent reachability scan.
