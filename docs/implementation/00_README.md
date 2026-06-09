# Calyx — Implementation Plan (master README)

This directory is the **build plan** for Calyx: how the system specified in
`docs/dbprdplans/` (the PRD) and bound by `docs/dbprdplans/DOCTRINE.md` is
actually constructed, phase by phase, **entirely on the aiwonder datacenter
PC**. The PRD says *what* and *why*; this plan says *in what order, on what
machine, proven how*.

> **Read order:** `DOCTRINE.md` → this README → `01_AIWONDER_ENVIRONMENT.md`
> → `02_WORKING_AGREEMENT.md` → `03_PHASE_MAP.md` → the stage files in order.
> Per-phase, do-now atomic task cards live in one subdir per phase
> (`PH05-*/` … `PH72-*/`), governed by `PHASE_TASKS_README.md`. Stage 0
> (PH00–PH04) is built and intentionally has no subdir.

---

## 1. The three non-negotiable framing facts

1. **Everything happens on aiwonder.** This Windows/WSL repo **authors** plan +
   code only. The project is **built, stored, run, and tested on aiwonder**
   (`croyse@aiwonder.mst.com`, over the Cisco VPN). The source-of-truth bytes
   that FSV reads live on aiwonder; a local run never counts (DOCTRINE §0/§8c,
   PRD `28 §5`). Connection + secrets: `../../.env` (gitignored).
2. **Calyx is self-contained on aiwonder.** All Calyx work lives under ONE root
   — `CALYX_HOME=/home/croyse/calyx` (and its dedicated ZFS datasets once
   provisioned). Nothing Calyx touches the existing `leapable`/`contextgraph`
   projects, the PostgreSQL control plane, or any shared dotfile. Build output,
   datasets, vaults, logs, HF cache — all under `CALYX_HOME`.
3. **FSV is the gate.** Every phase below is "done" only when a clause is proven
   by **reading the persisted bytes on aiwonder** (not a return value, not a
   green harness). There is no CI; FSV is CI (PRD `28 §6b`).

## 2. How the plan is organized

| File | Owns |
|---|---|
| `00_README.md` | this — how to use the plan, conventions, the dependency spine |
| `01_AIWONDER_ENVIRONMENT.md` | the **real** box (live readback), the self-contained Calyx layout, toolchain, GPU/CUDA, ZFS, services, secrets, the sudo constraint, the connect procedure |
| `02_WORKING_AGREEMENT.md` | the per-phase discipline: FSV protocol, ≤500-line rule, GitHub issues, test taxonomy, definition-of-done, doctrine compliance checklist |
| `03_PHASE_MAP.md` | the master table of **every** phase (PH00–PH72), its stage, dependencies, PRD/axiom mapping, exit gate, and the critical path |
| `STAGE1_5_EVIDENCE_MANIFEST.md` | the Stage 1-5 audit index: PH05-PH30 evidence roots, commands, artifact hashes, source-of-truth summaries, and live deferral-owner issues |
| `10_STAGE0_FOUNDATION.md` … `30_STAGE20_CRITICAL_CAPS.md` | one file per stage; each details its phases (objective · deps · deliverables · key tasks · FSV exit gate · axioms · risks) |
| `PHASE_TASKS_README.md` | **the per-phase task convention** — directory layout, the atomic task-card template, the README template, the binding rules every card inherits, and the coverage rule. Read before opening any phase subdir. |
| `PH05-*/` … `PH72-*/` | **one subdir per phase** (Stage 1 → Stage 20). Each holds a `README.md` (phase overview) + one `.md` atomic task card per actionable unit (`T01-…`, `T02-…`). When every card in every subdir is done, `BUILD_DONE` holds. Stage 0 (PH00–PH04) is already built and intentionally has no subdir. |

Completed-phase note: task-card checkboxes are implementation prompts and
design-history acceptance criteria. The current status source is, in order,
GitHub issue state/evidence comments, `STAGE1_5_EVIDENCE_MANIFEST.md` for
PH05-PH30, `03_PHASE_MAP.md`, the per-stage file, then the phase README. For
Stages 0-5, open/closed state and aiwonder FSV evidence supersede historical
task-card checklist prompts.

## 3. Numbering

- **Phases** are `PH00`–`PH72`, globally ordered, grouped into **Stages**
  `S0`–`S20`. Phase IDs are stable handles used in GitHub issues and commits.
- Each phase cross-references the PRD's own roadmap phases (`P0`–`P12` and the
  `Pxb` sub-phases in `dbprdplans/19`) and the axioms (`A1`–`A34`) it satisfies.
