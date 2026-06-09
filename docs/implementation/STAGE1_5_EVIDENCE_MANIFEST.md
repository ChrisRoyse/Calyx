# Stage 1-5 Evidence Manifest

Last updated: 2026-06-08 by agent:codex for #336.

This manifest is the single Stage 1-5 audit index. The source of truth is still
the aiwonder bytes under `/home/croyse/calyx`, plus the closed GitHub issue
comments that cite those bytes. PH05-PH30 task-card checkboxes are historical
implementation prompts; after #336 they are not the live work queue. Live work
must be represented by a GitHub issue.

#336 verification root:
`/home/croyse/calyx/data/fsv-issue336-stage1-5-evidence-manifest-20260608`

Root checksum convention: `root_manifest_sha256` is the SHA-256 of the sorted
per-file `sha256sum` listing for a root, using `./`-relative paths from that
root.

## Stage 1 - Aster (PH05-PH11)

| Phase(s) | Issue/ref | aiwonder root | Command/proof | Key hash | Manual SoT readback summary |
|---|---|---|---|---|---|
| PH05-PH11 | #23 / Stage 1 exit | `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216` | `cargo test -p calyx-aster`; `cargo test -p calyx-cli`; crash/readback drills | root_manifest_sha256 `3afe22858f3d004cea7bb33c7e2ecebab99dd11d6c6fcddb8a48493e73ed1509` | WAL, SST, CF, manifest, crash-drill, and CLI readback bytes exist under the Stage 1 exit root. |
| PH11 | #295 | `/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608` | durable tiering FSV and gates | root_manifest_sha256 `6f40840b016efd7669ac14a8e512b6170e95a003602bff0f0675407a8bedb6cf` | Hot/cold tier policy writes and manifest/vault readbacks prove the tiered vault path. |
| PH05-PH11 | #333 | `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608` | pre-Lodestar hardening FSV | root_manifest_sha256 `00cb83b756ca015bb7cef18f10f72c85e73a5c886c1513e293181f422bc20925` | SST body CRCs, manifest ref-hash verification, compacted-SST recovery, WAL-authoritative commit semantics, and deadline group commit are read back. |
| PH05-PH11 | #337 | `/home/croyse/calyx/data/fsv-issue337-aster-durability-residuals-20260608` | Aster durability residual FSV | root_manifest_sha256 `824edabcb161eccdc7e66374207e40ec3326e86903df766328d6d42e6d725fb3` | Torn-tail diagnostics, compaction write-amp bound, and post-WAL router failure behavior are read back from aiwonder artifacts. |
| PH06 | #341 + post-sweep SoA hardening | `/home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58` | derived slot-column SoA materialization FSV | full_manifest_sha256 `6a49f56ab4f87da1e3259d1f8920281cac80d1f2d03ab60600ac8e12eb7e6e3b` | Live `slot_06` row CF SST bytes read back as row codec (`CXS1` SST + dense tag `00`, not `CXA1`); derived `slot-column.cxa1` begins `CXA1`, stores dimension-contiguous column-major f32 values by manifest `CxId` order, manifest `CXSC1` lists 3 `CxId`s at dim 4, chunk SHA-256 matches, recursive `SHA256SUMS.txt` covers nested vault/materialized files, and empty/non-dense/corrupt/path-traversal edges fail closed. |

## Stage 2 - Forge (PH12-PH16)

