# 03 — Phase Map (PH00–PH72)

Every phase, in order, with stage, dependencies, the crate(s) it lands in, the
PRD roadmap phase + axioms it satisfies, and its one-line FSV exit gate. Detail
lives in the per-stage files (`10_…`–`30_…`). Phase IDs are stable handles.

Legend: **Dep** = phases that must be DONE first. **PRD** = `dbprdplans/19`
roadmap phase. **Gate** = the byte-level proof of done (full version in the
stage file). Status: **✅ DONE** · **▶ ACTIVE** (next up) · **· pending**.

---

## Current status (2026-06-08)

| Stage | Phases | Status |
|---|---|---|
| S0 Foundation | PH00–PH04 | ✅ DONE (`calyx-core`) |
| S1 Aster | PH05–PH11 | ✅ DONE, FSV-signed-off (`calyx-aster`); post-sweep PH11 durable tiering #295 FSV-backed |
| S2 Forge | PH12–PH16 | ✅ DONE, FSV-signed-off (`calyx-forge`: CPU SIMD + CUDA sm_120 + TurboQuant + MXFP4/grouped GEMM + autotune); CUDA top-k large-k overclaim #303 now fails loud |
| S3 Registry | PH17–PH22 | ✅ DONE, FSV-signed-off mechanics (`calyx-registry`: lens runtimes + frozen contract + candle/ONNX + hot-swap/backfill + capability cards + default panels + temporal E2/E3/E4); durable PH20 backfill scheduler #300 remains the Stage 1-5 blocker |
| S4 Sextant | PH23–PH26 | ✅ DONE, FSV-signed-off (`calyx-sextant`: dense/sparse indexes + RRF/provenance + planner/explain + PH26 query filters); PH26 reranker/filter follow-ups #296/#297 are FSV-backed and PH23/PH24 GPU overclaim #299 now fails loud |
| S5 Loom + Assay | PH27–PH30 | ✅ DONE, FSV-signed-off (`calyx-loom` + `calyx-assay`: DDA cross-terms + bits/differentiation/sufficiency); grounded-trust hardening #294 FSV-backed |
| S6 Lodestar | PH31–PH34 | ▶ **ACTIVE** (PH31-PH32 done/FSV-signed-off; PH33 active in `calyx-lodestar`; real Loom adapter #293 and groundedness bound #298 FSV-backed) |
| S7–S20 | PH35–PH72 | · pending |

FSV evidence is summarized in GitHub issue #23 (`[CONTEXT] You are here`).
Latest roots:
- Stage 1 Aster:
  `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`
- Stage 1 Aster PH11 durable tiering:
  `/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608`
- Stage 2 Forge PH12 CPU SIMD:
  `/home/croyse/calyx/data/fsv-q71-20260607115027` through
  `/home/croyse/calyx/data/fsv-q76-20260607122351`
- Stage 2 Forge CUDA top-k large-k hardening:
  `/home/croyse/calyx/data/fsv-issue303-cuda-topk-large-k-20260608`
- Stage 3 atomic suite:
  `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752`
- Stage 4 Sextant:
  `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`
- Stage 4 Sextant GPU parity/fan-out hardening:
  `/home/croyse/calyx/data/fsv-issue299-gpu-parity-fanout-20260608`
- Stage 5 Loom + Assay:
  `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`,
  `/home/croyse/calyx/data/fsv-issue294-assay-grounded-trust-20260608`
- Stage 6 Lodestar PH31/PH32 and PH33 follow-up:
  `/home/croyse/calyx/data/fsv-ph31-20260608`,
  `/home/croyse/calyx/data/fsv-ph32-20260608`,
  `/home/croyse/calyx/data/fsv-issue292-kernel-answer-max-hops-20260608`,
  `/home/croyse/calyx/data/fsv-issue293-loom-assoc-graph-20260608`,
  `/home/croyse/calyx/data/fsv-issue298-build-kernel-groundedness-bound-20260608`

---

## Stage 0 — Foundation & Environment  (`10_STAGE0_FOUNDATION.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate (FSV) |
|---|---|---|---|---|---|
| PH00 | aiwonder bootstrap & self-contained Calyx home | — | — | env | `CALYX_HOME` exists on aiwonder; `cargo`/`nvcc`/GPU readback printed; nothing outside the root |
| PH01 | Rust workspace + crate skeletons + line-count gate | PH00 | all | §8 | `cargo check` green on aiwonder; gate script prints ✅ |
| PH02 | GitHub repo + pinned context issues + workflow | PH00 | — | `29` | 5 `type:context` issues exist + read-state query returns them |
| PH03 | calyx-core: IDs, enums, error catalog | PH01 | core | A1/A16 | unit+proptest green; `CALYX_*` codes enumerated; round-trip IDs byte-exact |
| PH04 | calyx-core: core structs + traits | PH03 | core | A1/A4 | `Constellation`/`Slot`/`Anchor` + traits compile; serde round-trip byte-exact |

## Stage 1 — Aster storage core  (`11_STAGE1_ASTER.md`) — ✅ DONE (PH10 follow-ups tracked)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH05 | WAL + group-commit + fsync | PH04 | aster | P0/A15 | `kill -9` mid-write → replay → last-acked present, torn tail discarded (read WAL bytes) |
| PH06 | Memtable + LSM SSTable writer/reader | PH05 | aster | P0 | flush memtable → read SST back byte-exact; range scan ordered |
| PH07 | Column families + key encoding | PH06 | aster | P0/`04` | base/slot_*/anchors/ledger CFs round-trip; big-endian range scans correct |
| PH08 | MVCC sequence numbers + snapshot reads | PH07 | aster | P0/`03 §8` | concurrent write+read → no partial-constellation read (seq-pinned) |
| PH09 | Constellation CRUD + CxId + idempotent ingest | PH08 | aster | P0/A1 | put N cx → read base/slot CFs byte-exact; re-ingest same bytes = idempotent |
| PH10 | Manifest + atomic swap + crash recovery | PH09 | aster | P0/A15 | crash drill: recover to last consistent seq, byte-exact; corrupt shard fails closed |
| PH11 | Compaction + hot/cold tiering | PH10 | aster | `04 §6` | compaction snapshot-safe; cold slots on archive; write-amp bounded |

## Stage 2 — Forge math runtime  (`12_STAGE2_FORGE.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH12 | CPU SIMD backend (gemm/cosine/l2/normalize/topk) | PH04 | forge | P1/A13 | outputs match numpy/BLAS golden within tol | ✅ FSV (#71–#76) |
| PH13 | CUDA sm_120 backend + bit-parity | PH12 | forge | P1/A13 | CPU↔GPU ≤1e-3; matmul within 10% cuBLAS on sm_120 | ✅ FSV |
| PH14 | TurboQuant (rotate+scalar+QJL) | PH13 | forge | P4b/A25 | unbiased inner-product within distortion bound; re-quant with seed bit-identical | ✅ FSV |
| PH15 | MXFP4/microscaling + grouped GEMM | PH14 | forge | P4b/`23` | grouped GEMM invariant to N ≥ batched-loop; FP4 within bound where Assay-safe | ✅ FSV |
| PH16 | Autotune config cache | PH15 | forge | `12 §4` | best `(op,shape,dtype,device)` config cached + reused; A/B logged | ✅ FSV |

## Stage 3 — Registry / lenses  (`13_STAGE3_REGISTRY.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH17 | Lens trait + algorithmic + tei-http runtimes | PH12,PH09 | registry | P2/A4 | embed via :8088 twice → identical; algorithmic lens deterministic | ✅ FSV |
| PH18 | Frozen contract + content-addressed LensId | PH17 | registry | P2/A4 | weights-hash mismatch → `CALYX_LENS_FROZEN_VIOLATION`; LensId stable across vaults | ✅ FSV |
| PH19 | candle-local + onnx runtimes | PH18 | registry | P2/A4 | local + ONNX lens produce unit-norm finite vectors; dim guard fires | ✅ FSV |
| PH20 | Hot-swap add/retire/park + lazy backfill | PH19 | registry | P2/A5 | add lens → no re-embed; backfill observed on slot columns; retire tombstones | ✅ FSV |
| PH21 | Capability cards / profile | PH20 | registry | A6 | profile returns signal/spread/separation/cost without full ingest | ✅ FSV |
| PH22 | Default panels + temporal lenses E2/E3/E4 | PH21 | registry | A27 | text/code/civic/media panels instantiate; E2/E3/E4 closed-form deterministic | ✅ FSV |

> **Stage 1–5 audit note (2026-06-08):** Subagents and source readback found
> the pre-Lodestar Stage 1–5 hardening set #282-#292 is implemented and
> FSV-backed, with newly tracked follow-up gaps #293-#298 left open instead of
> hidden in docs. PH19 ONNX CUDA registration fails loud instead of silently
> falling back to CPU, with explicit CPU compatibility reported separately. PH23 now
> uses native `ef` HNSW traversal, PH24 explain provenance is refreshed from
> stored constellation provenance, WeightedRRF excludes unnamed and AP-60
> temporal slots before PH40, PH27 Loom cross-terms fail closed, and PH28/PH30
> persisted Assay rows require vault/anchor scope, Assay estimators reject
> ragged/non-finite sample matrices, PH25 Pipeline enforces sparse candidate
> subsets, PH26 reranker non-2xx fails closed, and PH22 temporal flags persist
> onto core Slot rows. The accepted seams are explicitly scoped:
> synthetic `LedgerRef` fallback remains only for documents with no stored
> provenance until Stage 7, and full user-facing Assay/abundance CLI commands
> remain in PH62 while Stage 5 readback bytes are already exposed through FSV
> JSON. Closed during sweep hardening: PH31/PH33 real Loom association-graph
> adapter #293, PH30 grounded Assay trust #294, PH11 durable tiering #295, PH26
> reranker search-path ordering #296, and PH26 scalar/anchor/built-in metadata
> filters #297. PH23/PH24 GPU parity/fan-out overclaim #299 now fails loud
> instead of comparing CPU outputs to themselves. PH13 CUDA top-k large-k
> overclaim #303 now fails loud for `k > 1024` until exact multi-pass merge
> exists. PH33 bounded build-time groundedness #298 is now FSV-backed. The only
> remaining pre-Lodestar Stage 1-5 implementation blocker is PH20 durable
> backfill scheduler #300.

## Stage 4 — Sextant search  (`14_STAGE4_SEXTANT.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH23 | Per-slot HNSW index | PH20 | sextant | P3/`10` | insert+search recall vs brute-force ≥ target; SingleLens p99 budget | ✅ FSV |
| PH24 | RRF/WeightedRRF/SingleLens fusion + provenance hits | PH23 | sextant | P3/`10` | multi-lens recall@10 ≥ single-lens +Δ on real qrels; every Hit carries LedgerRef | ✅ FSV |
| PH25 | Sparse lens inverted index | PH24 | sextant | `10` | sparse lens term-match + BM25 correct; pipeline recall stage works | ✅ FSV |
| PH26 | Query planner + intent + explain | PH25 | sextant | A17 | intent→strategy auto-select; `explain=true` returns per-lens breakdown | ✅ FSV |

## Stage 5 — Loom + Assay (DDA & bits)  (`15_STAGE5_LOOM_ASSAY.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH27 | Agreement graph + cross-terms (lazy) | PH24 | loom | P4/A8 | agreement scalars eager; lazy xterm = one matmul; storage O(n·n_eff) | ✅ FSV |
| PH28 | KSG MI + partitioned NMI | PH27 | assay | P4/`07` | MI on planted-signal synthetic within CI; fails closed below quorum (n<50) | ✅ FSV |
| PH29 | Differentiation contract + n_eff | PH28 | assay | P4/A7 | planted-redundant lens REJECTED (≤0.6); <0.05-bit lens REJECTED; n_eff correct | ✅ FSV |
| PH30 | Panel sufficiency + attribution + reports | PH29 | assay/loom | A8 | `abundance_report` shows N/C(N,2)/materialized/n_eff/DPI ceiling; per-sensor bits | ✅ FSV |

## Stage 6 — Lodestar kernel  (`16_STAGE6_LODESTAR.md`) — ▶ ACTIVE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH31 | mincut/paths: graph build + SCC + betweenness | PH27 | mincut/paths | P5/`08` | SCC condensation + betweenness match reference on planted graph | ✅ FSV |
| PH32 | Kernel-graph (~10%) + directed MFVS (~1%) | PH31 | lodestar | P5/A10 | algorithm finds planted feedback-vertex-set on synthetic graph | ✅ FSV |
| PH33 | Kernel index + kernel_answer + grounding_gaps | PH32 | lodestar | P5/A11 | kernel-only recall ≥ 0.95·full on ≥3 real corpora; gaps listed | ▶ ACTIVE |
| PH34 | Multi-scope kernel | PH33 | lodestar | A21 | kernel built at ≥4 scopes, each measured recall reported | · pending |

## Stage 7 — Ledger provenance  (`17_STAGE7_LEDGER.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH35 | Hash-chain append-only CF (in group-commit) | PH09 | ledger | P7/A15 | every mutation writes a chained entry in the WAL group-commit; chain verifies |
| PH36 | Merkle checkpoints + verify_chain + reproduce() | PH35 | ledger | P7 | flip a byte → `verify_chain` detects break at right seq; `reproduce` bit-parity |

## Stage 8 — Ward guard  (`18_STAGE8_WARD.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH37 | Gτ guard math + GuardProfile | PH22,PH13 | ward | P6/A12 | per-slot cosine gate; all-required pass logic; no-flatten enforced |
| PH38 | τ calibration (conformal) + novelty→new-region | PH37 | ward | P6/A12 | injection corpus blocked ≥99% at calibrated FAR; valid-novelty → new region |
| PH39 | Identity-locked generation (speaker/style) | PH38 | ward | `09 §5b` | SpeakerMatch/StyleHold anchors guard persona; injection→quarantine |

## Stage 9 — Temporal & dedup  (`19_STAGE9_TEMPORAL_DEDUP.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH40 | Temporal fusion + AP-60 post-retrieval boost | PH24,PH22 | sextant | A27 | E2/E3/E4 never dominant (weight 0 in retrieval); boost 50/35/15 applied after |
| PH41 | DedupPolicy TctCosine + recurrence series + signature | PH37,PH09 | aster/loom | A28/A29 | content-slot Gτ dedup; never merges conflicting anchors; recurrence signature fires |
| PH42 | Grounded recurrence wiring across engines | PH41,PH28 | (cross) | A29 | frequency→kernel/Oracle/Assay/Loom; oracle self-consistency from recurring outcomes |

## Stage 10 — Anneal + Intelligence Objective J  (`20_STAGE10_ANNEAL_J.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH43 | Tripwires + shadow-first + reversible/rollback | PH24,PH16 | anneal | A14 | a change crossing a tripwire auto-reverts; rollback = one pointer swap; Ledger-logged |
| PH44 | Self-heal (rebuild derived, degrade flags) | PH43,PH33 | anneal | `12 §2` | corrupt ANN/kernel → degraded flag + background rebuild, no data loss |
| PH45 | Mistake-closure + online heads + replay buffer | PH44 | anneal | `12 §3` | observed contradiction → online head update → same mistake not recur on replay |
| PH46 | Autotune loops (index/quant/fusion/materialization) | PH45,PH16 | anneal | A14 | 1e6-query soak: p99 ↓ ≥20%, no recall regression, no oscillation |
| PH47 | Lens proposal (sufficiency deficit) | PH46,PH30 | anneal | `12 §5` | `I(panel;anchor)` deficit → propose lens → admit only if contract clears |
| PH48 | J objective + growth curve + intelligence_report | PH47 | anneal | A32 | `J` measured; growth_curve rises on a real corpus; Goodhart held-out passes |

## Stage 11 — Oracle & AGI  (`21_STAGE11_ORACLE_AGI.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH49 | Consequence prediction + sufficiency gate | PH48,PH42 | oracle | A20 | predict with calibrated conf capped at oracle self-consistency; refuse when `I<H(Y)` |
| PH50 | Super-intelligence predicate + reverse_query | PH49 | oracle | A20/A23 | 6-tier predicate measurable per domain; reverse a known cause recovers it |
| PH51 | complete() unified primitive (predict=abduce=impute) | PH50 | oracle | `26 §11.1` | clamp/free slots → one energy descent; filled slots tagged `inferred` |
| PH52 | Advanced math (spectral/energy/transfer-entropy/TC/Bayesian) | PH51 | assay/oracle | `26` | each new number proven against a planted synthetic (period/causal/rare-class) |

## Stage 12 — Universal data layer  (`22_STAGE12_UNIVERSAL.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH53 | Collections-as-any-model (relational/doc/KV/TS/blob) | PH09 | aster | A19/`20` | each paradigm's root op (point/range/join/aggregate/traverse) round-trips |
| PH54 | Secondary indexes (btree/inverted) | PH53 | aster | `20` | index key written in same txn as data key; range/point correct |
| PH55 | Cross-model transactions + universal query surface | PH54,PH26 | sextant | A19 | one txn spans modes atomically (consistent seq); planner cost-capped |

## Stage 13 — Resource/GC/reliability  (`23_STAGE13_RESOURCE_GC.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH56 | Bounded caches/queues/memtables + arenas/pools | PH08 | aster/core | A26 | RSS bounded over 1e7 ops; backpressure before OOM |
| PH57 | VRAM budgeter + admission control | PH13 | forge | A26 | dispatch over budget → split/queue/`CALYX_FORGE_VRAM_BUDGET`; coexists with TEI |
| PH58 | GC reclaimers + long-reader watchdog + janitor | PH11 | aster/anneal | A26 | long reader aborted on lease → old version GC'd; tombstones reclaimed; logs bounded |
| PH59 | 25-hazard register FSV + soak | PH56,PH57,PH58 | (cross) | `24` | each of the 25 hazards has a passing FSV; 1e7-op soak bounded, no leak |

## Stage 14 — Security & privacy  (`24_STAGE14_SECURITY.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH60 | Encryption at rest/in transit + tenant isolation | PH09 | aster/calyxd | A33 | cross-vault read without grant → denied+audited; other tenant bytes unreadable |
| PH61 | Crypto-shred erasure + STRIDE FSV + secret-scan | PH60,PH36 | (cross) | A33 | after `erase`: raw disk+backup+Ledger have no recoverable content, tombstone remains |

## Stage 15 — Interfaces (MCP/CLI/migration)  (`25_STAGE15_INTERFACES.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH62 | calyx-cli (vault/lens/ingest/search/readback) | PH24 | cli | A17 | CLI does create/add_lens/ingest/anchor/search; `readback` prints real bytes |
| PH63 | calyx-mcp (stdio embedded tool surface) | PH62 | mcp | A17/`14` | MCP tools self-describe; search returns provenance; errors carry remediation |
| PH64 | Migration tool (sqlite→calyx vault) | PH62 | cli | P11/`15` | migrate a real `.db` → constellations; byte-exact on content via readback |

## Stage 16 — Server & deployment  (`26_STAGE16_SERVER_DEPLOY.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH65 | calyxd daemon (loopback, healthcheck) | PH24,PH13 | calyxd | P9 | `calyx healthcheck` → `"pass"`; binds loopback; CUDA init probed/fail-loud |
| PH66 | systemd + ZFS provisioning + Prometheus/Grafana | PH65 | infra | P9/`16` | (sudo-gated) unit live; `/metrics` up; Grafana panels read via screenshot |
| PH67 | restic backup + DR drill | PH66 | infra | `16 §7` | restore a vault from restic → byte-verify constellations/anchors/ledger; chain intact |

## Stage 17 — Scale  (`27_STAGE17_SCALE.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH68 | DiskANN dense + SPANN sparse | PH23,PH25 | sextant | P10 | server vault 1e8–1e9 cx within search SLO; disk-resident graphs |

## Stage 18 — Datasets & intelligence FSV  (`28_STAGE18_DATASETS_FSV.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH69 | Dataset acquisition + MANIFEST + checksum FSV | PH00 | — | `28 §3` | ≥1 verified dataset per (modality×outcome); checksums match; MANIFEST rows |
| PH70 | Intelligence validation on real corpora | PH69,PH48 | (cross) | `28 §2` | recall/bits/kernel/oracle/J each proven on real data, evidence in issues |

## Stage 19 — Leapable vault swap  (`29_STAGE19_LEAPABLE.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH71 | V0 shadow → V1 flip → V2 calyx-only | PH64,PH33,PH38 | cli/mcp | P11/`15` | shadow parity → flip → calyx-only; PostgreSQL untouched (verified) |

## Stage 20 — Critical capabilities  (`30_STAGE20_CRITICAL_CAPS.md`)

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH72 | Streaming ingest + reactive triggers + time-travel/as-of + universal summarization | PH41,PH34,PH08 | (cross) | `17 §8` | each capability FSV-proven on a real stream/corpus |

---

## Critical path & parallelism

- **Spine (must be serial):** PH00→PH04→PH05→…→PH09 (Aster core) →
  PH12/PH13 (Forge) → PH17→PH20 (lenses) → PH23/PH24 (search). This is the
  recommended first demo.
- **Parallelizable once the spine exists:** S5 (Loom/Assay) ∥ S7 (Ledger) ∥
  S12 (universal layer) ∥ S13 (resource) once Aster (S1) is up; S6 (Lodestar)
  needs S5's agreement graph; S8 (Ward) needs S3 + Forge; S10 (Anneal) needs
  S4 + S6. S13/S14 are **continuous hardening**, not a one-shot late stage.
- **Sudo-gated (operator):** ZFS dataset creation (PH00 relocation), systemd
  install (PH66) — never block dev; run from `CALYX_HOME` until provisioned.

## BUILD_DONE mapping

The PRD's mechanical `BUILD_DONE` predicate (`dbprdplans/19 §5`) is satisfied
exactly when the corresponding gates above all pass: **CORE=PH05–PH11 ✅ (done)**,
**MATH/ARRAYMATH/COMPRESS=PH12–PH16 ✅**, **LENS=PH17–PH22 ✅**,
**SEARCH=PH23–PH26 ✅**, **DDA_BITS=PH27–PH30 ✅**,
KERNEL/KERNEL_ANY=PH31–PH34, PROVENANCE=PH35–PH36,
GUARD=PH37–PH39, TEMPORAL/DEDUP/RECURRENCE=PH40–PH42, SELFOPT/INTELLIGENCE=
PH43–PH48, ORACLE=PH49–PH52, UNIVERSAL=PH53–PH55, RESOURCE=PH56–PH59,
SECURITY=PH60–PH61, DEPLOY=PH65–PH67, SCALE=PH68, DATA=PH69–PH70,
LEAPABLE=PH71, plus FSV throughout.
