# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Status (2026-06-10; Stage 9 PH41 active)

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
quarantine filter hardening #349 is also FSV-signed-off.

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
| `calyx-ward` | S8 PH37-PH39 | guard profile, verdict/error, AllRequired, KofN, OOD wrapper, no-average/no-flatten enforcement, PH37 readback harness, incoming-query `guard_query`, Assay-derived required-slot derivation, kernel-near guard priority, PH38 conformal tau calibration, provisional high-stakes refusal, novelty routing, drift monitoring, injection-corpus FSV, Sextant InRegionOnly guarded search, Ledger-backed calibration/guard-verdict provenance, and PH39 identity-profile, WavLM speaker lens, style lens, `guard_generate()`, identity injection quarantine construction, identity profile store hardening, and Stage 8 exit are FSV-signed-off through #280 |

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
`/home/croyse/calyx/data/fsv-issue265-ph38-t02-20260609-5c23db5`,
and
`/home/croyse/calyx/data/fsv-issue266-ph38-t03-20260609-fa0c263`,
and
`/home/croyse/calyx/data/fsv-issue267-ph38-t04-20260609-912b707`,
and
`/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`,
`/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`,
and
`/home/croyse/calyx/data/fsv-issue350-ph38-guard-id-mismatch-20260609-a1fca2f`,
and
`/home/croyse/calyx/data/fsv-issue357-ph38-timestamp-units-20260609-6e3ff73`,
and
`/home/croyse/calyx/data/fsv-issue351-ph38-rejection-rate-20260609-c6a2ccc`,
and
`/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`,
and
`/home/croyse/calyx/data/fsv-issue354-ph38-per-slot-calibration-20260609-f672547`,
and
`/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`,
and
`/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`,
and
`/home/croyse/calyx/data/fsv-issue356-sextant-multislot-guard-20260609-cfea3ac`,
and
`/home/croyse/calyx/data/fsv-issue359-sextant-guard-vector-readback-20260609-cf8d4b3`,
and
`/home/croyse/calyx/data/fsv-issue349-audit-query-hardening-20260609-5697553`,
and
`/home/croyse/calyx/data/fsv-issue279-ward-ledger-provenance-20260609-55fc1da`,
and
`/home/croyse/calyx/data/fsv-issue269-identity-profile-20260609`,
`/home/croyse/calyx/data/fsv-issue270-speaker-lens-20260609-ef729f8-ort126-sm120`,
`/home/croyse/calyx/data/fsv-issue271-style-lens-20260609-a43e546-ort126-sm120`,
`/home/croyse/calyx/data/fsv-issue272-guard-generate-20260609-3bce50c`,
`/home/croyse/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120`,
`/home/croyse/calyx/data/fsv-issue274-ph39-t06-20260609-8e29b51-v2-cpu-ort126`,
and `/home/croyse/calyx/data/fsv-issue280-stage8-exit-20260609-477d4a4`.