- Phases are sized so each maps to a small set of ≤500-line crate modules and a
  single FSV exit gate — i.e. a few days of agent work, not months.

## 4. The dependency spine (critical path)

```
S0 Foundation ─▶ S1 Aster ─▶ S2 Forge ─▶ S3 Registry ─▶ S4 Sextant ─▶ S5 Loom/Assay
                                                  │                         │
                                                  ▼                         ▼
                                            (lenses live)            S6 Lodestar ─▶ S8 Ward
                                                                          │
S7 Ledger threads through S1+ (provenance in group-commit) ───────────────┤
                                                                          ▼
S9 Temporal/Dedup ─▶ S10 Anneal+J ─▶ S11 Oracle/AGI                  (kernel + guard)
S12 Universal data layer (parallel to S5–S8, needs S1)
S13 Resource/GC + S14 Security (cross-cutting; harden continuously)
S15 Interfaces (MCP/CLI) usable from S4 onward; S16 Server/Deploy after S8
S17 Scale ▸ S18 Datasets/Intelligence-FSV ▸ S19 Leapable ▸ S20 Critical caps
```

The **recommended first demo** (PRD `19 §2`): `S0 → S1 → S2(CPU) → S3 → S4` +
the migration shadow (`S15`/`S19-V0`) — a Calyx vault answering with multiple
lenses + provenance. That alone justifies the project.

## 5. Engine → crate → stage cheat sheet

| Engine (PRD codename) | Crate | Stage |
|---|---|---|
| Aster (storage) | `calyx-aster` | S1 |
| Forge (GPU/SIMD math) | `calyx-forge` | S2 |
| Registry (lenses) | `calyx-registry` | S3 |
| Sextant (search/nav) | `calyx-sextant` | S4 |
| Loom (DDA) / Assay (bits) | `calyx-loom` / `calyx-assay` | S5 |
| Lodestar (kernel) | `calyx-lodestar` + `calyx-mincut`/`-paths` | S6 |
| Ledger (provenance) | `calyx-ledger` | S7 |
| Ward (Gτ guard) | `calyx-ward` | S8 |
| Temporal/Dedup | (in `aster`/`registry`/`loom`) | S9 |
| Anneal (self-opt) + `J` | `calyx-anneal` | S10 |
| Oracle/AGI | `calyx-oracle` | S11 |
| Universal data layer | `calyx-aster` (layers) + `calyx-sextant` | S12 |
| Resource/GC, Security | cross-cutting | S13, S14 |
| MCP / CLI / Server | `calyx-mcp` / `calyx-cli` / `calyxd` | S15, S16 |

## 6. What "ground truth" already exists on aiwonder (reuse, don't rebuild)

