# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Status (2026-06-09; Stage 8 active after Stage 7 #256)

Stages 0-5 (phases PH00-PH30) are built and FSV-signed-off on aiwonder.
Stage 6 (PH31-PH34 Lodestar) is closed and FSV-signed-off through #240,
including PH33 raw-vs-tuned recall #331 and kernel_answer anchor search #332.
Stage 7 Ledger (PH35-PH36) is closed and FSV-signed-off after PH35
#242-#248, PH35 failure-atomicity hardening #345, PH36 T01 #249,
range-bound signature hardening #347, and real Aster `merkle-root --vault`
hardening #348, verify_chain/quarantine #250, and checkpoint scheduler #251.
PH36 reproduce re-measure #252, fusion replay/drift #253, and audit query
surface #254 are also FSV-signed-off on aiwonder. PH36 exit FSV integration
#255 is signed off with flip-byte tamper detection at seq 11 and reproduce
bit-parity readback. Stage 7 exit rollup #256 is signed off with all 10
`EntryKind`s, group-commit atomicity, redaction, checkpoints, tamper
quarantine, reproduce bit-parity, and audit trace readback. PH36 audit-query
quarantine filter hardening remains tracked separately as follow-up #349.

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
| `calyx-ledger` | S7 PH35-PH36 | provenance: hash-chained append-only ledger CF, redaction, group-commit integration, Merkle checkpoints, verify-chain quarantine, reproduce, audit query surfaces |
| `calyx-ward` | S8 PH37-PH38 | guard profile, verdict/error, AllRequired, KofN, OOD wrapper, no-average/no-flatten enforcement, PH37 readback harness, incoming-query `guard_query`, Assay-derived required-slot derivation, kernel-near guard priority, PH38 conformal tau calibration T01, and PH38 provisional high-stakes refusal are active: #258-#265 and #275/#277/#278 are FSV-signed-off; PH38 T03+ remains before Ward exit |

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
`/home/croyse/calyx/data/fsv-issue348-merkle-vault-real-aster-cf-20260609`,
and
`/home/croyse/calyx/data/fsv-issue250-verify-chain-quarantine-20260609`,
and
`/home/croyse/calyx/data/fsv-issue251-checkpoint-scheduler-20260609`,
`/home/croyse/calyx/data/fsv-issue252-reproduce-20260609`,
`/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609`,
`/home/croyse/calyx/data/fsv-issue254-audit-query-20260609`, and
`/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609`, plus
`/home/croyse/calyx/data/fsv-issue256-stage7-exit-20260609-nomock`,
`/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`, and
`/home/croyse/calyx/data/fsv-issue259-ph37-t02-20260609`,
`/home/croyse/calyx/data/fsv-issue260-ph37-t03-20260609-20a2a34`, and
`/home/croyse/calyx/data/fsv-issue261-ph37-t04-20260609-bd35e1e`,
`/home/croyse/calyx/data/fsv-issue262-ph37-t05-20260609-3dbe1a6`,
`/home/croyse/calyx/data/fsv-issue263-ph37-t06-20260609-4cde3b7`,
`/home/croyse/calyx/data/fsv-issue264-ph38-t01-20260609-f95c817`,
`/home/croyse/calyx/data/fsv-issue275-ph37-t07-20260609-8b71024`,
`/home/croyse/calyx/data/fsv-issue277-ph37-t08-20260609-e75ade1`, and
`/home/croyse/calyx/data/fsv-issue278-ph37-t09-20260609-c2d3e30`, and
`/home/croyse/calyx/data/fsv-issue265-ph38-t02-20260609-5c23db5`.

Ward is now the active engine frontier. Remaining major engine crates
(`anneal`, `oracle`, `mcp`, `calyxd`) are still pending. Ledger PH35 is
FSV-signed-off, including the #345
failure-atomic staging hardening; PH36 Merkle root export #249,
range-bound signing #347, and real Aster `merkle-root --vault` #348 are signed
off. PH36 verify_chain/quarantine #250, checkpoint scheduler #251, reproduce
re-measure #252, reproduce fusion replay #253, and audit query surface #254
are signed off. PH36 exit FSV integration #255 and Stage 7 exit rollup #256
are signed off; residual PH36 audit-query quarantine filter hardening is
tracked in #349, covering filtered audit queries around unrelated quarantined
rows and typed `cx` mention matching. Stage 8 Ward has #258-#265 and
#275/#277/#278 signed off; PH37 is complete and PH38 T03+ remains before the
Ward exit claim under epic #257, with exit #280.

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
