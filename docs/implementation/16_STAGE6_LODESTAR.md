# Stage 6 — Lodestar Kernel (PH31–PH34)

> **STATUS: ▶ ACTIVE (PH31-PH32 DONE / PH33 ACTIVE).** Stages 1-5 are
> implemented, pushed, and FSV-signed-off. PH31 graph primitives are implemented
> in `calyx-paths` and `calyx-mincut`; PH32 kernel-graph + DFVS is implemented
> in `calyx-lodestar`. aiwonder readbacks live under
> `/home/croyse/calyx/data/fsv-ph31-20260608` and
> `/home/croyse/calyx/data/fsv-ph32-20260608`. PH33 kernel index/answer/gaps is
> the current Stage 6 implementation slice.

Autonomously find the ≈1% grounding kernel (directed MFVS) of any dataset and
use it as both an index and an answer-path — the most novel DB capability, no
other store has it. Lands in `calyx-lodestar` + the graph crates
`calyx-mincut`/`calyx-paths` (seeded from ContextGraph). **Living-system role:**
identity.

---

## PH31 — mincut/paths: graph build + SCC + betweenness
- **Status.** ✅ DONE / FSV-signed-off on aiwonder. Readbacks:
  `ph31-paths-graph-readback.json`, `ph31-paths-traversal-readback.json`,
  `ph31-scc-readback.json`, `ph31-betweenness-readback.json`,
  `ph31-graph-builder-readback.json`, `ph31-lp-readback.json`.
- **Objective.** The directed association graph + the graph primitives MFVS
  needs.
- **Deps.** PH27 (agreement graph).
- **Deliverables.** `calyx-paths` (traversal, hop-attenuation `0.9^hop`,
  bidirectional), `calyx-mincut` (Tarjan SCC, betweenness, LP scaffolding);
  graph built from agreement × directional confidence + citation/entity edges;
  frequency→node weight (A29).
- **Key tasks.** lift ContextGraph `mincut`/`paths` source into the crates;
  sparse adjacency; recurrence frequency raises in-degree.
- **FSV gate.** SCC condensation + betweenness match a reference implementation
  on a planted graph (read computed vs known).
- **Axioms/PRD.** `08 §2/§3`, A29, `19 §6` (reuse seeds).

## PH32 — Kernel-graph (~10%) + directed MFVS (~1%)
- **Status.** ✅ DONE / FSV-signed-off on aiwonder. Readbacks:
  `ph32-kernel-graph-readback.json`, `ph32-lp-round-readback.json`,
  `ph32-dfvs-readback.json`, `ph32-specialized-dfvs-readback.json`,
  `ph32-kernel-pipeline-readback.json`, `ph32-incremental-readback.json`.
- **Objective.** The staged, approximate kernel discovery pipeline.
- **Deps.** PH31.
- **Deliverables.** `kernel_graph.rs` (high in/out-degree + betweenness + low
  groundedness-distance; LP-relaxation rounding), `dfvs.rs` (LP-relaxation
  `O(log τ* log log τ*)` approx + local search; tournament 2-approx; bounded-
  genus specializations), approx-factor reporting.
- **Key tasks.** condense → kernel-graph → MFVS; incremental re-eval hook
  (Anneal); report the approximation factor (auditable, not asserted).
- **FSV gate.** on a **synthetic graph with a planted MFVS**, the algorithm
  finds the planted feedback-vertex-set (read members vs known).
- **Axioms/PRD.** A10, `08 §3`.

## PH33 — Kernel index + kernel_answer + grounding_gaps
- **Status.** ▶ ACTIVE. Follow-up #292 locks `kernel_answer` to fail closed when
  `max_hops` cannot reach `query_cx`; truncated answer prefixes are not valid
  answer paths.
- **Objective.** Use the kernel as a real index + answer-path; surface the
  cheapest grounding plan.
- **Deps.** PH32, PH33 needs anchors (PH09) + search (PH24).
- **Deliverables.** `idx/kernel` (dedicated ANN over kernel cx), `kernel_answer`
  (ground at nearest anchored kernel → traverse association edges, hop-
  attenuated, provenanced), `grounding_gaps` (kernel members not reaching an
  anchor), recall test.
- **Key tasks.** kernel-first funnel; anchor-reachability check; recall test
  (reconstruct held-out from kernel-only).
- **FSV gate.** **kernel-only recall ≥ 0.95·full** on **≥3 real corpora**
  (text/code/graph from the dataset catalog, run on aiwonder); `grounding_gaps`
  lists exactly the unanchored members (read both).
- **Axioms/PRD.** A10, A11, `08 §4/§7`, `19 §4`.

## PH34 — Multi-scope kernel
- **Objective.** Freedom of scope: kernel over all / collection / domain /
  subgraph / time-window / tenant / filter / union.
- **Deps.** PH33.
- **Deliverables.** `build_kernel(scope, anchor?, params?)`, scope cache
  `(scope_hash, panel_version)`, hierarchical kernel-of-regions for huge scopes,
  per-scope recall/grounded-fraction reporting.
- **Key tasks.** scope param → subgraph → MFVS; incremental update; composable
  answering; union/intersect for bridges.
- **FSV gate.** kernel built at **≥4 distinct scopes** on a real corpus, each
  with its own measured kernel-only recall + grounded fraction (read each).
- **Axioms/PRD.** A21, `08 §4b`, `22 §4`.

---

## Stage 6 exit
Lodestar finds the grounded ≈1% of any slice and uses it as index + reasoning
path, with measured (never assumed) recall and an actionable grounding plan —
PRD `KERNEL` + `KERNEL_ANY`. The semantic compressor and the AGI substrate's
kernel half.