Confirmed live (`01_AIWONDER_ENVIRONMENT.md`): RTX 5090 sm_120 + **CUDA 13.2**
toolkit, **Rust via rustup** (so we build natively — the PRD's "no rustc on
box" note is *superseded*), resident **TEI lenses** on :8088/:8089/:8090,
Prometheus on :9090, Docker, Infisical, HF cache, ZFS hot+cold pools. Userspace
`cmake` and `protoc` are installed under `/home/croyse/calyx/bin`. Stage 6 lifts
the ContextGraph `mincut`/`paths`/`witness`/`mejepa` logic as seeds (PRD
`19 §6`).

## 7. Status (current: 2026-06-09; latest pushed main tracked in #23)

**DONE — Stages 0–5 (PH00–PH30), FSV-signed-off on aiwonder.** Implemented
surfaces: `calyx-core`, `calyx-aster`, `calyx-forge`, `calyx-registry`,
`calyx-sextant`, `calyx-loom`, `calyx-assay`, plus `calyx-cli` and
`calyx-testkit`. Latest Stage 5 hardening: #318 wires seeded bootstrap CI
through KSG/logistic/AssayGate/PairGain/persisted AssayStore rows, and #319
adds the Aster-backed Assay materialization gate that feeds grounded PairGain
into Loom xterm CF materialization. Latest Stage 3/4 readiness hardening: #339
adds Registry determinism proof metadata, proves Registry->Aster backfill into
Sextant stored-provenance search, and makes qrels search require stored
provenance when requested. Latest pre-Lodestar audit hardening #333
adds Aster SST v2 full-body CRCs, manifest immutable-ref hash verification,
compacted-SST recovery, post-WAL commit-success semantics, real group-commit
window coalescing, and release-mode Forge grouped-GEMM absent-slot sentinel
checks. Evidence root:
`/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`.

**DONE — Stage 6 Lodestar (PH31–PH34).** PH31 graph primitives are built in
`calyx-paths`/`calyx-mincut`; PH32 kernel discovery is built in
`calyx-lodestar`; PH33 T01-T09 kernel index/answer/gaps/real-corpora recall and
Ledger provenance are implemented and FSV-backed; PH34 T01-T07 are implemented
and FSV-backed (scope materialization, identity-aware cache, dispatch,
hierarchical regions, bridge nodes, real multi-scope SciFact FSV, and
scope-cache identity). Stage 6 exit #240 is FSV-backed under
`/home/croyse/calyx/data/fsv-issue240-stage6-exit-lodestar-20260609`.
LP/DFVS solver-contract honesty #329 is FSV-backed under
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`. Recall gate
fail-closed behavior #330 is FSV-backed under
`/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
Raw-vs-tuned recall evidence #331 and anchor-aware answer search #332 are
FSV-backed under `/home/croyse/calyx/data/fsv-issue331-raw-vs-tuned-recall-20260608`
and `/home/croyse/calyx/data/fsv-issue332-kernel-answer-anchor-search-20260608`.

- **Stage 0** (PH00–PH04): `calyx-core` — IDs, enums, the full `CALYX_*` error
  catalog, the constellation model structs, engine traits, the injected `Clock`.
- **Stage 1** (PH05–PH11): `calyx-aster` storage core — WAL + group-commit,
  memtable + LSM SSTable, column families + key codecs, MVCC snapshots,
  constellation CRUD + idempotent ingest, manifest + crash recovery, compaction
  + hot/cold tiering. Plus `calyx-cli` readback/FSV/crash commands and
  `calyx-testkit`. **FSV-signed-off on aiwonder** by byte-level readback (87+
  `calyx-aster` tests, 6 `calyx-cli` tests; crash-drill recovered to last-acked
  seq; corrupt-shard failed closed). Evidence: GitHub issue #23 (`[CONTEXT] You
  are here`); FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.
  Satisfies PRD `CORE` (`dbprdplans/19 §5`). Most Stage-1 follow-ups are now
  resolved (`open` uses the manifest-anchored `recover_vault` + `set_start_seq`;
  durable-write / `CfRouter` / `CompactionScheduler` unified via
  `vault/compaction_bridge.rs`; dedicated `vault/ledger_stub.rs`;
  `CompactionDebt::measure` proptest landed). #333 further hardens the storage
  substrate with SST body CRCs, parent fsync after SST rename, manifest
  immutable-ref hash verification, compacted-SST recovery, WAL-authoritative
  post-append commit semantics, and group-commit deadline coalescing. Evidence
  root:
  `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`. Remaining
  deferrals
  (`degraded_rebuildable`→PH44, Arrow slot columns) are tracked in
  `11_STAGE1_ASTER.md`.
- **Stage 2** (PH12–PH16): `calyx-forge` math runtime — CPU SIMD backend
  (gemm/cosine/l2/normalize/topk, AVX-512), CUDA sm_120 backend with CPU↔GPU
  bit-parity suite (`cuda/` + `.cu` kernels), TurboQuant (rotation + scalar +
  QJL + binary prefilter), MXFP4/MXFP8 microscaling + grouped/ragged GEMM, and
  the per-shape autotune cache (microbench + explorer + reversible promotion).
  Stage 2 is FSV-signed-off; PH12 roots are listed in
  `PH12-cpu-simd-backend/README.md`, and aggregate evidence is recorded in #23.
  #333 promotes PH15 absent-slot sentinel protection from debug-only assertion
  to release-mode `ForgeError` fail-closed behavior.
- **Stage 3** (PH17–PH22): `calyx-registry` lens layer — uniform
  `Registry.measure` over algorithmic / TEI-HTTP / candle-local / ONNX runtimes,
  the frozen contract + content-addressed `LensId`, hot-swap add/retire/park with
  a lazy durable backfill scheduler, capability-card profiling, and the default
  panels + closed-form temporal lenses E2/E3/E4. FSV root:
  `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752`; durable
  PH20 scheduler hardening #300 root:
  `/home/croyse/calyx/data/fsv-issue300-backfill-scheduler-20260608`; #339
  Registry->Aster->Sextant integration root:
  `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608`.
- **Stage 4** (PH23–PH26): `calyx-sextant` search/navigation — per-slot dense
  and sparse indexes, RRF/WeightedRRF/SingleLens fusion with provenance,
  planner/explain/freshness, and real SciFact qrels evidence. #296 records the
  controlled SearchEngine reranker-ordering FSV and is separate from the
  resident `:8089` Stage 4 reranker readback. #299 records that
  Sextant GPU parity/fan-out is explicit fail-loud/unwired state, not a hidden
  CPU-self comparison. #339 adds explicit stored/stub provenance source and
  fail-closed stored-provenance queries. FSV root:
  `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`; #339 root:
  `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608`.
- **Stage 5** (PH27–PH30): `calyx-loom` + `calyx-assay` DDA/bits — agreement
  graph, lazy cross-terms, abundance reports, KSG-style MI, random projection,
  bootstrap CI, partitioned NMI, logistic probe, AssayGate pair gain,
  differentiation contract, stratified bits, n_eff, sufficiency, attribution,
  and assay provenance cache. FSV root:
  `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.
  Post-sweep roots #318 and #319 record seeded bootstrap CI and live
  Aster-backed Assay/Loom materialization evidence, respectively.

**Stage 6 / PH31:** `calyx-paths` + `calyx-mincut` graph primitives — sparse
association graph, 0.9^hop traversal, SCC condensation, Brandes betweenness,
Loom graph builder, and LP scaffolding. FSV root:
`/home/croyse/calyx/data/fsv-ph31-20260608`.

**Stage 6 / PH32:** `calyx-lodestar` kernel discovery pipeline — kernel-graph
scoring/rounding, DFVS approximation and specializations, Kernel struct,
anchored/provisional groundedness, and incremental re-eval hook. FSV root:
`/home/croyse/calyx/data/fsv-ph32-20260608`.

**Stage 6 / PH33:** `calyx-lodestar` kernel index + answer + grounding gaps +
real-corpora recall. T01-T05 are closed with aiwonder evidence, including
kernel-only recall on SciFact text, live Calyx code, and Cora graph under
`/home/croyse/calyx/fsv/ph33_*_20260608.*`. T06 Ledger provenance (#239) is
closed with PH35 Ledger append/readback evidence; PH36 trace/reproduce work is
closed in Stage 7 (#249-#256). T08 #331 and T09 #332 are signed off with
aiwonder evidence under the issue roots above.

**Stage 6 / PH34 T06: >=4 distinct scopes on a real corpus is DONE.** PH34 T01
scope materialization, T02 scope cache, T03 scoped dispatch/reports, T04
hierarchical kernel-of-regions, T05 bridge nodes, and T06 real multi-scope FSV
are closed with aiwonder readbacks under
`/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`. `KERNEL_ANY` is satisfied
for PH34. Stage 6 exit #240 is signed off.

**DONE — Stage 7 Ledger (PH35-PH36).** PH35 #242-#248 plus hardening #345 are
FSV-signed-off. PH36 T01 #249, range-bound signature hardening #347, real
Aster `calyx merkle-root --vault` hardening #348, verify_chain/quarantine
#250, checkpoint scheduler #251, reproduce re-measure #252, reproduce fusion
#253, audit query surface #254, PH36 integration #255, and Stage 7 exit #256
are FSV-signed-off under
`/home/croyse/calyx/data/fsv-issue249-merkle-root-ed25519-20260609`,
`/home/croyse/calyx/data/fsv-issue347-merkle-range-bound-signatures-20260609`,
and
`/home/croyse/calyx/data/fsv-issue348-merkle-vault-real-aster-cf-20260609`,
and
`/home/croyse/calyx/data/fsv-issue250-verify-chain-quarantine-20260609`,
and
`/home/croyse/calyx/data/fsv-issue251-checkpoint-scheduler-20260609`,
`/home/croyse/calyx/data/fsv-issue252-reproduce-20260609`,
`/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609`,
`/home/croyse/calyx/data/fsv-issue254-audit-query-20260609`,
`/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609`, and
`/home/croyse/calyx/data/fsv-issue256-stage7-exit-20260609-nomock`.
Stage 8 Ward is the active frontier: #258-#262 are FSV-signed-off, #263 is the
next task under epic #257, and exit remains #280.

**Remaining:** Open phases **PH37-PH72** are laid out as atomic task cards in the
per-phase subdirs, governed by `PHASE_TASKS_README.md`, each with a byte-level
FSV exit gate. Execution proceeds down the dependency spine (§4). Track live
state in the `ChrisRoyse/Calyx` GitHub `type:context` issues (doctrine §8d,
PRD `29`).