| Phase(s) | Issue/ref | aiwonder root | Command/proof | Key hash | Manual SoT readback summary |
|---|---|---|---|---|---|
| PH12 | #71-#76 | `/home/croyse/calyx/data/fsv-q71-20260607115027`; `/home/croyse/calyx/data/fsv-q76-20260607122351` | PH12 CPU SIMD checks and numerical guard tests | q71 root_manifest_sha256 `f7b6354caabe304fb7961653512d39eefb497e200fa20ce7d9c7a4c9e34f574d`; q76 root_manifest_sha256 `a392d88bfd2fdeb81e3c3fdf18f19eed37f789d38a1ba3505525dc6e59336f6c` | CPU backend and NaN/Inf guard logs exist; aggregate issue evidence covers the PH12 sequence. |
| PH13 | #303 | `/home/croyse/calyx/data/fsv-issue303-cuda-topk-large-k-20260608` | CUDA top-k large-k fail-loud FSV | root_manifest_sha256 `8f2bdbf4ddf1312f73490718dad17ecbd1e1e812c8f33f245115033df6f01e89` | CUDA exact top-k succeeds within supported ceiling and fails closed above it. |
| PH13-PH14 | #307 | `/home/croyse/calyx/data/fsv-issue307-cuda-gemm-parity-20260608` | CUDA normalize/GEMM parity FSV | root_manifest_sha256 `693a3bd54675430c7c57a26cdc2a97c41f5558daab87f610bb6e8e17dbf63f62` | CUDA normalize and GEMM parity readbacks prove relative/absolute near-zero behavior. |
| PH15 | #316 | `/home/croyse/calyx/data/fsv-issue316-grouped-gemm-mode-20260608` | grouped GEMM execution-mode FSV | root_manifest_sha256 `cc2558425a2b6f3b40d461ee6eef180fac0b257f5c13610c6968d185c28adadd` | Readback records grouped execution mode and strict unsupported-launch failure. |
| PH12-PH16 | #338 | `/home/croyse/calyx/data/fsv-issue338-forge-contract-honesty-20260608` | Forge backend contract honesty FSV | root_manifest_sha256 `9fdbdd4f7b824b9db9bcff122eb7be5c4e8444aedbd313e9b5b48929cefe32f7` | Backend ops, exact CUDA top-k ceiling, promotion log/provenance, and deferred catalog ops are explicit. |

## Stage 3 - Registry (PH17-PH22)

| Phase(s) | Issue/ref | aiwonder root | Command/proof | Key hash | Manual SoT readback summary |
|---|---|---|---|---|---|
| PH17-PH22 | Stage 3 exit | `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752` | Stage 3 atomic suite and manual readbacks | root_manifest_sha256 `1b420a1b2ef5ffabb093788441e0e8ad1f5a3100a7fae6c2d0293891d228f8f8` | Algorithmic/TEI/candle/ONNX registry behavior, panel templates, profiles, and backfill artifacts are present. |
| PH19 | #289 / #301 | `/home/croyse/calyx/data/fsv-issue289-onnx-provider-20260608`; `/home/croyse/calyx/data/fsv-issue301-candle-device-policy-20260608` | local runtime provider/device-policy FSV | #289 root_manifest_sha256 `099bbd364fe2ef27a6a375f963c9c8d3b9a178f8b53675bfa04179e3df5ce1ad`; #301 root_manifest_sha256 `55f2b0f5dce110b290b31f4e96b1fa60f787568294fecbbcae8221fdcdb35253` | ONNX CUDA fail-loud policy and Candle CPU/CUDA truth are recorded with model/device readbacks. |
| PH18 | #310 | `/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608` | frozen contract FSV | root_manifest_sha256 `86fc4906e02a6f4bb35ee4e1962cb3b17f81d9989d8a15d79b0e817b0fc2f890` | Plain registration fails closed and frozen registration artifacts/readbacks exist. |
| PH20 | #300 / #311 / #314 / #315 / #321 / #327 | see roots in #23/#24 and phase docs | hot-swap, durable scheduler, rollback, lifecycle/search FSV | #300 `ea4cade99b3becfa0907c4f21f21bacfb879dd9c119eef3ac672e15535986210`; #311 `eb4e1fb7ad92e13d5eea9b44ad4c449594f915ddbb9e968f71b32cecff63c071`; #327 `70573cb9b8075c1398f8667d13f7eb7b44a6f2404df4118118afa6cea17f61a5` | Durable enqueue, no-reembed scheduler state, fail-closed unregistered add, atomic persist/rollback, and inactive-slot search gates are read back. |
| PH21 | #334 | `/home/croyse/calyx/data/fsv-issue334-ph21-assay-registry-20260608` | Assay-backed capability-card FSV | root_manifest_sha256 `4f0770006ebce84ce849fd5a748e0410e6bb2387dfb069688fc5b4d0db217e3e` | Capability cards show proxy and Assay-owned metrics with scoped Assay rows. |
| PH17-PH26 | #339 | `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608` | Registry->Aster->Sextant integration FSV | root_manifest_sha256 `7532b0575896683f2d9e500166c1c5c88a6a417240f301933eee65570d50d917` | Registry determinism proof, durable scheduler, Aster slot CF rows, Sextant HNSW index, stored-provenance hits, and SciFact stored-provenance qrels are read back. |

