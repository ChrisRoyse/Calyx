# 03 — Phase Map (PH00–PH72)

Every phase, in order, with stage, dependencies, the crate(s) it lands in, the
PRD roadmap phase + axioms it satisfies, and its one-line FSV exit gate. Detail
lives in the per-stage files (`10_…`–`30_…`). Phase IDs are stable handles.

Legend: **Dep** = phases that must be DONE first. **PRD** = `dbprdplans/19`
roadmap phase. **Gate** = the byte-level proof of done (full version in the
stage file). Status: **✅ DONE** · **▶ ACTIVE** (next up) · **· pending**.

---

## Current status (2026-06-10)

| Stage | Phases | Status |
|---|---|---|
| S0 Foundation | PH00–PH04 | ✅ DONE (`calyx-core`) |
| S1 Aster | PH05–PH11 | ✅ DONE, FSV-signed-off (`calyx-aster`); post-sweep PH11 durable tiering #295 FSV-backed; pre-Lodestar durability hardening #333 FSV-backed; #341 derived dense slot-column materialization FSV-backed |
| S2 Forge | PH12–PH16 | ✅ DONE, FSV-signed-off (`calyx-forge`: CPU SIMD + CUDA sm_120 + TurboQuant + MXFP4/grouped GEMM + autotune); CUDA top-k large-k overclaim #303 now fails loud, CUDA normalize now uses the #306 `normalize_rows_f32` device kernel, #307 records GEMM near-zero parity by relative+absolute readback, #316 surfaces grouped GEMM execution mode with a strict fail-loud path, #333 hardens absent-slot sentinel checks with release CUDA FSV, and #338 documents shipped vs deferred Forge backend ops |
| S3 Registry | PH17–PH22 | ✅ DONE, FSV-signed-off (`calyx-registry`: lens runtimes + frozen contract + candle/ONNX + hot-swap/backfill + durable scheduler + capability cards + default panels + temporal E2/E3/E4); PH20 durable add-lens scheduler #311, frozen registered hot-swap guard #314, atomic backfill scheduler persistence #315, durable rollback #321, lifecycle idempotency/backfill-cancel #327, and Registry->Aster->Sextant integration/determinism proof #339 are FSV-backed |
| S4 Sextant | PH23–PH26 | ✅ DONE, FSV-signed-off (`calyx-sextant`: dense/sparse indexes + RRF/provenance + planner/explain + PH26 query filters); PH26 reranker/filter follow-ups #296/#297 are FSV-backed, #308 removes filtered-window and HNSW-update blind spots, #312 makes dense-only Pipeline fail closed, PH25 postings #322 fail closed, PH25 sparse vector readback #323 preserves original sparse IDs, PH25 Pipeline recall headroom #324 is configurable, PH26 reranker candidates #325 are zeroizing-owned, PH26 planned explain #326 integrates planner metadata with executed hits, PH20 inactive-slot gate #327 excludes parked/retired slots from search, PH23/PH24 GPU overclaim #299 now fails loud, and stored-provenance qrels/integration #339 is FSV-backed |
| S5 Loom + Assay | PH27–PH30 | ✅ DONE, FSV-signed-off (`calyx-loom` + `calyx-assay`: DDA cross-terms + bits/differentiation/sufficiency); grounded-trust #294, gate/abundance #309, Loom GPU fail-loud #313, NMI fail-closed #317, seeded bootstrap CI #318, Aster-backed Loom materialization gate #319, and Loom/Assay contract-hardening #340 are FSV-backed |
| S6 Lodestar | PH31–PH34 | ✅ DONE, FSV-signed-off (`calyx-paths` + `calyx-mincut` + `calyx-lodestar`; PH31-PH34 plus #331 raw-vs-tuned, #332 anchor search, #629 docs caveat, #632 helper split before #630/#631 real-corpus readbacks, and #240 exit evidence complete; compact-kernel ≈1% is a raw target, while acceptance is measured final/tuned recall; PH36 trace/reproduce closed in Stage 7) |
| S7 Ledger | PH35-PH36 | ✅ DONE, FSV-signed-off (PH35-PH36 through Stage 7 exit #256; PH36 audit-query quarantine filter hardening #349 signed off) |
| S8 Ward | PH37-PH39 | ✅ DONE, FSV-signed-off (#258-#280, #349, #350, #351, #352, #353, #354, #355, #356, #357, #358, and #359 signed off; exit #280 read back the full Ward surface) |
| S9 Temporal & Dedup | PH40-PH42 | ▶ ACTIVE beyond PH42 closeout (PH40 #373-#378 plus #615/#616/#618/#619 FSV-backed; PH41 #379/#380/#381/#382/#383/#384/#385/#386, #623, #578, #621, #624, #617, #622, #620, #626, #627, and #628 FSV-backed; PH42 readback-surface gate #625 FSV-backed; newer PH42 gaps such as #634/#635/#636 are tracked separately) |
| S10-S20 | PH43-PH72 | · pending |

FSV evidence is summarized in GitHub issue #23 (`[CONTEXT] You are here`).
Latest roots:
- Stage 1 Aster:
  `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`
- Stage 1 Aster PH11 durable tiering:
  `/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608`
- Stage 1-5 pre-Lodestar hardening (#333):
  `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`
- Stage 1-5 evidence manifest cleanup (#336):
  `/home/croyse/calyx/data/fsv-issue336-stage1-5-evidence-manifest-20260608`
- Stage 1 Aster derived slot-column SoA materialization (#341 + post-sweep hardening):
  `/home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58`
- Stage 9 PH40 temporal policy manifest persistence (#373):
  `/home/croyse/calyx/data/fsv-issue373-temporal-policy-manifest-20260609-9ca0a93`
- Stage 9 PH40 temporal policy cold-open hardening (#373 follow-up):
  `/home/croyse/calyx/data/fsv-issue373-temporal-policy-reopen-20260609-a54dcc1`
- Stage 9 PH40 temporal window helpers (#374):
  `/home/croyse/calyx/data/fsv-issue374-time-window-20260609-d872c7c`
- Stage 9 PH40 temporal boost helper (#375):
  `/home/croyse/calyx/data/fsv-issue375-temporal-boost-20260609-a54dcc1`
- Stage 9 PH40 causal confidence gate (#376):
  `/home/croyse/calyx/data/fsv-issue376-causal-gate-20260609-78f9b67`
- Stage 9 PH40 temporal search AP-60 integration (#377):
  `/home/croyse/calyx/data/fsv-issue377-temporal-search-20260610-b428b10`
- Stage 9 PH40 temporal never-dominant / boost-reorder proof (#378):
  `/home/croyse/calyx/data/fsv-issue378-temporal-never-dominant-20260610-2205edb`
- Stage 9 PH40 AP-60 final-surface hardening (#615):
  `/home/croyse/calyx/data/fsv-issue615-ap60-final-surface-20260610-b9a105c`
- Stage 9 PH41 DedupPolicy manifest persistence (#379):
  `/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`
- Stage 9 PH41 dedup engine cosine gate (#380):
  `/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`
- Stage 9 PH41 anchor-conflict guard (#381):
  `/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`
- Stage 9 PH41 `ingest_at(input, at: t)` (#382):
  `/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`
- Stage 9 PH41 recurrence series store (#383):
  `/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`
- Stage 9 PH41 recurrence signature detector (#384):
  `/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`
- Stage 9 PH41 recurrence temporal fallback hardening (#623):
  `/home/croyse/calyx/data/fsv-issue623-recurrence-fallback-20260610-1dc61cf`
- Stage 9 PH41 dedup audit / undo (#385):
  `/home/croyse/calyx/data/fsv-issue385-dedup-audit-20260610-cc9f57b`
- Stage 9 PH41 dedup invariant exit FSV (#386):
  `/home/croyse/calyx/data/fsv-issue386-dedup-invariants-20260610-5fdab01`
- Stage 9 PH41 public recurrence read APIs (#578):
  `/home/croyse/calyx/data/fsv-issue578-periodic-recall-20260610-240de5a`
- Stage 9 PH41 recurrence concurrency hardening (#621):
  `/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`
- Stage 9 PH41 WAL recovery/open serialization (#624):
  `/home/croyse/calyx/data/fsv-issue624-wal-recovery-lock-20260610-1e4b34c`
- Stage 9 PH41 durable dedup policy validation parity (#617):
  `/home/croyse/calyx/data/fsv-issue617-dedup-panel-validation-20260610-07884d9`
- Stage 9 PH41 recurrence WAL-failure error-code contract (#622):
  `/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`
- Stage 9 PH41 recurrence tombstone/reclaim integration (#620):
  `/home/croyse/calyx/data/fsv-issue620-recurrence-reclaim-20260610-209f843`
- Stage 2 Forge PH12 CPU SIMD:
  representative roots `/home/croyse/calyx/data/fsv-q71-20260607115027`
  and `/home/croyse/calyx/data/fsv-q76-20260607122351`; issue evidence
  covers #71-#76.
- Stage 2 Forge CUDA top-k large-k hardening:
  `/home/croyse/calyx/data/fsv-issue303-cuda-topk-large-k-20260608`
- Stage 2 Forge CUDA normalize/GEMM parity hardening:
  `/home/croyse/calyx/data/fsv-issue307-cuda-gemm-parity-20260608`
- Stage 2 Forge PH15 grouped GEMM execution mode:
  `/home/croyse/calyx/data/fsv-issue316-grouped-gemm-mode-20260608`
- Stage 2 Forge backend contract honesty:
  `/home/croyse/calyx/data/fsv-issue338-forge-contract-honesty-20260608`
- Stage 3 atomic suite:
  `/home/croyse/calyx/data/fsv-stage3-atomic-suite-20260607231752`
- Stage 3 PH20 durable backfill scheduler:
  `/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608`
- Stage 3 PH20 frozen registered hot-swap guard:
  `/home/croyse/calyx/data/fsv-issue314-registered-hot-swap-20260608`
- Stage 3 PH20 atomic backfill scheduler persistence:
  `/home/croyse/calyx/data/fsv-issue315-backfill-atomic-persist-20260608`
- Stage 3 PH20 durable scheduler rollback:
  `/home/croyse/calyx/data/fsv-issue321-durable-rollback-20260608`
- Stage 3 PH21 capability cards with Assay-backed metrics:
  `/home/croyse/calyx/data/fsv-issue334-ph21-assay-registry-20260608`
- Stage 3/4 Registry->Aster->Sextant integration:
  `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608`
  (`registry-sextant-readback.json`
  `2163eeb8397de004a8a1c39e04631ccc7aa3f68836a7aa713bca7a6911cf6708`,
  `real-qrels-readback.json`
  `b687d33525be9a32e46feebc333254a089fe7772f0195b6bd5bead2efc16a3ef`)
- Stage 4 Sextant:
  `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`
- Stage 4 Sextant GPU parity/fan-out hardening:
  `/home/croyse/calyx/data/fsv-issue299-gpu-parity-fanout-20260608`
- Stage 4 PH25 postings fail-closed hardening:
  `/home/croyse/calyx/data/fsv-issue322-postings-fail-closed-20260608`
- Stage 4 PH25 sparse vector readback hardening:
  `/home/croyse/calyx/data/fsv-issue323-sparse-vector-readback-20260608`
- Stage 4 PH25 Pipeline recall headroom:
  `/home/croyse/calyx/data/fsv-issue324-pipeline-recall-headroom-20260608`
- Stage 4 PH26 reranker candidate privacy:
  `/home/croyse/calyx/data/fsv-issue325-reranker-candidate-privacy-20260608`
- Stage 4 PH26 planned explain path:
  `/home/croyse/calyx/data/fsv-issue326-planned-explain-path-20260608`
- Stage 1-5 PH20/Sextant lifecycle/search sweep:
  `/home/croyse/calyx/data/fsv-issue327-lifecycle-search-gates-20260608`
- Stage 5 Loom + Assay:
  `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`,
  `/home/croyse/calyx/data/fsv-issue294-assay-grounded-trust-20260608`,
  `/home/croyse/calyx/data/fsv-issue309-stage5-gates-abundance-20260608`,
  `/home/croyse/calyx/data/fsv-issue313-loom-gpu-agreement-20260608`,
  `/home/croyse/calyx/data/fsv-issue317-nmi-fail-closed-20260608`,
  `/home/croyse/calyx/data/fsv-issue318-bootstrap-ci-20260608`,
  `/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608`,
  `/home/croyse/calyx/data/fsv-issue340-loom-assay-hardening-20260608`
- Stage 6 Lodestar PH31/PH32 and PH33 follow-up:
  `/home/croyse/calyx/data/fsv-ph31-20260608`,
  `/home/croyse/calyx/data/fsv-ph32-20260608`,
  `/home/croyse/calyx/data/fsv-issue292-kernel-answer-max-hops-20260608`,
  `/home/croyse/calyx/data/fsv-issue293-loom-assoc-graph-20260608`,
  `/home/croyse/calyx/data/fsv-issue298-build-kernel-groundedness-bound-20260608`,
  `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`,
  `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`,
  `/home/croyse/calyx/data/fsv-issue239-kernel-ledger-provenance-20260608`,
  `/home/croyse/calyx/fsv/ph33_*_20260608.*`
- Stage 6 Lodestar PH34:
  `/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608`,
  `/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`,
  `/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`,
  `/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608`,
  `/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608`,
  `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`,
  `/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`
- Stage 7 Ledger PH35:
  `/home/croyse/calyx/data/fsv-issue242-ledger-entry-20260608`,
  `/home/croyse/calyx/data/fsv-issue243-ledger-codec-20260608`,
  `/home/croyse/calyx/data/fsv-issue244-ledger-appender-20260608`,
  `/home/croyse/calyx/data/fsv-issue245-ledger-redaction-20260608`,
  `/home/croyse/calyx/data/fsv-issue246-ledger-group-commit-20260608`,
  `/home/croyse/calyx/data/fsv-issue247-ledger-actor-ts-20260608`,
  `/home/croyse/calyx/data/fsv-issue248-ledger-integration-smoke-20260608`
- Stage 7 Ledger PH36 and exit:
  `/home/croyse/calyx/data/fsv-issue249-merkle-root-ed25519-20260609`,
  `/home/croyse/calyx/data/fsv-issue347-merkle-range-bound-signatures-20260609`,
  `/home/croyse/calyx/data/fsv-issue348-merkle-vault-real-aster-cf-20260609`,
  `/home/croyse/calyx/data/fsv-issue250-verify-chain-quarantine-20260609`,
  `/home/croyse/calyx/data/fsv-issue251-checkpoint-scheduler-20260609`,
  `/home/croyse/calyx/data/fsv-issue252-reproduce-20260609`,
  `/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609`,
  `/home/croyse/calyx/data/fsv-issue254-audit-query-20260609`,
  `/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609`,
  `/home/croyse/calyx/data/fsv-issue256-stage7-exit-20260609-nomock`
- Stage 8 Ward PH37-PH38:
  `/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`,
  `/home/croyse/calyx/data/fsv-issue259-ph37-t02-20260609`,
  `/home/croyse/calyx/data/fsv-issue260-ph37-t03-20260609-20a2a34`,
  `/home/croyse/calyx/data/fsv-issue261-ph37-t04-20260609-bd35e1e`,
  `/home/croyse/calyx/data/fsv-issue262-ph37-t05-20260609-3dbe1a6`,
  `/home/croyse/calyx/data/fsv-issue263-ph37-t06-20260609-4cde3b7`,
  `/home/croyse/calyx/data/fsv-issue275-ph37-t07-20260609-8b71024`,
  `/home/croyse/calyx/data/fsv-issue277-ph37-t08-20260609-e75ade1`,
  `/home/croyse/calyx/data/fsv-issue278-ph37-t09-20260609-c2d3e30`,
  `/home/croyse/calyx/data/fsv-issue264-ph38-t01-20260609-f95c817`,
  `/home/croyse/calyx/data/fsv-issue265-ph38-t02-20260609-5c23db5`,
  `/home/croyse/calyx/data/fsv-issue266-ph38-t03-20260609-fa0c263`,
  `/home/croyse/calyx/data/fsv-issue267-ph38-t04-20260609-912b707`,
  `/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`,
  `/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`,
  `/home/croyse/calyx/data/fsv-issue350-ph38-guard-id-mismatch-20260609-a1fca2f`,
  `/home/croyse/calyx/data/fsv-issue357-ph38-timestamp-units-20260609-6e3ff73`,
  `/home/croyse/calyx/data/fsv-issue351-ph38-rejection-rate-20260609-c6a2ccc`,
  `/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`,
  `/home/croyse/calyx/data/fsv-issue354-ph38-per-slot-calibration-20260609-f672547`,
  `/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`,
  `/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`,
  `/home/croyse/calyx/data/fsv-issue356-sextant-multislot-guard-20260609-cfea3ac`,
  `/home/croyse/calyx/data/fsv-issue359-sextant-guard-vector-readback-20260609-cf8d4b3`
- Stage 8 Ward PH39:
  `/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609`
  `/home/croyse/calyx/data/fsv-issue270-speaker-lens-20260609-ef729f8-ort126-sm120`
  `/home/croyse/calyx/data/fsv-issue271-style-lens-20260609-a43e546-ort126-sm120`
  `/home/croyse/calyx/data/fsv-issue272-guard-generate-20260609-3bce50c`
  `/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120`
  `/home/croyse/calyx/data/fsv-issue274-ph39-t06-20260609-8e29b51-v2-cpu-ort126`
- Stage 8 Ward exit:
  `/home/croyse/calyx/data/fsv-issue280-stage8-exit-20260609-477d4a4`

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
| PH15 | MXFP4/microscaling + grouped GEMM | PH14 | forge | P4b/`23` | grouped GEMM invariant to N ≥ batched-loop; execution mode read back; strict grouped launch fails loud if unsupported; FP4 within bound where Assay-safe | ✅ FSV |
| PH16 | Autotune config cache | PH15 | forge | `12 §4` | best `(op,shape,dtype,device)` config cached + reused; A/B logged | ✅ FSV |

## Stage 3 — Registry / lenses  (`13_STAGE3_REGISTRY.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH17 | Lens trait + algorithmic + tei-http runtimes | PH12,PH09 | registry | P2/A4 | embed via :8088 twice → identical; algorithmic lens deterministic | ✅ FSV |
| PH18 | Frozen contract + content-addressed LensId | PH17 | registry | P2/A4 | plain `register*` fails closed; weights-hash mismatch → `CALYX_LENS_FROZEN_VIOLATION`; LensId stable across vaults | ✅ FSV (#310) |
| PH19 | candle-local + onnx runtimes | PH18 | registry | P2/A4 | local + ONNX lens produce unit-norm finite vectors; dim guard fires | ✅ FSV |
| PH20 | Hot-swap add/retire/park + lazy backfill | PH19 | registry | P2/A5 | durable add lens → scheduler JSON + no re-embed; backfill observed on slot columns; retire tombstones | ✅ FSV (#311) |
| PH21 | Capability cards / profile | PH20 | registry | A6 | profile returns signal/spread/separation/cost without full ingest | ✅ FSV |
| PH22 | Default panels + temporal lenses E2/E3/E4 | PH21 | registry | A27 | text/code/civic/media panels instantiate; E2/E3/E4 closed-form deterministic | ✅ FSV |

> **Stage 1–5 audit note (2026-06-08):** Subagents and source readback found
> the pre-Lodestar Stage 1–5 hardening set #282-#333 is implemented and
> FSV-backed. Follow-up gaps found during the sweep were tracked in GitHub
> issues and closed with aiwonder readback evidence instead of hidden in docs.
> PH19 ONNX CUDA registration fails loud instead of silently
> falling back to CPU, with explicit CPU compatibility reported separately. PH23 now
> uses native `ef` HNSW traversal, PH24 explain provenance is refreshed from
> stored constellation provenance, WeightedRRF excludes unnamed and AP-60
> temporal slots before PH40, PH20 durable backfill scheduler persists
> watermarks/throttle/restart-resume state, PH27 Loom cross-terms fail closed,
> and PH28/PH30
> persisted Assay rows require vault/anchor scope, Assay estimators reject
> ragged/non-finite sample matrices, PH25 Pipeline enforces sparse candidate
> subsets, PH26 reranker non-2xx fails closed with no public mock scoring
> helper left in the API (#305), and PH22 temporal flags persist onto core Slot
> rows. The accepted seams are explicitly scoped:
> synthetic `LedgerRef` fallback remains only for documents with no stored
> provenance until Stage 7, and full user-facing Assay/abundance CLI commands
> remain in PH62 while Stage 5 readback bytes are already exposed through FSV
> JSON. Closed during sweep hardening: PH31/PH33 real Loom association-graph
> adapter #293, PH30 grounded Assay trust #294, PH11 durable tiering #295, PH26
> reranker search-path ordering #296 (controlled SearchEngine wire FSV, distinct
> from the resident `:8089` Stage 4 readback), and PH26 scalar/anchor/built-in metadata
> filters #297, filtered searches no longer use a fixed `k*8` candidate window,
> and HNSW duplicate vector inserts rebuild neighbor links (#308). PH23/PH24 GPU parity/fan-out overclaim #299 now fails loud
> instead of comparing CPU outputs to themselves. PH13 CUDA top-k large-k
> overclaim #303 now fails loud for `k > 1024` until exact multi-pass merge
> exists. PH27/PH28/PH30 gate and abundance semantics #309 are now FSV-backed.
> PH27 Loom GPU agreement #313 now fails loud in default builds and uses Forge
> CUDA only behind the explicit `calyx-loom/cuda` feature. PH15 grouped GEMM
> execution mode #316 is surfaced in readback and strict grouped launch fails
> loud when unsupported. PH28 seeded bootstrap CI #318 now flows through KSG,
> logistic-probe, AssayGate lens signal, PairGain, and persisted AssayStore
> readback bytes. PH27/PH28 live Aster-backed PairGain materialization #319 now
> feeds Loom planning and xterm CF materialization. PH33 bounded build-time
> groundedness #298 is now FSV-backed. #333 adds SST body CRCs, manifest
> immutable-ref hash verification, compacted-SST recovery, WAL-authoritative
> post-append commit semantics, deadline-based group commit, and release-mode
> absent-slot sentinel checks. No remaining Stage 1-5 implementation blocker is
> hidden in the phase map; future seams are mapped to concrete later phase/card
> owners; that historical frontier has since moved past PH40 and PH41 T08 to
> Stage 9 PH41 follow-ups.

## Stage 4 — Sextant search  (`14_STAGE4_SEXTANT.md`) — ✅ DONE

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH23 | Per-slot HNSW index | PH20 | sextant | P3/`10` | insert+search recall vs brute-force ≥ target on PH23 10k-row FSV; #640 proves 1e6 embedded-scale SingleLens p99=686 us, RRF-6 p99=3570 us, pipeline p99=17507 us | ✅ FSV |
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

## Stage 6 - Lodestar kernel (`16_STAGE6_LODESTAR.md`) - DONE / FSV

| PH | Title | Dep | Crate | PRD/Ax | Gate | Status |
|---|---|---|---|---|---|---|
| PH31 | mincut/paths: graph build + SCC + betweenness | PH27 | mincut/paths | P5/`08` | SCC condensation + betweenness match reference on planted graph | ✅ FSV |
| PH32 | Kernel-graph (~10% target) + directed MFVS (~1% target) | PH31 | lodestar | P5/A10 | algorithm finds planted feedback-vertex-set on synthetic graph | ✅ FSV |
| PH33 | Kernel index + kernel_answer + grounding_gaps | PH32 | lodestar | P5/A11 | final/tuned kernel-only recall >= 0.95*full on >=3 real corpora; raw/tuned/pass_mode read back; gaps listed; below-gate recall fails closed | FSV, including #331/#332; PH36 trace/reproduce closed separately in Stage 7 |
| PH34 | Multi-scope kernel | PH33 | lodestar | A21 | kernel built at ≥4 scopes, each measured recall reported | Done / FSV (#238) |

## Stage 7 - Ledger provenance (`17_STAGE7_LEDGER.md`) - DONE / FSV

| PH | Title | Dep | Crate | PRD/Ax | Gate |
|---|---|---|---|---|---|
| PH35 | Hash-chain append-only CF (in group-commit) | PH09 | ledger | P7/A15 | every mutation writes a chained entry in the WAL group-commit; chain verifies |
| PH36 | Merkle checkpoints + verify_chain + reproduce() | PH35 | ledger | P7 | flip a byte → `verify_chain` detects break at right seq; `reproduce` bit-parity |

## Stage 8 — Ward guard  (`18_STAGE8_WARD.md`) - DONE / FSV

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
**MATH/COMPRESS=PH12–PH16 ✅** with ARRAYMATH foundations in place and true
derived dense slot-column materialization shipped as a sidecar while Stage 1
row-encoded slot CF bytes remain the CRUD/recovery source of truth,
**LENS=PH17–PH22 ✅**,
**SEARCH=PH23–PH26 ✅**, **DDA_BITS=PH27–PH30 ✅**,
KERNEL/KERNEL_ANY=PH31–PH34, PROVENANCE=PH35–PH36,
GUARD=PH37–PH39, TEMPORAL/DEDUP/RECURRENCE=PH40–PH42, SELFOPT/INTELLIGENCE=
PH43–PH48, ORACLE=PH49–PH52, UNIVERSAL=PH53–PH55, RESOURCE=PH56–PH59,
SECURITY=PH60–PH61, DEPLOY=PH65–PH67, SCALE=PH68, DATA=PH69–PH70,
LEAPABLE=PH71, plus FSV throughout.
