# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Status (2026-06-09; Stage 7 active at PH36 post-#348)

Stages 0-5 (phases PH00-PH30) are built and FSV-signed-off on aiwonder.
Stage 6 (PH31-PH34 Lodestar) is closed and FSV-signed-off through #240,
including PH33 raw-vs-tuned recall #331 and kernel_answer anchor search #332.
Stage 7 Ledger is active at PH36 verify_chain/reproduce work after PH35
#242-#248, PH35 failure-atomicity hardening #345, PH36 T01 #249,
range-bound signature hardening #347, and real Aster `merkle-root --vault`
hardening #348.

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
`/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608`, plus
`/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`,
`/home/croyse/calyx/data/fsv-issue249-merkle-root-ed25519-20260609`, and
`/home/croyse/calyx/data/fsv-issue345-ledger-group-commit-atomicity-20260609`,
`/home/croyse/calyx/data/fsv-issue347-merkle-range-bound-signatures-20260609`,
and
`/home/croyse/calyx/data/fsv-issue348-merkle-vault-real-aster-cf-20260609`.

Remaining major engine crates (`ward`, `anneal`, `oracle`, `mcp`, `calyxd`)
are still pending. Ledger PH35 is FSV-signed-off, including the #345
failure-atomic staging hardening; PH36 Merkle root export #249,
range-bound signing #347, and real Aster `merkle-root --vault` #348 are signed
off. PH36 verify_chain, checkpoint scheduling, reproduce, and audit surfaces
continue in #250-#256.

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