## Stage 4 - Sextant (PH23-PH26)

| Phase(s) | Issue/ref | aiwonder root | Command/proof | Key hash | Manual SoT readback summary |
|---|---|---|---|---|---|
| PH23-PH26 | Stage 4 exit | `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414` | Stage 4 full-stack FSV | root_manifest_sha256 `a2eed7c0026186bb04222f604532fadc82cc112d44707a26f8f1bcfc7a9ecd28` | HNSW, sparse/BM25, fusion, planner, freshness, and qrels readback artifacts exist. |
| PH23-PH24 | #299 | `/home/croyse/calyx/data/fsv-issue299-gpu-parity-fanout-20260608` | GPU parity/fan-out honesty FSV | root_manifest_sha256 `080e8e0284961fb89d7acbd9c74310c1bb043c66fc9d0e70e95f8c4de4392c15` | GPU parity shims fail loud and SearchEngine fan-out is documented as per-slot CPU/index-owned. |
| PH25-PH26 | #290 / #296 / #297 / #308 / #312 | see roots in #23/#24 and phase docs | Pipeline, reranker, filters, HNSW-update, no-stage1 FSV | #290 `23bf84d7e12a67275d87fec64f78301ca8a70f779b279732877a02d473e0805f`; #296 `7bbb7aed5522e76b930b80a93959706f476e31278457c03b484b45f45541da43`; #297 `54ccc54bb1f31bcc2c986ec049a79b3df229879e00c556c8cad25fab047255f6`; #308 `07060fd2a478349f0434b9763bbec2c7996e065777e7b3ae96fd00fedf5ded73`; #312 `893dd77f76604d0f5b21443120d1cbbae7f70d4623f23bf3e9102e8011ab5256` | Sparse candidate subsets, real reranker ordering/fail-closed behavior, filters, duplicate-vector rebuilds, and dense-only Pipeline no-hit behavior are read back. |
| PH25-PH26 | #322 / #323 / #324 / #325 / #326 | see roots in #23/#24 and phase docs | postings, sparse-vector, recall headroom, privacy, planned explain FSV | #322 `9440d88ef0ec4c37984989f00e4cb0f55a51f0a5bf8c5c58f61b43db773b1219`; #323 `6097a33f0d0c171d963462aa291651c75d3633bab70a3260eeb0edd90f451879`; #324 `1fbf4cdb8c3dce9e1d4d1578c3eb53b7e614f050a3a7c91e5b07078ba8dfbdc8`; #325 `44a0413fea0d04f545f575ff86286044a951b03e3ebb1428071e53074cbedd6c`; #326 `22aac5e34079be362c84a79110e537bd1ad057e54ae50f57d50ea7c3f225c2c4` | Varint postings fail closed, sparse IDs survive, recall headroom works, reranker request strings are owned/zeroizing, and planned explain includes executed hits. |

## Stage 5 - Loom + Assay (PH27-PH30)

