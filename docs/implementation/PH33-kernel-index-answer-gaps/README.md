# PH33 — Kernel index + kernel_answer + grounding_gaps

**Stage:** S6 — Lodestar Kernel  ·  **Crate:** `calyx-lodestar`  ·
**PRD roadmap:** P5  ·  **Axioms:** A10, A11

## Objective

Turn the ~1% MFVS kernel from PH32 into a production index and answer-path engine.
This phase builds three capabilities that together make the kernel's value concrete
and measurable: (1) `idx/kernel/` — a dedicated ANN index over kernel constellation
embeddings, enabling kernel-first query routing; (2) `kernel_answer` — answer a
query by grounding at the nearest anchored kernel node then traversing association
edges with `0.9^hop` attenuation, fully provenanced; (3) `grounding_gaps` — list
exactly which kernel members cannot reach any anchor (the cheapest grounding plan).
The phase closes with a recall test: **kernel-only recall ≥ 0.95·full on ≥3 real
corpora** acquired and verified on aiwonder.

## Dependencies

- **Phases:** PH32 (`Kernel` struct, `build_kernel_pipeline`, `dfvs_approx` pipeline),
  PH09 (Anchor, CxId, constellation CRUD — anchors for grounding check),
  PH24 (RRF/fusion search primitives — `kernel_search` uses the same funnel),
  PH31 (`AssocGraph`, hop-attenuated traversal from `calyx-paths`)
- **Provides for:** PH34 (multi-scope kernel uses `kernel_answer` + `grounding_gaps`
  per scope), PH43 (Anneal uses `grounding_gaps` as a grounding deficit signal),
  PH48 (J objective uses kernel recall ratio)

## Current state (build off what exists)