Stage 9 Temporal & Dedup is now the active engine frontier. PH40 is complete
under S9 epic #361, with T01-T06 #373-#378 and post-sweep hardening #615
FSV-signed-off. PH41 T01 #379 through T05 #383 are complete and
FSV-signed-off; the next atomic work is PH41 T06 #384.
Remaining major engine crates (`anneal`, `oracle`, `mcp`, `calyxd`) are still
pending. Ledger PH35 is
FSV-signed-off, including the #345
failure-atomic staging hardening; PH36 Merkle root export #249,
range-bound signing #347, and real Aster `merkle-root --vault` #348 are signed
off. PH36 verify_chain/quarantine #250, checkpoint scheduler #251, reproduce
re-measure #252, reproduce fusion replay #253, and audit query surface #254
are signed off. PH36 exit FSV integration #255 and Stage 7 exit rollup #256
are signed off; PH36 audit-query quarantine filter hardening #349 is signed off,
covering filtered audit queries around unrelated quarantined rows, typed `cx`
mention matching, physical row-key mismatch fail-closed behavior, and durable
Ledger SST readbacks. Stage 8 Ward has #258-#273,
#275/#276/#277/#278, #350, and #353 signed off; PH37 is complete, PH38 T05 is
proven against the real aiwonder injection corpus, PH38 T06 proves Sextant
InRegionOnly guarded search, and #350 hardens novelty guard-id provenance.
PH38 timestamp hardening #357, drift metric semantics hardening #351,
held-out injection split reporting #352, and per-slot calibration health #354
are also signed off; #358 preserves legacy `GuardHealth` JSON compatibility after
#354, #355 preserves Anneal notification retry after hook backpressure, and
#356 requires slot-aware `Query.guard_vectors` for multi-slot InRegionOnly
guarding. #359 adds direct readback of those query vectors and the candidate
slot vectors. #349 hardens PH36 audit query quarantine filtering. #279 adds
`calibrate_with_ledger()` and `guard_with_ledger()` wrappers that append durable
Ledger `kind=Guard` rows for Ward calibration and guard verdicts, then read
those rows back through PH36 audit/provenance while preserving the #349
quarantine contract. #269 adds the PH39 `IdentityProfile` construction and
identity-anchor fail-closed surface with durable JSON and SHA manifest readback.
#270 adds the pinned WavLM speaker lens, #271 adds the pinned style lens, #272
adds `guard_generate()` plus accepted/novel/rejected/provisional readbacks, #273
proves real prompt-injection quarantine on the numeric style slot, #274 proves
PH39 speaker-similarity target FSV, and #280 closes the full Stage 8 Ward exit.
PH40 T01 #373 stores temporal policy in Aster's durable vault manifest with
aiwonder readback at
`/home/croyse/calyx/data/fsv-issue373-temporal-policy-manifest-20260609-9ca0a93`;
post-sweep hardening keeps custom policy authoritative across cold open and
second flush at
`/home/croyse/calyx/data/fsv-issue373-temporal-policy-reopen-20260609-a54dcc1`.
PH40 T02 #374 adds `TimeWindow` helpers and stable-order temporal hit filtering
with aiwonder readback at
`/home/croyse/calyx/data/fsv-issue374-time-window-20260609-d872c7c`.
PH40 T03 #375 adds content-relative `apply_temporal_boost`, attaches
`TemporalScores`, caps temporal alpha at 0.10, preserves zero-content misses at
score 0.0, and reads back boost artifacts at
`/home/croyse/calyx/data/fsv-issue375-temporal-boost-20260609-a54dcc1`.
PH40 T04 #376 adds the causal confidence gate, attaches `CausalConfidence` and
`CausalGateEvidence` for explain/readback, validates causal multipliers in
`[0.0, 10.0]`, and reads back pipeline artifacts at
`/home/croyse/calyx/data/fsv-issue376-causal-gate-20260609-78f9b67`.
PH40 T05 #377 adds `temporal_search` AP-60 integration with primary retrieval
temporal weight `0.0`, pre-boost ranking capture, CLI explain readback, and FSV
artifacts at
`/home/croyse/calyx/data/fsv-issue377-temporal-search-20260610-b428b10`.
PH40 T06 #378 adds deterministic temporal-never-dominant and boost-reorder
proofs with FSV artifacts at
`/home/croyse/calyx/data/fsv-issue378-temporal-never-dominant-20260610-2205edb`.
PH40 post-sweep hardening #615 filters non-positive hits from the final
`temporal_search` surface while preserving boost-stage proof bytes, with FSV
artifacts at
`/home/croyse/calyx/data/fsv-issue615-ap60-final-surface-20260610-b9a105c`.
PH41 T01 #379 adds `DedupPolicy`, `TctCosineConfig`, `TauStrategy`,
`DedupAction`, `OccurrenceId`, and `DedupResult`, persists `dedup_policy` in
Aster's durable vault manifest, adds `calyx readback vault-manifest --field`,
and reads back the manifest bytes at
`/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`.
PH41 T02 #380 adds the bounded content-slot cosine dedup engine, shared
fail-closed cosine math, CLI `readback dedup-check`, exact fallback on DPI
exceed, runtime tau/config validation, and base/slot CF readback evidence at
`/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`.
PH41 T03 #381 adds the anchor-conflict guard before cosine checks, rejects
exact/same-CxId anchor-conflict bypasses, writes reciprocal `online` CF
contested rows, fail-closes anchor-vector validation, and reads back direct
base/online CF evidence at
`/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`.
PH41 T04 #382 adds `ingest_at(input, at: t)` as the Aster temporal ingest
facade, stores caller event time in base rows, writes Ledger payloads for new,
merge, exact-duplicate, and anchor-conflict outcomes, and reads back
base/online/ledger CF bytes at
`/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`.
PH41 T05 #383 adds the Aster-backed recurrence CF and Loom `SeriesStore`
facade, writes occurrence rows and `recurrence.frequency` in the same commit,
derives cadence on read, enforces active-row rollup/retention, adds CLI
`readback recurrence-series`, and proves happy/empty/rollup/oversized bytes at
`/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`
(`recurrence-series-readback.json` BLAKE3
`130010f0aefee719fe5f2b55c2d025e6d016c34f18d3773947597ccffc46b19a`).

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
