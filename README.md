# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Status (2026-06-08; Stage 6 active through #237)

Stages 0-5 (phases PH00-PH30) are built and FSV-signed-off on aiwonder.
Stage 6 is active: PH31 and PH32 are implemented, pushed, and FSV-signed-off;
PH33 T01-T05 are implemented with real-corpora recall FSV; PH34 T01-T05 are
implemented. The next implementable Stage 6 card is PH34 T06 (#238).

Implemented engine surfaces:

| Crate | Stage | What it is |
|---|---|---|
| `calyx-core` | S0 | IDs, enums, `CALYX_*` error catalog, constellation model, engine traits, `Clock` |
| `calyx-aster` | S1 | storage core: WAL+group-commit, LSM/SSTable, column families, MVCC, CRUD, crash recovery, compaction/tiering |
| `calyx-forge` | S2 | math runtime: CPU SIMD + CUDA sm_120 bit-parity, TurboQuant, MXFP4/grouped GEMM, autotune cache |
| `calyx-registry` | S3 | lenses: algorithmic/TEI/candle/ONNX runtimes, frozen contract + `LensId`, hot-swap+backfill, capability cards, default panels + temporal lenses |
| `calyx-sextant` | S4 | search/navigation: per-slot dense/sparse indexes, RRF/WeightedRRF/SingleLens fusion, provenance hits, planner/explain |
| `calyx-loom` / `calyx-assay` | S5 | DDA + bits: lazy cross-terms, agreement graph, abundance reports, MI/NMI/logistic estimators, differentiation contract, n_eff, sufficiency, attribution, cache provenance |
| `calyx-paths` / `calyx-mincut` | S6 PH31 | graph primitives: sparse association graph, 0.9^hop traversal, Tarjan SCC condensation, Brandes betweenness, Loom graph builder, LP scaffolding |
| `calyx-lodestar` | S6 PH32-PH34 | kernel discovery: kernel-graph scoring, LP-rounding interface, DFVS approximations, kernel pipeline, grounded/provisional tagging, incremental re-eval hook, kernel index/answer/gaps/recall FSV, scope materialization, scope cache |

Plus `calyx-cli` (readback/FSV/crash tools) and `calyx-testkit`. Current source
of truth is GitHub issue #23. Recent aiwonder FSV roots:
`/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`,
`/home/croyse/calyx/data/fsv-ph31-20260608`, and
`/home/croyse/calyx/data/fsv-ph32-20260608`. Current Lodestar FSV roots include
`/home/croyse/calyx/fsv/ph33_*_20260608.*`,
`/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608`,
`/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`,
`/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`,
`/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608`, and
`/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608`.

Remaining major engine crates (`ledger`, `ward`, `anneal`, `oracle`, `mcp`,
`calyxd`) are still pending. Stage 6 is not exit-complete until PH34 T06,
PH33 Ledger provenance #239 (after real Stage 7 Ledger primitives), and S6 exit
FSV #240 close with aiwonder readback evidence.

Full plan and per-phase status: `docs/implementation/` (start at `00_README.md`
-> `03_PHASE_MAP.md`).

## Per-Merge Gate

Run the gate on aiwonder before every merge:

```bash
cd /home/croyse/calyx/repo
source ./env.sh
bash scripts/check.sh
```

`scripts/check.sh` runs `cargo fmt`, `cargo check`, `cargo clippy -D warnings`,
`cargo test`, and the `scripts/linecount.sh` gate. There is no hosted CI for
Calyx; FSV evidence in GitHub Issues is the release gate.

Every `.rs` source/test file must stay at or below 500 lines. If a file exceeds
the limit, open a `type:task` issue and modularize it per
`docs2/modulateprompt.md` before the gate can pass.
