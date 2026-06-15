# PH33 — Kernel index + kernel_answer + grounding_gaps

**Stage:** S6 — Lodestar Kernel  ·  **Crate:** `calyx-lodestar`  ·
**PRD roadmap:** P5  ·  **Axioms:** A10, A11

## Objective

Turn the PH32 compact MFVS target into a production index and answer-path engine.
This phase builds three capabilities that together make the kernel's value concrete
and measurable: (1) `idx/kernel/` — a dedicated ANN index over kernel constellation
embeddings, enabling kernel-first query routing; (2) `kernel_answer` — answer a
query by grounding at the nearest anchored kernel node then traversing association
edges with `0.9^hop` attenuation, fully provenanced; (3) `grounding_gaps` — list
exactly which kernel members cannot reach any anchor (the cheapest grounding plan).
The phase closes with measured final/tuned recall: **kernel-only recall ≥
0.95·full on ≥3 real corpora**, with `raw_recall`, `tuned_recall`, and
`pass_mode` read back so the compact-kernel target is never mistaken for a
universal ≈1% guarantee.

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
`0.9611112`, live Calyx code ratio `0.9777778`, and Cora graph ratio
`0.9568264`, all non-exhaustive and warning-free. Reports live under
`/home/croyse/calyx/fsv/ph33_recall_*_20260608.json`; summary SHA-256
`1b0a6c0e1045de2a3230b326dd782f5767772dd6b5a9f4138543e65c5cdbe714`.
Raw-vs-tuned recall #331 is signed off under
`/home/croyse/calyx/data/fsv-issue331-raw-vs-tuned-recall-20260608`: raw ratios
were below gate (`0.08333334`, `0.09444446`, `0.064206704`) and final tuned
ratios passed (`0.9611112`, `0.96666664`, `0.9568264`) with
`pass_mode=tuned`. Anchor-aware answer search #332 is signed off under
`/home/croyse/calyx/data/fsv-issue332-kernel-answer-anchor-search-20260608`.
Real-corpus anchor-search bound readback #630 is signed off under
`/home/croyse/calyx/data/fsv-issue630-real-anchor-search-20260610`: SciFact
loads hash-checked corpus/qrels bytes, the selected grounded anchor is rank
`76` outside the old top-10 window, the current exhaustive fallback is bounded
by the tuned kernel's `158` candidates, and the test passes the full real
anchored set through production `kernel_answer`.
T06 (#239) adds PH35-backed Lodestar provenance APIs:
`build_kernel_pipeline_with_ledger` writes one `kind=Kernel` entry and
`kernel_answer_with_ledger` writes one `kind=Answer` entry per hop plus a final
complete Answer row for trusted `get_answer_trace` output, with fail-closed
`CALYX_LEDGER_*` error surfacing. Physical ledger row, decoded JSON, hex, and
secret-scan readbacks are FSV-backed at
`/home/croyse/calyx/data/fsv-issue239-kernel-ledger-provenance-20260608`; the
combined real-corpus readback #631 is signed off under
`/home/croyse/calyx/data/fsv-issue631-real-ledger-answer-20260610`.
Full PH36 reproduce remains closed in #252-#255.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/kernel_index.rs` | write/load `idx/kernel/`; ANN over kernel `CxId` embeddings; `kernel_search(query_vec) -> Vec<(CxId, f32)>` |
| `crates/calyx-lodestar/src/kernel_answer.rs` | `kernel_answer(query, anchor_kind) -> AnswerPath`; ground at nearest anchored kernel node → bounded `reach` to the query → hop-attenuate with `0.9^hop` → provenance-stamp each hop |
| `crates/calyx-lodestar/src/loom_assoc.rs` | read Loom XTerm CF agreement rows through `LoomStore`, require slot→CxId bindings + directional confidence, and emit CxId `AgreementEdge` inputs for Mincut/Lodestar |
| `crates/calyx-lodestar/src/grounding_gaps.rs` | `grounding_gaps(kernel, anchors) -> Vec<CxId>`; BFS from each kernel member; members not reaching any anchor are the gaps |
| `crates/calyx-lodestar/src/recall_test.rs` | `kernel_recall_test(...) -> RecallReport` for report-only warning bytes; `kernel_recall_gate(...) -> RecallReport` for fail-closed acceptance when ratio < 0.95 |
| `crates/calyx-lodestar/src/provenance.rs` | PH35-backed `kind=Kernel` / per-hop `kind=Answer` / complete `kind=Answer` Ledger append helpers for build and answer paths (#239/#631); PH36 reproduce is closed separately in Stage 7 |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `idx/kernel/` ANN index write + kernel-first search funnel | — (needs PH32 Kernel) |
| T02 | `kernel_answer`: ground → traverse association edges → provenance | T01 |
| T03 | `grounding_gaps`: anchor-reachability BFS + gap list | T01 |
| T04 | Recall test harness: kernel-only recall ≥ 0.95·full | T02, T03 |
| T05 | FSV: run on ≥3 real corpora on aiwonder; measure + report recall | T04 |
| T06 | Kernel build/answer → Ledger provenance wiring (`kind=Kernel`, per-hop Answer rows, complete trace row) (done #239/#631; PH36 reproduce separate) | PH35 |
| T07 | Recall below gate fails closed for acceptance flows (done #330) | T04 |
| T08 | Raw-vs-tuned recall evidence with `raw_recall`, `tuned_recall`, and `pass_mode` (done #331) | T05, T07 |
| T09 | Anchor-aware `kernel_answer` search exhausts the kernel index and continues to the first reachable anchor before failing closed (synthetic #332; real-corpus bound #630) | T02 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. `kernel_recall_gate` run on **≥3 real corpora** (text/code/graph acquired and
   verified on aiwonder); each corpus produces final/tuned `RecallReport`
   evidence with `ratio ≥ 0.95`.
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
- **Raw compact target vs. tuned acceptance:** PH32's ≈1% compact-kernel target
  is not the PH33 exit claim. #331 must remain visible in docs/readbacks because
  it proves raw-below/tuned-pass behavior with explicit `pass_mode=tuned`.
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
  Ledger rows per hop and a final complete Answer row (#239/#631). The legacy
  `kernel_answer` stub path is compatibility-only and must not be counted as
  real Stage 6 exit provenance. Direct-hit ledger answers must append a durable
  complete `kind=Answer` row with `expected_hops=0` before returning (#647);
  FSV root:
  `/home/croyse/calyx/data/fsv-issue647-direct-hit-ledger-20260611T073538Z`;
  PH36 owns broader reproduce.