`calyx-lodestar` has PH32 plus PH33 T01-T05 in-tree: kernel index persistence,
kernel search, `kernel_answer`, `grounding_gaps`, the recall harness, and the
FSV-backed #293 Loom xterm CF to association-graph adapter. The current `idx/kernel/`
implementation is `FsKernelStore`, which writes
`idx/kernel/<kernel_id>/index.json` under the configured root; moving this into
an Aster column-family/ANN shard is a later storage integration seam, not the
current PH33 T01 source of truth. Stage 6 consumers can treat Assay `trusted`
bits as grounded-only after #294; any Assay output without grounded Anchor
evidence is `provisional` and must not be used as a trusted kernel signal.
Build-time `Kernel.groundedness` now uses the same bounded
`KernelGraphParams.max_groundedness_distance` as the public `grounding_gaps`
API (#298). FSV root:
`/home/croyse/calyx/data/fsv-issue298-build-kernel-groundedness-bound-20260608`.
PH33 T05 real-corpora FSV (#232) is signed off on aiwonder: SciFact text ratio
`0.9611112`, live Calyx code ratio `0.96111107`, and Cora graph ratio
`0.9568264`, all non-exhaustive and warning-free. Reports live under
`/home/croyse/calyx/fsv/ph33_recall_*_20260608.json`; summary SHA-256
`b12ea6c3339cfce2dae34142d88419ffddf2371b9e9c38a85eaaa6ee4471b169`.
T06 (#239) adds PH35-backed Lodestar provenance APIs:
`build_kernel_pipeline_with_ledger` writes one `kind=Kernel` entry and
`kernel_answer_with_ledger` writes one `kind=Answer` entry per hop, with
fail-closed `CALYX_LEDGER_*` error surfacing. Physical ledger row, decoded JSON,
hex, and secret-scan readbacks are FSV-backed at
`/home/croyse/calyx/data/fsv-issue239-kernel-ledger-provenance-20260608`.
Full PH36 trace/reproduce remains in #252-#255.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/kernel_index.rs` | write/load `idx/kernel/`; ANN over kernel `CxId` embeddings; `kernel_search(query_vec) -> Vec<(CxId, f32)>` |
| `crates/calyx-lodestar/src/kernel_answer.rs` | `kernel_answer(query, anchor_kind) -> AnswerPath`; ground at nearest anchored kernel node → bounded `reach` to the query → hop-attenuate with `0.9^hop` → provenance-stamp each hop |
| `crates/calyx-lodestar/src/loom_assoc.rs` | read Loom XTerm CF agreement rows through `LoomStore`, require slot→CxId bindings + directional confidence, and emit CxId `AgreementEdge` inputs for Mincut/Lodestar |
| `crates/calyx-lodestar/src/grounding_gaps.rs` | `grounding_gaps(kernel, anchors) -> Vec<CxId>`; BFS from each kernel member; members not reaching any anchor are the gaps |
| `crates/calyx-lodestar/src/recall_test.rs` | `kernel_recall_test(kernel, corpus, held_out) -> RecallReport`; reconstruct held-out from kernel-only; ratio ≥ 0.95 is the gate |
| `crates/calyx-lodestar/src/provenance.rs` | PH35-backed `kind=Kernel` / `kind=Answer` Ledger append helpers for build and answer paths (#239); PH36 trace/reproduce remains separate |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `idx/kernel/` ANN index write + kernel-first search funnel | — (needs PH32 Kernel) |
| T02 | `kernel_answer`: ground → traverse association edges → provenance | T01 |
| T03 | `grounding_gaps`: anchor-reachability BFS + gap list | T01 |
| T04 | Recall test harness: kernel-only recall ≥ 0.95·full | T02, T03 |
| T05 | FSV: run on ≥3 real corpora on aiwonder; measure + report recall | T04 |
| T06 | Kernel build/answer → Ledger provenance wiring (`kind=Kernel`) (done #239; PH36 trace/reproduce separate) | PH35 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. `kernel_recall_test` run on **≥3 real corpora** (text/code/graph acquired and
   verified on aiwonder); each corpus produces a `RecallReport` with
   `ratio ≥ 0.95`.
2. `grounding_gaps` on the same corpora lists exactly the unanchored kernel members
   (cross-check by manual inspection of a small corpus).
3. Both reports read back via `calyx readback` or printed JSON on aiwonder;
   evidence attached to PH33 GitHub issue.
4. `CALYX_KERNEL_UNGROUNDED` fires on a synthetic corpus with no anchors (confirmed
   in the readback output).

## Risks / landmines

- **Recall test depends on real data:** aiwonder must have ≥3 real corpora
  available. Missing corpora are acquisition/verification work for PH33, not a
  reason to close with synthetic-only evidence.
- **ANN index vs. full search recall:** the `0.95` gate compares kernel-only ANN
  recall to full-corpus ANN recall on the same query set — both use the same ANN
  algorithm; the comparison is fair only if the same HNSW params are used.
- **Loom graph handoff:** #293 proved the real XTerm CF adapter with explicit
  slot→CxId bindings and directional-confidence rows. Missing bindings/confidence
  fail closed; synthetic graph-builder structs alone are not enough for PH33 FSV.
- **Answer traversal depth:** `0.9^hop` attenuation means answers beyond hop 10 have
  score ≤ 0.35; `max_hops` is a hard reachability bound, not a display limit.
  If the query cannot be reached inside the bound, `kernel_answer` must return
  `CALYX_PATHS_MAX_HOPS` rather than a truncated answer.
- **Groundedness distance:** `grounding_gaps` accepts a bounded anchor distance.
  Build-pipeline groundedness uses `KernelGraphParams.max_groundedness_distance`;
  an anchor just beyond that bound remains in `unanchored_members` (#298).
- **Assay trust handoff:** Lodestar may consume Assay `trusted` bits only from
  anchor-aware estimates/reports. No-anchor or ungrounded Assay results are
  intentionally `provisional` after #294 and cannot satisfy grounded kernel
  evidence requirements.
- **Provenance stamp per hop:** `kernel_answer_with_ledger` now appends real
  Ledger rows per hop (#239). The legacy `kernel_answer` stub path is
  compatibility-only and must not be counted as real Stage 6 exit provenance.
  PH36 owns `get_answer_trace` and `reproduce`.
