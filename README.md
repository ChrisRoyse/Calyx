# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Status (2026-06-08, commit `0ada102`)

Stages 0-5 (phases PH00-PH30) are built and FSV-signed-off on aiwonder.
Six engine surfaces are implemented:

| Crate | Stage | What it is |
|---|---|---|
| `calyx-core` | S0 | IDs, enums, `CALYX_*` error catalog, constellation model, engine traits, `Clock` |
| `calyx-aster` | S1 | storage core: WAL+group-commit, LSM/SSTable, column families, MVCC, CRUD, crash recovery, compaction/tiering |
| `calyx-forge` | S2 | math runtime: CPU SIMD + CUDA sm_120 bit-parity, TurboQuant, MXFP4/grouped GEMM, autotune cache |
| `calyx-registry` | S3 | lenses: algorithmic/TEI/candle/ONNX runtimes, frozen contract + `LensId`, hot-swap+backfill, capability cards, default panels + temporal lenses |
| `calyx-sextant` | S4 | search/navigation: per-slot dense/sparse indexes, RRF/WeightedRRF/SingleLens fusion, provenance hits, planner/explain |
| `calyx-loom` / `calyx-assay` | S5 | DDA + bits: lazy cross-terms, agreement graph, abundance reports, MI/NMI/logistic estimators, differentiation contract, n_eff, sufficiency, attribution, cache provenance |

Plus `calyx-cli` (readback/FSV/crash tools) and `calyx-testkit`. Current source
of truth is GitHub issue #23. Latest Stage 5 FSV root on aiwonder:
`/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.

Remaining engine crates (`lodestar`, `mincut`, `paths`, `ledger`, `ward`,
`anneal`, `oracle`, `mcp`, `calyxd`) are still skeletons. **Next: Stage 6
Lodestar kernel (PH31-PH34)**, starting with `calyx-paths`/`calyx-mincut`
graph primitives and then `calyx-lodestar`.

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