| Phase(s) | Issue/ref | aiwonder root | Command/proof | Key hash | Manual SoT readback summary |
|---|---|---|---|---|---|
| PH27-PH30 | Stage 5 exit | `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final` | Stage 5 full-stack FSV | root_manifest_sha256 `818ef632b7424ae7999c568fd2cbb3c7af19896253e8f555aa785896ffb20574` | Assay CF, xterm CF, and Stage 5 summary readbacks exist. |
| PH27-PH30 | #294 / #309 / #313 / #317 / #318 / #319 | see roots in #23/#24 and phase docs | grounded trust, gates/abundance, Loom GPU honesty, NMI, bootstrap CI, Aster materialization FSV | #294 root_manifest_sha256 `dca91519478bc414cc8a13ecb796f6ddb2bb88960a076237ee65b4925dd1f6de`; #309 `70e6d4fc8238cc0a4635dbeab3fcba50f36e4645493a3335fc665dbebe568a5f`; #313 `2d95c3374ebd86b1dac8c8c03d28decae0df3d5da836f6aee6f40189c1676d7e`; #317 `6c8572e8ad61ae5d87759f856e2808794321c817132ca10edca3c283d34d132c`; #318 `c26a46a4188c33783d3397c9e54471a7c8c673cedadb952ff71f20d1983d7944`; #319 `f365df06a8c97ac12855378c419c4273a07f61f68aeb80e3775051ae1e122502` | Grounded/trusted Assay rows, fail-closed MI/NMI/gate edges, seeded bootstrap CI, Loom GPU fail-loud/default behavior, and live Aster-backed materialization are read back. |
| PH27-PH30 | #340 | `/home/croyse/calyx/data/fsv-issue340-loom-assay-hardening-20260608` | real-data Loom/Assay hardening FSV | root_manifest_sha256 `8d7e65836fdb044f396bcd104805bad2dbe6649e0bcbcb48f2a80a9d03f45ffd` | Real Iris classification persists Assay/xterm rows, non-finite contract edges fail closed, GPU projection fails loud, and Loom materialization defaults fail closed with explicit fallback. |

## Intentional Deferrals

These are not Stage 1-5 blockers and must not appear as stray unchecked
PH05-PH30 checklist lines.

| Deferral | Owner phase/card | Notes |
|---|---|---|
| Derived-CF degraded/self-heal and rebuildable-index recovery | PH44 T01/T03/T06 | Stage 1 owns durable source bytes; corrupt derived state must degrade and rebuild under Anneal self-heal before any shipped claim. |
| Forge catalog math beyond the current `Backend` trait: advanced KSG/NMI paths, bilinear/cross-term kernels, graph kernels, and spectral math | PH27/PH28, PH31/PH32, PH52, PH70 T02 | Stage 2 ships core Forge kernels; later math phases own each consumer-facing advanced operation and its readback. |
| Forge/Sextant scale integrations: sparse GPU ops, ColBERT MaxSim/grouped fan-out, DiskANN/SPANN, and 1e6+ scale validation | PH46 T02/T03/T06, PH68 T01-T06, PH70 T01 | Current Sextant fan-out is CPU/index-owned and documented as such until scale/index phases wire and validate the real integrations. |
| PH16 promotion provenance upgrade from local JSONL to reversible Ledgered Anneal events | PH43 T05/T06, PH46 T05/T06 | JSONL is the Stage 2 stub; Ledger `kind=Anneal` promotion/revert rows are later Anneal work. |
| Store-backed default-panel/runtime activation from templates | PH62 T02/T03/T08, PH63 T02/T03/T08, PH71 T05/T06 | Stage 3 `instantiate_panel` is template-only; CLI/MCP and Leapable swap phases own product activation. |
| Resident-TEI dense-vector qrels and broader real dataset validation | PH69 T01-T03, PH70 T01 | Stage 4 SciFact proves stored-provenance mechanics; dataset acquisition and real-corpora validation own the expanded qrels. |
| Temporal boost, dedup recurrence series, and grounded recurrence wiring | PH40 T01-T06, PH41 T01-T08, PH42 T01-T07, PH72 | Stage 1-5 temporal lenses and reports exist; cross-engine temporal/recurrence behavior is Stage 9 and streaming work. |
| Stage 5 user-facing intelligence commands/tools | PH62 T05/T07/T08, PH63 T06/T08 | Stage 5 ships helper/report APIs and CF persistence, not final CLI/MCP product surfaces. |
| Sufficiency-deficit routing into Anneal lens proposal | PH47 T01-T05 | Stage 5 emits structured deficits; Anneal owns proposal, gate, hot-add, remeasure, and Ledger evidence. |

## Task-Card Cleanup Rule

After #336, PH05-PH30 task cards are archived completion prompts. A checked
historical box means the item is either implemented by the evidence above or
superseded by an intentional deferral linked here. A new unchecked PH05-PH30
box is allowed only if it points to an open GitHub issue and represents live
remaining work.
