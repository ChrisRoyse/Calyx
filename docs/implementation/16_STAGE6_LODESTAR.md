# Stage 6 — Lodestar Kernel (PH31–PH34)

> **STATUS: ✅ DONE / FSV-signed-off (PH31-PH34, Stage 6 exit #240).** Stages 1-5 are
> implemented, pushed, and FSV-signed-off. PH31 graph primitives are implemented
> in `calyx-paths` and `calyx-mincut`; PH32 kernel-graph + DFVS is implemented
> in `calyx-lodestar`. aiwonder readbacks live under
> `/home/croyse/calyx/data/fsv-ph31-20260608` and
> `/home/croyse/calyx/data/fsv-ph32-20260608`. PH33 kernel index/answer/gaps
> through real-corpora recall is signed off under
> `/home/croyse/calyx/fsv/ph33_*_20260608.*`. PH34 T01-T07 are signed off,
> including real multi-scope SciFact reports under
> `/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`. PH33 T06 (#239)
> real Ledger stamping is signed off under
> `/home/croyse/calyx/data/fsv-issue239-kernel-ledger-provenance-20260608`;
> PH34 scope-cache identity #328 is signed off under
> `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.
> PH32 LP/DFVS contract honesty #329 is signed off under
> `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`.
> PH33 recall-gate fail-closed behavior #330 is signed off under
> `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
> PH33 raw-vs-tuned recall evidence #331 is signed off under
> `/home/croyse/calyx/data/fsv-issue331-raw-vs-tuned-recall-20260608`;
> anchor-aware answer search #332 is signed off under
> `/home/croyse/calyx/data/fsv-issue332-kernel-answer-anchor-search-20260608`.
> Real-corpus anchor-search bound readback #630 is signed off under
> `/home/croyse/calyx/data/fsv-issue630-real-anchor-search-20260610`.
> Real-corpus `kernel_answer_with_ledger` trace readback #631 is signed off under
> `/home/croyse/calyx/data/fsv-issue631-real-ledger-answer-20260610`.
> Stage 6 exit #240 is signed off under
> `/home/croyse/calyx/data/fsv-issue240-stage6-exit-lodestar-20260609`.
> PH36 still owns broader Ledger reproduce. Post-stage-5 delta issue #360 is
> signed off under
> `/home/croyse/calyx/data/fsv-issue360-lodestar-add-full-rebuild-20260609-96ed8af`;
> SCC-merge `apply_node_add() -> FullRebuildRequired -> rebuild_dirty()` now
> preserves the pending candidate graph before stale state is cleared.

Autonomously find an auditable compact grounding kernel (directed MFVS target)
for each dataset/scope and use it as both an index and an answer-path — the most
novel DB capability, no other store has it. The ≈1% figure is the design target
for the raw compact kernel, not a universal measured guarantee: PH33/PH34
acceptance is the byte-read final/tuned kernel size, `raw_recall`,
`tuned_recall`, and `pass_mode` for each real corpus/scope. Lands in
`calyx-lodestar` + the graph crates `calyx-mincut`/`calyx-paths` (seeded from
ContextGraph). **Living-system role:** identity.

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

## PH32 — Kernel-graph (~10% target) + directed MFVS (~1% target)
- **Status.** ✅ DONE / FSV-signed-off on aiwonder. Readbacks:
  `ph32-kernel-graph-readback.json`, `ph32-lp-round-readback.json`,
  `ph32-dfvs-readback.json`, `ph32-specialized-dfvs-readback.json`,
  `ph32-kernel-pipeline-readback.json`, `ph32-incremental-readback.json`.
  Contract-hardening #329 is signed off under
  `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`;
  PH32 now explicitly documents the LP scaffold/fallback and exact/greedy DFVS
  contract until a real solver path exists. #346 corrects the incremental hook
  wording and behavior: dirty rebuild is a conservative full-pipeline rebuild,
  and non-kernel node removal now marks the evaluator stale instead of returning
  an empty dirty set. #360 signs off the sibling SCC-merge add path: a full
  rebuild retains the pending candidate graph before stale state is cleared.
- **Objective.** The staged, approximate kernel discovery pipeline.
- **Deps.** PH31.
- **Deliverables.** `kernel_graph.rs` (high in/out-degree + betweenness + low
  groundedness-distance; LP scaffold/fallback + injected-solution rounding),
  `dfvs.rs` (exact/greedy local search; tournament 2-approx; bounded-genus
  specializations), approx-factor and method reporting.
- **Key tasks.** condense → kernel-graph → MFVS; incremental re-eval hook
  (Anneal); report the approximation factor (auditable, not asserted).
- **FSV gate.** on a **synthetic graph with a planted MFVS**, the algorithm
  finds the planted feedback-vertex-set (read members vs known).
- **Axioms/PRD.** A10, `08 §3`.

## PH33 — Kernel index + kernel_answer + grounding_gaps
- **Status.** ✅ T01-T09 DONE / FSV-signed-off. T06 #239 now writes real
  PH35 Ledger rows for kernel build and answer hops; #631 adds the final
  complete Answer row that lets `get_answer_trace` return a trusted trace for
  successful non-direct Lodestar answers. T05 real-corpora recall FSV is signed
  off on aiwonder:
  SciFact text `0.9611112`, live Calyx code `0.9777778`, Cora graph
  `0.9568264`, and exact direct-anchor `grounding_gaps` readback. Follow-up #292
  locks `kernel_answer` to fail closed when `max_hops` cannot reach `query_cx`;
  truncated answer prefixes are not valid answer paths. T07 #330 makes recall
  acceptance fail closed with `CALYX_KERNEL_RECALL_BELOW_GATE`; FSV root:
  `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
  T09 #332 is signed off under
  `/home/croyse/calyx/data/fsv-issue332-kernel-answer-anchor-search-20260608`.
  #630 adds the real-corpus bounded fallback readback: SciFact anchor rank `76`
  is outside the old top-10 window while the current scan is bounded at `158`
  tuned-kernel candidates, with the full real anchored set passed through
  production `kernel_answer`.
  #631 signs off real SciFact `kernel_answer_with_ledger`: before ledger rows
  `0`, after rows `6`, kernel seq `0`, hop seqs `[1,2,3,4]`, complete Answer
  seq `5`, and `get_answer_trace` returns `trace_trusted=true`.
  T08 #331 raw-vs-tuned recall evidence is signed off under
  `/home/croyse/calyx/data/fsv-issue331-raw-vs-tuned-recall-20260608`.
- **Objective.** Use the kernel as a real index + answer-path; surface the
  cheapest grounding plan.
- **Deps.** PH32, PH33 needs anchors (PH09) + search (PH24).
- **Deliverables.** `idx/kernel` (dedicated ANN over kernel cx), `kernel_answer`
  (ground at nearest anchored kernel → traverse association edges, hop-
  attenuated, provenanced), `grounding_gaps` (kernel members not reaching an
  anchor), recall test.
- **Key tasks.** kernel-first funnel; anchor-reachability check; recall test
  (reconstruct held-out from kernel-only); fail-closed `kernel_recall_gate`.
- **FSV gate.** **tuned/final kernel-only recall ≥ 0.95·full** on **≥3
  real corpora** (text/code/graph from the dataset catalog, run on aiwonder).
  Reports must also expose `raw_recall`, `tuned_recall`, added member
  count/IDs/hash, and `pass_mode` so a raw-below/tuned-pass repair cannot be
  mistaken for a raw pass. `grounding_gaps` lists exactly the unanchored
  members (read both); below-gate recall returns `CALYX_KERNEL_RECALL_BELOW_GATE`.
- **Axioms/PRD.** A10, A11, `08 §4/§7`, `19 §4`.

## PH34 — Multi-scope kernel
- **Status.** DONE / FSV-signed-off. T01 scope enum/materialization is implemented and
  FSV-signed-off on aiwonder under
  `/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608`; T02
  `ScopeCache` is implemented and FSV-signed-off under
  `/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`; T03 scoped
  dispatch/reporting is implemented and FSV-signed-off under
  `/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`; T04
  hierarchical kernel-of-regions is implemented and FSV-signed-off under
  `/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608`; T05 bridge
  nodes are implemented and FSV-signed-off under
  `/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608`; T06 real
  multi-scope FSV is implemented and FSV-signed-off under
  `/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`; T07 scope-cache identity
  is implemented and FSV-signed-off under
  `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.
- **Objective.** Freedom of scope: kernel over all / collection / domain /
  subgraph / time-window / tenant / filter / union.
- **Deps.** PH33.
- **Deliverables.** `build_kernel(scope, anchor?, params?)`, scope cache
  `(scope_hash, panel_version, anchor_identity, corpus_identity)`, hierarchical kernel-of-regions for huge scopes,
  per-scope recall/grounded-fraction reporting.
- **Key tasks.** scope param → subgraph → MFVS; incremental update; composable
  answering; union/intersect for bridges.
- **FSV gate.** kernel built at **≥4 distinct scopes** on a real corpus, each
  with its own measured kernel-only recall + grounded fraction (read each).
- **Axioms/PRD.** A21, `08 §4b`, `22 §4`.

---

## Stage 6 exit
Done on aiwonder under
`/home/croyse/calyx/data/fsv-issue240-stage6-exit-lodestar-20260609`.
The exit readback summary hash is
`167bcb0db5691fae29749dec458da1b5e2469fc166cad96f51c982fbdb26baa0`;
the root manifest hash is
`065558c3697d155c9a7cd299b91d93fb86733a3d903145cfb8b822aeb658f322`.

The Stage 6 exit readback proves Lodestar builds compact grounded kernels and
uses them as index + reasoning paths. The acceptance evidence is measured, not
assumed: PH33 #331 records raw-vs-tuned recall and `pass_mode`, PH34 reports
per-scope kernel sizes/recall/grounded fractions, and #240 signs off the Stage
6 exit. It does not prove a universal ≈1% raw kernel for every slice; it proves
the PRD `KERNEL` / `KERNEL_ANY` gates with the measured final/tuned artifacts.
The semantic compressor and the AGI substrate's kernel half.
