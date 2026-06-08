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

`calyx-lodestar` is partially built (PH32 delivers `Kernel` + pipeline). This phase
adds the index write, the answer traversal, and the recall harness. The `idx/kernel/`
path is a new column family or ANN shard in the Aster store.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/kernel_index.rs` | write/load `idx/kernel/`; ANN over kernel `CxId` embeddings; `kernel_search(query_vec) -> Vec<(CxId, f32)>` |
| `crates/calyx-lodestar/src/kernel_answer.rs` | `kernel_answer(query, anchor_kind) -> AnswerPath`; ground at nearest anchored kernel node → traverse via `reach_scored` (hop-attenuated 0.9^hop) → provenance-stamp each hop |
| `crates/calyx-lodestar/src/grounding_gaps.rs` | `grounding_gaps(kernel, anchors) -> Vec<CxId>`; BFS from each kernel member; members not reaching any anchor are the gaps |
| `crates/calyx-lodestar/src/recall_test.rs` | `kernel_recall_test(kernel, corpus, held_out) -> RecallReport`; reconstruct held-out from kernel-only; ratio ≥ 0.95 is the gate |
| Stage 7 Ledger integration | #239 replaces PH33 stub provenance with real `kind=Kernel` ledger entries once PH35/PH36 land |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `idx/kernel/` ANN index write + kernel-first search funnel | — (needs PH32 Kernel) |
| T02 | `kernel_answer`: ground → traverse association edges → provenance | T01 |
| T03 | `grounding_gaps`: anchor-reachability BFS + gap list | T01 |
| T04 | Recall test harness: kernel-only recall ≥ 0.95·full | T02, T03 |
| T05 | FSV: run on ≥3 real corpora on aiwonder; measure + report recall | T04 |
| T06 | Kernel build/answer → Ledger provenance wiring (`kind=Kernel`) | PH35/PH36 |

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
- **Answer traversal depth:** `0.9^hop` attenuation means answers beyond hop 10 have
  score ≤ 0.35; document the practical max-hop budget in the code.
- **Provenance stamp per hop:** PH33 fills structured provenance references; #239
  remains open until PH35/PH36 provide real Ledger appends and readback.
