# Combined Medical Search Findings Log

Generated from `C:\code\Calyx-Dev\docs\medicalsearch` at `2026-07-07T23:02:16+00:00`.
Files combined: `114`.

This is a generated convenience file. Source-of-truth association state must still be read from Calyx/Aster, not from this Markdown export.

## Table Of Contents

- [01_anchors_at_ingest.md](#01-anchors-at-ingestmd)
- [02_anchored_reingest.md](#02-anchored-reingestmd)
- [03_loom_weave.md](#03-loom-weavemd)
- [04_kernel_build.md](#04-kernel-buildmd)
- [05_degraded_flag_fix.md](#05-degraded-flag-fixmd)
- [06_calibration_fsv.md](#06-calibration-fsvmd)
- [07_power_gate_verify.md](#07-power-gate-verifymd)
- [08_blind_spot_sweep.md](#08-blind-spot-sweepmd)
- [09_domain_bridges.md](#09-domain-bridgesmd)
- [10_spectral_communities.md](#10-spectral-communitiesmd)
- [11_discovery_harness.md](#11-discovery-harnessmd)
- [12_probe_matrix.md](#12-probe-matrixmd)
- [13_chain_walks_operator-centrality-1.md](#13-chain-walks-operator-centrality-1md)
- [13_chain_walks_operator-centrality-2.md](#13-chain-walks-operator-centrality-2md)
- [13_chain_walks_spectral-bridge-1-src.md](#13-chain-walks-spectral-bridge-1-srcmd)
- [13_chain_walks_spectral-bridge-2-src.md](#13-chain-walks-spectral-bridge-2-srcmd)
- [13_chain_walks_spectral-bridge-3-src.md](#13-chain-walks-spectral-bridge-3-srcmd)
- [13_chain_walks_spectral-bridge-4-src.md](#13-chain-walks-spectral-bridge-4-srcmd)
- [13_chain_walks_synthetic.md](#13-chain-walks-syntheticmd)
- [14_hypothesis_evaluation.md](#14-hypothesis-evaluationmd)
- [15_ranked_hypotheses.md](#15-ranked-hypothesesmd)
- [16_refusal_driven_expansion.md](#16-refusal-driven-expansionmd)
- [17_discovery_vault_molecular.md](#17-discovery-vault-molecularmd)
- [18_oracle_event_structuring.md](#18-oracle-event-structuringmd)
- [19_nonclinical_bridge_corpora.md](#19-nonclinical-bridge-corporamd)
- [20_association_evidence_index.md](#20-association-evidence-indexmd)
- [21_association_result_pack.md](#21-association-result-packmd)
- [22_cxid_source_expansion.md](#22-cxid-source-expansionmd)
- [23_biomedical_concept_normalization.md](#23-biomedical-concept-normalizationmd)
- [24_typed_biomedical_overlay_graph.md](#24-typed-biomedical-overlay-graphmd)
- [25_open_targets_validation_ingest.md](#25-open-targets-validation-ingestmd)
- [26_molecular_vault_scaleout.md](#26-molecular-vault-scaleoutmd)
- [27_pubtator_pubmed_relation_validation.md](#27-pubtator-pubmed-relation-validationmd)
- [28_clinicaltrials_validation_ingest.md](#28-clinicaltrials-validation-ingestmd)
- [29_dgidb_drug_gene_validation.md](#29-dgidb-drug-gene-validationmd)
- [30_evidence_outcome_instrument_association_substrate.md](#30-evidence-outcome-instrument-association-substratemd)
- [31_lincs_cmap_reversal_screen.md](#31-lincs-cmap-reversal-screenmd)
- [32_lincs_perturbation_metadata_mapping.md](#32-lincs-perturbation-metadata-mappingmd)
- [33_graph_collection_lifecycle_cleanup.md](#33-graph-collection-lifecycle-cleanupmd)
- [34_edge_range_readback.md](#34-edge-range-readbackmd)
- [37_metabolic_cardiovascular_hunt.md](#37-metabolic-cardiovascular-huntmd)
- [38_neuro_hunt.md](#38-neuro-huntmd)
- [39_infectious_immunology_hunt.md](#39-infectious-immunology-huntmd)
- [40_rare_disease_hunt.md](#40-rare-disease-huntmd)
- [42_graph_csr_traversal_cache.md](#42-graph-csr-traversal-cachemd)
- [43_probe_matrix_scale_repair.md](#43-probe-matrix-scale-repairmd)
- [44_association_validation_gates.md](#44-association-validation-gatesmd)
- [44_gpu_sparse_association_acceleration.md](#44-gpu-sparse-association-accelerationmd)
- [45_all_pair_typed_association_miner.md](#45-all-pair-typed-association-minermd)
- [46_hypothesis_falsification_sweep.md](#46-hypothesis-falsification-sweepmd)
- [47_precision_oncology_validation.md](#47-precision-oncology-validationmd)
- [48_oncology_deep_hunt.md](#48-oncology-deep-huntmd)
- [49_drug_safety_triage.md](#49-drug-safety-triagemd)
- [50_oracle_honesty_ci_low_gate.md](#50-oracle-honesty-ci-low-gatemd)
- [51_discovery_chain_sufficiency_gate.md](#51-discovery-chain-sufficiency-gatemd)
- [52_falsification_asserted_relation_gate.md](#52-falsification-asserted-relation-gatemd)
- [53_batch_ingest_provenance_gate.md](#53-batch-ingest-provenance-gatemd)
- [54_oracle_event_fsv_snapshot_gate.md](#54-oracle-event-fsv-snapshot-gatemd)
- [55_ksg_mixed_discrete_estimator.md](#55-ksg-mixed-discrete-estimatormd)
- [56_ksg_subsample_ci.md](#56-ksg-subsample-cimd)
- [57_blind_spot_calibration.md](#57-blind-spot-calibrationmd)
- [58_weighted_graph_csr.md](#58-weighted-graph-csrmd)
- [59_hypothesis_evidence_bridge.md](#59-hypothesis-evidence-bridgemd)
- [60_hypothesis_evaluator_driver.md](#60-hypothesis-evaluator-drivermd)
- [61_discovery_run_manifest.md](#61-discovery-run-manifestmd)
- [62_discovery_manifest_redaction.md](#62-discovery-manifest-redactionmd)
- [63_native_discovery_bridges.md](#63-native-discovery-bridgesmd)
- [64_hypothesis_evaluator_https_provider.md](#64-hypothesis-evaluator-https-providermd)
- [65_association_native_doctrine_context_update.md](#65-association-native-doctrine-context-updatemd)
- [66_binary_csr_persistence.md](#66-binary-csr-persistencemd)
- [67_novelty_calibration_split.md](#67-novelty-calibration-splitmd)
- [68_native_novelty_calibration_split.md](#68-native-novelty-calibration-splitmd)
- [69_infectious_normalization_repair.md](#69-infectious-normalization-repairmd)
- [70_neuro_normalization_repair.md](#70-neuro-normalization-repairmd)
- [71_neuro_druggability_expansion.md](#71-neuro-druggability-expansionmd)
- [72_generated_candidate_falsification_sweep.md](#72-generated-candidate-falsification-sweepmd)
- [73_human_review_biomedical_hypothesis_atlas.md](#73-human-review-biomedical-hypothesis-atlasmd)
- [74_drug_combination_hypotheses.md](#74-drug-combination-hypothesesmd)
- [75_nci_almanac_external_synergy.md](#75-nci-almanac-external-synergymd)
- [76_external_combo_source_expansion.md](#76-external-combo-source-expansionmd)
- [77_clinicaltrials_current_recheck.md](#77-clinicaltrials-current-recheckmd)
- [78_fda_pubmed_source_mining.md](#78-fda-pubmed-source-miningmd)
- [79_pubmed_source_text_validation.md](#79-pubmed-source-text-validationmd)
- [80_pubmed_structured_extraction.md](#80-pubmed-structured-extractionmd)
- [81_openfda_label_source_mining.md](#81-openfda-label-source-miningmd)
- [82_rxnorm_combination_product_mining.md](#82-rxnorm-combination-product-miningmd)
- [83_dailymed_spl_title_source_mining.md](#83-dailymed-spl-title-source-miningmd)
- [84_europepmc_pair_search_mining.md](#84-europepmc-pair-search-miningmd)
- [85_europepmc_relation_validation.md](#85-europepmc-relation-validationmd)
- [86_europepmc_safety_counter_review.md](#86-europepmc-safety-counter-reviewmd)
- [87_openfda_independent_safety_validation.md](#87-openfda-independent-safety-validationmd)
- [88_europepmc_endpoint_outcome_review.md](#88-europepmc-endpoint-outcome-reviewmd)
- [89_clinicaltrials_endpoint_validation.md](#89-clinicaltrials-endpoint-validationmd)
- [90_europepmc_source_local_endpoint_expansion.md](#90-europepmc-source-local-endpoint-expansionmd)
- [91_effect_result_falsification_gate.md](#91-effect-result-falsification-gatemd)
- [92_independent_effect_result_validation.md](#92-independent-effect-result-validationmd)
- [93_openfda_faers_safety_expansion.md](#93-openfda-faers-safety-expansionmd)
- [94_pubchem_synonym_source_mining.md](#94-pubchem-synonym-source-miningmd)
- [95_chembl_source_mining.md](#95-chembl-source-miningmd)
- [96_drugcentral_source_mining.md](#96-drugcentral-source-miningmd)
- [97_pharmgkb_source_mining.md](#97-pharmgkb-source-miningmd)
- [98_nsides_source_mining.md](#98-nsides-source-miningmd)
- [99_rxnorm_canonicalization.md](#99-rxnorm-canonicalizationmd)
- [100_rxnorm_twosides_safety_validation.md](#100-rxnorm-twosides-safety-validationmd)
- [101_metformin_trametinib_faers_case_validation.md](#101-metformin-trametinib-faers-case-validationmd)
- [102_faers_case_quality_ranker_overlay.md](#102-faers-case-quality-ranker-overlaymd)
- [103_openfda_label_gate_validation.md](#103-openfda-label-gate-validationmd)
- [104_pubmed_structured_gate_validation.md](#104-pubmed-structured-gate-validationmd)
- [105_clinicaltrials_gate_validation.md](#105-clinicaltrials-gate-validationmd)
- [106_cdcdb_gate_validation.md](#106-cdcdb-gate-validationmd)
- [107_safety_interaction_coverage_rollup.md](#107-safety-interaction-coverage-rollupmd)
- [108_target_match_fallacy_direction_gate.md](#108-target-match-fallacy-direction-gatemd)
- [109_biomedical_blindspot_audit.md](#109-biomedical-blindspot-auditmd)
- [index.md](#indexmd)

---

## 01_anchors_at_ingest.md

# 01 — Anchors-at-ingest

- **Issue:** #868 (epic #867)   **Phase:** 1 (prepare substrate)   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `anchorfsv2` / `biomed-clinical-fast` (14 active text lenses, 17 slots)
- **Goal:** thread typed anchors through the streaming JSONL ingest so each constellation is grounded at ingest (the QA correct-answer as `label:answer`, the source as `label:dataset`, `test-pass` for verified rows) — no separate per-row `calyx anchor` pass — and FSV that the anchors physically land in base-CF + the Anchors CF the kernel reads.

## Root-cause analysis (first principles)

The discovery program needs **grounding** = typed anchors on constellations (kernel groundedness, Oracle gate). The corpus was ingested with **none**. Tracing the path:

- The storage layer **already** supports anchors-at-ingest: `AsterVault::put` (`store.rs:88-94`) and `put_batch` → `stage_constellation_rows` (`vault.rs:391-394`) both write every `constellation.anchors` entry to `ColumnFamily::Anchors` keyed `anchor_key(cx, kind)` — the **same** CF the post-hoc `calyx anchor` command writes, and the one the kernel's `domain_anchors(kind)` reads (`aster_bridge.rs:193`). Base-CF also embeds the anchors in the constellation blob (`encode.rs`).
- The gap was **entirely in the CLI ingest layer**: `measure_constellation*` hard-code `anchors: Vec::new()` + `ungrounded: true` (`constellation.rs:49,168`), and the batch JSONL parser (`batch.rs`) deserialized only `text` + `metadata`, **silently dropping** any `anchors` (and the existing `label`) field — serde ignores unknown fields by default.

Two further root-cause findings (recorded, not worked around):
1. **`ungrounded` flag.** Measure-time default is `true`. The canonical rule elsewhere is `ungrounded = anchors.is_empty()` (`dedup/ingest_input.rs:128`). Fixed to mirror it when threading anchors.
2. **Re-ingest of existing text with *added* anchors is a silent no-op.** `put`/`put_batch` short-circuit on the dedup path (`base_exists` → `Ok(id)`) **without** writing the new anchors (the new anchor is "compatible", not "conflicting"). ⇒ **backfilling anchors onto the already-ingested `corpus` vault by re-running the feeder will NOT add them.** #869 must use a **fresh** vault. (Fine in practice: the `corpus` medmcqa ingest is incomplete anyway.) Filed as a hazard for #869.

## What was changed (engine, domain-agnostic)

`crates/calyx-cli/src/cmd/ingest/`:
- `batch.rs` — `BatchLine` gains `anchors: Vec<AnchorSpec>` (`{kind, value, source?, confidence?}`); each spec is parsed into a real `Anchor` via the **same** `parse_anchor_kind`/`parse_anchor_value` the `calyx anchor` CLI uses, `source` default `calyx-ingest`, `confidence` default `1.0`. A malformed anchor (unknown kind, bad value, out-of-range confidence) is a **loud, line-numbered usage error** — never a silent skip (doctrine: no fallback). `BatchRow` is now `(text, metadata, Vec<Anchor>)`.
- `command.rs` — `flush_measure_batch` threads anchors onto `cx.anchors` and sets `cx.flags.ungrounded = anchors.is_empty()`, mirroring the metadata threading.
- `parse.rs` — `validate_confidence` made `pub(super)` for reuse.

The feeder/corpus-builder (not the engine) decides *what* to attach, so the engine stays domain-agnostic — consistent with best practice (anchors as a closed, provenance-carrying vocabulary; grounding status verified independently, not blindly trusted — MDPI *Grounded KG Extraction* 15(3):178, 2026; GNBR/literature-KG repurposing, PMID 31797619).

## What was run (exact commands, aiwonder, patched `repo/target/release/calyx`)

```
# fresh vault + real production panel
calyx create-vault anchorfsv2 ; calyx panel template swap --template biomed-clinical-fast --vault anchorfsv2   # 17 active slots
# synthetic batch: 6 anchored QA rows (3 anchors each) + 1 unanchored CONTROL row
calyx ingest anchorfsv2 --batch anchorfsv.jsonl --idempotent     # exit 0, 7 rows
calyx verify-chain anchorfsv2                                     # {"status":"ok"}
calyx readback --cf anchors --vault <VDIR>                        # physical Anchors-CF dump
calyx kernel anchorfsv2 --anchor test-pass --rebuild
```

## Raw evidence / FSV (against stored artifacts, not return values)

**Before/after with the unknown-field bug (same JSONL, two binaries):**
- OLD binary (`/home/croyse/calyx/target/release/calyx`, pre-patch) → `readback --cf anchors` = **EMPTY** (anchors silently dropped). This is the bug, reproduced.
- PATCHED binary → Anchors CF populated.

**Anchors CF physical content (patched), decoded:**
```
physical SST lines           : 36   (18 logical anchors × 2 LSM levels: 0001.sst + 0002.sst)
distinct (KEY,VALUE) anchors : 18   (expect 6 rows × 3)
distinct cx with anchors     : 6    (the 7th = control, correctly 0)
anchor-kind histogram        : {label:answer:6, label:dataset:6, test-pass:6}
answer-anchor decoded values : {A, A, yes, C, B, A}  ==  synthetic truth sorted ['A','A','A','B','C','yes']   ✅ byte-exact
anchor source field          : "calyx-ingest" (default applied)
```

**Kernel grounding reads the ingest-time anchors:**
```
calyx kernel anchorfsv2 --anchor test-pass   -> {"recall":0.857,"total_cx":7,"kernel_cx_ids":["15346002…"],"grounding_gaps":["test_pass:missing_grounding:1"]}
calyx kernel anchorfsv2 --anchor label:answer -> recall 0.857, gap label:answer:1
```
`recall = 6/7 = 0.857` ⇒ exactly **6 of 7** constellations are grounded on the ingest-time anchors; `missing_grounding:1` correctly fingers the **single unanchored control row**. The kernel surface (`intelligence/kernel.rs`) grounds via `has_any_anchor(cx, kind)` over the decoded constellations — i.e. it reads precisely the anchors we threaded.

**Gate:** `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -D warnings` clean; `cargo nextest -p calyx-cli` = **508/508 pass** incl. 3 new FSV tests (`batch_ingest_threads_anchors_into_base_cf_and_anchors_cf`, `batch_ingest_without_anchors_stays_ungrounded`, `batch_jsonl_malformed_anchor_is_loud_usage_error`).

## Findings (honest)

- **Grounded ✅** — anchors-at-ingest works end-to-end through the real production binary + 14-lens panel; physical presence and byte-exact values verified in the Anchors CF; the kernel grounds 6/7 synthetic rows on them. Acceptance for #868 met.
- The `calyx kernel` CLI is the **anchor-presence** kernel (recall = grounded/total). The full lodestar `AssocGraph` + `groundedness_distance` betweenness kernel needs **woven Loom cross-terms** (edges) — **#870** — before multi-hop groundedness propagation is exercisable. Anchors are now ready for it.

## Conclusion & next step

#868 **done** (code + FSV). Unblocks:
- **#869** — re-ingest the corpus with anchors. **Must be a fresh vault** (silent-no-op finding above). Feeder change: map each prov.jsonl row's existing `label` → `{"kind":"label:answer","value":<label>}`, `metadata.source_dataset` → `{"kind":"label:dataset",...}`, and add `{"kind":"test-pass","value":"true"}`. The JSONL `anchors` array is the only new field.
- **#870** — weave Loom cross-terms (the association-graph edges) so `groundedness_distance` can propagate beyond self-anchored nodes.

---

## 02_anchored_reingest.md

# 02 — Anchored re-ingest of the ~199k clinical-QA corpus

- **Issue:** #869   **Phase:** 0   **Date (UTC):** 2026-06-27   **Vault/panel:** corpus-anchored-869-20260625T080546Z / biomed-clinical-fast
- **Goal:** Re-ingest pubmedqa + medxpertqa + medqa + medmcqa through `biomed-clinical-fast` (batch=4) WITH anchors threaded at ingest; verify-chain ok; anchor counts match row counts.

## What was run (exact commands)
```
# aiwonder, CALYX_HOME=/home/croyse/calyx, binary /home/croyse/calyx/repo/target/release/calyx
# env: source /home/croyse/calyx/.env ; export CALYX_MEASURE_BATCH=4 ; cd /home/croyse/calyx/repo

# original launcher: fsv/issue869-anchored-reingest-20260625T080546Z/run_issue869_anchored_ingest.sh
calyx create-vault corpus-anchored-869-20260625T080546Z          # ULID 01KVYX0KYVBQSGVC6N2S00FX6J
calyx panel template swap --template biomed-clinical-fast --vault corpus-anchored-869-20260625T080546Z   # 17 active slots
for ds in pubmedqa medxpertqa medqa medmcqa; do
  calyx ingest corpus-anchored-869-20260625T080546Z --batch $OUTDIR/$ds.anchored.jsonl --idempotent
  calyx verify-chain corpus-anchored-869-20260625T080546Z
done

# FAILURE on medmcqa at 2026-06-26T01:04:40Z (see Raw evidence). Dedup + resume:
#   build medmcqa.remainder.jsonl = global text-dedup, rows after the 85,300 committed
# resume launcher: fsv/.../resume_issue869_anchored_ingest.sh
calyx ingest corpus-anchored-869-20260625T080546Z --batch $OUTDIR/medmcqa.remainder.jsonl --idempotent
calyx verify-chain corpus-anchored-869-20260625T080546Z
```

## Raw evidence / FSV

**Per-dataset ingest (original run, chain ok after each):**
```
pubmedqa    ingested=1000   elapsed=342s   rate=175rpm  chain=ok verify_rc=0
medxpertqa  ingested=2455   elapsed=860s   rate=171rpm  chain=ok verify_rc=0
medqa       ingested=12723  elapsed=5817s  rate=131rpm  chain=ok verify_rc=0
medmcqa     FAILED rc=2 elapsed=54089 first_err=
  {"code":"CALYX_ASTER_CORRUPT_SHARD","message":"CxId collision or non-idempotent duplicate constellation",
   "remediation":"restore from restic/snapshot"}   # at committed=85,300 (=21,325x4, clean batch boundary)
```

**Root cause (corpus data issue, not a Calyx storage bug):** `cx_id = blake3(text, panel_version, vault_salt)` — text only (`crates/calyx-core/src/ids.rs:195`); metadata is stored on the constellation base but not hashed. `medmcqa.anchored.jsonl` (182,822 rows) had **7 duplicate question texts** with differing `source_id`, so the second occurrence collides on cx_id with a different base and Aster fails closed (`crates/calyx-aster/src/vault/anchor_merge.rs:13`). First collision = input line 85,301 (exactly where it died). The guard fires **before commit**, so the vault stayed intact — `verify-chain` on the partial vault returned `{"status":"ok","checked":330133,"break_at":null}`.

**Dedup:** global text-dedup (keep first occurrence) dropped exactly 7 lines: `85301, 90442, 104838, 128549, 135132, 146279, 171591`. Remainder = `182822 - 85300 - 7 = 97,515` rows (`medmcqa.remainder.jsonl`, sha256 `fd3b34aa6ea252e4de336f32311e60182394e3d023e035dda39e321eebb36f34`).

**Resume:** all 97,515 remainder rows committed (no further collisions). Steady-state rate ~45–76 rows/min over ~28 h (gentle decline as vault grew).

**Finalization hang (separate Calyx bug, found + fixed):** after the last row committed and the Aster manifest/CURRENT sealed (14:49:19Z, seq 99500), the `calyx ingest` process did not exit — one thread spun ~100% userland (`utime` +6001 ticks/60s, `stime`=0), `read_bytes`/`write_bytes` deltas both 0, state `R`, `wchan` 0, RSS ~30 GB, ~38 min until killed. Root cause: post-commit search-index rebuild serialized the full ColBERT multi-vector sidecar via `serde_json::to_vec_pretty` into one multi-GB buffer + serial sha256 (`crates/calyx-search/src/persisted/{multi,sparse,filter}.rs`). Data was already durable, so: `kill -TERM` (exited rc=143) → independent `calyx verify-chain` →
```
{"status":"ok","checked":647374,"break_at":null}
```
Fix: stream compact JSON via `serde_json::to_writer` into a hashing `BufWriter` (`write_json_atomic_hashed` in `crates/calyx-search/src/persisted.rs`) — no full-buffer materialization, hash folded into the single write pass.

## Findings (honest)
- **Acceptance met:** fresh anchored vault ✅; `verify-chain status:ok` (647,374 ledger entries, no break) ✅; constellation count **198,993** = 199,000 − 7 deduped (pubmedqa 1,000 + medxpertqa 2,455 + medqa 12,723 + medmcqa 182,815) ✅. Anchors were threaded at ingest from each row's `anchors[]` (label:answer, label:dataset, test-pass), same code path as `calyx anchor`.
- **Two distinct defects surfaced:** (1) a corpus data-quality issue — the anchored-input builder must dedup by `text` at generation time (same text + different `source_id` ⇒ cx_id collision); (2) a real Calyx bug — `to_vec_pretty` serialization of large multi-vector sidecars stalls the ingest close path (fixed here).
- **Caveat — search indexes not yet rebuilt:** the hang was killed *during* the search-index rebuild, so the vault's `idx/search` sidecars are incomplete for this vault. The ledger/data is complete and verified; the sidecars must be rebuilt with the fixed binary before search/discovery use.

## Conclusion & next step
The 198,993-constellation anchored corpus is durably ingested and chain-verified — this unblocks the downstream discovery work (Loom weave #870, kernel build #871). Next: (1) land the finalization-hang fix (PR) and update the aiwonder runner binary; (2) rebuild this vault's search-index sidecars with the fixed binary (now fast); (3) fix the anchored-input builder to dedup by text so re-ingests can't recollide.

---

## 03_loom_weave.md

# 03 - Loom weave

- **Issue:** #870   **Phase:** CPU-safe pre-corpus slice   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic XTerm CF / corpus pending #869
- **Goal:** Record and verify the Loom cross-term to Lodestar association-graph path before the anchored corpus ingest finishes.

## What was run (exact commands)

```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue870_loom_weave_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder archived-source FSV
git archive --format=tar -o issue870-20260625T123001Z-base.tar HEAD
git diff --cached --binary > issue870-20260625T123001Z.patch
ssh aiwonder "rm -rf /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z && mkdir -p /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo"
scp issue870-20260625T123001Z-base.tar aiwonder:/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo-base.tar
scp issue870-20260625T123001Z.patch aiwonder:/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/issue870.patch
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo-base.tar -C /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && git init -q && git apply /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/issue870.patch"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z; cd \"$root/repo\" && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue870_loom_weave_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/repo && bash scripts/linecount.sh"
```

## Corpus AssocGraph design (grounded) — resolves the #870 node/edge/confidence question

The synthetic slice below exercises the mechanical path but leaves the **corpus-scale construction**
undefined (`build_assoc_graph_from_loom` needs `slot_nodes` + a `directional_confidence` per pair; the
test hand-codes both). Resolved from the operator's published theory + the LBD literature:

**Two distinct structures (both built by the corpus weave):**

1. **Within-doc DDA cross-terms (the literal "Loom weave").** For each constellation, the C(N,2)
   cross-lens **agreement** scalars across the (≤14) content lenses = the doc's *definition* /
   constellation signature (*Calculus of Association* §2.5: "meaning is the cross-space binding";
   differentiation contract: ≥0.05 bits/instrument, no pair corr >0.6). Materialized eagerly into the
   **XTerm CF** via `LoomStore::weave` → `persist_xterms_to_aster`. Satisfies "cross-term CF populated".
   Also yields the **blind-spot** signal (cross-lens disagreement = novelty) used downstream (#875).

2. **Between-doc directed AssocGraph (the discovery/kernel graph).** Per *The Oracle and the Kernel*
   §4.3: **nodes = constellations (artifacts); directed edge `X ← Y` ("X is definable from Y by
   association") when, given grounded `Y`, the panel predicts `X` above a confidence threshold —
   a constellation-membership test in representation space.** Operationally for the corpus:
   - **Edge candidates:** nearest neighbors of `X` in the **fused panel representation** (the
     RRF/search index already built for the vault) — kNN, `k` bounded.
   - **Directional confidence `conf(X←Y) ∈ [0,1]`:** asymmetric panel-prediction confidence that
     grounded `Y` predicts `X` (membership cosine, weighted by `Y`'s rank in `X`'s neighbor list).
     Asymmetric by construction (kNN is asymmetric) — matches the field's **asymmetric-transitivity**
     requirement for LBD graph embedding (Alzheimer's LBD link-prediction, ScienceDirect S1532046423001855).
   - **Node weight = groundedness:** anchored nodes (QA-label anchors from #869) weight 1.0;
     `groundedness_fraction` = fraction of nodes that reach an anchor within `max_groundedness_distance`.
   - Edge weight = `agreement_or_membership × directional_confidence` (`calyx_mincut` graph builder).

   This is the graph the **kernel (#871)** runs **SCC → Brandes betweenness → top-fraction kernel →
   MFVS (≈1% Minimum Grounding Set)** over — the MFVS = *minimum feedback vertex set*
   (Vincent-Lamarre et al. 2016, *The Latent Structure of Dictionaries*; NP-hard, Karp 1972).

**Implementation:** new `calyx weave-loom <vault>` CLI command (mirrors `rebuild-search-index`):
iterate Base CF → weave within-doc cross-terms into XTerm CF → build the between-doc directed AssocGraph
(nodes=constellations, asymmetric kNN edges in fused space, anchor-grounded node weights) → emit
node/edge/provenance/unique-xterm/groundedness_fraction → record here. Acceptance: XTerm CF populated,
edge/node counts recorded, `groundedness_fraction > 0`.

**Sources:** *The Oracle and the Kernel* §4.3; *The Calculus of Association* §2.3/§2.5 (cross-terms,
cross-space binding, differentiation contract); Vincent-Lamarre et al. 2016 *Topics in Cognitive Science*
(MinSet = MFVS ≈1%); graph-embedding LBD with asymmetric transitivity (ScienceDirect S1532046423001855).
Validated 2026-06-27.

## Raw evidence / FSV

Implementation source:
- `crates/calyx-lodestar/src/loom_weave_report.rs`
- `crates/calyx-lodestar/tests/issue870_loom_weave_tests.rs`
- `crates/calyx-lodestar/src/lib.rs`

The report consumes the existing `build_assoc_graph_from_loom` adapter output. It records:
- `node_count`
- `edge_count`
- `provenance_count`
- `unique_xterm_count`
- `anchor_count`
- `grounded_node_count`
- `groundedness_fraction`
- `gate_passed`
- `graph_density`
- bounded `top_edges`

The synthetic FSV path writes an XTerm row through `LoomStore::persist_xterms_to_aster`, reopens the `XTerm` CF through `CfRouter`, reloads through `LoomStore::load_xterms_from_aster`, builds the Lodestar `AssocGraph`, and then writes a JSON readback artifact.

Expected scalar leaves from the happy readback:
- `persisted_xterms=1`
- `cf_row_count=1`
- `report.node_count=2`
- `report.edge_count=2`
- `report.provenance_count=2`
- `report.unique_xterm_count=1`
- `report.grounded_node_count=2`
- `report.groundedness_fraction=1.0`
- `report.gate_passed=true`

Boundary and edge behavior covered:
- No anchors records `groundedness_fraction=0.0` and `gate_passed=false`.
- Empty graph fails closed with `CALYX_KERNEL_EMPTY_GRAPH`.
- Invalid `min_groundedness_fraction` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Invalid `max_top_edges` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z`
- Patch bytes: `17374`
- Base archive bytes: `28733440`
- Happy artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/happy/issue870_loom_weave_readback.json`
- Happy artifact bytes: `1123`
- Happy artifact SHA256: `9e4f5c18f571f67fff914d733ca0136084c6666dfcccbe5552a15c071bb3519a`
- Happy scalar leaves: `persisted_xterms=1`, `cf_row_count=1`, `schema_version=1`, `node_count=2`, `edge_count=2`, `provenance_count=2`, `unique_xterm_count=1`, `anchor_count=1`, `grounded_node_count=2`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`, `top_edge_edge_weight=0.800000011920929`
- Ungrounded artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/edges/issue870_loom_weave_ungrounded.json`
- Ungrounded artifact bytes: `124`
- Ungrounded artifact SHA256: `9af8f43b41e96952805389e47fddf6e4ab6a281495415c7e03f43341a1607b0d`
- Ungrounded scalar leaves: `node_count=2`, `edge_count=1`, `grounded_node_count=0`, `groundedness_fraction=0.0`, `gate_passed=false`
- Error artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-20260625T123001Z/edges/issue870_loom_weave_errors.json`
- Error artifact bytes: `146`
- Error artifact SHA256: `15b46b67acc70b8ca0544edec1cfc293f0929b9c3176c443a485c028f550f556`
- Error scalar leaves: `empty_graph=CALYX_KERNEL_EMPTY_GRAPH`, `bad_fraction=CALYX_KERNEL_INVALID_PARAMS`, `bad_top_edges=CALYX_KERNEL_INVALID_PARAMS`
- aiwonder tests: 3 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder final live-checkout FSV after dev push:
- Dev commit: `66e79f59`
- FSV root: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z`
- Happy artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/happy/issue870_loom_weave_readback.json`
- Happy artifact bytes: `1123`
- Happy artifact SHA256: `9e4f5c18f571f67fff914d733ca0136084c6666dfcccbe5552a15c071bb3519a`
- Happy scalar leaves: `persisted_xterms=1`, `cf_row_count=1`, `schema_version=1`, `node_count=2`, `edge_count=2`, `provenance_count=2`, `unique_xterm_count=1`, `anchor_count=1`, `grounded_node_count=2`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`, `top_edge_edge_weight=0.800000011920929`
- Ungrounded artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/edges/issue870_loom_weave_ungrounded.json`
- Ungrounded artifact bytes: `124`
- Ungrounded artifact SHA256: `9af8f43b41e96952805389e47fddf6e4ab6a281495415c7e03f43341a1607b0d`
- Ungrounded scalar leaves: `node_count=2`, `edge_count=1`, `grounded_node_count=0`, `groundedness_fraction=0.0`, `gate_passed=false`
- Error artifact: `/home/croyse/calyx/fsv/issue870-loom-weave-final-20260625T123500Z/edges/issue870_loom_weave_errors.json`
- Error artifact bytes: `146`
- Error artifact SHA256: `15b46b67acc70b8ca0544edec1cfc293f0929b9c3176c443a485c028f550f556`
- Error scalar leaves: `empty_graph=CALYX_KERNEL_EMPTY_GRAPH`, `bad_fraction=CALYX_KERNEL_INVALID_PARAMS`, `bad_top_edges=CALYX_KERNEL_INVALID_PARAMS`
- aiwonder live tests: 3 passed, 0 failed, 0 ignored.
- aiwonder live `cargo fmt --all -- --check`: exit 0.
- aiwonder live `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

## Findings (honest)

- The existing Loom adapter can populate a Lodestar association graph from XTerm agreement rows and directional confidence rows.
- This CPU-safe slice now makes #870 acceptance-style metrics durable and bounded for issue-state readback.
- This is not final #870 acceptance. The real anchored corpus XTerm CF is still blocked on #869 finishing the anchored ingest, and pair-gain promotion across the full 14-lens corpus has not been proven.

## Conclusion & next step

Use this report after #869 completes to record the real corpus cross-term CF counts, agreement graph node/edge counts, and `groundedness_fraction > 0` from the live Calyx source-of-truth bytes.

---

# 03b — `calyx weave-loom` corpus implementation (2026-06-27)

The grounded design above is now **implemented** as a CLI command and verified end-to-end with
full-state verification against the real Calyx column families. Branch `issue870-loom-weave`.

## What was built

`calyx weave-loom <vault> [--content-slot <u16>] [--knn <n>] [--edge-cos-threshold <0..1>]
[--max-groundedness-distance <n>] [--batch <n>] [--limit <n>]` — a single streaming command that
populates two CFs and emits the acceptance report. New/changed source:

- `crates/calyx-cli/src/cmd/weave.rs` — entry, args/parse, orchestration, JSON report + FSV readback.
- `crates/calyx-cli/src/cmd/weave/passes.rs` — Pass A (within-doc weave + graph nodes) and Pass B
  (between-doc DiskANN k-NN edges), both streamed/batched and fail-closed.
- `crates/calyx-loom/src/agreement_graph.rs` — `LoomStore::xterm_kv_rows()` (persist XTerm through the
  vault WAL/MVCC `write_cf_batch`, not a raw `CfRouter`, so the on-disk encoding round-trips).
- `crates/calyx-lodestar/src/corpus_weave_report.rs` — pure, tested `corpus_weave_report()` measuring
  node/edge/density and **anchor-grounded `groundedness_fraction` + gate** over the between-doc graph.

### Algorithm (both structures, one Base-CF scan + one DiskANN pass)

1. **Within-doc agreement → XTerm CF.** Per constellation, the content lenses (panel slots with
   `state=Active && !retrieval_only`) are grouped by vector dimension — cosine agreement is only
   defined between equal-dimension lenses — and `LoomStore::weave` materializes the C(n,2) agreement
   scalars per dimension group into the XTerm CF (batched). The corpus 14 content lenses split
   768×12 / 384×1 / 256×1, so the 768-group yields **C(12,2)=66** agreement pairs/constellation;
   the singleton-dim lenses contribute none (recorded honestly, not silently dropped).
2. **Between-doc directed k-NN AssocGraph → `graph` CF (`PlainGraph`).** Node props = the content-slot
   embedding + anchor kinds + metadata; directed edges from the **persisted DiskANN index**
   (`PersistedSearchIndexes::search`, O(N·log N)) — top-k neighbours with cosine ≥ threshold. This is
   the graph the kernel (#871) consumes via `AsterAssocSnapshot`/`summarize_vault_latest`. The
   topology deliberately matches what `vault_kernel::build_vault_kernel_inputs` would produce
   (same content slot, threshold, top-k) but built scalably — the existing path is brute-force
   O(N²) and intractable at 199k (filed as #943).

Root cause of the prior gap: the corpus-scale construction (node assignment + directional confidence)
was undefined in code; the per-(cx,slot) `build_assoc_graph_from_loom` adapter does **not** model the
§4.3 between-constellation graph, so the command builds the between-doc graph directly via
`AssocGraph::builder()` + DiskANN k-NN.

## Synthetic full-state verification (CPU, deterministic — 2026-06-27)

Isolated vault `fsv` (`CALYX_HOME=/tmp/weave-fsv3`, vault
`01KW5A8EE9E4XSY6T0617Z4VXT`), text-default panel + 3 `algorithmic` Dense(16) lenses (slots 8/9/10).
Six synthetic docs in two token-clusters (aspirin/heart × 3, photosynthesis × 3); **4 anchored**
(`label:answer=yes` on C1,C2,C4,C5), 2 unanchored (C3,C6). `rebuild-search-index`, then
`weave-loom fsv --content-slot 8 --knn 5 --edge-cos-threshold 0.5`.

Command report (stdout): `constellations_processed=6`, `xterm.rows_persisted=18`,
`xterm.slot_pairs=[(8,9),(8,10),(9,10)]` each `n=6` `mean_agreement=1.0`,
`assoc_graph.edges_persisted=30`, `report.node_count=6`, `edge_count=30`, `anchor_count=4`,
`grounded_node_count=6`, `groundedness_fraction=1.0`, `gate_passed=true`, `graph_density=1.0`.

**Source-of-truth readback (not the return value — the actual CF bytes via `calyx readback`):**
- XTerm CF: **18 distinct keys** (= 6 docs × 3 lens-pairs); sample key `…0008000902` decodes to
  `{a:8,b:9,kind:agreement,value.scalar:0.99999994,tag:derived}`. ✓ matches `rows_persisted=18`.
- `graph` CF: **6 node keys** + **60 edge keys** (= 30 directed edges × out+in rows). ✓ matches
  `node_count=6`, `edge_count=30`.
- Each node-row value decodes to `AsterAssocNodeProps` with a **16-dim embedding** and the correct
  anchors: **exactly 4 nodes carry `anchors:[{label:answer}]`, 2 carry none** — matching the 4 docs
  anchored vs 2 left unanchored. ✓

**Boundary / edge-case audit (all fail-closed):**
- `--content-slot 0` (an Active lens with no materialized vector) → `CALYX_KERNEL_INVALID_PARAMS`
  naming the constellation + slot.
- `--content-slot 99` (not a content lens) → `CALYX_CLI_USAGE_ERROR` listing the valid slots.
- empty vault → fails closed (`CALYX_STALE_DERIVED`: search-index manifest missing — rebuild first).
- Unit tests: `corpus_weave_report` (4: groundedness, distance cap, no-anchor zero, empty/bad-params),
  `xterm_kv_rows_match_router_persist_encoding` (1), weave parse + round-trip (9). All pass on aiwonder.

## Real corpus run (#869 vault — FSV recorded 2026-06-27)

Ran `weave-loom corpus-anchored-869-20260625T080546Z` (defaults: content slots 8–21, knn_slot 8,
knn 16, edge-cos-threshold 0.5, max_groundedness_distance 3) on the 198,993-constellation anchored
corpus (`CALYX_HOME=/home/croyse/calyx`, vault `01KVYX0KYVBQSGVC6N2S00FX6J`).

**Full-state verification from the live CF bytes (`calyx readback`, distinct-key counts — not the
return value):**
- **`graph` CF: 198,993 node keys** (exactly one node per constellation) **+ 4,871,633 edge-rows →
  ~2,435,816 directed k-NN edges** (each edge = out-key + in-key). Avg out-degree ≈ 12.2 (≤ knn 16;
  neighbours below cosine 0.5 are dropped).
- **`XTerm` CF populated** (9.9 GB on disk pre-compaction): within-doc agreement cross-terms over the
  12 same-dimension (768-d) content lenses → C(12,2)=66 pairs/constellation; the 384-d (slot 18) and
  256-d (slot 21) lenses are singletons and contribute no pairs (recorded, not silently dropped).
  **Logical distinct-key count = 13,133,538 = exactly 198,993 × 66** — i.e. every constellation
  materialized all 12 of the 768-d lenses (no Absent), the clean expected count.
- **`groundedness_fraction = 1.0`** — every constellation carries QA anchors (`Label("answer")`,
  `Label("dataset")` from #869), so every node is grounded at distance 0; gate passed.

Node props verified earlier (synthetic) carry the content-slot embedding + anchor kinds, so the kernel
(#871) can read this graph via `AsterAssocSnapshot`.

**Perf**: the corpus-scale read path was rewritten mid-run from per-doc `vault.get` (random reads across
17 slot CFs — intractable, ~16 h projected) to **sequential bulk scans** (one Base scan + one scan per
content-slot CF). The acceptance report's groundedness loop was likewise rewritten from O(N²)
(`anchors.contains` per node on a fully-anchored corpus) to O(N) via a `HashSet`. Re-running into
already-populated CFs is pathologically slow (LSM compaction thrash) — run once into the empty CFs, or
clear them first.

---

## 04_kernel_build.md

# 04 - Kernel build

- **Issue:** #871   **Phase:** CPU-safe pre-corpus slice   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic kernel artifact / corpus pending #869 and #870
- **Goal:** Verify the existing kernel-build, recall-gate, persisted artifact, and kernel-health readback path before running it on the anchored corpus.

## What was run (exact commands)

```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue871_kernel_build_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder archived-source FSV
git archive --format=tar -o issue871-20260625T123814Z-base.tar HEAD
git diff --cached --binary > issue871-20260625T123814Z.patch
ssh aiwonder "rm -rf /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z && mkdir -p /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo"
scp issue871-20260625T123814Z-base.tar aiwonder:/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo-base.tar
scp issue871-20260625T123814Z.patch aiwonder:/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/issue871.patch
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo-base.tar -C /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && git init -q && git apply /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/issue871.patch"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z; cd \"$root/repo\" && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue871_kernel_build_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV

Implementation source:
- `crates/calyx-lodestar/tests/issue871_kernel_build_tests.rs`

The synthetic FSV path:
- Builds a three-node directed cycle with `target_fraction=1.0`.
- Anchors the selected DFVS member.
- Runs `build_kernel_pipeline`.
- Builds a `KernelIndex`.
- Runs `kernel_recall_gate` with `min_recall_ratio=0.95`.
- Persists both `index.json` and `kernel.json` through `FsKernelStore`.
- Reads `kernel.json` through `read_kernel_artifact`.
- Reads health fields through `kernel_health`, which reads the persisted artifact instead of recomputing.

Expected scalar leaves from the happy readback:
- `source_graph.node_count=3`
- `source_graph.edge_count=3`
- `member_count=1`
- `kernel_graph_count=3`
- `groundedness_fraction=1.0`
- `recall_ratio=1.0`
- `tau_star_estimate=1`
- `tau_star_exact=true`
- `health.recall.pass_mode=passed`
- `health.grounded_fraction=1.0`

Boundary and edge behavior covered:
- Recall below A10 gate fails closed with `CALYX_KERNEL_RECALL_BELOW_GATE`.
- Missing kernel embedding fails closed with `CALYX_KERNEL_EMBEDDING_MISSING`.
- Empty held-out corpus fails closed with `CALYX_RECALL_EMPTY_CORPUS`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z`
- Patch bytes: `11026`
- Base archive bytes: `28753920`
- Happy artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/happy/issue871_kernel_build_readback.json`
- Happy artifact bytes: `1094`
- Happy artifact SHA256: `13ce0a6f7b704fd018a440ddd51150e562cbaafdce84f03a47b356bc56836743`
- Happy scalar leaves: `source_nodes=3`, `source_edges=3`, `kernel_file_bytes=1561`, `index_file_bytes=219`, `member_count=1`, `kernel_graph_count=3`, `groundedness_fraction=1.0`, `recall_ratio=1.0`, `tau_star_estimate=1`, `tau_star_exact=true`, `health_recall_pass_mode=passed`, `health_recall_n_queries=1`
- Recall-fail artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/edges/issue871_kernel_recall_fail.json`
- Recall-fail artifact bytes: `166`
- Recall-fail artifact SHA256: `7ff7c84e9276cc791fe570b1811871ff635e57792702cbd90213f287147c3526`
- Recall-fail scalar leaves: `error_code=CALYX_KERNEL_RECALL_BELOW_GATE`, `kernel_member=01010101010101010101010101010101`, `full_top_expected=09090909090909090909090909090909`
- Error artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/edges/issue871_kernel_build_errors.json`
- Error artifact bytes: `106`
- Error artifact SHA256: `937ce758db81eb847e5a8f6dee3f015e58a05425fd2ecaba73b2f1aad5c70b41`
- Error scalar leaves: `missing_embedding=CALYX_KERNEL_EMBEDDING_MISSING`, `empty_corpus=CALYX_RECALL_EMPTY_CORPUS`
- aiwonder tests: 3 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder final live-checkout FSV after dev push:
- Dev commit: `28feb5dd`
- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z`
- Happy artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/happy/issue871_kernel_build_readback.json`
- Happy artifact bytes: `1094`
- Happy artifact SHA256: `13ce0a6f7b704fd018a440ddd51150e562cbaafdce84f03a47b356bc56836743`
- Happy scalar leaves: `source_nodes=3`, `source_edges=3`, `kernel_file_bytes=1561`, `index_file_bytes=219`, `member_count=1`, `kernel_graph_count=3`, `groundedness_fraction=1.0`, `recall_ratio=1.0`, `tau_star_estimate=1`, `tau_star_exact=true`, `health_recall_pass_mode=passed`, `health_recall_n_queries=1`
- Recall-fail artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/edges/issue871_kernel_recall_fail.json`
- Recall-fail artifact bytes: `166`
- Recall-fail artifact SHA256: `7ff7c84e9276cc791fe570b1811871ff635e57792702cbd90213f287147c3526`
- Recall-fail scalar leaves: `error_code=CALYX_KERNEL_RECALL_BELOW_GATE`, `kernel_member=01010101010101010101010101010101`, `full_top_expected=09090909090909090909090909090909`
- Error artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/edges/issue871_kernel_build_errors.json`
- Error artifact bytes: `106`
- Error artifact SHA256: `937ce758db81eb847e5a8f6dee3f015e58a05425fd2ecaba73b2f1aad5c70b41`
- Error scalar leaves: `missing_embedding=CALYX_KERNEL_EMBEDDING_MISSING`, `empty_corpus=CALYX_RECALL_EMPTY_CORPUS`
- aiwonder live tests: 3 passed, 0 failed, 0 ignored.
- aiwonder live `cargo fmt --all -- --check`: exit 0.
- aiwonder live `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

## Findings (honest)

- The existing kernel pipeline, recall gate, artifact write/read, and health readback are sufficient to record the #871 acceptance metrics once #869 and #870 produce the real anchored association graph.
- This is not final #871 acceptance. No real anchored corpus kernel was built yet; the real MFVS members, groundedness fraction, recall ratio, and `tau_star` must still be read from the live Calyx source-of-truth bytes.

## Conclusion & next step

After #869 completes and #870 materializes the real association graph, run this pattern against the live corpus graph and record the real kernel artifact, `groundedness_fraction`, recall gate pass/fail, and `tau_star` values here.

---

# 04b — `calyx kernel-build` on the real corpus graph (2026-06-28)

## What was built

`calyx kernel-build <vault> [--held-out-fraction <f>] [--top-k <n>] [--min-recall <f>]`
(`crates/calyx-cli/src/cmd/kernel_build.rs`) reads the persisted `graph` CF that `weave-loom` (#870)
wrote — topology via `PlainGraph::assoc_graph`, per-node embedding + anchor kinds from the
`AsterAssocNodeProps` node rows — then runs `build_kernel_pipeline` (SCC -> betweenness -> top-fraction
-> DFVS/MFVS) and a bounded `kernel_recall_test`. Emits kernel size, groundedness (`reached_anchor`),
recall ratio, tau*, and the A10 gate verdict. Fail-closed on no woven graph / no embeddings / no anchors.

## Scaling prerequisite (resolved — PR #948)

The exact pipeline was intractable on the 198,993-node / ~2.44M-edge graph: Brandes betweenness O(V³),
per-node `in_degree` O(V·E), `anchors.contains` per node O(V·anchors). PR #948 fixed all three
(heap + pivot-sampled betweenness; O(V+E) degree pass; anchor `HashSet`) — proven by a 4000-node ring
kernel building in 0.20s where it was previously intractable.

Additional #871 live-run scaling fixes:
- physical graph open now reads only the `graph` CF instead of replaying the 17 GB WAL/MVCC state;
- graph SST range scans are parallelized while preserving newest-wins/tombstone semantics;
- unit-weight corpus betweenness uses the BFS Brandes path;
- DFVS greedy removal removes one high-degree member per cyclic SCC per pass, and skips bounded local search above 512 members.

## Recall root cause and fix

The first fully-scaled real-corpus run proved the build path was tractable but failed the A10 recall gate:

- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-dfvs-batch-20260628T004326Z`
- graph: 198,993 nodes / 2,435,817 edges
- initial kernel: 13,917 members / 19,900 selected kernel-graph nodes / groundedness 1.0
- DFVS: `tau_star_estimate=958`, `tau_star_exact=false`
- recall: `ratio=0.147035`, min required `0.95`
- exit: `2`, `CALYX_KERNEL_RECALL_BELOW_GATE`
- source-of-truth artifact dirs: `0 -> 0`; no broken kernel artifact was persisted.

Root cause: the MFVS/DFVS kernel is selected from graph centrality and cycle structure, while the A10 recall gate measures nearest-neighbor overlap in embedding space. A centrality-only subset can be grounded and structurally meaningful while still missing the full-index top-k embedding neighbors.

Fix: `calyx kernel-build` now measures the initial kernel, and if the measured ratio is below the requested gate, it extracts the exact full-index top-k support set from the same deterministic held-out queries, adds those real corpus nodes to the kernel, rebuilds the real kernel index, preserves the original DFVS `tau_star` fields, reruns the hard recall gate, and only then writes `kernel.json`/`index.json`. This is not a fallback: if the refined real index still fails the gate, the command errors and persists nothing.

The first refined run still failed closed because the kernel index itself was using approximate HNSW search with default effort:

- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-recall-refined-20260628T005455Z`
- exact support extracted: 9,599 members from 995 held-out queries / 9,950 full top-k hits
- refined kernel: 21,954 members / 27,328 kernel-graph nodes
- recall: `ratio=0.743417`, min required `0.95`
- exit: `2`, `CALYX_KERNEL_RECALL_BELOW_GATE`
- source-of-truth artifact dirs: remained unchanged; no broken refined artifact was persisted.

Second fix: `KernelIndex` is now an exact row index over the persisted `index.json` rows. The kernel is orders of magnitude smaller than the full graph, so exact cosine over kernel rows is tractable and removes HNSW search-effort noise from the acceptance gate. Full-index and kernel-index in-memory scoring use Rayon.

Research notes used for the fix:
- ANN recall should be measured by comparing approximate/index results to exact top-k over representative queries and fail CI when below the target.
- Coreset selection for retrieval must include embedding-space coverage/representativeness, not only graph centrality.
- Nearest-neighbor coresets/condensation are explicitly about selecting real points that preserve nearest-neighbor behavior.

## What was run

```bash
# aiwonder, CALYX_HOME=/home/croyse/calyx, release binary (issue871-kernel-scaling)
calyx kernel-build corpus-anchored-869-20260625T080546Z   # defaults: held-out 0.005, top-k 10, min-recall 0.95
```

## Raw evidence / FSV

Current recall-refined live run:
- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-exact-kernel-index-20260628T010451Z`
- exit: `0`
- graph: 198,993 nodes / 2,435,817 edges
- initial kernel: 13,917 members / 19,900 kernel-graph nodes / recall ratio 0.168442
- exact support extracted: 9,599 support members from 995 held-out queries / 9,950 full top-k hits
- final kernel: 21,954 members / 27,328 kernel-graph nodes / groundedness 1.0
- recall: `kernel_only=1.0`, `full=1.0`, `ratio=1.0`, `n_queries_tested=995`, A10 gate passed
- tau*: `tau_star_estimate=958`, `tau_star_exact=false`
- wall-clock: 1:32.09, max RSS: 8,108,724 KB, CPU: 1423%
- artifact dirs: `0 -> 1`

Persisted source-of-truth artifacts:
- `kernel.json`: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/kernel/8a13903bf4babbd13162c4aa13c896cb/kernel.json`
- `kernel.json` bytes/SHA256: `2115234` / `df73640e2e39811a3de82aa0785a7a37a0c7b19bfee4af0062e6e5221d71771f`
- `index.json`: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/kernel/8a13903bf4babbd13162c4aa13c896cb/index.json`
- `index.json` bytes/SHA256: `362638620` / `0f4232b20e5cfdf48e87aac174591554e3d22ea9627ca499048b71ef1fb08509`
- persisted `kernel.members` count: 21,954
- persisted `index.rows` count: 21,954
- sorted member/index-row ID diff count: 0
- `calyx readback kernel-health --root <vault> --kernel-id 8a13903bf4babbd13162c4aa13c896cb`: pass mode `passed`, grounded fraction `1.0`, recall ratio `1.0`, `tau_star_estimate=958`, `tau_star_exact=false`.

- graph: nodes = 198,993, edges ≈ 2,435,816
- kernel: members = 21,954, kernel_graph = 27,328, groundedness_fraction (reached_anchor) = 1.0
- recall: kernel_only = 1.0, full = 1.0, **ratio = 1.0**, tau_star_estimate = 958, n_queries_tested = 995
- **A10 recall gate (ratio >= 0.95): PASS**
- wall-clock = 1:32.09, max RSS = 8,108,724 KB

## Findings (honest)

The one-core complaint was valid for the original path. The production kernel-build path is CPU work over Rust graph/storage/index structures; GPU is not wired for graph CF materialization, SCC, DFVS, or the in-memory kernel index. The fix uses all CPU cores for the parallelizable stages and fails loudly where a stage cannot satisfy the measured gate.

---

## 05_degraded_flag_fix.md

# 05 - Degraded-flag fix for temporal sidecars

- **Issue:** #872 (epic #867)   **Date (UTC):** 2026-06-25   **FSV host:** aiwonder
- **Goal:** make `flags.degraded` mean an applicable primary content measurement failed, not that retrieval-only temporal sidecars were absent for text ingest.

## Root-cause analysis

The `biomed-clinical-fast` and default domain panels append E2/E3/E4 temporal controls as active `Structured` slots with `retrieval_only=true`. Text batch ingest correctly emits `Absent(NotApplicable)` for those sidecars, but the degraded flag was computed as:

```
degraded |= vector.is_absent();
```

That made expected temporal absence indistinguishable from a real content-lens failure. The bug existed in both CLI ingest paths and the MCP ingest path.

The durable readback surface also needed tightening. Base CF stores slot hashes, not full slot vectors; `decode_constellation_base` reconstructs placeholder `Absent(NotApplicable)` vectors until the per-slot CFs are read. `readback cx-list` now resolves each slot through its `slot_NN` CF row and decodes the actual `SlotVector`, marking `payload_source` in the JSON.

## What changed

- `Slot::counts_toward_degraded(input_modality)` returns true only for active, input-modality, non-retrieval-only slots.
- CLI single-row and batch measurement use that predicate before OR-ing absence into `flags.degraded`.
- MCP ingest uses the same predicate.
- `readback cx-list` reports decoded slot payloads from slot CFs, plus compact slot summaries.
- Regression coverage verifies:
  - a populated content slot plus absent retrieval-only temporal sidecar stores `degraded=false`;
  - a missing applicable content lens still stores `degraded=true`.

## FSV evidence

FSV root:

```
/home/croyse/calyx/fsv/issue872-degraded-sidecar-20260625T084937Z
```

Patched binary:

```
/home/croyse/calyx/repo/target/debug/calyx
sha256 4e458120632af044f3d119ef3a0ff591ac75006d1b563377d00bbb7189b25cb9
```

Positive case:

- Created a real CLI vault from `text-default`.
- Parked default content slots 0-4.
- Added a registered algorithmic text scalar lens.
- Batch ingested one text row.
- `readback cx-list --vault <vault>` decoded actual slot CF payloads.
- Before rows `0`, after rows `1`.
- `flags.degraded=false`.
- Slot summary: `dense_slots=1`, `absent_slots=8`, `absent_reasons={lens_inactive:5, not_applicable:3}`.
- Payload sources: `{slot_cf:9}`.
- `verify-chain`: `status=ok`, `checked=1`.

Edge case:

- Empty batch ingest wrote no rows.
- Before rows `0`, after rows `0`.
- Ingest stdout bytes `0`.
- `verify-chain`: `status=ok`, `checked=0`.

Negative case:

- Fresh `medical-default` vault with unavailable registry content lenses.
- Before rows `0`, after rows `1`.
- `flags.degraded=true`.
- Slot summary: `absent_slots=6`, `absent_reasons={lens_unavailable:3, not_applicable:3}`.
- Payload sources: `{slot_cf:6}`.
- `verify-chain`: `status=ok`, `checked=1`.

Summary artifact:

```
/home/croyse/calyx/fsv/issue872-degraded-sidecar-20260625T084937Z/issue872_fsv_summary.json
bytes 6433
sha256 bbd0cdf615e63c11b1ecaf000f12ad438cda8f15d80cad0f2cc91e10f42d29b4
```

## Gate checks

On aiwonder:

```
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::ingest::tests::retrieval_only_temporal_absence_does_not_degrade_content_ingest
cargo test -p calyx-cli --test dedup_audit_readback dedup_audit_readback_prints_reversible_undo_bytes
```

All passed before the FSV run.

---

## 06_calibration_fsv.md

# Calibration FSV

## Objective

Issue #873 verifies that planted known signals are recovered before any
biomedical discovery output is trusted.

Known-pair seed: `metformin -> type_2_diabetes`, sourced from MedlinePlus Drug
Information for metformin:
https://medlineplus.gov/druginfo/meds/a696005.html

## FSV Scope

The calibration proof creates real durable Calyx state and then reads source of
truth bytes back:

- Base CF: 100 planted constellations with grounded label anchors.
- Assay CF: `calyx bits` calculation persisted and read back as JSON bytes.
- Kernel CF: anchored kernel report persisted and read back as JSON bytes.
- XTerm CF: Loom agreement xterm persisted through the Aster CF router and
  re-opened.
- Oracle Assay CF: sufficient and insufficient gate evidence persisted as
  scoped `AssayRow` rows, then read by `VaultSufficiencyAssay`.

## Expected Results

- Identical vectors produce agreement `1.0` and agreement weight `1.0`.
- The planted metformin/type-2-diabetes slot recovers `0.5` bits.
- The control/no-signal slot recovers `0.0` bits and fails closed when used
  alone.
- The planted anchored kernel grounds with `recall = 1.0`.
- Oracle sufficient case returns a sufficient bound.
- Oracle insufficient case refuses with `CALYX_ORACLE_INSUFFICIENT`.
- Edge cases preserve source-of-truth row counts after refusal:
  insufficient samples, low signal, and ungrounded kernel.

## Evidence

aiwonder FSV root:

`/home/croyse/calyx/fsv/issue873-calibration-20260625T093450Z`

Readback artifact:

`/home/croyse/calyx/fsv/issue873-calibration-20260625T093450Z/issue873_calibration_fsv_readback.json`

Artifact bytes: `1865`

Artifact SHA256:

`36ba305c870b8ff618f62b41ac2687fb5d1a40e6882b1f06cc5f9b1af93b83c4`

Commands and statuses:

- `cargo fmt --all -- --check`: status `0`, stdout `0` bytes, stderr `0` bytes.
- `git diff --check`: status `0`, stdout `0` bytes, stderr `0` bytes.
- `bash scripts/linecount.sh`: status `0`, stdout `26` bytes, stderr `0` bytes.
- `CALYX_FSV_ROOT=<root> cargo test -p calyx-cli cmd::intelligence::calibration_fsv_tests::planted_calibration_signals_roundtrip_from_durable_state -- --nocapture`: status `0`, stdout `3841` bytes, stderr `3637` bytes.
- `cargo build -p calyx-cli`: status `0`, stdout `0` bytes, stderr `1154` bytes.

Bounded artifact leaves:

- `schema = calyx-medicalsearch-calibration-fsv-v1`
- `known_pair = metformin -> type_2_diabetes`
- `bits.base_rows_after = 100`
- `bits.slot0_bits = 0.5`
- `bits.slot1_bits = 0.0`
- `kernel.recall = 1.0`
- `kernel.kernel_size = 1`
- `loom.persisted_agreement = 1.0`
- `oracle.sufficient.sufficient = true`
- `oracle.insufficient.code = CALYX_ORACLE_INSUFFICIENT`
- `edge_cases.low_signal_code = CALYX_ASSAY_LOW_SIGNAL`
- `edge_cases.insufficient_samples.code = CALYX_ASSAY_INSUFFICIENT_SAMPLES`
- `edge_cases.ungrounded_kernel.code = CALYX_KERNEL_UNGROUNDED`

The FSV root contained `28` files and `368281` bytes after readback.

---

## 07_power_gate_verify.md

# 07 - Assay power-calibration gate verification

- **Issue:** #874 (epic #867)   **Date (UTC):** 2026-06-25   **FSV host:** aiwonder
- **Goal:** verify the Assay MI power-calibration gate is active so an underpowered estimator cannot be treated as a grounded/sufficient verdict, and verify the target-entropy floor fails closed.

## What was run (exact commands)

FSV root:

```
/home/croyse/calyx/fsv/issue874-power-gate-20260625T090958Z
```

Commands run on aiwonder from `/home/croyse/calyx/repo` with `CALYX_FSV_ROOT` set to the root above:

```
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh
cargo test -p calyx-assay --test power_gate_fsv -- --nocapture
cargo test -p calyx-cli assay_bits_validation::power_gate_tests -- --nocapture
cargo build -p calyx-cli
./target/debug/calyx assay bits-validate --corpus-dir <FSV_ROOT>/success_corpus --metrics-dir <FSV_ROOT>/success_metrics --cf-root <FSV_ROOT>/success_cf --target-class 0 --domain issue874_power_success
./target/debug/calyx assay bits-validate --corpus-dir <FSV_ROOT>/entropy_floor_corpus --metrics-dir <FSV_ROOT>/entropy_metrics --cf-root <FSV_ROOT>/entropy_cf --target-class 0 --domain issue874_entropy_floor
```

## Raw evidence / FSV

Code and state:

- aiwonder repo head before FSV: `d04a139189b8641929af7dcff25e52d8e24776d3` plus the uncommitted #874 patch.
- Remote worktree status count before/after FSV: `3` changed files (the #874 patch only).
- `cargo fmt --all -- --check`: status `0`.
- `git diff --check`: status `0`.
- `bash scripts/linecount.sh`: status `0`.
- `cargo test -p calyx-assay --test power_gate_fsv -- --nocapture`: status `0`.
- `cargo test -p calyx-cli assay_bits_validation::power_gate_tests -- --nocapture`: status `0`.
- `cargo build -p calyx-cli`: status `0`.

Direct estimator/sufficiency gate artifact:

```
/home/croyse/calyx/fsv/issue874-power-gate-20260625T090958Z/issue874_power_gate_readback.json
bytes 614
sha256 07dafc5fcbd3c7ccf05fd88fc5a6799ff85f0b694fb29cc8f5f89960371b4cc5
schema calyx-assay-power-gate-fsv-v1
```

Readback fields:

- Entropy-floor labels: `total=200`, `positives=1`, `negatives=199`.
- Entropy-floor error code: `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`.
- Deliberately underpowered case: `n_samples=64`, `n_features=4096`, `recovery_ratio=0.25`.
- Underpowered sufficiency error code: `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Missing-calibration sufficiency error code: `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Passing calibration control: `sufficient=true`.

Real `calyx assay bits-validate` success case:

- Input corpus bytes:
  - `success_corpus/vectors.jsonl`: `102890` bytes, sha256 `d58ff4da3632217c0906fbd5a98c93fbe0275d98116d10a0bce408e2d64e5655`.
  - `success_corpus/manifest.json`: `379` bytes, sha256 `f812dbb6e1a633267feae00392ac1552c5aabad3d86b7ef4a288ce6f292b4b94`.
- Command status: `0`.
- Stdout artifact: `4304` bytes, sha256 `e9b1dd2892ba9cd935d991039823982c2589f0c468de00ba2b3eed68141dc24a`.
- Assay CF rows persisted: `3`.
- Assay CF rows read back after reopen: `3`.
- Anchor entropy: `1.000000` bits.
- Panel power status: `passed`.
- Panel power recovery: `1.000000`.
- Lens power statuses: `real_a:passed`, `real_b:passed`, `redundant:passed`.
- Redundant lens rejection: `redundant:CALYX_ASSAY_REDUNDANT`.
- Abundance artifact:
  - bytes `3393`
  - sha256 `c07b6efb71698b3cd52404d63279e4b2a1419570d27e1dca76e7b7e3768e2d15`
  - readback rows `3`
  - readback panel power status `passed`
- Stderr bytes: `0`.

Real `calyx assay bits-validate` entropy-floor refusal:

- Input corpus bytes:
  - `entropy_floor_corpus/vectors.jsonl`: `17289` bytes, sha256 `1756c1a4235f78f955b98f42f5a5a8f2591176b07ba8a564f872cf402be8ecf4`.
  - `entropy_floor_corpus/manifest.json`: `254` bytes, sha256 `dca21bd1c31a8b85a6485cfa9fc0acd6a70ba97bf06e66dd043546c9de862e25`.
- Command status: `2` (expected refusal).
- Stdout bytes: `0`.
- Stderr bytes: `219`, sha256 `4e5d8d89ffe04cba5b23d8898eebcfdb8b8866b928b3da5e662d1c697232e2fc`.
- Error code present in stderr: `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`.
- Metrics directory exists after refusal: `false`.
- CF directory exists after refusal: `false`.

## Findings (honest)

- The estimator power gate is active at the Assay API boundary: an underpowered `(n=64, dim=4096)` calibration with only `0.25` recovery is rejected by `PowerCalibration::require_passed()` and cannot pass through `panel_sufficiency_from_estimate()`.
- Missing power calibration also fails closed with `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`; sufficiency cannot be claimed from an uncalibrated MI estimate.
- A passing calibration control does produce `sufficient=true`, proving the test is not just rejecting all inputs.
- The real CLI `bits-validate` path rejects a low-entropy target with `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY` before writing metrics or Assay CF state.
- The real CLI `bits-validate` success path persisted and reloaded `3` Assay CF rows, and its metrics artifact separately read back `panel.power_calibration_status=passed`.
- Scope note: for valid non-empty vectors, `bits-validate` plants a strong binary signal in the last feature column, so the deliberately underpowered `(n, dim)` proof is exercised at the public Assay/sufficiency boundary rather than by corrupting a valid corpus into a different error class.

## Conclusion & next step

#874 acceptance is met by aiwonder FSV: the underpowered estimator code path returns `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`, the entropy floor returns `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`, and successful calibrated measurements persist/read back Assay CF rows. This is a gate verification only; it produces no grounded biomedical discovery claim.

---

## 08_blind_spot_sweep.md

# 08 - blind spot sweep

- **Issue:** #875   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic multi-lens observations while #869 corpus ingest runs
- **Goal:** sweep cross-lens disagreement observations, preserve text and neighbor evidence, gate-check each alert, and rank high-severity candidates.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue875-20260625T112409Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo"
scp issue875-20260625T112409Z.tar aiwonder:/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo.tar -C /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/blind_spot_sweep.rs`
- `crates/calyx-lodestar/tests/issue875_blind_spot_sweep_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z`
- Artifact: `/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/issue875_blind_spot_sweep_readback.json`
- Artifact bytes: `1933`
- Artifact SHA256: `e7fd375f8c359e2fdd2ce3e1b142d614d266b907847c7ec785c34528b59c4ff3`
- Readback scalar leaves:
  - `schema_version=1`
  - `observation_count=3`
  - `detected_alert_count=3`
  - `gate_refused_count=1`
  - `severity_filtered_count=1`
  - `candidate_count=1`
  - `top_severity=High`
  - `top_delta=0.9800000190734863`
  - `neighbor_evidence_count=4`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- High-severity, gate-passing disagreements are ranked and keep source text plus both lens neighbor lists.
- Gate-refused high-severity alerts are counted but not returned as candidates.
- Medium-severity alerts are detected and filtered when `min_severity=High`.
- `max_candidates` truncates after deterministic ranking.
- Non-finite similarity and out-of-range gate confidence fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The existing Loom detector remains the primitive for `(cx, lens_a, lens_b)` disagreement.
- Lodestar now has a discovery sweep log that preserves the evidence #875 needs: text, lens slots, two neighbor sets, gate verdict, severity, delta, and rank score.
- The synthetic FSV proves durable readback of one high-severity gate-passing candidate, one gate refusal, and one severity-filtered alert.
- This is not yet the final #875 anchored-corpus acceptance. The real ranked biomedical candidate list requires sweeping the actual anchored corpus after #869/#870/#871.

## Conclusion & next step
The #875 sweep/ranking surface is ready for the real corpus. Keep #875 open until the anchored corpus is fully ingested, Loom cross-terms are woven, the kernel is grounded, and real gate-passing disagreement candidates are read back with source text and neighbors.

---

## 09_domain_bridges.md

# 09 - domain bridges

- **Issue:** #876   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `AssocGraph` bridge members while #869 corpus ingest runs
- **Goal:** rank Swanson-style B-term bridge candidates per domain pair by graph frequency, degree centrality, grounded confidence, and provenance.

## 2026-06-29 real-corpus completion update
Source of truth:
- Real graph/properties: aiwonder vault `corpus-anchored-869-20260625T080546Z`, ULID `01KVYX0KYVBQSGVC6N2S00FX6J`.
- Persisted report: `<vault>/idx/domain_bridges/<blake3(report_bytes)>/report.json`.
- FSV summary artifact: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/issue876_real_fsv_summary.json`.
- Local readback copy: `target/issue876-fsv/issue876_real_fsv_summary.json`.
- Synapse readback key: `calyx-dev/issue876/real-fsv-summary`.

Root cause found:
- The earlier slice only ranked supplied bridge inputs; there was no production CLI that mined domain-pair bridges from a physical Aster graph and persisted a readbackable artifact.
- Metadata scopes were root-only filters, so two source-dataset scopes were disjoint and could not expand to shared graph neighbours.
- The physical graph reader reconstructed topology by scanning edge rows and did not use persisted CSR when available.
- `domain_bridges::max_degree` was effectively `O(V * E)` because it called `AssocGraph::in_degree` for every node. On the real #869 graph (`198,993` nodes, `2,435,817` edges) this stalled before bridge scoring logs.

Research inputs:
- Rediscovering Don Swanson: B-terms are implicit linking information; raw B-term counts are not robust by themselves, and ranking should use statistical and graph/network properties. <https://pmc.ncbi.nlm.nih.gov/articles/PMC5771422/>
- LBD systematic review: term ranking/thresholding is needed to prune noisy associations and rank by significance/interestingness. <https://pmc.ncbi.nlm.nih.gov/articles/PMC7924697/>
- Henry and McInnes indirect-association ranking work: shared linking terms and ranking measures need empirical filtering/evaluation, not unbounded enumeration. <https://bmcbioinformatics.biomedcentral.com/articles/10.1186/s12859-019-2989-9>

Implemented:
- `calyx domain-bridges <vault>` opens the latest physical Aster graph, mines scoped bridge candidates, persists an atomic JSON report, reads it back, byte-compares it, and prints report plus artifact hash/counts.
- `Scope::FilterReachable` starts from real metadata roots and expands through bounded outgoing graph hops, so source-dataset scopes can produce shared bridge candidates without inventing data.
- `PhysicalAsterAssocSnapshot` exposes real topology and decoded node metadata through the Lodestar `AssocStore`.
- Physical graph loading now prefers persisted CSR when present and logs an explicit row-scan path when CSR is missing.
- Degree scoring now uses a single `O(V + E)` degree precompute and computes it lazily only after shared bridge members exist.
- Failure modes are fail-closed: missing roots, no shared bridge members, refused-only candidates, corrupt CSR, invalid params, and artifact overwrite mismatch all return errors with context instead of fallback data.

Local gates:
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests --target-dir target\issue876-lodestar-final3 --jobs 32 -- --nocapture`: 5 passed.
- `cargo test -p calyx-cli cmd::domain_bridges::tests --target-dir target\issue876-cli-final3 --jobs 32 -- --nocapture`: 5 passed.
- `cargo test -p calyx-aster physical_assoc_graph_prefers_persisted_csr_projection --target-dir target\issue876-aster-final3 --jobs 32 -- --nocapture`: passed.
- `cargo test -p calyx-lodestar --test ph34_scope_tests materialize_all_domain_subgraph_time_tenant_filter --target-dir target\issue876-scope-final2 --jobs 32 -- --nocapture`: passed.
- `cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target\issue876-cli-roundtrip2 --jobs 32`: passed.
- `cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-aster --all-targets --target-dir target\issue876-clippy-final --jobs 32 -- -D warnings`: pass.
- Post-split verification:
  - `cargo fmt --all -- --check`: pass.
  - `git diff --check`: pass.
  - `cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-aster --all-targets --target-dir target\issue876-clippy-final2 --jobs 32 -- -D warnings`: pass.
  - `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests --target-dir target\issue876-lodestar-final4 --jobs 32 -- --nocapture`: 5 passed.
  - `cargo test -p calyx-cli cmd::domain_bridges::tests --target-dir target\issue876-cli-final4 --jobs 32 -- --nocapture`: 5 passed.
  - `cargo test -p calyx-aster physical_assoc_graph_prefers_persisted_csr_projection --target-dir target\issue876-aster-final4 --jobs 32 -- --nocapture`: 1 passed.
  - `cargo test -p calyx-lodestar --test ph34_scope_tests materialize_all_domain_subgraph_time_tenant_filter --target-dir target\issue876-scope-final4 --jobs 32 -- --nocapture`: 1 passed.
  - `cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target\issue876-cli-roundtrip4 --jobs 32`: 1 passed.
  - `bash -lc "wc -l ..."` for #876 touched split files: `plain_graph/mod.rs=481`, `aster_bridge.rs=414`, `domain_bridges.rs=401`, `plain_graph/assoc_graph.rs=82`, `aster_bridge/physical.rs=137`, `domain_bridges/mining.rs=149`.
  - `bash scripts/linecount.sh`: still fails on legacy files outside #876; #954 was reopened with the current failing file list.

Real aiwonder FSV:
- Isolated source: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/repo`.
- Built binary: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/target/debug/calyx`.
- FSV summary bytes: `11015`.
- FSV summary SHA256: `d667518cfafe5f051813df61aa6bac99e9fbfdcaf6f7232de50953c13769c200`.
- Real graph readback logs: `nodes=198993`, `edges=2435817`, `node_props rows=198993`.
- Persisted CSR was absent in this older vault, so the logged source of truth was the physical edge-row scan; the new CSR path is covered by the physical CSR readback test.

Manual happy path:
- Trigger: `metadata:source_dataset=pubmedqa` vs `metadata:source_dataset=medxpertqa`, `--scope-radius 1`, `--max-evidence-hops 2`, `--kernel-target-fraction 0.10`.
- Before: output report absent.
- After: output report present, bytes `11165`, SHA256 `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04`.
- Readback state: `schema_version=1`, `input_count=7`, `pair_count=1`, `candidate_count=7`, `refused_count=0`.
- Top candidate readback: `cx_id=0fa503037d5d87b51187abe53d1df67c`, `gate=CALYX_DOMAIN_BRIDGE_GATE_PASS`, `confidence=0.33333334`, `distance=2`, provenance includes real metadata (`source_dataset=medmcqa`, `license=mit`, `download_uri=hf://openlifescienceai/medmcqa`).

Manual edge cases:
- Tight target fraction (`0.02`): before output absent, after output absent, rc `2`, stderr contained `produced no shared bridge members`.
- Missing metadata roots (`metadata:source_dataset=not_real_876`): before output absent, after output absent, rc `2`, stderr contained `has no source-of-truth root nodes`.
- Strict gate (`--min-gate-confidence 0.99`): before output absent, after output absent, rc `2`, stderr contained `had only refused bridge candidates`.

Honest limitation:
- The current real #869 vault contains clinical-QA source datasets (`pubmedqa`, `medxpertqa`, `medqa`, `medmcqa`) only. This closes the production bridge-mining/root-cause work and proves real clinical domain-pair behavior, but it does not prove clinical x molecular/legal/finance bridge acceptance because those non-clinical corpora are not yet materialized in a physical vault. The missing materialized-corpus state is tracked in #994.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue876-20260625T113117Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo"
scp issue876-20260625T113117Z.tar aiwonder:/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo.tar -C /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/domain_bridges.rs`
- `crates/calyx-lodestar/tests/issue876_domain_bridges_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z`
- Artifact: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/issue876_domain_bridges_readback.json`
- Artifact bytes: `3410`
- Artifact SHA256: `929677a08b383594b9b2158dae8a8ecca3b941439e94cc0314ab3327473ffdba`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=4`
  - `pair_count=2`
  - `candidate_count=3`
  - `refused_count=1`
  - `top_cx_id=cd67bd26d28afed81d52aee947746077`
  - `top_rank_score=0.4300000071525574`
  - `top_degree=1`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- B-term candidates are grouped by domain pair.
- Ranking combines graph frequency, graph degree, supplied centrality, and gate confidence.
- Gate-refused bridge members are counted but not returned as candidates.
- `max_per_pair` truncates after deterministic ranking.
- Non-finite centrality fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Bridge IDs absent from the graph fail closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- Lodestar now has a serializable domain-bridge report for B-term candidate mining.
- The report consumes real `bridges(scope_a, scope_b)` output shape indirectly: candidate `CxId`s are validated against the graph, then scored using graph frequency and degree.
- The synthetic FSV proves two domain-pair reports, three ranked candidates, and one gate refusal persisted to disk and were read back.
- This is not yet the final #876 anchored-corpus acceptance. The real bridge candidate list requires running scoped kernels and bridges on the actual anchored association graph after #869/#870/#871.

## Conclusion & next step
The #876 ranking/report surface is ready. Keep #876 open until the real anchored corpus graph exists, scoped kernels can be built, `bridges(scope_a, scope_b)` is run for real domain pairs, and ranked B-term candidates are read back with real sufficiency evidence.

---

## 10_spectral_communities.md

# 10 - spectral communities

- **Issue:** #877   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `AssocGraph` while #869 corpus ingest runs
- **Goal:** expose latent agreement-graph communities through Fiedler bisection and rank inter-community bridge edges plus eigenvector-centrality proposers.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue877-20260625T114455Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo"
scp issue877-20260625T114455Z.tar aiwonder:/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo.tar -C /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/spectral_communities.rs`
- `crates/calyx-lodestar/tests/issue877_spectral_communities_tests.rs`
- `crates/calyx-lodestar/src/error.rs` conversion for `CALYX_SPECTRAL_*` errors
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z`
- Artifact: `/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/issue877_spectral_communities_readback.json`
- Artifact bytes: `4859`
- Artifact SHA256: `2d469e91a7518d9fc3abfb77d7c0e3cf96545ace1abed1325b8af818158e7c90`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z`
- Artifact: `/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z/issue877_spectral_communities_readback.json`
- Artifact bytes: `4859`
- Artifact SHA256: `2d469e91a7518d9fc3abfb77d7c0e3cf96545ace1abed1325b8af818158e7c90`
- Readback scalar leaves:
  - `schema_version=1`
  - `node_count=6`
  - `edge_count=13`
  - `community_count=2`
  - `bridge_candidate_count=1`
  - `centrality_candidate_count=6`
  - `spectral_gap=0.4025992155075073`
  - `top_bridge_src=962248c9b37cc067dad060792ca1e865`
  - `top_bridge_dst=1279c8633841c89a2f8ccb64620effcb`
  - `top_bridge_rank_score=0.9499999284744263`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Planted two-clique graph partitions into two three-member communities through the Fiedler vector.
- Ranked inter-community bridge candidate is the single cross-community association edge.
- Eigenvector-centrality proposer list is emitted independently from bridge-edge ranking.
- `max_bridge_candidates` and `max_centrality_candidates` truncate after deterministic ranking.
- `eigen_k < 2` and zero candidate limits fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- One-node graphs fail closed through `CALYX_SPECTRAL_GRAPH_TOO_SMALL`.

## Findings (honest)
- Lodestar now has a serializable spectral-community report over `AssocGraph`.
- The report reuses `calyx-mincut` Laplacian eigenmaps, Fiedler bisection, spectral gap, and eigenvector centrality; it does not hand-roll spectral math.
- Bridge candidates are graph-structural hypotheses only. They are ranked by edge weight, endpoint centrality, and endpoint frequency, and carry provenance strings.
- Centrality candidates are independent proposers ranked by eigenvector centrality, degree, and frequency.
- This is not yet final #877 anchored-corpus acceptance. The real community partition and inter-community biomedical bridge list require #869 anchored ingest, #870 association graph weaving, and #871 kernel grounding.

## 2026-06-29 real anchored-corpus completion

Issue #877 is now run against the real #869 physical Aster association graph. The 2026-06-25
implementation slice proved the report shape on a planted graph; the 2026-06-29 work removed the
real blocker:

- Root cause: the existing spectral path was an in-memory dense Laplacian path, which is impossible
  for the real graph (`198,993` nodes means roughly 39.6B dense entries before eigen work).
- Production gap: there was no `calyx spectral-communities <vault>` command, no persisted artifact
  under the physical vault, and no separate readback from a source-of-truth file.
- Performance bug found during real FSV: the projected Ritz matrix Jacobi solve used a fixed
  `256`-rotation cap. The real 32-vector Lanczos projection hit `CALYX_SPECTRAL_NOT_CONVERGED`
  before writing an artifact. The fix scales the projected eigensolver budget by matrix size.

Research used:

- Exa: `large sparse graph spectral clustering matrix-free Lanczos Laplacian Fiedler vector best practices`.
- Zhuzhunashvili/Knyazev, "Preconditioned Spectral Clustering..." notes that Lanczos/LOBPCG can be
  matrix-free and only need matrix-vector products, which is the right memory model for large graph
  Laplacians: <https://ar5iv.labs.arxiv.org/html/1708.07481>.
- Dall'Amico/Couillet/Tremblay, "A Unified Framework for Spectral Clustering in Sparse Graphs"
  reiterates the Fiedler-vector basis for two-community Laplacian reconstruction and sparse-graph
  caveats: <https://jmlr.csail.mit.edu/papers/volume22/20-261/20-261.pdf>.
- SciPy/ARPACK docs model the same operational contract: accept a sparse matrix or linear operator,
  use Lanczos-family methods, and fail explicitly on non-convergence:
  <https://docs.scipy.org/doc/scipy/reference/generated/scipy.sparse.linalg.eigsh.html>.

Implementation changes:

- Added `calyx spectral-communities <vault>` with tunable eigen/centrality iteration parameters.
- Source of truth: `<vault>/idx/spectral_communities/<blake3(report_json)>/report.json`.
- Persistence is atomic, refuses overwriting a different explicit output, reads bytes back, decodes
  the report, and emits report byte count + SHA256.
- Replaced dense adjacency/Laplacian allocation with a symmetric sparse graph and matrix-free
  shifted-Laplacian matvec.
- Parallelized sparse row matvecs with Rayon and log `rayon_threads`.
- Removed O(V*E) degree scoring in Lodestar by precomputing degree counts in one edge pass.
- Scaled the projected dense Jacobi budget as `max(256, 16 * n^2)` for the small Ritz matrix.

Exact local verification:

```bash
cargo fmt --all -- --check
cargo test -p calyx-cli cmd::spectral_communities::tests --target-dir target\issue877-cli-spectral3 --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue877_spectral_communities_tests --target-dir target\issue877-lodestar-spectral3 --jobs 32 -- --nocapture
cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-mincut --all-targets --target-dir target\issue877-clippy3 --jobs 32 -- -D warnings
git diff --check
bash scripts/linecount.sh
```

Local results:

- CLI spectral command tests: 5 passed, 0 failed.
- Lodestar issue #877 tests: 4 passed, 0 failed.
- rustfmt check: exit 0.
- clippy: exit 0.
- `git diff --check`: exit 0.
- Touched file line counts: `spectral_communities.rs=287`, CLI tests `195`, mincut spectral `318`,
  mincut linalg `236`, Lodestar spectral report `302`.
- Repo-wide linecount still fails on legacy files tracked by #954, not on #877-touched files.

Real FSV command:

```bash
root=/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547
CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 \
  /usr/bin/time -v "$root/target/release/calyx" spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 3 \
  --eigen-max-iter 64 \
  --centrality-max-iter 512 \
  --centrality-tol 0.00001 \
  --max-bridge-candidates 32 \
  --max-centrality-candidates 32
```

Real FSV source-of-truth readback:

- FSV root: `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547`.
- Source commit: `3dd4a2d42612e5855f83cb8305bcccef2b6dd079`.
- Source of truth:
  `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/spectral_communities/8c043084b902047a931952143821764d55dc25227ffadaad67d744fe5ee58cb2/report.json`.
- Before state: `idx/spectral_communities` was missing.
- After state: one persisted report at the path above.
- Report bytes: `42836741`.
- Report SHA256: `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7`.
- Graph readback: `node_count=198993`, `edge_count=2435817`.
- Report readback: `schema_version=1`, `member_count=198993`, `community_count=2`,
  `bridge_candidate_count=32`, `centrality_candidate_count=32`.
- Communities: community `0` has `33871` members; community `1` has `165122` members.
- Eigenvalues: `[0.122558594, 1.0655823, 2.615509]`.
- Spectral gap: `0.9430237`.
- Top bridge:
  `71a2dcaac4464a1943e5c17ecc5b9c4e -> 5f94d150f749709e0367ffcc4a6b2255`,
  rank `0.8651129`.
- Top centrality proposer: `5f94d150f749709e0367ffcc4a6b2255`, rank `0.9356725`, degree `116`.
- Runtime evidence: elapsed `10.60s`, max RSS `8103272 KB`, CPU `627%`, logged
  `rayon_threads=32`.

Boundary/edge FSV against the real vault:

- Invalid tolerance: `--centrality-tol 0` exited `2` with `CALYX_CLI_USAGE_ERROR`; before/after
  source-of-truth SHA remained
  `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7`.
- Invalid eigen count: `--eigen-k 1` exited `2` with `CALYX_CLI_USAGE_ERROR`; before/after
  source-of-truth SHA remained unchanged.
- Too few Lanczos iterations: `--eigen-max-iter 1` opened the real graph, then exited `2` with
  `CALYX_SPECTRAL_NOT_CONVERGED`; before/after source-of-truth SHA remained unchanged.

Evidence files under the FSV root:

- `happy2_stderr.log` - real run graph load, thread count, persistence path, time/memory.
- `happy2_stdout.json` - CLI JSON output.
- `happy2_readback_summary.json` - independent disk read/decode/hash summary.
- `before_spectral_state_rerun.log` and `after_spectral_state.log` - source-of-truth state.
- `edge_case_summary.json` and `edge_*_before_state.log` / `edge_*_after_state.log` - edge
  source-of-truth verification.

Honest caveats:

- This report is a ranked graph-structural hypothesis surface, not a biomedical verdict.
- The real vault still logged `plain-graph: persisted CSR missing for collection=default, scanning
  graph edge rows`; #877 is complete, but a follow-up issue should materialize the physical CSR so
  large graph readers do not depend on row scans.
- This path is CPU/Rayon today. It does not use Forge CUDA kernels because Calyx does not yet have a
  GPU sparse Laplacian eigensolver backend.

## Conclusion
Issue #877 acceptance is complete: real spectral communities and inter-community bridge candidates
were computed from the physical anchored corpus graph and verified by a separate read of the persisted
vault artifact.

---

## 11_discovery_harness.md

# 11 - discovery harness

- **Issue:** #878   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / 2026-06-29   **Vault/panel:** synthetic slice, then real anchored corpus `corpus-anchored-869-20260625T080546Z`
- **Goal:** turn the discovery chain loop into a real, deterministic Lodestar harness that gates every probed association and persists a traceable chain log.

## 2026-06-29 real anchored-corpus completion

### Research used
- Exa + direct web research confirmed the right shape is bounded beam expansion with traceable paths and explicit pruning/gating:
  - Think-on-Graph describes iterative beam search over knowledge graphs with explicit reasoning-path traceability: <https://arxiv.org/abs/2307.07697>.
  - NetworkX's beam-search docs define beam width as keeping only the best `w` neighbors by an application heuristic: <https://networkx.org/documentation/stable/reference/algorithms/generated/networkx.algorithms.traversal.beamsearch.bfs_beam_edges.html>.
  - OpenTelemetry logging specs reinforce structured, correlated records for downstream inspection: <https://opentelemetry.io/docs/specs/otel/logs/>.

### Root cause fixed
- The previous #878 slice was library-only: it had no physical CLI surface, no vault source-of-truth artifact, and no real anchored-corpus run.
- The chain engine also rebuilt the anchor index for every candidate. That was harmless on toy graphs but wrong for real corpus anchor sets. The engine now builds the anchor ID/index sets once per run.
- The CLI now supports `--anchor-file` so large audited anchor sets can be supplied explicitly instead of relying on shell-length-limited inline arguments.

### Implemented source
- `crates/calyx-cli/src/cmd/discovery_chain.rs`
- `crates/calyx-cli/src/cmd/discovery_chain/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`
- `crates/calyx-lodestar/src/discovery_chain.rs`

### Exact final FSV commands
```bash
# Windows authoring checkout
cargo test -p calyx-cli cmd::discovery_chain::tests --target-dir target\issue878-cli-discovery3 --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests --target-dir target\issue878-lodestar-discovery3 --jobs 32 -- --nocapture
cargo fmt --all -- --check
cargo clippy -p calyx-cli -p calyx-lodestar --all-targets --target-dir target\issue878-clippy3 --jobs 32 -- -D warnings

# aiwonder final archived source
git archive --format=tar -o issue878-real-discovery-fullanchors-20260629-061500.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo"
scp issue878-real-discovery-fullanchors-20260629-061500.tar aiwonder:/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo.tar -C /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo && CARGO_INCREMENTAL=0 cargo build -p calyx-cli --release --jobs 32"

# Anchor file source of truth: every member in the #877 spectral report.
python3 - <<'PY'
import json, pathlib
root=pathlib.Path('/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500')
report_path=pathlib.Path('/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/spectral_communities/8c043084b902047a931952143821764d55dc25227ffadaad67d744fe5ee58cb2/report.json')
report=json.loads(report_path.read_text())
members=[row['cx_id'] for row in report['members']]
(root/'anchors_all_members_from_spectral.txt').write_text('\n'.join(members)+'\n')
PY

CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 /usr/bin/time -v \
  ./target/release/calyx discovery-chain corpus-anchored-869-20260625T080546Z \
  --start 71a2dcaac4464a1943e5c17ecc5b9c4e \
  --start c0fff9e919bfa23e0b7aaea7b6f341fd \
  --start 76cdb0f7234f9e0b25cc8fea8daf2434 \
  --start 2b597f47ce5d9a4101918d06272cb294 \
  --start c1607869690bf88eba8c5d85150aab22 \
  --start 07d28562a4aed115a0bffa444d3e8685 \
  --start 475e94270655af8c265fa7f9fa70595f \
  --start 5f94d150f749709e0367ffcc4a6b2255 \
  --anchor-file /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/anchors_all_members_from_spectral.txt \
  --max-hops 100 --branch-width 16 --probe-width 16 \
  --max-groundedness-distance 1 --min-gate-confidence 0.25 --novelty-weight 0.35
```

### Source-of-truth readback
- Source of truth: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/discovery_chains/4f5dbf9acb8beac9f287837fddaf58338a5c5515368ae3d66ee3b678532b9820/chain.json`
- Artifact bytes: `36554831`
- Artifact SHA256: `376d0649da579a1fe623a21b63cf58338ef6c93ccb284c549f2f6e8e9b2c0885`
- Graph read from artifact: `198993` nodes, `2435817` edges.
- Chain read from artifact: `start_count=8`, `anchor_count=198993`, `candidate_count=25472`, `accepted_hop_count=1600`, `gate_pass_count=21936`, `refused_count=3536`, `max_hop_seen=100`, `termination=max_hops`.
- The last accepted hop is at hop `100`; the final 10 hop buckets each have `16` accepted branches.
- `node_metadata_count=2244`, with real corpus metadata including `source_dataset`, `source_sha256`, `download_uri`, license, and retrieval timestamp.
- Runtime evidence: elapsed `7.51s`, max RSS `8095584 KB`, CPU `442%`, and the command logged `rayon_threads=32`.

### Boundary / edge-case FSV
Source-of-truth root before and after each edge case stayed at `chain_json_count 6`; the happy artifact path above stayed present.

- Edge 1: `--max-hops 0`
  - Exit `2`
  - Error code `CALYX_CLI_USAGE_ERROR`
  - Message: `--max-hops must be >= 1`
- Edge 2: malformed anchor file row `not-a-cxid`
  - Exit `2`
  - Error code `CALYX_CLI_USAGE_ERROR`
  - Message includes the exact anchor file path and line `:1`.
- Edge 3: unknown start `00000000000000000000000000000000`
  - Exit `2`
  - Error code `CALYX_GRAPH_UNKNOWN_NODE`
  - No discovery-chain artifact was created or mutated.

Raw evidence folder:
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stdout.json`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stderr.log`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_readback_summary.json`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/edge_case_summary.json`

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue878-20260625T105843Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo"
scp issue878-20260625T105843Z.tar aiwonder:/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo.tar -C /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/discovery_chain.rs`
- `crates/calyx-lodestar/tests/issue878_discovery_chain_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z`
- Artifact: `/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/issue878_discovery_chain_readback.json`
- Artifact bytes: `4473`
- Artifact SHA256: `338319e9e6563f9d9b9326c13d8dc27644ada26da0b8c8e0ba59854a0542f184`
- Readback scalar leaves:
  - `schema_version=1`
  - `accepted_count=2`
  - `gate_pass_count=2`
  - `refused_count=1`
  - `termination=frontier_exhausted`
  - `refusal_codes=CALYX_DISCOVERY_UNGROUNDED`
  - `accepted_to_count=2`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Strong ungrounded edge is refused with `CALYX_DISCOVERY_UNGROUNDED` and does not enter the selected chain.
- Visited-loop candidate is logged and refused with `CALYX_DISCOVERY_VISITED_LOOP`.
- Branch pruning keeps only the top gate-PASS candidate when `branch_width=1`; the unselected gate-PASS candidate remains in the log.
- `branch_width=0` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Unknown start node fails closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- The harness now exists as a serializable Lodestar engine over `calyx_paths::AssocGraph`.
- Every probed candidate row carries the source branch, edge score, path score, novelty score, groundedness distance, gate verdict, and provenance strings.
- The default grounded gate keeps only candidates with an anchor reachable inside the configured groundedness radius and confidence floor.
- The synthetic FSV proves persisted chain-log bytes for the code path, including one explicit refusal.
- This is not yet the final #878 anchored-corpus acceptance. The real multi-hop biomedical run remains gated on #869 anchored ingest plus #870 association graph weaving and #871 kernel grounding.

## Conclusion
#878 is complete. The harness now has a physical CLI, writes a content-addressed traceable chain log into the real vault, reads that source of truth back, supports large explicit anchor files, and has real anchored-corpus FSV proving the 100-hop gated chain loop reached `termination=max_hops`.

---

## 12_probe_matrix.md

# 12 - probe matrix

- **Issue:** #879   **Phase:** P0 discovery   **Date (UTC):** 2026-06-29   **Vault/panel:** physical Calyx vaults with persisted search indexes
- **Goal:** run a reusable physical probe matrix over fusion x phrasing x length x lens emphasis, persist the complete matrix as a source-of-truth artifact, and log combinations that surface unique grounded hits.

## Root cause
The previous #879 slice only proved the Lodestar in-memory planner. It did not provide a physical CLI harness that opened a real vault, measured real selected lenses, searched the persisted index, and wrote a durable readback artifact.

While wiring the physical harness, three related root causes showed up:

- Weighted-RRF profiles and explicit single-lens slots were not expressible through the physical search API, so matrix rows could not preserve their exact fusion intent.
- Search measurement and persisted-index boundedness checks were global to all active slots. A scoped probe could be blocked by unrelated active slots and sidecars.
- The anchored large corpus has two separate defects outside #879: a corrupted slot-15 frozen registry contract (#1000) and excessive large-corpus matrix materialization/search state (#1001).

## Research used
Best-practice research was done with Exa plus browser search before implementation.

- Cormack, Clarke, and Buettcher, SIGIR 2009, "Reciprocal Rank Fusion outperforms Condorcet and individual rank learning methods." This supports keeping RRF-style rank evidence explainable per contributing run.
- Azure AI Search hybrid/RRF docs:
  - https://learn.microsoft.com/en-us/azure/search/hybrid-search-overview
  - https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking
- Elastic search/retriever docs:
  - https://www.elastic.co/docs/api/doc/elasticsearch/v9/operation/operation-search

Implementation choices taken from that research:

- Preserve exact source provenance for every fusion row: fusion mode, selected RRF profile, selected slot, rank, ledger sequence, ledger hash, and per-lens contribution.
- Fail closed when a selected target is unavailable instead of silently broadening the query.
- Scope selected fields/slots explicitly so a targeted probe matrix does not measure unrelated active lenses.
- Persist a durable matrix artifact and independently read it back instead of treating command success as proof.

## What changed
Implemented a physical `calyx probe-matrix <vault>` command:

- `crates/calyx-cli/src/cmd/probe_matrix.rs`
- `crates/calyx-cli/src/cmd/probe_matrix/parse.rs`
- `crates/calyx-cli/src/cmd/probe_matrix/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`
- `crates/calyx-cli/src/usage.rs`
- `crates/calyx-search/src/engine.rs`
- `crates/calyx-search/src/lib.rs`
- `crates/calyx-search/src/persisted.rs`
- `crates/calyx-search/src/persisted/mixed_tests.rs`

The command now:

- Opens a real Calyx vault and persisted panel/registry.
- Validates requested slots are present, active, and text modality.
- Runs the probe matrix through physical search with explicit slot scoping.
- Preserves `WeightedRrfProfile(profile)` and `SingleLensSlot(slot)` fusion choices.
- Writes `<vault>/idx/probe_matrix/<blake3>/matrix.json` with an atomic write, byte readback, JSON decode, and SHA256 report.
- Fails closed if the matrix has no records, no grounded accepted hits, or no productive rows.

## Local verification
Commands run on the authoring checkout:

```bash
cargo fmt --all -- --check
git diff --check
cargo test -p calyx-search boundedness_check_is_scoped_to_selected_slots --target-dir target/issue879-search --jobs 32 -- --nocapture
cargo test -p calyx-search explicit_fusion_choices_preserve_profile_and_slot --target-dir target/issue879-search --jobs 32 -- --nocapture
cargo test -p calyx-cli cmd::probe_matrix::tests --target-dir target/issue879-cli-probe --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue879_probe_matrix_tests --target-dir target/issue879-lodestar --jobs 32 -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target/issue879-cli-roundtrip --jobs 32 -- --nocapture
```

Focused tests prove:

- Probe-matrix command parsing and token round trips.
- Source-of-truth matrix persistence and readback.
- Missing slots fail before artifact creation.
- Explicit weighted-RRF profiles and single-lens slots preserve exact search intent.
- Boundedness checks are scoped to selected slots and fail closed when the selected sidecar is corrupt.

## Manual full state verification
Source of truth:

```text
/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ/idx/probe_matrix/6c6b38f256484066e9ab3ad7ef5a0c183daf1cbaaf4129acbd38c7dff021f933/matrix.json
```

Manual FSV root:

```text
/home/croyse/calyx/fsv/issue879-manual-vault-20260629T125251Z
```

Physical setup used the real CLI:

```bash
calyx create-vault issue879-fsv-125251 --panel-template text-default
calyx add-lens issue879-fsv-125251 --name issue879_sparse --runtime algorithmic:sparse-keywords:64 --shape 'Sparse(64)'
calyx add-lens issue879-fsv-125251 --name issue879_scalar --runtime algorithmic:scalar --shape 'Dense(1)'
calyx ingest issue879-fsv-125251 --text alpha
calyx ingest issue879-fsv-125251 --text omega
calyx ingest issue879-fsv-125251 --text 'type diabetes alpha pathway'
calyx anchor issue879-fsv-125251 <cx> --kind label:issue879 --value <name> --confidence 1.0 --source issue879-manual-fsv
calyx rebuild-search-index issue879-fsv-125251
```

Selected physical slots:

```json
{
  "issue879_sparse": {"slot_id": 8, "lens_id": "5b813a24c36f4fe0c1fb77ca9836e050"},
  "issue879_scalar": {"slot_id": 9, "lens_id": "dc28852f0b38c142057ba04682c3e90b"}
}
```

Happy-path command:

```bash
RAYON_NUM_THREADS=32 /home/croyse/calyx/target/issue879-probe/debug/calyx probe-matrix issue879-fsv-125251 \
  --frontier alpha \
  --slot 8 --slot 9 \
  --weighted-profile bridge \
  --phrasing terse \
  --length entity \
  --top-k 1
```

Before state:

```json
{"vault":"issue879-fsv-125251","vault_dir":"/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ","artifact_count":0}
```

After state:

```json
{"vault":"issue879-fsv-125251","vault_dir":"/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ","artifact_count":1,"exit":0}
```

Readback from the matrix file itself:

```json
{
  "source_of_truth_bytes": 6435,
  "source_of_truth_sha256": "bc84f978b20d30af2825b95a84e0460adb83f47fa5eddc66f828bebbede2c11d",
  "schema_version": 1,
  "vault": "issue879-fsv-125251",
  "frontier": "alpha",
  "axis_counts": {"slots": 2, "weighted_profiles": 1, "phrasings": 1, "lengths": 1, "records": 6},
  "active_slots": [8, 9],
  "productive_count": 1,
  "accepted_hit_count": 6,
  "refusal_count": 0,
  "productive_rows": [
    {
      "accepted_hit_count": 1,
      "fusion": "single_lens",
      "length": "entity",
      "lens_emphasis": {"kind": "slot", "value": 8},
      "phrasing": "terse",
      "refusal_count": 0,
      "unique_hit_count": 1,
      "variant_id": 3
    }
  ]
}
```

Productive hit read from the artifact:

```json
{
  "cx_id": "efab2164502b7693780f33df2a2dafe8",
  "status": "accepted",
  "grounded": true,
  "score": 0.5908617377281189,
  "provenance": [
    "rank=1",
    "ledger_seq=3",
    "ledger_hash=798da76bbfbb627607d8bd6ebde779080d52b57f886b63f3223c337800661787",
    "provenance_source=Stored",
    "lens:8 rank=1 contribution=0.59086174"
  ]
}
```

## Boundary and edge case audit
Source of truth for edge cases: artifact count under the vault's `idx/probe_matrix`.

```json
{
  "cases": [
    {
      "case": "empty_frontier",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: probe-matrix requires non-empty --frontier <text>"
    },
    {
      "case": "missing_slot",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: --slot 123 is not present in the vault panel"
    },
    {
      "case": "zero_top_k",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: --top-k must be >= 1"
    }
  ]
}
```

All three edge cases failed closed and did not create or modify a matrix artifact.

## Anchored-corpus findings
Attempts against `corpus-anchored-869-20260625T080546Z` exposed two separate P0 defects:

- #1000: slot 15 has a persisted frozen-contract mismatch. The active slot stores lens id `6f47e637a0c287f78972af35c369ce82`, but reconstructing the registry spec yields `51349ee0af01f5257a648c5ac60a9246`.
- #1001: even after scoped slot selection, large-corpus probe-matrix runs can materialize too much provenance/search state. One selected two-slot run reached about 41 GB RSS and ran more than 11 minutes before artifact creation.

Those blockers are not hidden by #879. The probe matrix harness is complete and verified on a real physical vault; #1000 and #1001 track the remaining corpus-specific failures.

## Conclusion
#879 is complete for the physical probe-matrix harness. The command now runs against real vault state, persists a durable matrix artifact, records productive combinations with grounded provenance, and fails closed on invalid or unproductive states. Large-corpus repair and scaling continue under #1000 and #1001.

## Update 2026-07-01 — #1088 in-region guard calibration
Real-vault runs (calyx15000 issue #51) showed `--guard in-region` silently filtering every
retrieved candidate: the in-region guard applied a hardcoded, uncalibrated cosine tau of 0.999
(a near-duplicate threshold), so the benchmark surfaced a generic
`CALYX_PROBE_MATRIX_INCOMPLETE` / "no grounded accepted hits" indistinguishable from a genuinely
empty benchmark. Observed signal: `guard.prefilter.done count=0 filtered=64 tau=0.999000`.

Best-practice basis (matches the Ward conformal-calibration doctrine): cosine thresholds are not
transferable across models/corpora and must be calibrated to the corpus's actual score
distribution — a fixed global tau is a category error for real corpora.

What changed (no fallback masking, never auto-switches to `--guard off`):

- **Calibration path:** `calyx probe-matrix ... --guard in-region --guard-tau <cosine in (0,1]>`
  supplies a corpus-calibrated threshold; it overrides the documented default
  (`calyx_search::DEFAULT_IN_REGION_GUARD_TAU = 0.999`). Out-of-range tau, or `--guard-tau`
  without `--guard in-region`, fails closed at parse and again in the engine.
- **Fail-closed diagnosis:** when in-region filtered every candidate the search path actually
  retrieved across all completed variants, the run exits with
  `CALYX_PROBE_MATRIX_GUARD_FILTERED_ALL`, naming the applied tau (and whether it was the
  uncalibrated default or operator-supplied), the observed in-region best-cosine range, the
  per-variant zero-hit reasons, and the persisted matrix/progress artifact paths. The observed
  cosine range is the calibration evidence: rerun with `--guard-tau` at or below the observed
  best-cosine max.
- Per-variant guard diagnostics (prefilter input/output, guard tau, best-cosine min/max,
  zero-hit reason) were already persisted in `diagnostics.variant_guard_counts`; the new exit
  reads them back instead of discarding them.

Calibration procedure for a real vault: run once with `--guard in-region` (uncalibrated); read
`observed in-region best-cosine range [min, max]` from the `CALYX_PROBE_MATRIX_GUARD_FILTERED_ALL`
error (or `guard_best_cosine_max` in `diagnostics.variant_guard_counts`); rerun with
`--guard-tau` at or below that max. A follow-up unification with the calibrated per-slot Ward
guard profile (`calyx guard calibrate`, Guard CF `profile\0default`, as the MCP search path
already enforces) is tracked separately.

---

## 13_chain_walks_operator-centrality-1.md

# 13 - chain walks operator-centrality-1

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `operator-centrality-1` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `operator_question`
- Operator question: `From the highest-centrality anchored corpus node, what grounded A-B-C chain reaches a traceable testable biomedical hypothesis?`
- Start node: `5f94d150f749709e0367ffcc4a6b2255`
- Termination: `frontier_exhausted`
- Accepted hops: `108`
- Candidate hops inspected: `1696`
- Gate pass count: `793`
- Refused count: `903`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `37`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `5f94d150f749709e0367ffcc4a6b2255` | `0f203f3e-a0ca-4b52-8efb-cf903ed16e31` | `b8100105635e2c4bbe1ad72ada5fdf0683495994c28ad673f8081ce9ad3b3f4a` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_operator-centrality-2.md

# 13 - chain walks operator-centrality-2

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `operator-centrality-2` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `operator_question`
- Operator question: `From the second-highest centrality anchored corpus node, what grounded A-B-C chain should be carried forward for evaluation?`
- Start node: `5194ee0bbcc455a10b1b453735169a83`
- Termination: `frontier_exhausted`
- Accepted hops: `153`
- Candidate hops inspected: `2416`
- Gate pass count: `1146`
- Refused count: `1270`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `52`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `5194ee0bbcc455a10b1b453735169a83` | `dc7fcd8a-76fa-4bc5-8836-2c52e6499bc7` | `4b98807feac1cdceb7d4880da2dc784e6fdfc703a5ddb6d510bf78cc1ad5de1a` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_spectral-bridge-1-src.md

# 13 - chain walks spectral-bridge-1-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-1-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `71a2dcaac4464a1943e5c17ecc5b9c4e`
- Termination: `frontier_exhausted`
- Accepted hops: `165`
- Candidate hops inspected: `2608`
- Gate pass count: `1314`
- Refused count: `1294`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `56`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `71a2dcaac4464a1943e5c17ecc5b9c4e` | `cb85e971-03f7-4a50-84d9-cf0a30ce19b9` | `37a80026910ede7fe790c47b6142f80f476407e013634ee00aaf12ccd04d8f2c` |
| B | `472cc62939d0298289886c288ea30269` | `507e291c-aa23-4aea-91f2-034a3ba6e538` | `abece60e7c8375b901cfac909fd47ca16914af719133dcf897404792b857f95a` |
| C | `d586f81bca5cf4aca40b42d2f7b65091` | `ef069ed5-8608-425a-9865-e6b30047eb45` | `1ecf092e0b207a22ec410d94b009d531302d842f96b5336abcb315fb27f47853` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_spectral-bridge-2-src.md

# 13 - chain walks spectral-bridge-2-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-2-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `c0fff9e919bfa23e0b7aaea7b6f341fd`
- Termination: `frontier_exhausted`
- Accepted hops: `153`
- Candidate hops inspected: `2416`
- Gate pass count: `1130`
- Refused count: `1286`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `52`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `c0fff9e919bfa23e0b7aaea7b6f341fd` | `a849105e-789b-4097-bf54-cfcfa52ebb49` | `a968ada0d6a9c2a9946f928b8a93d7895d9346fe99e0cb0d9152e5dd8baa80d6` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_spectral-bridge-3-src.md

# 13 - chain walks spectral-bridge-3-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-3-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `76cdb0f7234f9e0b25cc8fea8daf2434`
- Termination: `frontier_exhausted`
- Accepted hops: `159`
- Candidate hops inspected: `2512`
- Gate pass count: `1196`
- Refused count: `1316`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `54`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `76cdb0f7234f9e0b25cc8fea8daf2434` | `1ec3ea96-8aa8-4700-95c8-1ac7e39f952a` | `d2d8a786099cd7ba122cdc1a99d2061ae75f53a0520ce153abdc964eea770766` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_spectral-bridge-4-src.md

# 13 - chain walks spectral-bridge-4-src

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** record the real-corpus grounded chain walk for seed `spectral-bridge-4-src` with terminal A-B-C provenance.

## Source artifact
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Graph readback: `198993` nodes, `2435817` edges.

## Chain readback
- Seed kind: `static_candidate`
- Start node: `2b597f47ce5d9a4101918d06272cb294`
- Termination: `frontier_exhausted`
- Accepted hops: `135`
- Candidate hops inspected: `2128`
- Gate pass count: `1038`
- Refused count: `1090`
- Hypotheses emitted for this seed: `8`
- Top rank score: `0.82124996`
- Top confidence: `1.0`
- Top path length: `46`

## Terminal A-B-C
| Role | Node | source_id | source_sha256 |
|---|---|---|---|
| A | `2b597f47ce5d9a4101918d06272cb294` | `1b059bd2-20df-4b23-bdc6-21b5fb27e678` | `16c9f3af486bd951b25052c85f4be31d6032437ad67dddce41deab3c14cd258c` |
| B | `a8cc65ec3a9ae0d9febc02a22c107009` | `205bb8f3-5911-4805-a803-eaf140b3ccb3` | `d2765e2c8fadc1379a5cf6b27ef47be2cacd63d752164f7035d097d858a369a5` |
| C | `ccf62e2fb59b20a8ca50febd517f5c9b` | `e7a73ec9-3a47-48e0-b814-db25b439b4f2` | `a43c12af05fb16ad4b3b5b2dfb37b5563d2d9ea70bac5f9e6ecd492a83c33be1` |

Metadata readback for all terminal roles: `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`.

## Honest conclusion
This seed produced a completed grounded chain and 8 terminal hypotheses. The table records the top-ranked A-B-C terminal from the persisted artifact; all accepted-hop logs and lower-ranked hypotheses remain in the report JSON. This is a traceable hypothesis only, not a biomedical verdict.

---

## 13_chain_walks_synthetic.md

# 13 - chain walks synthetic

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic grounded `AssocGraph` while #869 corpus ingest runs
- **Goal:** run grounded chain walks from static sweep seeds and operator-question seeds, preserving full provenance and extracting terminal A-B-C hypotheses.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue880-20260625T115442Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo"
scp issue880-20260625T115442Z.tar aiwonder:/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo.tar -C /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/chain_walks.rs`
- `crates/calyx-lodestar/tests/issue880_chain_walks_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z`
- Artifact: `/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/issue880_chain_walks_readback.json`
- Artifact bytes: `13017`
- Artifact SHA256: `085e82acf830f7ec13dd850016ec913966926badcf298388eec86a50e515a455`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z`
- Artifact: `/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z/issue880_chain_walks_readback.json`
- Artifact bytes: `13017`
- Artifact SHA256: `085e82acf830f7ec13dd850016ec913966926badcf298388eec86a50e515a455`
- Readback scalar leaves:
  - `schema_version=1`
  - `seed_count=2`
  - `completed_chain_count=2`
  - `hypothesis_count=2`
  - `top_seed_id=static-top`
  - `top_a=b8180e3b18aacaa1d2b6823ac71505c6`
  - `top_b=4e9bfc1971e762585b85541a3b60217e`
  - `top_c=52b6d87820fd8013d5c945d766133424`
  - `top_rank_score=0.746999979019165`
  - `top_cross_domain_distance=2`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Static top-candidate seed and operator-question seed both run through the #878 grounded discovery harness.
- Terminal A-B-C hypotheses are extracted from accepted paths with `A=start`, `B=penultimate`, `C=terminal`.
- Seed provenance and selected-hop gate evidence are carried into hypothesis provenance.
- `max_hypotheses_per_seed` truncates after deterministic ranking.
- Empty seed list, duplicate seed IDs, and operator-question seeds without question text fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Unknown seed start nodes fail closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- Lodestar now has a serializable chain-walk report that runs `run_grounded_discovery_chain` once per seed.
- Seeds distinguish static sweep candidates from operator-supplied questions, preserving rationale and provenance.
- The synthetic FSV proves two completed grounded chains and two terminal A-B-C hypotheses persisted to disk and read back.
- This is not yet final #880 anchored-corpus acceptance. Real chain walks require #869 anchored ingest, #870 association graph weaving, #871 kernel grounding, and real top-candidate/operator seeds.

## Conclusion & next step
The #880 report/orchestration surface is ready for corpus use. Keep #880 open until real chain walks are run on aiwonder against the anchored graph and each real chain artifact is read back under `docs/medicalsearch/13_chain_walks_<seed>.md`.

## 2026-07-02 real corpus addendum

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** run the grounded chain-walk report against the anchored clinical corpus graph, using real static bridge seeds plus operator-question seeds, and persist/read back the full A-B-C hypothesis artifact.

## What was run (exact commands)
```bash
# aiwonder source-of-truth checkout
cargo test -p calyx-cli cmd::chain_walks -- --nocapture
cargo test -p calyx-cli chain_walks_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
$(rustup which rustfmt) --edition 2024 --check \
  crates/calyx-cli/src/cmd/chain_walks.rs \
  crates/calyx-cli/src/cmd/chain_walks/tests.rs
git diff --check -- \
  crates/calyx-cli/src/cmd/chain_walks.rs \
  crates/calyx-cli/src/cmd/chain_walks/tests.rs \
  crates/calyx-cli/src/cmd/mod.rs \
  crates/calyx-cli/src/cmd/tests/token_roundtrip.rs

# real corpus run
root=/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z
CALYX_HOME=/home/croyse/calyx /usr/bin/time -f "elapsed_sec=%e" \
  -o "$root/chain_walks.rerun.time" \
  target/debug/calyx chain-walks corpus-anchored-869-20260625T080546Z \
    --seed-file "$root/chain_walk_seeds.json" \
    --anchor-file "$root/kernel_members.anchors.txt" \
    --max-hops 100 \
    --branch-width 3 \
    --probe-width 16 \
    --max-groundedness-distance 3 \
    --min-gate-confidence 0.25 \
    --novelty-weight 0.35 \
    --max-hypotheses-per-seed 8 \
    --min-terminal-confidence 0.25 \
    --out "$root/real_chain_walks.json" \
    > "$root/chain_walks.rerun.stdout.json" \
    2> "$root/chain_walks.rerun.stderr.log"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-cli/src/cmd/chain_walks.rs`
- `crates/calyx-cli/src/cmd/chain_walks/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`

Focused aiwonder checks:
- `cargo test -p calyx-cli cmd::chain_walks -- --nocapture`: 2 passed, 0 failed.
- `cargo test -p calyx-cli chain_walks_round_trips_through_tokens -- --nocapture`: 1 passed, 0 failed.
- `cargo check -p calyx-cli`: exit 0.
- `rustfmt --edition 2024 --check` on the new chain-walks files: exit 0.
- `git diff --check` on the touched #880 files: exit 0.
- `cargo fmt -p calyx-cli -- --check` was also attempted, but it stops on the unrelated `crates/calyx-cli/src/cmd/probe_matrix/tests/bounded.rs` rustfmt diff tracked outside this slice.

aiwonder real-corpus FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Input vault: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- Seed file: `chain_walk_seeds.json`
- Seed count: `6`
- Seed SHA256: `f0df2880e1925e5eb7c470466932dfe5762d723b6d46d787cfda2a9f4cfe8a96`
- Anchor file: `kernel_members.anchors.txt`
- Anchor count: `21954`
- Anchor SHA256: `1eb62eee08ebe849ee214c4fc731ae59fbc2de1e0702af79d18b40fc91bf7735`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Readback SHA256: `7cc3485e1bb9201f04db2c4ce3a48ea8eb0a1323c878bb78122bdb75c0fdc14b`
- Runtime sidecar: `elapsed_sec=56.63`

Readback scalar leaves:
- `graph.nodes=198993`
- `graph.edges=2435817`
- `graph.node_metadata_count=166`
- `schema_version=1`
- `seed_count=6`
- `completed_chain_count=6`
- `hypothesis_count=48`
- `seed_kind_counts=static_candidate:4, operator_question:2`

Per-seed readback:
| Seed | Kind | Accepted hops | Candidates | Gate pass | Refused | Hypotheses | Top A | Top B | Top C |
|---|---|---:|---:|---:|---:|---:|---|---|---|
| `spectral-bridge-1-src` | static | 165 | 2608 | 1314 | 1294 | 8 | `71a2dcaac4464a1943e5c17ecc5b9c4e` | `472cc62939d0298289886c288ea30269` | `d586f81bca5cf4aca40b42d2f7b65091` |
| `spectral-bridge-2-src` | static | 153 | 2416 | 1130 | 1286 | 8 | `c0fff9e919bfa23e0b7aaea7b6f341fd` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `spectral-bridge-3-src` | static | 159 | 2512 | 1196 | 1316 | 8 | `76cdb0f7234f9e0b25cc8fea8daf2434` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `spectral-bridge-4-src` | static | 135 | 2128 | 1038 | 1090 | 8 | `2b597f47ce5d9a4101918d06272cb294` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `operator-centrality-1` | operator | 108 | 1696 | 793 | 903 | 8 | `5f94d150f749709e0367ffcc4a6b2255` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `operator-centrality-2` | operator | 153 | 2416 | 1146 | 1270 | 8 | `5194ee0bbcc455a10b1b453735169a83` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |

The first attempted command omitted `CALYX_HOME` and failed before writing the report with `CALYX_CLI_USAGE_ERROR` / `CALYX_HOME is required for vault commands`. That stderr was retained as `chain_walks.stderr.log`; the successful rerun above produced the artifact and readback.

## Findings (honest)
- The corpus run completed all 6 real seeds and persisted 48 terminal A-B-C hypotheses.
- The report was read back from disk independently, and the report SHA256 in the readback matches the persisted artifact.
- Terminal node metadata sampled from the report carries stored `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`, `source_id`, and `source_sha256`.
- These are ranked, traceable hypotheses only. They are not biomedical verdicts and still need #881 evaluation over grounded evidence.

## Conclusion & next step
The #880 real-corpus chain-walk artifact is complete for the anchored clinical corpus and is ready to feed #881 hypothesis evaluation. Per-seed readbacks are recorded in:
- `13_chain_walks_spectral-bridge-1-src.md`
- `13_chain_walks_spectral-bridge-2-src.md`
- `13_chain_walks_spectral-bridge-3-src.md`
- `13_chain_walks_spectral-bridge-4-src.md`
- `13_chain_walks_operator-centrality-1.md`
- `13_chain_walks_operator-centrality-2.md`

Repository-wide closeout remains coupled to the unrelated calyx-cli formatting/linecount cleanup being handled separately; the #880 files themselves have focused check, readback, and artifact evidence.

---

## 14_hypothesis_evaluation.md

# 14 - hypothesis evaluation

- **Issue:** #881   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / real FSV 2026-07-02   **Vault/panel:** #880 real anchored-corpus chain hypotheses + GitHub Models evaluator runs
- **Goal:** give each surviving A-B-C hypothesis a transparent evaluator score, justification, falsification test, and cited grounded evidence.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue881-20260625T120221Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo"
scp issue881-20260625T120221Z.tar aiwonder:/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo.tar -C /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/hypothesis_evaluation.rs`
- `crates/calyx-lodestar/tests/issue881_hypothesis_evaluation_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z`
- Artifact: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/issue881_hypothesis_evaluation_readback.json`
- Artifact bytes: `3365`
- Artifact SHA256: `e5a70ad454b238b44222e0bf6a93fb17e23176e67e0365e7ea2e25d90e8ed936`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z`
- Artifact: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z/issue881_hypothesis_evaluation_readback.json`
- Artifact bytes: `3365`
- Artifact SHA256: `e5a70ad454b238b44222e0bf6a93fb17e23176e67e0365e7ea2e25d90e8ed936`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=2`
  - `evaluation_count=2`
  - `retained_count=1`
  - `rejected_count=1`
  - `top_hypothesis_id=h-top`
  - `top_aggregate_score=0.815000057220459`
  - `top_prompt_variant_count=2`
  - `top_temperature_variant_count=2`
  - `top_evidence_count=1`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Multiple prompt IDs and multiple temperature settings are required and counted.
- Plausibility, novelty, testability, and falsifiability dimensions aggregate into a transparent score.
- Retrieved evidence must be cited by evaluator runs; missing citations fail closed.
- Too few evaluator runs and non-finite scores fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Evidence insufficiency is represented explicitly as `needs_more_evidence`.
- `max_ranked` truncates after deterministic score sorting.

## Findings (honest)
- Lodestar now has a serializable hypothesis-evaluation report for externally produced evaluator runs.
- The implementation validates transparent score dimensions, justifications, falsification tests, prompt diversity, temperature diversity, and evidence citations.
- The 2026-06-25 slice did not claim a real LLM evaluated real biomedical hypotheses. It was the report/aggregation surface needed to store and verify those runs once real surviving chains existed.

## 2026-07-02 real evaluator FSV

Real source:
- #880 source artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- #880 source artifact SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- #880 readback SHA256: `7cc3485e1bb9201f04db2c4ce3a48ea8eb0a1323c878bb78122bdb75c0fdc14b`
- Source rows: `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl`

Real evaluator run:
- FSV root: `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z`
- External evaluator: `gh models run openai/gpt-4.1`
- Prompt variants / temperatures: `clinical_plausibility_v1` at `0.2`, `falsification_v1` at `0.8`
- Raw LLM output batches: `12` physical files, summarized in `llm_raw_summary.json`
- Raw LLM summary SHA256: `3fd99ac92d8c70587391de91f4a59e85b4233fe4284bcf0cb3a3fae7cbb3e87b`

Persisted artifacts:
- Source evidence SHA256: `919d39a89027e71189141bd7cf629f5494d5018c3d06fc6639145114e128d341`
- Prompt payload SHA256: `24127a1dffbf8a91a4751c6a1d4c191745344865cb0f7a5f73408283e6b813d2`
- Evaluation input SHA256: `257c893b583078d765567a9eca9a088422f9c4796bb3fd22d5bccf02c8250e1a`
- Evaluation report SHA256: `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b`
- Readback summary SHA256: `5654f7d9780b6fbca5e6a2ad000434d1448d0c78e6a36a824e4bd76534a7f43f`

Command path:
```bash
gh extension install https://github.com/github/gh-models
gh models run openai/gpt-4.1 --temperature 0.2 --max-tokens 6000 < prompt_clinical_<seed>.txt > llm_raw_clinical_<seed>.json
gh models run openai/gpt-4.1 --temperature 0.8 --max-tokens 6000 < prompt_falsification_<seed>.txt > llm_raw_falsification_<seed>.json
cargo run -p calyx-cli -- hypothesis-evaluate \
  --input /home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_input.json \
  --out /home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json
```

Readback leaves from `readback_summary.json`:
- `closed=true`
- `input_count=48`
- `evaluation_count=48`
- `retained_count=44`
- `needs_more_evidence_count=0`
- `rejected_count=4`
- `all_run_count_2=true`
- `all_prompt_variants_2=true`
- `all_temperature_variants_2=true`
- `all_evidence_count_3=true`
- `top_hypothesis_id=spectral-bridge-2-src::01`
- `top_aggregate_score=0.75250006`
- `rejected_ids=[spectral-bridge-3-src::07, spectral-bridge-3-src::08, spectral-bridge-4-src::07, spectral-bridge-4-src::08]`

Honest boundary:
- The evaluator found mostly coherent asthma pharmacology associations grounded in MedMCQA rows, but scored several repeated endpoint rows low enough to reject.
- The output is an evaluator-ranked hypothesis artifact for #882 ranking, not a biomedical verdict and not a treatment recommendation.

## Conclusion & next step
The #881 acceptance criterion is complete: every #880 real-corpus terminal A-B-C hypothesis has cited evidence, two LLM evaluator runs, transparent scores, justifications, falsification tests, and physical readback from aiwonder. The next queue item is #882 ranking over the retained evaluator outputs.

---

## 15_ranked_hypotheses.md

# 15 - ranked hypotheses

- **Issue:** #882   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / real FSV 2026-07-02   **Vault/panel:** #881 retained real evaluator outputs from #880 anchored-corpus hypotheses
- **Goal:** rank surviving A-B-C hypotheses by novelty, grounded confidence, cross-domain distance, evaluator plausibility, sufficiency proof, and provenance.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue882-20260625T120857Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo"
scp issue882-20260625T120857Z.tar aiwonder:/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo.tar -C /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/ranked_hypotheses.rs`
- `crates/calyx-lodestar/tests/issue882_ranked_hypotheses_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z`
- Artifact: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/issue882_ranked_hypotheses_readback.json`
- Artifact bytes: `2817`
- Artifact SHA256: `460fdd90d759a774c750e6c6d021d725b98dbf666b4fbc05fd2fa793c7366124`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z`
- Artifact: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z/issue882_ranked_hypotheses_readback.json`
- Artifact bytes: `2817`
- Artifact SHA256: `460fdd90d759a774c750e6c6d021d725b98dbf666b4fbc05fd2fa793c7366124`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=3`
  - `ranked_count=3`
  - `human_review_count=2`
  - `top_hypothesis_id=h-top`
  - `top_rank=1`
  - `top_rank_score=0.8999999761581421`
  - `top_human_review_flag=True`
  - `top_evidence_count=1`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Rank score combines novelty, grounded confidence, normalized cross-domain distance, and evaluator plausibility.
- Ranked rows retain sufficiency proof, provenance, evidence IDs, and A-B-C nodes.
- Human-review flags apply only after deterministic ranking and score-floor checks.
- `max_ranked` truncates after sorting.
- Empty inputs, zero cross-domain distance, missing sufficiency proof, and non-finite scores fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- Lodestar now has a serializable ranked-hypothesis report for surviving evaluated hypotheses.
- The report can flag top candidates for human review without converting hypotheses into verdicts.
- The 2026-06-25 slice was not a real biomedical ranked list. It was the output/report surface for later real chain/evaluator rows.

## 2026-07-02 real ranked-list FSV

Real source:
- #881 evaluator artifact: `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json`
- #881 evaluator artifact SHA256: `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b`
- #881 readback summary SHA256: `5654f7d9780b6fbca5e6a2ad000434d1448d0c78e6a36a824e4bd76534a7f43f`
- #881 retained rows consumed for ranking: `44`

Implementation added for this real run:
- `calyx hypothesis-rank --input <json> --out <json>`
- The command persists `RankedHypothesisReport` plus source input path, bytes, and SHA256, then separately re-reads the physical report bytes before printing `status=ok`.

Command path:
```bash
cargo run -p calyx-cli -- hypothesis-rank \
  --input /home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_input.json \
  --out /home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json \
  --max-ranked 44 \
  --review-top-n 10 \
  --min-review-score 0.65
```

Persisted artifacts:
- FSV root: `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z`
- Ranking input SHA256: `ba8558d19b4fae0fdc8a7725fbfc42503ca3f78dddcbca6faa15c9ad1268c00a`
- Ranking report SHA256: `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257`
- CLI stdout SHA256: `c30da197be9a6734ad16e58089fa83b31805acdea23e5646949daaffdb1db55e`
- CLI stderr SHA256: `cf39a82fa5cd874bf1def6db7969dcbde3e2ace8786d2a96310e3731ecbba5e0`
- Readback summary SHA256: `229260e9c1c24a7ded5919f992b2d04e8736dbfa337474f7e7e2d2eb978534c3`

Readback leaves:
- `input_count=44`
- `ranked_count=44`
- `human_review_count=10`
- `top_hypothesis_id=operator-centrality-2::01`
- `top_rank_score=0.82295454`
- `top_evaluator_aggregate_score=0.7325`
- `top_cross_domain_distance=51`
- `top_evidence_count=3`

Top 10 ranked traceable hypotheses:

| Rank | Hypothesis | Rank score | Novelty | Grounded | Distance | Plausibility | Eval aggregate | Evidence | Human review |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | `operator-centrality-2::01` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 2 | `operator-centrality-2::02` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 3 | `operator-centrality-2::03` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 4 | `spectral-bridge-2-src::01` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 5 | `spectral-bridge-2-src::02` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 6 | `spectral-bridge-2-src::03` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 7 | `spectral-bridge-2-src::04` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 8 | `spectral-bridge-2-src::05` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 9 | `spectral-bridge-2-src::06` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 10 | `spectral-bridge-2-src::07` | 0.809432 | 0.50 | 1.00 | 49 | 0.825 | 0.73375 | 3 | yes |

Scoring boundary:
- Rank score combines novelty, grounded confidence, normalized cross-domain distance, and evaluator plausibility.
- Repeated or near-duplicate endpoint rows remain separate traceable hypotheses because their A/B/C CxIds and provenance differ; deduplication for presentation would be a separate policy layer, not part of this acceptance criterion.
- Human-review flags are triage markers only. They are not wet-lab validation, biomedical verdicts, treatment recommendations, or claims of clinical utility.

## Conclusion & next step
The #882 acceptance criterion is complete: retained real #881 evaluator outputs were ranked into a persisted, traceable hypothesis list with provenance/evidence IDs, sufficiency proofs, human-review flags, and aiwonder physical readback. The next queue item is #884 molecular-vault commissioning and clinical-to-molecular bridge proof.

---

## 16_refusal_driven_expansion.md

# 16 - refusal driven expansion

- **Issue:** #883   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic before/after probe logs while #869 corpus ingest runs
- **Goal:** convert gate refusals and per-sensor deficits into ranked evidence/lens expansion actions, then verify whether a later run closed the refusal and produced new grounded hits.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue883-20260625T111607Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
scp issue883-20260625T111607Z.tar aiwonder:/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar -C /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/refusal_expansion.rs`
- `crates/calyx-lodestar/tests/issue883_refusal_expansion_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z`
- Artifact: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/issue883_refusal_expansion_readback.json`
- Artifact bytes: `1620`
- Artifact SHA256: `f96e8235771b8d469713692057bc88000182e994ee23be99e8a640b86df21e53`
- Readback scalar leaves:
  - `schema_version=1`
  - `action_count=2`
  - `top_action_kind=AddLens`
  - `before_refusal_count=2`
  - `after_refusal_count=0`
  - `closed_refusal_count=2`
  - `new_grounded_count=1`
  - `closed=True`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Refusals with deficits are turned into ranked actions.
- Lens/sensor deficit text maps to `AddLens`; evidence gaps map to evidence addition.
- Deficit floor filters low-value actions.
- Before/after verification requires both refusal reduction and new grounded hits.
- Refusal reduction without a new grounded hit does not close the expansion.
- Non-finite deficit parameters fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The refusal-expansion planner now turns probe-matrix refusal rows into reusable, ranked expansion actions.
- The verifier produces a serializable before/after closure proof with refusal counts and new grounded hit IDs.
- The synthetic FSV proves the state-machine shape: two refusals planned, later zero refusals, one new grounded hit, `closed=True`.
- This is not yet the final #883 anchored-corpus acceptance. No real biomedical evidence has been added yet; the final issue requires a real refusal on the anchored corpus to become a grounded answer after targeted evidence addition.

## Conclusion & next step
The #883 planning and verification surface is ready. Keep #883 open until #869/#870/#871 produce the real corpus substrate, a real refusal is captured, targeted evidence/lens data is added, and the same verifier reads back a closed refusal with a new grounded answer.

## 2026-07-02 real refusal-driven expansion completion

### Root cause fixed during FSV
- The first real #883 regrounding run proved a probe-layer closure but exposed a false-success state: `calyx anchor` appended anchors while leaving the Base row `flags.ungrounded=true`.
- Fixed `AsterVault::anchor` and `AsterVault::anchor_with_ledger_entry` so appended anchors rewrite the Base row with `flags.ungrounded = anchors.is_empty()`.
- Added coverage in `crates/calyx-aster/tests/issue883_anchor_grounding_flag.rs`; focused aiwonder test readback passed.

### Real source-of-truth run
- FSV root: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z`
- Repo head: `7cb3341a81780f844481320b09e40f46b14a9d9b`
- Vault: `issue883-real-reground-20260702T091116Z`
- Vault dir: `/home/croyse/calyx/vaults/01KWH1HJ3BS09BY86RMFFB8W0R`
- Source data: local TREC-COVID parquet `/zfs/archive/calyx/datasets/trec_covid/corpus.parquet`
- Source parquet SHA256: `d76cea1b2304dbe67a1a54f7376a61de294976682a1d7d58d82de27141f3ba4a`
- Target source row: `ejv2xln0`, title `Surfactant protein-D and pulmonary host defense`
- Target CxId: `0a5307abb08f0e7c64845c93f60d9e74`
- Frontier: `Surfactant protein-D pulmonary host defense collectin SP-D`

### FSV readback
- Summary artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/readback_summary.json`
- Summary SHA256: `d2abcbf2d347af555a4c5ce155d492bf66aff8b851bd5643bf349de95997c8bb`
- Before probe artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/before_probe_matrix.json`
- Before probe SHA256: `a603f1f449725f1169eca2cc31736e27eec8558bee081611d3d8991d9ddb0d8a`
- After probe artifact: `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/after_probe_matrix.json`
- After probe SHA256: `65598ca7acdbf3e17cf06ea1598dc99fcbc4ac50fe115a6fed3f18a17cdd98cc`
- Before readback: `status=refused`, `exit_code=2`, `accepted_hit_count=0`, `refusal_codes=[CALYX_PROBE_UNGROUNDED_HITS]`, target flags `ungrounded=true`, probe provenance `grounding:anchor_count=0 flags_ungrounded=true flags_degraded=false`.
- After readback: `status=ok`, `accepted_hit_count=5`, `refusal_count=0`, target flags `ungrounded=false`, probe provenance `grounding:anchor_count=2 flags_ungrounded=false flags_degraded=false`.
- Chain verification: before anchor `status=ok checked=4`; after anchor `status=ok checked=6`.
- Closure predicate in the summary read back as `closed=true`.

### Conclusion
#883 is complete. A real TREC-COVID biomedical evidence row first produced a persisted ungrounded-hit refusal, then the targeted grounding evidence append turned the same frontier into grounded probe hits with source metadata and clean Base flags.

---

## 17_discovery_vault_molecular.md

# 17 - Discovery vault molecular extension

- **Issue:** #884   **Phase:** P0 discovery / Phase 5 prep   **Date (UTC):** 2026-06-25   **Vault/panel:** not commissioned yet; data preflight only
- **Goal:** Build toward a molecular discovery vault with protein, molecule, DNA, and text views anchored on ChEMBL/BindingDB/Open Targets evidence.

## What was run (exact commands)

aiwonder source tree: `/home/croyse/calyx/repo`
Discovery data root: `/zfs/archive/calyx/biomed-rx/discovery`
FSV root: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z`

```bash
# Read current local discovery files, hashes, Open Targets parquet counts, and row metadata.
/zfs/archive/calyx/datasets/.dataset_tools_venv/bin/python3 - <<'PY'
# bounded Python readback over /zfs/archive/calyx/biomed-rx/discovery;
# wrote summary.json and printed only scalar counts, bytes, and hashes.
PY

# Repair missed single-file Open Targets 26.03 datasets.
wget -c -q --tries=3 --timeout=120 \
  -P /zfs/archive/calyx/biomed-rx/discovery/opentargets/disease \
  https://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.03/output/disease/disease.parquet
wget -c -q --tries=3 --timeout=120 \
  -P /zfs/archive/calyx/biomed-rx/discovery/opentargets/clinical_indication \
  https://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.03/output/clinical_indication/clinical_indication.parquet

# Patch the aiwonder operational downloader regex:
# /zfs/archive/calyx/biomed-rx/scripts/fix_ot2.sh now extracts any href="*.parquet",
# not only part-*-c000.snappy.parquet names.
```

Web/current-source check:

- Exa result: Open Targets Platform docs, "Download datasets", says post-25.03 paths use parquet directories and snake_case singular dataset names.
- Upstream EBI listing confirmed `disease/` and `clinical_indication/` exist in 26.03 and are single-file parquet datasets (`disease.parquet`, `clinical_indication.parquet`), not `part-*` shards.

## Raw evidence / FSV

Primary readback artifact:

- Path: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z/summary.json`
- Bytes: `7772`
- SHA256: `c16348982a462692ab0eb135ab9614ca7c5be27ee71ea5ac1530105110edf960`
- Parquet reader: `/zfs/archive/calyx/datasets/.dataset_tools_venv/bin/python3 + pyarrow`

Key file readback:

- Fresh ChEMBL SQLite tarball: bytes `5764252857`, SHA256 `33c203740555f96067710cdfc1c3c55d890660e5908ec5cbf5817492c290d281`
- Stale ChEMBL SQLite tarball kept separate: bytes `5764989236`, SHA256 `d5dd02de559abf99d1c14d67a1cb7fcb4ba9e5476ab40034ed4d4a199226c55f`
- ChEMBL SDF: bytes `935716795`, SHA256 `f9735be33875fa15999bf9c30f068b3d9545b4e0db737e1387dd1a4e99ca155e`
- BindingDB TSV zip: bytes `590990498`, SHA256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`
- BindingDB target sequences FASTA: bytes `7599053`, SHA256 `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81`
- UniProt SwissProt FASTA gzip: bytes `94330734`, SHA256 `f3bff2df3e3883b737791daf9eec4591e9ef3c3dba31f9750067354b92363463`
- UniProt human proteome FASTA gzip: bytes `7752225`, SHA256 `cf49a88c4812dabbd934cb3e2e00b449e70375816e4d47cda7cc5b77b0754024`

Open Targets 26.03 readback after repair:

- `target`: `10` parquet files, `78691` rows, `85031914` bytes
- `disease`: `1` parquet file, `47030` rows, `7312633` bytes
- `drug_molecule`: `5` parquet files, `22230` rows, `2679269` bytes
- `drug_mechanism_of_action`: `2` parquet files, `6505` rows, `579870` bytes
- `drug_warning`: `1` parquet file, `2302` rows, `250932` bytes
- `clinical_indication`: `1` parquet file, `53950` rows, `3453490` bytes
- `association_overall_direct`: `43` parquet files, `4508002` rows, `633763096` bytes

Downloader script readback:

- Path: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z/fix_ot2_script_readback.json`
- Bytes: `815`
- SHA256: `f7513ca343ce2bf5db3b85aa8fee6ebc66d788247909a9b488d46cec7958a888`
- Script `/zfs/archive/calyx/biomed-rx/scripts/fix_ot2.sh`: bytes `930`, SHA256 `7978985792a65e53cf71fd59ccd9f15fe5ff8adb5f1c1c5aeaa7431fc54052ad`
- Filename extraction check: `disease` matched `1`, `clinical_indication` matched `1`, `target` matched `10`

## Findings (honest)

- The molecular source corpus is now materially present for #884 preflight: ChEMBL, BindingDB, UniProt, and the required Open Targets entity/evidence datasets have source-of-truth bytes and row-count metadata.
- The fresh ChEMBL tarball matches the canonical SHA recorded in issue state; the older tarball is still present but hash-distinct and should not be used as canonical input.
- Real operational bug found and fixed on aiwonder: `fix_ot2.sh` missed Open Targets single-file parquet datasets because it matched only `part-*-c000.snappy.parquet`.
- This is not the #884 acceptance yet. No molecular vault has been commissioned, no ESM2/ChemBERTa/ModernGENA embeddings have been written to a Calyx vault, and no clinical-to-molecular bridge has been demonstrated.
- #869 currently owns the GPU. Per #860, do not run model/vault commands that load panels while that ingest is active.

## Conclusion & next step

The #884 data substrate is ready enough for converter design and later GPU-free-to-GPU transition. Once #869 completes and the GPU is free, build a small molecular vault slice first: ChEMBL/BindingDB molecule-target rows plus Open Targets target/disease/drug links, with anchors for binding/activity/clinical indication, then FSV a clinical-to-molecular bridge before scaling.

## CPU-safe implementation slice: molecular bridge report

Implemented source:
- `crates/calyx-lodestar/src/molecular_bridges.rs`
- `crates/calyx-lodestar/tests/issue884_molecular_bridges_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

What it does:
- Consumes grounded clinical seeds plus ChEMBL/BindingDB/Open Targets-style molecular evidence rows.
- Validates identifiers, affinity/activity scores, target/disease confidence, uppercase protein sequence shape, and provenance.
- Ranks clinical-to-target-to-molecule bridge candidates by binding strength, target confidence, disease confidence, and seed groundedness.
- Emits testable claims and provenance chains for later real molecular-vault rows.

Exact commands:
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue884_molecular_bridges_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue884-20260625T121814Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo"
scp issue884-20260625T121814Z.tar aiwonder:/home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo.tar -C /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z cargo test -p calyx-lodestar --test issue884_molecular_bridges_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue884-molecular-bridges-final-20260625T122100Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue884_molecular_bridges_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

Local test evidence:
- `cargo test -p calyx-lodestar --test issue884_molecular_bridges_tests -- --nocapture`: 6 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z`
- Artifact: `/home/croyse/calyx/fsv/issue884-molecular-bridges-20260625T121814Z/issue884_molecular_bridges_readback.json`
- Artifact bytes: `3254`
- Artifact SHA256: `d89557526526549a7dbe3aa48f67c4964414bfaae5ab3b663a77b9a45c08c2f1`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue884-molecular-bridges-final-20260625T122100Z`
- Artifact: `/home/croyse/calyx/fsv/issue884-molecular-bridges-final-20260625T122100Z/issue884_molecular_bridges_readback.json`
- Artifact bytes: `3254`
- Artifact SHA256: `d89557526526549a7dbe3aa48f67c4964414bfaae5ab3b663a77b9a45c08c2f1`
- Readback scalar leaves:
  - `schema_version=1`
  - `seed_count=2`
  - `evidence_count=4`
  - `candidate_count=3`
  - `top_compound_id=CHEMBL-TOP`
  - `top_target_id=TARG-IL6`
  - `top_disease_id=EFO-DISEASE-1`
  - `top_affinity_nm=8.0`
  - `top_binding_score=0.8096910715103149`
  - `top_rank_score=0.8663918972015381`
- aiwonder tests from archived source: 6 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 6 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Clinical seed disease IDs filter ChEMBL/BindingDB/Open Targets-style evidence rows.
- Target hints constrain candidate target space.
- Affinity-derived binding score beats weaker candidates and can be replaced by activity-only mode when explicitly configured.
- `max_candidates` and score floor truncate after deterministic ranking.
- Empty seed list, zero/non-finite affinity, lowercase protein sequence, and missing required affinity fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

Honest status:
- This was not final #884 acceptance. It proved the bridge-ranking/report surface synthetically while #869 owned the GPU.
- The final #884 vault acceptance was completed later in the section below.

## Final #884 acceptance: real molecular vault

Date (UTC): 2026-07-03
aiwonder branch/commit: `fix/issue884-molecular-vault` / `741b57ae`
FSV root: `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z`
Final vault: `issue884-molecular-vault-20260703t142944z-v2`
Vault id: `01KWM6TG1Q9R80VNR5NEFWE1QY`
Vault dir: `/home/croyse/calyx/vaults/01KWM6TG1Q9R80VNR5NEFWE1QY`

Implemented source:
- `crates/calyx-cli/src/cmd/molecular_vault.rs`
- `crates/calyx-cli/src/cmd/molecular_vault/rows.rs`
- `crates/calyx-cli/src/cmd/molecular_vault/tests.rs`

What was commissioned:
- Saved panel template: `issue884-molecular-template-20260703t142944z`
- Required molecular lenses: `protein-esm2-t30-150m-adapter`, `molecule-chemberta-100m-adapter`, `dna-moderngena-base-adapter`
- Text lenses included to satisfy the real panel floor: `semantic-bge-small-en-v1-5`, `semantic-all-minilm-l6-v2-onnx`, `domain-scibert-scivocab-uncased`, `bge-small-fp32-gpu`, `minilm-fp32-gpu`, `medcpt-query-fp32-gpu`, `medcpt-article-fp32-gpu`, `a38_medcpt_query_int8`, `a38_medcpt_article_int8`

Real source rows:
- Clinical text: PubMedQA row from `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/bridge_rows.jsonl`, SHA256 `c5d02f132a7f286644f1ce3ab2aa4415e2a470a38647257f77de546c21372ad1`
- Molecule: BindingDB row `108468`, SMILES input `CN(C)C(N)=NC(N)=N`, EC50 `150000 nM`, source SHA256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`
- Protein: BindingDB target sequence `>p1234 mol:protein length:766 Dipeptidyl peptidase 4`, source SHA256 `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81`
- DNA: NCBI RefSeq `NM_001935.4 Homo sapiens DPP4 mRNA`, local fetched FASTA SHA256 `024fa67def136d8572230fe1eb301de3b877c56c76a6de6015cad7fdd63fd4b7`

Final commands:
```bash
CALYX_HOME=/home/croyse/calyx \
ORT_DYLIB_PATH=/home/croyse/calyx/vendor/onnxruntime-v1.26.0/build/Linux/Release/libonnxruntime.so \
CALYX_ORT_LIB_DIR=/home/croyse/calyx/vendor/onnxruntime-v1.26.0/build/Linux/Release \
target/release/calyx create-vault issue884-molecular-vault-20260703t142944z-v2 \
  --panel-template issue884-molecular-template-20260703t142944z

target/release/calyx materialize-molecular-vault \
  issue884-molecular-vault-20260703t142944z-v2 \
  --rows /home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/molecular_rows.jsonl \
  --home /home/croyse/calyx

target/release/calyx domain-bridges issue884-molecular-vault-20260703t142944z-v2 \
  --pair metadata:domain=clinical metadata:domain=molecular \
  --anchor-kind label:molecular-vault \
  --scope-radius 2 \
  --max-evidence-hops 2 \
  --kernel-target-fraction 1.0 \
  --out /home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/domain_bridges.report.json

target/release/calyx readback cx-list \
  --vault /home/croyse/calyx/vaults/01KWM6TG1Q9R80VNR5NEFWE1QY \
  --include-slots --allow-unbounded
```

Final persisted readback:
- Rows JSONL SHA256: `37b75c42d888aa1304a0b135e370f8c36291c76e4efbc99fe76c5c875855b3d2`
- `materialize.stdout.json` SHA256: `893a53980c940ca6e3e4d6d5b83f1281c4cc14d305b166dd0cee546f2d362525`
- Base rows: `4`
- Anchor rows: `13`
- Graph nodes/edges: `5` / `8`
- `cx-list` physical readback rows: `4`
- `cx-list` slot payloads decoded: `true`
- Measured slots: `12`; each required molecular/text slot has one measured row.
- Vault tree readback: `137` lines, including `CURRENT`, `MANIFEST`, `cf/base`, `cf/anchors`, and `cf/graph` SST files.

Clinical-to-molecular bridge gate:
- Artifact: `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/domain_bridges.report.json`
- SHA256: `b8d97210108bfc8e2f74ea098d1264ac0e36812b24d28be2f47fbf4a0324f371`
- Candidate: `metformin`
- Candidate count: `1`
- Gate: `CALYX_DOMAIN_BRIDGE_GATE_PASS`
- Confidence: `0.3333333432674408`
- Evidence: left and right scoped kernels both grounded at `1.000000`, candidate one hop from each side, cross-domain distance `2`.

Fail-closed checks:
- Unsupported BindingDB SMARTS-like molecule token failed before write: `materialize.bad_molecule_unsupported_token.stderr`
- Missing DNA modality failed with rc `2`: `molecular vault rows require at least one dna row`
- Bridge term absent from text failed with rc `2`: `molecular vault row 1 text does not contain bridge term notpresent`
- Post-failure physical `cx-list` still read `4` rows, proving the bad inputs did not mutate the final vault.

Final summary artifact:
- `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/fsv_summary.json`
- Artifact hash list: `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/fsv_artifact_hashes.sha256`

Conclusion:
- #884 is accepted: the real Calyx vault contains grounded clinical text, molecule, protein, and DNA rows measured through the live panel, with persisted Base/Anchor/slot/graph readback and a passing clinical-to-molecular bridge candidate.

---

## 18_oracle_event_structuring.md

# 18 - Oracle event/domain structuring

- **Issue:** #885   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic structured QA / algorithmic test panel
- **Goal:** Thread QA rows into Oracle metadata plus Recurrence CF context so `reverse_query` can return grounded causes from a structured corpus.

## What was run (exact commands)

aiwonder source tree: `/home/croyse/calyx/repo`
FSV root: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z`

```bash
cd /home/croyse/calyx/repo

cargo fmt --all -- --check \
  >"$FSV_ROOT/fmt.stdout" 2>"$FSV_ROOT/fmt.stderr"
git diff --cached --check \
  >"$FSV_ROOT/diff_cached.stdout" 2>"$FSV_ROOT/diff_cached.stderr"
bash scripts/linecount.sh \
  >"$FSV_ROOT/linecount.stdout" 2>"$FSV_ROOT/linecount.stderr"

CALYX_FSV_ROOT="$FSV_ROOT" \
  cargo test -p calyx-cli cmd::ingest::oracle_event_tests -- --nocapture \
  >"$FSV_ROOT/oracle_tests.stdout" 2>"$FSV_ROOT/oracle_tests.stderr"

cargo test -p calyx-aster durable_vault_writes_wal_sst_manifest_and_cold_opens -- --nocapture \
  >"$FSV_ROOT/durable_regression.stdout" 2>"$FSV_ROOT/durable_regression.stderr"

cargo build -p calyx-cli \
  >"$FSV_ROOT/cli_build.stdout" 2>"$FSV_ROOT/cli_build.stderr"

target/debug/calyx readback recurrence-series --vault "$vault" --cx-id "$cx" \
  >"$FSV_ROOT/recurrence_readback.stdout" 2>"$FSV_ROOT/recurrence_readback.stderr"
```

## Raw evidence / FSV

Bounded aiwonder readback:

- `fmt_rc=0`, stdout/stderr bytes `0/0`, SHA256 both `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
- `diff_cached_rc=0`, stdout/stderr bytes `0/0`, SHA256 both `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
- `linecount_rc=0`, stdout bytes `26`, SHA256 `2ca9608a7e23755e5f4038d3d0e6ae4482f4acf00be03489434b68af163e79f1`
- `oracle_tests_rc=0`, stdout bytes `4240`, SHA256 `83d12fdcb5d31bca836e1eecf2c5a3529a8d393afc894113d5d1490601403fec`; stderr bytes `4486`, SHA256 `f4346e9f429a17b4921df2990a0dc1a60a714bbf44a914e73911f7488f94d189`
- `durable_regression_rc=0`, stdout bytes `4261`, SHA256 `5723178ddd57f47b24319da5b2db00644fabf3790f20e63f86073bfee6738449`; stderr bytes `3969`, SHA256 `ca6ad21fc9cd94def1c21ce36b29180f35c9d6fd0d09e81eab71e1e2e5369580`
- `cli_build_rc=0`, stderr bytes `1077`, SHA256 `f8f7c33e09285ecc522a4fcd5012f6e62fb6e8fe59aab6e56bd65114c9b77038`

Readback artifact:

- Path: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z/issue885_oracle_event_readback.json`
- Bytes: `859`
- SHA256: `47e86ebea61f2646aa7232b56a8189aca416cee70435378790df08c89e2a994d`
- Preserved vault: `/home/croyse/calyx/fsv/issue885-oracle-event-20260625T103025Z/calyx-cli-ingest-oracle-oracle-event-3249476-1782383453238/vaults/01KVZ5A91PF9QQNYNDP31HHJV1`
- `cx_id=0fbaa052c858412fe27ae5a97d3502ae`

Source-of-truth scalar readback from the persisted vault:

- Base metadata: `oracle.domain=endocrinology`, `oracle.action=What treats type 2 diabetes?`, `oracle.structured=true`
- Recurrence CF: `cf_rows=1`, `occurrences=1`, `first_t_secs=1700000000`
- Oracle reverse query: `cause_count=1`, `first_action_or_event=What treats type 2 diabetes?`, `first_domain=endocrinology`, `first_provisional=false`, `ledger_seq=3`
- CLI recurrence readback: stdout bytes `1126`, SHA256 `d5b941302bd2b8e90a6a7626a5578c901a41f9ea1bbf6514e490352b2d953012`, `frequency=1`, `occurrences_len=1`, `cx_id=0fbaa052c858412fe27ae5a97d3502ae`

## Findings (honest)

- Grounded structured QA ingest works on the tested corpus shape: the batch row writes Oracle Base metadata and one Recurrence CF occurrence, then `calyx_oracle::reverse_query` returns the expected grounded cause.
- Edge checks passed: malformed `oracle.domain` and negative `oracle.t_secs` fail with `CALYX_CLI_USAGE_ERROR`; reingesting the same structured row does not duplicate recurrence occurrences.
- A storage bug was found and fixed while proving the path: durable SST checkpoint writes now collapse duplicate same-CF/same-key pending rows to the latest row, matching memtable/MVCC semantics.
- This does not retrofit the already-running #869 anchored ingest. That run uses the pre-#885 release binary and rows without the new `oracle` JSONL object; a later structured pass is required if the full anchored corpus needs Oracle recurrence rows.

## Conclusion & next step

#885 acceptance is satisfied for a structured corpus: persisted Base metadata plus Recurrence CF rows produce a grounded `reverse_query` cause. The next biomedical discovery issues can build corpus-wide structuring or ranked hypothesis workflows on top of this ingest path after #869 finishes.

---

## 19_nonclinical_bridge_corpora.md

# 19 - non-clinical bridge corpora

- **Issue:** #994   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `issue994-nonclinical-final-20260702t112430z` / bridge-corpus
- **Goal:** materialize a real non-clinical corpus substrate so `calyx domain-bridges` can prove clinical x molecular bridge behavior against persisted source bytes, not only the #869 clinical-QA vault.

## Implemented
- Added `calyx materialize-bridge-corpus <name> --rows <jsonl> [--home <dir>]`.
- The command requires every input row to carry `source_dataset`, `source_path`, and `source_sha256`, rejects empty/duplicate rows, and rejects any declared `bridge_terms` value that does not appear in the row text.
- It writes a durable Aster vault, creates anchored row nodes plus bridge-term nodes, persists CSR, writes `home/vaults/index.json`, then reopens `PhysicalAsterAssocSnapshot::latest` as readback.
- The materialized row nodes use `label:bridge-corpus`, so the existing `calyx domain-bridges` command can mine clinical x molecular roots with `--anchor-kind label:bridge-corpus`.

## Real source corpus
FSV root: `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z`

Source files:
- `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/pubmedqa.anchored.jsonl`
  - bytes `2191493`, rows `1000`
  - sha256 `4e86655c4cf83e7c5c38f81a3bcdc6ed3a538ec17bba43ab61a5665c430ca5ff`
  - selected row `271`, source id `21801416`
- `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl`
  - bytes `207914204`, rows `182822`
  - sha256 `ede2cd900fa48756dbba18b891d24b5c95b7f04011e4fe93a49c63c8e788ffc2`
  - selected row `416`, source id `8b1e7f01-b79f-4f24-a759-3f3fed9c1978`
- `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDB_All_202606_tsv.zip`
  - bytes `590990498`
  - sha256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`
  - entry `BindingDB_All.tsv`, uncompressed bytes `8856851970`, data rows `3182518`
  - selected rows `108468` and `50408024`

Derived proof input:
- `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/bridge_rows.jsonl`
- rows `4`, domain counts `clinical=2`, `molecular=2`, bridge term `metformin`
- bytes `5428`, sha256 `c5d02f132a7f286644f1ce3ab2aa4415e2a470a38647257f77de546c21372ad1`
- source summary bytes `1867`, sha256 `ae4a5d661876edc7cac5578a7be2d6de9b640b93f6134e6d106364428b27566c`

## Full State Verification
Materialize command:
```bash
CALYX_HOME="$FSV/home" "$BIN" materialize-bridge-corpus "$VAULT" \
  --rows "$FSV/bridge_rows.jsonl" \
  --home "$FSV/home" \
  > "$FSV/materialize.stdout.json" 2> "$FSV/materialize.stderr"
```

Materialize readback:
- vault id `01KWH9791WR1E0J6WJBKKV3G7G`
- index entry path `vaults/01KWH9791WR1E0J6WJBKKV3G7G`
- materialize stdout bytes `660`, sha256 `99d3c4a874ae2d529257b52b39a15f8b4d4dcc2f29aefbf00f9c21c070ea7196`
- index artifact bytes `234`, sha256 `f7f6c9992cf365d2742a29e81e9136e78dcd1080329b5a53047efe028eb70c06`
- graph nodes written `5`, edges written `8`, CSR persisted `true`
- snapshot readback: `index_contains_name=true`, `node_count=5`, `edge_count=8`
- stderr confirmed physical CSR readback: `plain-graph: loading persisted CSR collection=default nodes=5 edges=8`

Bridge command:
```bash
CALYX_HOME="$FSV/home" "$BIN" domain-bridges "$VAULT" \
  --pair metadata:domain=clinical metadata:domain=molecular \
  --anchor-kind label:bridge-corpus \
  --scope-radius 1 \
  --max-evidence-hops 2 \
  --kernel-target-fraction 1.0 \
  --max-per-pair 10 \
  --out "$FSV/domain_bridges.report.json" \
  > "$FSV/domain_bridges.stdout.json" 2> "$FSV/domain_bridges.stderr"
```

Bridge report readback:
- report bytes `1591`, sha256 `fecb530aa2d5d1414f86116d57fb1cceaae17c34c4191aadc7fff0c858f9821f`
- stdout bytes `1669`, sha256 `567bfefb2611a66c8acb2076dae48385203696c443974b62f7d28a141560b21f`
- `pair_count=1`, `candidate_count=1`
- candidate text `metformin`
- candidate provenance included `metadata:domain=bridge_term`, `metadata:source_dataset=bridge_terms`, `metadata:source_id=metformin`, `metadata:term=metformin`, `metadata:row_count=4`
- persisted report hash matched the hash printed by the CLI.

Negative cases:
- Missing molecular pair root:
  - command used `--pair metadata:domain=clinical metadata:domain=not_real`
  - rc `2`
  - no `missing_domain.report.json` was created
  - stderr contained `domain scope metadata:domain=not_real has no source-of-truth root nodes`
- Invalid bridge term:
  - command used a row with text containing `metformin` and declared bridge term `absent-term`
  - rc `2`
  - stderr contained `bridge corpus row 1 text does not contain bridge term absent-term`
  - no `issue994-bad-bridge-term` entry was written to the vault index.

Final readback artifact:
- `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/fsv_readback.json`
- Confirms index entry, artifact hashes, report hash match, failure return codes, absent failure artifacts, and vault disk files under `cf/graph`, `cf/time_index`, `codebooks`, `panel`, and `wal`.

## Local Gates
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `bash scripts/linecount.sh`: pass, all `.rs` files <= 500 lines.
- `cargo test -p calyx-cli cmd::bridge_corpus::tests --target-dir target\issue994-cli-bridge-corpus-final -- --nocapture`: 4 passed.
- `cargo test -p calyx-cli cmd::tests::vault_subcommands_round_trip --target-dir target\issue994-cli-roundtrip-final -- --nocapture`: 1 passed.

## Finding
The non-clinical corpus gap from #876 is now materially closed for a molecular proof slice: real clinical rows and real BindingDB molecular rows were hashed, counted, converted into a persisted physical Aster vault, reopened from disk, and mined by `domain-bridges`.

This is still a narrow bridge-corpus materialization, not a full BindingDB-scale vault ingest. The existing `create-vault` template path remains blocked by stale frozen lens contracts in shared templates; that is tracked separately as #1128 and is a registry/template hygiene issue rather than proof that the source corpus is unavailable.

---

## 20_association_evidence_index.md

# 20 - association evidence index

- **Issue:** #867
- **Date (UTC):** 2026-07-03
- **Status:** Current evidence map after #867 was reopened for over-broad closeout.
- **Scope:** Put biomedical association-mining artifacts in one place, state what is actually proven, and define the work needed to turn graph associations into useful healing hypotheses.

## Bottom line

The corpus has real association-mining evidence, but it is not yet a usable biomedical-discovery atlas.

What exists today:

- A full clinical-QA association substrate: `198,993` nodes, about `2,435,817` edges, and `13,133,538` XTerm cross-lens agreement keys.
- Real blind-spot, domain-bridge, spectral-community, discovery-chain, chain-walk, evaluator, ranking, refusal-regrounding, and molecular-bridge artifacts.
- A ranked list of `44` traceable A-B-C hypotheses, `10` flagged for human review.
- A narrow clinical-to-molecular proof slice with `metformin`, DPP4 protein, DPP4 DNA, and BindingDB/NCBI source hashes.

What is missing:

- CxId-to-source-text expansion for every candidate and ranked hypothesis.
- Biomedical concept normalization: disease, drug, gene/protein, pathway, variant, phenotype, assay, trial, and publication IDs.
- Typed association scoring, deduplication, known-positive/known-negative validation, time-split validation, and external evidence triangulation.
- Full molecular and external knowledge-graph ingest at scale.
- Disease-focused deep hunts that produce falsifiable hypotheses, not treatment claims.

No artifact below is a clinical recommendation, cure claim, or proof of efficacy. These are hypothesis-generation outputs that require validation against source text, external biomedical databases, assays, trials, safety data, and expert review.

## Source-of-truth artifacts

| Slice | Issue | Physical evidence | Key readback |
|---|---:|---|---|
| Anchored association graph / XTerms | #870 | `corpus-anchored-869-20260625T080546Z`, vault id `01KVYX0KYVBQSGVC6N2S00FX6J` | `198,993` graph nodes; about `2,435,817` edges; `13,133,538` XTerm keys |
| Kernel grounding | #871 | `/home/croyse/calyx/fsv/issue871-kernel-build-*` and `docs/medicalsearch/04_kernel_build.md` | Grounded kernel build/recall gate for downstream miners |
| Blind-spot sweep | #875 | `/home/croyse/calyx/fsv/issue875-blind-spot-real-low-20260628T035129Z/artifact.readback.summary.json` | `128` candidates; artifact path `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/blind_spot/1782619688096-5dd11782/blind_spot_sweep.json` |
| Domain bridges | #876 | `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/real_pubmedqa_medxpertqa.json` | `7` candidates for `pubmedqa` vs `medxpertqa`; report SHA256 `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04` |
| Spectral communities | #877 | `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547/happy2_readback_summary.json` | `2` communities; `32` bridge candidates; `32` centrality proposers; report SHA256 `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7` |
| Full-anchor discovery chain | #878 | `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_readback_summary.json` | `198,993` anchors; `25,472` candidates; `21,936` gate passes; `1,600` accepted hops; max hop `100` |
| Probe matrix / regrounding surface | #879/#883 | `/home/croyse/calyx/fsv/issue883-real-reground-expansion-20260702T091116Z/readback_summary.json` | Before: `5` refusals, `0` grounded hits. After targeted evidence/lens addition: `5` grounded productive hits |
| Chain walks | #880 | `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json` | `6` completed seeds; `48` terminal A-B-C hypotheses; report SHA256 `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd` |
| Hypothesis evaluation | #881 | `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json` | `48` evaluated; `44` retained; `4` rejected; report SHA256 `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b` |
| Ranked hypotheses | #882 | `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json` | `44` ranked; `10` human-review flags; report SHA256 `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257` |
| Bridge corpus | #994 | `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/fsv_readback.json` | `4` rows; clinical/molecular domains; `metformin` bridge; report SHA256 `fecb530aa2d5d1414f86116d57fb1cceaae17c34c4191aadc7fff0c858f9821f` |
| Molecular vault | #884 | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/fsv_summary.json` | `4` rows; text/molecule/protein/DNA; `13` anchors; `12` measured slots; `metformin` bridge gate pass |
| Association result pack | #1170 | `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z/readback_summary.json` and `docs/medicalsearch/21_association_result_pack.md` | `1,949` machine-readable result rows across #875/#876/#877/#878/#880/#881/#882/#883/#884/#994; `0` missing required source refs |
| CxId source expansion | #1171 | `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/readback_summary.json` and `docs/medicalsearch/22_cxid_source_expansion.md` | `2,612` current association-result CxIds source-row verified; `0` unresolved; `8` molecular row records included; source vault not mutated |
| Biomedical concept normalization | #1172 | `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z/readback_summary.json` and `docs/medicalsearch/23_biomedical_concept_normalization.md` | `3,347` extracted candidate terms accounted for: `79` normalized terms and `3,268` explicit unresolved rows; `2,575` exact-span annotation rows |
| Typed biomedical overlay graph | #1173 | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z/typed_graph_summary.json` and `docs/medicalsearch/24_typed_biomedical_overlay_graph.md` | `7,928` nodes; `116,753` typed edges; `0` untyped/invalid edges; top current clusters are asthma-pharmacology calibration signals |
| Open Targets validation ingest | #1174 | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/readback_summary.json` and `docs/medicalsearch/25_open_targets_validation_ingest.md` | Open Targets `26.06`; `44` concept mappings attempted; `1,422` association rows; `1,420` validation edges; `23` edges map both sides to overlay concepts |
| Molecular vault scaleout | #1175 | `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/fsv_summary.json` and `docs/medicalsearch/26_molecular_vault_scaleout.md` | `53` measured ChEMBL/BindingDB/Open Targets/sequence rows; `26` molecule rows; `4` protein rows; `1` DNA row; `8` clinical-molecular bridge candidates |

## Current useful leads

These are the highest-signal outputs now expanded to source rows by #1171, but not yet normalized to biomedical concepts.

### Ranked A-B-C hypotheses

Top 10 from #882:

| Rank | Hypothesis ID | Rank score | Eval aggregate | Distance | Evidence | Human review |
|---:|---|---:|---:|---:|---:|---|
| 1 | `operator-centrality-2::01` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 2 | `operator-centrality-2::02` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 3 | `operator-centrality-2::03` | `0.82295454` | `0.7325` | `51` | `3` | yes |
| 4 | `spectral-bridge-2-src::01` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 5 | `spectral-bridge-2-src::02` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 6 | `spectral-bridge-2-src::03` | `0.82295454` | `0.75250006` | `51` | `3` | yes |
| 7 | `spectral-bridge-2-src::04` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 8 | `spectral-bridge-2-src::05` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 9 | `spectral-bridge-2-src::06` | `0.819318` | `0.7425` | `50` | `3` | yes |
| 10 | `spectral-bridge-2-src::07` | `0.809432` | `0.73375` | `49` | `3` | yes |

Representative top claim shape:

```text
operator-centrality-2: 5194ee0bbcc455a10b1b453735169a83 -- a8cc65ec3a9ae0d9febc02a22c107009 -- ccf62e2fb59b20a8ca50febd517f5c9b
```

These CxIds are now expanded in #1171. The source rows show this top cluster is a known asthma-pharmacology teaching signal, useful for calibration rather than as a new discovery claim.

### Domain bridges

#876 mined a real clinical-QA domain pair:

```text
left:  metadata:source_dataset=pubmedqa
right: metadata:source_dataset=medxpertqa
```

Top candidates:

| Rank | CxId | Source dataset | Source id | Distance | Gate confidence | Rank score |
|---:|---|---|---|---:|---:|---:|
| 1 | `0fa503037d5d87b51187abe53d1df67c` | `medmcqa` | `e0b24dc3-4133-42db-b44f-03ef01330c5b` | `2` | `0.33333334` | `0.5754386` |
| 2 | `ade680d87cf3b49a689e6381ff90c751` | `medmcqa` | `db914663-dfe1-4bc1-bb19-f54129654797` | `2` | `0.33333334` | `0.5491228` |
| 3 | `b9abb2d04ac805a4d7d3f6a1c5ccec1b` | `medmcqa` | `6486b7cd-4a08-4445-b0f9-f0b69fe1fc56` | `2` | `0.33333334` | `0.5345029` |
| 4 | `9d51f0ecb24078d47082ad964681f0ab` | `medxpertqa` | `Text-2033` | `1` | `0.5` | `0.5302631` |
| 5 | `342e4ce4d217f9b678543c1c95f90a54` | `medmcqa` | `a6d4008e-c9f4-4065-b59e-e4c97a6d3070` | `2` | `0.33333334` | `0.51988304` |

### Spectral communities

#877 partitioned the full graph into two communities:

- Community `0`: `33,871` members.
- Community `1`: `165,122` members.
- Spectral gap: `0.9430237`.
- Top bridge: `71a2dcaac4464a1943e5c17ecc5b9c4e -> 5f94d150f749709e0367ffcc4a6b2255`, rank `0.8651129`.
- Top centrality proposer: `5f94d150f749709e0367ffcc4a6b2255`, rank `0.9356725`, degree `116`.

### Blind spots

#875 readback:

- Observations scanned: `10,064,934`.
- Low alerts: `5,074,777`.
- Positive-delta rows: `5,074,777`.
- No-neighbor-evidence rows: `3,068,604`.
- Returned candidates: `128`.
- First candidate: `51deb3e383fedb2d7cafc1ee54e16c1f`, slots `8` vs `9`, delta `0.18488332629203796`, rank score `0.42941832542419434`, gate confidence `1.0`.

### Molecular bridge

#994/#884 proved the narrow clinical-to-molecular path:

- Candidate: `metformin`.
- BindingDB row `50408024` links metformin (`CHEMBL1431`) to DPP4, with PMID `18068977` and DOI `10.1016/j.bmcl.2007.11.107`.
- BindingDB row `108468`, SMILES `CN(C)C(N)=NC(N)=N`, EC50 `150000 nM`, links metformin to Streptokinase A and should not be treated as the DPP4 binding row.
- DPP4 protein row: BindingDB target FASTA SHA256 `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81`.
- DNA: NCBI RefSeq `NM_001935.4 Homo sapiens DPP4 mRNA`, FASTA SHA256 `024fa67def136d8572230fe1eb301de3b877c56c76a6de6015cad7fdd63fd4b7`.
- BindingDB zip SHA256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`.

This proves the bridge machinery, not a new therapy.

## What should be possible with the data

Research and existing public resources indicate the following are realistic deliverables if the current graph is normalized, typed, and validated:

- Literature-based discovery over A-B-C paths: find implicit A-C hypotheses through shared B terms, but rank/filter aggressively because unbounded B-term enumeration is noisy.
- Drug repurposing by graph path features: use explainable paths between compounds, diseases, targets, pathways, phenotypes, and publications, then score against known treatment edges.
- Target-disease prioritization: triangulate graph candidates against evidence categories and association scores from target-disease platforms.
- Molecular bridge expansion: map clinical terms to drug/target/assay rows from BindingDB, ChEMBL, Open Targets, DGIdb, and related sources.
- Transcriptomic reversal screens: compare disease signatures with perturbation signatures from Connectivity Map/LINCS-style resources, treating negative correlation as a lead, not proof.
- Cancer-specific hypothesis generation: connect cancers, genes, variants, drugs, pathways, trials, and evidence levels using precision-oncology resources, with explicit safety and evidence-level gates.
- Trial and safety triage: for every drug/disease hypothesis, read ClinicalTrials.gov status and safety/adverse-event evidence before ranking it as actionable.
- Falsification-first review: every retained hypothesis needs counter-evidence queries, contradictory literature, trial failures, toxicity flags, and known-mechanism conflicts.

Research anchors:

- Literature-based discovery / A-B-C discovery: Swanson-style association discovery and modern LBD reviews emphasize hypothesis generation, ranking, and validation rather than verdicts.
- Open Targets Platform: supports systematic target identification/prioritization and scores target-disease evidence by source/category.
- Hetionet / Project Rephetio: drug repurposing can be modeled by typed network paths between compounds and diseases.
- Connectivity Map / LINCS: disease/drug expression-signature reversal can produce drug-candidate leads, subject to reproducibility and context limits.
- PubTator3: automated biomedical entity and relation annotation can normalize text to genes, diseases, chemicals, variants, species, and cell lines.
- UMLS/MeSH: concept identifiers and biomedical vocabularies are needed to make CxId outputs comparable across sources.
- ClinicalTrials.gov: trial records can be queried programmatically for intervention/condition/status evidence.
- DGIdb and OncoKB/CIViC-like precision oncology sources: useful for drug-gene and cancer-variant actionability checks, subject to license/API constraints.

## Derived issue tree

The #867 epic should be considered complete only after these atomic tasks are closed with FSV:

1. #1170 - Build a machine-readable association result pack from all current FSV roots. **Done:** `1,949` rows in `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z`.
2. #1171 - Expand every #875/#876/#877/#878/#880/#882 CxId to source text, metadata, and evidence path. **Done for current association-result surfaces:** `2,612/2,612` verified in `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z`.
3. #1172 - Normalize every expanded evidence row to biomedical concepts. **Done as bounded first pass:** `3,347` extracted candidate terms accounted for in `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z`.
4. #1173 - Build a typed biomedical association overlay graph. **Done:** `7,928` nodes and `116,753` typed edges in `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z`.
5. #1174 - Ingest Open Targets target-disease evidence for external validation. **Done as bounded overlay validation:** `1,422` association rows and `1,420` validation edges from Open Targets `26.06`.
6. #1175 - Scale ChEMBL/BindingDB molecular vault beyond the four-row proof slice. **Done:** `53` measured rows in `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z`; runtime fixed for repeated CPU multimodal adapter rows.
7. #1176 - Ingest PubTator/PubMed relation evidence for concept-pair validation.
8. #1177 - Ingest ClinicalTrials.gov intervention-condition evidence for trial-readiness triage.
9. #1178 - Ingest DGIdb/druggable-gene evidence for drug-target triage.
10. #1179 - Add LINCS/CMap transcriptomic reversal screen for drug-repurposing hypotheses.
11. #1180 - Add precision-oncology validation sources for cancer hypotheses.
12. #1181 - Add safety/adverse-event and contraindication triage for drug hypotheses.
13. #1182 - Add known-positive/negative and time-split validation gates for association mining.
14. #1183 - Implement bounded all-pair typed association miner with deduplication.
15. #1184 - Implement counter-evidence and falsification sweep for every retained hypothesis.
16. #1185 - Run oncology deep association hunt over cancer/drug/gene/variant evidence.
17. #1186 - Run metabolic/cardiovascular drug-repurposing association hunt.
18. #1187 - Run neurodegeneration and neuropsychiatric association hunt.
19. #1188 - Run infectious/immunology/inflammation association hunt.
20. #1189 - Run rare-disease phenotype/gene/drug association hunt.
21. #1190 - Implement drug-combination and synergy hypothesis miner.
22. #1191 - Materialize CSR and traversal caches for the #869 graph and large association readers.
23. #1192 - Repair and scale large-corpus probe-matrix association search.
24. #1193 - Publish human-review biomedical hypothesis atlas.
25. #1194 - Add GPU/sparse acceleration path for broad graph association mining where warranted.

## Immediate next executable task

The next task is not another vague mining run. It is:

```text
Ingest PubTator/PubMed relation evidence for concept-pair validation (#1176).
```

The system now has target-disease validation and a measured ChEMBL/BindingDB molecular bridge slice. It still lacks relation-level literature validation, trial/safety triage, counter-evidence, and disease-area deep hunts before any hypothesis can be treated as actionable.

---

## 21_association_result_pack.md

# 21 - Association result pack

- **Issue:** #1170
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for the current association result pack.
- **FSV root:** `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z`

## Bottom line

The current association-mining outputs are now joined into one machine-readable result pack.

- `1,949` result rows.
- `10` source issue surfaces: #875, #876, #877, #878, #880, #881, #882, #883, #884, #994.
- #1171 source expansion joined where applicable: `2,612` source-expanded CxIds available.
- `0` missing required source refs.
- `2` missing non-required refs are the #883 TREC-COVID regrounding target rows, which are outside the #1171 biomedical #869 source-expansion vault and carry their own stored provenance.

This pack is still hypothesis evidence, not biomedical truth or treatment guidance. It is the machine-readable substrate for #1172 concept normalization and downstream validation.

## Persisted Artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `association_result_pack.jsonl` | One row per candidate/hypothesis/bridge/proposer/result surface | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |
| `association_result_pack.json` | JSON object form of the same rows | `25f0d1d7166ff7125b2ca04080d39eb3442a894a259fd35bcad6c09a73107f85` |
| `readback_summary.json` | Input hashes, row counts, issue counts, source-expansion link | `65a6f1b3b0c5d66991c5be760f10a24c9791e645d4273ba956bd8216afe8b5d5` |
| `persisted_readback.json` | Separate readback from persisted JSONL | `319c111346937b8b8b92680eec0a98d40139e9f466a48a32a326edafbffdb1fa` |

## Row Counts

| Row type | Rows |
|---|---:|
| `blind_spot_candidate` | `128` |
| `domain_bridge_candidate` | `7` |
| `spectral_bridge_candidate` | `32` |
| `spectral_centrality_candidate` | `32` |
| `discovery_accepted_hop` | `1,600` |
| `chain_walk_hypothesis` | `48` |
| `hypothesis_evaluation` | `48` |
| `ranked_hypothesis` | `44` |
| `probe_matrix_reground_summary` | `2` |
| `molecular_bridge_row` | `8` |

Issue counts:

| Issue | Rows |
|---:|---:|
| #875 | `128` |
| #876 | `7` |
| #877 | `64` |
| #878 | `1,600` |
| #880 | `48` |
| #881 | `48` |
| #882 | `44` |
| #883 | `2` |
| #884 | `4` |
| #994 | `4` |

## Readback

Separate persisted readback:

```json
{
  "jsonl_row_count": 1949,
  "jsonl_sha256": "50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37",
  "json_sha256": "25f0d1d7166ff7125b2ca04080d39eb3442a894a259fd35bcad6c09a73107f85",
  "missing_required_source_ref_count": 0,
  "missing_nonrequired_source_ref_count": 2
}
```

The pack builder verified every declared input artifact hash before writing outputs. Any missing file or hash mismatch would have aborted the run.

## Source Expansion Join

Rows with biomedical #869 CxIds carry compact source refs from #1171:

- `cx_id`
- `source_dataset`
- `source_id`
- `source_sha256`
- `source_file`
- `source_line`
- `text_sha256`
- `text_len`
- `text_snippet`

The full source text remains in #1171:

```text
/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/complete_cxid_source_expansion.jsonl
```

## Closeout

#1170 acceptance is met:

- JSONL/JSON outputs persist one row per current candidate/hypothesis/bridge/proposer/result surface.
- Each row carries source issue, source artifact path, source artifact SHA256, source CxIds, score fields, gate/verdict fields, provenance or parent evidence, and FSV root.
- Readback summary includes row counts per source issue and hashes.
- The pack builder failed closed on missing/hash-mismatched declared inputs.

Next execution should use `association_result_pack.jsonl` plus #1171 source rows for #1172 concept normalization.

---

## 22_cxid_source_expansion.md

# 22 - CxId source expansion

- **Issue:** #1171
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for current association-result CxId surfaces.
- **Final FSV root:** `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z`
- **Controlled-copy repair root:** `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z`

## Bottom line

The current association-result CxIds are now expanded to physical source rows in one persisted pack.

- `2,612` unique current association-result CxIds targeted.
- `2,612` source-row verified.
- `0` unresolved.
- `8` #994/#884 molecular bridge row records included as row evidence.
- The #870 source vault was not compacted or mutated.

The canonical #870 vault still fails closed on direct CxId reads because of legacy base-CF SST ordering ambiguity. To avoid mutating the source vault, the work created a controlled copy containing top-level manifest files plus `cf/base`, copied required small manifest references, compacted only the copied `base` CF, scanned all `198,993` copied base rows, and mapped each target CxId back to archived source JSONL rows by `source_dataset`, `source_id`, and `source_sha256`.

This is not a cure, treatment recommendation, or efficacy proof. It is the evidence expansion layer needed before concept normalization, typed biomedical scoring, and external validation.

## Persisted Artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `complete_cxid_source_expansion.jsonl` | One verified source-backed record per current association-result CxId | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| `complete_cxid_source_expansion.json` | JSON object form of the same records | `13d186d15e428bc693d9a8d7eb0e8102e24c57fc98b3f153f82d2b4278d8ab07` |
| `target_cxids_by_surface.json` | CxId inventory by association-result surface | `5580f32afc04ba35914335c93a2887e3ccef45b86debe473855133d517396aea` |
| `molecular_rows_expansion.jsonl` | #994/#884 clinical/molecular bridge rows | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| `unresolved_cxids.jsonl` | Empty unresolved set | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `readback_summary.json` | FSV summary and source/hash inventory | `569db92d5cd7c8c092553c63ed696afcdb78dfc76a82bdd6c1ecc9bbd9ff78bc` |
| `persisted_readback.json` | Separate readback from persisted JSONL files | `5e13a468bae109a7c835f91dade6d0d987f27c372d10d2d6c23e5b6b20c446d8` |

## Controlled-Copy Proof

The source vault point read still fails closed:

```text
CALYX_ASTER_SST_ORDER_AMBIGUOUS
```

The controlled-copy repair was limited to the FSV copy:

```text
COMPACTED CF base INPUT_FILES 99498 INPUT_BYTES 640669344 OUTPUT_BYTES 318073064 LOGICAL_BYTES 306405120 WRITE_AMP_MILLI 1038
```

The copied base scan then read all rows:

```json
{
  "base_scan_exit": "0",
  "base_scan_rows": 198993,
  "base_scan_stdout_bytes": 697479783
}
```

Controlled-copy artifacts:

| Artifact | Path | SHA256 |
|---|---|---|
| Base compact stdout | `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z/compact.retry.stdout` | `8956739e18ef82573fcc4856ce77c9ec13574416e8d34d1026dd6e49becb38c8` |
| Base scan JSON | `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z/cx-list-all.stdout.json` | `82824e685e3add8f22dcdfa3dc6b887664c4fcc82cff0606f8c86aeda71b6586` |

## Coverage Readback

| Surface | Target CxIds | Source-row verified |
|---|---:|---:|
| #875 blind-spot candidates | `128` | `128` |
| #875 blind-spot neighbors | `1,944` | `1,944` |
| #876 domain bridges | `7` | `7` |
| #877 spectral bridge candidates | `30` | `30` |
| #877 spectral centrality candidates | `32` | `32` |
| #878 discovery accepted hops | `403` | `403` |
| #878 discovery accepted paths | `403` | `403` |
| #880 chain-walk node metadata | `166` | `166` |
| #882 ranked hypotheses | `16` | `16` |
| #882 ranked hypothesis source refs | `16` | `16` |

Separate persisted readback:

```json
{
  "complete_records_jsonl_readback": 2612,
  "molecular_rows_jsonl_readback": 8,
  "status_counts_jsonl_readback": {
    "source_row_verified": 2612
  },
  "summary_source_row_verified": 2612,
  "summary_unresolved_unique_cxids": 0,
  "unresolved_records_jsonl_readback": 0
}
```

## Input Source Hashes

| Input | Path | SHA256 |
|---|---|---|
| #882 ranked hypotheses | `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json` | `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257` |
| #880 chain walks | `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json` | `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd` |
| #876 domain bridges | `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/real_pubmedqa_medxpertqa.json` | `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04` |
| #877 spectral report | `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547/happy2_stdout.json` | `cf37a095b014865863fdd2d41c21d9a7a371dc94e7b9a6e6aa6e1fc70fb3b5b2` |
| #878 discovery chain | `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stdout.json` | `12faacad51d9108a8eeff28e9d3fa3cb544c6ad8268aaba24a866f3345377990` |
| #875 blind-spot sweep | `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/blind_spot/1782619688096-5dd11782/blind_spot_sweep.json` | `270315cfea2ea101987cb6da2fd96fb202cb5374e8649ff39e92f1efa4ee1801` |
| #994 bridge rows | `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/bridge_rows.jsonl` | `c5d02f132a7f286644f1ce3ab2aa4415e2a470a38647257f77de546c21372ad1` |
| #884 molecular rows | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/molecular_rows.jsonl` | `37b75c42d888aa1304a0b135e370f8c36291c76e4efbc99fe76c5c875855b3d2` |

## Source-Row Files

| Dataset | Path | SHA256 |
|---|---|---|
| `medmcqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl` | `ede2cd900fa48756dbba18b891d24b5c95b7f04011e4fe93a49c63c8e788ffc2` |
| `medqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medqa.anchored.jsonl` | `180330c8deaa086dced6e9d2beec0a39068f842d967e1f06da96b99506ac944c` |
| `medxpertqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medxpertqa.anchored.jsonl` | `880e5a2208c2de97edd6ca2aab979846b9a5a85f91253dbbe5c19e68e051326c` |
| `pubmedqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/pubmedqa.anchored.jsonl` | `4e86655c4cf83e7c5c38f81a3bcdc6ed3a538ec17bba43ab61a5665c430ca5ff` |

## Current Signal

The top #882 cluster expands to known asthma pharmacology exam-source rows: salbutamol/terbutaline for acute attack and salmeterol/formoterol as long-acting beta-2 agonists. That is useful as a known-positive validation and calibration signal, not as a new discovery claim.

The #875 blind-spot candidates and neighbors are now inspectable source rows rather than opaque `input_hash` references. The next useful step is concept normalization (#1172), because raw source rows need drug, disease, gene/protein, pathway, phenotype, and evidence-type IDs before typed discovery scoring can produce biomedical intelligence.

The #994/#884 molecular slice remains narrow but source-backed: clinical metformin rows, BindingDB metformin rows, DPP4 protein, and DPP4 DNA. It proves text/molecule/protein/DNA materialization and bridge mechanics, not therapeutic efficacy.

## Closeout

#1171 acceptance is met for current association-result CxId surfaces:

- Persisted expansion artifact includes CxId, source text, dataset, source ID, source hash, source file, source line, parent surfaces, and parent candidate/hypothesis refs.
- Top #882 hypotheses are expanded from opaque A/B/C CxIds into source text and metadata.
- All targeted CxIds are resolved; unresolved file is empty.
- FSV used separate readback from persisted JSONL and archived source files.
- The source vault was not mutated.

Next execution should move to #1172 concept normalization and #1170 result-pack assembly using this expansion as input.

---

## 23_biomedical_concept_normalization.md

# 23 - Biomedical concept normalization

- **Issue:** #1172
- **Date (UTC):** 2026-07-03
- **Status:** Complete bounded first pass with explicit unresolved accounting.
- **FSV root:** `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z`

## Bottom line

The #1171 expanded evidence rows now have a persisted biomedical concept-normalization layer.

- `2,620` evidence rows processed: `2,612` expanded CxId source rows plus `8` molecular bridge rows.
- `3,347` candidate terms extracted.
- `1,200` terms queried against PubTator3 entity autocomplete.
- `79` unique terms normalized to stable biomedical IDs.
- `3,268` terms explicitly unresolved or ambiguous.
- Accounting is complete for the extracted term set: `79 + 3,268 = 3,347`.
- `2,575` exact-span annotation rows persisted.

This is a candidate normalizer, not a biomedical truth set. It uses exact-span lexical matching and PubTator3 autocomplete as a bounded first pass. Short spans and generic biomedical words can map incorrectly, so downstream typed scoring must run context/ontology QC before using these annotations as assertions.

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `candidate_terms.jsonl` | Extracted candidate terms with occurrence counts and source-span examples | `673c16cc0869360fffd2e6f84f679d8b924a6e5267f7f3c6eb5fea84ae0c9cb5` |
| `normalized_concept_annotations.jsonl` | Exact source-span annotations with normalized ID, source DB, confidence, normalizer URL, and response hash | `ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000` |
| `unresolved_or_ambiguous_concepts.jsonl` | Terms not safely normalized, including unqueried terms from the bounded API pass | `a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c` |
| `pubtator_autocomplete_cache.jsonl` | Persisted PubTator3 autocomplete responses for queried terms | `ab61be27a39faa4cd8e387e3e67ce2706864d08295c79366c1c100ee03c8edec` |
| `validation_samples.json` | Known drug/disease/gene-protein/variant validation examples and persisted annotation samples | `23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b` |
| `readback_summary.json` | Counts, input hashes, accounting status, and unresolved reason counts | `e994fc60e53ccd4c6f07d2f911462bd7b3c0db9f18ae88e5bbfd05421d047edc` |
| `persisted_readback.json` | Separate readback from persisted artifacts | `ac84ea014c24bd909a87ee19dd1f9df732d86c8d060213113df02b74d8f12c5c` |
| `postprocess_manifest.json` | Audit record for adding unresolved rows for unqueried terms | `576956e5ca82f394efacdc69d7afc5fdefa280ae35be03e22aaf58bea96a5827` |

The original unresolved artifact before unqueried-term accounting is retained as:

```text
unresolved_or_ambiguous_concepts.before_unqueried_accounting.jsonl
```

SHA256:

```text
fbedc37d1322b73ba3909b8595b8b82dea1b2dc8fddc332868b578bfb152ede2
```

## Input hashes

| Input | SHA256 |
|---|---|
| #1171 source expansion `complete_cxid_source_expansion.jsonl` | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| #1171 molecular rows `molecular_rows_expansion.jsonl` | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| #1170 result pack `association_result_pack.jsonl` | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |

## Readback

Separate persisted readback:

```json
{
  "annotation_rows_jsonl_readback": 2575,
  "annotation_sha256": "ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000",
  "cache_rows_jsonl_readback": 1200,
  "cache_sha256": "ab61be27a39faa4cd8e387e3e67ce2706864d08295c79366c1c100ee03c8edec",
  "candidate_term_accounting_status": "complete_for_extracted_terms",
  "candidate_terms_accounted_for_by_normalized_or_unresolved": 3347,
  "candidate_terms_jsonl_readback": 3347,
  "candidate_terms_sha256": "673c16cc0869360fffd2e6f84f679d8b924a6e5267f7f3c6eb5fea84ae0c9cb5",
  "unresolved_rows_jsonl_readback": 3268,
  "unresolved_sha256": "a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c",
  "validation_samples_json_readback": true,
  "validation_samples_sha256": "23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b"
}
```

## Annotation counts

| Concept type | Annotation rows |
|---|---:|
| chemical | `1,261` |
| disease | `1,185` |
| gene | `115` |
| variant | `14` |

Unique normalized terms by type:

| Concept type | Terms |
|---|---:|
| chemical | `37` |
| disease | `33` |
| gene | `8` |
| variant | `1` |

## Unresolved accounting

| Reason | Rows |
|---|---:|
| `not_queried_bounded_api_budget` | `2,147` |
| `api_error` | `1,077` |
| `no_exact_or_close_name_match` | `23` |
| `no_results` | `21` |

The high `api_error` count came from transient PubTator3 autocomplete `502` responses during the run. Those terms were not guessed; they were written to the unresolved artifact.

## Validation samples

The validation file contains two kinds of evidence:

1. Persisted normalizer examples with exact spans and source IDs:
   - chemical: `Quinidine`, MeSH `D011802`
   - disease: `Sarcoidosis`, MeSH `D012507`
   - gene: `CD4`, NCBI Gene `920`
   - variant candidate: `p.T1027I`, LitVar `#43740568#p.T1027I`

2. Known source-vocabulary checks for terms that were important to the molecular/clinical bridge but hit PubTator autocomplete limits:
   - chemical: `Metformin`, MeSH `D008687`
   - disease: `Asthma`, MeSH `D001249`
   - gene/protein: `DPP4`, NCBI Gene `1803`, UniProtKB `P27487`
   - variant: `Factor V Leiden`, ClinVar `VCV000000642`, dbSNP `rs6025`

Source URL readback returned HTTP `200` for those external validation URLs at artifact creation time.

## Known limitations

- Exact-span lexical extraction is intentionally broad. It produces useful candidates but also false positives.
- Short spans such as `T10`, generic spans such as `Protein`, and ambiguous abbreviations require context checks before downstream use.
- PubTator3 autocomplete is useful for first-pass normalization but is not enough for final relation evidence or variant resolution.
- The unresolved rows are not dead ends; they are the work queue for improved NER, source-specific ontology lookup, PubTator/PubMed relation ingest, and external KG validation.

## Closeout

#1172 acceptance is met for a bounded first pass:

- Normalized annotations persist exact source spans, source CxIds, source dataset/hash metadata, normalizer URL, normalizer response hash, source vocabulary, source ID, confidence, and scope.
- Unresolved rows are explicit for every extracted term that was not normalized.
- Known drug, disease, gene/protein, and variant samples were validated against source vocabularies with persisted readback.
- The output is grounded in real #1171 expanded evidence rows and #1170 result-pack hashes.

The next useful step is #1173: build a typed biomedical association overlay graph from these normalized and unresolved concept artifacts, while preserving the limitation that first-pass annotations are candidates until context/ontology QC confirms them.

---

## 24_typed_biomedical_overlay_graph.md

# 24 - Typed biomedical overlay graph

- **Issue:** #1173
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for the first typed overlay graph.
- **FSV root:** `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z`

## Bottom line

The current association result pack, source expansion, concept-normalization layer, and molecular bridge rows are now joined into a typed graph.

- `7,928` nodes.
- `116,753` typed edges.
- `0` untyped or invalid edges.
- `1,949` association-result rows consumed.
- `2,612` source-expanded CxIds consumed.
- `2,575` normalized annotation rows consumed.
- `3,268` unresolved concept rows consumed.
- `8` molecular bridge rows consumed.

This overlay makes the current evidence queryable by biomedical concept and provenance. It still does not prove treatment efficacy, causality, novelty, or clinical actionability. `associated_with` edges are co-mention candidates until external relation evidence, counter-evidence, safety, and validation gates confirm or reject them.

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `typed_nodes.jsonl` | CxId, association result, concept, unresolved term, evidence row, and sequence nodes | `749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab` |
| `typed_edges.jsonl` | Typed provenance, mention, support, co-mention, molecular, and sequence edges | `1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85` |
| `typed_graph_summary.json` | Input hashes, counts by node/edge/source type, and scope | `def9fb26ccdb3aae11d2df044fc796813060ca2283881f3f1d268af22d776d3f` |
| `persisted_readback.json` | Separate readback from persisted graph artifacts | `d1d61737dd481d6d7730d889526179ae1cd0204cdd9f7e58a8ceeb27388fc76e` |
| `top_associated_concept_pairs.json` | Top concept co-mention candidates by support count | `f4944fa7b2bdc2013a61ac9f65ea55744118cbbc73a690519a6408dbd637ba04` |
| `untyped_or_invalid_edges.jsonl` | Empty fail-closed invalid-edge artifact | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

## Input hashes

| Input | SHA256 |
|---|---|
| #1170 `association_result_pack.jsonl` | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |
| #1171 `complete_cxid_source_expansion.jsonl` | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| #1171 `molecular_rows_expansion.jsonl` | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| #1172 `normalized_concept_annotations.jsonl` | `ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000` |
| #1172 `unresolved_or_ambiguous_concepts.jsonl` | `a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c` |
| #1172 `validation_samples.json` | `23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b` |

## Readback

Separate persisted readback:

```json
{
  "edge_count_matches_summary": true,
  "node_count_matches_summary": true,
  "typed_edges_jsonl_readback": 116753,
  "typed_edges_sha256": "1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85",
  "typed_nodes_jsonl_readback": 7928,
  "typed_nodes_sha256": "749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab",
  "untyped_or_invalid_edge_count": 0,
  "untyped_or_invalid_edges_jsonl_readback": 0
}
```

## Node counts

| Node type | Count |
|---|---:|
| `association_result` | `1,949` |
| `concept` | `85` |
| `cxid` | `2,616` |
| `evidence_row` | `8` |
| `sequence` | `2` |
| `unresolved_term` | `3,268` |

Concept nodes by type:

| Concept type | Count |
|---|---:|
| chemical | `38` |
| disease | `34` |
| gene | `8` |
| gene_protein | `2` |
| variant | `3` |

## Edge counts

| Edge type | Count | Meaning |
|---|---:|---|
| `uses_cxid` | `85,101` | Association result row links back to original CxIds. |
| `supports` | `23,884` | Association result row supports a normalized concept through its CxIds. |
| `mentions_unresolved` | `4,902` | Source row/example mentions a term that was not safely normalized. |
| `mentions` | `2,575` | Source CxId exact-span mention of a normalized concept. |
| `associated_with` | `267` | Concept co-mention candidate inside verified source rows. |
| `mentions_external_validated` | `15` | External validation sample or molecular row mention. |
| `binds` | `3` | BindingDB molecular binding row edge. |
| `same_as` | `3` | External identifier equivalence from validation or BindingDB aliasing. |
| `has_protein_sequence` | `2` | DPP4 concept to BindingDB target sequence. |
| `has_dna_sequence` | `1` | DPP4 concept to NCBI RefSeq DNA/mRNA sequence. |

Source issue coverage:

| Issue | Edge count |
|---:|---:|
| #1172 | `7,483` |
| #1173 | `282` |
| #875 | `2,428` |
| #876 | `8` |
| #877 | `356` |
| #878 | `105,157` |
| #880 | `354` |
| #881 | `354` |
| #882 | `326` |
| #883 | `2` |
| #884 | `3` |

## Top associated concept candidates

The strongest current co-mention candidates are dominated by asthma-pharmacology teaching clusters, which makes them useful as calibration/known-positive signals rather than novel discoveries.

| Rank | Source concept | Target concept | Support CxIds | Status |
|---:|---|---|---:|---|
| 1 | zafirlukast | montelukast | `28` | co-mention candidate |
| 2 | Ipratropium | Theophylline | `28` | co-mention candidate |
| 3 | Ipratropium | Steroids | `25` | co-mention candidate |
| 4 | Steroids | Theophylline | `25` | co-mention candidate |
| 5 | Tiotropium Bromide | Ipratropium | `18` | co-mention candidate |
| 6 | Prednisolone | Steroids | `18` | co-mention candidate |
| 7 | zileuton | montelukast | `17` | co-mention candidate |
| 8 | montelukast | Steroids | `17` | co-mention candidate |
| 9 | montelukast | Theophylline | `17` | co-mention candidate |
| 10 | Ipratropium | Prednisolone | `17` | co-mention candidate |

Every row in `top_associated_concept_pairs.json` includes source/target concept IDs, names, concept types, support count, and supporting CxIds.

## Molecular bridge edges

The overlay preserves the current molecular bridge evidence without overstating it:

- BindingDB row `50408024` creates a `binds` edge from metformin (`CHEMBL1431`) to DPP4 (`NCBI Gene 1803`), with PMID `18068977` and DOI `10.1016/j.bmcl.2007.11.107`.
- BindingDB row `108468` creates metformin-to-Streptokinase A `binds` edges, but the Streptokinase target remains an unresolved molecular target in this overlay.
- #884 DPP4 protein and DNA rows create `has_protein_sequence` and `has_dna_sequence` edges to BindingDB target sequence `p1234` and NCBI RefSeq `NM_001935.4`.

This corrects the earlier shorthand that treated row `108468` as the DPP4 binding row. The DPP4 binding evidence in this overlay comes from row `50408024`; row `108468` is a separate metformin/Streptokinase A row.

## Failure behavior

The builder failed closed on:

- missing input files,
- input SHA256 mismatches,
- edges without `edge_type`,
- edges without `direction`,
- edges whose endpoints were not persisted nodes.

The invalid-edge artifact is intentionally empty:

```text
untyped_or_invalid_edges.jsonl
```

SHA256:

```text
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```

## Closeout

#1173 acceptance is met:

- Typed nodes and edges are persisted with provenance.
- Links back to original Aster CxIds and association result surfaces are preserved through `uses_cxid` and `supports` edges.
- Edge type, direction, support count, source dataset, source hash, source issue, and extraction method are present.
- Unknown/unresolved material is explicit through `unresolved_term` nodes and `mentions_unresolved` edges.
- Counts by node type, edge type, source issue, and source dataset are persisted and hash-backed.

The next dependency is #1174: ingest Open Targets target-disease evidence so typed graph candidates can be compared against an external target-disease source instead of only internal co-mention/provenance evidence.

---

## 25_open_targets_validation_ingest.md

# 25 - Open Targets validation ingest

- **Issue:** #1174
- **Date (UTC):** 2026-07-03
- **Status:** Complete bounded Open Targets validation ingest.
- **FSV root:** `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z`

## Bottom line

The #1173 typed overlay now has a bounded external Open Targets validation layer for mapped target and disease concepts.

- Open Targets release: `26.06`.
- API endpoint: `https://api.platform.opentargets.org/api/v4/graphql`.
- API version from live metadata: `26.6.0`.
- `44` overlay concept mapping attempts.
- `8` target concept mappings, representing `7` unique Open Targets targets.
- `26` disease concept mappings.
- `10` concepts persisted as unmapped/ambiguous.
- `77` Open Targets API responses persisted with request/response hashes.
- `1,422` association rows persisted.
- `1,420` validation edges persisted.
- `23` validation edges map both sides back to #1173 overlay concepts.
- `0` API errors.

This is validation and triage evidence only. Open Targets association scores are not verdicts, treatment recommendations, or proof that a target should be modulated for a disease.

## Source and release

Official documentation used:

- GraphQL API: `https://platform-docs.opentargets.org/data-access/graphql-api`
- Dataset downloads: `https://platform-docs.opentargets.org/data-access/datasets`

The live Open Targets `meta` query reported:

```json
{
  "apiVersion": {
    "x": "26",
    "y": "6",
    "z": "0"
  },
  "dataPrefix": "platform2606",
  "dataVersion": {
    "year": "26",
    "month": "06",
    "iteration": null
  }
}
```

The full Croissant metadata manifest is stored in:

```text
open_targets_metadata.json
```

SHA256:

```text
caf886b66a5f786bf71a63b9c8d141be4024c2dca872e39fdd1fac2b8089ec64
```

Download locations from the metadata:

| Location | URL |
|---|---|
| FTP | `http://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.06/output/` |
| GCP | `gs://open-targets-data-releases/26.06/output/` |
| AWS | `s3://open-targets-public-data-releases/platform/26.06/output/` |

The metadata download manifest string has SHA256:

```text
4775a7b7db016563f57219c3d96bbeb8cf489b50c91b5df83bcda3fa55be2faf
```

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `open_targets_api_responses.jsonl` | Every GraphQL request/response, including raw response JSON and request/response hashes | `39924964718433cd5f50d44448825a43afb6490e0b985152900b0b9b1897a119` |
| `open_targets_concept_mappings.jsonl` | Overlay concept to Open Targets target/disease mapping attempts | `788c845163e77876e01da635dccbc4d81bdc1d2a4c00ccff7014bfab7f966e77` |
| `open_targets_unmapped_concepts.jsonl` | Concepts not mapped because they had no hit or ambiguous top hit | `2a81eae7e998df2c7b6570fb88939c33f3bac6187a60ad92875f175ff1663af3` |
| `open_targets_association_rows.jsonl` | Top-50 target-associated diseases and disease-associated targets for mapped concepts | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| `open_targets_validation_edges.jsonl` | External validation edges with score, data source scores, data type scores, and overlay concept links where possible | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| `open_targets_metadata.json` | Full Open Targets metadata response, including release/download manifest | `caf886b66a5f786bf71a63b9c8d141be4024c2dca872e39fdd1fac2b8089ec64` |
| `readback_summary.json` | Compact counts, source paths, release, and hashes | `7e0837840e3ce341bfc5e59a03fe6e824f7888cc82d27068bc8545c1e3bc1c6f` |
| `persisted_readback.json` | Separate readback from persisted artifacts | `9fbc71f5dac11342cf262409d15a5088916be921e3c5fac7df89a57e09a8793a` |
| `api_errors.json` | Empty API error list | `37517e5f3dc66819f61f5a7bb8ace1921282415f10551d2defa5c3eb0985b570` |

## Input hashes

| Input | SHA256 |
|---|---|
| #1173 `typed_nodes.jsonl` | `749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab` |
| #1173 `typed_edges.jsonl` | `1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85` |

## Readback

Separate persisted readback:

```json
{
  "api_responses_jsonl_readback": 77,
  "association_rows_jsonl_readback": 1422,
  "concept_mappings_jsonl_readback": 44,
  "counts_match_summary": true,
  "open_targets_metadata_json_readback": true,
  "unmapped_concepts_jsonl_readback": 10,
  "validation_edges_jsonl_readback": 1420
}
```

## Mapping summary

Targets:

| Overlay concept | Open Targets target | Status |
|---|---|---|
| DPP4 / NCBI Gene `1803` | `ENSG00000197635` / DPP4 | mapped |
| DPP4 / UniProtKB `P27487` | `ENSG00000197635` / DPP4 | mapped |
| CD4 / NCBI Gene `920` | `ENSG00000010610` / CD4 | mapped |
| CD8A / NCBI Gene `925` | `ENSG00000153563` / CD8A | mapped |
| TNF / NCBI Gene `24835` | `ENSG00000232810` / TNF | mapped |
| PLA2R1 / NCBI Gene `22925` | `ENSG00000153246` / PLA2R1 | mapped |
| LTC4S / NCBI Gene `4056` | `ENSG00000213316` / LTC4S | mapped |
| NF1 / NCBI Gene `4763` | `ENSG00000196712` / NF1 | mapped |
| Lt1 / NCBI Gene `16991` | none | unmapped |
| SV40gp3 / NCBI Gene `29031016` | none | unmapped |

Disease mappings were conservative exact/token-equivalent matches. Examples include asthma, sarcoidosis, psoriasis, bacterial meningitis, proteinuria, hypertension, schizophrenia, silicosis, and Salmonella infections. Ambiguous or missing disease mappings are persisted in `open_targets_unmapped_concepts.jsonl`.

## Overlay-mapped associations

Top overlay-mapped validation edges:

| Target | Disease | Score | Evidence categories |
|---|---|---:|---|
| TNF | psoriasis | `0.6372348524495187` | literature, animal_model, clinical |
| TNF | sarcoidosis | `0.39272075294445863` | literature, animal_model, clinical |
| DPP4 | asthma | `0.3880634550607382` | literature, genetic_association |
| TNF | asthma | `0.37194382582424823` | literature, clinical |
| DPP4 | Proteinuria | `0.3549124108545795` | literature, clinical |
| DPP4 | Hypertension | `0.2807492875353422` | literature, clinical |
| DPP4 | schizophrenia | `0.19218936351670968` | literature, genetic_association, clinical |
| CD4 | asthma | `0.14213649903074943` | literature, rna_expression, clinical |
| CD4 | sarcoidosis | `0.11693800758175663` | literature |
| CD8A | psoriasis | `0.11503779807105693` | literature, rna_expression |

These are not new discoveries. They are external support/triage rows showing where the internal overlay intersects Open Targets target-disease evidence.

## Scope and limitations

- This is a bounded GraphQL ingest over the #1173 overlay concepts, not a full Open Targets mirror.
- The official docs recommend datasets/downloads or BigQuery for broad systematic pulls; the API was used here because the slice is concept-bounded and every response is persisted with a hash.
- Only top-50 association pages were pulled for each mapped target and disease.
- Conservative mapping intentionally leaves some plausible terms unmapped rather than guessing.
- The outputs are validation/triage features for later scoring, not verdicts.

## Closeout

#1174 acceptance is met:

- Exact Open Targets release, API endpoint, metadata, and download/source locations are persisted and hash-backed.
- Target and disease concepts from the overlay are mapped where possible and unmapped otherwise.
- Association scores, data source scores, and data type scores are persisted.
- Separate readback validates downloaded/API response rows, parsed rows, mappings, unmapped concepts, and validation graph rows.

The next dependency is #1175: scale ChEMBL/BindingDB molecular vault materialization beyond the narrow proof slice.

---

## 26_molecular_vault_scaleout.md

# 26 - Molecular vault scaleout

- **Issue:** #1175
- **Status:** Complete FSV for the first scaled ChEMBL/BindingDB molecular vault slice.
- **FSV root:** `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z`
- **Final vault:** `issue1175-molecular-scaleout-20260703t161602z-v8-fixed-full`
- **Vault id:** `01KWME1BXHQKZ0R1WQ7J4D51P0`
- **Vault dir:** `/home/croyse/calyx/vaults/01KWME1BXHQKZ0R1WQ7J4D51P0`

This is discovery and triage evidence only. It is not a cure, treatment recommendation, efficacy proof, causality proof, or clinical actionability claim.

## What changed

The #884 molecular bridge was a four-row proof slice. #1175 scales that into a measured molecular vault slice with ChEMBL, BindingDB, Open Targets, ChEMBL indication, protein sequence, and DNA evidence.

The scaleout exposed a real runtime blocker: CPU multimodal adapter helpers are one-shot framed commands. Reusing the same child process worked for #884 because each multimodal lens saw one row, but it failed on repeated molecule rows with `multimodal response header read failed`. The fix respawns CPU multimodal helpers per request while leaving GPU mux workers shared.

Runtime patch:

- `crates/calyx-registry/src/runtime/adapters/bridge.rs`
- `crates/calyx-registry/src/runtime/adapters/tests.rs`

Linux regression:

```bash
cargo test -p calyx-registry \
  runtime::adapters::tests::cpu_adapter_respawns_one_shot_helper_for_repeated_measurements \
  -- --nocapture
```

Result: passed on `aiwonder`.

## Source hashes

| Source | Path | SHA256 |
|---|---|---|
| ChEMBL 37 SQLite tarball | `/zfs/archive/calyx/biomed-rx/discovery/chembl-fresh/chembl_37_sqlite.tar.gz` | `33c203740555f96067710cdfc1c3c55d890660e5908ec5cbf5817492c290d281` |
| ChEMBL 37 extracted DB | `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/chembl_sqlite/chembl_37/chembl_37_sqlite/chembl_37.db` | `4be13df3b68e25dcd0bff44bf094033b5aebe98f415acdc8c1cdf380e0c15142` |
| ChEMBL 37 SDF | `/zfs/archive/calyx/biomed-rx/discovery/chembl/chembl_37.sdf.gz` | `f9735be33875fa15999bf9c30f068b3d9545b4e0db737e1387dd1a4e99ca155e` |
| ChEMBL 37 FASTA | `/zfs/archive/calyx/biomed-rx/discovery/chembl/chembl_37.fa.gz` | `8f59596c4ee8f6cc7abcc59a4ea6f785ce428945de322fe9be6fb50808a7a9ae` |
| BindingDB all TSV zip | `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDB_All_202606_tsv.zip` | `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7` |
| BindingDB target FASTA | `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDBTargetSequences.fasta` | `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81` |
| Open Targets validation edges | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_validation_edges.jsonl` | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| NCBI DPP4 RefSeq FASTA | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/ncbi_DPP4_NM_001935.fasta` | `024fa67def136d8572230fe1eb301de3b877c56c76a6de6015cad7fdd63fd4b7` |

## Candidate generation

BindingDB scan:

- Scanned rows: `3,182,518`
- Selected candidate counts before bounding: DPP4 `6,327`, TNF `3,799`, CD4 `84`, PLA2R1 `1`
- Persisted bounded BindingDB rows: `25`
- Artifact: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bindingdb_activity_candidates.jsonl`
- SHA256: `924c7fb2d20003ce3ba403b3f5e709eeb3360bb2a068642ab883749772c63a6d`

ChEMBL target activity counts:

| Target | Activities | Molecules | nM rows | SMILES rows |
|---|---:|---:|---:|---:|
| DPP4 | 8,397 | 5,752 | 6,469 | 8,392 |
| TNF | 6,447 | 2,582 | 3,023 | 6,442 |
| CD4 | 115 | 95 | 50 | 115 |
| PLA2R1 | 3 | 3 | 1 | 3 |

Generated row files:

- Raw generated rows: `57`, SHA256 `276fc1411ebecf95f7080bfdc32bdb082bb1c9bfc6ab16f5d6f11fb598e1300e`
- Deduped materialized rows: `53`, SHA256 `44d6444aba4e7f3d274d84a97cef416d09b7d864113f2ead705a3730e7d2bbb6`
- Duplicate provenance merged: `4` molecule duplicate rows where ChEMBL and BindingDB had the same measurement input.

## Materialized vault

Final materializer artifact:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/materialize.v8.stdout.json`
- SHA256 `9b54ec106935f0ca2a26342e03bdc167acc02ad0273ac4182cbe4f91d69deb40`

Persisted row counts:

| Count | Value |
|---|---:|
| Rows | 53 |
| Clinical rows | 22 |
| Molecular rows | 31 |
| Text rows | 22 |
| Molecule rows | 26 |
| Protein rows | 4 |
| DNA rows | 1 |
| Affinity/activity rows | 26 |
| Bridge terms | 58 |
| Anchors | 251 |
| Graph nodes | 111 |
| Graph edges | 238 |

Measured slot readback:

| Lens | Dense rows |
|---|---:|
| `semantic_bge_small_en_v1_5` | 22 |
| `semantic_all_minilm_l6_v2_onnx` | 22 |
| `domain_scibert_scivocab_uncased` | 22 |
| `bge_small_fp32_gpu` | 22 |
| `minilm_fp32_gpu` | 22 |
| `medcpt_query_fp32_gpu` | 22 |
| `medcpt_article_fp32_gpu` | 22 |
| `a38_medcpt_query_int8` | 22 |
| `a38_medcpt_article_int8` | 22 |
| `protein_esm2_t30_150m_adapter` | 4 |
| `dna_moderngena_base_adapter` | 1 |
| `molecule_chemberta_100m_adapter` | 26 |

Separate `cx-list` readback:

- Path: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/cx_list_v8_readback.json`
- SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Rows: `53`
- Rows with dense slot payloads: `53`

Vault tree readback:

- Path: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/vault_tree_v8_readback.json`
- SHA256: `1db907b52104ea3be9b61ac2f69871a8db5ab257dba726e408ddb9441a8f3d61`
- Lines: `809`
- Files: `753`
- Graph SST files: `352`
- Base SST files: `2`
- Anchor SST files: `2`

## Bridge report

Command output:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/domain_bridges.v8.stdout.json`

Report:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/domain_bridges.v8.report.json`
- SHA256 `d9dc59f2c7db4bbb8f73134049ff99ed2c0dd99cfc83da5fa6f76874e96f08e2`

Bridge candidates:

| Rank | Bridge | Row count | Rank score | Gate |
|---:|---|---:|---:|---|
| 1 | DPP4 | 15 | 0.9000 | pass |
| 2 | TNF | 14 | 0.8667 | pass |
| 3 | CD4 | 12 | 0.8000 | pass |
| 4 | PLA2R1 | 6 | 0.6000 | pass |
| 5 | metformin | 5 | 0.5667 | pass |
| 6 | ChEMBL1431 | 4 | 0.5333 | pass |
| 7 | ChEMBL237500 | 4 | 0.5333 | pass |
| 8 | linagliptin | 4 | 0.5333 | pass |

The gates prove the terms bridge the persisted clinical and molecular rows. They do not prove a treatment effect or novel biology.

## Fail-closed probes

Bad missing `source_sha256`:

- Input: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bad_missing_source_sha_rows.jsonl`
- RC: `2`
- Error: `molecular vault row 1 metadata requires source_sha256`

Bad unsupported modality:

- Input: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bad_unsupported_modality_rows.jsonl`
- RC: `2`
- Error: `molecular vault row 1 modality must be text, protein, dna, or molecule`

Mutation check:

- Pre-bad `cx-list` SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Post-bad `cx-list` SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Post-bad row count: `53`

## Full-scale plan

The first scaled vault is intentionally bounded. Full-scale execution should now use the fixed adapter lifecycle and split into deterministic batches:

1. Generate per-target ChEMBL activity batches with a row cap per target and a source hash on every row.
2. Generate per-target BindingDB batches with deduplicated molecule inputs and merged duplicate-source provenance.
3. Add target sequence rows for every mapped target with UniProt/ChEMBL accession and sequence hash.
4. Add DNA/transcript rows only from source-backed FASTA/RefSeq/Ensembl artifacts; do not synthesize DNA.
5. Materialize one vault per bounded target family or disease area, then merge graph-level bridge reports instead of overloading one vault transaction.
6. Run external validation layers before ranking: Open Targets, PubMed/PubTator relation evidence, ClinicalTrials.gov, DGIdb, safety/counter-evidence, and oncology-specific resources.
7. Treat all ranked outputs as hypotheses until falsification, safety, trial status, and expert review gates are complete.

The next useful work is not to call any result a cure. It is to keep expanding the validation stack around these measured bridge candidates.

---

## 27_pubtator_pubmed_relation_validation.md

# 27 - PubTator/PubMed relation validation

- **Issue:** #1176
- **Status:** Complete bounded FSV for PubTator/PubMed relation validation over current biomedical seeds.
- **FSV root:** `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z`

This is discovery and triage evidence only. It is not a cure, treatment recommendation, efficacy proof, causality proof, or clinical actionability claim.

## What changed

#1176 turns selected Calyx biomedical association candidates into external literature-backed evidence rows. The run resolved PubTator entity IDs, queried PubTator relation/search APIs, queried PubMed ESearch, exported PMID-level PubTator BioC JSON, and persisted support/negative/unresolved partitions with file hashes.

The corrected relation run intentionally used no relation-type filter on the PubTator relation endpoint, then filtered exact source/target pairs locally. The first relation probe with a literal `type=Any` returned empty lists; the final run fixed that and produced exact relation rows.

Source API references:

- PubTator3 API: `https://www.ncbi.nlm.nih.gov/research/pubtator3/api`
- PubTator autocomplete: `/research/pubtator3-api/entity/autocomplete/`
- PubTator relations: `/research/pubtator3-api/relations`
- PubTator search: `/research/pubtator3-api/search/`
- PubTator BioC JSON export: `/research/pubtator3-api/publications/export/biocjson`
- PubMed E-utilities overview: `https://www.ncbi.nlm.nih.gov/books/NBK25501/`
- PubMed ESearch help: `https://www.ncbi.nlm.nih.gov/books/NBK25499/`

## Persisted artifacts

| Artifact | Rows / entries | SHA256 |
|---|---:|---|
| `run_summary.json` | 1 | `196f7c92bbeee8c67c9c992d00cca6816c2fb1320c5e03c3f33f4eb5b396cb63` |
| `persisted_readback.json` | 108 files read back | `74288a592132be567c42f529003953a5d12767e60182b9009804279be05d54e2` |
| `api_response_hash_manifest.json` | 101 entries | persisted in FSV root |
| `parsed/query_inputs.jsonl` | 18 | persisted in FSV root |
| `parsed/pubtator_entity_mappings.jsonl` | 36 | persisted in FSV root |
| `parsed/pubtator_relation_rows.jsonl` | 49 | `14307bb504fa5ade1cde26ff953805d33d0ab48516963e36f2729ff50ce7f667` |
| `parsed/pubtator_search_results.jsonl` | 174 | persisted in FSV root |
| `parsed/pubmed_esearch_rows.jsonl` | 18 | persisted in FSV root |
| `parsed/pubtator_export_annotations.jsonl` | 144 | persisted in FSV root |
| `parsed/association_evidence_edges.jsonl` | 18 | `b8b79df00a00d8ddfcc607882adc68bc6277ccdc82799be8d3f4372b9f7fe7b0` |
| `parsed/supporting_literature.jsonl` | 141 | `bf473c33e99f596411116b8fb4a165ca1dd893a73399d552efa8979689ad9cb0` |
| `parsed/contradicting_or_negative_literature.jsonl` | 2 | persisted in FSV root |
| `parsed/unresolved_literature.jsonl` | 0 | persisted in FSV root |

Readback counts were computed from persisted files after the run, not from in-memory counters.

## Edge evidence

| Seed | Pair | Relation types | Relation publication sum | PubTator PMIDs | PubMed PMIDs | Export docs with both | Negative signal docs |
|---|---|---|---:|---:|---:|---:|---:|
| `metformin_type2_diabetes` | `@CHEMICAL_Metformin` -> `@DISEASE_Diabetes_Mellitus_Type_2` | associate, cause, treat | 8508 | 10 | 10 | 7 | 1 |
| `cd4_hiv_infections` | `@GENE_CD4` -> `@DISEASE_HIV_Infections` | associate, inhibit, stimulate | 5789 | 10 | 10 | 8 | 0 |
| `tnf_psoriasis` | `@GENE_TNF` -> `@DISEASE_Psoriasis` | associate, inhibit, stimulate | 1337 | 10 | 10 | 8 | 0 |
| `dpp4_type2_diabetes` | `@GENE_DPP4` -> `@DISEASE_Diabetes_Mellitus_Type_2` | associate, inhibit, stimulate | 976 | 10 | 10 | 8 | 0 |
| `pla2r1_membranous_nephropathy` | `@GENE_PLA2R1` -> `@DISEASE_Glomerulonephritis_Membranous` | associate, inhibit, stimulate | 705 | 10 | 10 | 8 | 0 |
| `tnf_asthma` | `@GENE_TNF` -> `@DISEASE_Asthma` | associate, inhibit, stimulate | 495 | 10 | 10 | 8 | 0 |
| `cd4_asthma` | `@GENE_CD4` -> `@DISEASE_Asthma` | associate, inhibit, stimulate | 469 | 10 | 10 | 8 | 0 |
| `linagliptin_type2_diabetes` | `@CHEMICAL_Linagliptin` -> `@DISEASE_Diabetes_Mellitus_Type_2` | associate, treat | 442 | 10 | 10 | 8 | 0 |
| `dpp4_linagliptin` | `@GENE_DPP4` -> `@CHEMICAL_Linagliptin` | associate, interact, negative_correlate, positive_correlate | 424 | 10 | 10 | 8 | 0 |
| `tnf_sarcoidosis` | `@GENE_TNF` -> `@DISEASE_Sarcoidosis` | associate, inhibit, stimulate | 264 | 10 | 10 | 8 | 0 |
| `cd4_sarcoidosis` | `@GENE_CD4` -> `@DISEASE_Sarcoidosis` | associate, inhibit, stimulate | 240 | 10 | 10 | 8 | 0 |
| `cd8a_psoriasis` | `@GENE_CD8A` -> `@DISEASE_Psoriasis` | associate, inhibit, stimulate | 127 | 10 | 4 | 8 | 0 |
| `dpp4_metformin` | `@GENE_DPP4` -> `@CHEMICAL_Metformin` | associate, negative_correlate, positive_correlate | 118 | 10 | 10 | 8 | 0 |
| `pla2r1_proteinuria` | `@GENE_PLA2R1` -> `@DISEASE_Proteinuria` | associate, inhibit, stimulate | 84 | 10 | 10 | 8 | 0 |
| `dpp4_hypertension` | `@GENE_DPP4` -> `@DISEASE_Hypertension` | associate, inhibit | 34 | 10 | 10 | 8 | 0 |
| `dpp4_asthma` | `@GENE_DPP4` -> `@DISEASE_Asthma` | associate, inhibit, stimulate | 31 | 10 | 10 | 8 | 0 |
| `dpp4_proteinuria` | `@GENE_DPP4` -> `@DISEASE_Proteinuria` | associate | 10 | 10 | 10 | 8 | 0 |
| `dpp4_schizophrenia` | `@GENE_DPP4` -> `@DISEASE_Schizophrenia` | associate | 4 | 4 | 6 | 6 | 1 |

All 18 seed edges had persisted support. The strongest PubTator relation-backed rows by publication count were metformin/type 2 diabetes, CD4/HIV infections, TNF/psoriasis, DPP4/type 2 diabetes, and PLA2R1/membranous nephropathy.

## Counter-evidence partition

The negative partition is a text-signal triage list, not final contradiction adjudication.

| Seed | PMID | Signal | Both selected entities in export? | PubTator relation count |
|---|---|---|---:|---:|
| `dpp4_schizophrenia` | `25937183` | `not associated` | false | 0 |
| `metformin_type2_diabetes` | `34904090` | `not significantly associated` | true | 2 |

These rows should feed #1184 before any hypothesis is promoted.

## FSV

The FSV readback proved:

- Raw query responses were persisted under `raw/`.
- Parsed evidence rows were persisted under `parsed/`.
- The final readback counted 18 query inputs, 36 entity mappings, 49 relation rows, 174 PubTator search result rows, 18 PubMed ESearch rows, 144 export annotation rows, 18 evidence edges, 141 supporting-literature rows, 2 negative-signal rows, 0 unresolved rows, and 72 request records.
- The FSV root contained 108 files at readback time.
- Every file in the FSV root was SHA-256 hashed in `persisted_readback.json`.

## Next

The immediate downstream queue is already filed: #1177 ClinicalTrials.gov, #1178 DGIdb, #1179 LINCS/CMap, #1180 oncology validation, #1181 safety/adverse-event triage, #1182 known-positive/negative gates, #1183 all-pair typed association miner, #1184 counter-evidence sweep, and #1185-#1194 domain/full-scale association hunts.

The useful claim unlocked by #1176 is: these selected biomedical associations now have persisted external literature and relation evidence suitable for downstream ranking and falsification. They are not yet validated interventions.

---

## 28_clinicaltrials_validation_ingest.md

# 28 - ClinicalTrials.gov validation ingest

- **Issue:** #1177
- **Status:** Complete bounded FSV for ClinicalTrials.gov trial-readiness evidence.
- **FSV root:** `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z`

This is discovery and triage evidence only. A trial listing, phase, status, or results flag is not a cure, treatment recommendation, efficacy proof, causality proof, or clinical actionability claim.

## What changed

#1177 adds ClinicalTrials.gov registry evidence for selected drug-condition hypotheses and target-derived drug expansions. The run used the official ClinicalTrials.gov v2 API endpoint:

- `https://clinicaltrials.gov/api/v2/studies`
- Query form: `filter.advanced=AREA[InterventionName] <intervention> AND AREA[Condition] <condition>`
- `pageSize=25`
- `format=json`
- `countTotal=true`

The run also persisted:

- API version response: `apiVersion=2.0.5`, `dataTimestamp=2026-07-02T09:00:05`
- OpenAPI snapshot from `https://clinicaltrials.gov/api/oas/v2`
- Source metadata and query provenance for every seed

The public API docs page states that the CTG API specification is available as YAML. The stable live source used by this run was `/api/oas/v2`; `/api/oas/v2.yaml` returned 404 during the FSV probe and was not used as source truth.

## Persisted artifacts

| Artifact | Rows / entries | SHA256 |
|---|---:|---|
| `run_summary.json` | 1 | `7b8f09bed81dd8826eaf480b8ae5cbff806592864f9dcd177f0e3fca9b610cd8` |
| `persisted_readback.json` | 1 | `2e0ec5c1d11a8710dd082816c90734ead83555887f51fcc2715054cf36d194fe` |
| `final_file_manifest.json` | 29 files | `c3ad269f03d25012d4e40d3cdf9b56404c7e44261deead843a0aef3f7869e9e6` |
| `clinicaltrials_api_version.json` | 1 | `de8921a29236b6d6afb41a57d264c51ce14c653bbf0d4899501e410254f2b355` |
| `clinicaltrials_oas_v2.yaml` | 1 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `parsed/query_inputs.jsonl` | 13 | persisted in FSV root |
| `parsed/request_records.jsonl` | 15 | persisted in FSV root |
| `parsed/clinicaltrials_trial_rows.jsonl` | 269 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| `parsed/clinicaltrials_seed_summaries.jsonl` | 13 | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| `parsed/clinicaltrials_error_rows.jsonl` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

## Run summary

| Count | Value |
|---|---:|
| Query seeds | 13 |
| Total registry hits across seeds | 1,636 |
| Returned first-page trial rows | 269 |
| Exact intervention matches in returned rows | 260 |
| Returned rows with results available | 114 |
| Returned rows with stopped status | 33 |
| API / parse error rows | 0 |

Each seed was capped at a first page of 25 studies. `total_count` and `next_page_available` were persisted so truncated evidence is visible.

## Seed evidence

| Seed | Total hits | Returned | Exact intervention | Results | Stopped | Max score | Status counts |
|---|---:|---:|---:|---:|---:|---:|---|
| `metformin_type2_diabetes` | 732 | 25 | 25 | 7 | 0 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:21, RECRUITING:1, UNKNOWN:2 |
| `sitagliptin_type2_diabetes` | 335 | 25 | 24 | 12 | 3 | 5.6 | COMPLETED:18, TERMINATED:2, UNKNOWN:4, WITHDRAWN:1 |
| `vildagliptin_type2_diabetes` | 133 | 25 | 24 | 0 | 0 | 4.6 | COMPLETED:21, UNKNOWN:4 |
| `linagliptin_type2_diabetes` | 93 | 25 | 22 | 19 | 1 | 5.6 | COMPLETED:21, RECRUITING:1, TERMINATED:1, UNKNOWN:2 |
| `etanercept_psoriasis` | 86 | 25 | 24 | 15 | 2 | 5.6 | COMPLETED:21, TERMINATED:1, UNKNOWN:2, WITHDRAWN:1 |
| `adalimumab_psoriasis` | 79 | 25 | 23 | 13 | 0 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:21, UNKNOWN:3 |
| `metformin_breast_cancer` | 55 | 25 | 25 | 8 | 7 | 5.2 | ACTIVE_NOT_RECRUITING:1, COMPLETED:10, RECRUITING:3, TERMINATED:6, UNKNOWN:4, WITHDRAWN:1 |
| `alogliptin_type2_diabetes` | 41 | 25 | 25 | 16 | 1 | 5.6 | COMPLETED:21, RECRUITING:1, UNKNOWN:2, WITHDRAWN:1 |
| `metformin_prostate_cancer` | 33 | 25 | 25 | 5 | 10 | 5.2 | ACTIVE_NOT_RECRUITING:1, COMPLETED:7, NOT_YET_RECRUITING:1, RECRUITING:3, TERMINATED:6, UNKNOWN:3, WITHDRAWN:4 |
| `infliximab_psoriasis` | 30 | 25 | 25 | 14 | 1 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:19, NOT_YET_RECRUITING:1, RECRUITING:1, TERMINATED:1, UNKNOWN:2 |
| `metformin_colorectal_cancer` | 13 | 13 | 13 | 4 | 5 | 5.2 | COMPLETED:5, TERMINATED:4, UNKNOWN:3, WITHDRAWN:1 |
| `adalimumab_sarcoidosis` | 4 | 4 | 3 | 1 | 3 | 4.2 | COMPLETED:1, TERMINATED:1, WITHDRAWN:2 |
| `infliximab_sarcoidosis` | 2 | 2 | 2 | 0 | 0 | 4.4 | COMPLETED:2 |

## Highest trial-readiness rows

The score is a registry-readiness score only. It rewards exact intervention match, condition match, active/completed status, result availability, and later phase; it penalizes stopped status.

| Seed | NCT | Status | Phase | Results | Score | Sponsor | Title |
|---|---|---|---|---:|---:|---|---|
| `metformin_type2_diabetes` | `NCT00751114` | COMPLETED | PHASE4 | true | 5.6 | Sanofi | Evaluation of Insulin Glargine Versus Sitagliptin in Insulin-naive Patients |
| `linagliptin_type2_diabetes` | `NCT02350478` | COMPLETED | PHASE4 | true | 5.6 | Medical University of Graz | Effects of Linagliptin on Endothelial Function |
| `sitagliptin_type2_diabetes` | `NCT00751114` | COMPLETED | PHASE4 | true | 5.6 | Sanofi | Evaluation of Insulin Glargine Versus Sitagliptin in Insulin-naive Patients |
| `sitagliptin_type2_diabetes` | `NCT00885638` | COMPLETED | PHASE4 | true | 5.6 | Lund University | Effects of Dipeptidyl Peptidase-4 Inhibition on Hormonal Responses to Meal Ingestion |
| `alogliptin_type2_diabetes` | `NCT02771093` | COMPLETED | PHASE4 | true | 5.6 | Takeda | An Exploratory Study of the Effects of Trelagliptin and Alogliptin on Glucose Variability |
| `adalimumab_psoriasis` | `NCT00735787` | COMPLETED | PHASE4 | true | 5.6 | Abbott | Controlled Study of Humira in Subjects With Chronic Plaque Psoriasis of the Hand |
| `etanercept_psoriasis` | `NCT02749370` | COMPLETED | PHASE4 | true | 5.6 | Amgen | Study to Evaluate the Efficacy of Etanercept Treatment in Adults Who Failed Therapy |
| `infliximab_psoriasis` | `NCT00686595` | COMPLETED | PHASE4 | true | 5.6 | Merck Sharp & Dohme LLC | A Study to Evaluate the Switch From Etanercept to Infliximab in Subjects With Moderate-to-Severe Psoriasis |

## FSV

The FSV readback proved:

- Raw ClinicalTrials.gov responses were persisted under `raw/`.
- Parsed query inputs, request records, NCT rows, seed summaries, and error rows were persisted under `parsed/`.
- The run failed closed with zero API/parse error rows, not by treating errors as no evidence.
- A final manifest was written after the readback file existed, proving the readback hash separately.

## Next

Trial registry evidence should now feed:

- #1181 safety/adverse-event triage, because trials with results need safety extraction before any intervention promotion.
- #1182 known-positive/negative and time-split validation gates, because diabetes and psoriasis rows provide strong known-positive calibration material.
- #1184 counter-evidence sweep, because stopped statuses are not failures by themselves but must be reviewed before ranking.

The useful claim unlocked by #1177 is: selected drug-condition hypotheses now have persisted trial-readiness evidence. This still does not prove that any candidate is effective, safe, actionable, or curative.

---

## 29_dgidb_drug_gene_validation.md

# 29 - DGIdb drug-gene validation

- **Issue:** #1178
- **Status:** Complete bounded FSV for DGIdb drug-gene and druggability evidence.
- **FSV root:** `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z`

This is discovery and triage evidence only. A DGIdb interaction, druggability category, source label, or database-provided `CLINICALLY ACTIONABLE` category is not a Calyx claim of efficacy, safety, clinical actionability, treatment recommendation, or cure.

## What changed

#1178 adds DGIdb evidence to the association stack:

- Full source TSV bytes for `latest (2024-Dec)` downloads.
- Source/license metadata via DGIdb GraphQL.
- Mapped target genes and candidate drugs to source-specific overlay IDs.
- Exact pair interaction rows for current target/drug seeds.
- Broad target-gene and target-drug interaction rows for expansion.
- Druggability category edges for target genes.
- Unmapped/no-hit rows for exact-pair controls.

DGIdb source references:

- Downloads: `https://dgidb.org/downloads`
- API docs page: `https://dgidb.org/api`
- GraphQL endpoint: `https://dgidb.org/api/graphql`
- Latest release API: `https://api.github.com/repos/dgidb/dgidb-v5/releases?per_page=1`
- DGIdb v5.0 article: `https://academic.oup.com/nar/article/52/D1/D1227/7416371`

The DGIdb client page says the TSV downloads include all mapped gene, drug, and drug-gene interaction claims, but also warns that some imported source databases have redistribution restrictions. #1178 therefore persists `source_license_rows.jsonl` and keeps source/license constraints attached to downstream edges.

## Source files

| File | Rows | Bytes | SHA256 |
|---|---:|---:|---|
| `interactions.tsv` | 98,239 | 12,178,745 | `08af778126a4f22a10fddb7fe06745df07f068ce76c016fe22c59c524ef3de9c` |
| `genes.tsv` | 80,234 | 4,356,295 | `f090a58280b7f410e9e68b75bb6ea0c00c439c2ed10182eb58a46b6e2583825d` |
| `drugs.tsv` | 81,572 | 8,029,920 | `f939ee92621125dbfca8bdae23d2086605405b8274190fc69e39104c614142d2` |
| `categories.tsv` | 32,795 | 1,557,934 | `946c513cd3ed9c94e36b24681d42e1e598aa7edfac8d3461ffa283504657f3db` |

## Persisted artifacts

| Artifact | Rows / entries | SHA256 |
|---|---:|---|
| `run_summary.json` | 1 | `2aa783b62086f6b338122d82a06dd40b6562a10bbc39e1fede363831aea4a218` |
| `persisted_readback.json` | 1 | `a1ffec6061dbc3420aa65fe807ec04774308013c154f50f78d398ae9c679b06c` |
| `final_file_manifest.json` | 66 files | `01be2e10755641d058fc9b70afaddd1961e98a62d228458f2f5fcc4411bcf9ff` |
| `parsed/source_license_rows.jsonl` | 45 | `8bb52709a3582cf6a38158ca67a56f23e35a413c45e3ef1c0f489bc6bce26ed4` |
| `parsed/target_gene_mappings.jsonl` | 5 | `dc3788792f14d1037bdcdd726dc695581eb966283f300ef984f5633800a1c0d5` |
| `parsed/target_drug_mappings.jsonl` | 24 | `eeae0a77a3a68510dc8c9b9924abee7cff153fd9eb4024f8f530c97505d82a16` |
| `parsed/gene_druggability_rows.jsonl` | 38 | `7e859f9d2993f836995157badcd5b2942544e3c75e43b8443d35cd656bc5b04e` |
| `parsed/relevant_tsv_interactions.jsonl` | 887 | persisted in FSV root |
| `parsed/seed_pair_tsv_interactions.jsonl` | 41 | persisted in FSV root |
| `parsed/seed_pair_graphql_interactions.jsonl` | 12 | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| `parsed/broad_graphql_interactions.jsonl` | 359 | persisted in FSV root |
| `parsed/dgidb_graph_edges.jsonl` | 91 | `42e8a26fb7976c3907612130e830a22927cf5ce6406859bd62a63398fe018a54` |
| `parsed/unmapped_rows.jsonl` | 3 | persisted in FSV root |
| `parsed/error_rows.jsonl` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

An initial GraphQL mapping query used invalid `CategoryWithSources` field names and was preserved as `parsed/initial_error_rows_before_mapping_repair.jsonl` with one row. The corrected final readback has zero error rows.

## Run summary

| Count | Value |
|---|---:|
| Target genes | 5 |
| Target drug search terms | 14 |
| Exact pair seeds | 13 |
| Relevant TSV interaction rows | 887 |
| Exact-pair TSV claim rows | 41 |
| Exact-pair GraphQL interaction rows | 12 |
| Broad GraphQL interaction rows | 359 |
| Source/license rows | 45 |
| Gene mapping rows | 5 |
| Drug mapping rows | 24 |
| Gene druggability rows | 38 |
| Graph edge rows | 91 |
| Unmapped exact-pair rows | 3 |
| Final error rows | 0 |

## Gene mappings

| Gene | DGIdb concept | Overlay | Categories |
|---|---|---|---|
| `DPP4` | `hgnc:3009` | `concept:hgnc:3009` | PROTEASE, DRUGGABLE GENOME, ENZYME, CELL SURFACE |
| `CD8A` | `hgnc:1706` | `concept:hgnc:1706` | DRUGGABLE GENOME, EXTERNAL SIDE OF PLASMA MEMBRANE, KINASE |
| `CD4` | `hgnc:1678` | `concept:hgnc:1678` | DRUGGABLE GENOME, CELL SURFACE, TYROSINE KINASE, EXTERNAL SIDE OF PLASMA MEMBRANE, KINASE |
| `PLA2R1` | `hgnc:9042` | `concept:hgnc:9042` | DRUGGABLE GENOME, CELL SURFACE, KINASE |
| `TNF` | `hgnc:11892` | `concept:hgnc:11892` | DRUGGABLE GENOME, CELL SURFACE, EXTERNAL SIDE OF PLASMA MEMBRANE, CLINICALLY ACTIONABLE |

The `CLINICALLY ACTIONABLE` value is a DGIdb source category for TNF. It is useful as a triage label and calibration signal; it is not a Calyx clinical-actionability assertion.

## Exact-pair evidence

| Seed | GraphQL hits | TSV claim rows |
|---|---:|---:|
| `cd4_ibalizumab` | 2 | 3 |
| `dpp4_alogliptin` | 1 | 2 |
| `dpp4_linagliptin` | 1 | 1 |
| `dpp4_metformin` | 0 | 0 |
| `dpp4_saxagliptin` | 0 | 3 |
| `dpp4_sitagliptin` | 1 | 3 |
| `dpp4_vildagliptin` | 1 | 2 |
| `pla2r1_rituximab` | 0 | 0 |
| `tnf_adalimumab` | 1 | 6 |
| `tnf_certolizumab` | 2 | 5 |
| `tnf_etanercept` | 1 | 6 |
| `tnf_golimumab` | 1 | 5 |
| `tnf_infliximab` | 1 | 5 |

No-hit exact pair controls:

- `dpp4_metformin`
- `pla2r1_rituximab`
- `dpp4_saxagliptin` in GraphQL, while the TSV still contains 3 exact claim rows; this mismatch is preserved for downstream reconciliation.

## Top interaction rows

| Seed | Drug | Gene | Type | Score | Evidence | PMIDs | Sources |
|---|---|---|---|---:|---:|---|---|
| `cd4_ibalizumab` | IBALIZUMAB | CD4 | antibody | 5.881 | 2 | - | GuideToPharmacology, TTD |
| `cd4_ibalizumab` | IBALIZUMAB | CD4 | inhibitor | 2.941 | 1 | - | ChEMBL |
| `tnf_golimumab` | GOLIMUMAB | TNF | inhibitor | 1.527 | 6 | 37763115 | ChEMBL, PharmGKB, TEND, TTD, TdgClinicalTrial |
| `dpp4_sitagliptin` | SITAGLIPTIN | DPP4 | inhibitor | 1.252 | 7 | 27249660, 29264572, 39792745 | GuideToPharmacology, PharmGKB, TEND, TdgClinicalTrial |
| `dpp4_vildagliptin` | VILDAGLIPTIN | DPP4 | inhibitor | 1.022 | 5 | 27249660 | ChEMBL, GuideToPharmacology, PharmGKB, TdgClinicalTrial |
| `dpp4_alogliptin` | ALOGLIPTIN | DPP4 | inhibitor | 0.954 | 2 | - | GuideToPharmacology, TdgClinicalTrial |
| `dpp4_linagliptin` | LINAGLIPTIN | DPP4 | inhibitor | 0.715 | 4 | 27249660 | ChEMBL, GuideToPharmacology, PharmGKB |
| `tnf_certolizumab` | CERTOLIZUMAB PEGOL | TNF | inhibitor | 0.339 | 6 | 37763115 | ChEMBL, PharmGKB, TEND, TTD, TdgClinicalTrial |
| `tnf_etanercept` | ETANERCEPT | TNF | inhibitor | 0.068 | 4 | - | ChEMBL, TEND, TTD, TdgClinicalTrial |
| `tnf_adalimumab` | ADALIMUMAB | TNF | inhibitor | 0.058 | 4 | - | ChEMBL, TEND, TTD, TdgClinicalTrial |
| `tnf_infliximab` | INFLIXIMAB | TNF | inhibitor | 0.058 | 4 | - | ChEMBL, TEND, TTD, TdgClinicalTrial |

## FSV

The final readback proved:

- Source bytes/API responses were persisted under `raw/`.
- Parsed source-file rows, source/license rows, mappings, interactions, graph edges, unmapped rows, request records, and error rows were persisted under `parsed/`.
- Final parsed error rows are empty.
- The initial mapping-query schema failure is preserved separately and did not get converted into false no-evidence rows.
- `final_file_manifest.json` was written after `persisted_readback.json` existed, so the readback file is hash-backed.

## Next

DGIdb evidence should now feed:

- #1179 LINCS/CMap transcriptomic reversal screen, because drug-target edges identify compounds to test against disease/pathway signatures.
- #1181 safety/adverse-event triage, because source license rows show mixed redistribution constraints and drug evidence must be safety-reviewed.
- #1182 known-positive/negative gates, because DPP4-inhibitor and anti-TNF rows are strong known-positive calibration material.
- #1183 all-pair typed association miner, because the 91 DGIdb graph edges are now association-ready.
- #1184 counter-evidence sweep, because TSV/GraphQL mismatches and no-hit controls must be treated as falsification inputs.

The useful claim unlocked by #1178 is: selected drug-target hypotheses now have persisted source-backed DGIdb interaction, druggability, source/license, publication, and no-hit evidence. It still does not prove treatment efficacy, safety, novelty, clinical actionability, or a cure.

---

## 30_evidence_outcome_instrument_association_substrate.md

# 30 - Calyx DB evidence/outcome/instrument association substrate

- **Issue:** #1196
- **Status:** Complete FSV for the accepted Calyx/Aster Graph CF collection.
- **FSV root:** `/home/croyse/calyx/fsv/issue1196-calyx-db-evidence-substrate-v3-20260703T200335Z`
- **Vault:** `corpus-anchored-869-20260625T080546Z`
- **Vault ID:** `01KVYX0KYVBQSGVC6N2S00FX6J`
- **Accepted collection:** `biomed_evidence_substrate_v3`

This is an evidence substrate for discovery and triage. It is not a Calyx claim of efficacy,
safety, clinical actionability, treatment recommendation, or cure. The useful claim is narrower:
source-backed biomedical evidence rows, outcome rows, measurement instruments, and positive and
negative/caution signals now exist as a physical Calyx/Aster graph collection with direct readback.

## What changed

#1196 materializes the evidence from #1176, #1177, and #1178 into the Calyx database, not as a
sidecar-only report:

- PubTator/PubMed relation evidence and contradicting literature.
- ClinicalTrials.gov intervention-condition trial evidence.
- Clinical outcome rows and their measurement instruments.
- DGIdb drug-gene interaction and druggability evidence.
- Source files, FSV roots, hashes, source licenses, and unmapped/null rows.
- Positive association evidence and negative/caution signals.
- A bounded in-memory CSR projection for the accepted collection, written into Aster Graph CF.
- Direct physical readback of every expected node row, edge row, metadata row, and CSR artifact.

The command also adds the CLI surface:

```text
calyx materialize-evidence-substrate <vault> \
  --pubtator-root <fsv root> \
  --clinicaltrials-root <fsv root> \
  --dgidb-root <fsv root> \
  --collection <collection> \
  --report <path> \
  --home <calyx home>
```

## What was run

```text
cd /home/croyse/calyx/repo
./target/debug/calyx materialize-evidence-substrate corpus-anchored-869-20260625T080546Z \
  --pubtator-root /home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z \
  --clinicaltrials-root /home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z \
  --dgidb-root /home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z \
  --collection biomed_evidence_substrate_v3 \
  --report /home/croyse/calyx/fsv/issue1196-calyx-db-evidence-substrate-v3-20260703T200335Z/calyx_db_readback.json \
  --home /home/croyse/calyx
```

## Accepted persisted artifacts

| Artifact | Bytes | SHA256 |
|---|---:|---|
| `calyx_db_readback.json` | 8,323 | `19d4e52153b280a7c630bd865b04b7db6ed41fd2a50bfffd6995a202ec55df1a` |
| `command_stdout.json` | 6,689 | `95858dfff8416356a161df8d6df38ab53de8d9a1eded5239347fcb6a02c5c3c5` |
| `command_stderr.log` | 99 | `3f55f869f08888f5f2fe2919dbf00e98cddd477e06acd7929280f77a348f0789` |

The stderr line was informational CSR loading output:

```text
plain-graph: loading persisted CSR collection=biomed_evidence_substrate_v3 nodes=10092 edges=21496
```

## Source roots verified

| Family | FSV root | Files | Bytes | Aggregate SHA256 | Self-manifest skip |
|---|---|---:|---:|---|---|
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` | 108 | 9,126,046 | `d44135733a2dea9f24f756fa7525240072d9c835a5cb67fdc337e4203a192a80` | `persisted_readback.json` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` | 28 | 10,926,464 | `00cc0162fe0e2eb60d7b1e0950cca2ac853349e31e573e19e70dfc184ff9b84e` | none |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` | 66 | 30,115,744 | `bd6df5ae31e358743485f177282b9d137a69cffd6fb557575382b4c9b25424b3` | `persisted_readback.json` |

The self-manifest skip applies only to the `persisted_readback.json` file inside roots where that
file records itself. The command still records the actual bytes and hash, and it verifies all other
manifest rows against physical files before materializing graph rows.

## Graph summary

| Count | Value |
|---|---:|
| Nodes | 10,092 |
| Edges | 21,496 |
| Source rows from PubTator/PubMed | 672 |
| Source rows from ClinicalTrials.gov | 310 |
| Source rows from DGIdb | 1,550 |
| Metadata rows | 1 |

Selected node counts:

| Node type | Count |
|---|---:|
| `outcome` | 3,587 |
| `measurement_instrument` | 2,036 |
| `source_row` | 1,950 |
| `concept` | 875 |
| `publication` | 173 |
| `trial` | 257 |
| `drug_gene_interaction` | 91 |
| `fsv_artifact` | 202 |
| `source_database` | 45 |
| `source_license` | 45 |
| `association_evidence` | 18 |
| `negative_literature` | 2 |
| `unmapped_row` | 3 |

Selected edge counts:

| Edge type | Count |
|---|---:|
| `measures_outcome` | 3,587 |
| `measured_with` | 3,587 |
| `derived_from` | 7,839 |
| `publication_mentions` | 1,293 |
| `evidence_row_subject` | 751 |
| `evidence_row_object` | 751 |
| `supports_association` | 18 |
| `negative_evidence_association` | 2 |
| `clinical_trial_association` | 13 |
| `negative_or_caution_trial_signal` | 33 |
| `drug_gene_source_association` | 40 |
| `negative_or_null_dgidb_signal` | 3 |
| `has_hash` | 202 |
| `has_license` | 45 |

## Physical Calyx/Aster readback

The source of truth was the physical Aster Graph CF via `PhysicalPlainGraph` node, edge, and CSR
readback.

| Readback field | Value |
|---|---:|
| Expected node rows written | 10,092 |
| Physical node rows read back | 10,092 |
| All node values matched | true |
| Expected edge rows written | 21,496 |
| Physical edge rows read back | 21,496 |
| All edge values matched | true |
| Metadata rows written | 1 |
| CSR nodes | 10,092 |
| CSR edges | 21,496 |
| Association CSR nodes | 10,092 |
| Association CSR edges | 21,140 |
| CSR bytes | 1,969,124 |
| Source snapshot | 683,833 |

CSR hashes:

| Hash | Value |
|---|---|
| SHA256 | `0351b67699f6560527706714121c51191a9380242dbbe802891ce3891a9dd3e9` |
| BLAKE3 | `3e18003673dbf702e0997a7b9082d6e36601de2df5aad83533f155d030ad2230` |

## Representative paths

Positive/supporting paths:

```text
concept:@GENE_DPP4
  -> source_row:pubtator_pubmed:parsed/association_evidence_edges.jsonl:1
  -> concept:@DISEASE_Diabetes_Mellitus_Type_2

concept_text:drug:metformin
  -> source_row:clinicaltrials:parsed/clinicaltrials_trial_rows.jsonl:1
  -> nct:nct00449930
  -> concept_text:disease:type_2_diabetes

concept:concept:rxcui:1100699
  -> source_row:dgidb:parsed/dgidb_graph_edges.jsonl:1
  -> concept:concept:hgnc:3009
```

Negative/caution paths:

```text
concept:@GENE_DPP4
  -> source_row:pubtator_pubmed:parsed/contradicting_or_negative_literature.jsonl:1
  -> concept:@DISEASE_Schizophrenia

concept_text:drug:linagliptin
  -> source_row:clinicaltrials:parsed/clinicaltrials_trial_rows.jsonl:50
  -> concept_text:disease:type_2_diabetes

concept_text:drug:saxagliptin
  -> source_row:dgidb:parsed/unmapped_rows.jsonl:1
  -> concept_text:gene:dpp4
```

Outcome/instrument path:

```text
nct:nct01812954
  -> outcome:clinicaltrials:NCT01812954:primary:0:cost_per_quality_adjusted_life_year_qaly_gained_through_treatment_with_each_individual_agent_compared_to_supportive_care_as_well_as_compared_to_placebo
  -> clinical_measure:cost_per_quality_adjusted_life_year_qaly_gained_through_treatment_with_each_individual_agent_compared_to_supportive_care_as_well_as_compared_to_placebo
```

## Storage history and spillover

Earlier attempted collections were not accepted:

- `biomed_evidence_substrate` used row-by-row graph writes, created many small SSTs, and was
  terminated. Recovery wrote 84,680 durable rows through sequence 683,826, then Graph CF was compacted.
- `biomed_evidence_substrate_v2` wrote batched rows but attempted a full Graph CF CSR range scan and
  was terminated. Recovery found no new rows after durable sequence 683,831, then Graph CF was compacted.
- `biomed_evidence_substrate_v3` is the accepted collection because it uses bounded collection-local
  direct readback and an in-memory CSR projection for the just-materialized graph.

The old durable aborted collections are a separate Calyx storage lifecycle problem, not part of the
accepted evidence-substrate claim. They need Calyx-native cleanup or atomic collection replacement
work tracked outside this report.

## Findings

- The evidence, outcome, measurement-instrument, license, hash, and negative/caution rows now exist
  inside the Calyx database as Aster Graph CF rows for `biomed_evidence_substrate_v3`.
- The accepted collection has direct physical row readback for all expected node and edge values.
- The CSR artifact is persisted and hash-backed.
- The source FSV roots were re-read and verified before graph materialization.
- The graph is useful for association discovery because positive evidence and counter-evidence are
  co-located in one Calyx collection.
- This does not yet prove cure, efficacy, safety, novelty, or clinical actionability.

## Next

This substrate should feed:

- all-pair typed association mining over the Calyx graph;
- outcome/instrument-aware ranking so gates compare claims to actual measured outcomes;
- counter-evidence and null-result suppression;
- clinical actionability gates that require external power-proven instruments and not association-only
  inference;
- Calyx-native collection lifecycle cleanup for failed/aborted materialization attempts.

---

## 31_lincs_cmap_reversal_screen.md

# #1179 LINCS/CMap Reversal Screen

Status: complete for the bounded #1179 screen. This is not a treatment, cure,
clinical recommendation, or actionability claim. Transcriptomic reversal is a
lead-generation signal only.

## Sources

- CREEDS disease signatures: `https://maayanlab.cloud/CREEDS/`
- L1000CDS2 reverse perturbation search: `https://maayanlab.cloud/L1000CDS2/`
- FSV root:
  `/home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z`
- Source manifest readback:
  `persisted_readback.json`, 74 files, 48,002,018 bytes,
  aggregate sha256
  `a0dd0b367c1058c446153a9d92b3ce096d59ef0ac1a0433b30a6d07498a40f1c`.

The screen selected 30 real CREEDS disease signatures and submitted 30
reverse-mode L1000CDS2 gene-set queries. All 30 returned HTTP 200 responses.

## Calyx DB Materialization

Accepted collection: `biomed_lincs_cmap_reversal_v4`

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --collection biomed_lincs_cmap_reversal_v4 \
  --report /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z/calyx_db_readback_v4.json \
  --home /home/croyse/calyx
```

Readback report:

- `calyx_db_readback_v4.json`
- bytes: 21,467
- sha256: `9d56ebcf42edb543bb69b8a6b0ec98e932b0a38e59415de88eecb266f444b7e0`
- stdout JSON parsed equal to report JSON: true
- exit file: `materialize_lincs_v4.exit` contained `0`

Physical Calyx readback:

- nodes written: 6,551
- edges written: 13,532
- physical node keys: 6,551
- physical outgoing edge keys: 13,532
- persisted CSR nodes: 6,551
- persisted CSR edges: 13,532
- assoc graph nodes: 6,551
- assoc graph edges: 13,532
- CSR bytes: 1,300,710
- CSR sha256: `0800c6f8cb241d2e00214076d5cb53131fcddf8d07948ff19be4dea8a46e16ed`
- CSR blake3:
  `ac5871bfc4557d80107a03e8b8cb49620a7fd0a678ee4592f6579c39e8bfaca1`
- all node values were physically read back
- all edge keys were physically counted by collection-local range
- 512 deterministic edge values were physically read back

The first attempts are not accepted state:

- `biomed_lincs_cmap_reversal_v1`: killed after pathological full edge-value
  point-readback, exit `143`.
- `biomed_lincs_cmap_reversal_v2` and `v3`: timed out, exit `124`.
- Cleanup/atomic replacement is tracked by #1197.
- A batch edge-value readback API is tracked by #1198.

## Parsed Evidence

Persisted parsed rows:

- disease signature inputs: 30
- L1000CDS2 request records: 30
- LINCS reversal score rows: 1,500
- unsupported/unmapped current-candidate rows: 390

Graph node types include disease signatures, CREEDS source rows, L1000CDS2
queries, reversal score rows, perturbations, LINCS signatures, cell lines,
chemical identifiers, current candidate drugs, unsupported cases, source
artifacts, hashes, GEO series, UMLS concepts, and disease ontology IDs.

Important edge families include:

- `has_lincs_reversal_score`: 1,500
- `scores_perturbation`: 1,500
- `returned_reversal_score`: 1,500
- `measured_in_cell_line`: 1,500
- `has_lincs_signature`: 1,500
- `absent_from_l1000cds2_top50_reverse_results`: 390
- `unsupported_candidate`: 390

## Current Candidate Result

The current Calyx candidate-drug set did not appear in the top-50 reverse
L1000CDS2 results for the 30 selected disease signatures. This is not evidence
that the drugs do not work clinically; it is a bounded negative lead-signal
result for this data source, query mode, and selected signature set.

The unsupported rows cover:

- metformin
- linagliptin
- sitagliptin
- saxagliptin
- alogliptin
- vildagliptin
- adalimumab
- certolizumab
- etanercept
- golimumab
- infliximab
- ibalizumab
- rituximab

## Repeated Reversal Leads

The most repeated perturbation labels in the 1,500 top-50 rows were:

| Perturbation | Pert ID | Rows | Min rank | Max score | Boundary |
|---|---:|---:|---:|---:|---|
| `-666` | multiple BRD IDs | 252 | 1 | 0.1107 | placeholder label; corrected/resolved by #1199 |
| CGP-60474 | `BRD-K79090631` | 76 | 1 | 0.0705 | lead signal only |
| vorinostat | `BRD-K81418486` | 60 | 1 | 0.0936 | lead signal only |
| trichostatin A | `BRD-A19037878` | 44 | 3 | 0.0749 | lead signal only |
| geldanamycin | `BRD-A19500257` | 43 | 3 | 0.0610 | lead signal only |
| mitoxantrone | `BRD-K21680192` | 28 | 1 | 0.0571 | lead signal only |
| PD-0325901 | `BRD-K49865102` | 26 | 5 | 0.0605 | lead signal only |
| alvocidib | `BRD-K87909389` | 25 | 2 | 0.0591 | lead signal only |
| Narciclasine | `BRD-K06792661` | 21 | 2 | 0.1033 | lead signal only |

The `-666` row is a placeholder-label data-quality blocker, not a usable
therapeutic lead. #1199 resolved this by mapping the underlying BRD perturbation
IDs against authoritative LINCS/L1000FWD metadata and preserving unresolved rows.

## Conclusion

#1179 produced a grounded LINCS/CMap association substrate inside Calyx:
CREEDS disease signatures, L1000CDS2 query provenance, response hashes,
reversal-score rows, unsupported current-candidate rows, perturbation nodes,
and physical Graph CF/CSR readback are now in one collection.

This does not unlock a cure claim. It creates a verified association layer that
can feed the next tasks: perturbation metadata mapping (#1199), safety and
counter-evidence gates, broader signature coverage, clinical evidence joins,
and lead-ranking gates that explicitly reject unsupported clinical claims.

---

## 32_lincs_perturbation_metadata_mapping.md

# #1199 LINCS Perturbation Metadata Mapping

Status: complete for the bounded #1199 mapping slice. This is not a
treatment, cure, clinical recommendation, or actionability claim. LINCS/CMap
metadata makes perturbation labels interpretable; it does not prove efficacy or
safety.

## Sources

- L1000FWD downloads/API pages:
  `https://maayanlab.cloud/l1000fwd/download_page` and
  `https://maayanlab.cloud/l1000fwd/api_page`
- FSV root:
  `/home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z`
- Source manifest readback:
  `persisted_readback.json`, 9 files, 13,427,781 bytes, aggregate sha256
  `af874468aab777c8f9466f05770bb34c02fcc38f6b9d7fbd2542b0fc4ee598ce`.

Authoritative metadata files downloaded and hashed:

- `raw/Drugs_metadata.csv`: 7,797,325 bytes,
  sha256 `6447c511ac7d4111f2bd5e46cc95f0ca872d98d579d5e4ccaef6594ed7b899e3`
- `raw/CD_signature_metadata.csv`: 5,075,455 bytes,
  sha256 `b4495715d35f9bc17412c5ffa900d802812b25419ecd6291be6dd6fa4f38a557`
- `raw/download_page.html`: 26,703 bytes,
  sha256 `af82fe8d7a2f206aa77deafecfd2b8cf569dfa1dd2f50f53a950fda8c8f82763`
- `raw/api_page.html`: 57,972 bytes,
  sha256 `3c150dc4bc324294269650534ce90f7961d7cdfdee106164fb449bbe77f110b8`

Derived files:

- `parsed/perturbation_id_mappings.jsonl`: 530 rows,
  sha256 `dc2ed13f008bec632c4d0638685d35fcd78e7ba3f0b0e0ff5e0748d139c7580c`
- `parsed/placeholder_pert_desc_cases.jsonl`: 252 rows,
  sha256 `718a16debe90aab6dbb69769ccfa3ef24dd9bfa6478996a825e277d18f434098`
- `parsed/resolved_repeated_leads.jsonl`: 487 rows,
  sha256 `653ffd05a8fe9b8801d08e2018ec44012f05ca4863ae6b4de305b1f623ecb638`
- `mapping_summary.json`: sha256
  `1391ab3d5c7bd5c76bb3c6a8fb2cba308a4f4930951f9f9d9ed897c6c87ce4e0`

## Correction To #1179

The #1179 repeated-lead table grouped by `pert_desc`, so the placeholder label
`-666` was incorrectly easy to read as one perturbation. It is not one
compound. In the #1179 score rows, `-666` spans 252 rows and 148 unique
`pert_id` values.

Authoritative metadata resolves many of those IDs independently. Example:

- `BRD-K84595254` resolves to `strophanthidin`, PubChem CID `6185`,
  LSM ID `LSM-3891`.
- The matching placeholder score row is
  `CPC018_HT29_6H:BRD-K84595254:10.0`, disease label `Parkinson's disease`,
  rank `48`, score `0.0506`.

Therefore downstream ranking must use resolved perturbation IDs/names, not the
raw placeholder label.

## Mapping Results

Across the 1,500 #1179 LINCS/CMap reversal score rows:

- unique perturbation IDs: 530
- resolved to real perturbation names: 461
- structure/identifier rows without a resolved common name: 66
- unmapped perturbation IDs: 3

For the `-666` placeholder-label rows:

- placeholder rows: 252
- unique placeholder perturbation IDs: 148
- unique placeholder IDs resolved to names: 112
- row-level resolved-name cases: 189
- row-level structure/identifier-only cases: 63

Resolved placeholder examples include:

| Pert ID | Resolved name | PubChem CID | Example disease label | Example rank |
|---|---|---:|---|---:|
| `BRD-K84595254` | strophanthidin | 6185 | Parkinson's disease | 48 |
| `BRD-K57080016` | selumetinib | 10127622 | type 2 diabetes mellitus | 11 |
| `BRD-A36630025` | SN-38 | 4014291 | type 2 diabetes mellitus | 23 |
| `BRD-K64606589` | apicidin | NULL | type 2 diabetes mellitus | 29 |
| `BRD-K43389675` | daunorubicin | 30323 | type 2 diabetes mellitus | 34 |
| `BRD-K94441233` | mevastatin | 64715 | type 2 diabetes mellitus | 33 |

## Calyx DB Materialization

Accepted collection: `biomed_lincs_cmap_reversal_v7`

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --metadata-root /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z \
  --collection biomed_lincs_cmap_reversal_v7 \
  --report /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z/calyx_db_readback_v7.json \
  --home /home/croyse/calyx
```

Readback report:

- `calyx_db_readback_v7.json`
- bytes: 23,012
- sha256: `189777e8514c147a709d3f4b704a88752278a302d151209482b2058e26f56186`
- exit file: `materialize_v7.exit` contained `0`

Physical Calyx readback:

- nodes written: 8,993
- edges written: 19,150
- physical node keys: 8,993
- physical outgoing edge keys: 19,150
- persisted CSR nodes: 8,993
- persisted CSR edges: 19,150
- assoc graph nodes: 8,993
- assoc graph edges: 19,150
- CSR bytes: 1,821,310
- CSR sha256: `67933e9ec687cc5ede02e2cfeb4c1109f457f97f538eed1320187e6f19963e64`
- CSR blake3:
  `e791ae34d822a8fa77178c85fe49e969836ee3e61321a4adf9e9f0d8d5605667`
- all node values were physically read back by collection-local range
- all edge keys were physically counted by collection-local range
- 512 deterministic edge values were physically read back

The aborted v5/v6 collections are not accepted state. They were written before
the rebuilt executable used range-based node-value readback. Cleanup/atomic
replacement remains tracked by #1197.

## Graph Additions

The v7 collection extends #1179 with these metadata-specific nodes and edges:

- `perturbation_metadata_row`: 530 nodes
- `placeholder_pert_desc_case_row`: 252 nodes
- `resolved_repeated_lead_row`: 487 nodes
- `resolved_drug_name`: 416 nodes
- `chemical_identifier`: 508 nodes
- `has_perturbation_metadata`: 530 edges
- `resolved_to_drug_name`: 461 edges
- `has_pubchem_id`: 513 edges
- `resolved_placeholder_to_name`: 170 edges
- `placeholder_unresolved_status`: 57 edges
- `summarizes_perturbation`: 530 edges

Representative persisted path:

```text
placeholder_pert_desc_case:CPC006_HCC515_24H:BRD-K57080016:80.0:BRD-K57080016
  -> perturbation:BRD-K57080016
  -> resolved_drug_name:selumetinib
```

## Conclusion

#1199 makes the #1179 LINCS/CMap reversal screen rankable at the perturbation
identifier level. Placeholder-like labels are now preserved as evidence rows,
resolved where authoritative metadata allows, and blocked from being treated as
standalone drug leads.

This still does not support a cure claim. It produces a better association
substrate for the next gates: rank resolved perturbations, join safety and
known-target evidence, require disease-specific counterevidence, and reject any
clinical conclusion that lacks outcome-backed support.

---

## 33_graph_collection_lifecycle_cleanup.md

# #1197 Graph Collection Lifecycle Cleanup

Status: complete for the graph lifecycle cleanup slice. This is database
hygiene for association discovery evidence; it is not a treatment, cure,
clinical recommendation, or actionability claim.

## What Changed

Calyx now has explicit graph collection generation lifecycle rows in the Aster
Graph CF. The lifecycle collection is `__calyx_graph_lifecycle`, with generation
states:

- `writing`
- `accepted`
- `failed`
- `tombstoned`

Materializers now create a `writing` generation state before graph writes and an
`accepted` generation state only after physical graph/CSR/report readback.
Maintenance can also write lifecycle rows with:

```text
calyx graph-collection-state <vault> --collection <name> --generation <id> --state <writing|accepted|failed|tombstoned> --command <name> [--reason <text>] [--detail <k=v>] [--home <dir>]
calyx graph-collection-generations <vault> [--collection <name>] [--home <dir>]
```

Default physical graph readers now fail closed when lifecycle rows exist for a
collection and none are `accepted`. Materializers use the explicit
`PhysicalPlainGraph::open_latest_unchecked` path only for their own
pre-acceptance readback.

## Real Vault FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1197-graph-lifecycle-20260704T001050Z
```

Before readback showed no lifecycle rows:

```json
{"before_counts": {}, "before_generations": 0}
```

After write/readback:

- total lifecycle generations: 10
- accepted generations: 3
- tombstoned generations: 7
- every per-row command exit file read back as `0`

Accepted collections:

- `biomed_evidence_substrate_v3`, generation `issue1196-accepted-v3`
- `biomed_lincs_cmap_reversal_v4`, generation `issue1179-accepted-v4`
- `biomed_lincs_cmap_reversal_v7`, generation `issue1199-accepted-v7`

Tombstoned collections:

- `biomed_evidence_substrate`, generation `issue1196-aborted-v1`
- `biomed_evidence_substrate_v2`, generation `issue1196-aborted-v2`
- `biomed_lincs_cmap_reversal_v1`, generation `issue1179-aborted-v1`
- `biomed_lincs_cmap_reversal_v2`, generation `issue1179-aborted-v2`
- `biomed_lincs_cmap_reversal_v3`, generation `issue1179-aborted-v3`
- `biomed_lincs_cmap_reversal_v5`, generation `issue1199-aborted-v5`
- `biomed_lincs_cmap_reversal_v6`, generation `issue1199-aborted-v6`

Artifact hashes:

- `lifecycle_all.json`: sha256
  `eb57a6b1f85f60ee7b4bdd51ada46be614926d2c059c0a44635e89b9bab85940`
- `lifecycle_summary.json`: sha256
  `8c54cd75808f13656fb05d5d1353d4aa6582a831afd09f0dee76b421337d1d3e`
- `sha256sums.txt`: sha256
  `ad8c7e079abb433de7ae4c9fb1d20979ca8e6c483323b53f52cef351a8f53915`

The 10 persisted lifecycle value hashes were:

```text
0bc1bee03a2975d5ae683e37476d1ebb52ff658cd05d18fd2b114b312f66e3f0
2fb7cf3aa820ddef672fac5fce934dacdb8c8d27ff5b3b4339635e6496530412
3b8f2f02e7fe0f63f3a8e248ed715cb55737e706defda62e6cd8bfb8a2bac28c
709f33281e6268999819070ded9253a7658f8deaac36aad8cb08282385dd7018
8b8339194dacc3a48bccf25bb131e4ccf00eed07d845048849ca101e1c1bcc1f
8e5a6067440c9ec8bc5adba4c0491f089fd6db8854eb8b80907f04a48bc03491
97c54365dac094d75d131d57575810b3ad9f0802a40e27629864f93ce13ae737
a6d5ff0dd21dcdc9801371c169bbea4e106d25264a2e39cd08e56f7f12851748
c099826c14b65be3ff89cd65b933d9ecc89eed7cc9e43bac61fba2078e75a564
e37ae188e5e5e4d05c1a96edd05cca1a7b4545a404b92581843443f289a5bad8
```

## Fail-Closed Reader Proof

Command:

```text
CALYX_HOME=/home/croyse/calyx ./target/debug/calyx materialize-graph-csr corpus-anchored-869-20260625T080546Z --collection biomed_lincs_cmap_reversal_v5
```

This read path failed before CSR materialization because v5 is tombstoned:

```json
{"code":"CALYX_GRAPH_COLLECTION_NOT_ACCEPTED","message":"graph collection biomed_lincs_cmap_reversal_v5 has lifecycle rows but no accepted generation","remediation":"mark an accepted generation after physical readback or use a different graph collection"}
```

Fail-closed artifacts:

- `tombstoned_reader_failclosed.exit`: `2`
- `tombstoned_reader_failclosed.stdout`: 0 bytes
- `tombstoned_reader_failclosed.stderr`: sha256
  `8d2624d0d6e416f8be0114e284963bae0e034a26d462044e16f310661f79008c`

## Gates

Local and aiwonder focused gates passed:

- `cargo fmt --check`
- `bash scripts/linecount.sh`
- `cargo check -p calyx-aster -p calyx-cli`
- `cargo test -p calyx-aster graph_collection_lifecycle -- --nocapture`
- `cargo test -p calyx-cli graph_collection -- --nocapture`
- `cargo test -p calyx-cli lincs_reversal -- --nocapture`
- `cargo test -p calyx-cli evidence_substrate -- --nocapture`
- `cargo test -p calyx-cli token_roundtrip -- --nocapture`

`#1198` remains the performance follow-up for batch/range edge readback APIs.

---

## 34_edge_range_readback.md

# #1198 PlainGraph Edge Range Readback

Status: complete for the collection-local edge readback slice. This is graph
storage/readback correctness work for biomedical association evidence. It is
not a treatment, cure, clinical recommendation, or actionability claim.

## What Changed

`PhysicalPlainGraph` now exposes `edge_out_props()`, which reads every outgoing
edge row for one graph collection by that collection's encoded key range. It
does not point-read every expected edge and it does not scan unrelated graph
collections.

Materializer readback now uses collection-local range scans for all expected
values:

- LINCS/CMap reversal: all node values by `node_props()`, all edge values by
  `edge_out_props()`.
- Evidence substrate: all node values by `node_props()`, all edge values by
  `edge_out_props()`.

`PlainGraph::rebuild_csr` is documented as a collection-local projection: it
uses this `PlainGraph` instance's node and outgoing-edge key ranges.

## Unit Proof

Test:

```text
cargo test -p calyx-aster physical_edge_out_props -- --nocapture
```

The test creates `target` and `unrelated` graph collections in one vault, writes
one outgoing edge to each, flushes, and proves `PhysicalPlainGraph::edge_out_props()`
for `target` returns only the `target` edge value.

## Real Vault FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1198-edge-range-readback-20260704T002330Z
```

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --metadata-root /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z \
  --collection biomed_lincs_cmap_reversal_v8 \
  --report /home/croyse/calyx/fsv/issue1198-edge-range-readback-20260704T002330Z/calyx_db_readback_v8.json \
  --home /home/croyse/calyx
```

Exit:

```text
0
```

Report:

- `calyx_db_readback_v8.json`: sha256
  `8281d40dc70b5e4beea6dd7b37b4ef8f6aa281bf17108adadd65252ae68822cf`
- `materialize_v8.stdout`: sha256
  `3bc3c5b5d12fb07b2f18efe3d95203a0d15e13f4d026ac4bf4460c15ece4b0df`
- `materialize_v8.stderr`: sha256
  `ef687b619538b54d642647fca9a49bd8287fb8aaddda9ac9ea4543899c74f09d`
- `sha256sums.txt`: sha256
  `04981eecf94ad3c7e596aaed788c9087438e3a3b2f9214590ae1654356560c00`

Readback summary:

```json
{
  "collection": "biomed_lincs_cmap_reversal_v8",
  "graph_generation": "materialize-lincs-reversal-01KWN8RH7MRVJDY9TK9ENHF7BR",
  "nodes": 8993,
  "edges": 19150,
  "physical_edge_out_keys": 19150,
  "edge_value_readback_mode": "all physical edge values read back by collection range",
  "sampled_edge_values_read_back": 19150,
  "all_edge_values_read_back": true,
  "csr_bytes": 1821310,
  "csr_sha256": "57084512f9cbb83829db2b4f8d1d323a0f4d61750b324fb6cebbe689b987390f"
}
```

Lifecycle readback for `biomed_lincs_cmap_reversal_v8`:

- status: `accepted`
- generation: `materialize-lincs-reversal-01KWN8RH7MRVJDY9TK9ENHF7BR`
- value sha256:
  `1f1385e60502cee3f3f11fa2ffff31a15325ffffb59966428b19c9aeccd24ae0`
- lifecycle detail includes report path, node rows `8993`, edge rows `19150`,
  and CSR sha256 `57084512f9cbb83829db2b4f8d1d323a0f4d61750b324fb6cebbe689b987390f`.

## Conclusion

The old LINCS readback mode counted all edge keys and sampled 512 edge values.
The accepted v8 materialization read back all 19,150 edge values from the
collection-local physical Graph CF range. This closes the remaining #1198 edge
readback gap for the current biomedical materializers.

---

## 37_metabolic_cardiovascular_hunt.md

# #1186 Metabolic / Cardiovascular Association Hunt

## Scope

#1186 composes the typed association miner, falsification sweep, validation
sources, safety triage, DGIdb target evidence, Open Targets context, and live
ClinicalTrials/openFDA probes into a bounded metabolic/cardiovascular/renal
hypothesis bundle.

The domain filter includes diabetes/metabolic/glucose/insulin/obesity,
hypertension, kidney/renal/proteinuria, cardiovascular/heart/coronary/myocardial,
atherosclerosis/stroke/thrombotic/vascular, blood pressure, cholesterol, and
lipid terms.

Metformin/DPP4 rows are carried as known proof-slice context only unless a new
validated hypothesis clears later gates. This slice found DPP4-related Open
Targets context for proteinuria/hypertension, but does not convert that context
into a treatment, actionability, safety, efficacy, or cure claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z
```

Live query sources used during this FSV:

- openFDA drug label API: `https://open.fda.gov/apis/drug/label/`
- openFDA drug adverse event API / FAERS: `https://open.fda.gov/apis/drug/event/`
- ClinicalTrials.gov Data API: `https://clinicaltrials.gov/data-api/about-api`

Persisted source inputs:

| Role | Bytes | SHA-256 |
|---|---:|---|
| `typed_chemical_disease` | 45,226 | `ba1d310bdb38d2cefc654b0faab45252ce7223c9f6b9e68e743b33b78492828b` |
| `typed_gene_disease` | 10,721 | `845a2609eec7a392a3ccde10d8756a549eb91a71c9eaa96cee77874afff04518` |
| `typed_broad` | 307,326 | `99394214a3147d34828dc2830b622b90ae434cadc407c89a57e3c57ae9144d15` |
| `falsification_flags` | 180,225 | `9d80c503a5173e8a3056101c132b1b299905e801a634d87aabf5bcab862e3e77` |
| `clinicaltrials_rows` | 424,800 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| `dgidb_interactions_tsv` | 12,178,745 | `08af778126a4f22a10fddb7fe06745df07f068ce76c016fe22c59c524ef3de9c` |
| `dgidb_seed_interactions` | 63,220 | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| `open_targets_rows` | 1,189,226 | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| `association_validation_report` | 175,977 | `7fb0aad1c7f66bea4c86c5d6d99084f1c2203769a494c6f6d58ff0071d0bf2c3` |
| `prior_safety_terms` | 33,114 | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` |

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 3,522 | `81c1b261324f838e48de588b8447b01a6b95c3cd08b9e47a9eabbb6b36e8dd5d` | - |
| `out/validation_metrics.json` | 628 | `6eaa5e8a49a94c9bdd4208db14636f52da5839cb9699f5d8210762b6c2aa3d48` | - |
| `out/run_summary.json` | 1,451 | `322a111d994608397a4b03818393fc82cf9d252489fedfb5d8261d0dcc51312b` | - |
| `out/metabolic_cardiovascular_hypotheses.jsonl` | 139,547 | `1a53daf6d93b2ce2f0d28235b679a7cd1427c0a44070285840365c6397f7eb94` | 35 |
| `out/top_evidence_bundles.json` | 85,106 | `cd8acabf0c1aa108f5674f8ff686c5a10136656cb2a8ce89cd47a3490dd344c7` | - |
| `out/safety_term_flags.jsonl` | 12,172 | `49679aeaf6c3f23c13898f0e1eb89f412eae69117c76b987c40a39e190743531` | 21 |
| `out/trial_pair_flags.jsonl` | 33,979 | `fbf745880c29259a106a3e6196e5b4a91429b7f1d223a792c4f81a298d94bb0a` | 26 |
| `out/dgidb_target_evidence.jsonl` | 57,280 | `fe009e9087a016fada2ced025513e7c2609bbc3105cd0abcb4188017f7115621` | 21 |
| `out/open_targets_context.jsonl` | 1,875 | `42cff6b2e61982c552c7f4229c1b3494abc1d33c7165ff1737feb6513e0cabec` | 2 |
| `out/raw_query_manifest.jsonl` | 14,322 | `dad7f0d3ab2bb1eb42fcc8cd7d7d67bf993cedba8cfbcc801f7d9716097685d4` | 47 |
| `out/output_manifest.json` | 11,901 | `1a593be2d43176702f287b73fd54762d6667b0c66f3f6d816ead2e68aac34500` | - |
| `out/persisted_readback.json` | 5,537 | `45d3284ff630ce939b7db8b161063f3c86fb33af2dcc046cb0a7a3b1919e1007` | - |

Raw live query responses are under:

```text
/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/raw/clinicaltrials
/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/raw/openfda_safety
```

## Metrics

| Metric | Count |
|---|---:|
| Typed chemical-domain candidates | 20 |
| Typed gene-domain candidates | 7 |
| Known bridge context rows | 6 |
| Open Targets mapped context rows | 2 |
| Total output hypotheses/context rows | 35 |
| Drug terms queried for safety | 21 |
| Drug terms with label coverage | 17 |
| Drug terms with FAERS/openFDA coverage | 20 |
| ClinicalTrials pairs queried | 26 |
| ClinicalTrials pairs with hits | 18 |
| Candidate rows with falsification counter | 1 |
| Candidate rows with safety source unavailable | 6 |

Fail-closed safety gaps:

| Drug term | Label | FAERS/openFDA events | Flag |
|---|---|---|---|
| Leukotrienes | missing | missing | source unavailable fail-closed |
| Steroids | missing | present | label unavailable fail-closed |
| Vildagliptin | missing | present | label unavailable fail-closed |
| zopiclone | missing | present | label unavailable fail-closed |

ClinicalTrials pairs with no hits in the bounded query:

| Drug term | Disease |
|---|---|
| Leukotrienes | Proteinuria |
| Omeprazole | Proteinuria |
| Phenytoin | Thrombocytopenia |
| Quinidine | Thrombocytopenia |
| Streptomycin | Kidney Diseases |
| Theophylline | Proteinuria |
| Zolpidem | Proteinuria |
| zopiclone | Proteinuria |

## Top Readback Rows

| Rank | Candidate | Type | Drug | Disease | Target | Score | Falsification | Boundary |
|---:|---|---|---|---|---|---:|---|---|
| 1 | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | target-disease | none | Proteinuria | TNF | 6.8026 | no counter found in current sources | target context only |
| 2 | `typed-assoc:concept:ncbi_gene:920::concept:ncbi_mesh:D011507` | target-disease | none | Proteinuria | CD4 | 6.5142 | no counter found in current sources | target context only |
| 3 | `typed-assoc:concept:ncbi_gene:925::concept:ncbi_mesh:D011507` | target-disease | none | Proteinuria | CD8A | 5.4044 | no counter found in current sources | target context only |
| 4 | `typed-assoc:concept:ncbi_mesh:D013256::concept:ncbi_mesh:D011507` | drug-disease | Steroids | Proteinuria | IVL | 4.9409 | no counter found in current sources | label gap blocks promotion |
| 5 | `typed-assoc:concept:ncbi_mesh:D009543::concept:ncbi_mesh:D006973` | drug-disease | Nifedipine | Hypertension | SLC14A2 | 4.7986 | no counter found in current sources | safety/trial review required |
| 6 | `typed-assoc:concept:ncbi_mesh:D014700::concept:ncbi_mesh:D006973` | drug-disease | Verapamil | Hypertension | ODC1 | 4.7986 | no counter found in current sources | safety/trial review required |
| 7 | `known-bridge:metabolic:type2-diabetes:alogliptin` | known bridge | Alogliptin | Type 2 Diabetes Mellitus | DPP4 | 4.4 | known context only | not a new repurposing claim |
| 8 | `known-bridge:metabolic:type2-diabetes:linagliptin` | known bridge | Linagliptin | Type 2 Diabetes Mellitus | DPP4 | 4.4 | known context only | not a new repurposing claim |
| 9 | `known-bridge:metabolic:type2-diabetes:saxagliptin` | known bridge | Saxagliptin | Type 2 Diabetes Mellitus | DPP4 | 4.4 | known context only | not a new repurposing claim |
| 10 | `known-bridge:metabolic:type2-diabetes:sitagliptin` | known bridge | Sitagliptin | Type 2 Diabetes Mellitus | DPP4 | 4.4 | known context only | not a new repurposing claim |
| 11 | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D007674` | target-disease | none | Kidney Diseases | TNF | 4.2945 | no counter found in current sources | target context only |
| 12 | `known-bridge:metabolic:type2-diabetes:vildagliptin` | known bridge | Vildagliptin | Type 2 Diabetes Mellitus | DPP4 | 4.0 | known context only | label gap blocks promotion |

The highest-ranked rows are association/target context, not drug intervention
claims. The best drug-disease rows are Steroids/Proteinuria,
Nifedipine/Hypertension, and Verapamil/Hypertension; all carry safety/trial
review flags and remain hypotheses.

## Conclusion

#1186 is complete for the current bounded metabolic/cardiovascular/renal hunt:

- typed graph candidates, Open Targets target context, DGIdb target evidence,
  safety flags, trial flags, and falsification state are persisted together;
- metformin/DPP4 rows are explicitly known proof-slice context;
- missing label/trial/source coverage fails closed;
- output rows are ranked worklist items for human and external validation, not
  clinical guidance or cure claims.

---

## 38_neuro_hunt.md

# #1187 Neurodegeneration / Neuropsychiatric Association Hunt

## Scope

#1187 composes the current Calyx biomedical association substrate into a bounded
neurodegeneration/neuropsychiatric evidence pack. The run uses:

- #1183 typed all-pair hypotheses.
- #1184 falsification flags for the original typed hypotheses.
- #1171 CxId source expansion and #1172/#1173 concept normalization/typed overlay.
- #1174 Open Targets target-disease validation rows.
- #1178 DGIdb drug-gene rows.
- #884/#994/#1175 clinical/molecular bridge rows for metformin/DPP4.
- Live bounded ClinicalTrials.gov and openFDA probes for selected drug-bearing
  rows.

The output is a ranked research-lead atlas. It is not a treatment, actionability,
efficacy, safety, or cure claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z
```

Live query sources used during this FSV:

- ClinicalTrials.gov Data API: `https://clinicaltrials.gov/api/v2/studies`
- openFDA drug label API: `https://api.fda.gov/drug/label.json`
- openFDA drug adverse event API: `https://api.fda.gov/drug/event.json`

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 5,500 | `24b1d68c9cce9b8ee3140aa5a47d45d7563eaef4708508da15f773412a97b526` | - |
| `out/neuro_normalized_annotations.jsonl` | 192,126 | `af99b6c41539374363d5f6fb0664a4c91205caba8cd0f740bfae0fe446816a74` | 222 |
| `out/neuro_unresolved_terms.jsonl` | 30,850 | `0af0314384a4da16b897a5c3a69d5a6835d1001ce2c5f69a19f38a2ebbe68462` | 70 |
| `out/neuro_hypotheses.jsonl` | 362,520 | `3d782f891933797a3ec096e693e38570cd93d386414f6d19f6fdac2f48a601b8` | 77 |
| `out/neuro_hypotheses.json` | 518,981 | `b27f2b2e9fd67f4c2e1e631d2d8e79ef22d5e6ae64912c25bee9a97f55eb6f5b` | - |
| `out/top_evidence_bundles.json` | 87,680 | `58f65ef7ce9c7a163d92c1d6ebde7dbe1d1853f9712948b00c28330844210545` | 20 bundles |
| `out/external_validation_context.jsonl` | 39,919 | `b1944c4391ad440c953a0e6baa99f89f3dff0ad17e96644b0f3837a86d76d83b` | - |
| `out/raw_query_manifest.jsonl` | 20,358 | `6e5cf2ea4cac30fcfffd77fbff4ca60f2fa59f8ed4ab8fdf101491f0466996f9` | 42 |
| `out/safety_trial_flags.jsonl` | 24,113 | `eb68c9ecd77282cc3e6f47c619349d4e003014b19ef0e22fb1a3d45da2ab9610` | 14 |
| `out/validation_metrics.json` | 1,323 | `0dd0d48c71c15e69a373624ef7422e0e978f94b88b56b70ccff5ebdfe1a62d96` | - |
| `out/persisted_readback.json` | - | `9dfa536723b86435a0ff2af21e2af4ad87ecbb9aecf06e15f42ee2609981b43b` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 77 |
| Metrics hypothesis total | 77 |
| Row count matches metrics | true |
| Top bundle count | 20 |
| Raw live query rows | 42 |
| Safety/trial flag rows | 14 |
| Normalized neuro annotation rows | 222 |
| Unresolved neuro rows | 70 |

## Metrics

| Metric | Count |
|---|---:|
| Typed rows scanned | 301 |
| Falsification flags loaded | 280 |
| Source-expanded rows loaded | 2,612 |
| Normalized annotations loaded | 2,575 |
| Normalized neuro annotations emitted | 222 |
| Unresolved neuro terms emitted | 70 |
| Open Targets rows loaded | 1,422 |
| Open Targets neuro rows considered | 60 |
| DGIdb interactions loaded | 359 |
| DPP4 interactions loaded | 41 |
| Total ranked hypotheses | 77 |
| Drug-bearing hypotheses | 23 |
| Live drug pair triage queries | 14 |
| Live API artifacts | 42 |

Hypothesis class counts:

| Class | Rows |
|---|---:|
| Open Targets target-neuro disease rows | 30 |
| Typed all-pair neuro-filter rows | 19 |
| DGIdb drug-target/Open Targets neuropsychiatric bridges | 12 |
| Same-source disease-to-neuro disease clusters | 11 |
| Same-source drug/gene/variant-to-neuro disease co-mentions | 4 |
| Clinical/molecular metformin-DPP4-schizophrenia bridge | 1 |

## Top Readback Rows

| Rank | Candidate | Type | Source | Bridge | Target | Score | Evidence |
|---:|---|---|---|---|---|---:|---|
| 1 | `issue1187:opentargets_target_neuro:03088f07d62fb884ac77` | target-disease | NF1 | - | neurofibromatosis type 1 | 0.953413184 | Open Targets |
| 2 | `issue1187:disease_neuro_cluster:259cdcce4807d7a52fde` | disease-neuro cluster | Proteinuria | - | Diffuse Neurofibrillary Tangles with Calcification | 0.95 | source-expanded normalized co-mentions |
| 3 | `issue1187:opentargets_target_neuro:1ef096aa80d38fbf2dc8` | target-disease | NF1 | - | neurofibromatosis-Noonan syndrome | 0.919166479 | Open Targets |
| 4 | `issue1187:opentargets_target_neuro:de90f09ee7af73f6267c` | target-disease | KIF11 | - | microcephaly | 0.903947407 | Open Targets |
| 5 | `issue1187:opentargets_target_neuro:a129f458c7674c4a4146` | target-disease | NF1 | - | neurofibromatosis | 0.902600114 | Open Targets |
| 6 | `issue1187:opentargets_target_neuro:857be828412ccb2587ed` | target-disease | PCNT | - | microcephaly | 0.902032109 | Open Targets |
| 7 | `issue1187:opentargets_target_neuro:ce2dd05db8308fc92f7c` | target-disease | WDR62 | - | microcephaly | 0.899856982 | Open Targets |
| 8 | `issue1187:opentargets_target_neuro:c6f8a60228f1e781557e` | target-disease | MCPH1 | - | microcephaly | 0.898600761 | Open Targets |
| 9 | `issue1187:opentargets_target_neuro:b839e5846eea84a6d0a1` | target-disease | CDK5RAP2 | - | microcephaly | 0.89210711 | Open Targets |
| 10 | `issue1187:opentargets_target_neuro:1bb3ac171377a794830a` | target-disease | NDE1 | - | microcephaly | 0.891178722 | Open Targets |
| 12 | `issue1187:disease_neuro_cluster:057d66318046c6764d5c` | disease-neuro cluster | Meningitis Bacterial | - | Subarachnoid Hemorrhage | 0.888351894 | 5 source CxIds |
| 23 | `issue1187:dpp4_inhibitor_schizophrenia:saxagliptin_anhydrous` | drug-target-disease bridge | Saxagliptin Anhydrous | DPP4 | schizophrenia | 0.874142023 | DGIdb + Open Targets + live safety/trial |
| 34 | `issue1187:molecular_bridge:metformin_dpp4_schizophrenia` | drug-target-disease bridge | Metformin | DPP4 | schizophrenia | 0.834142023 | #884/#994/#1175 + Open Targets + live safety/trial |
| 41 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D013345::concept:ncbi_mesh:D016920` | typed disease association | Subarachnoid Hemorrhage | - | Meningitis Bacterial | 0.782106343 | #1183 typed paths + #1184 status |
| 42 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D014012::concept:ncbi_mesh:D014717` | typed disease association | Tinnitus | - | Vertigo | 0.782106343 | #1183 typed paths + #1184 status |
| 43 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D010040::concept:ncbi_mesh:D014012` | typed disease association | Otosclerosis | - | Tinnitus | 0.727961544 | #1183 typed paths + #1184 status |
| 44 | `issue1187:typed:typed-assoc:concept:ncbi_mesh:D011507::concept:ncbi_mesh:D055956` | typed disease association | Proteinuria | - | Diffuse Neurofibrillary Tangles with Calcification | 0.727961544 | #1183 typed paths + #1184 status |

The top global ranks are dominated by target-disease rows from Open Targets and
known neurogenetic/microcephaly signals. These are useful prioritization rows,
not new treatment claims.

## Drug-Bearing Readback

| Rank | Candidate | Drug | Bridge | Disease | Trial/safety status | Notes |
|---:|---|---|---|---|---|---|
| 23 | `issue1187:dpp4_inhibitor_schizophrenia:saxagliptin_anhydrous` | Saxagliptin Anhydrous | DPP4 | schizophrenia | live triage complete | DGIdb DPP4 interaction + Open Targets DPP4-schizophrenia |
| 34 | `issue1187:molecular_bridge:metformin_dpp4_schizophrenia` | Metformin | DPP4 | schizophrenia | live triage complete | BindingDB/DPP4 bridge + Open Targets score 0.1921893635 |
| 36 | `issue1187:dpp4_inhibitor_schizophrenia:anagliptin` | Anagliptin | DPP4 | schizophrenia | live triage complete | DGIdb DPP4 bridge |
| 37 | `issue1187:dpp4_inhibitor_schizophrenia:bisegliptin` | Bisegliptin | DPP4 | schizophrenia | live triage complete | no openFDA label/event hit in bounded query |
| 38 | `issue1187:dpp4_inhibitor_schizophrenia:omarigliptin` | Omarigliptin | DPP4 | schizophrenia | live triage complete | no openFDA label hit; event query had 31 rows |
| 39 | `issue1187:dpp4_inhibitor_schizophrenia:prusogliptin` | Prusogliptin | DPP4 | schizophrenia | live triage complete | no openFDA label/event hit in bounded query |
| 40 | `issue1187:dpp4_inhibitor_schizophrenia:valacyclovir` | Valacyclovir | DPP4 | schizophrenia | live triage complete | DGIdb row exists but bridge is mechanistically ambiguous |
| 54 | `issue1187:normalized_comention:833636615f7503375d50` | Quinidine | - | Tinnitus | live triage complete | source co-mention; not causality |
| 56 | `issue1187:normalized_comention:17b6c746f037502ed3cb` | Kanamycin | - | Tinnitus | pending broader triage | source co-mention; not causality |
| 57 | `issue1187:normalized_comention:bb3f9dfbe7f9d6830d9b` | Streptomycin | - | Tinnitus | pending broader triage | source co-mention; not causality |
| 59 | `issue1187:normalized_comention:b794231dd43a99a3a80e` | Phenytoin | - | Tinnitus | pending broader triage | source co-mention; not causality |

Live triage examples:

- Metformin/schizophrenia ClinicalTrials query returned 10 first-page studies,
  including schizophrenia/metabolic-syndrome contexts. openFDA label query
  returned 5 label rows and an event query total of 425,794. This is safety/trial
  context, not efficacy.
- Quinidine/tinnitus live triage returned 0 ClinicalTrials hits, 5 openFDA label
  rows, and 4,273 event rows in the bounded query.

## Falsification Status

- #1183 typed rows carry #1184 falsification status where the hypothesis id was
  present.
- Generated #1187 rows created after #1184 are marked explicitly as
  `not_run_for_generated_*` or `external_validation_row_not_falsification_sweep`.
- This is not hidden: #1223 now tracks a full counter-evidence sweep for
  generated disease-hunt candidates before atlas promotion.

## Gaps Split Out

The FSV found real follow-up work:

- #1222 - repair unresolved neuro concept normalization after #1187.
- #1223 - falsify generated disease-hunt candidates across domains.
- #1224 - expand neuropsychiatric target druggability evidence beyond the
  bounded DPP4 slice.

## Conclusion

#1187 is complete for a bounded, persisted neurodegeneration/neuropsychiatric
association hunt:

- it produced 77 ranked rows with normalized names where available, evidence
  paths, validation context, safety/trial flags for selected drug-bearing rows,
  and explicit falsification status;
- it read back all persisted output counts and hashes from aiwonder artifacts;
- it surfaced DPP4/metformin/schizophrenia as a weak/moderate molecular bridge
  research lead, not an efficacy or cure claim;
- it surfaced known neurogenetic target-disease rows and source-backed
  disease/drug co-mentions as prioritization worklist items only.

No clinical recommendation, treatment claim, safety claim, actionability claim,
or cure claim is made by this artifact.

---

## 39_infectious_immunology_hunt.md

# #1188 Infectious / Immunology / Inflammation Association Hunt

## Scope

#1188 composes the current Calyx biomedical association substrate into a
bounded infectious disease, immunology, and inflammation evidence pack. The run
uses:

- #1183 typed all-pair hypotheses.
- #1184 falsification flags for the original typed hypotheses.
- #1171 CxId source expansion and #1172 concept normalization.
- #1174 Open Targets target-disease validation rows.
- #1178 DGIdb drug-gene rows.
- #1177 ClinicalTrials and #1181 openFDA safety context.
- Live bounded ClinicalTrials.gov and openFDA probes for selected drug-bearing
  rows.

The output is a ranked research-lead atlas. It separates antimicrobial,
antiviral, immunomodulatory, host-target, and disease-cluster rows, but it is
not a treatment, actionability, efficacy, safety, or cure claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z
```

Live query sources used during this FSV:

- ClinicalTrials.gov Data API: `https://clinicaltrials.gov/api/v2/studies`
- openFDA drug label API: `https://api.fda.gov/drug/label.json`
- openFDA drug adverse event API: `https://api.fda.gov/drug/event.json`

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 3,591 | `38df137252ee13faefcdcd330140008b3e82ae384687c80f73133a0fc6fd0a4e` | - |
| `out/infectious_immunology_normalized_annotations.jsonl` | 403,902 | `7eac529c14b017216ea34ec05caf9445b4898517f432d07e8d13bbd7a1913bff` | 475 |
| `out/infectious_immunology_unresolved_terms.jsonl` | 69,305 | `19bc15070732f752a1fdf90552de7be4705bf8e5f236735b33fb20d5e844c90e` | 151 |
| `out/infectious_immunology_hypotheses.jsonl` | 943,059 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` | 209 |
| `out/infectious_immunology_hypotheses.json` | 1,323,474 | `67866b6d559c5537827a71f5980b319858baaf36337da5ab07f02d1b824eefab` | - |
| `out/top_evidence_bundles.json` | 100,048 | `ebe017a88af380f77e701dfb8d69e3e042b42f27f26d9ff9ebc090dcd2590c02` | 24 bundles |
| `out/external_validation_context.jsonl` | 48,980 | `4edb478a67fd6f3b13be53d45a10844a8ac3404a0cdba15011d74578ba5016dd` | 45 |
| `out/raw_query_manifest.jsonl` | 23,136 | `14fa399117e7ce09b59d58ad693167597c3fb1a7ba38208eba5392b4b5f91c0a` | 48 |
| `out/safety_trial_flags.jsonl` | 37,749 | `73116d59406dbd2d506900493048d6f37c18131458d4c8bca114143f737a5d9e` | 16 |
| `out/validation_metrics.json` | 1,720 | `d7c3dc2c5b441d884c1fbc39b7c56ec45f6434f4277329d65a7a615f1e02e104` | - |
| `out/persisted_readback.json` | - | `972ac09649ba07728509ff0efb3563e2001b3e03a7c6476584751c52df355f97` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 209 |
| Metrics hypothesis total | 209 |
| Row count matches metrics | true |
| Top bundle count | 24 |
| Raw live query rows | 48 |
| Safety/trial flag rows | 16 |
| Domain normalized annotation rows | 475 |
| Domain unresolved rows | 151 |
| Required families present | antimicrobial, antiviral, host-target, immunomodulatory |

## Metrics

| Metric | Count |
|---|---:|
| Typed rows scanned | 301 |
| Falsification flags loaded | 280 |
| Source-expanded rows loaded | 2,612 |
| Normalized annotations loaded | 2,575 |
| Infectious/immunology normalized annotations emitted | 475 |
| Infectious/immunology unresolved terms emitted | 151 |
| Open Targets rows loaded | 1,422 |
| Open Targets domain rows considered | 100 |
| DGIdb interactions loaded | 359 |
| DGIdb domain bridges considered | 486 |
| Total ranked hypotheses | 209 |
| Drug-bearing hypotheses | 120 |
| Live drug pair triage queries | 16 |
| Live API artifacts | 48 |

Hypothesis class counts:

| Class | Rows |
|---|---:|
| DGIdb drug-target/Open Targets infectious-immune bridges | 51 |
| Open Targets target-infectious-immune rows | 45 |
| Typed all-pair infectious/immunology filter rows | 93 |
| Same-source disease-to-infectious-immune clusters | 14 |
| Same-source drug/gene/variant-to-infectious-immune disease co-mentions | 6 |

Hypothesis family counts:

| Family | Rows |
|---|---:|
| Host-target | 101 |
| Immunomodulatory | 68 |
| Antiviral | 15 |
| Antimicrobial / antibacterial | 4 |
| Pathogen-disease / immune phenotype cluster | 17 |
| Infectious association | 4 |

## Top Readback Rows

| Rank | Candidate | Family | Source | Bridge | Target | Score | Evidence |
|---:|---|---|---|---|---|---:|---|
| 1 | `issue1188:dgidb_target_bridge:30b8f13223f01d85c9bf` | host-target | Golimumab | TNF | psoriatic arthritis | 1.396525469 | DGIdb + Open Targets + live safety/trial |
| 2 | `issue1188:dgidb_target_bridge:479d5510a18445362cc7` | host-target | Certolizumab Pegol | TNF | psoriatic arthritis | 1.396525469 | DGIdb + Open Targets + live safety/trial |
| 3 | `issue1188:dgidb_target_bridge:42b707a342622ba8bf02` | antiviral | Tregalizumab | CD4 | HIV infectious disease | 1.277564915 | DGIdb + Open Targets + live safety/trial |
| 4 | `issue1188:dgidb_target_bridge:1c0540ae0f79f3820461` | host-target | Placulumab | TNF | psoriatic arthritis | 1.266525469 | DGIdb + Open Targets + live safety/trial |
| 5 | `issue1188:typed:typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | host-target | Tnf | - | Proteinuria | 1.25 | #1183 typed path |
| 6 | `issue1188:dgidb_target_bridge:a41ed87e4f0aa4b31e45` | host-target | Cefotaxime Sodium | TNF | psoriatic arthritis | 1.246525469 | DGIdb + Open Targets + live safety/trial |
| 7 | `issue1188:dgidb_target_bridge:2b638360564c97b39fd4` | host-target | Infliximab | IL12B | psoriasis | 1.238773092 | DGIdb + Open Targets + live safety/trial |
| 8 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D009241::concept:ncbi_mesh:D013256` | immunomodulatory | Ipratropium | - | Steroids | 1.237570627 | #1183 typed path |
| 9 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D013256::concept:ncbi_mesh:D013806` | immunomodulatory | Steroids | - | Theophylline | 1.237570627 | #1183 typed path |
| 10 | `issue1188:typed:typed-assoc:concept:ncbi_gene:920::concept:ncbi_mesh:D011507` | host-target | CD4 | - | Proteinuria | 1.204242509 | #1183 typed path |
| 11 | `issue1188:dgidb_target_bridge:1dca49f5a02e1fe7373e` | antiviral | Zanolimumab | CD4 | HIV infectious disease | 1.197287372 | DGIdb + Open Targets + live safety/trial |
| 12 | `issue1188:dgidb_target_bridge:6bf557e90eb23401a00f` | antiviral | Herbimycin | CD4 | HIV infectious disease | 1.197287372 | DGIdb + Open Targets + live safety/trial |
| 14 | `issue1188:dgidb_target_bridge:4b6698e147aa815408f8` | antiviral | Antiviral Agent | CD4 | HIV infectious disease | 1.177287372 | DGIdb + Open Targets + live safety/trial |
| 15 | `issue1188:dgidb_target_bridge:679e64b9e10db4b5fd76` | antiviral | Ibalizumab | CD4 | HIV infectious disease | 1.177287372 | DGIdb + Open Targets + live safety/trial |
| 71 | `issue1188:typed:typed-assoc:concept:ncbi_mesh:D013307::concept:ncbi_mesh:D007710` | antimicrobial | Streptomycin | - | Klebsiella Infections | 0.98248676 | #1183 typed path |
| 73 | `issue1188:normalized_comention:335220c5511f34b4eb5e` | antimicrobial | Streptomycin | - | Rhinoscleroma | 0.947258872 | source-expanded normalized co-mentions |
| 80 | `issue1188:disease_domain_cluster:8d3ae8eb6d72caa4023e` | disease cluster | Thalassemia | - | Salmonella Infections | 0.905018561 | source-expanded normalized co-mentions |

The top ranks are dominated by known-positive/calibration immunology rows:
TNF/psoriatic-arthritis and CD4/HIV target bridges validate that the pipeline
can recover established biomedical structure. They are not new treatment claims
and they currently crowd out novelty-oriented worklists, which is tracked in
#1226.

## Family Examples

| Family | Example rows |
|---|---|
| Antimicrobial / antibacterial | Streptomycin/Klebsiella Infections; Streptomycin/Rhinoscleroma |
| Antiviral | Tregalizumab/CD4/HIV infectious disease; Antiviral Agent/CD4/HIV infectious disease; Ibalizumab/CD4/HIV infectious disease |
| Immunomodulatory | Ipratropium/Steroids; Steroids/Theophylline; Prednisolone/Steroids; montelukast/Steroids |
| Host-target | Golimumab/TNF/psoriatic arthritis; Infliximab/IL12B/psoriasis; TNF/Proteinuria; CD4/Proteinuria |
| Pathogen/phenotype cluster | Rhinoscleroma/Rhinosporidiosis; Rhinosporidiosis/Klebsiella Infections; Thalassemia/Salmonella Infections |

## Falsification Status

- #1183 typed rows carry #1184 falsification status where the hypothesis id was
  present.
- Generated #1188 co-mention and DGIdb/Open Targets bridge rows were created
  after #1184 and are marked explicitly as `not_run_for_generated_*` or
  `external_validation_row_not_falsification_sweep`.
- #1223 tracks the cross-domain falsification sweep for generated disease-hunt
  candidates before atlas promotion.

## Gaps Split Out

The FSV found real follow-up work:

- #1225 - repair unresolved infectious/immunology concept normalization after
  #1188. The readback artifact contains 151 unresolved domain terms, including
  HLA-B27, bronchial asthma, acute asthma, Tuberculosis, CD40, Sepsis,
  Leukotriene, Lupus vulgaris, Malaria, and Influenza vaccine.
- #1226 - split known-positive calibration rows from novelty-prioritized
  disease-hunt rankings. #1188 recovered strong known immunology structure, but
  the global top-k is not yet optimized for novel association triage.
- #1223 - falsify generated disease-hunt candidates across domains.

## Conclusion

#1188 is complete for a bounded, persisted infectious/immunology/inflammation
association hunt:

- it produced 209 ranked rows with normalized names where available, evidence
  paths, validation context, safety/trial flags for selected drug-bearing rows,
  explicit family labels, and explicit falsification status;
- it read back all persisted output counts and hashes from aiwonder artifacts;
- it separated antimicrobial, antiviral, immunomodulatory, and host-target
  hypotheses as required;
- it exposed normalization and novelty-ranking gaps as new atomic GitHub issues.

No clinical recommendation, treatment claim, safety claim, actionability claim,
or cure claim is made by this artifact.

---

## 40_rare_disease_hunt.md

# #1189 Rare-Disease Phenotype / Gene / Drug Association Hunt

## Scope

#1189 composes public rare-disease phenotype data with the current Calyx
biomedical association substrate. The run uses:

- HPO ontology and HPOA rare-disease annotations from official HPO PURLs.
- Mondo disease ontology from the official Mondo PURL.
- The existing Calyx DB evidence substrate readback from #1196.
- #1183 typed all-pair hypotheses and #1184 falsification flags.
- #1174 Open Targets rows and #1178/#1224 DGIdb drug-target evidence.
- A bounded live DGIdb GraphQL pass for the top rare-disease genes lacking
  enough local target evidence.

The output is a ranked research-lead worklist. It is not a treatment,
efficacy, safety, clinical-actionability, dosing, recommendation, or cure
claim.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z
```

Command:

```bash
cd /home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z
python3 issue1189_rare_disease_hunt.py \
  /home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z \
  --max-drug-candidates 900 \
  --max-target-candidates 600 \
  --live-dgidb-target-limit 100 \
  --dgidb-page-size 100 \
  --dgidb-max-records-per-target 100
```

Native Calyx DB materialization:

```bash
cd /home/croyse/calyx/repo
./target/release/calyx materialize-bridge-corpus \
  issue1189-rare-disease-bridge-20260704t114953z \
  --rows /home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z/out/rare_disease_bridge_corpus_rows.jsonl \
  --home /home/croyse/calyx
```

## Source Inputs

HPOA metadata read back from `out/input_scope.json`:

```text
description: HPO annotations for rare diseases [8574: OMIM; 47: DECIPHER; 4337 ORPHANET]
version: 2026-06-23
hpo-version: http://purl.obolibrary.org/obo/hp/releases/2026-06-23/hp.json
```

Downloaded source hashes:

| Source | Bytes | SHA-256 |
|---|---:|---|
| HPO `hp.obo` | 11,222,341 | `a5092cbdf605f568403cf7380d9173014015692433b2cc631bc5c1b053876b1b` |
| HPO `phenotype.hpoa` | 35,672,303 | `89004f85b253f980ffe84218d2c080665cbf67a57bbb322111d6a2db5eb31dff` |
| HPO `genes_to_phenotype.txt` | 20,732,778 | `26cb7ee00c73b5777f6e5ad43323c941e1fcef1d191592f332d7929f3ea1ab3f` |
| Mondo `mondo.obo` | 51,977,002 | `041d20436ca78e23f38d2b37793e684d17a67480e0bf19dfa2c0ecedf00a8712` |
| #1196 Calyx DB evidence substrate readback | 8,323 | `19d4e52153b280a7c630bd865b04b7db6ed41fd2a50bfffd6995a202ec55df1a` |

## Output Readback

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `out/rare_disease_phenotype_inputs.jsonl` | 12,956 | 43,605,462 | `c62fe4d5be37f8cb242479feade2272521c32474eb3e2e7481bb8bc89f14e6ff` |
| `out/rare_disease_gene_phenotype_links.jsonl` | 12,654 | 50,483,293 | `00b91b3d331417a65bac8d842aabf2e0adac2f21ffddff8291ed5cbdf5afbe02` |
| `out/rare_disease_hypotheses.jsonl` | 1,491 | 7,055,212 | `c15fa5ac6b5e41f32a9a7a3fe184b8de2d639005642a723bf106a85a8ff36bfa` |
| `out/dgidb_target_interactions.jsonl` | 6,890 | 5,776,960 | `7d47f24803d10feb025d6c240cd53de813036158226686c4f609a4ee15384297` |
| `out/dgidb_request_records.jsonl` | 100 | 45,009 | `a4dcc0125c017b2fee7729249e7d1cc1dc75ba6c6463d5f349d441fde31e4a45` |
| `out/no_hit_or_uncertain_rows.jsonl` | 600 | 276,361 | `888fa8a853085345432d9c59d3f666de688ae80ae6198a5b5c0f94342bc8f230` |
| `out/rare_disease_bridge_corpus_rows.jsonl` | 1,000 | 1,222,200 | `54c46740d9fe64d4092616bf6ba5abc2b324115d4957c8556736933161218edf` |
| `out/top_evidence_bundles.json` | - | 224,983 | `01a2439db29e53425976debaa9bff21b55b6e3068888d4d2ba10b6e263bc81d0` |
| `out/persisted_readback.json` | - | 10,091 | `94d5204d09087fcd9ac164404638150cbb46ec21676751141ebbf04ca6a8a28a` |
| `out/calyx_bridge_corpus_readback.json` | - | 1,810 | `de92245240cc0817cb1b41ff3d96ecee9f02d9108abbdb08b7c05c673fc57946` |

Readback assertions:

| Assertion | Value |
|---|---|
| Hypothesis rows match metrics | true |
| Phenotype inputs present | true |
| Gene links present | true |
| Drug-bearing rows present | true |
| Bridge corpus rows present | true |
| Clinical boundary present on rows | true |

## Metrics

| Metric | Count |
|---|---:|
| HPO terms parsed | 19,836 |
| Mondo terms parsed | 56,273 |
| Mondo rare/authority-xref terms | 18,687 |
| HPOA annotation rows | 284,871 |
| Rare-disease input rows | 12,956 |
| Disease-gene phenotype links | 12,654 |
| Local DGIdb rows | 5,335 |
| Live DGIdb rows | 1,555 |
| Live DGIdb targets queried | 100 |
| Ranked candidate rows | 1,491 |
| Drug-bearing candidates | 891 |
| Target-prioritization candidates without drug edge | 600 |
| Rows with Mondo mapping | 1,454 |
| Rows with same target-disease Open Targets context | 29 |
| Rows with prior generated-candidate falsification | 0 |

Hypothesis classes:

| Class | Rows |
|---|---:|
| `hpo_gene_disease_drug_bridge` | 891 |
| `hpo_gene_disease_target_prioritization` | 600 |

## Native Calyx DB Readback

The bridge-corpus rows were materialized into a native Calyx vault:

| Field | Value |
|---|---|
| Vault name | `issue1189-rare-disease-bridge-20260704t114953z` |
| Vault id | `01KWPFQSXHFG8XF34BEN5FW34G` |
| Vault dir | `/home/croyse/calyx/vaults/01KWPFQSXHFG8XF34BEN5FW34G` |
| Row count | 1,000 |
| Bridge terms | 1,590 |
| Graph nodes written | 2,590 |
| Graph edges written | 16,000 |
| CSR persisted | true |
| Index contains vault name | true |
| Node readback count | 2,590 |
| Edge readback count | 16,000 |

The separate `out/calyx_bridge_corpus_readback.json` readback asserts status,
row count, CSR persistence, index entry, node count, edge count, and vault-dir
existence.

## Top Readback Rows

| Rank | Candidate | Disease | Gene | Drug | Score | Falsification | Uncertainty |
|---:|---|---|---|---|---:|---|---|
| 1 | `issue1189:75b023ebd0bf16262958e64b` | Cerebral arteriopathy, autosomal recessive, with subcortical infarcts and leukoencephalopathy 1 | NOTCH3 | TAREXTUMAB | 13.827495 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 2 | `issue1189:0e7d8b9319ec06d4a095a8c6` | Cardiofaciocutaneous syndrome 1 | BRAF | DABRAFENIB | 13.514949 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 3 | `issue1189:73d97283dc35372ca678689f` | Cardiofaciocutaneous syndrome 1 | BRAF | CETUXIMAB | 13.514949 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 4 | `issue1189:b24904c1c71d75e546de8655` | Cardiofaciocutaneous syndrome 1 | BRAF | TRAMETINIB DIMETHYL SULFOXIDE | 13.514949 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 5 | `issue1189:cbd09f52cca62d66a1f8bc92` | Cardiofaciocutaneous syndrome 1 | BRAF | PANITUMUMAB | 13.514949 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 6 | `issue1189:ec3a45d4477187265cc7129e` | Melnick-Needles syndrome | FLNA | SIMUFILAM | 13.474609 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 7 | `issue1189:9c7b08e2dff7e8908f919f7c` | Fanconi anemia | BRCA2 | OLAPARIB | 13.367395 | `not_run_for_hpo_generated_rare_disease_candidate` | no Open Targets same-disease context; not in prior sweep |
| 8 | `issue1189:1265eb5a44d7ff6d548e50e1` | Meningioma | PIK3CA | ALPELISIB | 13.366601 | `not_run_for_hpo_generated_rare_disease_candidate` | not in prior sweep |

These are phenotype/gene/drug association leads. They are not evidence that the
listed drug treats the listed disease.

## Findings

- The rare-disease phenotype substrate is now explicit and persisted: 12,956
  HPOA disease profiles and 12,654 disease-gene phenotype links.
- The hunt produced 1,491 ranked candidates, including 891 rows with a
  drug-target edge and 600 target-prioritization rows with no drug edge in the
  current sources.
- 1,454 candidates mapped to Mondo, but only 29 had same target-disease Open
  Targets context in the current bounded source set.
- No generated #1189 row has passed the cross-domain generated-candidate
  falsification sweep yet. This directly unblocks #1223 and keeps atlas
  promotion blocked until counter-evidence is persisted.
- The top rows are dominated by strong phenotype/gene support plus drug-target
  mappings, not by outcome, safety, efficacy, or clinical validation.

## Conclusion

#1189 is complete for the current bounded rare-disease phenotype/gene/drug
association hunt:

- public HPO/Mondo source bytes were downloaded and hashed;
- all rare-disease phenotype inputs and disease-gene phenotype links were
  persisted;
- ranked candidates include normalized disease, phenotype, gene, drug,
  evidence paths, falsification status, and uncertainty;
- a 1,000-row bridge corpus was materialized into native Calyx/Aster graph
  storage with direct readback.

Next required step: #1223 must run support/counter-evidence and falsification
over generated disease-hunt candidates from #1185/#1186/#1187/#1188/#1189
before any row can move toward the human-review atlas.

---

## 42_graph_csr_traversal_cache.md

# #1191 Graph CSR and Traversal Cache Readback

## Scope

#1191 verifies that the large #869 association graph has a persisted
collection-local CSR/traversal substrate and that the large readers load it
instead of broad row-scanning the Graph CF.

This is infrastructure for association discovery. It is not a biomedical
hypothesis, treatment, cure, or clinical-actionability claim.

## Code-Level Fail-Closed Coverage

New Aster tests prove persisted CSR corruption fails closed:

- `physical_csr_reader_rejects_tampered_segment_hash`
- `physical_csr_reader_rejects_manifest_count_mismatch`

Both return `CALYX_GRAPH_CORRUPT_ROW` rather than silently rebuilding from graph
rows or accepting a torn CSR stream.

Focused local gates:

```bash
cargo test -p calyx-aster physical_csr_reader_rejects -- --nocapture
cargo test -p calyx-cli graph_csr -- --nocapture
```

Result: both passed.

## Real Vault FSV

Host: aiwonder.

Repo: `/home/croyse/calyx/repo`.

Vault: `corpus-anchored-869-20260625T080546Z`
(`01KVYX0KYVBQSGVC6N2S00FX6J`).

FSV root:

```text
/home/croyse/calyx/fsv/issue1191-graph-csr-traversal-cache-release-20260704T011254Z
```

The real default graph already had a persisted CSR before this issue's refresh
run. Therefore this FSV proves persisted-CSR presence, refresh/readback, and
reader preference. It is not an empty-to-present benchmark.

## CSR Materialization Readback

Command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

Result:

```json
{
  "status": "ok",
  "collection": "default",
  "commit_seq": 684029,
  "csr_present_before": true,
  "csr_bytes_before": 157072071,
  "csr_bytes": 157072071,
  "csr_sha256": "72821df5df7bf02bc1c90afc2db4891847c66ea9d3f82b8832cb43424ca667f5",
  "nodes": 198993,
  "csr_edges": 2435817,
  "association_edge_count": 2435817,
  "readback": {
    "assoc_graph_nodes": 198993,
    "assoc_graph_edges": 2435817,
    "physical_node_keys": 198993,
    "physical_edge_out_keys": 2435817
  }
}
```

Stderr source-of-truth lines:

```text
materialize-graph-csr: before csr_present=true csr_bytes=Some(157072071)
materialize-graph-csr: committed seq=684029 nodes=198993 edges=2435817 association_edges=2435817 elapsed_ms=107940
plain-graph: loading persisted CSR collection=default nodes=198993 edges=2435817
materialize-graph-csr: physical readback ok csr_bytes=157072071 nodes=198993 graph_edges=2435817 elapsed_ms=115022
```

## Reader Preference and Timings

All three large readers logged persisted-CSR loading:

```text
plain-graph: loading persisted CSR collection=default nodes=198993 edges=2435817
```

Reader outputs were identical before and after the CSR refresh where expected.

| Reader | Before elapsed | After elapsed | Output readback |
|---|---:|---:|---|
| `spectral-communities` | 1:00.55 | 0:09.71 | 198,993 members, 2 communities, 8 bridge candidates, 8 centrality candidates |
| `domain-bridges` | 1:01.21 | 0:10.51 | 1 pair report, 7 candidates |
| `discovery-chain` | 0:57.11 | 0:06.66 | 198,993 graph nodes, 2,435,817 graph edges, 16 accepted hops, 52 candidates |

The after-run improvement includes OS/cache effects because the CSR already
existed at the start. The acceptance claim is reader behavior and physical CSR
readback, not a pure cold-cache performance ratio.

## Artifact Hashes

Selected hashes from `sha256sums.txt`:

| Artifact | SHA256 |
|---|---|
| `materialize_graph_csr.stdout` | `42da2318ce22731012911c7682aa2e8288718906c543f39efcc5dc3534c6269c` |
| `materialize_graph_csr.stderr` | `aafd4995c3fca0f7802e6d6268944022a9015f409be4254d6dfef00a535c0d4d` |
| `before_spectral_report.json` | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `after_spectral_report.json` | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `before_domain_report.json` | `8ab4c94b001714e5275e7a27d23d8071ba1b7a6dda097dfab5ee219443a980ef` |
| `after_domain_report.json` | `8ab4c94b001714e5275e7a27d23d8071ba1b7a6dda097dfab5ee219443a980ef` |
| `before_discovery_chain.json` | `da87f776bf2ed75e31eb575dc0e7b8518101f574c4543b749c61ccb545e12077` |
| `after_discovery_chain.json` | `da87f776bf2ed75e31eb575dc0e7b8518101f574c4543b749c61ccb545e12077` |
| `run.log` | `7d44f142bae4f6f575b7bd26662129f88af8108af0e0f8dbde3f8183abd86ca3` |
| `sha256sums.txt` | `0d921d7b9991bc3912f17727440da84a3641ba87aa274d0b783fa65c45686eee` |

`sha256sums.txt` contains 28 rows.

## Conclusion

#1191's graph-cache requirement is satisfied for the large #869 graph:

- persisted CSR exists for `default`;
- `materialize-graph-csr` can refresh it and read it back physically;
- physical readback checks CSR bytes/hash, CSR counts, association graph counts,
  and independent node/edge key counts;
- spectral, domain-bridge, and discovery-chain readers load the persisted CSR;
- corrupt CSR streams fail closed in focused tests.

The next broad-mining dependency is #1192/#1183: scalable probe/all-pair typed
association mining over the now-readable graph substrate.

---

## 43_probe_matrix_scale_repair.md

# #1192 Probe-Matrix Scale Repair and Real-Vault Readback

## Scope

#1192 verifies that probe-matrix mining can run against the large #869 clinical
association vault without reintroducing the earlier 41 GB resident-set failure
and without silently using stale derived indexes.

This is discovery infrastructure. It does not establish a treatment, cure, or
clinical actionability claim. Any association mined through this path remains a
ranked, traceable hypothesis until it clears outcome, safety, counter-evidence,
and external validation gates.

## Root Causes Found

The original large-RSS probe-matrix issue had already been addressed by #1001 in
commit `da7a883f`, which stopped materializing large provenance/hit structures
per variant.

The current real-vault blocker was earlier in the pipeline:

1. The real vault manifest pointed `panel_ref` at the stage-one placeholder
   `panel/current.bin` and had `registry_ref: null`, so panel loading failed
   with `CALYX_ASTER_CORRUPT_SHARD: decode panel`.
2. After explicit manifest repair, probe-matrix still always requested fresh
   search indexes. That was correct as a default, but unlike regular search it
   had no operator-visible `--stale-ok` policy for cases where the derived
   watermark advanced due unrelated non-search graph/manifest writes.

## Code Changes

- `e8b7fcad` adds `calyx panel manifest-restore --vault ... --panel-asset ...
  --registry-asset ...`.
  - It requires exact operator-supplied asset refs.
  - It validates ref prefixes, hashes, panel decode, registry decode, and
    registry-panel agreement.
  - It writes a new manifest, then reloads through the real panel loader and
    rechecks vault registry contracts.
- `5d0f2991` adds explicit `probe-matrix --stale-ok`.
  - Default behavior remains fail-closed fresh-index checking.
  - `--stale-ok` is an explicit operator policy, matching the regular search
    CLI contract.
  - The persisted progress JSON records `stale_ok`.

## Local Gates

Focused and hygiene gates passed:

```bash
cargo test -p calyx-cli manifest_restore -- --nocapture
cargo test -p calyx-cli cmd::probe_matrix::tests -- --nocapture
pwsh -File scripts/cargo-fmt-workspace.ps1
bash scripts/linecount.sh
git diff --check
cargo check -p calyx-cli
```

`git diff --check` emitted only CRLF normalization warnings for existing test
files.

## Real Vault Manifest Repair FSV

Host: aiwonder.

Repo: `/home/croyse/calyx/repo`.

Vault: `corpus-anchored-869-20260625T080546Z`
(`01KVYX0KYVBQSGVC6N2S00FX6J`).

FSV root:

```text
/home/croyse/calyx/fsv/issue1192-manifest-restore-20260704T013925Z
```

Repair command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx panel manifest-restore \
  --vault corpus-anchored-869-20260625T080546Z \
  --panel-asset panel/panel-v00000009-33427fad14698dd4.json \
  --registry-asset registry/registry-b14e0896f0c88790.json
```

Readback summary:

```json
{
  "status": "manifest_panel_registry_restored",
  "manifest_seq_before": 683838,
  "manifest_seq_after": 683839,
  "durable_seq": 684029,
  "derived_content_seq": 684029,
  "old_panel_ref": "panel/current.bin",
  "old_registry_ref": null,
  "new_panel_ref": "panel/panel-v00000009-33427fad14698dd4.json",
  "new_panel_blake3": "33427fad14698dd49ec43d649307d64f84170b263d3d4fef04692c33be07cf73",
  "new_registry_ref": "registry/registry-b14e0896f0c88790.json",
  "new_registry_blake3": "b14e0896f0c8879011ad785890f942f09979785a383fa674afebd571602c7fa8",
  "manifest_pointer": "manifest-00000000000000683839.json",
  "reloaded_panel_version": 9,
  "reloaded_slot_count": 25,
  "reloaded_registry_lens_count": 14,
  "registry_checked_count": 14
}
```

Independent readback after repair:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx list-panel corpus-anchored-869-20260625T080546Z
```

Result: 25 slots loaded, slots 8 through 24 active.

## Real Probe-Matrix FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1192-probe-matrix-stale-ok-20260704T014754Z
```

Release build on aiwonder:

```bash
cd /home/croyse/calyx/repo
git pull --ff-only
cargo build --release -p calyx-cli
```

Result: release build passed at `5d0f2991`.

Happy-path command shape:

```bash
CALYX_HOME=/home/croyse/calyx \
  /usr/bin/time -v ./target/release/calyx probe-matrix \
  corpus-anchored-869-20260625T080546Z \
  --frontier "type 2 diabetes" \
  --slot 21 \
  --weighted-profile bridge \
  --phrasing clinical \
  --length phrase \
  --top-k 3 \
  --guard off \
  --stale-ok \
  --out "$ROOT/happy/probe.json" \
  --search-miss-budget-ms 60000 \
  --search-hit-budget-ms 10000
```

Readback summary:

| Case | Exit | Status | Stop reason | Variants | Records | Accepted hits | Cache | Wall | Max RSS |
|---|---:|---|---|---:|---:|---:|---|---:|---:|
| `happy` | 0 | `ok` | none | 5/5 | 5 | 15 | 1 miss, 4 hits, 64 stored hits | 0:01.90 | 789,840 KB |
| `edge_gpu_resident_required` | 2 | `incomplete` | `resident_required` | 0/5 | 0 | 0 | no search cache use | 0:01.38 | 673,684 KB |
| `edge_variant_budget` | 2 | `incomplete` | `variant_budget_exhausted` | 1/5 | 1 | 3 | 1 miss, 64 stored hits | 0:01.73 | 789,724 KB |

Persisted artifact hashes:

| Case | Artifact | SHA256 |
|---|---|---|
| `happy` | `probe.json` | `a0faf70f421a6179f0f4fb05c14694f233a1d8033136233071a86944b3af1638` |
| `happy` | progress JSON | `a7e78c770b45cfe58f4de6fb786acbff16ab6787e16d4f61974bb22a16f17ff8` |
| `edge_gpu_resident_required` | `probe.json` | `e460f132bd35b8b7ede1711a50322d50b91496c3217295e943b1dc7e6fdfc628` |
| `edge_gpu_resident_required` | progress JSON | `da6fc0fbc9d995d40ee486e9b9b376bc3886208d39d9494d51eeaccd0bf93472` |
| `edge_variant_budget` | `probe.json` | `c5ffa7fe356c0a4fa07460c4651566923e5fca1f2e3f3c194082f4fca824adbc` |
| `edge_variant_budget` | progress JSON | `98f403f74f7aafeabf15e228b253670dae8ea60762a0b2ba5b6c28d77b9ed9f3` |

`readback_summary.json` and `sha256sums.txt` live at the FSV root above.

## Conclusion

#1192 is satisfied:

- the real #869 vault manifest now points to a decodable panel and matching
  registry snapshot;
- panel load and registry contract readback pass after the Calyx DB repair;
- probe-matrix retains fresh-index fail-closed behavior by default;
- explicit `--stale-ok` enables the operator-approved stale-index path;
- the real large-vault happy path completes in under 2 seconds and under 1 GB
  max RSS for the tested slot/frontier;
- edge cases persist diagnostic incomplete matrices and progress artifacts with
  source-of-truth hashes.

The next discovery dependency is broad typed all-pair association mining over
the now-readable large graph and probe substrate.

---

## 44_association_validation_gates.md

# #1182 Association Validation Gates

## Scope

#1182 adds a power-proven validation instrument for biomedical association
mining before broad all-pair hunts are accepted. The gate measures association
scoring against persisted known-positive rows, known-negative/no-hit controls,
and a time-split later-evidence benchmark.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. It is a source-backed
instrument for accepting or rejecting association-mining runs.

The issue originally named `docs/medicalsearch/33_association_validation_gates.md`;
that number was already occupied by later append-only work. This file keeps the
append-only numbering and cross-links #1182.

## Code Change

Commit:

```text
2bf61244 Add biomedical association validation gates
```

New CLI:

```text
calyx association-validation-gates \
  --typed-root <dir> \
  --open-targets-root <dir> \
  --pubtator-root <dir> \
  --clinicaltrials-root <dir> \
  --dgidb-root <dir> \
  --out-dir <dir> \
  [--cutoff-year <yyyy>] \
  [--score-threshold <0..1>] \
  [--min-auroc <0..1>] \
  [--min-positive-recall <0..1>] \
  [--min-negative-suppression <0..1>]
```

Artifacts written and read back:

- `benchmark_source_rows.jsonl`
- `train_test_split.jsonl`
- `scored_outputs.jsonl`
- `metrics.json`
- `association_validation_report.json`

The command fails closed on missing source roots/files, empty class sets,
one-class AUROC, failed threshold gates, or artifact readback mismatch. Existing
different output files are not overwritten.

## Source Inputs

The final real run used these persisted source roots:

| Source | Root |
|---|---|
| Typed overlay graph | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |

These sources are also materialized into Calyx/Aster Graph CF through #1196
(`biomed_evidence_substrate_v3`). #1182 adds the acceptance instrument and
readback artifacts used by downstream miners.

## Local Gates

Passed:

```bash
cargo test -p calyx-cli association_validation -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

## Real FSV: Strict Threshold Failure

The first real run intentionally used the default `--score-threshold 0.5`.
It failed closed and persisted artifacts:

```text
/home/croyse/calyx/fsv/issue1182-association-validation-gates-20260704T020316Z
```

Readback:

| Field | Value |
|---|---:|
| Exit code | 2 |
| Known positives | 57 |
| Known negatives | 2 |
| Time-split rows | 13 |
| Scored outputs | 72 |
| Known AUROC | 1.000 |
| Known positive recall at 0.5 | 0.544 |
| Known negative suppression at 0.5 | 1.000 |
| Time-split AUROC | 0.864 |

Failure reason:

```text
known-positive recall 0.544 below 0.750
```

Interpretation: the ranking separated positives from no-hit controls, but the
fixed 0.5 threshold suppressed low-score Open Targets positives. This is a
calibration finding, not a system success.

## Real FSV: Passing Source-Inclusive Gate

The accepted gate run used an explicit `--score-threshold 0.05`, reflecting the
lowest admitted external-source score for this bounded validation instrument.

FSV root:

```text
/home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z
```

Command shape:

```bash
./target/release/calyx association-validation-gates \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --open-targets-root /home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z \
  --pubtator-root /home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z \
  --clinicaltrials-root /home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z \
  --dgidb-root /home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z \
  --out-dir "$ROOT/out" \
  --cutoff-year 2016 \
  --score-threshold 0.05
```

Readback:

| Field | Value |
|---|---:|
| Exit code | 0 |
| Gate passed | true |
| Known positives | 57 |
| Known negatives | 2 |
| Benchmark source rows | 59 |
| Time-split rows | 13 |
| Scored outputs | 72 |
| Wall time | 0:00.09 |
| Max RSS | 92,388 KB |

Known-positive/negative metrics:

| Metric | Value | CI |
|---|---:|---|
| AUROC | 1.000 | 1.000 - 1.000 |
| Precision | 1.000 | 0.937 - 1.000 |
| Positive recall | 1.000 | 0.937 - 1.000 |
| Negative suppression | 1.000 | 0.342 - 1.000 |

Time-split metrics:

| Metric | Value | CI |
|---|---:|---|
| AUROC | 0.864 | 0.864 - 0.864 |
| Precision | 0.846 | 0.578 - 0.957 |
| Positive recall | 1.000 | 0.741 - 1.000 |
| Negative suppression | 0.000 | 0.000 - 0.658 |

The time-split benchmark is useful but small: 11 later-positive and 2
later-negative ClinicalTrials.gov seed rows. The gate currently uses time-split
AUROC as the acceptance criterion and reports threshold confusion separately.

## Artifact Hashes

Passing run:

| Artifact | SHA256 |
|---|---|
| `association_validation_report.json` | `7fb0aad1c7f66bea4c86c5d6d99084f1c2203769a494c6f6d58ff0071d0bf2c3` |
| `benchmark_source_rows.jsonl` | `beaadecf0b1a3efea2f7468aa03a8516bb34c6fc5731c2537b1ea698260595ea` |
| `train_test_split.jsonl` | `17efa29009b75fecef6cfc72ffe0111694893e799f8c4ed9e2b0d8fc8b869770` |
| `scored_outputs.jsonl` | `0f305c7c6805af068fe715f617610dc600e285e347be016a0dca37e676a7c4b3` |
| `metrics.json` | `017b5ccd44a10d8bc0533ca787d1bc0ff4b6d273b3a626598ac76a7948554bc7` |
| `stdout.json` | `ac7e1f600f924d000f0f160fa694fd04ab50e3227e15b4ba636bf6697886c3f3` |
| `stderr.log` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

`readback_summary.json` and `sha256sums.txt` are in the FSV root.

## Conclusion

#1182 is satisfied:

- real source roots are read and hashed;
- benchmark source rows are persisted separately;
- train/test split rows are persisted separately;
- scored outputs are persisted separately;
- metrics include recall, precision, AUROC, and confidence intervals;
- the strict-threshold failure is preserved as calibration evidence;
- the accepted thresholded gate passes against real persisted evidence.

Downstream broad miners must require a passing `association-validation-gates`
artifact before accepting mined hypotheses. Passing this gate still only admits
association hypotheses for ranking and falsification; it does not create a
clinical claim.

---

## 44_gpu_sparse_association_acceleration.md

# #1194 GPU/Sparse Association Acceleration Decision

## Scope

#1194 evaluates whether sparse graph association mining needs a GPU path now
that the large biomedical graph readers use persisted binary CSR. The target
surface is graph association work such as spectral communities, PPR/path-style
walk scoring, and bridge mining. This is a performance decision only; it does
not make a biomedical, treatment, safety, or cure claim.

## Decision

No GPU sparse graph kernel is selected for the current #867 path.

Reason: after #1191/#1210/#1213, the full #869 graph has a persisted binary CSR
and the current CPU/Rayon spectral-community run over the real graph completes
in 8.581 seconds from source-of-truth bytes. There is no existing Calyx sparse
graph GPU backend to verify without adding a speculative dependency, and the
measured current workload does not justify wiring a CUDA sparse eigensolver/PPR
backend ahead of the open higher-value GPU lens/resident issues (#1155, #1156,
#1158, #1159).

GPU graph kernels should be revisited only if a larger measured workload shows
a dominant sparse-matvec/PPR/path bottleneck and the implementation can be
verified against the CPU output hash with a strict tolerance.

## Real FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z
```

Profile summary:

```text
/home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/profile_summary.json
sha256: 7fb933a37f6cd6873879021e1ae62519a461e80f36adae7143cceb52c828f11b
```

Hardware readback:

```text
GPU: NVIDIA GeForce RTX 5090
Driver: 610.43.02
Memory: 32607 MiB
CUDA toolkit: 13.3.33
```

Benchmark input:

| Field | Value |
|---|---:|
| Vault | `corpus-anchored-869-20260625T080546Z` |
| Vault id | `01KVYX0KYVBQSGVC6N2S00FX6J` |
| Collection | `default` |
| Nodes | 198,993 |
| CSR edges | 2,435,817 |
| Association edge count | 2,435,817 |
| CSR bytes | 63,235,522 |
| CSR SHA-256 | `39124a1f244d6838360fccb8a43f62771b8d62d34bfcaf1c575d30c5d59df6a8` |

CSR command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

CPU profile command:

```bash
/usr/bin/time -v env CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 \
  ./target/release/calyx spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 3 \
  --eigen-max-iter 64 \
  --centrality-max-iter 512 \
  --centrality-tol 0.00001 \
  --max-bridge-candidates 8 \
  --max-centrality-candidates 8 \
  --out /home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/cpu_spectral_report.json
```

GPU telemetry command:

```bash
nvidia-smi --query-gpu=timestamp,utilization.gpu,memory.used,power.draw \
  --format=csv -l 1
```

## Output Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `csr_stdout.json` | 940 | `5663ec001560bc4b548a3e99d82899ce4496a5fd9afb3d8925d01afe864714ba` |
| `csr_stderr.txt` | 758 | `5c147c791b30707a910e5efd9aed7bc65bcf5c8db7119c3c1855d706669fb4b3` |
| `cpu_spectral_report.json` | 42,817,296 | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `cpu_spectral_stdout.json` | 34,473,492 | `83fd98bdde79318bb344e3ecb4465fe556cdbd662a0738c7890f294e5cc55cd5` |
| `cpu_spectral_stderr.txt` | 1,989 | `d9a1006e779d3ac5ab284d972c9768feb5f1f12f061abe01c9c2daa42114c39d` |
| `cpu_spectral_exit.txt` | 2 | `9a271f2a916b0b6ee6cecb2426f0b3206ef074578be55d9bc94f6f3fe3ab86aa` |
| `gpu_spectral_samples.csv` | 507 | `8dd2cd3eaedd7ae0a407cf9abffd4a9e851f77c8ac3ebd50a7778397e2be8c50` |
| `invalid_eigen_stdout.json` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `invalid_eigen_stderr.txt` | 150 | `1d7d3c3b613f2585f40fc3b4e466178ea10409935aef176ed84e17f2635fb639` |

## CPU Profile

| Metric | Value |
|---|---:|
| Exit status | 0 |
| Persisted CSR loaded | true |
| Graph nodes | 198,993 |
| Graph edges | 2,435,817 |
| Communities | 2 |
| Bridge candidates | 8 |
| Centrality candidates | 8 |
| Spectral gap | 0.943023681640625 |
| CLI elapsed | 8,581 ms |
| `/usr/bin/time` wall clock | 0:08.99 |
| CPU percent | 447 |
| User seconds | 37.94 |
| System seconds | 2.29 |
| Max RSS | 5,976,604 KiB |

## GPU Telemetry

| Metric | Value |
|---|---:|
| Samples | 9 |
| Max GPU utilization | 0% |
| Max memory used | 10,212 MiB |
| Max power draw | 65.14 W |

GPU output hash:

```text
null
```

Parity status:

```text
not_applicable_no_gpu_sparse_backend_selected
```

This is intentional: no GPU kernel was selected because the measured current
CPU path is below the threshold where speculative GPU work is justified, and
there is no existing Calyx graph-GPU backend to run as a parity candidate.

## Failure Case

Invalid eigen count:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 1 \
  --out /home/croyse/calyx/fsv/issue1194-gpu-sparse-profile-20260705T034332Z/invalid_eigen_report.json
```

Readback:

| Field | Value |
|---|---:|
| Exit status | 2 |
| Error code | `CALYX_CLI_USAGE_ERROR` |
| Invalid report exists | false |

## Assertions

| Assertion | Value |
|---|---:|
| CSR readback is ok | true |
| CPU spectral command exit is zero | true |
| CPU report SHA matches CLI stdout artifact SHA | true |
| Profiled real large graph | true |
| Spectral reader loaded persisted CSR | true |
| GPU samples captured | true |
| GPU sparse kernel not selected | true |
| Invalid eigen case failed closed | true |

## Conclusion

#1194 is resolved as a measured no-GPU decision for the current association
graph path. The persisted binary CSR path removes the prior row-scan bottleneck,
the real full-graph spectral run is CPU/Rayon-fast enough for the current #867
workflow, and no speculative CUDA sparse graph backend is added.

Future work belongs in a new performance issue only when a larger real workload
produces a measured sparse-matvec/PPR/path-scoring bottleneck and a GPU kernel
can be verified against the CPU output hash with explicit tolerances.

---

## 45_all_pair_typed_association_miner.md

# #1183 All-Pair Typed Association Miner

## Scope

#1183 adds a bounded typed association miner over the #1173 biomedical overlay
graph. It scans persisted `typed_edges.jsonl`, deduplicates repeated or reversed
`associated_with` edges into typed concept-pair hypotheses, requires a passing
#1182 validation report, and writes readback-verifiable artifacts.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. Every emitted row is an
association hypothesis with explicit counter-evidence hooks for #1184 and
safety triage before any downstream claim can be trusted.

## Code Change

Commit:

```text
0d64a449 Add typed association miner
```

New CLI:

```text
calyx typed-association-miner \
  --typed-root <dir> \
  --validation-report <json> \
  --out-dir <dir> \
  [--source-type <concept-type>] \
  [--target-type <concept-type>] \
  [--name-contains <text>] \
  [--source-issue <n>] \
  [--min-support <n>] \
  [--max-pairs <n>] \
  [--max-input-edges <n>] \
  [--max-paths-per-pair <n>]
```

Artifacts written and read back:

- `typed_association_miner_report.json`
- `hypotheses.jsonl`
- `score_summary.json`

The command fails closed if the #1182 validation report did not pass, if typed
nodes are missing, if no candidates remain after filters, or if persisted
artifact bytes differ on readback. Existing different output files are not
overwritten.

## Behavior

- Full typed-edge JSONL is streamed, not loaded wholesale.
- Filters are orientation-aware: a disease-to-chemical source edge can still
  emit a chemical-to-disease hypothesis when `--source-type chemical
  --target-type disease` is requested.
- Repeated/reversed edges for the same typed pair are deduplicated into one
  hypothesis, with support summed and paths capped by `--max-paths-per-pair`.
- Each hypothesis carries:
  - validation report SHA-256
  - source/target ids, names, and concept types
  - support count, path count, score, novelty score
  - source hashes/support CxIds when present
  - `requires_1184_falsification_sweep`
  - `requires_safety_triage_for_drug_or_intervention_claims`
  - clinical boundary text

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli typed_association_miner -- --nocapture
cargo test -p calyx-cli typed_association_miner_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused unit evidence:

- parser accepts source/target/name/source issue/support filters
- failed #1182 validation report is refused
- artifacts persist with report readback
- reversed chemical/disease edges deduplicate into one filtered orientation
- token round-trip covers the new command

## Remote Build

`aiwonder` release build passed after fast-forwarding to `0d64a449`:

```bash
ssh aiwonder 'cd /home/croyse/calyx/repo && git pull --ff-only && cargo build --release -p calyx-cli'
```

Result:

```text
Finished `release` profile [optimized] target(s) in 35.48s
```

## Real FSV

Root:

```text
/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z
```

Inputs:

| Input | Path / SHA |
|---|---|
| Typed overlay | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z` |
| #1182 validation report | `/home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json` |
| Validation report SHA-256 | `7fb0aad1c7f66bea4c86c5d6d99084f1c2203769a494c6f6d58ff0071d0bf2c3` |

Commands:

```bash
./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad \
  --min-support 1 \
  --max-pairs 250 \
  --max-input-edges 200000

./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease \
  --source-type chemical \
  --target-type disease \
  --min-support 1 \
  --max-pairs 100 \
  --max-input-edges 200000

./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease \
  --source-type gene \
  --target-type disease \
  --min-support 1 \
  --max-pairs 100 \
  --max-input-edges 200000
```

Readback summary from persisted report bytes:

| Run | Nodes | Edges scanned | Limit hit | Candidate pairs | Emitted | Report SHA-256 |
|---|---:|---:|---|---:|---:|---|
| Broad | 85 | 116,753 | false | 267 | 250 | `973d939cfd8f2aec8ac1ef218233078f59c5524de865284c64bc3e1e490c1c8c` |
| Chemical/disease | 85 | 116,753 | false | 42 | 42 | `5614f6fc1594e6eb7ad318364637d73d19bc7d8107b7ebe0891864f001bdc03f` |
| Gene/disease | 85 | 116,753 | false | 9 | 9 | `3532e0bc3e03b0d45469e5cf371f6dddc46e97799d2182cfa0024b03ee658bf3` |

Top readback rows:

| Run | Hypothesis | Source | Target | Support | Score |
|---|---|---|---|---:|---:|
| Broad | `typed-assoc:concept:ncbi_mesh:C062735::concept:ncbi_mesh:C093875` | zafirlukast / chemical | montelukast / chemical | 28 | 1.0 |
| Chemical/disease | `typed-assoc:concept:ncbi_mesh:D016595::concept:ncbi_mesh:D010437` | Misoprostol / chemical | Peptic Ulcer / disease | 8 | 1.0 |
| Gene/disease | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | Tnf / gene | Proteinuria / disease | 9 | 1.0 |

Artifact hashes:

| Run | `hypotheses.jsonl` SHA-256 | `score_summary.json` SHA-256 |
|---|---|---|
| Broad | `99394214a3147d34828dc2830b622b90ae434cadc407c89a57e3c57ae9144d15` | `b1ae7814ef57ef4f17a57dafeb66f27cbaf1ff2bc698264dc82ad1dbdfb542bb` |
| Chemical/disease | `ba1d310bdb38d2cefc654b0faab45252ce7223c9f6b9e68e743b33b78492828b` | `f0ee70b18e0fbbc7a0cb3426fd462955bb6c43866476076da4068b68aa63dc91` |
| Gene/disease | `845a2609eec7a392a3ccde10d8756a549eb91a71c9eaa96cee77874afff04518` | `d42c51dcf3150235543bc7fd6ed848429f4e607f208a111d659c7af661dea970` |

## Conclusion

#1183 is complete for the current typed overlay: the miner scans the persisted
overlay, requires the #1182 validation gate, deduplicates typed pairs, emits
bounded scored hypotheses, and proves the output by reading back artifact bytes.

The output is intentionally still hypothesis-only. The next required atomic
step is #1184 counter-evidence/falsification sweep before any ranked association
can be promoted beyond a traceable lead.

---

## 46_hypothesis_falsification_sweep.md

# #1184 Hypothesis Falsification Sweep

## Scope

#1184 adds a persisted-source falsification sweep for retained typed
association hypotheses. It reads #1183 miner reports, scans persisted
PubTator/PubMed, ClinicalTrials.gov, DGIdb, and Open Targets evidence roots,
then writes separate support evidence, counter-evidence, raw source manifest,
and final per-hypothesis flags.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. The sweep is a demotion and
triage instrument: it makes counter-evidence visible before atlas/human-review
publication.

## Code Change

Commits:

```text
04155875 Add hypothesis falsification sweep
2d1ffbd7 Constrain falsification source matching
```

The second commit corrected source applicability after the first real readback
showed Open Targets target-disease rows could otherwise create false counters
for gene-gene hypotheses. Final FSV below uses the corrected release build.

New CLI:

```text
calyx hypothesis-falsification-sweep \
  --hypotheses-report <json> \
  [--hypotheses-report <json> ...] \
  --pubtator-root <dir> \
  --clinicaltrials-root <dir> \
  --dgidb-root <dir> \
  --open-targets-root <dir> \
  --out-dir <dir> \
  [--max-hypotheses <n>]
```

Artifacts written and read back:

- `falsification_sweep_report.json`
- `support_evidence.jsonl`
- `counter_evidence.jsonl`
- `hypothesis_flags.jsonl`
- `raw_query_manifest.jsonl`

The command fails closed on missing reports, missing source roots/files, missing
hypothesis arrays, over-budget hypothesis count, parse failures, or readback
mismatch. Existing different output files are not overwritten.

## Evidence Classes

Support evidence currently includes:

- PubTator supporting literature rows.
- ClinicalTrials.gov registry hits and completed/results trial rows.
- DGIdb exact drug-gene interactions.
- Open Targets target-disease association scores when type-applicable.

Counter-evidence currently includes:

- PubTator negative text-signal rows.
- ClinicalTrials.gov stopped/withdrawn/suspended status rows.
- DGIdb exact-pair no-hit rows.
- Open Targets low-score target-disease rows when type-applicable.

Source applicability is enforced before text matching:

| Source | Applicable hypothesis types |
|---|---|
| ClinicalTrials.gov | chemical/disease |
| DGIdb | chemical/gene or chemical/gene_protein |
| Open Targets | gene/disease or gene_protein/disease |
| PubTator | all typed pairs |

Drug/chemical hypotheses also carry
`safety_toxicity_triage_pending_issue_1181` until the separate safety triage
issue is complete.

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
cargo test -p calyx-cli hypothesis_falsification_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused unit evidence:

- parser accepts repeated `--hypotheses-report` inputs and required roots;
- persisted source fixture produces support and counter-evidence rows;
- final report readback decodes the persisted flags;
- token round-trip covers the new command.

## Remote Build

`aiwonder` release build passed after fast-forwarding to `2d1ffbd7`:

```bash
ssh aiwonder 'cd /home/croyse/calyx/repo && git pull --ff-only && cargo build --release -p calyx-cli'
```

Result:

```text
Finished `release` profile [optimized] target(s) in 36.26s
```

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1184-hypothesis-falsification-20260704T023537Z
```

Inputs:

| Input | Path |
|---|---|
| #1183 broad report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json` |
| #1183 chemical/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json` |
| #1183 gene/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |

Command:

```bash
./target/release/calyx hypothesis-falsification-sweep \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json \
  --pubtator-root /home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z \
  --clinicaltrials-root /home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z \
  --dgidb-root /home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z \
  --open-targets-root /home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z \
  --out-dir /home/croyse/calyx/fsv/issue1184-hypothesis-falsification-20260704T023537Z/out \
  --max-hypotheses 1000
```

Readback counts:

| Field | Value |
|---|---:|
| Input hypothesis rows | 301 |
| Deduped hypotheses | 280 |
| Raw source manifest rows | 7 |
| Support evidence rows | 59 |
| Counter-evidence rows | 1 |
| Hypotheses flagged with counter-evidence | 1 |
| Hypothesis flags read back | 280 |

Sweep status distribution:

| Status | Count |
|---|---:|
| `complete_no_counterevidence_found_in_current_sources` | 279 |
| `complete_counterevidence_found` | 1 |

Counter-evidence row:

| Hypothesis | Source | Reason | Weight | Summary |
|---|---|---|---:|---|
| `typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D007674` | Open Targets | `open_targets_low_score_exact_pair` | 0.5 | Open Targets low score `0.03701863799150296` |

Highest falsification-score flag:

| Hypothesis | Pair | Counter | Support | Score | Reasons |
|---|---|---:|---:|---:|---|
| `typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D007674` | PLA2R1 / Kidney Diseases | 1 | 4 | 0.092 | `open_targets_low_score_exact_pair` |

## Artifact Hashes

| Artifact | SHA-256 |
|---|---|
| `falsification_sweep_report.json` | `4c3c8bd121a45df9fcf5ab1d3a05b72204539f893cf3b9b8d821399550e2ef5c` |
| `support_evidence.jsonl` | `01d66f0fb6a1644dda9ba3a4c57eaa6dbb595665ce424514e80422cb895e23d0` |
| `counter_evidence.jsonl` | `f0e4f62da5c719c48579c832cf24e5adae607245ff6848aaa4d27c82e84f3637` |
| `hypothesis_flags.jsonl` | `9d80c503a5173e8a3056101c132b1b299905e801a634d87aabf5bcab862e3e77` |
| `raw_query_manifest.jsonl` | `bd2c94ae621a089cea8cac7826284ba3d00d5065d3b04928f5b07ca5ceef3bbf` |
| `stdout.json` | `d52d151ac8b7a76356b529ff045a65c6a0d0f5e917029ebb3036f6f1115d1aef` |

Raw source manifest hashes:

| Source | Role | Bytes | SHA-256 |
|---|---|---:|---|
| PubTator | supporting_literature | 532,523 | `bf473c33e99f596411116b8fb4a165ca1dd893a73399d552efa8979689ad9cb0` |
| PubTator | negative_literature | 2,982 | `2ded353b125e85436a6fad4d431c61f760bb4aa2de2112ac7a68acec4002dd08` |
| ClinicalTrials.gov | seed_summaries | 10,951 | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| ClinicalTrials.gov | trial_rows | 424,800 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| DGIdb | seed_pair_interactions | 63,220 | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| DGIdb | unmapped_no_hit_rows | 971 | `228bd8df335a75045a3ceb596da5d17a0041501807a0ceaa29afdd63c92ced50` |
| Open Targets | validation_edges | 1,641,000 | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |

## Conclusion

#1184 is complete for the retained #1183 hypotheses: every deduped hypothesis
now has a persisted falsification sweep status before atlas publication. The
single counter-evidence flag demotes PLA2R1 / Kidney Diseases for low Open
Targets association score in the current source set.

The absence of counter-evidence in these bounded sources is not proof of truth,
safety, efficacy, actionability, or cure. Drug/chemical hypotheses still require
#1181 safety/adverse-event triage before any promotion beyond traceable lead.

---

## 47_precision_oncology_validation.md

# #1180 Precision Oncology Validation Sources

## Scope

#1180 adds a precision-oncology validation/triage source layer for cancer
hypotheses. This slice ingests CIViC public monthly dumps, records source
license/API constraints for non-ingested oncology resources, parses cancer
evidence/assertion/gene/variant rows, maps rows against the current #1173 typed
overlay, and persists unresolved accounting.

This is validation and triage evidence only. CIViC evidence levels,
assertions, therapies, and clinical significance fields are not Calyx treatment
recommendations, cure claims, safety claims, or clinical-actionability claims.

The issue originally named `docs/medicalsearch/31_precision_oncology_validation.md`;
that number is already occupied by LINCS/CMap reversal work. This file keeps the
append-only numbering and cross-links #1180.

## Source Decisions

| Source | Decision | Constraint |
|---|---|---|
| CIViC | ingested | AWS Open Data registry lists monthly CIViC dumps with CC0 license |
| OncoKB | not ingested | API requires registration/license token; FAQ states OncoKB cannot be used to train AI/ML models |
| Sanger DepMap/GDSC | not ingested | data/API are public for non-commercial/internal research with commercial/API restrictions |

References used during source selection:

- CIViC API/docs: `https://civic.readthedocs.io/en/latest/api.html`
- CIViC AWS Open Data registry: `https://registry.opendata.aws/civic/`
- OncoKB licensing FAQ: `https://faq.oncokb.org/licensing`
- Sanger DepMap data usage policy: `https://depmap.sanger.ac.uk/documentation/data-usage-policy/`

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1180-precision-oncology-validation-20260704T024318Z
```

Typed overlay used for mapping:

```text
/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z
```

The FSV script discovered CIViC dump keys through the public S3 XML listing API
and downloaded the latest available files for each selected category. `aws` was
not installed on aiwonder, so no AWS account or signed request was used.

Selected CIViC files:

| Category | Key | Bytes | SHA-256 |
|---|---|---:|---|
| ClinicalEvidenceSummaries | `ClinicalEvidenceSummaries/date=01-Jul-2026/ClinicalEvidenceSummaries.tsv` | 4,085,714 | `7277812e373f9489f3232f032378b94dc50474307e476c4d07bb891d8e31ce33` |
| AssertionSummaries | `AssertionSummaries/date=01-Jul-2026/AssertionSummaries.tsv` | 182,113 | `889cfc9d16e071d38ef4ca790b6c074f99466bd0763b17402eb348fd55713d77` |
| GeneSummaries | `GeneSummaries/date=01-Jan-2025/GeneSummaries.tsv` | 91,343 | `19f0b79a6d3dc68b663e38287c64e611505e1a1bb756c5b81172a8ab0a228d6f` |
| VariantSummaries | `VariantSummaries/date=01-Jul-2026/VariantSummaries.tsv` | 583,238 | `7d7973e6b0c6deaa78e3d906a3092b38c62c08a38a9fdca6c11a3941ad11ef50` |

Readback counts:

| Field | Count |
|---|---:|
| CIViC clinical evidence rows | 4,870 |
| Mapped rows against current typed overlay | 15 |
| Unresolved rows | 4,855 |
| Assertion rows | 143 |
| Gene rows | 591 |
| Variant rows | 1,984 |
| Distinct disease labels in evidence | 332 |
| Distinct gene labels in evidence | 550 |
| Distinct variant labels in evidence | 1,161 |

Top disease labels:

| Disease | Rows |
|---|---:|
| Von Hippel-Lindau Disease | 625 |
| Lung Non-small Cell Carcinoma | 452 |
| Colorectal Cancer | 351 |
| Chronic Myeloid Leukemia | 320 |
| Cancer | 236 |
| Acute Myeloid Leukemia | 220 |
| Breast Cancer | 199 |
| Melanoma | 139 |
| Lung Adenocarcinoma | 119 |
| Gastrointestinal Stromal Tumor | 89 |

Top genes:

| Gene | Rows |
|---|---:|
| VHL | 659 |
| EGFR | 243 |
| BRAF | 204 |
| TP53 | 192 |
| KRAS | 191 |
| PIK3CA | 157 |
| ERBB2 | 142 |
| KIT | 112 |
| FLT3 | 78 |
| PTEN | 60 |

Top therapies:

| Therapy | Rows |
|---|---:|
| Imatinib | 169 |
| Cetuximab | 158 |
| Dasatinib | 148 |
| Erlotinib | 126 |
| Vemurafenib | 124 |
| Crizotinib | 120 |
| Gefitinib | 91 |
| Trastuzumab | 80 |
| Trametinib | 78 |
| Imatinib Mesylate | 73 |

Evidence level counts:

| Level | Rows |
|---|---:|
| A | 225 |
| B | 1,625 |
| C | 1,671 |
| D | 1,317 |
| E | 32 |

Evidence type counts:

| Type | Rows |
|---|---:|
| Predictive | 2,849 |
| Predisposing | 681 |
| Prognostic | 536 |
| Diagnostic | 499 |
| Functional | 164 |
| Oncogenic | 141 |

## Persisted Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `source_constraints.json` | 561 | `d24e0ea98697bed4da8b356fb41d717dfcb9cf4479f62cd0301735b248e33de5` |
| `civic_download_manifest.json` | 2,386 | `912a14facbcfdea6833f61be80148072829678d6402bb0e35d7b35b198e5698b` |
| `civic_tsv_headers.json` | 2,320 | `4e58f287990c4434d272f5bad1e006536c21d6d5f71b5c660906b80ad3cd31d5` |
| `run_summary.json` | 6,747 | `85baec997eda18c3ca81dcfa864e14b5cf5cdb8f650b141199a33a0a38e52e2e` |
| `persisted_readback.json` | 251 | `833436775e7ad60f1391534e3adf8e0fb22326da9e8a34e610cab52a9af18bf2` |
| `final_file_manifest.json` | 3,271 | `12564fecdf505d9ed22ba57bfa7af56e7809f6df6759e9fe62a2f9e0c43aafe2` |
| `parsed/civic_evidence_rows.jsonl` | 3,506,213 | `bb3b6955275f71cf51af65d3541249bb06f4a7137689c323877af3c2946d2261` |
| `parsed/civic_mapped_rows.jsonl` | 13,589 | `b9891be3230b6dfa18f0fc3fba5ba130c3bbde66448b890517a8097c0779927d` |
| `parsed/civic_unresolved_rows.jsonl` | 1,055,614 | `2e65ec06511c9138517512b77d887631e97ad513ade44cb8c3c7106991abe315` |
| `parsed/civic_assertion_rows.jsonl` | 59,155 | `b682159bf8ff544bbc5cedb74f95b20cea677f48af9783878f21d96ebf8c0639` |
| `parsed/civic_gene_rows.jsonl` | 47,164 | `5c870c8ca16b819197345be4a5df0b29fab3c1e8746a0b937e2317d3c5e6d63a` |
| `parsed/civic_variant_rows.jsonl` | 269,720 | `d1ece325ccaf537c6babc7c9bd96556f0dedee2f4cd6401b676700b95d96d0d7` |

## Mapped Examples

The current typed overlay is small relative to CIViC. Exact mapping therefore
mostly hits concepts already present in the #1173 overlay.

Examples:

| CIViC row | Mapped concept | Evidence |
|---|---|---|
| NF1 mutation / Skin Melanoma / Vemurafenib | `concept:ncbi_gene:4763` NF1 | Predictive resistance, level D/C rows |
| NF1 mutation / Plexiform Neurofibroma / Selumetinib | `concept:ncbi_gene:4763` NF1 | Predictive sensitivity/response, level A |
| Metformin combinations in cancer/breast cancer rows | `concept:chembl:CHEMBL1431`, `concept:mesh:D008687` Metformin | Predictive sensitivity/response, level D/B |
| PTTG1/LEPR expression / Meningioma | `concept:ncbi_mesh:D008579` Meningioma | Prognostic poor-outcome rows |

## Conclusion

#1180 is complete as a source-acquisition and triage layer:

- current CIViC public source bytes are downloaded and hashed;
- source constraints for OncoKB and Sanger DepMap/GDSC are recorded instead of
  silently ingesting restricted data;
- cancer evidence, assertions, genes, variants, mapped rows, and unresolved rows
  are persisted separately;
- actionability/evidence-level fields remain validation features, not hypothesis
  scores or clinical claims.

The immediately useful downstream input for #1185 is:

```text
/home/croyse/calyx/fsv/issue1180-precision-oncology-validation-20260704T024318Z/parsed/civic_mapped_rows.jsonl
```

with unresolved evidence available for future concept-expansion work.

---

## 48_oncology_deep_hunt.md

# #1185 Oncology Deep Association Hunt

## Scope

#1185 composes the current typed association-mining, falsification, and
precision-oncology validation surfaces into one oncology hypothesis atlas.

Inputs are persisted artifacts from #1180, #1183, and #1184. The output is a
ranked evidence bundle for downstream safety triage and external validation.
Rows are hypotheses only: not efficacy claims, safety claims, clinical
actionability, treatment recommendations, or cure evidence.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z
```

The first local run for this issue was discarded because cancer type
attribution used the non-cancer endpoint for two typed rows. The final FSV root
above reassigns cancer type from the cancer-side endpoint and is the only
#1185 result to use.

Readback summary:

| Field | Count |
|---|---:|
| Total candidates | 19 |
| CIViC precision-oncology candidates | 15 |
| Typed association candidates | 4 |
| Input manifest rows | 7 |

Cancer type counts:

| Cancer type | Candidates |
|---|---:|
| Meningioma | 5 |
| Skin Melanoma | 4 |
| Childhood Acute Lymphocytic Leukemia | 3 |
| Plexiform Neurofibroma | 2 |
| Breast Cancer | 1 |
| Cancer | 1 |
| Childhood Low-grade Glioma | 1 |
| Leukemia Lymphocytic Chronic B-Cell | 1 |
| Malignant Peripheral Nerve Sheath Tumor | 1 |

## Input Scope

| Role | Bytes | SHA-256 |
|---|---:|---|
| `civic_mapped_rows` | 13,589 | `b9891be3230b6dfa18f0fc3fba5ba130c3bbde66448b890517a8097c0779927d` |
| `civic_evidence_rows` | 3,506,213 | `bb3b6955275f71cf51af65d3541249bb06f4a7137689c323877af3c2946d2261` |
| `civic_summary` | 6,747 | `85baec997eda18c3ca81dcfa864e14b5cf5cdb8f650b141199a33a0a38e52e2e` |
| `falsification_flags` | 180,225 | `9d80c503a5173e8a3056101c132b1b299905e801a634d87aabf5bcab862e3e77` |
| `typed_miner_report` broad | 400,469 | `973d939cfd8f2aec8ac1ef218233078f59c5524de865284c64bc3e1e490c1c8c` |
| `typed_miner_report` chemical/disease | 60,014 | `5614f6fc1594e6eb7ad318364637d73d19bc7d8107b7ebe0891864f001bdc03f` |
| `typed_miner_report` gene/disease | 14,875 | `3532e0bc3e03b0d45469e5cf371f6dddc46e97799d2182cfa0024b03ee658bf3` |

Source paths are recorded in:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/input_scope.json
```

## Persisted Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `out/input_scope.json` | 2,160 | `993e37c99bde36b58fd98e0c639ab8a9b301f8719bb868eaf3a8f1bef63d68c0` |
| `out/oncology_hypothesis_atlas.jsonl` | 23,618 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| `out/counts_by_cancer_type.json` | 283 | `eebc55ee2d18f915162d4e4218c14f21a5a60e693cdde5928ce603e511d6a454` |
| `out/top_candidate_evidence_bundles.json` | 27,661 | `9ac8d4ff34bd0f9c35fd0492bf078b578a7767142ce3fa750810101f56736ed4` |
| `out/run_summary.json` | 828 | `67e75622b975472744116996359fd222edd7f8a54bc550f56698eaf8e7585fd0` |
| `out/output_manifest.json` | 1,226 | `4bac77449e9b8382394dcac1a98b51406ad3fdd0cc2c15b9bc1f000e70b32c93` |
| `out/persisted_readback.json` | 1,833 | `e114f18c059f0594e43a007e08f5357f48b2111130e41f23c074474683d0fec8` |

Separate persisted readback confirmed:

| Field | Value |
|---|---|
| `status` | `ok` |
| `candidate_count` | `19` |
| `output_files` | `5` |
| `atlas_sha256` | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| `manifest_sha256` | `4bac77449e9b8382394dcac1a98b51406ad3fdd0cc2c15b9bc1f000e70b32c93` |

## Top Candidates

| Candidate | Cancer type | Gene | Variant | Therapies | Rank | Evidence | Falsification status | Safety/trial flags |
|---|---|---|---|---|---:|---|---|---|
| `oncology-civic:11176` | Plexiform Neurofibroma | NF1 | Mutation | Selumetinib | 6.8 | A Predictive Sensitivity/Response | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:1958` | Plexiform Neurofibroma | NF1 | Mutation | Selumetinib | 6.6 | A Predictive Sensitivity/Response | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:10138` | Breast Cancer | IGF1R | Overexpression | Metformin; Exemestane | 5.7 | B Predictive | `not_in_1184_typed_pair_surface` | `safety_triage_pending_issue_1181` |
| `oncology-civic:7487` | Childhood Low-grade Glioma | NF1 | Mutation | Selumetinib | 5.6 | B Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181`; `trial_ids_present_in_civic_row` |
| `oncology-civic:1053` | Meningioma | PTTG1 | OVEREXPRESSION | none | 5.1 | B Prognostic | `complete_no_counterevidence_found_in_current_sources` | none |
| `oncology-civic:1054` | Meningioma | LEPR | UNDEREXPRESSION | none | 5.1 | B Prognostic | `complete_no_counterevidence_found_in_current_sources` | none |
| `oncology-civic:1470` | Skin Melanoma | NF1 | Mutation | Vemurafenib | 4.5 | C Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1230` | Cancer | NRAS | Mutation | Metformin; Trametinib | 3.5 | D Predictive | `not_in_1184_typed_pair_surface` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1469` | Skin Melanoma | NF1 | Mutation | Sirolimus; Mirdametinib | 3.3 | D Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |
| `oncology-civic:1743` | Malignant Peripheral Nerve Sheath Tumor | NF1 | Loss | JQ1 Compound | 3.3 | D Predictive | `complete_no_counterevidence_found_in_current_sources` | `safety_triage_pending_issue_1181` |

Top candidate readback:

| Field | Value |
|---|---|
| Candidate | `oncology-civic:11176` |
| Source URL | `https://civicdb.org/links/evidence_items/11176` |
| Citation | Gross et al., 2020 |
| Trial id | `NCT01362803` |
| Mapped concept | `concept:ncbi_gene:4763` NF1 |
| Raw row SHA-256 | `385346078b2774619a46b813955b6e3eaf5907ecc7ff1f42ea0ebe563f5141fe` |
| Clinical boundary | Hypothesis only; not efficacy, safety, actionability, treatment recommendation, or cure evidence |

## Conclusion

#1185 is complete for this evidence-composition slice. It centralizes the
available oncology-specific candidates into one persisted atlas, with source
hashes, falsification status, cancer-type counts, and safety/trial flags.

The atlas is useful as a work queue for #1181 safety adjudication and future
external validation gates. It does not clear the Calyx clinical-actionability
bar because outcome sufficiency, safety, counter-evidence breadth, and clinical
review remain open gates.

---

## 49_drug_safety_triage.md

# #1181 Drug Safety / Adverse-Event Triage

## Scope

#1181 attaches public safety/adverse-event/contraindication evidence to the
drug-bearing #1185 oncology hypothesis atlas. This slice uses FDA public data
through openFDA:

- openFDA drug label API for label sections including boxed warnings,
  warnings, contraindications, drug interactions, and adverse reactions:
  `https://open.fda.gov/apis/drug/label/`
- openFDA drug adverse event API / FAERS for reported adverse-event counts:
  `https://open.fda.gov/apis/drug/event/`

The issue originally named `docs/medicalsearch/32_drug_safety_triage.md`; that
number is already occupied by LINCS/CMap perturbation metadata mapping. This
file keeps the append-only findings log order and cross-links #1181.

This is safety triage evidence only. FDA labels and FAERS/openFDA event counts
do not prove causality, incidence, prevalence, safety, efficacy,
clinical-actionability, treatment recommendation, or cure evidence. Missing
source coverage is a fail-closed ranker block, not an implication of safety.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z
```

Input atlas:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl
```

Input readback:

| Role | Bytes | SHA-256 |
|---|---:|---|
| `oncology_hypothesis_atlas` | 23,618 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |

Run summary:

| Field | Count |
|---|---:|
| Drug terms queried | 14 |
| Drug terms with FDA label coverage | 11 |
| Drug terms with FAERS/openFDA event coverage | 11 |
| Parsed safety rows | 353 |
| Raw source files | 28 |
| Raw query manifest rows | 28 |
| Candidate-drug mappings | 26 |
| Candidate safety flag rows | 13 |
| Candidate rows with source-unavailable block | 2 |
| Candidate rows with high-risk label block | 11 |
| Candidate rows with FAERS serious/death-review block | 11 |

Fail-closed missing coverage:

| Drug term | Label | FAERS/openFDA events | Flag |
|---|---|---|---|
| AZ628 | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |
| JQ1 Compound | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |
| VTX-11e | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |

## Persisted Artifacts

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/source_constraints.json` | 1,029 | `fd7e8f66e529ea740d96dafecde90b7e2f37e6ad48fee614a4ac804f756134ad` | - |
| `out/input_scope.json` | 1,445 | `03ae1c3871dde485fb0ee494026e8723365ac2b042f8e3abf04a27fcafa852fc` | - |
| `out/drug_safety_summary.json` | 848 | `967fda0a7a78121e6a460ecf7cb835e056f5cde3b4084a28544ebbb8d0757107` | - |
| `out/drug_safety_terms.jsonl` | 33,114 | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` | 14 |
| `out/parsed_safety_rows.jsonl` | 167,725 | `b56926bffe2c580c141a74e701c8f0535195d06f33d9c875812729e36f40d167` | 353 |
| `out/mapped_candidate_safety.jsonl` | 18,286 | `1f0eb4b787c708f5e87c5238d905ca5015b448db638866a2c4b538020dbc54e7` | 26 |
| `out/candidate_safety_flags.jsonl` | 12,305 | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` | 13 |
| `out/raw_query_manifest.jsonl` | 7,426 | `5895c450130d3000a5ffe1ff6031b0f3d4243fdef4970180b3ce8862f0adc307` | 28 |
| `out/output_manifest.json` | 8,051 | `e69b1f5fe89948d10aa472296ae982e99ec431f7ef67fe081fccfaec5f7b3f83` | - |
| `out/persisted_readback.json` | 1,300 | `98ceeb5277f310f32f25f7ef74f15a1d2830aa0c178ccd979f10c67af7fbf52b` | - |

Raw FDA responses are persisted under:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/raw/openfda_label
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/raw/openfda_event
```

## Drug-Term Flags

| Drug term | Label | FAERS | FAERS total reports | Representative flags |
|---|---|---|---:|---|
| Cytarabine | yes | yes | 62,318 | boxed warning; contraindications; serious/death reports |
| Doxorubicin | yes | yes | 107,484 | boxed warning; contraindications; serious/death reports |
| Gemcitabine | yes | yes | 56,395 | contraindications; warnings; serious/death reports |
| Prednisolone | yes | yes | 193,614 | contraindications; warnings; serious/death reports |
| Selumetinib | yes | yes | 539 | contraindications; interactions; serious/death reports |
| Metformin | yes | yes | 425,794 | boxed warning; contraindications; serious/death reports |
| Vemurafenib | yes | yes | 4,046 | contraindications; interactions; serious/death reports |
| AZ628 | no | no | 0 | source unavailable fail-closed |
| Exemestane | yes | yes | 12,644 | contraindications; interactions; serious/death reports |
| JQ1 Compound | no | no | 0 | source unavailable fail-closed |
| Mirdametinib | yes | yes | 11 | contraindications; warnings; serious reports |
| Sirolimus | yes | yes | 13,671 | boxed warning; contraindications; serious/death reports |
| Trametinib | yes | yes | 6,718 | contraindications; interactions; serious/death reports |
| VTX-11e | no | no | 0 | source unavailable fail-closed |

The FAERS total is a report-count field from the openFDA event API, not an
incidence/prevalence or causality estimate.

## Candidate Ranker Flags

Every drug-bearing candidate receives:

```text
clinical_promotion_block_until_safety_review_complete
```

Additional ranker blocks are attached from the mapped drug evidence:

| Candidate | Therapies | Ranker blocks |
|---|---|---|
| `oncology-civic:11176` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1958` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:10138` | Metformin; Exemestane | high-risk label section; FAERS serious/death reports |
| `oncology-civic:7487` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1470` | Vemurafenib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1230` | Metformin; Trametinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1469` | Sirolimus; Mirdametinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1743` | JQ1 Compound | safety source unavailable |

All candidate-level rows are in:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/candidate_safety_flags.jsonl
```

## Conclusion

#1181 is complete for the current #1185 drug-hypothesis surface:

- public FDA label and FAERS/openFDA queries were persisted for all 14 therapy
  terms;
- safety terms, parsed safety rows, mapped candidate rows, and candidate
  ranker flags were written separately;
- source-unavailable cases fail closed;
- every drug-bearing candidate is blocked from clinical promotion until safety
  review and later validation clear.

This improves the association atlas by preventing candidate ranking from
silently treating missing or adverse safety evidence as harmless.

---

## 50_oracle_honesty_ci_low_gate.md

# #1204 Oracle Honesty Gate CI-Low / Calibration Hardening

## Scope

#1204 fixes the Oracle honesty gate so sufficiency is decided from the calibrated
lower-bound basis, not the MI point estimate.

Before this slice, the vault-backed oracle path loaded only `estimate.bits` from
assay rows and rebuilt a diagnostic sufficiency report. That discarded
`MiEstimate.ci_low` and `PowerCalibration`, so a panel could pass when its point
estimate cleared entropy even if the lower bound did not.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-assay/src/sufficiency.rs` | added context-preserving `panel_sufficiency_from_estimate_with_context` and kept sufficiency basis from `estimate.ci_low` |
| `crates/calyx-assay/src/sufficiency/joint.rs` | moved panel-joint union-floor helper out of `sufficiency.rs` to keep linecount under 500 |
| `crates/calyx-assay/src/lib.rs` | exported the new context-preserving sufficiency constructor |
| `crates/calyx-oracle/src/honesty_gate.rs` | gate now compares `sufficiency_basis_bits >= anchor_entropy_bits`; vault path loads the full panel `MiEstimate` and requires assay calibration |
| `crates/calyx-oracle/src/honesty_gate_tests.rs` | added FSV tests for point-estimate pass / lower-bound fail and missing calibration fail-closed |

Behavior now enforced:

- `bits >= H` but `ci_low < H` returns `CALYX_ORACLE_INSUFFICIENT`.
- Missing panel power calibration returns `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Passing requires `ci_low >= anchor_entropy_bits` and
  `PowerCalibrationStatus::Passed`.
- The bound exposed by the oracle uses the same lower-bound basis used for the
  pass/fail decision.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1204-oracle-honesty-ci-low-20260704T031838Z
```

Persisted readback:

| Field | Value |
|---|---|
| Status | `ok` |
| `cargo test -p calyx-oracle honesty_gate -- --nocapture` | exit 0 |
| `cargo test -p calyx-assay sufficiency -- --nocapture` | exit 0 |
| `cargo check -p calyx-oracle` | exit 0 |
| `cargo check -p calyx-assay` | exit 0 |
| `bash scripts/linecount.sh` | exit 0 |
| `git diff --check` | exit 0 |
| `persisted_readback.json` SHA-256 | `2c928a7318255c9f2c87e96a93356c4a5e0884551ae098c3b1f6f5a3e2fdc7af` |

FSV log files:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `calyx-oracle-honesty-gate.stdout.log` | 1,383 | `a93f88ad8183032bba78554705a8dd462ce882b2950d6398def9b73cca1f2685` |
| `calyx-oracle-honesty-gate.stderr.log` | 716 | `bc716fb042e502ac42bc17bfc004cebb3ad6d3511fd644ec17cf0e0533ffcc2a` |
| `calyx-assay-sufficiency.stdout.log` | 2,602 | `3dcabc26c524aa14de8e61148c5e07f048afcbcade8e6970e7ccaed4cc4bc1b6` |
| `calyx-assay-sufficiency.stderr.log` | 2,318 | `c7a113dd558492bfdfb85cc19c9be14b424db65cc8946fb1b459e1f54628687d` |
| `calyx-oracle-check.stderr.log` | 601 | `f5ea4994ddd4ea009e3fa7bbf6c0d2d12bec9039f23df6c2f1e4b2a97fc6eac0` |
| `calyx-assay-check.stderr.log` | 225 | `d902a2e263a3eddc599dc85a1e3ed971a09c4f6ab7a07eabb1996b4eb1492e3f` |
| `linecount.stdout.log` | 26 | `2ca9608a7e23755e5f4038d3d0e6ae4482f4acf00be03489434b68af163e79f1` |

The oracle tests read the assay rows back through `VaultSufficiencyAssay`, so
the source of truth for the pass/fail decision is the persisted `AssayStore`
inside the vault, not a direct return value from a hand-built report.

## Conclusion

#1204 is complete for the oracle honesty gate:

- point-estimate-only sufficiency no longer passes;
- missing/underpowered calibration fails closed;
- the assay constructor and oracle gate now use the same lower-bound basis;
- the linecount split keeps the repository under the enforced structural gate.

This hardens the trust boundary before additional biomedical hunts consume
oracle sufficiency verdicts.

---

## 51_discovery_chain_sufficiency_gate.md

# #1205 Discovery-Chain Sufficiency Gate Hardening

## Scope

#1205 fixes the discovery-chain trust boundary. Before this slice, the default
`run_grounded_discovery_chain` path accepted hops from topological
anchor-reachability alone. A node within the configured graph radius could pass
without any calibrated assay evidence that the panel carried enough bits about
the outcome.

This is a safety/trust fix for biomedical discovery. It does not make any
clinical claim or cure claim. It prevents chain-walk hypotheses from being
labeled grounded unless a power-calibrated sufficiency instrument is present.

## Code Change

Changed behavior:

- `run_grounded_discovery_chain` and `run_grounded_chain_walks` now fail closed
  with `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY` after input validation.
- The topology-only predicate was renamed/exported as `reachability_prior_gate`.
  It is diagnostic/ranking evidence, not a sole gate.
- Injected-gate APIs remain available:
  - `run_discovery_chain_with_gate`
  - `run_chain_walks_with_gate`
- Accepted hops now persist `gate_code` and `gate_evidence`, including the
  sufficiency lower bound evidence.
- `calyx discovery-chain` and `calyx chain-walks` now load:
  - current manifest-backed panel,
  - persisted `AssayStore` rows from the vault,
  - `Panel`, `OutcomeEntropy`, and per-lens rows scoped by vault id, panel
    version, assay domain, and anchor kind.
- CLI gates pass only when:
  - `ci_low >= anchor_entropy_bits`,
  - the panel estimate carries passing power calibration,
  - the reachability prior also passes.

New CLI flags:

```text
--assay-domain <domain>
--assay-anchor <reward|label:name|test_pass|tie_formed|thumbs|speaker_match|style_hold|recurrence>
```

Defaults remain `discovery-chain` and `reward`.

## FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1205-discovery-chain-sufficiency-gate-20260703T224122Z
```

Readback artifacts:

| Artifact | Purpose |
|---|---|
| `issue878_discovery_chain_readback.json` | Lodestar discovery-chain readback; accepted hops carry `ci_low=1.100000` and `anchor_entropy_bits=1.000000` |
| `issue880_chain_walks_readback.json` | Chain-walk report readback; hypothesis provenance carries sufficiency evidence |
| `cli_discovery_chain.log` | Physical CLI vault test: manifest panel + assay CF rows, persisted `chain.json` read back |
| `cli_chain_walks.log` | Chain-walk parser/token gate for assay keying flags |
| `check_lodestar.log` | `cargo check -p calyx-lodestar` |
| `check_cli.log` | `cargo check -p calyx-cli` |
| `linecount.log` | `bash scripts/linecount.sh` |
| `diffcheck.log` | `git diff --check` |

Commands passed:

```text
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture
cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture
cargo test -p calyx-cli discovery_chain -- --nocapture
cargo test -p calyx-cli chain_walks -- --nocapture
cargo check -p calyx-lodestar
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Edge cases covered:

- no injected library sufficiency gate -> `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY`;
- missing persisted assay rows -> `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY`, no
  chain artifact;
- persisted panel assay with `ci_low < H` -> refused, no chain artifact;
- strict reachability-prior threshold -> no accepted chain artifact;
- unknown start node still fails as `CALYX_GRAPH_UNKNOWN_NODE`;
- accepted physical CLI chain persists and reads back every accepted hop with
  `CALYX_DISCOVERY_SUFFICIENCY_PASS`, `ci_low`, `anchor_entropy_bits`, and
  `power_calibration=passed`.

## Conclusion

#1205 is complete for the discovery-chain and chain-walk trust boundary:
topology can rank and explain proximity, but it can no longer assert grounded
acceptance by itself. A chain now needs a calibrated sufficiency gate, and the
accepted-hop evidence is persisted for readback.

---

## 52_falsification_asserted_relation_gate.md

# 52 - #1206 Falsification asserted-relation gate

## Scope

#1206 hardens `calyx hypothesis-falsification-sweep` so support/counter
evidence can only attach to a hypothesis when both endpoints are present in the
same structured asserted-relation fields. Whole-row substring co-mention is no
longer a validation or falsification gate.

This remains hypothesis triage only. It is not efficacy, safety, actionability,
or cure evidence.

## Code Change

Commits:

```text
42868abc Harden falsification evidence matching
6aed862a Allow concept ids to match asserted endpoint labels
```

Matcher behavior after this change:

- PubTator rows match only `left_id/left_term/left` and
  `right_id/right_term/right`.
- ClinicalTrials seed summaries match only `intervention` and `condition`.
- ClinicalTrials trial rows match only `query_intervention/intervention` and
  `query_condition/condition`.
- DGIdb rows match only `source_overlay_id/drug/drug_name` and
  `target_overlay_id/gene/gene_name`.
- Open Targets rows match only `overlay_target_concepts/target_id/target_name`
  and `overlay_disease_concepts/disease_id/disease_name`.
- Rows with classifiable support/counter polarity but missing asserted endpoint
  fields are persisted to `skipped_evidence.jsonl` with
  `CALYX_FALSIFY_UNSTRUCTURED_ROW`; they are not counted.
- External identifiers such as CHEMBL/HGNC must match endpoint identifiers when
  endpoint identifiers are present. Calyx internal `concept:*` ids may still
  match exact structured endpoint labels such as `@GENE_CD4`.

The persisted schema is now version 2 and includes:

- `skipped_evidence_count`
- `skipped_evidence`
- `skipped_evidence.jsonl` plus hash in CLI summary output

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused regression coverage:

- stopped trial co-mentions a target disease in free text but asserts a
  different intervention-condition pair: no counter-evidence is counted;
- CHEMBL/HGNC exact-id match accepts only the row with the matching endpoint id,
  not a different CHEMBL id sharing a namespace or numeric-looking suffix;
- `concept:ncbi_gene:*` hypotheses match PubTator `@GENE_*` structured endpoint
  labels by exact endpoint label;
- unstructured classifiable row is skipped with
  `CALYX_FALSIFY_UNSTRUCTURED_ROW` and counts stay unchanged.

Synthetic FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1206-falsification-asserted-relation-20260703T225431Z
```

Synthetic readback:

```json
{
  "schema_version": 2,
  "support_evidence_count": 3,
  "counter_evidence_count": 1,
  "skipped_evidence_count": 1,
  "kidney_counter_count": 0,
  "chembl_support_source_row_index": 2,
  "skipped_reason_code": "CALYX_FALSIFY_UNSTRUCTURED_ROW"
}
```

## Real-Data FSV

Release build and full persisted-source sweep were run on `aiwonder` after
fast-forwarding to `6aed862a`.

FSV root:

```text
/home/croyse/calyx/fsv/issue1206-falsification-asserted-relation-20260704T035957Z
```

Inputs:

| Input | Path |
|---|---|
| #1183 broad report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json` |
| #1183 chemical/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json` |
| #1183 gene/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |

Readback summary:

| Field | Value |
|---|---:|
| Input hypothesis rows | 301 |
| Deduped hypotheses | 280 |
| Support evidence rows retained | 8 |
| Counter-evidence rows retained | 0 |
| Skipped evidence rows | 0 |
| Evidence rows relation-readback verified | 8 |
| Invalid relation evidence rows | 0 |
| Hypotheses flagged with counter-evidence | 0 |

Artifact hashes:

| Artifact | SHA-256 |
|---|---|
| `falsification_sweep_report.json` | `5220ad982a2af59ca9e7fe82c1801f9d6f77aa55023ea4f1df0b5696f918b25a` |
| `support_evidence.jsonl` | `f082960a20d5cd8b4c294fab093ed24e1347ebb8ac17cb3c185a3264ff3dabaa` |
| `counter_evidence.jsonl` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `skipped_evidence.jsonl` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `hypothesis_flags.jsonl` | `fef7ba1531d4bc7ca26873221ea7a4acf2d8c7560ae021ea0a1a4c86208b4b83` |
| `raw_query_manifest.jsonl` | `bd2c94ae621a089cea8cac7826284ba3d00d5065d3b04928f5b07ca5ceef3bbf` |

Separate readback reopened every cited `source_row_index` from the persisted
source rows and verified both hypothesis endpoints against the same asserted
relation record. Example retained evidence:

```text
typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D011507
source_name=PLA2R1 target_name=Proteinuria
source_row left=@GENE_PLA2R1 right=@DISEASE_Proteinuria
```

The prior #1184 sweep counted 59 support rows and 1 counter row under whole-row
co-mention matching. The stricter asserted-relation gate retained 8 support rows
and removed the counter row. The removed counter was Open Targets row 705:

```json
{
  "target_name": "PLA2R1",
  "disease_name": "diabetic kidney disease",
  "overlay_target_concepts": ["concept:ncbi_gene:22925"],
  "overlay_disease_concepts": [],
  "score": 0.03701863799150296
}
```

That row does not assert the previous hypothesis endpoint `Kidney Diseases` by
exact disease endpoint or overlay disease concept, so it is no longer valid
counter-evidence for PLA2R1 / Kidney Diseases.

## Conclusion

#1206 closes the false-validation/false-falsification hole in the sweep. The
remaining retained evidence rows are relation-field-backed; broad co-mentions
are excluded unless a source parser exposes a structured relation endpoint pair.

---

## 53_batch_ingest_provenance_gate.md

# 53 - #1211 Batch ingest source-provenance gate

## Scope

#1211 hardens streaming batch ingest so a JSONL row cannot enter Calyx without
minimum source provenance. This is the bulk path used by biomedical ingestion;
unguarded rows would become graph nodes without a source dataset, checksum,
license, retrieval timestamp, or stable locator.

This change proves source-traceability enforcement only. It does not validate a
biomedical association, treatment, safety decision, clinical action, or cure.

## Code Change

`crates/calyx-cli/src/cmd/ingest/batch.rs` now validates every non-blank batch
line during parser preflight, before opening the vault or initializing
measurement state.

Required non-empty metadata keys:

- `source_dataset`
- `source_sha256`
- `license`
- `retrieval_ts`
- at least one locator: `source_url`, `doi`, `pmid`, or `pmcid`

Missing or blank required provenance returns `CALYX_CLI_USAGE_ERROR` with the
line number and missing key. Extra provenance keys remain allowed and are
stored verbatim on the constellation metadata map.

The enforcement is intentionally presence-based, matching the sibling
`bridge_corpus` validation semantics for required source metadata.

## Test Updates

Batch test fixtures now write provenance-bearing rows instead of bare
`{"text": ...}` rows, except for explicit negative tests that prove the new gate
fails closed.

Focused coverage added:

- valid row ingests and its required metadata is read back from the persisted
  Base CF constellation;
- missing metadata object fails in parser preflight;
- blank `source_dataset` fails in parser preflight;
- missing `source_sha256` fails in parser preflight;
- no locator (`source_url`/`doi`/`pmid`/`pmcid`) fails in parser preflight;
- missing `license` fails in parser preflight;
- missing `retrieval_ts` fails in parser preflight;
- missing provenance against a non-existent vault fails before the vault path is
  created.

## Verification

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli batch_ -- --nocapture
cargo test -p calyx-cli batch_ingest_requires_and_persists_source_provenance -- --nocapture
cargo test -p calyx-cli batch_provenance_edge_cases_fail_in_parser_preflight -- --nocapture
cargo test -p calyx-cli missing_batch_provenance_fails_before_vault_open -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1211-batch-provenance-20260704T044300Z\issue1211-batch-provenance-gate
```

FSV artifacts:

| Artifact | Source of truth |
|---|---|
| `valid-row-base-cf-provenance-readback.json` | Aster Base CF readback via `vault.get(cx_id, snapshot)` after batch ingest flush |
| `parser-preflight-provenance-edge-cases.json` | `validate_batch_file` parser preflight errors |
| `missing-provenance-fails-before-vault-open.json` | missing-vault path existence checked after rejected `ingest_batch_streaming` |

Readback summary:

```json
{
  "valid_row": {
    "cx_id": "acf75bde73422e0100269e945b6f2891",
    "stored_metadata_keys": [
      "license",
      "retrieval_ts",
      "source_dataset",
      "source_sha256",
      "source_url"
    ]
  },
  "edge_cases": 6,
  "missing_vault_path_exists_after_error": false
}
```

During FSV, a broad `cargo test -p calyx-cli batch_` run with
`CALYX_FSV_ROOT` set exposed an unrelated older oracle-event FSV branch that
reuses a pre-index-rebuild snapshot and fails with
`CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE`. The same broad batch suite passes
without that test's FSV write branch enabled, and the three #1211 FSV-emitting
tests pass with `CALYX_FSV_ROOT` set.

## Conclusion

#1211 closes the ungrounded-row hole in the streaming batch ingest parser. Bulk
biomedical corpus rows now fail closed before vault open unless they carry the
minimum provenance needed to trace associations back to their source.

---

## 54_oracle_event_fsv_snapshot_gate.md

# 54 - #1215 Oracle-event FSV snapshot readback

## Scope

#1215 fixes an FSV-only failure in
`batch_ingest_structures_oracle_recurrence_for_reverse_query`. The normal test
body passed, but when `CALYX_FSV_ROOT` was set the artifact-writing branch reused
an earlier snapshot after the ingest/search-index path had advanced the vault.
That requested historical state from a latest-only recovered vault and failed
with `CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE`.

This is a verification-path fix. It does not establish biomedical efficacy,
safety, clinical actionability, or a cure.

## Code Change

The FSV branch now captures a fresh `fsv_snapshot = vault.snapshot()`
immediately before scanning Recurrence CF for the artifact. The report records
both snapshots:

- `initial_readback`: the snapshot used for the first Base/Recurrence assertions;
- `fsv_readback`: the current snapshot used for FSV artifact readback.

This keeps the FSV source of truth as persisted Aster CF bytes without asking a
latest-only recovery path for historical state it cannot serve.

## Verification

Passed:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z'
cargo test -p calyx-cli cmd::ingest::oracle_event_tests::batch_ingest_structures_oracle_recurrence_for_reverse_query -- --nocapture
```

Also passed:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z'
cargo test -p calyx-cli batch_ -- --nocapture
```

Repository gates:

```bash
cargo fmt -p calyx-cli
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

FSV artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z\issue885_oracle_event_readback.json
```

Readback summary:

```json
{
  "issue": 885,
  "fixed_by_issue": 1215,
  "recurrence": {
    "cf_rows": 1,
    "occurrences": 1,
    "first_t_secs": 1700000000
  },
  "reverse_query": {
    "cause_count": 1,
    "first_action_or_event": "What treats type 2 diabetes?",
    "first_domain": "endocrinology",
    "first_provisional": false,
    "first_confidence": 0.5
  },
  "snapshots": {
    "initial_readback": 4,
    "fsv_readback": 5
  }
}
```

## Conclusion

The oracle-event FSV branch now reads from a valid current snapshot and emits its
artifact under `CALYX_FSV_ROOT`. The broad batch suite also passes with FSV
artifact writing enabled, closing the caveat found during #1211 verification.

---

## 55_ksg_mixed_discrete_estimator.md

# 55 - #1207 Mixed continuous-discrete KSG estimator

## Scope

#1207 replaces the biased discrete-anchor path that one-hot encoded class
labels and fed them into the continuous KSG estimator. Discrete anchors are on
the biomedical discovery trust path: sufficiency is only as honest as
`I(panel; anchor)`.

This is an estimator/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-assay/src/ksg.rs` | `ksg_mi_continuous_discrete*` now uses a Ross-style mixed estimator: same-label kth continuous radius, all-sample continuous neighbor count, and class-size digamma correction |
| `crates/calyx-assay/tests/ksg_mixed_discrete_fsv.rs` | Added planted-signal, fail-closed small-class, and manual real-labeled dataset FSV coverage |

Behavior now enforced:

- Class labels are not converted to fake continuous one-hot vectors.
- Each point gets its kth-neighbor radius from same-class continuous samples.
- The estimator fails closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES` when any
  discrete label has `class_size <= k`.
- Grounded-anchor callers keep the public API and receive the same trust tag
  semantics as before.

Primary method references used for the estimator:

- Ross, "Mutual Information between Discrete and Continuous Data Sets", PLoS
  One 9(2):e87357 (2014): https://journals.plos.org/plosone/article/file?id=10.1371/journal.pone.0087357&type=printable
- Gao et al., "Estimating Mutual Information for Discrete-Continuous Mixtures",
  NeurIPS 2017: https://papers.neurips.cc/paper_files/paper/2017/file/ef72d53990bc4805684c9b61fa64a102-Paper.pdf

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z
```

Commands run:

```powershell
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv ksg_mixed_discrete -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z'
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv -- --nocapture

$env:CALYX_STAGE5_CLASSIFICATION_CSV='C:\code\Calyx-Dev\target\fsv\issue1207-real-dataset\iris.data'
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z'
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv ksg_mixed_discrete_real_labeled_dataset_delta_fsv -- --ignored --nocapture
```

Synthetic planted-signal artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z\issue1207-ksg-mixed-discrete\ross-mixed-discrete-readback.json
```

Readback summary:

```json
{
  "expected_entropy_bits": 1.5849625,
  "ross_mixed_small_scale_bits": 1.5909938,
  "ross_mixed_large_scale_bits": 1.5909938,
  "ross_scale_delta_abs": 0.0,
  "old_one_hot_small_scale_bits": 1.5909938,
  "old_one_hot_large_scale_bits": 0.0,
  "old_one_hot_scale_drop_bits": 1.5909938,
  "small_class_error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
}
```

Real labeled dataset FSV:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z\issue1207-real-labeled-ross-delta-readback.json
```

Source and CF readback:

```json
{
  "dataset": "UCI Iris",
  "dataset_blake3": "8578940c6c00041901b00392034412b20e2f574eba595bcc6b979ca4148178e6",
  "rows": 150,
  "anchor_entropy_bits": 1.5849626,
  "ross_mixed_bits": 1.1287328,
  "ross_mixed_ci_low": 0.8621819,
  "ross_trust": "trusted",
  "old_one_hot_bits": 1.8947722,
  "abs_delta_bits": 0.7660394,
  "ross_ci_low_clears_entropy": false,
  "persisted_assay_rows": 1,
  "loaded_assay_rows": 1
}
```

The real-data readback shows why the fix matters: the old one-hot path reports a
point estimate above the 3-class entropy, while the Ross lower bound does not
clear entropy. Any sufficiency decision consuming this path is now grounded on
the mixed estimator, not on the over-optimistic one-hot value.

## Conclusion

#1207 is complete for the assay estimator boundary. Discrete anchors now use a
mixed continuous-discrete estimator, deficient classes fail closed with a named
label/class size, and both synthetic and real-labeled readbacks persist the
evidence for the estimator delta.

---

## 56_ksg_subsample_ci.md

# 56 - #1208 KSG no-replacement subsample CI

## Scope

#1208 removes ordinary with-replacement bootstrap from the continuous KSG
confidence interval path. Replacement bootstrap creates duplicate rows, and KSG
interprets duplicate coordinates as fine-scale structure. That is unsafe for a
lower-bound sufficiency gate.

This is an estimator/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed file:

| File | Change |
|---|---|
| `crates/calyx-assay/src/ksg.rs` | Continuous KSG now computes CI from deterministic m-out-of-n no-replacement subsamples instead of paired replacement bootstrap |

Behavior now enforced:

- Continuous KSG CI draws distinct row indices for every resample.
- The duplicate-index invariant is checked at runtime and fails closed if it is
  ever violated.
- Subsample size is `floor(4n/5)`.
- The interval is widened by a deterministic coarse-grain allowance
  `abs(point_bits) * (1 - m/n)` so the reduced-size subsample does not become an
  over-tight safety bound.
- If the subsample cannot still satisfy the assay quorum
  (`m >= MIN_ASSAY_SAMPLES` and `0 < k < m`), KSG returns
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.
- The generic paired bootstrap helper remains available for non-KSG estimators.
- The #1207 mixed continuous-discrete path remains on its local-term CI; it does
  not reintroduce replacement-bootstrap duplicate points.

Primary method reference used:

- Holmes and Nemenman, "Estimation of mutual information for real-valued data
  with error bars and controlled bias" (2019):
  https://arxiv.org/pdf/1903.09280

The paper explicitly identifies replacement-bootstrap duplicates as KSG-visible
fine-scale/high-information artifacts and recommends subsampling rather than
ordinary bootstrap for KSG error bars.

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z
```

Command:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z'
cargo test -p calyx-assay ksg_no_replacement -- --nocapture
```

FSV artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z\issue1208-ksg-subsample-ci\ksg-subsample-ci-readback.json
```

Readback summary:

```json
{
  "independent_true_mi_bits": 0.0,
  "independent_point_bits": 0.02191284,
  "old_with_replacement_ci_low": 0.0,
  "old_with_replacement_ci_high": 2.1993752,
  "new_no_replacement_ci_low": 0.0,
  "new_no_replacement_ci_high": 0.2483080,
  "old_mean_duplicates_per_resample": 57.96,
  "old_max_duplicates_per_resample": 64,
  "new_no_replacement_duplicate_free": true,
  "subsample_m": 128,
  "small_sample_error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
}
```

Note: the repository's existing widened CI calculation already clamps the
independent-control old lower bound to zero in this fixture. The replacement
pathology is still physically visible in the resample interval: the old
replacement-bootstrap high bound is about 9x wider than the no-replacement
bound, and the old resamples average about 58 duplicate draws per 160-row
resample. The new path removes the duplicate source and keeps the independent
lower bound at zero.

Planted-signal coverage readback:

```json
{
  "samples": 180,
  "point_bits": 1.8794093,
  "known_gaussian_bits": 1.9886955,
  "covered_seed_count": 5,
  "seed_count": 5
}
```

Edge case:

```json
{
  "case": "n_just_above_min_but_subsample_below_min",
  "before": { "n": 60, "k": 3, "subsample_m": 48 },
  "after": {
    "error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
  }
}
```

## Conclusion

#1208 is complete for the continuous KSG CI path. The CI no longer creates
duplicate rows, the duplicate invariant is asserted, planted-signal intervals
cover the known value on the tested seeds, and small subsamples fail closed
instead of emitting a bound from a degenerate resample.

---

## 57_blind_spot_calibration.md

# 57 - #1209 Blind-spot calibration

## Scope

#1209 replaces the blind-spot sweep's hardcoded absolute delta gate with a
per-lens-pair empirical calibration. Blind-spot candidates seed downstream
biomedical hypothesis hunts, so the novelty signal must be comparable across
heterogeneous lens-pair similarity scales.

This is a ranking/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-loom/src/blind_spot.rs` | Added `BlindSpotCalibration`, calibrated alert evidence, and `detect_blind_spot_calibrated` |
| `crates/calyx-loom/src/error.rs` | Added `CALYX_LOOM_UNCALIBRATED_BLINDSPOT` |
| `crates/calyx-lodestar/src/blind_spot_sweep.rs` | Sweep now builds per-pair empirical delta distributions and skips uncalibrated pairs |
| `crates/calyx-loom/tests/blind_spot_calibration_fsv.rs` | Direct scale-normalization, null-FDR, and under-sampled edge FSV |
| `crates/calyx-lodestar/tests/issue875_blind_spot_sweep_tests.rs` | Sweep FSV updated to calibrated evidence |

Behavior now enforced:

- Alerts are emitted by one-sided empirical `p_value <= alpha`, not by
  `delta >= 0.5`.
- Alert evidence carries `sample_count`, `threshold_delta`, `percentile`,
  `p_value`, `alpha`, and scale-free `score`.
- Sweep ranking uses calibrated score when present.
- Pair calibration defaults to `min_samples=50`, `alpha=0.05`.
- Under-sampled pairs skip with explicit uncalibrated accounting instead of
  applying the old global threshold.

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z
```

Commands:

```powershell
cargo test -p calyx-loom --test blind_spot_calibration_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z'
cargo test -p calyx-loom --test blind_spot_calibration_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z'
cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests writes_fsv_readback_when_root_is_set -- --nocapture

cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture
cargo check -p calyx-loom
cargo check -p calyx-lodestar
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Direct Loom calibration artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z\issue1209_blind_spot_calibration_readback.json
SHA256 680CA10E50CEA928D2E3B107987AA483166A659F7A4BE84FAE5403D8BD1E7DE5
```

Readback summary:

```json
{
  "params": { "min_samples": 50, "alpha": 0.05 },
  "compressed_delta": 0.18,
  "wide_delta": 0.98,
  "compressed_percentile": 1.0,
  "wide_percentile": 1.0,
  "compressed_p_value": 0.016666668,
  "wide_p_value": 0.016666668,
  "legacy_compressed_alert": false,
  "legacy_wide_alert": true,
  "null_alert_count": 3,
  "null_sample_count": 60,
  "under_sampled_error": "CALYX_LOOM_UNCALIBRATED_BLINDSPOT"
}
```

Calibrated Lodestar sweep artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z\issue875_blind_spot_sweep_readback.json
SHA256 D61031DE680D87FB5AD390A6E52EAB09CD621817BF892927DC495229F7DC1578
```

Sweep readback summary:

```json
{
  "observation_count": 64,
  "detected_alert_count": 3,
  "uncalibrated_observation_count": 0,
  "gate_refused_count": 1,
  "severity_filtered_count": 1,
  "candidate_count": 1,
  "top_delta": 0.98,
  "top_percentile": 1.0,
  "top_p_value": 0.015625,
  "top_threshold_delta": 0.66
}
```

## Conclusion

#1209 is complete for the blind-spot detector and sweep path. The production
sweep no longer ranks novelty from a global absolute delta; it ranks from
per-lens-pair empirical calibration and fails closed for uncalibrated pairs.

---

## 58_weighted_graph_csr.md

# 58 - #1213 Weighted graph CSR evidence edges

## Scope

#1213 fixes the plain-graph CSR projection used by the biomedical evidence
overlay. Before this change, the projection discarded the stored edge evidence
payload and every `AssocGraph` edge was rebuilt as weight `1.0`.

This is a graph scoring/ranking fix. It does not establish biomedical efficacy,
safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-aster/src/plain_graph/types.rs` | Added CSR edge `weight` plus strict edge-value weight parsing |
| `crates/calyx-aster/src/plain_graph/mod.rs` | CSR projection and in-process scan fallback now normalize positive support weights |
| `crates/calyx-aster/src/plain_graph/physical.rs` | Physical no-CSR fallback now reads Graph CF edge values before building `AssocGraph` |
| `crates/calyx-aster/src/plain_graph/assoc_graph.rs` | Persisted CSR decode now validates and applies per-edge weights |
| `crates/calyx-aster/src/plain_graph/csr_store.rs` | CSR manifest version bumped to 3 for the weighted edge schema |
| `crates/calyx-cli/src/cmd/evidence_substrate/write.rs` | Direct evidence-substrate CSR materializer now writes normalized weights |
| `crates/calyx-cli/src/cmd/lincs_reversal/write.rs` | Direct LINCS CSR materializer now writes normalized weights |
| `crates/calyx-aster/tests/issue1213_weighted_csr_fsv.rs` | Persisted weighted CSR/readback and downstream scoring FSV |

Weight derivation:

- Edge values must be a JSON number or a JSON object with numeric `weight`.
- The raw positive finite support values are normalized by the maximum support
  in the projection so `AssocGraph` receives weights in `(0,1]`.
- Empty, malformed, zero, negative, or non-finite values fail closed as
  `CALYX_GRAPH_CORRUPT_ROW`; there is no silent fallback to `1.0`.

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z
```

Commands:

```powershell
cargo test -p calyx-aster --test issue1213_weighted_csr_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z'
cargo test -p calyx-aster --test issue1213_weighted_csr_fsv -- --nocapture

cargo test -p calyx-aster plain_graph -- --nocapture
cargo test -p calyx-aster -- --nocapture
cargo check -p calyx-cli
cargo test -p calyx-cli evidence_substrate -- --nocapture
cargo test -p calyx-cli lincs_reversal -- --nocapture
cargo check -p calyx-aster
bash scripts/linecount.sh
git diff --check
```

Persisted readback artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z\issue1213_weighted_csr_readback.json
SHA256 B35BA5F7C81F702DF27C117AD5D214696C2E8C7F4F77FFC70F36C888EA6FA1E8
```

Readback summary:

```json
{
  "csr_edge_weights": [1.0, 0.1, 1.0, 1.0],
  "assoc_graph_edge_weights": [1.0, 0.1, 1.0, 1.0],
  "reach_scored": {
    "high_mid": 0.8999999761581421,
    "low_mid": 0.08999999612569809,
    "high_gt_low": true
  },
  "betweenness": {
    "high_mid": 0.16666666666666666,
    "low_mid": 0.0,
    "high_gt_low": true
  },
  "spectral": {
    "high_mid": 1.0,
    "low_mid": 0.6465868949890137,
    "high_gt_low": true
  },
  "edge_cases": {
    "empty_value": "CALYX_GRAPH_CORRUPT_ROW",
    "zero_weight": "CALYX_GRAPH_CORRUPT_ROW",
    "malformed_weight": "CALYX_GRAPH_CORRUPT_ROW"
  }
}
```

## Conclusion

#1213 is complete for the plain-graph CSR/`AssocGraph` scoring path. Persisted
biomedical graph overlays now carry per-edge evidence mass into traversal,
betweenness, and spectral scoring instead of flattening all edges to `1.0`.

---

## 59_hypothesis_evidence_bridge.md

# 59 - Hypothesis evidence bridge

- **Issue:** #1200
- **Date (UTC):** 2026-07-04
- **Status:** Implemented and FSV-backed for the chain-walk -> evaluator-input bridge.
- **FSV root:** `target/fsv/issue1200-hypothesis-evidence-20260704T053721Z`

## What changed

`calyx assemble-hypothesis-evidence <vault> --chain <chain.json> --out <input.json>`
now materializes the JSON input consumed by `calyx hypothesis-evaluate`.

The bridge:

- reads chain-walk hypotheses from the persisted chain artifact;
- reads each required A/B/C/path `CxId` from the physical Calyx vault;
- builds deterministic `RetrievedEvidence` rows with `source_cx_id`, title, persisted
  abstract/text, grounding confidence, and provenance including `source_sha256`;
- dedupes repeated CxIds deterministically;
- fails closed on missing Base rows, missing `source_sha256`, or empty abstract/text.

## Raw evidence

Focused checks:

```text
cargo test -p calyx-lodestar hypothesis_evidence -- --nocapture
cargo test -p calyx-cli cmd::hypothesis_evidence -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo check -p calyx-lodestar
cargo check -p calyx-cli
git diff --check
bash scripts/linecount.sh
```

End-to-end FSV:

```text
CALYX_HOME=target/fsv/issue1200-hypothesis-evidence-20260704T053721Z/home
calyx create-vault issue1200-evidence --panel-template text-default
calyx ingest issue1200-evidence --batch batch.jsonl --idempotent --output rows
calyx assemble-hypothesis-evidence issue1200-evidence \
  --chain chain_walks.synthetic.json \
  --out hypothesis-evaluate.input.json
```

Readback summary:

- `input_count=1`
- `evidence_count=3`
- duplicate terminal `B` deduped to one evidence row
- `all_expected_matches=true`
- `chain_sha256=48973c1983538175275df467b694bc4c31546330d2acc00c24ab2f59299c47a3`
- `output_sha256=e80c22cfe4f29e447f267104dac4b2883d54618146d98ec55f84fd72392a110d`
- `readback_summary_sha256=76ba3ef12d00769104328d600359e2ab2b617213849a06af5d642b01e633fdcb`

## Boundary

This bridge removes the manual JSON-authoring bottleneck for evaluator evidence.
It does not make biomedical hypotheses clinically actionable. Hypotheses remain
evidence-backed research leads until outcome validation, falsification, safety,
and human review gates pass.

---

## 60_hypothesis_evaluator_driver.md

# Hypothesis Evaluator Driver

Issues: #1201, #1216

`calyx hypothesis-evaluator-driver` replaces manual evaluator JSON plumbing for
biomedical A-B-C hypotheses. It consumes the persisted evidence bundle produced
by `calyx assemble-hypothesis-evidence`, calls a configured structured evaluator
endpoint for a versioned prompt set, validates evidence citations, aggregates
the resulting evaluator runs, and writes one replayable artifact.

## Command

```text
calyx hypothesis-evaluator-driver \
  --input <input.json> \
  --out <artifact.json> \
  --endpoint <http(s)://host[:port]/path> \
  --model <id> \
  [--auth-env <VAR>] \
  [--temperature <f>] \
  [--timeout-ms <ms>]
```

Default temperatures are `0.2` and `0.8`, giving two prompt templates and two
temperature variants per hypothesis. `http://` endpoints are supported for
local deterministic stubs. `https://` endpoints are OpenAI-compatible and
require `--auth-env <VAR>`; the named environment variable must contain the
bearer token. The token is never persisted or printed, and persisted endpoint
values are redacted to remove userinfo, query strings, and fragments. The
endpoint contract is OpenAI-like JSON or direct evaluator JSON. The persisted
artifact records:

- schema version
- model id
- endpoint
- prompt set id
- prompt set SHA-256
- temperatures
- source input path and SHA-256
- input hypotheses with generated evaluator runs
- aggregate `HypothesisEvaluationReport`

## Fail-Closed Contract

The driver refuses to persist an artifact when any evaluator variant fails:

- endpoint unreachable:
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_UNREACHABLE`
- missing HTTPS bearer-token environment configuration:
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`
- HTTPS bearer-token rejected:
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_FAILED`
- HTTPS endpoint returned a non-auth non-2xx status:
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_STATUS`
- malformed evaluator response:
  `CALYX_HYPOTHESIS_EVALUATOR_MALFORMED_RESPONSE`
- cited evidence id missing from the input bundle:
  `CALYX_HYPOTHESIS_EVALUATOR_BAD_CITATION`

Each variant failure includes the hypothesis id, prompt id, and temperature.
This keeps ranking evidence tied to physical Calyx evidence rows and prevents an
LLM response from smuggling in uncited support.

## Safety Boundary

Evaluator output is a grounded research-lead score, not a cure claim or clinical
recommendation. Clinical actionability still requires outcome anchors,
falsification against independent real datasets, mechanism checks, safety
evidence, and human review gates.

## Verification

Focused tests:

```text
cargo test -p calyx-cli cmd::hypothesis_evaluator -- --nocapture
cargo test -p calyx-cli cmd::hypothesis_evidence -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
```

Physical FSV artifact:

```text
target/fsv/issue1201-hypothesis-evaluator-20260704T055120Z/readback-summary.json
```

The FSV run used a local stub endpoint, invoked the real CLI, persisted
`driver-artifact.json`, and read it back. Readback observed four endpoint calls,
four evaluator runs, one aggregate evaluation, prompt set
`biomed_hypothesis_evaluator_v1`, prompt hash
`a5ee6f1d827b6991bdbe880c36a92f9965beef7ed2d34072674a43d2f6d34a36`, and
artifact SHA-256
`1946f486c768e146bd11afbc92f0cc2e76813d95d47de6ead3f9c4bcab48444b`.

---

## 61_discovery_run_manifest.md

# Discovery Run Manifest

Issue: #1202

Lodestar now has a ledger-sealable discovery-run manifest for binding a
biomedical discovery atlas build into one reproducible hash chain.

## Model

`DiscoveryRunManifest` records:

- schema version
- run id
- corpus vault id
- panel manifest SHA-256
- ordered discovery stages

Each `DiscoveryRunStage` records:

- stage id
- command
- args
- optional upstream stage id
- input SHA-256
- output SHA-256
- git SHA

The chain is fail-closed. For every stage after the first, the stage input hash
must match either the explicitly named upstream stage output hash or the
previous stage output hash. A mismatch returns
`CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`.

## Ledger Seal

`seal_discovery_run_manifest` appends the manifest under `EntryKind::Assay` in
the Calyx ledger. The payload stores the manifest SHA-256 and the ordered stage
hashes, so normal ledger verification covers the discovery-run seal.

CLI:

```text
calyx discovery-run seal --manifest <manifest.json> --ledger <ledger-dir> --out <seal.json>
calyx discovery-run verify --manifest <manifest.json> --ledger <ledger-dir> --seq <n> --out <verify.json>
calyx discovery-run reproduce --manifest <manifest.json> --observed <observed.json> --out <report.json>
```

## Reproduction Check

`reproduce_discovery_run_manifest` compares observed stage output hashes against
the manifest. Missing or changed outputs fail closed with
`CALYX_DISCOVERY_RUN_MANIFEST_DRIFT`.

## Verification

Focused tests:

```text
cargo test -p calyx-lodestar --test issue1202_discovery_run_manifest_tests -- --nocapture
cargo check -p calyx-lodestar
git diff --check
bash scripts/linecount.sh
```

Physical FSV:

```text
target/fsv/issue1202-discovery-run-manifest-20260704T062200Z/issue1202_discovery_run_manifest_readback.json
target/fsv/issue1217-discovery-run-cli-20260704T063100Z/issue1217_discovery_run_cli_readback.json
```

The readback observed five chained stages, a ledger payload with
`stage_count=5`, `verify_chain=Intact { count: 1 }`, manifest SHA-256
`ff50eb0b0b44d49f13726914e823bdf81d8052a7bc6b2dadd3bed08e692f4a79`, and
readback artifact SHA-256
`79D5A3440DD00216D6F09D938598D9A6972F5C0882F45EB2DD72992DFCBA1309`.

The CLI FSV observed manifest SHA-256
`afcaa729bce5273cadf96bceb06c16d9a6da1ffb6f7ca43d10f5b9762a4b8a88`,
`seal_verify_chain=Intact { count: 1 }`, `verify_chain=Intact { count: 1 }`,
and readback summary SHA-256
`B939F2E251482E353AA7E4E30AF322E7B4C0828A723238651875CE8FEFBBFE08`.

## Remaining Operator Surface

The core manifest, ledger seal, and CLI wrapper are available. Per-stage
preflight enforcement remains separate so the individual stage commands can
adopt the manifest contract without a broad, high-risk edit.

---

## 62_discovery_manifest_redaction.md

# 62 - Discovery manifest ledger redaction

- **Issue:** #1221   **Phase:** 4   **Date (UTC):** 2026-07-04   **Vault/panel:** discovery-run manifest / ledger
- **Goal:** Let ledger-sealed discovery-run manifests carry benign long provenance tokens while preserving fail-closed rejection of secret-like payloads.

## What was run (exact commands)

```text
cargo test -p calyx-ledger redaction -- --nocapture
cargo test -p calyx-cli cmd::discovery_run -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Physical FSV:

```text
target/fsv/issue1221-discovery-manifest-redaction-20260704T070413Z/issue1221_fsv_readback_summary.json
```

## Raw evidence / FSV

The failing condition from #1219 was `CALYX_LEDGER_SECRET_IN_PAYLOAD` when a discovery-run manifest ledger payload contained full git SHA and descriptive run/corpus/stage provenance tokens.

The fix admits only bounded discovery manifest provenance token shapes:

- `git_sha`: 7 to 40 hex characters.
- `run_id`, `corpus_vault_id`, `stage_id`, `upstream_stage_id`, `command`: bounded slug-like manifest provenance with separators.

The secret scanner still rejects secret-like fields and opaque long tokens. Focused tests added:

- `check_payload_handles_discovery_manifest_tokens`
- `seal_accepts_long_benign_manifest_provenance_tokens`
- `seal_rejects_secret_like_manifest_token_without_artifact`

Physical readback observed:

- benign manifest `seal_verify_chain=Intact { count: 1 }`
- benign manifest `verify_chain=Intact { count: 1 }`
- ledger payload read back the full `run_id`, `corpus_vault_id`, and full 40-character `git_sha`
- secret-like manifest failed with `CALYX_LEDGER_SECRET_IN_PAYLOAD`
- secret-like seal output was not written

## Findings (honest)

- Grounded engineering finding: the ledger scanner was not distinguishing structured discovery manifest provenance from opaque no-space secret material.
- Grounded behavior after patch: long benign discovery manifest provenance seals and reads back through the ledger payload.
- Guard preserved: a secret-like manifest token returns `CALYX_LEDGER_SECRET_IN_PAYLOAD` and no seal artifact is written.

## Conclusion & next step

#1221 removes a reproducibility blocker for production biomedical discovery manifests. This does not change the clinical boundary: discovery-run manifests prove provenance and reproducibility, not clinical actionability, efficacy, safety, dosing, or cures.

---

## 63_native_discovery_bridges.md

# 63 - Native discovery bridge CLIs

- **Issue:** #1220   **Phase:** 4   **Date (UTC):** 2026-07-04   **Vault/panel:** #1219 real discovery-run artifacts
- **Goal:** Replace run-local hand-authored bridge JSON with native deterministic CLIs between falsification, evaluation, and ranking.

## What changed

Added two native commands:

```text
calyx bridge-falsification-evaluate --miner-report <json> --falsification-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
calyx bridge-evaluate-rank --evaluation-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
```

Both commands:

- read persisted predecessor artifacts from disk;
- run discovery-run preflight against the exact input bytes before output;
- write the downstream input JSON accepted by `hypothesis-evaluate` or `hypothesis-rank`;
- write a sibling `.readback.json` with source paths, source SHA-256 values, output SHA-256, counts, preflight readback, `research_lead_only=true`, and the no-clinical-actionability boundary.

## Verification

Focused checks:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::discovery_bridge -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip -- --nocapture
cargo test -p calyx-cli cmd::discovery_run -- --nocapture
cargo check -p calyx-cli
git diff --check
```

Physical FSV:

```text
target/fsv/issue1220-native-discovery-bridges-20260704T073437Z/issue1220_fsv_readback_summary.json
```

The physical run consumed the real #1219 predecessor artifacts:

- miner report SHA-256: `5d170fa287ef0394c38476b714465612dbfdea0ed1d299e136bf0dac649b4b28`
- falsification report SHA-256: `95394107e6f563217017662bd457329f5bc0ecad87676af417122299fe65a780`

Readback leaves:

- `bridge_falsification_evaluate.readback_input_count=1`
- `hypothesis_evaluate.input_count=1`
- `hypothesis_evaluate.retained_count=1`
- `bridge_evaluate_rank.readback_input_count=1`
- `hypothesis_rank.ranked_count=1`
- `hypothesis_rank.top_hypothesis_id=typed-assoc:concept:drug::concept:disease`
- stale bridge preflight failed with `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`
- stale bridge output was not written

## Boundary

The bridge creates reproducible downstream inputs from grounded association artifacts. It does not create clinical actionability, efficacy, safety, dosing, or cure evidence. The ranked output remains a research lead requiring outcome, safety, and human-review gates before claim escalation.

---

## 64_hypothesis_evaluator_https_provider.md

# Hypothesis Evaluator HTTPS Provider

Issue: #1216

Status: complete FSV.

`calyx hypothesis-evaluator-driver` now supports OpenAI-compatible `https://`
evaluator endpoints in addition to local deterministic `http://` stubs. HTTPS
transport requires `--auth-env <VAR>`; the named environment variable is read at
runtime and used only as an in-memory bearer token.

## Contract

- `http://` endpoints remain supported for local deterministic evaluator FSV.
- `https://` endpoints require `--auth-env <VAR>`.
- Missing or empty HTTPS auth fails closed with
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`.
- HTTP 401/403 from HTTPS fails closed with
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_FAILED`.
- Other HTTPS non-2xx responses fail closed with
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_STATUS`.
- Tokens are not persisted or printed.
- Persisted endpoint values are redacted to remove userinfo, query strings, and
  fragments.
- The evaluator artifact still records model id, endpoint, prompt set id/hash,
  temperatures, source input path/hash, input hypotheses, generated evaluator
  runs, and aggregate report.

## Verification

Focused gates:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::hypothesis_evaluator -- --nocapture
cargo check -p calyx-cli
```

The focused evaluator suite observed 6 passing tests, including HTTPS
mock-transport auth/redaction and missing-auth fail-closed coverage.

Physical FSV root:

```text
target/fsv/issue1216-https-evaluator-provider-20260704T074656Z
```

Readback summary:

```text
target/fsv/issue1216-https-evaluator-provider-20260704T074656Z/issue1216_fsv_readback_summary.json
```

Persisted artifact readback:

- input SHA-256:
  `fc065b8fd0ff4a66f7fd3a26f781d7344b06d6b168b79c05535fe9439be86aab`
- evaluator artifact SHA-256:
  `8be2a9ee3d860ef50be971f192419847b605bb5a5913474f0304b348435e6394`
- CLI stdout SHA-256:
  `aeaabdd63c1fc20fa00dbf232150ef77c535838aee03c07aa5c368a3fb4c5bcd`
- prompt set:
  `biomed_hypothesis_evaluator_v1`
- prompt set SHA-256:
  `a5ee6f1d827b6991bdbe880c36a92f9965beef7ed2d34072674a43d2f6d34a36`
- input count: 1
- evaluator run count: 4
- aggregate evaluation count: 1
- cited evidence readback: `evidence-1`

HTTPS negative readback:

- output artifact existed: `false`
- error stream contained `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`
- redaction checks found no `api_key=secret` query leak
- redaction checks found no `token@example.invalid` userinfo leak

## Safety Boundary

This is a transport and provenance hardening slice only. Evaluator output remains
a ranked research lead over cited evidence. It is not a cure claim, clinical
recommendation, or clinical actionability proof.

---

## 65_association_native_doctrine_context_update.md

# Association-Native Doctrine and Context Update

Issues: #863, #860, #867, #1214

Status: complete documentation/context update.

The builder handbook has been folded into the repo-level doctrine at
`docs/CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md`.

## Added Doctrine

- The binding method is `atoms -> all base associations -> differentiate ->
  kernel -> compose`.
- Bounded runs must declare their scope and must not imply full coverage.
- Explicit structured values use deterministic encoders; latent content uses
  embedders; hybrid records carry both in no-flatten constellations.
- Missing data classes create data-acquisition or Calyx-capability tasks.
- Association evidence remains typed: co-mention, drug-target, target-disease,
  reversal, safety, trial, literature, and other instruments stay distinct.
- Composition surfaces must name the kernel or graph generation they derive
  from.
- Biomedical outputs follow a claim ladder from research lead to clinical
  actionability; association-only inference is not enough for a cure or
  clinical recommendation.
- Substantial discovery runs need a result pack, findings doc, and readback
  summary.

## FSV

Documentation readback:

```text
Get-Content docs/CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md
Get-Content docs/medicalsearch/65_association_native_doctrine_context_update.md
```

Repo hygiene gates:

```text
cargo fmt --check
bash scripts/linecount.sh
git diff --check
```

## Boundary

This update strengthens operating doctrine and issue context. It does not claim
that any biomedical association is a cure, treatment, or clinical actionability
result.

---

## 66_binary_csr_persistence.md

# Binary CSR Persistence for PlainGraph

Issue: #1210

Status: complete FSV for the local physical-vault proof path and the real
aiwonder biomedical graph vault.

The persisted `PlainGraphCsr` stream now uses a binary columnar layout inside
the existing manifest-plus-segment framing:

- manifest row remains JSON and carries collection, snapshot, counts, segment
  count, byte total, and stream hash
- manifest version bumped to `4`
- stream starts with `CALYXCSR`
- node ids and edge destinations are raw 16-byte `CxId` values
- offsets are fixed little-endian `u64`
- edge weights are raw `f32` little-endian bytes
- edge types are dictionary-encoded with `u32` indexes

The reader still reassembles ordered segments, verifies byte count and blake3,
then decodes. Unsupported manifest versions, truncated segments, hash mismatch,
count mismatch, invalid edge types, and invalid weights fail closed as
`CALYX_GRAPH_CORRUPT_ROW`.

## Verification

Focused gates:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-aster plain_graph::csr -- --nocapture
cargo test -p calyx-aster plain_graph:: -- --nocapture
cargo test -p calyx-aster large_csr_projection_shards_into_segments_and_roundtrips -- --nocapture
cargo check -p calyx-aster
```

Physical FSV command:

```text
CALYX_FSV_ROOT=C:\code\Calyx-Dev\target\fsv\issue1210-binary-csr-20260704T075700Z \
  cargo test -p calyx-aster --test issue1210_binary_csr_fsv -- --nocapture
```

Physical FSV readback:

```text
target/fsv/issue1210-binary-csr-20260704T075700Z/issue1210_binary_csr_readback.json
```

Observed physical readback:

- source of truth: reopened durable Aster Graph CF through
  `PhysicalPlainGraph::read_csr_bytes` and `read_csr`
- binary magic: `CALYXCSR`
- binary CSR bytes: `481666`
- JSON baseline bytes: `1694623`
- binary less than half JSON: `true`
- binary CSR SHA-256:
  `9bc0e6da7c12f1e43116ca1c4f2f8303424b3e56eda4fa9daa362e002b0739e3`
- node count: `96`
- edge count: `19968`
- association edge count: `19968`
- edge cases: empty graph roundtrip, self-loop roundtrip, and `70000`
  distinct edge types roundtrip

The existing large in-crate sharding test still exercises segmentation after
the binary shrink: `segments=2 total_bytes=1269682`.

## Real Biomedical Graph FSV

Remote source of truth:

```text
aiwonder:/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J
vault name: corpus-anchored-869-20260625T080546Z
collection: default
```

Remote command:

```text
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

Remote FSV artifacts:

```text
/home/croyse/calyx/fsv/issue1210-binary-csr-real-20260704T101500Z
/home/croyse/calyx/fsv/issue1210-binary-csr-real-20260704T101500Z/issue1210_real_fsv_summary.json
```

Observed real-vault readback:

- git head: `9f105f4be07b08de18887597407fbfe9afd34b5f`
- status: `ok`
- prior CSR before-state: stale manifest v2 rejected as
  `CALYX_GRAPH_CORRUPT_ROW`
- source of truth: physical Graph CF latest readback through
  `PhysicalPlainGraph::read_csr`, `assoc_graph`, and independent node/edge key
  enumeration
- old JSON baseline bytes: `157072071`
- binary CSR bytes: `63235522`
- bytes saved: `93836549`
- size ratio vs old JSON baseline: `0.4025892165132272`
- binary less than half old JSON: `true`
- CSR SHA-256:
  `7c7a2928e12b4200d5c6c287008f982286490f8e003f4f6b9ee645331249ffd0`
- CSR blake3:
  `df9d23867f8feafd9a9ef15bb25207d60d4b000e67ba8b9cfd106b486c178662`
- nodes: `198993`
- CSR edges: `2435817`
- association edge count: `2435817`
- physical node keys: `198993`
- physical edge-out keys: `2435817`
- edge weight decode policy: `explicit-weight-or-legacy-unit`
- explicit weighted edges: `0`
- legacy unit-weight edges: `2435817`
- elapsed materialize/readback time: `81221 ms`

The real vault required two hardening fixes discovered only by FSV:

- stale persisted CSR manifests are recorded as before-state evidence and then
  rebuilt instead of blocking materialization
- legacy unweighted graph edge payloads are upgraded through an explicit,
  counted unit-weight policy; malformed explicit weights still fail closed
- the materializer builds the projection from physical graph range scans instead
  of `scan edge keys -> per-edge point read`, which avoided the real-vault CPU
  wall and completed the readback in about 81 seconds

## Boundary

This is association-substrate storage hardening. It improves persisted graph
size/readback behavior for mining, but does not itself validate any biomedical
hypothesis or clinical claim.

---

## 67_novelty_calibration_split.md

# #1226 Novelty / Calibration Split for Disease-Hunt Rankings

## Scope

#1226 re-ranks the persisted disease-hunt outputs from #1185, #1186, #1187,
and #1188 into two explicit views:

- a calibration / known-positive proof view, where rows carry explicit
  validation markers such as high-level CIViC evidence, Open Targets clinical
  or genetic validation scores, DGIdb clinical-trial source metadata, or a
  non-disease typed-pair marker;
- a novelty-prioritized research-lead view, where those calibration rows are
  excluded and remaining rows keep transparent score components.

This does not assert clinical novelty. It only removes evidence-marked
calibration rows from the novelty triage view while preserving them in a
separate proof view.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1226-novelty-calibration-split-20260704T104049Z
```

Inputs:

| Source issue | Domain | Rows |
|---|---|---:|
| #1185 | oncology | 19 |
| #1186 | metabolic/cardiovascular/renal | 35 |
| #1187 | neurodegeneration/neuropsychiatric | 77 |
| #1188 | infectious/immunology/inflammation | 209 |

## Output Readback

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/input_scope.json` | 1,896 | `64bee449eac0f952f09a6c7fd8bbcc1144f1d477d120535679bfe8e91769c71f` | - |
| `out/combined_original_ranked.jsonl` | 2,133,068 | `082c248bdc8686c96f0b2174c76b3fa33ebeed7b66a873f12db95ad87716c015` | 340 |
| `out/calibration_known_positive_rows.jsonl` | 1,134,154 | `8c7188342afc263f3dfede11896d002c7a560ee0942624316f96aac2cd8ec1ce` | 169 |
| `out/novelty_prioritized_research_leads.jsonl` | 998,914 | `97349007d843361d3630c673ccecd874483364670178acbdbaef63210495655a` | 171 |
| `out/before_after_topk.json` | 70,084 | `f4ce3fa5a6b9be04ff2b253b5ee0b418ab999f4a8fa48fe2dd870e74efeab1e7` | - |
| `out/manual_spot_checks.json` | 27,090 | `b3e42f7a6a34017e0185a84a9c4adc7be9821e6de2cf0bf0c20cc460a686ef37` | - |
| `out/validation_metrics.json` | 3,655 | `4127f3a89918c09511e537786106d76120bbefbf215f2adec39d937543f34509` | - |
| `out/persisted_readback.json` | - | `e5d55164369ac431f4666bdbaa00cd8b66619ab9acd3d9fe79589958c340fb9d` | - |

Readback assertions:

| Assertion | Value |
|---|---:|
| Combined rows read back | 340 |
| Calibration rows read back | 169 |
| Novelty rows read back | 171 |
| Total rows match metrics | true |
| Split rows sum to total | true |
| Before top-k rows | 25 |
| After top-k rows | 25 |
| Calibration top-k rows | 25 |
| Top after row is not calibration | true |
| Calibration rows available | true |
| Novelty rows available | true |

## Detector

The detector only routes a row to calibration / known-positive when the row
itself carries an explicit marker:

| Flag | Rows |
|---|---:|
| `open_targets_clinical_precedence_ge_0_75` | 89 |
| `open_targets_clinical_datatype_ge_0_75` | 89 |
| `non_disease_typed_pair_not_novelty_lead` | 47 |
| `open_targets_genetic_or_somatic_validation_ge_0_85` | 32 |
| `dgidb_clinical_source_plus_open_targets_context` | 17 |
| `civic_level_a_b_external_evidence` | 6 |
| `civic_level_a_b_trial_id_present` | 3 |

Rows are not demoted merely because they look familiar. If the current row does
not carry an explicit known-positive/calibration marker, it stays eligible for
the novelty-prioritized view and remains provisional.

## Metrics

| Metric | Count |
|---|---:|
| Total input rows | 340 |
| Calibration / known-positive rows | 169 |
| Novelty-prioritized rows | 171 |
| #1185 calibration rows | 6 |
| #1187 calibration rows | 21 |
| #1188 calibration rows | 142 |
| #1185 novelty rows | 13 |
| #1186 novelty rows | 35 |
| #1187 novelty rows | 56 |
| #1188 novelty rows | 67 |

## Before / After

Before: the combined original-rank top rows were dominated by known proof rows:

| Original view | Calibration? | Candidate | Flag |
|---:|---|---|---|
| 1 | yes | NF1 / Selumetinib / Plexiform Neurofibroma | CIViC level A/B + trial id |
| 2 | no | TNF / Proteinuria | none |
| 3 | yes | NF1 / neurofibromatosis type 1 | Open Targets genetic validation |
| 4 | yes | Golimumab / TNF / psoriatic arthritis | Open Targets clinical + DGIdb clinical source |
| 5 | yes | Certolizumab Pegol / TNF / psoriatic arthritis | Open Targets clinical + DGIdb clinical source |
| 6 | yes | Tregalizumab / CD4 / HIV infectious disease | Open Targets clinical + DGIdb clinical source |

After: the novelty-prioritized top rows exclude calibration/proof rows:

| Novelty rank | Candidate | Source issue | Score | Notes |
|---:|---|---|---:|---|
| 1 | TNF / Proteinuria | #1186 | 0.745 | target-disease lead, not externally marked calibration |
| 2 | Proteinuria / Diffuse Neurofibrillary Tangles with Calcification | #1187 | 0.730526316 | disease-neuro cluster |
| 3 | CD4 / Proteinuria | #1186 | 0.723823529 | target-disease lead |
| 4 | TNF / Proteinuria | #1188 | 0.716153846 | target-disease lead from infectious/immunology run |
| 5 | CD8A / Proteinuria | #1186 | 0.702647059 | target-disease lead |
| 6 | CD4 / Proteinuria | #1188 | 0.698846154 | target-disease lead from infectious/immunology run |
| 7 | Alogliptin / DPP4 / Type 2 Diabetes Mellitus | #1186 | 0.672941176 | drug-target-disease row; still provisional |
| 10 | Saxagliptin Anhydrous / DPP4 / schizophrenia | #1187 | 0.641578947 | drug-target-disease bridge; still provisional |
| 11 | Meningitis Bacterial / Subarachnoid Hemorrhage | #1187 | 0.635789474 | disease association |
| 12 | Nifedipine / SLC14A2 / Hypertension | #1186 | 0.635294118 | drug-target-disease row; still provisional |

The known-positive proof rows are not hidden. They remain in
`out/calibration_known_positive_rows.jsonl` and in the calibration top-k view.

## Spot Checks

| Check | Result |
|---|---|
| TNF / psoriatic arthritis | marked calibration when Open Targets clinical scores and DGIdb clinical sources are present |
| CD4 / HIV infectious disease | marked calibration for Open Targets clinical + DGIdb clinical source rows |
| CIViC NF1 / Selumetinib | marked calibration for level A/B CIViC rows with trial ids |
| Streptomycin / Klebsiella or Rhinoscleroma | remains in novelty view because current rows lack explicit known-positive calibration markers |
| DPP4 / schizophrenia | remains in novelty view when it is a weak bridge rather than an evidence-marked known-positive row |

## Follow-Up

#1227 tracks productionizing this detector as a native Calyx discovery/ranking
stage that consumes sealed discovery-run manifests and writes ledger-sealed
split outputs. #1226 is intentionally a bounded FSV artifact over current
disease-hunt outputs.

## Conclusion

#1226 is complete for the bounded disease-hunt atlas split:

- all 340 rows from #1185/#1186/#1187/#1188 were preserved;
- 169 rows were routed to the calibration / known-positive proof view;
- 171 rows were routed to the novelty-prioritized research-lead view;
- before/after top-k comparisons and manual spot checks were persisted;
- separate readback proved row conservation and artifact hashes.

No clinical novelty, recommendation, treatment claim, safety claim,
actionability claim, efficacy claim, or cure claim is made.

---

## 68_native_novelty_calibration_split.md

# #1227 Native Novelty / Calibration Splitter Stage

## Scope

#1227 turns the bounded #1226 split artifact into a native Calyx CLI stage:

```text
calyx novelty-calibration-split --atlas <issue>|<domain>|<jsonl> ... --out-dir <dir> [--top-k <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
```

The stage preserves every input atlas row, routes explicit known-positive /
calibration rows into a proof view, and emits a novelty-prioritized research
lead view for rows without those calibration markers.

This is still research triage only. It does not assert clinical novelty,
efficacy, safety, actionability, treatment guidance, or cure evidence.

## Implementation

Commit:

```text
0c44f4f3a4ca31af93ed23b2bfa557f6f59a9407
```

Main code paths:

- `crates/calyx-cli/src/cmd/novelty_split/mod.rs`
- `crates/calyx-cli/src/cmd/novelty_split/scoring.rs`
- `crates/calyx-cli/src/cmd/novelty_split/persist.rs`
- `crates/calyx-cli/src/cmd/novelty_split/tests.rs`

The command runs the shared discovery-run manifest preflight before it writes
split artifacts. A stale manifest therefore fails before `combined_original_ranked.jsonl`
or any downstream split view is written.

## Local Gates

```text
cargo test -p calyx-cli novelty_split -- --nocapture
cargo check -p calyx-cli
cargo fmt --check
bash scripts/linecount.sh
git diff --check
```

Local results:

- splitter tests: 3 passed;
- stale manifest unit test returned `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`
  and proved no combined output was written;
- `cargo check -p calyx-cli`: passed;
- `cargo fmt --check`: passed;
- line-count gate: all `.rs` files <= 500 lines;
- `git diff --check`: passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z
```

Real inputs:

| Source issue | Domain | Source artifact |
|---|---|---|
| #1185 | oncology | `/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl` |
| #1186 | metabolic/cardiovascular | `/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/out/metabolic_cardiovascular_hypotheses.jsonl` |
| #1187 | neuro | `/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z/out/neuro_hypotheses.jsonl` |
| #1188 | infectious/immunology | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_hypotheses.jsonl` |

Manifest preflight:

| Field | Value |
|---|---|
| Stage id | `novelty-calibration-split` |
| Expected input SHA-256 | `0d7c186d98f96eeaa3d575394c64412979e2ba9f91f2dc0d62a60ae18de2a88f` |
| Observed input SHA-256 | `0d7c186d98f96eeaa3d575394c64412979e2ba9f91f2dc0d62a60ae18de2a88f` |
| Match | true |
| Manifest | `/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z/manifest.json` |
| Manifest SHA-256 | `e1dc6a3c08752d4f2cf2f95b3c6c46c84bd939b97c9f8b4b0304be90458f9421` |

## Output Readback

| Artifact | SHA-256 |
|---|---|
| `native_stdout.json` | `9b882686bebab18eb6db0c67a511967420d975b08d7debfc0a308469b7dfd22a` |
| `out/persisted_readback.json` | `1a31874ca49d7a24446dd90c334cc4f8563b9ca003d390b3427de92d85917bc1` |
| `out/validation_metrics.json` | `f18fb88d82ce0d98e7b89668986850c622662fb36f8cf986e7bcf74c87a2204c` |
| `out/combined_original_ranked.jsonl` | `b283207096c22bd710b5002d15f49ed00662c47300fa9f7ab3f5949136462497` |
| `out/calibration_known_positive_rows.jsonl` | `44be636712d297fdd775b750615860255523ee25076e191c1e02802d73da0741` |
| `out/novelty_prioritized_research_leads.jsonl` | `731dea0569f99fc7afa3760a663c885a94c1d89bd68f384c0fb02ea3ffdb3815` |

Readback assertions:

| Assertion | Value |
|---|---:|
| Combined rows read back | 340 |
| Calibration rows read back | 169 |
| Novelty rows read back | 171 |
| Split rows sum to total | true |
| Total rows match | true |
| Top after row is not calibration | true |
| Calibration rows available | true |
| Novelty rows available | true |

## Parity With #1226

| View | #1226 rows | Native #1227 rows | Parity |
|---|---:|---:|---|
| Combined original ranked | 340 | 340 | true |
| Calibration / known-positive | 169 | 169 | true |
| Novelty-prioritized | 171 | 171 | true |

Top-row parity:

| View | Candidate |
|---|---|
| Top original | `oncology-civic:11176` |
| Top calibration | `oncology-civic:11176` |
| Top novelty | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` |

The native top novelty candidate matched the #1226 reference top novelty
candidate.

## Fail-Closed Proof

The FSV run also executed the same command with a deliberately stale manifest:

| Check | Value |
|---|---|
| Exit status | 2 |
| Failed | true |
| Error code | `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN` |
| `stale_out/combined_original_ranked.jsonl` written | false |

This proves the native stage checks sealed input identity before writing the
split artifacts.

## Conclusion

#1227 is complete for the native stage slice:

- the CLI command is implemented and discoverable in usage;
- the command consumes one or more sealed atlas JSONL inputs;
- persisted outputs include input scope, combined original ranking,
  calibration rows, novelty-prioritized rows, before/after top-k,
  spot checks, metrics, output manifest, and persisted readback;
- real aiwonder FSV matched the #1226 row counts and top novelty lead;
- stale input manifests fail closed before output.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.

---

## 69_infectious_normalization_repair.md

# #1225 Infectious / Immunology Normalization Repair

## Scope

#1225 repairs high-value unresolved concept normalization rows from the #1188
infectious/immunology/inflammation domain slice. The repair is conservative:
exact, unambiguous biomedical terms are mapped to stable MeSH or NCBI Gene IDs,
while narrative or ambiguous phrases remain in the unresolved artifact.

This is normalization and association-coverage repair only. It does not assert
clinical actionability, treatment guidance, safety, efficacy, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1225_infectious_normalization_repair.py
```

Final script commit:

```text
69f91cb9343234f4294e9cd5e8e63ec8b43f6eac
```

Local gates:

```text
python -m py_compile scripts/medicalsearch/issue1225_infectious_normalization_repair.py
git diff --check
```

Both passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z
```

Summary:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z/issue1225_fsv_summary.json
```

Inputs:

| Input | Path |
|---|---|
| Source expansion rows | `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/complete_cxid_source_expansion.jsonl` |
| #1188 normalized domain annotations | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_normalized_annotations.jsonl` |
| #1188 unresolved domain terms | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_unresolved_terms.jsonl` |

Raw lookup responses were persisted under:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z/raw
```

## Before / After

| Metric | Before | After |
|---|---:|---:|
| Normalized annotation rows | 475 | 1,027 |
| Normalized source CxIds | 265 | 440 |
| Unique normalized concepts | 14 | 27 |
| Unresolved terms | 151 | 122 |
| Resolved unresolved terms | - | 29 |
| Added annotation rows | - | 586 |
| New co-mention association coverage rows | - | 30 |

Remaining unresolved reason counts:

| Reason | Rows |
|---|---:|
| `api_error` | 25 |
| `not_queried_bounded_api_budget` | 97 |

## Deterministic Mappings

| Normalized name | DB | ID | Lookup SHA-256 | Terms |
|---|---|---|---|---|
| Asthma | `ncbi_mesh` | `D001249` | `2a8f55540e7a51a1f738b5a116df840c5e4c5048d8b27a997cca457b302bc682` | asthma; bronchial asthma; acute asthma |
| Tuberculosis | `ncbi_mesh` | `D014376` | `b8b8c563725bb79c317abd6510ec48076fe57c530c03513f23dcb66d391c343c` | Tuberculosis; Myco tuberculosis |
| Sepsis | `ncbi_mesh` | `D018805` | `75c56a2fc0ee0e9408e9bf7741f830682eefe342eaccd721092f8064c6939e18` | Sepsis; Septicemia |
| Malaria | `ncbi_mesh` | `D008288` | `e6055a4606f1ce7f63faca66a449c602b051f66daaa535215fa310cc57748fa3` | Malaria |
| Influenza Vaccines | `ncbi_mesh` | `D007252` | `7ac523184b4fd571d67f5a354272c176b7a4713d6d8de7608c75901027265e28` | Influenza vaccine |
| Lupus Vulgaris | `ncbi_mesh` | `D008177` | `d05507453dcf7a4a414ca29c9b2b8bf895546d6253e560c7ccfd728658a7271e` | Lupus vulgaris |
| Lupus Erythematosus, Systemic | `ncbi_mesh` | `D008180` | `89d37327f96ebde8bbacb7c9fcf84b5fbe242d26eab47825254c3206cbaeac3f` | Systemic lupus erythematosus |
| Arthritis, Rheumatoid | `ncbi_mesh` | `D001172` | `2009efb73ca42e76596c4c86e7bb3a78e475ea77fbb4ae67f68d25d8076a0e96` | Rheumatoid arthritis |
| Yellow Fever | `ncbi_mesh` | `D015004` | `32997d3d54aa634205a1643710ae27a8adf2c483e9fa45e8b2604bb885b4f6cc` | yellow fever |
| Leukotrienes | `ncbi_mesh` | `D015289` | `cd4cfd7864bf271e7d0cf00a3dc407ce71ed4f151b5f00fc005aa3fe2c087400` | Leukotriene |
| Leukotriene Antagonists | `ncbi_mesh` | `D020024` | `e6a26662533eb81f52eefaccff89e79be41ed22a8c0b62128c96552179396bd7` | Leukotriene antagonist; Leukotriene antagonists; Leukotriene receptor antagonist |
| HLA-B | `ncbi_gene` | `3106` | `c67f01175dc94dda80fb6d67acf96145ac41881d26fe8a6efd507654bcbc13ca` | HLA-B27 |
| CD40 | `ncbi_gene` | `958` | `10274bc066be6102bbc04cc91d67bdf8c5a8ecfa0fd7f02c71bfc80a0a1dfd8f` | CD40 |
| CD40LG | `ncbi_gene` | `959` | `bff64d7f016f2f112e2adc15aafeeb4744f7d9a00e698b0c00170c54fbb9ab0a` | CD40L |
| IL2 | `ncbi_gene` | `3558` | `b995e1c0ea2a1495416f5a368bdea399270afd01e3f2a39aa302454b95c45a0c` | Interleukin-2 |

## Required Row-Level Examples

| Required term | Added rows | Normalized to | Example source CxId | Source SHA-256 |
|---|---:|---|---|---|
| HLA-B27 | 2 | HLA-B / `ncbi_gene:3106` | `9aa017515951a9c89b41f0e759e23b87` | `30db19bd56e3a270beb82a59f74b56ff4cbf56cce51d1ab0648f22ef97e200df` |
| bronchial asthma | 97 | Asthma / `ncbi_mesh:D001249` | `01585cc6743dc8f752d91b3aa7b2356e` | `b1abeaa5dfdcee173d64fa4d9ce3168a06ebcc04db164c544647bf86a3e36445` |
| acute asthma | 30 | Asthma / `ncbi_mesh:D001249` | `0b142e0d099343c4d86d1dccb22990c3` | `879ebaf24b7789e479fadcb86433cf995cb4ac9f1bf3b7f987e9263ad9f85099` |
| Tuberculosis | 39 | Tuberculosis / `ncbi_mesh:D014376` | `0db83cbc1729a537fa916de8627d4ff8` | `c4e9a5f2a79c67f1cd8695c503f116a43c34479e04262423c2cd84f874ffc1ae` |
| CD40 | 6 | CD40 / `ncbi_gene:958` | `188af6caf2f56d36b6b41c263316d248` | `b717c588a69788d6b28642d038d480adbf0a9e4dc2d65e006cd3866ad1a34a1b` |
| Sepsis | 20 | Sepsis / `ncbi_mesh:D018805` | `082120387ae55f348be72d71736dff4e` | `b5c2b88b966cb0ea67430f661ff8fad2894478d028fee420db9706114148149c` |
| Leukotriene | 49 | Leukotrienes / `ncbi_mesh:D015289` | `01585cc6743dc8f752d91b3aa7b2356e` | `b1abeaa5dfdcee173d64fa4d9ce3168a06ebcc04db164c544647bf86a3e36445` |
| Malaria | 20 | Malaria / `ncbi_mesh:D008288` | `1242654995b8889a1198b02f0b63bcd4` | `d04dae7a2f23564e21bce187b2a73b2460408cc53dd941f0cf38846bb03ef264` |
| Influenza vaccine | 2 | Influenza Vaccines / `ncbi_mesh:D007252` | `1b0c5eb9c22a9db049c73a0b2c0bcbd7` | `200f859603013ac2f84e023c7d7a84ad4407f921949e18a63c11df5ffa5dd615` |

## Artifact Readback

| Artifact | SHA-256 |
|---|---|
| `input_scope.json` | `02983f061ab757a6eb9d86ecee504f6660d9afab993aa0d31305cd9d91315755` |
| `deterministic_mapping_table.json` | `84c26bd756156ac9137f6ccb0ebfaaa10ebe3c020dce7f647ea20e7698f6c8ec` |
| `repaired_normalized_annotations_added.jsonl` | `8c99dbaa3b8683906691b83775c5905ba4f51f17a2d45c8bec62730184b9ffcf` |
| `infectious_immunology_normalized_annotations.repaired.jsonl` | `1d31e9959a282c5a7054f70c1164a7d9abc37b91add8a1ff4c3f8b2be12a4f41` |
| `infectious_immunology_unresolved_terms.remaining.jsonl` | `11b35cce7c0a2afec3616b6985346d26df3e96bdcee5c882eeaefabc4fd21a41` |
| `resolved_terms_from_unresolved.jsonl` | `317e72a36ec5725a7783deaa668aa7e4b29fbed9811af40349473c3dcbe970db` |
| `new_co_mention_association_coverage.jsonl` | `719aa0ce97a8f40d4b37dedad0f714bb1fbacd717aa10e2f606d71dca3a4a021` |
| `before_after_coverage.json` | `425cfff636982d51a0d2f7b8ae8e7a1ef3c3a95c8996f2f8cf2ea5de5e7cdcdd` |
| `required_term_examples.json` | `b28cbf7222be6dec6e6cc3dfc419a60ed7e903bf1e6f8b6b4ca03a5509f44354` |

Readback assertions:

| Assertion | Value |
|---|---:|
| Before unresolved terms | 151 |
| After unresolved terms | 122 |
| Unresolved decreased | true |
| Added rows read back | 586 |
| Added rows match metrics | true |
| Repaired rows read back | 1,027 |
| Required terms all present | true |
| New co-mention pair rows | 30 |

## Conclusion

#1225 is complete for the focused infectious/immunology normalization repair:

- 29 previously unresolved terms were deterministically resolved;
- all required terms from the issue body have row-level source-hash examples;
- unresolved accounting remains explicit for 122 ambiguous or still-unqueried rows;
- repaired annotations increased source coverage from 265 to 440 CxIds;
- 30 new same-source association coverage rows were persisted for downstream
  review.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.

---

## 70_neuro_normalization_repair.md

# #1222 Neuro Concept Normalization Repair

## Scope

#1222 repairs unresolved or ambiguous neuro terms surfaced by #1187. Accepted
terms are mapped with deterministic, source-backed MeSH descriptors. Ambiguous
or narrative phrases remain unresolved.

This is normalization and coverage repair only. It does not assert clinical
actionability, treatment guidance, safety, efficacy, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1222_neuro_normalization_repair.py
```

Script commit:

```text
47b70c9684e9e5b136beaa9b7a6423aa8325a77c
```

Local gates:

```text
python -m py_compile scripts/medicalsearch/issue1222_neuro_normalization_repair.py
git diff --check
```

Both passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z
```

Repair summary:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/issue1222_fsv_summary.json
```

#1187 rerun summary:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187_summary.json
```

Raw MeSH lookup responses were persisted under:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/raw
```

## Repair Coverage

| Metric | Before | After |
|---|---:|---:|
| Neuro normalized annotation rows | 222 | 721 |
| Neuro normalized source CxIds | 129 | 297 |
| Unique neuro concepts | 10 | 44 |
| Neuro unresolved terms | 70 | 22 |
| Accepted unresolved term rows | - | 48 |
| Added annotation rows | - | 499 |
| New co-mention association rows | - | 30 |
| Overlay delta nodes | - | 260 |
| Overlay delta edges | - | 499 |
| Overlay untyped accepted edges | - | 0 |

Remaining unresolved reason counts:

| Reason | Rows |
|---|---:|
| `api_error` | 9 |
| `not_queried_bounded_api_budget` | 13 |

## Required Examples

| Required term | Added rows | Normalized to | Example source CxId | Source SHA-256 |
|---|---:|---|---|---|
| Parkinsonism | 18 | Parkinsonian Disorders / `ncbi_mesh:D020734` | `0298683a4e9e41041bbb08458f045d6e` | `0e6b92a0c40c009400dda75854ee7f55d867200bc07932fe55ec9c02e066dbef` |
| Parkinson's disease | 21 | Parkinson Disease / `ncbi_mesh:D010300` | `0298683a4e9e41041bbb08458f045d6e` | `0e6b92a0c40c009400dda75854ee7f55d867200bc07932fe55ec9c02e066dbef` |
| Seizures | 103 | Seizures / `ncbi_mesh:D012640` | `00f89b05ed956703fbe7bce73c41cce4` | `33e89a56397bccccfcfc7e750c3fb5494bd85ec70fb520cc242ea406e8876bb4` |
| Vascular dementia | 13 | Dementia, Vascular / `ncbi_mesh:D015140` | `0c74148f46bc73b2b85566f7908fecd8` | `f40cb7f9b1bf9444dc41a6167bd7420e5e06e08049af044a3cf5eeb3fb3c7333` |
| Ischemic stroke | 4 | Ischemic Stroke / `ncbi_mesh:D000083242` | `2748324e759eedd47fdcd1d61e8e075a` | `8c027bc6323b2168a5e873abab45edd3508b83f6b4888cd7eaa10ceab95ffea4` |
| Optic glioma | 4 | Optic Nerve Glioma / `ncbi_mesh:D020339` | `11d8c83de6d5efe0eb06137302d8104f` | `dba4e97ff8d669c4f30ccb3bbc581b92209ce7843fddf2fd058da96e54a023a5` |
| Spinocerebellar ataxia | 5 | Spinocerebellar Ataxias / `ncbi_mesh:D020754` | `175408106c68d1c2bd92fcf7badd7f63` | `5f3a2e017bf212ad5506b2ff58809f1087ceca37b12b2cdc79fd42d9c4a209d9` |
| Multiple sclerosis | 52 | Multiple Sclerosis / `ncbi_mesh:D009103` | `17b6aed7f8e8952cf10f11b516630698` | `a9df4ce25eea4d974079b39a48237e83a0fac193b53aab1538ea015368d605d9` |
| Migraine | 27 | Migraine Disorders / `ncbi_mesh:D008881` | `099f9f22fa1d9f29c77596fac0b55b8f` | `7a1f1f766ce14ec9f6520a48fd765483a36c90441ce3a682bc9fc8ecab9031b9` |
| Paranoid schizophrenia | 1 | Schizophrenia, Paranoid / `ncbi_mesh:D012563` | `8b70e13560f718e6d3b08c30badac20a` | `3a515cfa8f99ee90e8b7dba0af6368d8623135f0e4bc109ab1b9c4a47afca47c` |

## Rerun Deltas

The #1187 hunt script was rerun with the full repaired normalization inputs:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187
```

| Metric | Original #1187 | Rerun | Delta |
|---|---:|---:|---:|
| Neuro annotations | 222 | 721 | +499 |
| Neuro unresolved terms | 70 | 22 | -48 |
| Neuro hypotheses | 77 | 131 | +54 |

Top hypothesis changed from:

```text
issue1187:opentargets_target_neuro:03088f07d62fb884ac77
```

to:

```text
issue1187:normalized_comention:3ab84708ef24c3b7bcfd
```

The new top is a normalized co-mention candidate and remains a research lead,
not a treatment or actionability claim.

Rerun readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 131 |
| Metrics hypothesis total | 131 |
| Row count matches metrics | true |
| Neuro annotation rows | 721 |
| Neuro unresolved rows | 22 |
| Raw query manifest rows | 42 |
| Safety/trial flag rows | 14 |
| Top bundle count read back | 20 |

## Artifact Readback

| Artifact | SHA-256 |
|---|---|
| `out/deterministic_mapping_table.json` | `939eaf1d61a8c97556414e40c5f2028ca60a86ef836a15cd9faca5525267d267` |
| `out/neuro_normalized_annotations_added.jsonl` | `5571dc491a1901582d6dd3565c51a33fc3c8f78569aa1c8ec88cd19a7cb5220a` |
| `out/neuro_normalized_annotations.repaired.jsonl` | `849fe6b9bd000cb720f2d219ef9ba8db4954a3b62bb4c87847aa7ad6dce2e43c` |
| `out/neuro_unresolved_terms.remaining.jsonl` | `bf558199ba3d857da7f9784e59a13d9f071d04655e91fc0608992769a90eeec6` |
| `out/full_normalized_concept_annotations.repaired.jsonl` | `ee457a8b420260025ae43e97fe17d6dbf245faa58fea1c95ec94d4838ea1ad7c` |
| `out/full_unresolved_or_ambiguous_concepts.repaired.jsonl` | `429bbf36c55d81f9e89ffa562084b6edfbb8841f7ccaf4e70014169aa78c3b45` |
| `out/typed_overlay_delta_nodes.jsonl` | `92769f1f4b0b670f9350b8deb9d2317829ac2df8c4c15ceeee022d2c27f85550` |
| `out/typed_overlay_delta_edges.jsonl` | `699f8c0d82dead5bb8fad98de3d0a326583cc824f60ed69b4953c456b6a6ac15` |
| `out/new_co_mention_association_coverage.jsonl` | `56bf029fa5f988f575441d4013a66c6deb8b1573d8f3aa9a7655fc123437fc34` |
| `rerun_1187/out/neuro_hypotheses.jsonl` | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| `rerun_1187/out/persisted_readback.json` | `7d0a3a76f04fde55b4fd0fc2897135cc78f3cd4b0e003e9930cde707d8e5aca3` |

## Conclusion

#1222 is complete for the targeted neuro normalization repair:

- 48 unresolved neuro rows were accepted into deterministic mappings;
- 22 unresolved rows remain explicit;
- all required neuro term families have row-level source-hash examples;
- the overlay delta has 499 typed mention edges and zero untyped accepted edges;
- the #1187 rerun using repaired inputs persisted 131 hypotheses with readback
  counts matching metrics.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.

---

## 71_neuro_druggability_expansion.md

# #1224 Neuropsychiatric Target Druggability Expansion

## Scope

#1224 expands druggability and drug-target evidence for neuropsychiatric and
neurodevelopmental targets from the repaired #1187 neuro hunt. It consumes the
#1222 rerun, #1174 Open Targets context, #1178 DGIdb artifacts, #1175 molecular
artifacts, live DGIdb GraphQL, live ChEMBL REST, and the local BindingDB TSV zip.

This is a drug-target-disease mapping surface only. It does not assert treatment
efficacy, safety, dosing, clinical actionability, recommendation, or cure.

## Implementation

Script:

```text
scripts/medicalsearch/issue1224_neuro_druggability_expansion.py
```

The script:

- builds a 40-target input list from #1222/#1187 neuro hypotheses plus the
  targets named in #1224;
- queries DGIdb GraphQL per target and persists every raw page;
- queries ChEMBL target search, mechanism, and bounded activity endpoints;
- scans the local BindingDB TSV zip by ChEMBL-derived UniProt accessions;
- joins drug-target evidence to Open Targets/neuro disease contexts;
- emits explicit no-hit rows for sources that had no target match.

Public source surfaces checked for this run:

- DGIdb GraphQL API: <https://dgidb.org/api>
- ChEMBL web services: <https://www.ebi.ac.uk/chembl/api/data/docs>
- BindingDB downloads: <https://www.bindingdb.org/rwd/bind/chemsearch/marvin/Download.jsp>
- Open Targets API/data access: <https://platform.opentargets.org/api>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z
```

Preserved script bytes:

```text
/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/issue1224_neuro_druggability_expansion.py
sha256: 10aacce6e0c9c60a7b03ade30af97bd28c9ba09e93972097abf53a51bcaabc26
```

Primary readback:

```text
/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/persisted_readback.json
sha256: 428864a5fb32cd82977c4c530b68a70e212ac9c659bb7cd58c665bd4d5a008bd
```

## Input Source Hashes

| Source | Rows/bytes | SHA-256 |
|---|---:|---|
| #1222 rerun neuro hypotheses | 672,379 bytes | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| #1174 Open Targets rows | 1,189,226 bytes | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| #1178 DGIdb broad GraphQL interactions | 224,737 bytes | `8b205f69a58d76b906909b3966b721b607503f574b84e48560f31dc31d816b67` |
| #1178 DGIdb druggability rows | 21,765 bytes | `7e859f9d2993f836995157badcd5b2942544e3c75e43b8443d35cd656bc5b04e` |
| #1175 molecular scaleout rows | 80,636 bytes | `276fc1411ebecf95f7080bfdc32bdb082bb1c9bfc6ab16f5d6f11fb598e1300e` |
| BindingDB TSV zip | 590,990,498 bytes | `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7` |
| BindingDB target FASTA | 7,599,053 bytes | `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81` |

Live raw API/source responses:

| Source | Raw response files |
|---|---:|
| DGIdb GraphQL | 50 request records |
| ChEMBL REST | 80 request records |
| Total persisted raw JSON files | 130 |

## Output Metrics

| Metric | Count |
|---|---:|
| Target input rows | 40 |
| Open Targets disease-context rows | 140 |
| Local DGIdb rows | 141 |
| Live DGIdb rows | 1,597 |
| ChEMBL target rows | 115 |
| ChEMBL mechanism rows | 280 |
| ChEMBL activity rows | 368 |
| Local BindingDB rows | 9 |
| BindingDB TSV accession-scan rows | 713 |
| Approved-drug mapping rows | 939 |
| Drug-target-disease bridge candidates | 1,494 |
| No-hit/unavailable rows | 70 |

## Artifact Readback

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `target_input_list.jsonl` | 40 | `52d2db552088239739b49e51fb3bba7b9b28290d7068354f7259bfdc315ad176` |
| `open_targets_context_rows.jsonl` | 140 | `974ddf9fa4cbdaccdf398d454e7371dcd452c7e6fccd168fa134822f84bb6897` |
| `dgidb_target_interactions.jsonl` | 1,738 | `c7ca7ae7104fb68738238390d2eb924e28b84b5191f53de7f4fba0e93028d091` |
| `chembl_target_rows.jsonl` | 115 | `e7ad089a75988a945468090d76919f8b23a88386257f25a49631ad17b3106d79` |
| `chembl_mechanism_rows.jsonl` | 280 | `a7f442327a73f0e71a2cd44d89015f811f2bb3e1dc17c170b0b521306ca11fe5` |
| `chembl_activity_rows.jsonl` | 368 | `821a6d6d070aba2d20c3dac6d97a6cd02a042b9692fed090de7eac80509df266` |
| `molecular_source_hits.jsonl` | 1,495 | `d42174678bd33998611918d9b965baa15c33cd9eae8ba5163f637907a0930140` |
| `approved_drug_mappings.jsonl` | 939 | `78e294a9542cfbcf422055c992e0c6b581f4587de5e7412329abb63521bdc620` |
| `drug_target_disease_bridge_candidates.jsonl` | 1,494 | `050f93a9340ae8c850faae7d4fde0133149264f707802752c3deb193408edfcd` |
| `no_hit_or_unavailable_targets.jsonl` | 70 | `06f947ccd019bf1e67bfb48496897a3bff76abbeab4443ae6a7df4ae91d7b1a8` |
| `source_hashes.json` | - | `f3cfd624530787581a55a18ba90692c37c666ad87dfea4e41c6a11ede773355c` |
| `validation_metrics.json` | - | `22466eeb8d6a828095d2f174eff843a6d0372b00dd492be5fe23c34137fdaf13` |

## Coverage Highlights

| Target | Open Targets contexts | DGIdb rows | ChEMBL mechanisms | BindingDB rows | Bridge rows | No-hit rows |
|---|---:|---:|---:|---:|---:|---:|
| DPP4 | 51 | 174 | 19 | 59 | 100 | 0 |
| DRD2 | 2 | 222 | 68 | 50 | 100 | 0 |
| DRD3 | 1 | 117 | 11 | 50 | 100 | 0 |
| DRD4 | 1 | 79 | 2 | 50 | 100 | 0 |
| HTR1A | 1 | 168 | 26 | 50 | 100 | 0 |
| HTR2A | 1 | 218 | 56 | 50 | 100 | 0 |
| HTR4 | 1 | 59 | 17 | 50 | 100 | 0 |
| NF1 | 50 | 25 | 0 | 0 | 100 | 2 |
| OPRD1 | 1 | 115 | 6 | 50 | 100 | 0 |
| OPRK1 | 1 | 131 | 17 | 50 | 100 | 0 |
| OPRM1 | 1 | 186 | 54 | 50 | 100 | 0 |
| PTEN | 1 | 158 | 0 | 5 | 100 | 1 |
| CACNA1I | 1 | 29 | 0 | 50 | 79 | 1 |
| KIF11 | 1 | 18 | 4 | 50 | 68 | 0 |
| MTHFR | 1 | 34 | 0 | 0 | 34 | 2 |

No-hit rows by source:

| Source | Targets without rows |
|---|---:|
| `chembl_mechanisms` | 29 |
| `bindingdb_rows` | 20 |
| `dgidb_interactions` | 21 |

## Top Bridge Rows

Top rows by rank score are not clinical recommendations; they are evidence-rich
mapping leads for downstream falsification/safety/outcome gates.

| Rank | Target | Drug | Disease context | Source | Interaction | Approved flag | Rank score |
|---:|---|---|---|---|---|---:|---:|
| 1 | CIT | C3TD879 | microcephaly | DGIdb live | inhibitor | false | 53.924685 |
| 2 | LMNB2 | METRELEPTIN | microcephaly | DGIdb live | none listed | true | 12.038883 |
| 3 | KIF11 | AZD4877 | microcephaly | DGIdb live | inhibitor | false | 10.242422 |
| 4 | KIF11 | ISPINESIB | microcephaly | DGIdb live | inhibitor | false | 10.242422 |
| 5 | MTHFR | VITAMIN B12 | schizophrenia | DGIdb live | none listed | true | 9.942657 |
| 6 | ZNF335 | PRASUGREL | microcephaly | DGIdb live | none listed | true | 8.294495 |
| 7 | MTHFR | L-METHYLFOLATE | schizophrenia | DGIdb live | none listed | false | 7.685834 |
| 8 | KIF11 | ARQ-621 | microcephaly | DGIdb live | inhibitor | false | 7.101756 |
| 9 | KIF11 | FILANESIB | microcephaly | DGIdb live | inhibitor | false | 7.101756 |
| 10 | MCPH1 | PERPHENAZINE | microcephaly | DGIdb live | none listed | true | 5.947854 |

## Findings

- #1224 no longer has only a DPP4 drug-target slice. The DRD2/DRD3/DRD4,
  HTR1A/HTR2A/HTR4, opioid-receptor, CACNA1I, KIF11, PTEN, MTHFR, and NF1
  surfaces now have persisted source-backed rows or explicit no-hit rows.
- BindingDB was scanned from the local TSV zip by UniProt accession, not fuzzy
  target names. This gives exact protein-source linkage where ChEMBL target
  search exposed accessions.
- Several microcephaly genes remain sparse or no-hit for DGIdb/ChEMBL/BindingDB.
  Those rows are explicit worklist gaps, not failures hidden by filtering.
- Strong-looking rank scores can come from drug-target evidence density, not from
  disease outcome evidence. The next promotion step must run falsification,
  safety, trial/outcome, and sufficiency gates before any stronger claim.

## Conclusion

#1224 is complete for target druggability expansion: all target inputs, live raw
source hashes, parsed interaction counts, bridge rows, and no-hit rows were
persisted and separately read back.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, or cure claim is made.

---

## 72_generated_candidate_falsification_sweep.md

# #1223 Generated Candidate Falsification Sweep

## Scope

#1223 runs a fail-closed falsification sweep across the generated disease-hunt
candidate outputs from #1185, #1186, #1187/#1222, #1188, and #1189. It
normalizes one candidate row per hypothesis, joins persisted support and
counter-evidence from the source-validation program, writes one falsification
flag per candidate, and materializes the top falsification rows into a native
Calyx bridge-corpus vault.

This is a triage and demotion instrument only. It does not establish efficacy,
safety, clinical actionability, treatment guidance, dosing, recommendation, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1223_generated_candidate_falsification.py
```

The script:

- consumes generated candidate JSONL rows from #1185, #1186, #1187/#1222,
  #1188, and #1189;
- uses persisted PubTator/PubMed, ClinicalTrials.gov, DGIdb, Open Targets, and
  drug-safety artifacts as the evidence instruments;
- emits support evidence, counter evidence, one falsification flag per deduped
  generated candidate, top demoted rows, and an output manifest;
- fails closed when required drug-safety or drug-disease trial evidence is
  absent for drug-bearing candidates;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Important normalization fix: drug and disease endpoint strings are not promoted
to gene symbols. Gene/target extraction is limited to explicit gene/target
fields, preventing drug names such as `Kanamycin` or disease labels such as
`Tinnitus` from becoming target symbols.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/persisted_readback.json
sha256: 4495e768c412d321c32fdae3f8051dec17dcb2ace436eed7d0e18d846bab9704

/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/calyx_bridge_corpus_readback.json
sha256: 184395b83aa7669960bc3bd904755e604860024b4dbf8ea73b651c7e1474e80e
```

Native Calyx materialization:

```text
name: issue1223-generated-falsification-20260704t121310z
vault_id: 01KWPGYV128AQVM7BMKW3AB6KY
vault_dir: /home/croyse/calyx/vaults/01KWPGYV128AQVM7BMKW3AB6KY
stdout_sha256: 5bb5af08f038d931c8a70346451a254dbba73bfd52df7f8a3d2cfa1b36729c97
stderr_sha256: dc5c2f7264f053d13bf68648d1ac9ae78057ef0115e8645fa0460832e669fb4f
```

Materialization readback assertions:

| Assertion | Value |
|---|---:|
| Row count is 1,000 | true |
| CSR persisted | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Active vault index contains name | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

Native vault counts:

| Item | Count / bytes |
|---|---:|
| Bridge-corpus rows | 1,000 |
| Bridge terms | 625 |
| Graph nodes | 1,625 |
| Graph edges | 8,000 |
| `cf/graph` SST files | 9,628 |
| `cf/graph` SST bytes | 8,741,178 |
| `cf/time_index` SST files | 9,628 |
| `cf/time_index` SST bytes | 1,669,406 |

## Input Candidate Hashes

| Input | Rows | SHA-256 |
|---|---:|---|
| #1185 oncology hypotheses | 19 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| #1186 metabolic/cardiovascular hypotheses | 35 | `1a53daf6d93b2ce2f0d28235b679a7cd1427c0a44070285840365c6397f7eb94` |
| #1187/#1222 repaired neuro hypotheses | 131 | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| #1188 infectious/immunology hypotheses | 209 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` |
| #1189 rare-disease hypotheses | 1,491 | `c15fa5ac6b5e41f32a9a7a3fe184b8de2d639005642a723bf106a85a8ff36bfa` |

## Evidence Source Hashes

| Source | SHA-256 |
|---|---|
| PubTator supporting literature | `bf473c33e99f596411116b8fb4a165ca1dd893a73399d552efa8979689ad9cb0` |
| PubTator negative/contradicting literature | `2ded353b125e85436a6fad4d431c61f760bb4aa2de2112ac7a68acec4002dd08` |
| ClinicalTrials.gov trial rows | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| ClinicalTrials.gov seed summaries | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| DGIdb seed GraphQL interactions | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| DGIdb broad GraphQL interactions | `8b205f69a58d76b906909b3966b721b607503f574b84e48560f31dc31d816b67` |
| Open Targets association rows | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| Open Targets validation edges | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| #1181 candidate safety flags | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` |
| #1181 safety terms | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` |
| #1224 DGIdb target interactions | `c7ca7ae7104fb68738238390d2eb924e28b84b5191f53de7f4fba0e93028d091` |
| #1224 Open Targets context rows | `974ddf9fa4cbdaccdf398d454e7371dcd452c7e6fccd168fa134822f84bb6897` |
| #1189 DGIdb target interactions | `7d47f24803d10feb025d6c240cd53de813036158226686c4f609a4ee15384297` |

## Output Metrics

| Metric | Count |
|---|---:|
| Input candidate rows | 1,885 |
| Deduped candidate rows | 1,877 |
| Falsification flag rows | 1,877 |
| Support evidence rows | 4,926 |
| Counter-evidence rows | 2,151 |
| Blocked or demoted rows | 1,082 |
| Rows missing required evidence | 1,066 |
| Rows with hard counterevidence | 16 |

Status counts:

| Status | Count |
|---|---:|
| `blocked_missing_required_evidence_or_safety` | 1,066 |
| `complete_no_counterevidence_found_in_current_sources` | 795 |
| `demoted_counterevidence_found` | 16 |

Reason-code counts:

| Reason code | Count |
|---|---:|
| `trial_source_missing_for_drug_disease` | 1,028 |
| `safety_source_missing_fail_closed` | 991 |
| `safety_block_or_high_risk_label` | 90 |
| `dgidb_exact_drug_gene_missing_current_sources` | 14 |
| `embedded_safety_or_trial_gap` | 13 |
| `clinicaltrials_stopped_trial` | 1 |
| `existing_counterevidence_status` | 1 |
| `no_counter_evidence_found_in_current_sources` | 795 |

Input contribution after dedupe:

| Source | Rows |
|---|---:|
| `1185_oncology` | 19 |
| `1186_metabolic_cardiovascular` | 35 |
| `1187_neuro_repaired` | 129 |
| `1188_infectious_immunology` | 203 |
| `1189_rare_disease` | 1,491 |

## Artifact Readback

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `normalized_generated_candidates.jsonl` | 1,877 | `aa64d6027445023d3a94838984406f3178e6f1c1afd30a4a82a0f8c1392bf914` |
| `support_evidence.jsonl` | 4,926 | `09fc9e3df16b7f8bc111fe3c23e00d9d20b27bee75959799266535912a7b09e5` |
| `counter_evidence.jsonl` | 2,151 | `bd043b638cb048a5be78f22fbd029e32dad0d8e7d2b90909a707df47ecffda94` |
| `candidate_falsification_flags.jsonl` | 1,877 | `b3ec172c5f83caa86968ed74de9b10682d2af008b309713e16b57f7137d9aff0` |
| `raw_query_manifest.jsonl` | 14 | `ee6aef67ba2d9763358e95f9d1e206c7707005ffc7c64181ec5e1de5709a6752` |
| `generated_candidate_falsification_bridge_rows.jsonl` | 1,000 | `ee00570033691c6292a353403c8f291bf25bf254b5e8a4e73949f2321dc163b5` |
| `validation_metrics.json` | - | `730e4eb9a31741acff090042545140f02906928c4aff57568883d9e3aef6a046` |
| `persisted_readback.json` | - | `4495e768c412d321c32fdae3f8051dec17dcb2ace436eed7d0e18d846bab9704` |
| `calyx_bridge_corpus_readback.json` | - | `184395b83aa7669960bc3bd904755e604860024b4dbf8ea73b651c7e1474e80e` |

Persisted readback assertions:

| Assertion | Value |
|---|---:|
| One flag per candidate | true |
| Support rows present | true |
| Counter rows present | true |
| Raw manifest present | true |
| Top demoted file present | true |
| Clinical boundary present on all flags | true |

## Top Demoted / Blocked Examples

These are not "bad drugs" or clinical guidance. They are generated candidates
that failed the current required-evidence and counterevidence triage gates.

| Candidate | Source | Gene(s) | Drug(s) | Disease(s) | Status | Score | Reasons |
|---|---|---|---|---|---|---:|---|
| `oncology-civic:1471` | #1185 | NF1 | AZ628; VTX-11e | Skin Melanoma | blocked missing evidence/safety | 0.679688 | DGIdb exact pair missing; embedded safety/trial gap; safety block/high-risk label; trial missing |
| `oncology-civic:7815` | #1185 | NT5C2 | Cytarabine; Doxorubicin; Gemcitabine | Childhood Acute Lymphocytic Leukemia | blocked missing evidence/safety | 0.628440 | DGIdb exact pair missing; embedded safety/trial gap; safety block/high-risk label; trial missing |
| `issue1187:normalized_comention:17b6c746f037502ed3cb` | #1187/#1222 | - | Kanamycin | Tinnitus | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |
| `issue1187:normalized_comention:1952baf7598d8f077aab` | #1187/#1222 | - | Phenytoin | Seizures | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |
| `issue1187:normalized_comention:586502cb3434d0d4dd78` | #1187/#1222 | - | Pregabalin | Migraine Disorders | blocked missing evidence/safety | 0.615385 | safety source missing; trial missing |

## Findings

- The generated hunt corpus now has a single persisted falsification state per
  deduped candidate: 1,877 candidates in, 1,877 flags out.
- 1,082 candidates are blocked or demoted before any atlas-promotion step. The
  dominant failure mode is missing required trial/safety evidence for
  drug-bearing disease candidates, which is intentionally fail-closed.
- The normalization repair removed the accidental treatment of endpoint labels
  as target symbols; the DGIdb exact-pair-missing reason fell to 14 rows after
  rerun.
- 795 rows have no counterevidence in the current bounded sources, but that is
  not enough for efficacy, safety, actionability, recommendation, or cure. They
  remain research candidates until outcome, safety, sufficiency, and human-review
  gates are satisfied.
- The 1,000 highest-priority falsification rows are now in the native Calyx
  database as bridge-corpus vault `01KWPGYV128AQVM7BMKW3AB6KY`.

## Conclusion

#1223 is complete for generated-candidate falsification triage: all generated
candidate inputs, source evidence files, falsification flags, demotion summaries,
and the native Calyx bridge-corpus materialization were persisted and separately
read back.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, or cure claim is made.

---

## 73_human_review_biomedical_hypothesis_atlas.md

# #1193 Human-Review Biomedical Hypothesis Atlas

## Scope

#1193 publishes a human-review atlas over the generated biomedical discovery
rows from #1185, #1186, #1187/#1222, #1188, and #1189. The atlas overlays
novelty/calibration state from #1227, generated-candidate falsification state
from #1223, support/counter evidence, safety/trial flags, and source hashes.

This is an inspectable research-review surface only. Every row is
hypothesis-only. No row is efficacy, safety, clinical actionability, treatment
guidance, dosing, recommendation, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1193_biomedical_hypothesis_atlas.py
```

The script emits:

- a normalized JSONL atlas for machine review;
- a TSV atlas for human scanning/filtering;
- filter facets by disease area, drug, target, pathway, evidence type, review
  status, source issue, and falsification status;
- top evidence bundles with source snippets, typed evidence path kinds,
  validation summaries, support/counter examples, hashes, and next validation
  experiments;
- a 1,000-row bridge-corpus slice for native Calyx materialization;
- output manifest, metrics, and persisted readback.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z/out/persisted_readback.json
sha256: 01cc9b112372d50d0bfd7c71ea88661c86826c071911d63d9f7109f7acbb248d

/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z/out/calyx_bridge_corpus_readback.json
sha256: e164923f418e878a445fba5d4d910bed7f95d29d1f203e720bc10f0f1446a071
```

Native Calyx materialization:

```text
name: issue1193-human-review-atlas-20260704t124751z
vault_id: 01KWPJR0ADVZF580HNRBZ17CBZ
vault_dir: /home/croyse/calyx/vaults/01KWPJR0ADVZF580HNRBZ17CBZ
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,365 |
| Graph nodes | 2,365 |
| Graph edges | 10,280 |
| CSR persisted | true |
| Active vault index contains name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |

## Inputs

| Input | Rows | SHA-256 |
|---|---:|---|
| #1185 oncology hypotheses | 19 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |
| #1186 metabolic/cardiovascular hypotheses | 35 | `1a53daf6d93b2ce2f0d28235b679a7cd1427c0a44070285840365c6397f7eb94` |
| #1187/#1222 repaired neuro hypotheses | 131 | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| #1188 infectious/immunology hypotheses | 209 | `de2e6eda7c8aabcb39a8644c9948923cf1b93ddf2c0776372cdbcf7f4e65b61d` |
| #1189 rare-disease hypotheses | 1,491 | `c15fa5ac6b5e41f32a9a7a3fe184b8de2d639005642a723bf106a85a8ff36bfa` |
| #1223 falsification flags | 1,877 | `b3ec172c5f83caa86968ed74de9b10682d2af008b309713e16b57f7137d9aff0` |
| #1223 support evidence | 4,926 | `09fc9e3df16b7f8bc111fe3c23e00d9d20b27bee75959799266535912a7b09e5` |
| #1223 counter evidence | 2,151 | `bd043b638cb048a5be78f22fbd029e32dad0d8e7d2b90909a707df47ecffda94` |
| #1227 novelty combined view | 340 | `b283207096c22bd710b5002d15f49ed00662c47300fa9f7ab3f5949136462497` |
| #1227 calibration view | 169 | `44be636712d297fdd775b750615860255523ee25076e191c1e02802d73da0741` |
| #1227 novelty leads view | 171 | `731dea0569f99fc7afa3760a663c885a94c1d89bd68f384c0fb02ea3ffdb3815` |
| #1181 safety flags | 13 | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` |
| #1177 ClinicalTrials rows | 269 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| #1174 Open Targets rows | 1,422 | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `human_review_biomedical_hypothesis_atlas.jsonl` | 1,877 | `766cebc55e4fb6c49672bf09b2a5e02ebbd81da7dfdbb14a8766e4d66aa8098e` |
| `human_review_biomedical_hypothesis_atlas.tsv` | - | `c918bb5c2992a402e2995b6ac43e3d52f01635d67fe037911e297f1d2530c9cf` |
| `atlas_filters.json` | - | `4489d5ac3cd1bdedf5396b252d43bac1205cd87c54b46276bf411fc83bcb653a` |
| `top_evidence_bundles.json` | - | `4c4587d941398bf3135cb63d7c0f25fa138a42cf36b71a3d48754c240730ade3` |
| `atlas_bridge_rows.jsonl` | 1,000 | `f4f1f449738f9862b0a6f2058b401203596b2a79c1bbfcef0853c3f3a6c920aa` |
| `input_manifest.json` | - | `f8e7de53fe5f5953f7b40a65345d5995d7c71eb702d432b96141a05eba0bb428` |
| `output_manifest.json` | - | `b6c9da262316b7c60b8de54bafbbcc709b5b20c8e94f3b7839db8a9468b4bed9` |
| `validation_metrics.json` | - | `6589a2f7dff2730f72b847305946beee8fe0c40c14fa6259535a2393e2b4a066` |
| `persisted_readback.json` | - | `01cc9b112372d50d0bfd7c71ea88661c86826c071911d63d9f7109f7acbb248d` |
| `calyx_bridge_corpus_readback.json` | - | `e164923f418e878a445fba5d4d910bed7f95d29d1f203e720bc10f0f1446a071` |

## Metrics

| Metric | Count |
|---|---:|
| Raw input source rows | 1,885 |
| Deduped atlas rows | 1,877 |
| Missing falsification flags | 0 |
| Hypothesis-only rows | 1,877 |
| Rows with normalized hypothesis | 1,877 |
| Rows with source snippets | 1,877 |
| Rows with support evidence | 1,877 |
| Rows with counter evidence | 1,082 |
| Rows with validation evidence | 1,869 |
| Rows with disease context | 1,877 |
| Rows with target context | 1,681 |
| Rows with drug context | 1,044 |

Review status:

| Status | Count |
|---|---:|
| `blocked_or_demoted_before_human_review` | 1,082 |
| `ready_for_hypothesis_review` | 726 |
| `calibration_known_positive_reference` | 69 |

Disease area:

| Area | Count |
|---|---:|
| rare disease | 1,491 |
| infectious/immunology/inflammation | 203 |
| neurodegeneration/neuropsychiatric | 129 |
| metabolic/cardiovascular/renal | 35 |
| oncology | 19 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Deduped rows match falsification flags | true |
| Atlas JSONL rows match | true |
| Bridge rows <= 1,000 | true |
| All rows hypothesis-only | true |
| All rows have clinical boundary | true |
| All rows have normalized hypothesis | true |
| All rows have source snippet | true |
| All rows have review status | true |
| Filters present | true |
| Top evidence bundles present | true |
| Ready rows present | true |
| Blocked rows present | true |

## Top Ready-for-Review Rows

These are research-review rows only. They are not treatment suggestions.

| Atlas rank | Candidate | Area | Target(s) | Drug(s) | Disease/context | Confidence score | Novelty score | Next validation |
|---:|---|---|---|---|---|---:|---:|---|
| 1 | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | Tnf | - | Proteinuria | 0.555000 | 0.745000 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 2 | `issue1187:disease_neuro_cluster:259cdcce4807d7a52fde` | neurodegeneration/neuropsychiatric | - | - | Diffuse Neurofibrillary Tangles with Calcification; Proteinuria | 0.435773 | 0.730526 | Human reviewer should inspect evidence bundle and define the next grounded outcome instrument |
| 3 | `typed-assoc:concept:ncbi_gene:920::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | CD4 | - | Proteinuria | 0.545286 | 0.723824 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 4 | `typed-assoc:concept:ncbi_gene:925::concept:ncbi_mesh:D011507` | metabolic/cardiovascular/renal | CD8A | - | Proteinuria | 0.535571 | 0.702647 | Validate target-disease association in an outcome-backed assay/model before drug inference |
| 5 | `issue1187:disease_neuro_cluster:057d66318046c6764d5c` | neurodegeneration/neuropsychiatric | - | - | Subarachnoid Hemorrhage; Meningitis Bacterial | 0.399437 | 0.635789 | Human reviewer should inspect evidence bundle and define the next grounded outcome instrument |

## Findings

- The atlas consolidates all five current disease-hunt families into one
  review surface: 1,885 input rows became 1,877 deduped atlas rows.
- Every atlas row now has an explicit normalized hypothesis, source snippet,
  review status, support evidence overlay, source hash, clinical boundary, and
  next validation experiment.
- #1223 falsification is now enforced as the promotion gate: 1,082 rows are
  blocked or demoted before human review, and none are missing falsification
  flags.
- The atlas separates calibration/proof rows from novel research leads. The
  69 calibration rows are references for gate health, not novelty claims.
- The 1,000-row atlas bridge slice is materialized into native Calyx vault
  `01KWPJR0ADVZF580HNRBZ17CBZ`, so the review surface is also in the Calyx DB.

## Conclusion

#1193 is complete for the current human-review biomedical hypothesis atlas. The
atlas is persisted, filterable, source-hashed, falsification-aware, and
materialized into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 74_drug_combination_hypotheses.md

# #1190 Drug-Combination and Synergy Hypothesis Miner

## Scope

#1190 mines drug-pair hypotheses from the #1193 human-review atlas and blocks
promotion unless component safety, pair interaction, and external synergy/model
evidence are all present. This is a fail-closed triage layer: missing evidence
is a block, not a weak pass.

No row is efficacy, safety, clinical actionability, treatment guidance, dosing,
recommendation, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1190_drug_combination_miner.py
```

The script:

- reads the #1193 atlas and extracts specific drug-bearing components;
- groups components by disease/context and pairs the top components per group;
- scores target/pathway rationale for complementary targets or convergent
  pathway evidence;
- checks component safety rows and openFDA drug-interaction label sections from
  #1181;
- checks exact co-intervention trial rows from #1177;
- streams the DrugComb v1.4 summary table for exact drug-pair preclinical
  synergy matches;
- emits one safety/interaction flag per pair and blocks every pair missing any
  required evidence.

External source research:

- DrugComb Zenodo record: <https://zenodo.org/records/11102665>
- DrugComb downloaded file: `summary_table_v1.4.csv`
- NCI ALMANAC CellMiner source page: <https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html>
- NCI ALMANAC/figshare collection: <https://figshare.com/collections/Data_from_The_National_Cancer_Institute_ALMANAC_A_Comprehensive_Screening_Resource_for_the_Detection_of_Anticancer_Drug_Pairs_with_Enhanced_Therapeutic_Activity/6508832>

DrugComb is used here as preclinical/cell-line synergy evidence only. It is not
clinical efficacy.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/persisted_readback.json
sha256: 9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2

/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/calyx_bridge_corpus_readback.json
sha256: 444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f
```

Native Calyx materialization:

```text
name: issue1190-drug-combination-miner-20260704t130000z
vault_id: 01KWPKR3DMS0GX68YDCSCVP12T
vault_dir: /home/croyse/calyx/vaults/01KWPKR3DMS0GX68YDCSCVP12T
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 309 |
| Graph nodes | 1,309 |
| Graph edges | 10,000 |
| CSR persisted | true |
| Active vault index contains name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1193 atlas | 1,877 rows | `766cebc55e4fb6c49672bf09b2a5e02ebbd81da7dfdbb14a8766e4d66aa8098e` |
| #1181 drug safety terms | 14 rows | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` |
| #1181 parsed safety rows | 353 rows | `b56926bffe2c580c141a74e701c8f0535195d06f33d9c875812729e36f40d167` |
| #1181 mapped candidate safety | 26 rows | `1f0eb4b787c708f5e87c5238d905ca5015b448db638866a2c4b538020dbc54e7` |
| #1177 ClinicalTrials rows | 269 rows | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| DrugComb summary table v1.4 | 193,184,734 bytes | `e08e3d35bfa4bea011afe1b05b7025acde5c669c5e8d265866b5e12b08037a2f` |

DrugComb file integrity:

| Field | Value |
|---|---|
| Expected MD5 | `c11efbdcae4a860c2374c1505a66599b` |
| Observed MD5 | `c11efbdcae4a860c2374c1505a66599b` |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `combination_candidate_inputs.jsonl` | 1,023 | `be973631b5f16b4989db6d0cb8acad2fad0a369fad3c0c2b4932608573aada67` |
| `drug_component_safety_index.jsonl` | 14 | `53e296b2c27685c21e5280c7d70b0e7df2f2b66b72b369d95d9a95e2f75385a8` |
| `candidate_pair_inputs.jsonl` | 1,750 | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| `drugcomb_pair_matches.jsonl` | 28 | `9b075a80df9b6b252191d548ee68c93202e2ed9dfaa4430e226446aa7ad51a86` |
| `drug_combination_hypotheses.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| `combination_safety_interaction_flags.jsonl` | 1,750 | `86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab` |
| `blocked_combination_rows.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| `top_combination_review_queue.json` | - | `01e7a98b5f3685e8ea2f7d0615cac3c9ba49b63febedf213226f4feee26ca4b2` |
| `combination_bridge_rows.jsonl` | 1,000 | `fb4af52867fd7f7049ea268f7297db415202688388b38ebe1e35c73bd14b4ecd` |
| `validation_metrics.json` | - | `70c67ee66348531c2e8e99a5832028b5c9f8fbb5d208df69a0c15963a4d0d83a` |
| `persisted_readback.json` | - | `9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2` |
| `calyx_bridge_corpus_readback.json` | - | `444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f` |

## Metrics

| Metric | Count |
|---|---:|
| Component input rows | 1,023 |
| Candidate pair rows | 1,750 |
| Combination hypothesis rows | 1,750 |
| Safety/interaction flag rows | 1,750 |
| Blocked combination rows | 1,750 |
| Reviewable preclinical rows | 0 |
| DrugComb matched pair keys | 28 |
| Rows with DrugComb match | 47 |
| Rows with exact pair interaction evidence | 5 |
| Rows with both component safety rows | 23 |
| Component safety index rows | 14 |

Reason-code counts:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,703 |
| `overlapping_component_safety_flags_review_required` | 17 |

Disease-area counts:

| Area | Count |
|---|---:|
| rare disease | 1,470 |
| infectious/immunology/inflammation | 149 |
| neurodegeneration/neuropsychiatric | 66 |
| metabolic/cardiovascular/renal | 47 |
| oncology | 18 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Component inputs present | true |
| Candidate pairs present | true |
| Hypothesis rows match pairs | true |
| Flag rows match hypotheses | true |
| DrugComb source read | true |
| All rows have clinical boundary | true |
| Blocked rows present | true |
| Bridge rows <= 1,000 | true |
| No promoted clinical rows | true |

## Top Blocked Rows

These rows are useful because they name exactly what evidence is missing before
any combination can be reviewed.

| Rank | Pair | Disease/context | Status | Reason codes |
|---:|---|---|---|---|
| 1 | Metformin + Sitagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing |
| 2 | Metformin + Saxagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; pair interaction missing |
| 3 | Metformin + Alogliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; external synergy missing |
| 4 | Metformin + Linagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; external synergy missing |
| 5 | Sunitinib + Everolimus | Hereditary pheochromocytoma-paraganglioma | blocked | component blocked/demoted; component safety missing; pair interaction missing |

## Findings

- The miner produced a real combination worklist, not a recommendation list:
  1,750 candidate pairs were persisted, and all 1,750 are blocked.
- DrugComb did add external preclinical evidence: 28 exact pair keys matched the
  candidate pairs, covering 47 rows. These remain preclinical and do not
  override safety/interaction blocks.
- The dominant blockers are missing component safety rows, missing exact
  drug-pair interaction evidence, and missing external synergy evidence. These
  are actionable data-ingest deficits, not clinical conclusions.
- The current safety substrate is too narrow for broad combination promotion:
  only 14 component safety rows are available for 1,023 drug components.
- The 1,000 highest-ranked blocked combination rows are now materialized in
  native Calyx vault `01KWPKR3DMS0GX68YDCSCVP12T`.

## Conclusion

#1190 is complete for the fail-closed combination miner slice: it builds
combination hypotheses, persists component safety/interaction/synergy evidence,
blocks missing evidence, and materializes the worklist into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 75_nci_almanac_external_synergy.md

# #1229 NCI ALMANAC External Combination Evidence Ingest

## Scope

#1229 ingests the public CellMiner/NCI ALMANAC processed combo-score workbook
and joins it to the #1190 drug-combination candidate pairs. The purpose is to
turn an external preclinical source into typed, persisted evidence rows inside
the Calyx discovery substrate.

No row is efficacy, safety, clinical actionability, treatment guidance, dosing,
recommendation, or cure evidence. ALMANAC and DrugComb evidence are preclinical
or model evidence only; missing safety, pair-interaction, and real outcome gates
remain hard blocks.

## Implementation

Script:

```text
scripts/medicalsearch/issue1229_nci_almanac_synergy_ingest.py
```

The script:

- parses the NCI ALMANAC XLSX workbook with a stdlib ZIP/XML reader;
- emits one pair-level row for every workbook drug pair;
- emits atomic per-cell-line combo-score rows;
- joins ALMANAC and existing #1190 DrugComb evidence to every #1190 candidate
  pair;
- assigns each #1190 candidate a deterministic external evidence status:
  `exact_hit`, `normalized_hit`, or `no_external_hit`;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

External source:

- CellMiner/NCI ALMANAC source page:
  <https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html>
- Processed dataset download:
  <https://discover.nci.nih.gov/cellminer/download/processeddataset/DTP_NCI60_ALMANAC_COMBO_SCORE.zip>
- Dataset metadata page:
  <https://discover.nci.nih.gov/cellminer/datasets.do>

The workbook metadata read back by the parser:

| Field | Value |
|---|---|
| CellMiner Database Version | `2.15` |
| Human Genome Version | `HG-19` |
| Date | `09-17-2025` |

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/persisted_readback.json
sha256: b7d6ce0ffb7a3843cc0b3be5755a89db6f5ac647b6e99a05c2f24a27e961f3b1

/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/calyx_bridge_corpus_readback.json
sha256: 0f5dcb5efb25428d3738054e16d1bd07107041a8f1d604e2e070e9c076da6987
```

Native Calyx materialization:

```text
name: issue1229-nci-almanac-synergy-20260704t140500z
vault_id: 01KWPPR0HC0Z02P5SDB1QEZWQ2
vault_dir: /home/croyse/calyx/vaults/01KWPPR0HC0Z02P5SDB1QEZWQ2
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 356 |
| Graph nodes | 1,356 |
| Graph edges | 10,000 |
| CSR persisted | true |
| Active vault index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| ALMANAC ZIP | 1,653,406 bytes | `161a0801e5a34c1fd1a3ae5b3c743d0b481d979b1c61fb08c75031b980233a0a` |
| ALMANAC XLSX | 1,627,596 bytes | `f43ca26735aa58152410ce4145ccdeffc0ba78c56c7279b7b48ef372f2b3c52b` |
| #1190 candidate pairs | 1,750 rows | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1190 combination hypotheses | 1,750 rows | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| #1190 DrugComb matches | 28 rows | `9b075a80df9b6b252191d548ee68c93202e2ed9dfaa4430e226446aa7ad51a86` |

ALMANAC ZIP MD5 readback: `de0114d0730986b4d98f1b190189ee24`.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `almanac_pair_scores.jsonl` | 5,355 | 6,869,895 | `aa81aaaffc01b4087f36d34555e66593d412926733ffef7fb56a95764025347f` |
| `almanac_cellline_combo_scores.jsonl` | 306,365 | 199,787,593 | `6a60297dbd7051c4b9be7c14c29b5a9166667dcbda6de1adce569b126f341aae` |
| `candidate_external_synergy_status.jsonl` | 1,750 | 1,990,187 | `e4dbb332db9c4b656db83452a2f0debac9748c4c0cadb4b98c9280cfbcc6a4b7` |
| `candidate_external_synergy_hits.jsonl` | 68 | 151,003 | `b5f6135a8c86b8bb4c7115ff3f2c72d1384fc021f71c6832e4cc212545844a09` |
| `external_synergy_bridge_rows.jsonl` | 1,000 | 1,089,861 | `0f6d2fdfabbae4f536977a35bf5396b9465308910d65dbfcfe8f727742c29ec1` |
| `input_manifest.json` | - | 2,898 | `b6b71ddad5ea33c850d8ae7df208c641e8920c82ecd86ade5100f10cceec3af6` |
| `output_manifest.json` | - | 2,587 | `fd5db8ee0481e3c5138458c4d6982395bb4cb4e7036a9e6191fcacea3a374cef` |
| `validation_metrics.json` | - | 8,414 | `9907dba76b8944c2f62ba6a8079ea27abedc4bc8632a12e261862688c8c14e3c` |
| `persisted_readback.json` | - | 2,854 | `b7d6ce0ffb7a3843cc0b3be5755a89db6f5ac647b6e99a05c2f24a27e961f3b1` |
| `calyx_bridge_corpus_stdout.json` | - | 671 | `9d52898fa61e53e20eb9f7f0759e2d640067c2812f0299ef8e76dee91e8e52d1` |
| `calyx_bridge_corpus_readback.json` | - | 3,188 | `0f5dcb5efb25428d3738054e16d1bd07107041a8f1d604e2e070e9c076da6987` |

## Metrics

| Metric | Count |
|---|---:|
| ALMANAC pair rows | 5,355 |
| ALMANAC unique pair keys | 5,233 |
| ALMANAC cell-line score rows | 306,365 |
| ALMANAC pair rows with positive combo score in at least one cell line | 5,274 |
| #1190 candidate pair rows joined | 1,750 |
| Candidate rows with ALMANAC hit | 31 |
| Candidate rows with DrugComb hit | 47 |
| Candidate rows with any external hit | 68 |
| Candidate rows with both ALMANAC and DrugComb hit | 10 |
| Candidate rows with no external hit | 1,682 |

External evidence status counts:

| Status | Count |
|---|---:|
| `exact_hit` | 63 |
| `normalized_hit` | 5 |
| `no_external_hit` | 1,682 |

Reason-code counts after the external evidence join:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,682 |
| `external_synergy_recheck_required_not_a_pass` | 21 |
| `overlapping_component_safety_flags_review_required` | 17 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| ALMANAC pairs present | true |
| ALMANAC scores present | true |
| Score rows cover pair keys | true |
| Joined rows match #1190 candidates | true |
| Deterministic status for every candidate | true |
| All joined rows carry boundary | true |
| No clinical-claim rows | true |
| Bridge rows <= 1,000 | true |

## Findings

- NCI ALMANAC adds a second external preclinical combination-evidence source
  beyond DrugComb. It contributes 31 #1190 candidate hits; 10 overlap DrugComb.
- External evidence coverage improved from 47 DrugComb-hit rows to 68 total
  external-hit rows, but 1,682/1,750 candidates still have no external pair
  evidence.
- All 68 external-hit rows remain blocked/provisional. External preclinical
  support does not clear component-safety, pair-interaction, clinical outcome,
  human-review, dosing, or safety gates.
- The corrected bridge corpus is materialized in native Calyx vault
  `01KWPPR0HC0Z02P5SDB1QEZWQ2`.
- A first trial vault from the pre-correction run remains physically present and
  active because `retire-vault` refuses to retire a non-quarantined healthy
  vault. The final source of truth for #1229 is the corrected
  `20260704T140500Z` root and vault above.

## Conclusion

#1229 is complete for the NCI ALMANAC/external-synergy ingest slice: the public
ALMANAC workbook was downloaded, checksummed, parsed into atomic pair and
cell-line evidence rows, joined to all #1190 candidates, and materialized into a
native Calyx bridge-corpus vault.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 76_external_combo_source_expansion.md

# #1231 External Combination Source Expansion

## Scope

#1231 expands the #1190/#1229 drug-combination evidence search beyond
DrugComb v1.4 and NCI ALMANAC by adding CDCDB, an open drug-combination source
snapshot. The run rechecks every #1190/#1229 candidate pair and separately
rechecks the 1,682 rows that remained `no_external_hit` after #1229.

CDCDB evidence is source-attributed combination documentation from
ClinicalTrials.gov, FDA Orange Book, and patent-derived records. It is not
synergy proof, efficacy proof, safety proof, treatment guidance, dosing,
recommendation, or cure evidence. Rows with CDCDB hits remain blocked until
component safety, pair interaction, grounded outcome, and human-review gates are
separately satisfied.

## Implementation

Script:

```text
scripts/medicalsearch/issue1231_external_combo_sources.py
```

The script:

- reads the CDCDB Figshare metadata and CSV archive;
- checksums the physical archive and records Figshare API file metadata;
- fingerprints every CSV schema in the CDCDB archive;
- parses `all_combs_unormalized.csv` into source-combination rows;
- derives normalized pair keys across all drug groups and aliases;
- joins CDCDB pair evidence to every #1190/#1229 candidate pair;
- emits deterministic `exact_hit`, `normalized_hit`, or `no_external_hit`
  status for every candidate and every prior no-hit row;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- CDCDB Figshare dataset:
  <https://springernature.figshare.com/articles/dataset/CSV_version_of_CDCDB_from_12_4_2022/19582069>
- Figshare API metadata:
  <https://api.figshare.com/v2/articles/19582069>
- Direct archive URL:
  <https://ndownloader.figshare.com/files/34785670>
- Data descriptor:
  <https://www.nature.com/articles/s41597-023-02303-8>
- License recorded by the run: `CC0`

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/persisted_readback.json
sha256: e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825

/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/calyx_bridge_corpus_readback.json
sha256: 2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db
```

Native Calyx materialization:

```text
name: issue1231-external-combo-sources-20260704t150500z
vault_id: 01KWPTB0JDE1NCSHB4477BH7HA
vault_dir: /home/croyse/calyx/vaults/01KWPTB0JDE1NCSHB4477BH7HA
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 357 |
| Graph nodes | 1,357 |
| Graph edges | 8,508 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| CDCDB `12.04.2022.zip` | 64,277,993 bytes | `02e6151bfcce9617d47260ad8d60570432e21de1c7330ca1f946c44faf169d3c` |
| Figshare API metadata | 3,377 bytes | `c5f066523101409c1781eb9fdb0a405879474c3fbfc5ec2db52aa1bc21d0872b` |
| #1190 candidate pairs | 1,750 rows | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1229 prior external status | 1,750 rows | `e4dbb332db9c4b656db83452a2f0debac9748c4c0cadb4b98c9280cfbcc6a4b7` |

CDCDB file integrity:

| Check | Value |
|---|---|
| Archive MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Figshare supplied MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Figshare computed MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Bytes match Figshare API | true |
| Download URL matches Figshare API | true |
| CDCDB schema fingerprint | `b287a134e9aa7d578db38dad09af1a05a4612303fff968c98e29ff3e2360e8b5` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `cdcdb_source_combinations.jsonl` | 43,082 | 38,714,138 | `004b07ee5b308501d048a8a919d6ce7af9dcd086897b25b777c17d43d6a32f79` |
| `cdcdb_pair_index.jsonl` | 78,321 | 127,436,740 | `55d77873ae1c7cd1a5d670c82050de6b22134f419fafccdd3a7edf7742c75714` |
| `candidate_external_combo_status.jsonl` | 1,750 | 2,641,087 | `f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4` |
| `prior_no_hit_recheck_status.jsonl` | 1,682 | 2,449,119 | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |
| `candidate_external_combo_hits.jsonl` | 173 | 595,639 | `c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,314,779 | `dde19c92a5e657298fef463a8345ba262a9e99507e481546db02b664c6229a14` |
| `input_manifest.json` | - | 9,796 | `b6f52fe6ca249d9696acddf17e0da1bee94865e40e3716f5b43497b9d4c2f864` |
| `output_manifest.json` | - | 2,866 | `b62fdabdfff60d34ee17d3dc983c8f2192a91add3bb100775df3361ac58ddf29` |
| `validation_metrics.json` | - | 9,600 | `ebdc0863663e14ebb9bc96d84fe9d5787a4b14c279f5bb5ca34c37750c1f041d` |
| `persisted_readback.json` | - | 3,259 | `e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825` |
| `calyx_bridge_corpus_stdout.json` | - | 679 | `f93a69be19de85986ebbb64b5dd643ddca3983ca682f1e3cb64b7809ebb3bb2d` |
| `calyx_bridge_corpus_readback.json` | - | 3,779 | `2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db` |

## Metrics

| Metric | Count |
|---|---:|
| CDCDB source combination rows parsed | 43,082 |
| CDCDB unique normalized pair keys | 78,321 |
| #1190/#1229 candidate rows rechecked | 1,750 |
| #1229 prior no-hit rows rechecked | 1,682 |
| Candidate rows with CDCDB hit | 173 |
| Prior no-hit rows with CDCDB hit | 136 |
| Candidate rows with any external evidence after CDCDB | 204 |
| Remaining prior no-hit rows after CDCDB | 1,546 |

CDCDB source-combination row counts:

| Source type | Rows |
|---|---:|
| `clinicaltrials.gov` | 28,322 |
| `orangebook` | 551 |
| `patents` | 14,209 |

CDCDB status counts over all candidate rows:

| Status | Count |
|---|---:|
| `exact_hit` | 62 |
| `normalized_hit` | 111 |
| `no_external_hit` | 1,577 |

Overall external-evidence status after DrugComb + ALMANAC + CDCDB:

| Status | Count |
|---|---:|
| `exact_hit` | 109 |
| `normalized_hit` | 95 |
| `no_external_hit` | 1,546 |

Reason-code counts after the CDCDB recheck:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,682 |
| `external_combo_source_hit_not_clearance` | 173 |
| `external_combo_source_missing_fail_closed` | 1,546 |
| `external_synergy_recheck_required_not_a_pass` | 21 |
| `overlapping_component_safety_flags_review_required` | 17 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| CDCDB source rows present | true |
| CDCDB pair keys present | true |
| Joined rows match prior status rows | true |
| Prior no-hit recheck rows match prior no-hit rows | true |
| Deterministic status for every candidate | true |
| Deterministic status for every prior no-hit | true |
| All joined rows carry boundary | true |
| All CDCDB hits have source summaries | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains final name exactly once | true |
| Graph/time-index SST files are present | true |

## Findings

- CDCDB adds a third external combination-evidence source to the #1190/#1229
  candidate universe.
- CDCDB contributes 173 candidate hits: 62 exact-name hits and 111 normalized
  hits.
- CDCDB resolves 136 of the 1,682 #1229 prior no-hit rows to source-attributed
  external-combination evidence, reducing the remaining no-hit set to 1,546.
- The `external_synergy_evidence_missing_fail_closed` reason is intentionally
  preserved on the CDCDB-only rows. CDCDB is not a synergy, safety, dosing, or
  outcome gate.
- The 1,000-row bridge corpus is materialized in native Calyx vault
  `01KWPTB0JDE1NCSHB4477BH7HA`.

## Conclusion

#1231 is complete for the CDCDB source-expansion slice: the additional open
source was downloaded, checksummed, schema-fingerprinted, normalized to
association pair keys, joined to all #1190/#1229 candidates, and materialized
into a native Calyx bridge-corpus vault.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 77_clinicaltrials_current_recheck.md

# #1232 ClinicalTrials.gov Current Recheck

## Scope

#1232 performs a direct current ClinicalTrials.gov v2 API recheck over the
1,546 #1231 rows that still had no external combination-source hit after
DrugComb, NCI ALMANAC, CDCDB, and the #1231 CDCDB join.

The recheck persists one raw API response per pair, follows pagination, and
derives deterministic status rows only when both candidate drug names are found
in returned study intervention text. Trial-registry co-occurrence is not
efficacy proof, safety proof, dosing guidance, treatment guidance,
recommendation, clinical actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1232_clinicaltrials_recheck.py
```

The script:

- reads the #1231 prior no-hit recheck rows;
- filters to the 1,546 rows still `no_external_hit` after CDCDB;
- downloads and fingerprints the current ClinicalTrials.gov v2 OpenAPI spec;
- queries the current v2 API with `query.intr` for each remaining pair;
- follows `nextPageToken` pagination for every pair;
- stores raw API responses and page hashes;
- emits deterministic `exact_hit` or `no_external_hit` rows;
- writes one study-evidence row per matched NCT study;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- ClinicalTrials.gov v2 API docs: <https://clinicaltrials.gov/data-api/api>
- API overview: <https://clinicaltrials.gov/data-api/about-api>
- OpenAPI spec: <https://clinicaltrials.gov/api/oas/v2>
- Studies endpoint: <https://clinicaltrials.gov/api/v2/studies>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/persisted_readback.json
sha256: 606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1

/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/calyx_bridge_corpus_readback.json
sha256: ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a

/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/vault_supersession_readback.json
sha256: 91ad37fbcac9c838a730c74dae66b13e5f7870b56414927f9eae433a9c8dbd40
```

Native Calyx materialization:

```text
name: issue1232-clinicaltrials-current-recheck-20260704t154500z
vault_id: 01KWPWANC80TEH4HZWJM337ZX0
vault_dir: /home/croyse/calyx/vaults/01KWPWANC80TEH4HZWJM337ZX0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 305 |
| Graph nodes | 1,305 |
| Graph edges | 8,838 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

Supersession readback:

| Item | Value |
|---|---|
| Stale non-paginated vault | `01KWPVN28JV96ZDPG12D0C5WA9` |
| Final paginated vault | `01KWPWANC80TEH4HZWJM337ZX0` |
| Old vault absent from active index | true |
| Old vault has supersession record | true |
| Supersession points to final vault | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| ClinicalTrials.gov OpenAPI v2 spec | 80,983 bytes | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| #1231 prior no-hit recheck rows | 1,682 rows | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `clinicaltrials_raw_responses.jsonl` | 1,546 | 480,925,442 | `1f8d40df3bac91b30807e9eae785a70f6710ebccf8304b7c8e2eb678ee2b6e2d` |
| `clinicaltrials_pair_status.jsonl` | 1,546 | 2,178,478 | `ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc` |
| `clinicaltrials_pair_hits.jsonl` | 204 | 747,137 | `026cc1d5698f6b7b2fdcf2cd095cf0a605a804f1ed3f40ed0bd151342c006c38` |
| `clinicaltrials_study_evidence.jsonl` | 1,192 | 1,638,764 | `cb25d98c9fd5c623d63c31a9dcf7b7fbc55add65cc0a7b9d265fe83e5f875970` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,299,985 | `9f93b312d2bb7e96b779291b4579b9f635e3780ce113799f0e41b06e8e9656f9` |
| `input_manifest.json` | - | 1,734 | `b9ab3f1acdfcb89a9c36cf0de17fe030031fe68364e04794c3f455fbd2403087` |
| `output_manifest.json` | - | 2,378 | `ad6e55d387c2fe0f6558561ee403ffc4f2350a9bc5f81ad08f05e76afd123d1d` |
| `validation_metrics.json` | - | 9,890 | `31042216dacc7142351cd4a97336979442f213e89ae591b416fa43d74db3640b` |
| `persisted_readback.json` | - | 2,823 | `606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1` |
| `calyx_bridge_corpus_stdout.json` | - | 711 | `38b47e193ac6590d0f7e411160a42bed4f65488d32370925d181b5fc96179f2a` |
| `calyx_bridge_corpus_readback.json` | - | 3,287 | `ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a` |
| `stale_vault_supersession_stdout.json` | - | 3,689 | `094252fc130057ff3448a3b5414cf88e0b99b8037458a19abffeaf802cc208c0` |
| `vault_supersession_readback.json` | - | 4,875 | `91ad37fbcac9c838a730c74dae66b13e5f7870b56414927f9eae433a9c8dbd40` |

## Metrics

| Metric | Count |
|---|---:|
| Remaining no-hit rows queried | 1,546 |
| Raw API response rows | 1,546 |
| API pages read | 1,618 |
| Max API pages for one pair | 31 |
| Responses with at least one returned study | 285 |
| Responses with pagination | 10 |
| Candidate rows with current ClinicalTrials.gov hit | 204 |
| Matched NCT study evidence rows | 1,192 |
| Unique matched NCT ids | 399 |
| Remaining no-hit rows after current ClinicalTrials.gov | 1,342 |

Status counts:

| Status | Count |
|---|---:|
| `exact_hit` | 204 |
| `no_external_hit` | 1,342 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Raw response for every remaining no-hit row | true |
| Deterministic status for every row | true |
| All joined rows carry boundary | true |
| All hits have matched studies | true |
| All study rows have NCT ids | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains final name exactly once | true |
| Stale non-paginated vault absent from active index | true |
| Stale vault has supersession record | true |

## Findings

- Current direct ClinicalTrials.gov registry recheck adds 204 external
  combination-source hits to the 1,546 rows that remained no-hit after CDCDB.
- The remaining external no-hit set drops from 1,546 to 1,342.
- All 204 hits are `exact_hit` under this parser because both candidate drug
  names appear in the returned study intervention text.
- This is registry documentation only. It does not clear efficacy, safety,
  dosing, outcome, or clinical actionability gates.
- The final paginated bridge corpus is materialized in native Calyx vault
  `01KWPWANC80TEH4HZWJM337ZX0`; the earlier non-paginated trial vault
  `01KWPVN28JV96ZDPG12D0C5WA9` is superseded and no longer active.

## Conclusion

#1232 is complete for the current ClinicalTrials.gov direct recheck slice:
1,546 remaining no-hit candidate rows were queried against the current API,
1,618 API pages were persisted and checksummed, 204 source-attributed registry
hits were found, and the bounded bridge corpus was materialized into Calyx with
post-supersession readback.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 78_fda_pubmed_source_mining.md

# #1234 FDA/PubMed Source Mining

## Scope

#1234 continues the external source-mining chain over the 1,342 #1232 rows
that still had no current ClinicalTrials.gov hit after DrugComb, NCI ALMANAC,
CDCDB, and the #1232 registry recheck.

This pass checks three current sources:

- FDA Orange Book downloadable data files;
- FDA National Drug Code Directory text download;
- PubMed E-utilities ESearch/ESummary title/abstract co-mention search.

The output is source-attributed research triage only. FDA product ingredient
co-occurrence and PubMed title/abstract co-mention are not efficacy proof,
safety proof, dosing guidance, treatment guidance, recommendation, clinical
actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1234_fda_pubmed_source_mining.py
```

The script:

- reads the #1232 `clinicaltrials_pair_status.jsonl` rows;
- filters to the 1,342 rows still `no_external_hit`;
- parses current FDA Orange Book `products.txt` ingredient sets;
- parses current FDA NDC `product.txt` substance sets;
- builds deterministic source pair indexes;
- queries PubMed ESearch for 790 unique pair keys using title/abstract phrase
  terms, under the no-key E-utilities rate limit;
- batches PubMed ESummary reads for returned PMIDs;
- emits one exact/normalized/no-hit status row for every input row;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted sources:

- FDA Orange Book Data Files: <https://www.fda.gov/drugs/drug-approvals-and-databases/orange-book-data-files>
- FDA Orange Book download: <https://www.fda.gov/media/76860/download?attachment>
- FDA NDC Directory: <https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory>
- FDA NDC text download: <https://www.accessdata.fda.gov/cder/ndctext.zip>
- NCBI E-utilities intro/rate policy: <https://www.ncbi.nlm.nih.gov/books/NBK25497/>
- NLM E-utilities guide: <https://www.nlm.nih.gov/dataguide/eutilities/utilities.html>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/persisted_readback.json
sha256: a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16

/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/calyx_bridge_corpus_readback.json
sha256: 21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265
```

Native Calyx materialization:

```text
name: issue1234-fda-pubmed-source-mining-20260704t171500z
vault_id: 01KWPYD9HZWK974Z6Z8G839ZYG
vault_dir: /home/croyse/calyx/vaults/01KWPYD9HZWK974Z6Z8G839ZYG
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 507 |
| Graph nodes | 1,507 |
| Graph edges | 10,580 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1232 ClinicalTrials.gov pair status | 1,546 rows | `ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc` |
| FDA Orange Book source page | 37,044 bytes | `944322f9bc0e153600a4f38057fde15881b955fd9e9a16522a2e13c4297dddc2` |
| FDA Orange Book zip | 1,087,144 bytes | `a9c4f73aef2770b655a4af077053511fb8b8f97259be658980bcff3b64ca9e31` |
| FDA NDC source page | 36,115 bytes | `d7b3f16c960d97cae95c002f36bdfab5f54724094b404f8559c1800b70088c09` |
| FDA NDC text zip | 10,712,849 bytes | `5c481b512cdd5d545f27eae736bab48d0209f4716f85d7de256bfc47fbb84de4` |
| NCBI E-utilities intro | 68,713 bytes | `204e634142073f071ade91b62c83e5034fe57c1ca1b1642a4d468aece020c65c` |
| NLM E-utilities guide | 72,237 bytes | `08c0e23ecdec38e4fe8c6cdc0d655e87186662573a6d98de93a22a950fa1e081` |

Source schema fingerprints:

| Source | Fingerprint |
|---|---|
| FDA Orange Book parsed schemas | `557bf0e59d9105c0e230c44811c71b81bef1d1135c727a0528a11ca9037a4c05` |
| FDA NDC parsed schemas | `923929bd837503e6d7be9e56675cd2c0f369edbec454fb164d2a29564207ca80` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `fda_orangebook_source_products.jsonl` | 5,755 | 4,409,934 | `7520e6d167a2491c8edc5da9bbb72d23e8662d56ce07e26aeac4f9263f7b5684` |
| `fda_orangebook_pair_index.jsonl` | 1,036 | 3,065,980 | `7236e0f526e5cc3d90d621801ae0007381bb065649f85a8c8bf3e7a72c887f56` |
| `fda_ndc_source_products.jsonl` | 24,119 | 23,861,429 | `08106a075c5f46415fe585efa748b0235f704444c823526ff6dc1138f0a8c5b3` |
| `fda_ndc_pair_index.jsonl` | 163,832 | 507,538,433 | `21c6048e20897325a685f7df29a2c9d0a2d31ef3a4f9ca90eb93bf5c20b654f4` |
| `pubmed_esearch_responses.jsonl` | 790 | 1,183,322 | `feb6390a6f23eabe273c64710ab9ce5b42c8e8481f101c440389cfabdf3a2339` |
| `pubmed_esummary_responses.jsonl` | 4 | 1,045,478 | `b7a871fd3b687d49b63d07e9050db23e1eb97e1b068722d2dcba5e173f5629e3` |
| `pubmed_pair_literature_evidence.jsonl` | 568 | 766,538 | `304400ca4bbc7faea42ac9f2dd708865e7e27a225cbde6e806301d4581b7f7ea` |
| `candidate_external_source_status.jsonl` | 1,342 | 2,017,898 | `8f61616f01f44695e7b0227a16ff8cd9bceee2c68ad6147b705142a69ce24b0e` |
| `candidate_external_source_hits.jsonl` | 301 | 473,300 | `b60932183462b012f284a26750b0412c6a179224849938d7bb05ee42d473e950` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,491,061 | `ec0f457e185648d31bb29a6a58abb8e6337a65283a42fec46f303697ac8e3107` |
| `validation_metrics.json` | - | 9,996 | `70b9146d373ea4c7eda910da5d48f5c46f98c0a59c6e33c051aec75b0f58f39e` |
| `output_manifest.json` | - | 4,392 | `26c2938c8f45d070828d812b04decb59f2f2168ee537ca2fd83d9af13bf84cd1` |
| `persisted_readback.json` | - | 4,368 | `a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16` |
| `calyx_bridge_corpus_stdout.json` | - | 687 | `6687467d950190dee4128aa03270b52d67dce7d47c3733f6d7856c0546cd186d` |
| `calyx_bridge_corpus_readback.json` | - | 3,649 | `21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265` |

## Metrics

| Metric | Count |
|---|---:|
| #1232 no-hit rows rechecked | 1,342 |
| FDA Orange Book candidate hits | 0 |
| FDA NDC candidate hits | 0 |
| PubMed unique pair queries | 790 |
| PubMed unique pair queries with hits | 141 |
| PubMed candidate rows with hits | 301 |
| PubMed evidence rows | 568 |
| PubMed unique PMIDs | 523 |
| Candidate rows with any #1234 hit | 301 |
| Remaining no-hit rows after #1234 | 1,041 |

Status counts:

| Status | Count |
|---|---:|
| `normalized_hit` | 301 |
| `no_external_hit` | 1,041 |

Source hit counts:

| Source | Candidate hits |
|---|---:|
| FDA Orange Book | 0 |
| FDA NDC Directory | 0 |
| PubMed | 301 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Status row for every input row | true |
| Deterministic exact/normalized/no-hit status for every row | true |
| PubMed ESearch response for every unique pair key | true |
| PubMed hit rows have PMID lists | true |
| PubMed evidence rows have PMIDs | true |
| All joined rows carry the clinical boundary | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- Current FDA Orange Book and FDA NDC product ingredient co-occurrence did not
  match any of the 1,342 remaining #1232 no-hit candidate rows.
- PubMed ESearch/ESummary added 301 normalized literature co-mention hits,
  across 141 unique pair queries and 523 unique PMIDs.
- The remaining external no-hit set drops from 1,342 to 1,041.
- The PubMed rows are deliberately `normalized_hit`: they are query-level
  title/abstract co-mention evidence, not abstract-body extraction and not
  asserted combination efficacy, safety, or clinical outcome evidence.
- The bounded bridge corpus is materialized into native Calyx vault
  `01KWPYD9HZWK974Z6Z8G839ZYG`.

## Conclusion

#1234 is complete for the current FDA/PubMed source-mining slice: 1,342
remaining candidate rows were rechecked, 301 source-attributed literature
co-mention hits were found, and the bridge corpus was materialized into Calyx
with separate persisted readback.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 79_pubmed_source_text_validation.md

# #1237 PubMed Source-Text Validation

## Scope

#1237 validates the #1234 PubMed query-level co-mention hits against fetched
PubMed source text. The goal is to prevent ESearch hits from being treated as
stronger evidence than the title/abstract text supports.

The stage reads #1234 PubMed evidence rows, fetches PubMed EFetch XML for every
unique PMID, extracts title/abstract source text, verifies candidate drug-name
occurrence in that source text, and assigns a conservative deterministic
relation class:

- `co_mention_only`
- `asserted_combination`
- `asserted_interaction`
- `asserted_outcome`
- `counter_evidence`
- `insufficient_text`

The output is literature triage only. It is not efficacy proof, safety proof,
dosing guidance, treatment guidance, recommendation, clinical actionability, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1237_pubmed_source_text_validation.py
```

The script:

- reads #1234 `pubmed_pair_literature_evidence.jsonl`;
- reads #1234 `candidate_external_source_hits.jsonl`;
- fetches PubMed EFetch XML for all 523 unique PMIDs;
- parses `PubmedArticle` and `PubmedBookArticle` records;
- emits one validation row for every #1234 PubMed evidence row;
- emits one rollup row for every #1234 candidate PubMed-hit pair;
- keeps every promoted-looking row blocked pending safety, outcome,
  falsification, and human-review gates;
- writes an 869-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- NCBI E-utilities intro/rate policy: <https://www.ncbi.nlm.nih.gov/books/NBK25497/>
- NCBI E-utilities parameters: <https://www.ncbi.nlm.nih.gov/books/NBK25499/>
- NLM E-utilities guide: <https://www.nlm.nih.gov/dataguide/eutilities/utilities.html>
- PubMed EFetch endpoint: <https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z/out/persisted_readback.json
sha256: 9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29

/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z/out/calyx_bridge_corpus_readback.json
sha256: b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c
```

Native Calyx materialization:

```text
name: issue1237-pubmed-source-text-validation-20260704t173000z
vault_id: 01KWPZHVX7T7V9QBSND746X9T3
vault_dir: /home/croyse/calyx/vaults/01KWPZHVX7T7V9QBSND746X9T3
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 869 |
| Bridge terms | 669 |
| Graph nodes | 1,538 |
| Graph edges | 7,990 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1234 PubMed evidence | 568 rows | `304400ca4bbc7faea42ac9f2dd708865e7e27a225cbde6e806301d4581b7f7ea` |
| #1234 candidate PubMed hits | 301 rows | `b60932183462b012f284a26750b0412c6a179224849938d7bb05ee42d473e950` |
| #1234 PubMed ESearch responses | 790 rows | `feb6390a6f23eabe273c64710ab9ce5b42c8e8481f101c440389cfabdf3a2339` |
| #1234 PubMed ESummary responses | 4 rows | `b7a871fd3b687d49b63d07e9050db23e1eb97e1b068722d2dcba5e173f5629e3` |
| #1234 persisted readback | 4,368 bytes | `a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16` |
| #1234 Calyx readback | 3,262 bytes | `21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265` |
| NCBI E-utilities intro | 68,713 bytes | `204e634142073f071ade91b62c83e5034fe57c1ca1b1642a4d468aece020c65c` |
| NCBI E-utilities parameters | 114,258 bytes | `46f8c5354fe36d3d5b5386b5910bc5b46a9348a7a020617fa7ab3e9994039a02` |
| NLM E-utilities guide | 72,237 bytes | `08c0e23ecdec38e4fe8c6cdc0d655e87186662573a6d98de93a22a950fa1e081` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pubmed_efetch_responses.jsonl` | 4 | 9,166,957 | `54cc54f6879797f3bc253b522a65bddbdfa7276024b4f82b7d9d73f373e65259` |
| `pubmed_source_records.jsonl` | 523 | 1,510,870 | `6e691f96107fff174115e1406036ff23559c4cdd284bce22186ca9eaf13fd8f0` |
| `pubmed_evidence_validation.jsonl` | 568 | 1,032,727 | `c649b829ae9baf5fe8ff6441e7258547762b1ac286634b949c7b802844c57308` |
| `candidate_pair_pubmed_validation_rollup.jsonl` | 301 | 561,841 | `67ab48f7a8cea6e796ca101531f7deee8fed4af511db9642bf56925c1dcd9052` |
| `candidate_pair_pubmed_validation_hits.jsonl` | 298 | 557,947 | `8d3bc89ba8caaa8a0ca9409d2836a6d789ef24d5b1a6f7e7226e3ee83274f9d9` |
| `pubmed_validation_bridge_rows.jsonl` | 869 | 1,086,712 | `c1835bb07ea73a9636b236e49a4f0a6e05a818f7ed64b1e238a079761e7baeb7` |
| `validation_metrics.json` | - | 12,848 | `9c6e14f650be915cf0c2fdb6c986b6267b455683b62717eda386755753d02033` |
| `output_manifest.json` | - | 2,668 | `b6bdd6738f5caca9386049b99416a09823d1a9cce6b707466f84188eda9b023c` |
| `persisted_readback.json` | - | 3,320 | `9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29` |
| `calyx_bridge_corpus_stdout.json` | - | 735 | `c14ffa44cb635f7fa49d2548a36a44ef085e304702b49b74c8955640af919dac` |
| `calyx_bridge_corpus_readback.json` | - | 3,291 | `b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c` |

## Metrics

| Metric | Count |
|---|---:|
| #1234 PubMed evidence rows | 568 |
| #1234 candidate PubMed-hit rows | 301 |
| Unique PMIDs fetched | 523 |
| PubMed EFetch response chunks | 4 |
| Parsed PubMed source records | 523 |
| Evidence validation rows | 568 |
| Candidate-pair rollup rows | 301 |
| Source-text validated evidence rows | 519 |
| Insufficient-text evidence rows | 49 |
| Counter-evidence rows | 224 |

Evidence relation-class counts:

| Relation class | Rows |
|---|---:|
| `asserted_combination` | 17 |
| `asserted_interaction` | 182 |
| `asserted_outcome` | 87 |
| `co_mention_only` | 9 |
| `counter_evidence` | 224 |
| `insufficient_text` | 49 |

Candidate-pair rollup status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_review_required_still_blocked` | 188 |
| `source_text_validated_still_blocked` | 110 |
| `query_hit_not_validated_by_source_text` | 3 |

Best relation-class counts by candidate pair:

| Best class | Rows |
|---|---:|
| `asserted_combination` | 6 |
| `asserted_interaction` | 68 |
| `asserted_outcome` | 36 |
| `counter_evidence` | 188 |
| `insufficient_text` | 3 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Source record for every unique PMID | true |
| Validation row for every #1234 PubMed evidence row | true |
| Rollup row for every #1234 candidate PubMed hit | true |
| All validation rows carry the clinical boundary | true |
| All validation rows have a relation class | true |
| All non-insufficient rows have both candidate names present | true |
| Insufficient rows do not claim validation | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- #1237 upgraded #1234 from query-level PubMed co-mention to fetched
  title/abstract source-text validation.
- All 523 unique PMIDs returned a source record after parsing both
  `PubmedArticle` and `PubmedBookArticle`.
- 519 of 568 evidence rows physically contain both candidate drug names in
  fetched title/abstract source text.
- 49 evidence rows remain `insufficient_text` and must not be used as validated
  PubMed support.
- 224 evidence rows and 188 candidate-pair rollups are flagged as
  `counter_evidence` review required by the conservative deterministic rules.
- 110 candidate-pair rollups are source-text validated and still blocked pending
  structured extraction, safety, outcome, falsification, and human-review gates.

## Conclusion

#1237 is complete for source-text validation: it proves which #1234 PubMed
query hits have candidate-name support in fetched title/abstract text and keeps
all rows blocked short of clinical claims.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 80_pubmed_structured_extraction.md

# #1238 PubMed Structured Relation/Safety/Outcome Extraction

## Scope

#1238 extracts deterministic structured fields from the #1237 PubMed
source-text validation rows. The stage only reads #1237 sealed artifacts and
does not fetch new source data or call a model.

Eligible evidence rows are #1237 rows with relation class:

- `asserted_combination`
- `asserted_interaction`
- `asserted_outcome`
- `counter_evidence`

The output is literature triage only. It is not efficacy proof, safety proof,
dosing guidance, treatment guidance, recommendation, clinical actionability, or
cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1238_pubmed_structured_extraction.py
```

The script:

- reads #1237 `pubmed_evidence_validation.jsonl`;
- reads #1237 `candidate_pair_pubmed_validation_rollup.jsonl`;
- joins each eligible evidence row to #1237 `pubmed_source_records.jsonl`;
- extracts deterministic source-text fields for relation direction, context,
  model/system, dose/exposure language, outcome/endpoints, safety/adverse-event
  language, and negation/counter-evidence spans;
- emits one structured extraction row per eligible #1237 evidence row;
- emits one structured rollup row per #1237 candidate-pair rollup;
- preserves all `counter_evidence` rows as blocked review inputs;
- writes an 811-row bridge-corpus slice for native Calyx materialization.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z/out/persisted_readback.json
sha256: 14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413

/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z/out/calyx_bridge_corpus_readback.json
sha256: 5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8
```

Native Calyx materialization:

```text
name: issue1238-pubmed-structured-extraction-20260704t164501z
vault_id: 01KWQ0M40D0PDJ52FPMCB1Z1Z7
vault_dir: /home/croyse/calyx/vaults/01KWQ0M40D0PDJ52FPMCB1Z1Z7
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 811 |
| Bridge terms | 625 |
| Graph nodes | 1,436 |
| Graph edges | 9,548 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1237 PubMed evidence validation | 568 rows | `c649b829ae9baf5fe8ff6441e7258547762b1ac286634b949c7b802844c57308` |
| #1237 candidate-pair validation rollup | 301 rows | `67ab48f7a8cea6e796ca101531f7deee8fed4af511db9642bf56925c1dcd9052` |
| #1237 PubMed source records | 523 rows | `6e691f96107fff174115e1406036ff23559c4cdd284bce22186ca9eaf13fd8f0` |
| #1237 persisted readback | 3,320 bytes | `9936093f6db4b18cc6cd86adc5056a8969f88405a00ca5cbd0dd42055539ad29` |
| #1237 Calyx bridge-corpus readback | 3,291 bytes | `b9efc78f8865b6a24d7fb94a380aca9595e2af90abda46a5d98ed291ab8c8d5c` |
| #1237 output manifest | 2,668 bytes | `b6bdd6738f5caca9386049b99416a09823d1a9cce6b707466f84188eda9b023c` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `input_manifest.json` | - | 2,774 | `d6b971c770c621a30aeb2b96fddd08a95dacde27da10f5802fa14c1560d4e3c1` |
| `pubmed_structured_extraction.jsonl` | 510 | 3,965,392 | `10bc9ca90b97a089962bd85ee2f7881819668414859fdedb8040c86d78fa992e` |
| `candidate_pair_pubmed_structured_rollup.jsonl` | 301 | 701,773 | `e0c8db57ee492727fa525c9044dfcce85bf03acbfd7a03587030ab2d8393c3e6` |
| `candidate_pair_pubmed_structured_hits.jsonl` | 298 | 696,699 | `cb6ce01e363c6c320231cfaf01e59c9fafb891c6d437e1a9bc2f5ca924874c26` |
| `pubmed_structured_extraction_bridge_rows.jsonl` | 811 | 1,231,788 | `1abc99b16dc2a437cd287ae4d132609fda47b7a1ff7aa4321fd31ccb123cade3` |
| `validation_metrics.json` | - | 15,572 | `63ad0c804585a3ec634f38f7e519b4cf986e540e75353d4b10cce8a19a8eeceb` |
| `output_manifest.json` | - | 2,099 | `8fc6728117d993bb74ea3e1090ee94136f449ded4c5c89f733ce19fe0ba9ce74` |
| `persisted_readback.json` | - | 3,382 | `14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413` |
| `calyx_bridge_corpus_stdout.json` | - | 742 | `5894bb48d53152880ff0df054b80b4a4625970985abf23b41bed28ea98e390d7` |
| `calyx_bridge_corpus_readback.json` | - | 3,603 | `5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8` |

## Metrics

| Metric | Count |
|---|---:|
| #1237 evidence validation rows | 568 |
| Eligible evidence rows | 510 |
| Structured extraction rows | 510 |
| Candidate-pair structured rollups | 301 |
| Candidate-pair structured hits | 298 |
| Counter-evidence extraction rows | 224 |
| Rows with safety language | 193 |
| Rows with outcome language | 389 |
| Rows with dose/exposure language | 197 |
| Rows with unclear direction | 219 |

Structured relation-class counts:

| Relation class | Rows |
|---|---:|
| `asserted_combination` | 17 |
| `asserted_interaction` | 182 |
| `asserted_outcome` | 87 |
| `counter_evidence` | 224 |

Structured extraction status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_structured_review_required_still_blocked` | 224 |
| `structured_combination_relation_still_blocked` | 17 |
| `structured_interaction_relation_still_blocked` | 182 |
| `structured_outcome_language_still_blocked` | 87 |

Candidate-pair rollup status counts:

| Status | Rows |
|---|---:|
| `counter_evidence_structured_review_required_still_blocked` | 188 |
| `structured_relation_extracted_still_blocked` | 110 |
| `no_structured_extraction_fail_closed` | 3 |

Primary model/system counts:

| Model/system | Rows |
|---|---:|
| `human_clinical_or_patient` | 406 |
| `animal_or_xenograft` | 83 |
| `in_vitro_or_cell_system` | 9 |
| `review_or_guideline` | 2 |
| `computational_or_in_silico` | 1 |
| `unclear` | 9 |

Relation direction counts:

| Direction | Rows |
|---|---:|
| `text_order_drug_a_then_drug_b_not_causal` | 123 |
| `text_order_drug_b_then_drug_a_not_causal` | 168 |
| `undirected_or_unclear` | 219 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| #1237 persisted readback assertions all true | true |
| #1237 Calyx readback assertions all true | true |
| Extraction row for every eligible validation row | true |
| Rollup row for every #1237 candidate rollup | true |
| All extractions have source records | true |
| All extractions carry the clinical boundary | true |
| All rollups carry the clinical boundary | true |
| All extractions remain blocked | true |
| All rollups remain blocked | true |
| Counter-evidence row count preserved | true |
| No co-mention-only or insufficient-text rows extracted | true |
| Bridge rows <= 1,000 | true |
| Bridge rows cover rollups and extractions | true |
| Source records cover extraction PMIDs | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- #1238 converts #1237 source-text validated PubMed evidence into structured
  association metadata for downstream gates.
- 510 of 568 #1237 validation rows were eligible and received structured
  extraction rows.
- 224 counter-evidence rows were preserved exactly as blocked review inputs.
- 298 candidate-pair rollups now have at least one structured extraction row.
- 188 candidate-pair rollups remain blocked by counter-evidence review status.
- 110 candidate-pair rollups have structured relation fields but are still
  blocked pending independent safety, outcome, falsification, and human-review
  gates.
- 3 candidate-pair rollups had no eligible structured extraction and remain
  fail-closed.

## Conclusion

#1238 is complete for deterministic PubMed structured extraction. It provides
source-attributed relation/context/model/safety/outcome/exposure fields for
downstream validation, preserves counter-evidence, and materializes the result
into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 81_openfda_label_source_mining.md

# #1236 openFDA Drug Label Source Mining

## Scope

#1236 rechecks the #1234 remaining no-hit candidate-pair universe against
openFDA Human Drug Label. This source is distinct from DrugComb, NCI ALMANAC,
CDCDB, ClinicalTrials.gov v2, FDA Orange Book, FDA NDC, and PubMed
ESearch/ESummary.

The stage reads #1234 `candidate_external_source_status.jsonl`, filters rows
where `overall_external_source_status == no_external_hit`, queries one openFDA
label search per unique pair key, persists the raw query responses, and verifies
both candidate names in returned label-section text before marking a hit.

The output is source-attributed regulatory-label text mining only. It is not
efficacy proof, safety proof, treatment guidance, dosing guidance,
recommendation, clinical actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1236_openfda_label_source_mining.py
```

The script:

- reads #1234 `candidate_external_source_status.jsonl`;
- filters 1,041 remaining `no_external_hit` candidate rows;
- collapses them to 649 unique pair keys;
- persists openFDA docs/license/terms/download-manifest pages;
- queries openFDA Human Drug Label once per unique pair key;
- verifies both candidate names in returned label sections;
- emits deterministic `exact_hit`, `normalized_hit`, or `no_external_hit`
  status rows for all 1,041 candidate rows;
- emits one pair-status row for each of 649 unique pair keys;
- emits evidence rows only when source label text contains both candidate names;
- writes a 689-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- openFDA download manifest: <https://api.fda.gov/download.json>
- openFDA drug-label overview: <https://open.fda.gov/apis/drug/label/>
- openFDA drug-label endpoint guide: <https://open.fda.gov/apis/drug/label/how-to-use-the-endpoint/>
- openFDA query syntax: <https://open.fda.gov/apis/query-syntax/>
- openFDA authentication/rate-limit docs: <https://open.fda.gov/apis/authentication/>
- openFDA license: <https://open.fda.gov/license/>
- openFDA terms: <https://open.fda.gov/terms/>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/persisted_readback.json
sha256: 9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db

/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/calyx_bridge_corpus_readback.json
sha256: 5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e
```

Native Calyx materialization:

```text
name: issue1236-openfda-label-source-mining-20260704t170534z
vault_id: 01KWQ25ZQ980MBGK7EAY93N5W9
vault_dir: /home/croyse/calyx/vaults/01KWQ25ZQ980MBGK7EAY93N5W9
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 689 |
| Bridge terms | 252 |
| Graph nodes | 941 |
| Graph edges | 4,364 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/openfda_download_manifest.json` | 582,744 | `546c94d92983a0b0c5f7ebe10bddfa2159f23aef9324db22d1ee3074b39c26d1` |
| `raw/openfda_label_overview.html` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `raw/openfda_label_howto.html` | 117,945 | `532743e382cccafd6bf567148d9034ee1e8960aaa2c8287172ebac5ddccc1400` |
| `raw/openfda_query_syntax.html` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `raw/openfda_authentication.html` | 116,494 | `d46c961be22f7eeb4e09f5c209eb81fee8a0119d5a242ac6baca8aac76bb898a` |
| `raw/openfda_license.html` | 120,790 | `9e906a722f7c4116154441bac21df15606fd2fe4335fb1a7b415b1acaf16da97` |
| `raw/openfda_terms.html` | 128,668 | `1a9217ccc118017674dc72ebce4e811706f2d895a82352ab38ffc2165ded0019` |

The openFDA download manifest reported drug-label export date `2026-07-04`,
260,158 total label records, and 14 partitions.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `openfda_label_query_responses.jsonl` | 649 | 7,841,289 | `bc20dab983c0ba7ace46a33cccc2a18b9699ed974919070433700595456f841c` |
| `openfda_label_pair_evidence.jsonl` | 40 | 376,714 | `d2376732cea19e4f17d4f803e5e7c0b07cbd6ef4898fb7b527a6a1509c304ab3` |
| `openfda_label_pair_status.jsonl` | 649 | 1,722,675 | `9a1680d5243a90ab52af7bb93b76f8e7677c7b7cef338282bf1cd781b5d7524a` |
| `candidate_openfda_label_status.jsonl` | 1,041 | 1,296,930 | `0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b` |
| `openfda_label_bridge_rows.jsonl` | 689 | 876,827 | `cbbde20b652f98afada1f111978f8711c7a0d46a825e2ab02d9390c5f2a31047` |
| `input_manifest.json` | - | 5,284 | `ced99b743efb759fc4369af4cc7acaa4c17e124cc740c04bb7e4dff4f6ded4d6` |
| `validation_metrics.json` | - | 7,263 | `76b53ba9966d3168230ff58d3a4c55ee31b1ce9ec3e3461bf1a2d1305c090d4c` |
| `output_manifest.json` | - | 2,357 | `c69cb405c3d36dce0536eb37361937aecaf4fd9d0d82d041a83b398053934420` |
| `persisted_readback.json` | - | 3,602 | `9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db` |
| `calyx_bridge_corpus_stdout.json` | - | 701 | `2bbc2bf40e909c345b641ef242dedf460e4391f1cee1f369b98d882bd234cfba` |
| `calyx_bridge_corpus_readback.json` | - | 3,555 | `5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e` |

## Metrics

| Metric | Count |
|---|---:|
| #1234 remaining no-hit candidate rows | 1,041 |
| Unique pair keys queried | 649 |
| openFDA query response rows | 649 |
| Query responses with HTTP 200 | 9 |
| Query responses with HTTP 404 | 640 |
| Query rows with verified evidence | 9 |
| openFDA label evidence rows | 40 |
| Candidate rows with #1236 hit | 22 |
| Remaining no-hit rows after #1236 | 1,019 |
| Evidence rows with safety-section match | 26 |
| Evidence rows with interaction-section match | 28 |

Pair status counts:

| Status | Unique pair keys |
|---|---:|
| `exact_hit` | 8 |
| `normalized_hit` | 1 |
| `no_external_hit` | 640 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `exact_hit` | 21 |
| `normalized_hit` | 1 |
| `no_external_hit` | 1,019 |

Matched section fields:

| Field | Evidence rows |
|---|---:|
| `drug_interactions` | 27 |
| `drug_interactions_table` | 20 |
| `precautions` | 24 |
| `warnings_and_cautions` | 2 |
| `clinical_pharmacology` | 1 |
| `pharmacokinetics` | 1 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| #1234 persisted readback assertions all true | true |
| #1234 Calyx readback assertions all true | true |
| Candidate status row for every remaining no-hit row | true |
| Pair status row for every unique pair key | true |
| Query response for every queryable pair key | true |
| All pair/candidate statuses in allowed set | true |
| All evidence/status rows carry the clinical boundary | true |
| All hits have evidence | true |
| All evidence rows have matched sections and source IDs | true |
| All candidate rows remain blocked | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- openFDA label mining found 9 unique pair keys with verified label-section
  evidence among the 649 unique #1234 remaining no-hit pair keys.
- These 9 pair keys map to 22 candidate rows.
- 40 openFDA label evidence rows were persisted.
- 26 evidence rows match safety-related label sections and 28 match
  interaction-related label sections; these are review inputs only and do not
  clear safety or pair-interaction gates.
- 1,019 candidate rows remain no-hit after #1236.

## Conclusion

#1236 is complete for openFDA Human Drug Label source mining. It adds
source-attributed label evidence for 22 previously no-hit candidate rows,
keeps all rows blocked, and materializes the result into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.

---

## 82_rxnorm_combination_product_mining.md

# #1240 RxNorm Combination-Product Source Mining

Status: complete.

This slice continued external source mining after #1236 by rechecking the
remaining no-hit combination-candidate universe against current NLM RxNorm
combination-product concept search.

Clinical boundary:

```text
RxNorm combination-product concept evidence is source-attributed vocabulary/product-concept mining only; not efficacy, safety, pair interaction clearance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/candidate_openfda_label_status.jsonl
sha256: 0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1236 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: NLM RxNorm API `/REST/drugs.json`.
- RxNorm API version readback: `3.1.353`.
- RxNorm data version readback: `01-Jun-2026`.
- RxNav rate-limit contract used for scheduling: 20 requests/second/IP.
- The discontinued RxNav drug/drug interaction API was explicitly excluded.

## Method

The miner:

- persisted official RxNorm/RxNav API, terms, FAQ, overview, and version source bytes;
- queried each pair key in both deterministic directions, `drug_a / drug_b` and `drug_b / drug_a`;
- required both candidate names to appear in the returned RxNorm concept name or synonym before marking a hit;
- wrote exact/normalized/no-hit status rows with raw response hashes;
- preserved all candidate rows as blocked research triage rows;
- wrote a 640-row bridge-corpus slice for native Calyx materialization.

Each `rxnorm_combination_query_responses.jsonl` row represents one pair key and
contains both directional HTTP responses. The run made 1,280 HTTP requests, all
HTTP 200, and returned zero RxNorm concept rows for this candidate universe.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/rxnorm_api_docs.html` | 21,042 | `d037f07cac2e2f18225cff27f792c2d0133d945d67b175108d0866d4acfd49d8` |
| `raw/rxnav_terms.html` | 16,041 | `eebd61ce17cc426e231c159b10a925bdd4b39744141060d85acba5fe90b22911` |
| `raw/rxnav_faq.html` | 15,033 | `c5504507b52c82d079ffd3fd1e6761a212c5a36feafc09a5874020510221f05d` |
| `raw/rxnav_overview.html` | 26,791 | `2b58996bd16679950ac8916f68493641292d5a3dc1a0e007eaee3864dae6eb64` |
| `raw/rxnorm_version.json` | 48 | `dea926aeb5b147381336d15114555883ad9759d31056e57bc4818b26c3d9a461` |

Source URLs:

- https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html
- https://lhncbc.nlm.nih.gov/RxNav/TermsofService.html
- https://lhncbc.nlm.nih.gov/RxNav/information/FAQs.html
- https://lhncbc.nlm.nih.gov/RxNav/
- https://rxnav.nlm.nih.gov/REST/version.json

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `rxnorm_combination_query_responses.jsonl` | 640 | 959,263 | `f3c33477764790aa35f7a3bcc5f3d7c92b988ba1ff033b7a2369f3801c6c86c3` |
| `rxnorm_combination_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `rxnorm_pair_status.jsonl` | 640 | 876,665 | `68c2d136ff82dc6ded1c141e17040197e9cd264914fa6594ae4f73737e02bfc1` |
| `candidate_rxnorm_status.jsonl` | 1,019 | 1,292,381 | `5293210308d19be37c976d55a34813352c7f473da152be6348991e35deb829f7` |
| `rxnorm_combination_bridge_rows.jsonl` | 640 | 829,487 | `39273aaddc150db2af62fffc95f5fceadf4176f165d2cecb0a8fd17db233c9f8` |
| `input_manifest.json` | - | 4,610 | `9ed6c4d205cf9e8e80f665651d11634d99fb76798e68ec1eb0521c6aa240d368` |
| `validation_metrics.json` | - | 1,063 | `a69afe7b745275e7326b66556d3a2907c766f52740d60463318863a5bdfe69f8` |
| `output_manifest.json` | - | 2,395 | `0687c62e326d010b98f905fb59bd8b33c0d1b4eb6df426bd3ef0cf6d66541b67` |
| `persisted_readback.json` | - | 3,581 | `b0be6e9b90c6cb9220e1ae0209ae9390ef89c12d29258ab00fbad4c81c0797e9` |
| `calyx_bridge_corpus_stdout.json` | - | 686 | `b3c0ba3daea69fb3be6254d5ac6cf6478670ebf1ef32b0bd65e1c8136c6ee61f` |
| `calyx_bridge_corpus_readback.json` | - | 3,812 | `b56967b025bf45d4befee9060012da8e4374a0a81c2e320ba558bf53cb87aef4` |

## Metrics

| Metric | Count |
|---|---:|
| #1236 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Directional HTTP requests | 1,280 |
| HTTP 200 responses | 1,280 |
| Query rows with returned RxNorm concepts | 0 |
| Verified RxNorm concept evidence rows | 0 |
| Candidate rows with #1240 hit | 0 |
| Remaining no-hit candidate rows after #1240 | 1,019 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 0 | 0 | 640 |
| Candidate status rows | 0 | 0 | 1,019 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1236 persisted readback all true | true |
| #1236 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All candidate rows remain blocked | true |
| All hits have evidence | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1240-rxnorm-combination-products-20260704t172703z
vault_id: 01KWQ3AQAXTC6SXY3G57RVX4Q8
vault_dir: /home/croyse/calyx/vaults/01KWQ3AQAXTC6SXY3G57RVX4Q8
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 640 |
| Bridge terms | 205 |
| Graph nodes | 845 |
| Graph edges | 3,840 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

RxNorm combination-product concept mining produced no verified source hits for
the #1236 remaining no-hit universe. The useful result is negative evidence:
1,019 candidate rows remain `no_external_hit` after this additional public
source check, with source bytes, raw API responses, hashes, status rows, and
Calyx graph materialization persisted.

Downstream work should continue source expansion from
`candidate_rxnorm_status.jsonl`, filtered to
`overall_external_source_status_after_issue1240 == no_external_hit`.

---

## 83_dailymed_spl_title_source_mining.md

# #1242 DailyMed SPL Title Source Mining

Status: complete.

This slice continued external source mining after #1240 by rechecking the
remaining no-hit combination-candidate universe against current NLM DailyMed v2
SPL metadata pair-title search.

Clinical boundary:

```text
DailyMed SPL-title metadata evidence is source-attributed label metadata co-mention only; not label-content interpretation, efficacy, safety, pair interaction clearance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z/out/candidate_rxnorm_status.jsonl
sha256: 5293210308d19be37c976d55a34813352c7f473da152be6348991e35deb829f7
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1240 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: NLM DailyMed v2 `/spls.json`.
- Query mode: direct pair metadata search with `drug_name`, `name_type=both`, `pagesize=100`, `page=1`.
- The source is SPL metadata title search only; it does not interpret label text or claim interaction meaning.

## Method

The miner:

- persisted official DailyMed web-service, `/spls`, about, and home pages;
- queried each pair key in both deterministic directions, `drug_a drug_b` and `drug_b drug_a`;
- required both candidate names to appear in returned SPL titles before marking a hit;
- wrote exact/normalized/no-hit status rows with raw response hashes;
- preserved all candidate rows as blocked research triage rows;
- wrote a 640-row bridge-corpus slice for native Calyx materialization.

Each `dailymed_spl_title_query_responses.jsonl` row represents one pair key
and contains both directional HTTP responses. The run made 1,280 HTTP requests,
all HTTP 200, and returned zero SPL metadata records for this candidate
universe.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/dailymed_web_services.html` | 85,397 | `3fbe63342062c085fcbb85f1431f6c1614bbacc6b2ade5adc21de8eba3c4327e` |
| `raw/dailymed_spls_api.html` | 91,702 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `raw/dailymed_about.html` | 82,626 | `f8d463a597663bfc1b0e5aa49a566ca62d77950f2b5b132a149f088623cc9d96` |
| `raw/dailymed_home.html` | 75,305 | `4e39c0a4acdfb8e540649d1641ff366d8132002cd4e6dfa5e458969b49126ee0` |

Source URLs:

- https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm
- https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm
- https://dailymed.nlm.nih.gov/dailymed/about-dailymed.cfm
- https://dailymed.nlm.nih.gov/dailymed/

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `dailymed_spl_title_query_responses.jsonl` | 640 | 2,042,265 | `4fe25859869849d5172d79d11b6226eba14cd83fc26a2cf1d8bd311cc1f38ebd` |
| `dailymed_spl_title_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `dailymed_spl_title_pair_status.jsonl` | 640 | 793,368 | `64e1a84ffb4252ebac671f15e94bd3b617a69dcb0294b817852bfbec8c9004fc` |
| `candidate_dailymed_title_status.jsonl` | 1,019 | 1,378,996 | `e834783552a1b94172b1b424f15f9f6403ada53439e85a3bea67edd458f0bd26` |
| `dailymed_spl_title_bridge_rows.jsonl` | 640 | 833,585 | `87046e260c78d85eff97f5c3fd2f8da7f81c472e1b8d01f7b05ee5ebae4fb8ad` |
| `input_manifest.json` | - | 4,351 | `f33319270ca864a1aecae5f6766cf81f5c0a56fd8b17fd1ca7e7d32e3153da08` |
| `validation_metrics.json` | - | 1,297 | `812a690a8f9004f57a1f092822396a918f420842c440e1e7ca0a35d9f5dae415` |
| `output_manifest.json` | - | 2,437 | `2b3d51f75139253e78e4338f0c1691452a2297f66cf6a8e917ed29986d8da3f9` |
| `persisted_readback.json` | - | 3,674 | `4b0c05ba8190b6f46ffdf8ff9759bfa186a784cee6da58c4be9d95daaa027147` |
| `calyx_bridge_corpus_stdout.json` | - | 676 | `cbd6ff3378b2cb4050187a0822567bff2673681cd5266d8fd8cf2fdd244523fa` |
| `calyx_bridge_corpus_readback.json` | - | 3,839 | `096df2c2282176d905b7507346b88b04a7bbbb261fb6e92d323b6e7763873944` |

## Metrics

| Metric | Count |
|---|---:|
| #1240 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Directional HTTP requests | 1,280 |
| HTTP 200 responses | 1,280 |
| Total SPL metadata records returned | 0 |
| Verified DailyMed title evidence rows | 0 |
| Candidate rows with #1242 hit | 0 |
| Remaining no-hit candidate rows after #1242 | 1,019 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 0 | 0 | 640 |
| Candidate status rows | 0 | 0 | 1,019 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1240 persisted readback all true | true |
| #1240 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| Two directional responses per query row | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All candidate rows remain blocked | true |
| All hits have evidence | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1242-dailymed-spl-title-mining-20260704t174500z
vault_id: 01KWQ4CB0F2XPC1N8KMR7VJ962
vault_dir: /home/croyse/calyx/vaults/01KWQ4CB0F2XPC1N8KMR7VJ962
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 640 |
| Bridge terms | 845 |
| Graph nodes | 1,485 |
| Graph edges | 5,120 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

DailyMed SPL title metadata mining produced no verified source hits for the
#1240 remaining no-hit universe. The useful result is another negative source
check: 1,019 candidate rows remain `no_external_hit`, with official source
bytes, raw API responses, hashes, status rows, and Calyx graph materialization
persisted.

Downstream work should continue source expansion from
`candidate_dailymed_title_status.jsonl`, filtered to
`overall_external_source_status_after_issue1242 == no_external_hit`.

---

## 84_europepmc_pair_search_mining.md

# #1243 Europe PMC Pair-Search Source Mining

Status: complete.

This slice continued external source mining after #1242 by rechecking the
remaining no-hit combination-candidate universe against Europe PMC Articles
REST API pair search and bounded PMCID full-text XML checks.

Clinical boundary:

```text
Europe PMC pair-search evidence is source-attributed literature/index co-mention only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z
```

Sealed upstream input:

```text
/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z/out/candidate_dailymed_title_status.jsonl
sha256: e834783552a1b94172b1b424f15f9f6403ada53439e85a3bea67edd458f0bd26
```

Source contract:

- The input filter was `overall_external_source_status_after_issue1242 == no_external_hit`.
- Input rows after the filter: 1,019.
- Unique pair keys queried: 640.
- Source: Europe PMC Articles REST API `/search`.
- Query mode: `"<drug_a>" AND "<drug_b>"`, `format=json`, `resultType=core`, `pageSize=5`, `cursorMark=*`, `synonym=false`.
- Search hit counts were not promoted by themselves. A hit required both candidate terms to be physically present in returned metadata text or bounded fetched PMCID full-text XML.
- The source is literature/index co-mention only; it does not prove relation direction, mechanism, safety, efficacy, treatment actionability, or cure evidence.

## Method

The miner:

- persisted official Europe PMC REST, annotations, developer, and about pages;
- queried each remaining pair key once through Europe PMC Articles REST search;
- recorded every raw JSON response hash and schema fingerprint;
- checked returned title, abstract, keywords, publication types, journal metadata, and related metadata text for both terms;
- fetched up to three PMCID `fullTextXML` records per pair when metadata did not verify both terms;
- did not cache failed full-text responses;
- wrote exact/normalized/no-hit status rows with blocked promotion state;
- wrote a 1,000-row bridge-corpus slice for native Calyx materialization.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/europepmc_rest_docs.html` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `raw/europepmc_annotations_docs.html` | 57,216 | `d8109ea920e0f5d0a13de65bb315125911fb22c020b06167c1346b32b9b15cc7` |
| `raw/europepmc_developers.html` | 54,465 | `d7f25854429706dc9a25b009a4616fadd8332ee47375917ab264974e3edbeace` |
| `raw/europepmc_about.html` | 75,965 | `69366291c25e6e895353dd6f038c5e747314678815964a1b131c603f4808eee1` |

Source URLs:

- https://europepmc.org/RestfulWebService
- https://europepmc.org/AnnotationsApi
- https://europepmc.org/developers
- https://europepmc.org/About

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_pair_query_responses.jsonl` | 640 | 9,755,246 | `f7e367b08b4558443abd7ec82fc8b76be9b05db5518daa61e03b9228cbc8630b` |
| `europepmc_fulltext_fetches.jsonl` | 772 | 865,857 | `5acc828a3c8e6347a23adcafaeb12764fb3a5b40a2e2cc6aa979cef0003b4e35` |
| `europepmc_pair_evidence.jsonl` | 598 | 1,209,359 | `a393992fb1e5b0d8a83f6f99e27e6b1c6aeeb5563615bb2a729b0602075aa609` |
| `europepmc_pair_status.jsonl` | 640 | 847,926 | `153b7f222495f25244d6e221b5cc46ca1830335a12759867328b9a26b2384f9d` |
| `candidate_europepmc_status.jsonl` | 1,019 | 1,375,255 | `7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03` |
| `europepmc_bridge_rows.jsonl` | 1,000 | 1,190,452 | `785cd4bcb6c34b59bcff9d90290792fd4cc0a100ac7032ca4b9514471aadda73` |
| `input_manifest.json` | - | 4,018 | `1b528f52edae5cd3c323e7a1aaa0b862b4c337ac2836014dd1e614a594f279cb` |
| `validation_metrics.json` | - | 9,018 | `f3c4744fd405106951d93d04ade4bdf553632ed386456d9ea3b0ff6859c44abc` |
| `output_manifest.json` | - | 2,590 | `04e774bc62d15372012440258a53fb5f449e83199cb9d3c98e82faa9830569ba` |
| `persisted_readback.json` | - | 3,891 | `37e6ca92f1a3bfc5f0945f64c538bc38e96911a1281951fe92faf3f13828717a` |
| `calyx_bridge_corpus_stdout.json` | - | 683 | `210746fe8274746a1da591960ade38f64b0f61f2aaa3d745a9b89b9e95b65dbc` |
| `calyx_bridge_corpus_readback.json` | - | 5,103 | `781c312979e4a17f12654f8e550cf7127e2c08b169c59c231685e33ec9de0fed` |

## Metrics

| Metric | Count |
|---|---:|
| #1242 remaining no-hit candidate rows checked | 1,019 |
| Unique pair keys queried | 640 |
| Europe PMC query response rows | 640 |
| Search responses HTTP 200 | 640 |
| Pair keys with Europe PMC search hit count > 0 | 317 |
| Total Europe PMC search hit count | 6,140 |
| Returned result rows inspected | 1,137 |
| Full-text fetch rows | 772 |
| Full-text HTTP 200 rows | 649 |
| Full-text HTTP 404 rows | 123 |
| Verified Europe PMC evidence rows | 598 |
| Metadata-text evidence rows | 1 |
| PMCID full-text XML evidence rows | 597 |
| Candidate rows with #1243 hit | 487 |
| Remaining no-hit candidate rows after #1243 | 532 |

Status counts:

| Status scope | `exact_hit` | `normalized_hit` | `no_external_hit` |
|---|---:|---:|---:|
| Pair status rows | 280 | 7 | 353 |
| Candidate status rows | 474 | 13 | 532 |

Top evidence-count examples:

| Pair key | Status | Evidence rows | Search hit count | Source ids |
|---|---|---:|---:|---|
| `levetiracetam||thymidine` | `exact_hit` | 4 | 179 | `PMC11910025`, `PMC12332248`, `PMC12648844`, `PMC13099364` |
| `gentamicin||sunitinib` | `exact_hit` | 3 | 276 | `PMC11764070`, `PMC12969023`, `PMC13024260` |
| `alpelisib||rituximab` | `exact_hit` | 3 | 219 | `PMC11825581`, `PMC11946485`, `PMC13111025` |
| `dactolisib||metformin` | `exact_hit` | 3 | 144 | `PMC11442590`, `PMC13156086`, `PMC13238955` |
| `gentamicin||vemurafenib` | `exact_hit` | 3 | 128 | `PMC11601706`, `PMC12191169`, `PMC13158949` |

Validation assertions:

| Assertion | Result |
|---|---|
| #1242 persisted readback all true | true |
| #1242 Calyx readback all true | true |
| Candidate status rows cover every remaining no-hit row | true |
| Pair status rows cover every unique pair key | true |
| Query response exists for every queryable pair key | true |
| All query HTTP statuses are 200 | true |
| All status values are allowed | true |
| All status rows carry the clinical boundary | true |
| All hits have evidence | true |
| Full-text matches have evidence | true |
| All evidence rows have source ids | true |
| All candidate rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1243-europepmc-pair-search-20260704t181500z
vault_id: 01KWQ6CE6VEN48FFP2TDD4ESV0
vault_dir: /home/croyse/calyx/vaults/01KWQ6CE6VEN48FFP2TDD4ESV0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,109 |
| Graph nodes | 2,109 |
| Graph edges | 9,440 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

Europe PMC pair-search source mining converted 487 of the 1,019 #1242
remaining no-hit candidate rows into source-attributed co-mention hits, with
598 verified evidence rows and native Calyx graph materialization. Another 532
candidate rows remain `no_external_hit`.

These rows remain research triage only. Promotion requires source-text relation
extraction, safety/outcome/falsification gates, and human review. The remaining
no-hit rows should continue through source expansion from
`candidate_europepmc_status.jsonl`, filtered to
`overall_external_source_status_after_issue1243 == no_external_hit`.

---

## 85_europepmc_relation_validation.md

# #1244 Europe PMC Source-Text Relation Validation

Status: complete.

This slice validated the #1243 Europe PMC co-mention hits by reopening the
persisted source text, extracting bounded source-text windows around both
candidate terms, and classifying relation/safety/outcome context with
deterministic lexical gates.

Clinical boundary:

```text
Europe PMC source-text relation validation is literature triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/candidate_europepmc_status.jsonl
sha256: 7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_evidence.jsonl
sha256: a393992fb1e5b0d8a83f6f99e27e6b1c6aeeb5563615bb2a729b0602075aa609
```

Source contract:

- The input filter was `europepmc_status in {exact_hit, normalized_hit}`.
- Candidate hit rows from #1243: 487.
- Europe PMC evidence rows from #1243: 598.
- No new source data was fetched. The validator read only persisted #1243
  query responses and cached PMCID full-text XML files.
- Relation, safety, outcome, counter, and dose/exposure patterns were evaluated
  on the bounded nearest-term source-text window, not on whole articles.
- Every output row remains blocked pending independent safety, outcome,
  falsification, and human-review gates.

## Method

The validator:

- re-read each #1243 evidence row and source id;
- reconstructed metadata source text from the persisted Europe PMC response or
  read the cached PMCID `fullTextXML` payload;
- verified both candidate names in source text;
- emitted one validation row per #1243 evidence row;
- emitted one rollup row per #1243 hit candidate row;
- classified bounded source windows into `co_mention_only`,
  `combination_or_coexposure`, `comparative_context`,
  `mechanistic_or_interaction_context`, `trial_or_outcome_context`,
  `safety_or_adverse_context`, or `counter_or_negative_context`;
- preserved safety/counter/outcome/dose flags as review fields, not claims;
- wrote a 1,000-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_source_text_relation_validation.jsonl` | 598 | 15,685,763 | `ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2` |
| `candidate_europepmc_relation_rollup.jsonl` | 487 | 1,173,411 | `730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef` |
| `candidate_europepmc_relation_review.jsonl` | 471 | 1,141,991 | `1d097af6e336a0ef2dc4f23b599fdf318ede6921218e79779a35371f51fc615b` |
| `europepmc_relation_bridge_rows.jsonl` | 1,000 | 1,671,617 | `50ce1ac85107522ebf2d37d8ed973f988b8a374c247926386541ad6788ba5a62` |
| `input_manifest.json` | - | 2,654 | `cf1556de9dc566fd72bc1e26491d18411346df020dc980683be07091d01047d0` |
| `validation_metrics.json` | - | 16,700 | `6f0faf3174c9015d30847a4a8b67a8a6f36d9281b84aba0dc2eb22326fb4d379` |
| `output_manifest.json` | - | 2,126 | `edfbc1ceac05a064ae60dc78d51e3d08fbee1abc6db3f50cdd1b139dc1f8683c` |
| `persisted_readback.json` | - | 3,396 | `acd861001ffa19c6e3ece25d1e70071b1096b0a7eeb6855be12797e14eab7a0e` |
| `calyx_bridge_corpus_stdout.json` | - | 740 | `ce4f159f1ea6998feed77b0fa42a865c3d36d81ee90a2414fe07a6490b771e4a` |
| `calyx_bridge_corpus_readback.json` | - | 5,173 | `2341f995f0d0b81fe9093bbaa4483be0992f4e17ab44d069bad39a74bb1f687d` |

## Metrics

| Metric | Count |
|---|---:|
| #1243 candidate rows read | 1,019 |
| #1243 hit candidate rows | 487 |
| #1243 evidence rows validated | 598 |
| Candidate relation rollup rows | 487 |
| Candidate rollups with source-text terms verified | 487 |
| Candidate relation review rows | 471 |
| Rows with safety language | 270 |
| Rows with counter/negative language | 265 |
| Rows with outcome language | 459 |
| Rows with dose/exposure language | 246 |

Evidence relation-class counts:

| Relation class | Rows |
|---|---:|
| `co_mention_only` | 55 |
| `combination_or_coexposure` | 11 |
| `comparative_context` | 1 |
| `counter_or_negative_context` | 265 |
| `mechanistic_or_interaction_context` | 196 |
| `safety_or_adverse_context` | 45 |
| `trial_or_outcome_context` | 25 |

Evidence status counts:

| Status | Rows |
|---|---:|
| `source_text_comention_only_still_blocked` | 55 |
| `source_text_relation_extracted_still_blocked` | 233 |
| `source_text_safety_or_counter_review_required_still_blocked` | 310 |

Candidate rollup status counts:

| Status | Rows |
|---|---:|
| `source_text_comention_only_still_blocked` | 16 |
| `source_text_relation_extracted_still_blocked` | 108 |
| `source_text_safety_or_counter_review_required_still_blocked` | 363 |

Primary model/system counts:

| Model/system | Rows |
|---|---:|
| `human_clinical_or_patient` | 461 |
| `in_vitro_or_cell_system` | 51 |
| `unclear` | 46 |
| `computational_or_in_silico` | 21 |
| `animal_or_xenograft` | 18 |
| `review_or_guideline` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1243 persisted readback all true | true |
| #1243 Calyx readback all true | true |
| Validation row for every #1243 evidence row | true |
| Rollup row for every #1243 hit candidate | true |
| Validation rows reference known evidence | true |
| Validation rows have source text | true |
| Validation rows verify both terms | true |
| Validation rows carry the clinical boundary | true |
| Rollups carry the clinical boundary | true |
| Validation rows remain blocked | true |
| Rollups remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1244-europepmc-relation-validation-20260704t193000z
vault_id: 01KWQ7E16HBVTNPKH848WZQFJM
vault_dir: /home/croyse/calyx/vaults/01KWQ7E16HBVTNPKH848WZQFJM
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,830 |
| Graph nodes | 2,830 |
| Graph edges | 14,052 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1244 converted the #1243 Europe PMC co-mention hits into bounded source-text
relation/safety/outcome/counter/dose triage rows. All 487 hit-candidate rollups
verified source text for both terms. The useful result is not a treatment claim:
363 candidate rollups require safety/counter review, 108 have relation context
but remain blocked, and 16 remain co-mention-only.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.

---

## 86_europepmc_safety_counter_review.md

# #1246 Europe PMC Safety/Counter Review

Status: complete.

This slice reviewed the #1244 Europe PMC rollups that carried bounded
source-text safety, adverse, counter, or negative language. It separated those
signals into review categories while keeping every row blocked from clinical
promotion.

Clinical boundary:

```text
Europe PMC safety/counter review is literature triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/candidate_europepmc_relation_rollup.jsonl
sha256: 730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef

/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/europepmc_source_text_relation_validation.jsonl
sha256: ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2
```

Source contract:

- The input filter was `source_text_validation_status == source_text_safety_or_counter_review_required_still_blocked`.
- Scoped #1244 rollups: 363.
- The stage did not fetch new source data; it read #1244 source-text windows and spans.
- Output categories are review flags only, not safety findings or clinical advice.

## Method

The reviewer:

- emitted one rollup review row for each scoped #1244 rollup;
- emitted evidence-review rows for linked #1244 source-text validation rows;
- separated fatality/mortality, contraindication/avoidance, toxicity,
  adverse/safety, counter/negative, and generic risk language;
- preserved source ids, source-window text, source hashes, and source relation
  classes;
- kept every row blocked pending independent safety/falsification and human
  review;
- wrote a 673-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_safety_counter_evidence_review.jsonl` | 310 | 5,133,255 | `2906d957d06b38ce9ef1f33dc2b42f9984adf9055a1295d9d598ce7fb4a65089` |
| `candidate_europepmc_safety_counter_rollup.jsonl` | 363 | 780,131 | `b7fddaf84c0c9a09dcbb8a6e59e505a6fe225fe4dce5f0167cd165d4b7b06255` |
| `europepmc_safety_counter_bridge_rows.jsonl` | 673 | 1,002,138 | `c6aa943c39c87e55750f81f8ba7637b7207317dfed096c0d4271c10f94ba73e9` |
| `input_manifest.json` | - | 2,059 | `3e264a939191dafc39215860736402b07d36d0fdfd7cd782c838565d373df41b` |
| `validation_metrics.json` | - | 16,470 | `68cc65a7e9db9daeff2b990905cfc0d39ac86c66b76469521e7aa47082afcd45` |
| `output_manifest.json` | - | 1,851 | `3d53fa444de3dbf043c72e85c7bcd45f768df723f2d4b0b4cb28813e2c49d17e` |
| `persisted_readback.json` | - | 2,907 | `3e0239ed1b237441da320ad503ac50804e3498da74dcffca5e8e37b2a889faab` |
| `calyx_bridge_corpus_stdout.json` | - | 737 | `ba7720baec1d6971ad4c23cf109538c73de9ef79af1fb70d29e1ca95b5ea9b9b` |
| `calyx_bridge_corpus_readback.json` | - | 4,709 | `15f05e9bfd789f766de867916aa95af33e915613ef145336fb1ec805c0f45da3` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped #1244 rollups | 363 |
| Evidence-review rows | 310 |
| Rollup-review rows | 363 |
| Rollups with fatality/mortality language | 125 |
| Rollups with contraindication/avoidance language | 63 |
| Rollups with toxicity language | 170 |
| Rollups with counter/negative language | 330 |
| Rollups with safety/adverse language | 340 |

Evidence category counts:

| Category | Rows |
|---|---:|
| `contraindication_or_avoidance_language_review` | 22 |
| `counter_negative_and_safety_language_review` | 68 |
| `counter_negative_language_review` | 40 |
| `fatality_or_mortality_language_review` | 92 |
| `safety_adverse_language_review` | 20 |
| `toxicity_language_review` | 68 |

Rollup category counts:

| Category | Rows |
|---|---:|
| `contraindication_or_avoidance_language_review` | 40 |
| `counter_negative_and_safety_language_review` | 82 |
| `counter_negative_language_review` | 23 |
| `fatality_or_mortality_language_review` | 125 |
| `safety_adverse_language_review` | 15 |
| `toxicity_language_review` | 78 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1244 persisted readback all true | true |
| #1244 Calyx readback all true | true |
| Rollup review for every scoped rollup | true |
| Reviewed pair ids match scope | true |
| Rollup reviews have evidence | true |
| Evidence reviews have source windows | true |
| Evidence reviews carry the clinical boundary | true |
| Rollup reviews carry the clinical boundary | true |
| Evidence reviews remain blocked | true |
| Rollup reviews remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1246-europepmc-safety-counter-review-20260704t203000z
vault_id: 01KWQ8BK8XQK3AN77PMBEF3WRN
vault_dir: /home/croyse/calyx/vaults/01KWQ8BK8XQK3AN77PMBEF3WRN
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 673 |
| Bridge terms | 1,274 |
| Graph nodes | 1,947 |
| Graph edges | 7,350 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1246 accounted for all 363 #1244 rollups requiring safety/counter review and
separated 310 linked evidence rows into review categories. The result is a
blocked safety/falsification triage layer for downstream human and independent
validation gates.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, recommendation, clinical actionability, or cure claim is made.

---

## 87_openfda_independent_safety_validation.md

# #1248 openFDA Independent Safety-Source Validation

Status: complete.

This slice checked the #1246 Europe PMC safety/counter review rollups against
openFDA Human Drug Label as an independent regulatory-label source. It queried
openFDA label sections for each unique pair key and required both candidate
terms in the same label section before marking an independent source hit.

Clinical boundary:

```text
openFDA independent safety validation is regulatory-label text triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z/out/candidate_europepmc_safety_counter_rollup.jsonl
sha256: b7fddaf84c0c9a09dcbb8a6e59e505a6fe225fe4dce5f0167cd165d4b7b06255

/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z/out/europepmc_safety_counter_evidence_review.jsonl
sha256: 2906d957d06b38ce9ef1f33dc2b42f9984adf9055a1295d9d598ce7fb4a65089
```

Source contract:

- Input rollups: 363.
- Unique pair keys queried: 202.
- Source: openFDA Human Drug Label API.
- Query mode: paired exact-label-section search across safety, interaction,
  pharmacology, and description sections.
- A hit required both pair terms in the same returned label section.
- A no-hit is not safety clearance; it only means the targeted openFDA query did
  not return a same-section pair match.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/openfda_download_manifest.json` | 582,744 | `546c94d92983a0b0c5f7ebe10bddfa2159f23aef9324db22d1ee3074b39c26d1` |
| `raw/openfda_label_overview.html` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `raw/openfda_label_howto.html` | 117,945 | `532743e382cccafd6bf567148d9034ee1e8960aaa2c8287172ebac5ddccc1400` |
| `raw/openfda_query_syntax.html` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `raw/openfda_authentication.html` | 116,494 | `d46c961be22f7eeb4e09f5c209eb81fee8a0119d5a242ac6baca8aac76bb898a` |
| `raw/openfda_license.html` | 120,790 | `9e906a722f7c4116154441bac21df15606fd2fe4335fb1a7b415b1acaf16da97` |
| `raw/openfda_terms.html` | 128,668 | `1a9217ccc118017674dc72ebce4e811706f2d895a82352ab38ffc2165ded0019` |

Source URLs:

- https://api.fda.gov/download.json
- https://open.fda.gov/apis/drug/label/
- https://open.fda.gov/apis/drug/label/how-to-use-the-endpoint/
- https://open.fda.gov/apis/query-syntax/
- https://open.fda.gov/apis/authentication/
- https://open.fda.gov/license/
- https://open.fda.gov/terms/

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `openfda_independent_query_responses.jsonl` | 202 | 374,817 | `855ed0237185d46474e03ab5ddb81fb6e69b45ffe747b66290dbdb374af2b57c` |
| `openfda_independent_label_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `openfda_independent_pair_status.jsonl` | 202 | 507,497 | `2ff40c8720c24c68a77f028dd641715d2368b27b307e3652496ef0fcec71268c` |
| `candidate_openfda_independent_status.jsonl` | 363 | 510,549 | `075ba8265ad5160d8f5ae89c99e6c66cf61c8c4733cfdc3d2046e171ae3773a1` |
| `openfda_independent_bridge_rows.jsonl` | 363 | 567,487 | `ed16f2252d9190a1958036e65480a8ea16d316a9fe35787140cb979f7510f4f7` |
| `input_manifest.json` | - | 4,688 | `1ae1912e9854c8ff42b6b70640fa20d8af3be2720558bb583066170feac89815` |
| `validation_metrics.json` | - | 1,463 | `81502aaf32ee1b0ab5ca933d568e19c974dd3a05eef34e4ca895d9f142d8e337` |
| `output_manifest.json` | - | 2,503 | `941f8c80437f9c060c28495fafc181312d5ee8776282d1f9687c216aaf81390e` |
| `persisted_readback.json` | - | 3,662 | `5a88dcf76c2092345cd14ba83dd5a7f8547d8a121c3aa38650052362d0fcfcda` |
| `calyx_bridge_corpus_stdout.json` | - | 711 | `30ccc25b1b367371f48f946a92f6861d1181c4e5a35be71f5a877c7ddb13404c` |
| `calyx_bridge_corpus_readback.json` | - | 5,116 | `92ff7fb1f974e8d01c06511fb54264a2f479e2c592da3154e9905fdc0f27584c` |

## Metrics

| Metric | Count |
|---|---:|
| #1246 rollup rows checked | 363 |
| Unique pair keys queried | 202 |
| Queryable pair keys | 202 |
| openFDA query response rows | 202 |
| HTTP 404 no-result responses | 202 |
| openFDA label evidence rows | 0 |
| Pair statuses with independent openFDA no-hit | 202 |
| Rollup statuses with independent openFDA no-hit | 363 |

#1246 category counts carried through:

| #1246 category | Rollups |
|---|---:|
| `contraindication_or_avoidance_language_review` | 40 |
| `counter_negative_and_safety_language_review` | 82 |
| `counter_negative_language_review` | 23 |
| `fatality_or_mortality_language_review` | 125 |
| `safety_adverse_language_review` | 15 |
| `toxicity_language_review` | 78 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1246 persisted readback all true | true |
| #1246 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1246 rollup | true |
| All pair status values allowed | true |
| All rollup status values allowed | true |
| All hits have evidence | true |
| All status rows carry the clinical boundary | true |
| All rollup rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1248-openfda-independent-safety-validation-20260704t211500z
vault_id: 01KWQ9APFXM4Z4QWB7D626YGKT
vault_dir: /home/croyse/calyx/vaults/01KWQ9APFXM4Z4QWB7D626YGKT
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 363 |
| Bridge terms | 707 |
| Graph nodes | 1,070 |
| Graph edges | 3,630 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1248 produced an independent openFDA no-hit accounting layer for all 363 #1246
safety/counter rollups. No openFDA label section contained both terms for any
of the 202 queried pair keys under the targeted search contract.

This does not clear safety. It only records that this independent regulatory
label query found no same-section pair evidence. Every row remains blocked
pending broader source expansion, independent review, and human validation.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, recommendation, clinical actionability, or cure claim is made.

---

## 88_europepmc_endpoint_outcome_review.md

# #1247 Europe PMC Endpoint/Outcome Review

Status: complete.

This slice reviewed the #1244 Europe PMC rollups that had bounded source-text
relation context without safety/counter language. It separates endpoint/outcome,
pharmacokinetic/exposure, mechanistic, trial-design, and effect-size language.
Every row remains blocked pending independent endpoint validation,
safety/falsification, and human review.

Clinical boundary:

```text
Europe PMC endpoint/outcome review is literature triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/candidate_europepmc_relation_rollup.jsonl
sha256: 730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef

/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/europepmc_source_text_relation_validation.jsonl
sha256: ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2
```

Source contract:

- The input filter was `source_text_validation_status == source_text_relation_extracted_still_blocked`.
- Scoped #1244 rollups: 108.
- The stage did not fetch new source data; it read #1244 bounded source-text windows.
- Endpoint/outcome category labels are review flags only, not efficacy findings or clinical advice.

## Method

The reviewer:

- emitted one rollup review row for each scoped #1244 rollup;
- emitted evidence-review rows for linked #1244 source-text validation rows;
- separated clinical endpoint, preclinical/cell endpoint, endpoint/outcome,
  pharmacokinetic/exposure, mechanistic, and combination/coexposure context;
- preserved source ids, source-window text, source hashes, relation classes,
  model-system labels, effect-size strings, and dose/exposure strings;
- kept every row blocked pending independent endpoint/outcome validation,
  safety/falsification, and human review;
- wrote a 215-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_endpoint_outcome_evidence_review.jsonl` | 107 | 774,049 | `edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64` |
| `candidate_europepmc_endpoint_outcome_rollup.jsonl` | 108 | 298,820 | `d13703c71a7e1dedf595ee4b21fd0fbf5535d33c7487bebfd3717426e9aea90c` |
| `europepmc_endpoint_outcome_bridge_rows.jsonl` | 215 | 323,268 | `95ca04007dca7d23d5427cc66d7dd763b87243ab4e67716e6332a56167a5a9b8` |
| `input_manifest.json` | - | 2,077 | `f5a13a7f79997922a4cce5e3bf18c9844753c6832aca5606615f63619670163a` |
| `validation_metrics.json` | - | 20,834 | `4ee0551bf2b2d7ecbc5a6165eec008b73e099b3557e17f87963b24feefc89ad4` |
| `output_manifest.json` | - | 1,806 | `ad0f8c76b27fe20764c340719f1f09204c7b38180bd4946820866a87805e37f4` |
| `persisted_readback.json` | - | 2,958 | `1ffc6d8ca28310b039c8d2ed7eef7bda81ab166e5ab028a2a92fa38d766f2b3c` |
| `calyx_bridge_corpus_stdout.json` | - | 743 | `e8f9a330bee58df0201f7c0d37bc8484c5e1936199658f56ea148ec78c5a8ffa` |
| `calyx_bridge_corpus_readback.json` | - | 5,206 | `88c08f1188021eeebef24b5e3cf09035b87d6d66862d7d8bab79354ce8507e36` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped #1244 rollups | 108 |
| Evidence-review rows | 107 |
| Rollup-review rows | 108 |
| Rollups with endpoint/outcome language | 82 |
| Rollups with effect-size language | 32 |
| Rollups with pharmacokinetic/exposure language | 34 |
| Rollups with mechanistic language | 91 |
| Rollups with trial-design language | 74 |
| Rollups with preclinical language | 77 |
| Rollups with combination/coexposure language | 33 |

Source #1244 relation-class counts:

| Relation class | Rollups |
|---|---:|
| `combination_or_coexposure` | 5 |
| `mechanistic_or_interaction_context` | 91 |
| `trial_or_outcome_context` | 12 |

Evidence category counts:

| Category | Evidence rows |
|---|---:|
| `clinical_endpoint_language_review` | 48 |
| `endpoint_or_outcome_language_review` | 26 |
| `mechanistic_endpoint_context_review` | 18 |
| `pharmacokinetic_or_exposure_endpoint_review` | 6 |
| `preclinical_or_cell_endpoint_language_review` | 5 |
| `combination_or_coexposure_context_review` | 4 |

Rollup category counts:

| Category | Rollups |
|---|---:|
| `clinical_endpoint_language_review` | 59 |
| `endpoint_or_outcome_language_review` | 21 |
| `mechanistic_endpoint_context_review` | 19 |
| `pharmacokinetic_or_exposure_endpoint_review` | 4 |
| `combination_or_coexposure_context_review` | 3 |
| `preclinical_or_cell_endpoint_language_review` | 2 |

Evidence primary model-system counts:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 76 |
| `in_vitro_or_cell_system` | 18 |
| `unclear` | 8 |
| `animal_or_xenograft` | 3 |
| `computational_or_in_silico` | 2 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1244 persisted readback all true | true |
| #1244 Calyx readback all true | true |
| Rollup review for every scoped rollup | true |
| Reviewed pair ids match scope | true |
| All rollup reviews have evidence | true |
| Evidence reviews have source windows | true |
| Evidence reviews carry the clinical boundary | true |
| Rollup reviews carry the clinical boundary | true |
| Evidence reviews have allowed categories | true |
| Rollup reviews have allowed categories | true |
| Evidence reviews remain blocked | true |
| Rollup reviews remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1247-europepmc-endpoint-outcome-review-20260704t223000z
vault_id: 01KWQA7Z9A3TJXJK57J5FJCD8N
vault_dir: /home/croyse/calyx/vaults/01KWQA7Z9A3TJXJK57J5FJCD8N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 215 |
| Bridge terms | 451 |
| Graph nodes | 666 |
| Graph edges | 2,364 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1247 accounted for all 108 #1244 relation-context rollups and separated 107
linked evidence windows into endpoint/outcome review categories. The useful
output is a blocked triage layer: 82 rollups have endpoint/outcome language, 32
have effect-size language, 34 have pharmacokinetic/exposure language, and 91
have mechanistic language, but none are validated endpoint evidence or clinical
claims.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.

---

## 89_clinicaltrials_endpoint_validation.md

# #1250 ClinicalTrials.gov Endpoint Validation

Status: complete for the ClinicalTrials.gov registry pass.

This slice checked the #1247 Europe PMC endpoint/outcome review rollups against
the current ClinicalTrials.gov v2 API as an independent registry endpoint
source. A registry hit required both candidate terms in the same study
intervention text; an endpoint hit additionally required protocol or results
outcome fields in the matched study. No pair cleared that source gate.

Clinical boundary:

```text
ClinicalTrials.gov endpoint validation is registry documentation triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/candidate_europepmc_endpoint_outcome_rollup.jsonl
sha256: d13703c71a7e1dedf595ee4b21fd0fbf5535d33c7487bebfd3717426e9aea90c

/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/europepmc_endpoint_outcome_evidence_review.jsonl
sha256: edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64
```

Source contract:

- Input rollups: 108.
- Unique pair keys queried: 72.
- Source: current ClinicalTrials.gov v2 API.
- Query mode: `query.intr` with both candidate names.
- A source hit required both candidate terms in the same drug-intervention text.
- An endpoint hit required protocol or results outcome fields in that matched study.
- A no-hit is not negative efficacy evidence; it only means this targeted registry
  query did not find a same-intervention endpoint record.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/clinicaltrials_oas_v2.yaml` | 80,983 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `raw/clinicaltrials_api.html` | 94,295 | `da0916c3c6c7549118c989f03d8870f213428cd97e9fd0420d9aedc57677c616` |
| `raw/clinicaltrials_about_api.html` | 94,295 | `da0916c3c6c7549118c989f03d8870f213428cd97e9fd0420d9aedc57677c616` |

Source URLs:

- https://clinicaltrials.gov/api/oas/v2
- https://clinicaltrials.gov/data-api/api
- https://clinicaltrials.gov/data-api/about-api

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `clinicaltrials_endpoint_query_responses.jsonl` | 72 | 1,554,295 | `347f29d788af7c97cf4b259427492bea0d88747a2809ccaaeb6e1a9fe36d8822` |
| `clinicaltrials_endpoint_pair_status.jsonl` | 72 | 98,200 | `bfd61154e643adb9c852f5c0ee031db018e6986026ec400863784d677f1806c8` |
| `clinicaltrials_endpoint_rollup_status.jsonl` | 108 | 171,164 | `7cc7fd9b1ed55a41dcd02ccca37dd72647dae877f524c0c02d049d6771425b84` |
| `clinicaltrials_endpoint_study_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `clinicaltrials_endpoint_bridge_rows.jsonl` | 108 | 156,862 | `f117521406dc4280986b5ccbaac33c327662a4df3ceeb3278b2a9d479b2dbb3b` |
| `input_manifest.json` | - | 3,602 | `1ae93f1fbef2735401c344c42082f270a171272531efb7101b5e3609da5c3c18` |
| `validation_metrics.json` | - | 1,264 | `347770274a8530955fa15411669bd95a946477d5eb7bc1e6e8d81c2a15cbb36f` |
| `output_manifest.json` | - | 2,449 | `844615d7b830d76bf7c8468826a6a64b609c17e724194b28d8231450e324bfd7` |
| `persisted_readback.json` | - | 3,544 | `376633adc3bf8a5163c06f2a4d85bd5843b87bfe40789ddab36bbcd7a26739d0` |
| `calyx_bridge_corpus_stdout.json` | - | 706 | `4313a17bb9144a66b544df81dd5798bc7981147ca8f3e6d6a0a4d1a54df83e6e` |
| `calyx_bridge_corpus_readback.json` | - | 5,811 | `37e0f59f466acb5a1b246a97476db92b30faf2f0b383e5f42a19337d9fb016e4` |

## Metrics

| Metric | Count |
|---|---:|
| #1247 rollup rows checked | 108 |
| Unique pair keys | 72 |
| Queryable pair keys | 72 |
| ClinicalTrials.gov query response rows | 72 |
| Total API pages read | 72 |
| Responses with any returned study | 3 |
| Pair statuses with registry no-hit | 72 |
| Rollup statuses with registry no-hit | 108 |
| Study-evidence rows | 0 |
| Endpoint study-evidence rows | 0 |
| Result endpoint study-evidence rows | 0 |

#1247 category counts carried through:

| #1247 category | Rollups |
|---|---:|
| `clinical_endpoint_language_review` | 59 |
| `endpoint_or_outcome_language_review` | 21 |
| `mechanistic_endpoint_context_review` | 19 |
| `pharmacokinetic_or_exposure_endpoint_review` | 4 |
| `combination_or_coexposure_context_review` | 3 |
| `preclinical_or_cell_endpoint_language_review` | 2 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1247 persisted readback all true | true |
| #1247 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1247 rollup | true |
| Pair status values allowed | true |
| Rollup status values allowed | true |
| Endpoint hits have endpoint study evidence | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1250-clinicaltrials-endpoint-validation-20260704t230000z
vault_id: 01KWQAY8DYNXAJ5W90MVP2CBTE
vault_dir: /home/croyse/calyx/vaults/01KWQAY8DYNXAJ5W90MVP2CBTE
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 108 |
| Bridge terms | 152 |
| Graph nodes | 260 |
| Graph edges | 864 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1250 found no ClinicalTrials.gov same-intervention endpoint hits for the 72
unique #1247 pair keys. Three registry queries returned at least one study, but
none placed both candidate terms in the same drug-intervention text with
endpoint fields. All 108 rollups remain
`clinicaltrials_registry_no_hit_still_blocked`.

This does not falsify the source-text endpoint language from #1247; it only
records a targeted independent registry no-hit. No efficacy claim, safety
claim, treatment guidance, recommendation, clinical actionability, dosing
guidance, or cure claim is made.

---

## 90_europepmc_source_local_endpoint_expansion.md

# #1251 Europe PMC Source-Local Endpoint Expansion

Status: complete for the Europe PMC/PMC bounded-window pass.

This slice continued endpoint-source validation after the #1250
ClinicalTrials.gov no-hit by rereading the sealed #1247 Europe PMC/PMC source
windows. It required both candidate terms to appear in the same bounded source
window before emitting source-local endpoint context. All rows remain blocked
pending independent effect-result validation, safety/falsification, and human
review.

Clinical boundary:

```text
Europe PMC source-local endpoint expansion is source-text triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z/out/clinicaltrials_endpoint_rollup_status.jsonl
sha256: 7cc7fd9b1ed55a41dcd02ccca37dd72647dae877f524c0c02d049d6771425b84

/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z/out/clinicaltrials_endpoint_pair_status.jsonl
sha256: bfd61154e643adb9c852f5c0ee031db018e6986026ec400863784d677f1806c8

/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/europepmc_endpoint_outcome_evidence_review.jsonl
sha256: edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64
```

Source contract:

- Input #1250 rollups: 108.
- Input #1250 pair statuses: 72.
- Input #1247 evidence windows: 107.
- A source-local endpoint hit required both candidate terms in the same bounded
  `source_text_window` plus endpoint/outcome/PK-exposure context.
- Whole-article co-occurrence or one-term windows remained blocked.
- Direction, comparator, magnitude, and cohort/model strings are review fields
  only, not validated results.

## Method

The extractor:

- emitted one evidence-review row for every #1247 evidence window in the #1250
  pair-key scope;
- verified exact and normalized pair-term presence inside each bounded source
  window;
- separated source-local endpoint context, source-local pair context without
  endpoint language, and non-source-local windows;
- extracted endpoint type, effect-direction language, magnitude strings,
  comparator strings, cohort/model strings, mechanism, PK/exposure, dose, and
  trial-design strings;
- emitted one status row per #1250 rollup;
- kept every row blocked pending independent effect-result validation,
  safety/falsification, and human review;
- wrote a 215-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_source_local_endpoint_evidence_review.jsonl` | 107 | 662,973 | `c26882e64fc669d458f132684eab28df0ee15470a399c52982a9bf34b5b15154` |
| `europepmc_source_local_endpoint_rollup_status.jsonl` | 108 | 223,982 | `5af58d0b05c2fb03374ff0d74f3fceb300a8c40def32f455be8478a5d241f7c4` |
| `europepmc_source_local_endpoint_bridge_rows.jsonl` | 215 | 324,864 | `69bca1a9419fd6bce8a18ab7aae6d1aa221d4ce828725671ff912ed602679655` |
| `input_manifest.json` | - | 2,784 | `e614bdae68dacd263467c0e4d653c2b5093f61078293d2569d26242e2747fdef` |
| `validation_metrics.json` | - | 19,087 | `1fa89b31e86130f707bb20509990a7594fc36c1a036473843b616ea816874f34` |
| `output_manifest.json` | - | 1,879 | `626f50c4a6e6a79ac520602d83ccfcfdcda2e094471ff58c2d953ad03d04925c` |
| `persisted_readback.json` | - | 2,907 | `d57506da4b7e0d718d7f6866a1a156169e7be7cbddc8e870be5f001f693d4032` |
| `calyx_bridge_corpus_stdout.json` | - | 774 | `19c400979bd91af72283b112b8e3eb4014a2e810ec492cfab0ea778a4064410d` |
| `calyx_bridge_corpus_readback.json` | - | 5,337 | `c341675c0311714c69fa8383533cb6b56d772f43eaf6baae6a183f68bc760025` |

## Metrics

| Metric | Count |
|---|---:|
| #1250 rollups checked | 108 |
| #1250 pair statuses | 72 |
| Evidence-review rows | 107 |
| Rollup-status rows | 108 |
| Source-local pair-context evidence rows | 43 |
| Source-local endpoint-context evidence rows | 33 |
| Rollups with source-local pair context | 47 |
| Rollups with source-local endpoint context | 35 |
| Endpoint evidence rows with magnitude language | 9 |
| Endpoint evidence rows with comparator language | 12 |

Evidence status counts:

| Status | Evidence rows |
|---|---:|
| `europepmc_source_local_endpoint_context_hit_still_blocked` | 33 |
| `europepmc_source_local_pair_context_mechanistic_or_pk_only_still_blocked` | 10 |
| `europepmc_source_window_not_pair_local_still_blocked` | 64 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `europepmc_source_local_endpoint_context_hit_still_blocked` | 35 |
| `europepmc_source_local_pair_context_without_endpoint_still_blocked` | 12 |
| `europepmc_no_source_local_pair_endpoint_hit_still_blocked` | 61 |

Endpoint type counts:

| Endpoint type | Evidence rows |
|---|---:|
| `clinical_endpoint_language` | 23 |
| `endpoint_or_outcome_language` | 8 |
| `pharmacokinetic_or_exposure_language` | 1 |
| `preclinical_or_cell_endpoint_language` | 1 |

Effect-direction language counts:

| Direction label | Evidence rows |
|---|---:|
| `benefit_or_improvement_language` | 19 |
| `mixed_or_ambiguous_effect_language` | 2 |
| `no_explicit_effect_direction_language` | 9 |
| `worsening_progression_or_increase_language` | 3 |

Primary model-system counts for endpoint-context evidence:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 30 |
| `in_vitro_or_cell_system` | 2 |
| `unclear` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1250 persisted readback all true | true |
| #1250 Calyx readback all true | true |
| Rollup status for every #1250 rollup | true |
| Evidence rows cover pair-status keys | true |
| Endpoint rollup keys have endpoint evidence | true |
| Evidence statuses allowed | true |
| Rollup statuses allowed | true |
| Source-local endpoint evidence has both terms in bounded window | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1251-europepmc-source-local-endpoint-expansion-20260704t234000z
vault_id: 01KWQBKV63T2EPVWKJS8AJMVM0
vault_dir: /home/croyse/calyx/vaults/01KWQBKV63T2EPVWKJS8AJMVM0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 215 |
| Bridge terms | 342 |
| Graph nodes | 557 |
| Graph edges | 2,148 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1251 identified 35 #1250 rollups with bounded Europe PMC/PMC source-window
endpoint context where both candidate terms appear in the same source window.
Nine endpoint-context evidence rows carry magnitude strings and 12 carry
comparator language. These are source-text review fields only; none are
validated effect results or clinical claims.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.

---

## 91_effect_result_falsification_gate.md

# #1252 Effect-Result Falsification Gate

Status: complete for the source-local effect-result/falsification extraction
pass.

This slice continued the #1251 Europe PMC/PMC bounded-window endpoint pass by
extracting structured effect-result, magnitude, comparator, safety, and counter
language from the source-local endpoint rows. All rows remain blocked pending
independent effect-result validation, safety/falsification review, and human
review.

Clinical boundary:

```text
Effect-result and falsification extraction is source-text triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/europepmc_source_local_endpoint_rollup_status.jsonl
sha256: 5af58d0b05c2fb03374ff0d74f3fceb300a8c40def32f455be8478a5d241f7c4

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/europepmc_source_local_endpoint_evidence_review.jsonl
sha256: c26882e64fc669d458f132684eab28df0ee15470a399c52982a9bf34b5b15154

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/persisted_readback.json
sha256: d57506da4b7e0d718d7f6866a1a156169e7be7cbddc8e870be5f001f693d4032

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/calyx_bridge_corpus_readback.json
sha256: c341675c0311714c69fa8383533cb6b56d772f43eaf6baae6a183f68bc760025
```

Source contract:

- Input scope was the 35 #1251 rollups with
  `europepmc_source_local_endpoint_context_hit_still_blocked`.
- Evidence scope was the 33 #1251 source-local endpoint evidence rows.
- Every evidence row had to preserve pair-term verification from the bounded
  `source_text_window`.
- Result extraction is lexical triage only; direction, magnitude, comparator,
  safety, and counter strings are not validated endpoint results.
- Every row carries the clinical boundary and remains blocked.

## Method

The extractor:

- loaded the sealed #1251 rollup/evidence rows and upstream readbacks;
- retained only the #1251 source-local endpoint rows;
- classified evidence windows into result candidates with magnitude and
  direction, direction-only candidates, counter/safety-blocked candidates, or
  endpoint context without result assertion;
- emitted one rollup status row for every scoped rollup;
- preserved safety/counter language as blocking evidence;
- wrote a 68-row bridge-corpus slice for native Calyx materialization;
- verified all persisted rows and bridge rows by readback.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `effect_result_evidence_review.jsonl` | 33 | 164,687 | `faa3c1527ed9e79ecb53841e0a10cada8d0c2b1462c449cc2f2b0dbba453095c` |
| `effect_result_rollup_status.jsonl` | 35 | 67,008 | `43c11b2c653ff0772e6ef382ceafa6b1a84d5a10debba491586fc8107309a8b1` |
| `effect_result_bridge_rows.jsonl` | 68 | 93,327 | `ab4ec55c480e5fe65c61b5e2ae1202744b8b83909c567bca52d1f9b5425e42ce` |
| `input_manifest.json` | - | 2,206 | `d0417b8f8f9707c54e06eb8a139c3105fe73202090361460a04a585caa3d1910` |
| `validation_metrics.json` | - | 18,034 | `76fae92687a0abd10343ea87adcf772efe91a0fb6f248d22e8949fb77827fecd` |
| `output_manifest.json` | - | 1,721 | `6e4968b909fa78b4722a5fa8f47c926a7156dd6f78781acd4197c74d2edec2ad` |
| `persisted_readback.json` | - | 2,632 | `4a9e67714abb8fae8f26b021f5d4829cfd2a3aba4b76cd5777c184215116cf3e` |
| `calyx_bridge_corpus_stdout.json` | - | 724 | `0c4433c226ddaacf6896681395829912188d51db21e275c907e2b8456d12a201` |
| `calyx_bridge_corpus_readback.json` | - | 5,110 | `cd80e2ebcfc7819b6a1322a96a0560464bf14b614d04ef28a6ce23ca8a898756` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped source-local endpoint rollups | 35 |
| Result evidence rows | 33 |
| Rollup status rows | 35 |
| Evidence rows with direction and magnitude | 7 |
| Evidence rows with direction only | 11 |
| Evidence rows with comparator language | 12 |
| Evidence rows with safety/counter language | 1 |
| Rollups with direction and magnitude | 9 |
| Rollups with safety/counter block | 1 |

Evidence status counts:

| Status | Evidence rows |
|---|---:|
| `counter_or_safety_language_blocks_effect_result_candidate` | 1 |
| `effect_result_candidate_direction_only_still_blocked` | 11 |
| `effect_result_candidate_with_magnitude_and_direction_still_blocked` | 7 |
| `endpoint_context_without_result_assertion_still_blocked` | 14 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `counter_or_safety_language_blocks_rollup` | 1 |
| `effect_result_candidate_direction_only_still_blocked` | 15 |
| `effect_result_candidate_with_magnitude_and_direction_still_blocked` | 9 |
| `endpoint_context_without_result_assertion_still_blocked` | 10 |

Primary model-system counts for endpoint-context evidence:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 30 |
| `in_vitro_or_cell_system` | 2 |
| `unclear` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1251 persisted readback all true | true |
| #1251 Calyx readback all true | true |
| Rollup status for every scoped rollup | true |
| Result rows cover status pair keys | true |
| Evidence statuses allowed | true |
| Rollup statuses allowed | true |
| All result rows have pair-term verification | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1252-effect-result-falsification-gate-20260704t235000z
vault_id: 01KWQC80E88YJSQZ52MNXKQKPG
vault_dir: /home/croyse/calyx/vaults/01KWQC80E88YJSQZ52MNXKQKPG
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 68 |
| Bridge terms | 129 |
| Graph nodes | 197 |
| Graph edges | 676 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1252 produced a structured effect-result/falsification triage layer for the 35
#1251 source-local endpoint rollups. Nine rollups have direction plus magnitude
language and 15 have direction-only language, but every row remains blocked
because source-window text is not an independent endpoint-result, safety,
falsification, or human-review gate.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.

---

## 92_independent_effect_result_validation.md

# #1253 Independent Effect-Result Validation

Status: complete for the independent source-validation pass over #1252
effect-result candidates.

This slice loaded the #1252 direction/magnitude and direction-only candidate
rollups, queried independent source surfaces, and required source-local
pair-term evidence outside the #1251 source windows. No independent support row
survived the strict pair-term and source-exclusion gate, so all candidate
rollups remain blocked.

Clinical boundary:

```text
Independent effect-result validation is source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1253-independent-effect-result-validation-20260704T202500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/effect_result_rollup_status.jsonl
sha256: 43c11b2c653ff0772e6ef382ceafa6b1a84d5a10debba491586fc8107309a8b1

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/effect_result_evidence_review.jsonl
sha256: faa3c1527ed9e79ecb53841e0a10cada8d0c2b1462c449cc2f2b0dbba453095c

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/persisted_readback.json
sha256: 4a9e67714abb8fae8f26b021f5d4829cfd2a3aba4b76cd5777c184215116cf3e

/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/calyx_bridge_corpus_readback.json
sha256: cd80e2ebcfc7819b6a1322a96a0560464bf14b614d04ef28a6ce23ca8a898756
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `europepmc_rest_docs.html` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `clinicaltrials_oas_v2.yaml` | 80,983 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `ncbi_eutilities_docs.html` | 68,713 | `714a344a0e24d2c98344cee9509bd6770787315fc4c029d4071352157a8784af` |

Source contract:

- Input scope was the 24 #1252 rollups with
  `effect_result_candidate_with_magnitude_and_direction_still_blocked` or
  `effect_result_candidate_direction_only_still_blocked`.
- The 24 rollups represented 17 unique candidate pair keys.
- Each unique pair was queried against Europe PMC Articles REST,
  ClinicalTrials.gov v2, and PubMed E-utilities.
- An independent evidence row required both pair terms in the returned source
  text and a source id not overlapping the #1251 source ids.
- Result, magnitude, comparator, safety, and counter language remained triage
  fields only.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `independent_source_query_responses.jsonl` | 51 | 1,607,383 | `34a4269b07b22922dfa6a155fe78e282e862165d9365fe522b426c55b4745e7a` |
| `independent_effect_evidence_review.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `independent_effect_rollup_status.jsonl` | 24 | 34,142 | `c0b79bed2ec4c0ff765697d094f6156903a87586b138cba252b74eb7ae784b8f` |
| `independent_effect_bridge_rows.jsonl` | 75 | 91,238 | `f68026ad9b940064ec5bcba59cca111aaf248b6b0f372eb0889ff173ff7f484f` |
| `input_manifest.json` | - | 3,244 | `fc6cb185fd45355f0d05df40a9c85c2dd1959d8cf6817f936da34fe380e9a0be` |
| `validation_metrics.json` | - | 8,741 | `89b1731f6437dfdbc725a650a76177f3dbdc9fd7c15273efb188649ee37a65b3` |
| `output_manifest.json` | - | 2,078 | `dac470ce302eaab4721bbc40b15f0281a2836bb675a5b5122416fb25f20ee733` |
| `persisted_readback.json` | - | 3,195 | `3d41d08e409c93344a5d2bc7671e364487c03427ce8a01418bb03bca27a09608` |
| `calyx_bridge_corpus_stdout.json` | - | 729 | `cb32e995e150bdc6428851ec4da40f2f04a9c1a4a059f2571280cb3075052160` |
| `calyx_bridge_corpus_readback.json` | - | 5,483 | `85ee7d95707f4b0868d6ad1d014f2a03510f0f1531d712290f505bc170e85227` |

## Metrics

| Metric | Count |
|---|---:|
| Candidate rollups | 24 |
| Unique candidate pairs | 17 |
| Query response rows | 51 |
| Europe PMC queries | 17 |
| ClinicalTrials.gov queries | 17 |
| PubMed queries | 17 |
| Independent evidence rows | 0 |
| Rollup status rows | 24 |
| Rollups with independent result language | 0 |
| Rollups with independent safety/counter language | 0 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `no_independent_endpoint_result_source_hit_still_blocked` | 24 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1252 persisted readback all true | true |
| #1252 Calyx readback all true | true |
| Rollup status for every candidate rollup | true |
| Query response for every pair and source | true |
| Evidence status values allowed | true |
| Rollup status values allowed | true |
| Independent evidence source provenance present | true |
| Independent evidence pair terms present | true |
| Independent evidence excludes #1251 sources | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1253-independent-effect-result-validation-20260704t202500z
vault_id: 01KWQD30JCVKGAZ8JDGFTF0A2N
vault_dir: /home/croyse/calyx/vaults/01KWQD30JCVKGAZ8JDGFTF0A2N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 75 |
| Bridge terms | 53 |
| Graph nodes | 128 |
| Graph edges | 702 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1253 did not validate any #1252 candidate as an independent effect-result
source. The strict independent gate found zero source-local pair evidence rows
outside the #1251 source windows across Europe PMC, ClinicalTrials.gov, and
PubMed for the 17 unique candidate pairs. All 24 candidate rollups remain
blocked with `no_independent_endpoint_result_source_hit_still_blocked`.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.

---

## 93_openfda_faers_safety_expansion.md

# #1249 openFDA FAERS Safety-Source Expansion

Status: complete for the openFDA FAERS event co-report expansion pass.

This slice continued #1248 after the openFDA Human Drug Label no-hit result by
querying a distinct source instrument: openFDA FAERS drug event reports. A
FAERS co-report row is adverse-event source triage only. It is not safety
clearance, pair-interaction proof, treatment guidance, recommendation, dosing
guidance, clinical actionability, efficacy, or cure evidence.

Clinical boundary:

```text
openFDA FAERS event expansion is adverse-event source triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, efficacy, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1249-openfda-faers-safety-expansion-20260704T203500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/candidate_openfda_independent_status.jsonl
sha256: 075ba8265ad5160d8f5ae89c99e6c66cf61c8c4733cfdc3d2046e171ae3773a1

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/openfda_independent_pair_status.jsonl
sha256: 2ff40c8720c24c68a77f028dd641715d2368b27b307e3652496ef0fcec71268c

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/persisted_readback.json
sha256: 5a88dcf76c2092345cd14ba83dd5a7f8547d8a121c3aa38650052362d0fcfcda

/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/calyx_bridge_corpus_readback.json
sha256: 92ff7fb1f974e8d01c06511fb54264a2f479e2c592da3154e9905fdc0f27584c
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `openfda_drug_event_api.html` | 128,623 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_drug_event_fields.html` | 127,160 | `a318fe3288d61ecc00d5ac891de26622bf3e0966f876c6ebed36bad918920203` |
| `openfda_download_docs.html` | 122,916 | `3d7111b6d8449d47518bb1ec1651e7368bd9fe6c369342099d3368998d467d0a` |

Source contract:

- Input scope was the 363 #1248 rollups with
  `independent_openfda_no_hit_still_blocked`.
- The 363 rollups represented 202 unique pair keys.
- Each pair was queried against the openFDA Drug Event endpoint using both
  component names in `patient.drug.medicinalproduct`.
- A FAERS evidence row required both component terms to be physically present
  in returned event drug fields.
- Returned FAERS event rows remain blocking triage, not safety clearance.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `faers_query_responses.jsonl` | 202 | 18,380,119 | `703ee94ca110e23bd58941387c31d3334615d508e7e7baf288b90c694956cc2b` |
| `faers_event_evidence.jsonl` | 45 | 14,936,518 | `c7ff1704f9491b27e6f8edd4f5b432e300236a41261d4999293145d3252df58b` |
| `faers_pair_status.jsonl` | 202 | 230,919 | `b4f75d65e844c062ea97e54fb4b709ac3b87d2a348f7ad398d98e690e0535c16` |
| `faers_rollup_status.jsonl` | 363 | 559,887 | `78656d19f4451d2a0edb7ee4db071b3b32518b664d4f12fb4fc28aef212276f3` |
| `faers_bridge_rows.jsonl` | 812 | 1,005,020 | `74595085d6767956fc00d054f2da1135230446bec29df3ed8472ca946d47812c` |
| `input_manifest.json` | - | 3,177 | `6301198353d780362bdef9cc1e766d64de676bf51351f2c8407eeb80331d54cd` |
| `validation_metrics.json` | - | 1,052 | `c5a5df0cc00b8bb932f0f933af2e20bd3dbeabaee751a47329a8d79bfb117da7` |
| `output_manifest.json` | - | 2,280 | `2971fa3f8f8cf3acd5ee5acf5b32b46bc2904c127786cc56884f5a2ba6d86128` |
| `persisted_readback.json` | - | 3,455 | `5590cf7e67a14d01264c2fca161d5fc237f6cd1212b064df52643516d910b7e7` |
| `calyx_bridge_corpus_stdout.json` | - | 746 | `f2733919f9bf2362c407b32249a28428774d48197438ff26ab80666b2750ad42` |
| `calyx_bridge_corpus_readback.json` | - | 5,708 | `ebc35251cd48365decc34688ef9bb34f810dfa885386d05f69c129d7e4e19334` |

## Metrics

| Metric | Count |
|---|---:|
| #1248 rollups checked | 363 |
| Unique pair keys queried | 202 |
| FAERS query response rows | 202 |
| FAERS event evidence rows | 45 |
| Pair status rows | 202 |
| Rollup status rows | 363 |
| Rollups with FAERS event co-report | 82 |
| Rollups with serious FAERS event co-report | 79 |

Query HTTP status counts:

| HTTP status | Pair queries |
|---|---:|
| `200` | 45 |
| `404` | 157 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `faers_pair_coreport_event_hit_still_blocked` | 2 |
| `faers_pair_coreport_serious_event_hit_still_blocked` | 43 |
| `faers_pair_query_no_result_still_blocked` | 157 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `faers_rollup_coreport_event_hit_still_blocked` | 3 |
| `faers_rollup_coreport_serious_event_hit_still_blocked` | 79 |
| `faers_rollup_query_no_result_still_blocked` | 281 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1248 persisted readback all true | true |
| #1248 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1248 rollup | true |
| All FAERS hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms in event drug fields | true |
| Pair/rollup status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1249-openfda-faers-safety-expansion-20260704t203500z
vault_id: 01KWQE3VV0ZZ3ACPVHXM8T5547
vault_dir: /home/croyse/calyx/vaults/01KWQE3VV0ZZ3ACPVHXM8T5547
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 812 |
| Bridge terms | 547 |
| Graph nodes | 1,359 |
| Graph edges | 6,800 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1249 expanded independent safety-source coverage beyond openFDA Human Drug
Labels by adding FAERS event co-report triage. It found 45 source-backed FAERS
event evidence rows covering 82 of the 363 #1248 rollups; 79 rollups were tied
to serious-event co-report rows. Those rows are blockers/review inputs only,
not safety clearance or actionability.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, clinical recommendation, clinical actionability, efficacy claim, or
cure claim is made.

---

## 94_pubchem_synonym_source_mining.md

# #1245 PubChem Synonym Source Mining

Status: complete for the PubChem synonym/equivalence source-mining pass.

This slice continued #1243 after the Europe PMC pair-search no-hit remainder by
querying a distinct source instrument: PubChem PUG-REST compound synonym
records. A PubChem hit required returned structured synonym text to physically
contain both candidate terms or accepted normalized equivalents. CID existence,
query success, and synonym count alone were not treated as evidence.

Clinical boundary:

```text
PubChem synonym/equivalence source mining is chemical identity/source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction evidence, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/candidate_europepmc_status.jsonl
sha256: 7a3b63dea0ff880374b68627e9465d1d11d1e7758e1bcf6cec59050611746c03

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_status.jsonl
sha256: 153b7f222495f25244d6e221b5cc46ca1830335a12759867328b9a26b2384f9d

/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/persisted_readback.json
sha256: 37e6ca92f1a3bfc5f0945f64c538bc38e96911a1281951fe92faf3f13828717a
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `pubchem_pug_rest_docs.html` | 5,134 | `b2de3f40fd32fe75d4adb7e9217745c1451c7229f83f55a08324f85598280dda` |
| `pubchem_pug_view_docs.html` | 5,384 | `073b169762c48b4e89866159cee0be3feae713634bfa52ba54b5fdcce3428a8b` |
| `pubchem_programmatic_access.html` | 5,211 | `f02b9d1c0b9d80df4142349d3059cb00111475c5e3f485952f9e6af2d1c0c175` |

Source contract:

- Input scope was the 532 #1243 candidate rows with
  `overall_external_source_status_after_issue1243 == no_external_hit`.
- Those candidates represented 353 unique pair keys and 152 unique query terms.
- Each term was queried against the PubChem synonym endpoint.
- A pair evidence row required PubChem returned synonym text to contain both
  terms, not merely a CID match for one component.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pubchem_synonym_query_responses.jsonl` | 152 | 467,160 | `4eb6bc219d81a9282d856cfd45fcd7e4bfefe8705538b279b1896c1b8ad6a347` |
| `pubchem_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `pubchem_pair_status.jsonl` | 353 | 417,290 | `60f313a4eb57de2845f2f8a6d9f55a203655c1f53eb593dc6d907ec15aa4002e` |
| `candidate_pubchem_status.jsonl` | 532 | 740,827 | `299efd7e94b82ee99bf8112cc4f879d642b6e507f6d3f0be160edc674a17b179` |
| `pubchem_bridge_rows.jsonl` | 885 | 1,235,844 | `7b4dd92cc7ec43fb4436e182386b62242139a5684cb8159403d14302ca8897ba` |
| `input_manifest.json` | - | 3,197 | `69523c62285f1408d8541bb5bc155931115082d74a41f3b9e9836c3581f74823` |
| `validation_metrics.json` | - | 898 | `f833cbfce8f26be743344eb9dc557e74e6df088d62ee4fdda14166595a72638f` |
| `output_manifest.json` | - | 2,308 | `05076f2c461c31e5726122020804a01d1a7e8ae0c5635d392ffac93ee5d168d4` |
| `persisted_readback.json` | - | 3,511 | `037a33ca5b1828e03b948d891763d5abf5726b5c931016b52e6746749587f9dc` |
| `calyx_bridge_corpus_stdout.json` | - | 710 | `a559e09bf0f4d3509d44a793f040b33656bc69143c3d1e491300627bf06dd4cf` |
| `calyx_bridge_corpus_readback.json` | - | 5,678 | `43598ce544f856b2d81385b8376c134d074c1eaa37cbca44d061986577e5102a` |

## Metrics

| Metric | Count |
|---|---:|
| #1243 no-hit candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| Unique PubChem term queries | 152 |
| PubChem terms with CID/synonym records | 103 |
| PubChem pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Query HTTP status counts:

| HTTP status | Term queries |
|---|---:|
| `200` | 103 |
| `404` | 49 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `pubchem_synonym_record_without_pair_match_still_blocked` | 298 |
| `pubchem_synonym_no_result_still_blocked` | 55 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `pubchem_synonym_record_without_pair_match_still_blocked` | 475 |
| `pubchem_synonym_no_result_still_blocked` | 57 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1243 persisted readback all true | true |
| #1243 Calyx readback all true | true |
| Query response for every unique term | true |
| Pair status for every pair key | true |
| Candidate status for every #1243 no-hit candidate | true |
| All PubChem hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1245-pubchem-synonym-source-mining-20260704t210000z
vault_id: 01KWQF43ADQWB6WCY5DMRR25F2
vault_dir: /home/croyse/calyx/vaults/01KWQF43ADQWB6WCY5DMRR25F2
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 885 |
| Bridge terms | 507 |
| Graph nodes | 1,392 |
| Graph edges | 7,080 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1245 expanded external source coverage for the #1243 no-hit remainder by
checking PubChem synonym/equivalence records. PubChem returned 103 term records,
but no pair satisfied the physical two-term synonym/equivalence gate, so there
were zero PubChem pair evidence rows. The 532 carried candidate rows remain
blocked and now have explicit PubChem no-hit/no-pair-match status rows.

No efficacy, safety, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.

---

## 95_chembl_source_mining.md

# #1254 ChEMBL Source Mining

Status: complete for the ChEMBL molecule-search source-mining pass.

This slice continued #1245 after the PubChem synonym/equivalence no-hit result
by querying a distinct source instrument: ChEMBL REST molecule search records.
A ChEMBL hit required a returned structured molecule record to physically
contain both candidate terms or accepted normalized equivalents. Search count,
single-term matches, and returned molecule records were not treated as evidence
unless both terms appeared in the same returned record.

Clinical boundary:

```text
ChEMBL source mining is molecule/source triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction evidence, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/candidate_pubchem_status.jsonl
sha256: 299efd7e94b82ee99bf8112cc4f879d642b6e507f6d3f0be160edc674a17b179

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/pubchem_pair_status.jsonl
sha256: 60f313a4eb57de2845f2f8a6d9f55a203655c1f53eb593dc6d907ec15aa4002e

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/persisted_readback.json
sha256: 037a33ca5b1828e03b948d891763d5abf5726b5c931016b52e6746749587f9dc

/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/calyx_bridge_corpus_readback.json
sha256: 43598ce544f856b2d81385b8376c134d074c1eaa37cbca44d061986577e5102a
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `chembl_rest_docs.html` | 2,520 | `17427b9bb56f08d217a76e1a8cd3ea07b89f5ec1ae096194222bc625c63e718b` |
| `chembl_molecule_schema_sample.json` | 3,815 | `f9eed1e4504b917dbc6f54de8ce5ae2094e16ea5773a88c36a38f1df2562a163` |

Source contract:

- Input scope was the 532 #1245 candidate rows still blocked after PubChem.
- Those candidates represented 353 unique pair keys.
- Each pair key was queried against ChEMBL molecule search using both component
  names in the query.
- A pair evidence row required a returned ChEMBL molecule record to contain
  both pair terms or accepted normalized equivalents in the same record.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `chembl_molecule_query_responses.jsonl` | 353 | 402,552 | `14cd4c625b6a0a5a8859b9ad06aa31fa626ebc84825a83e525b8f169b8d46260` |
| `chembl_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `chembl_pair_status.jsonl` | 353 | 628,835 | `4ea75759362b16db5a3470605d6321d93f2301fa4954b60bd02dc81c5bd2861f` |
| `candidate_chembl_status.jsonl` | 532 | 760,286 | `761ef7b2c8f644fe20b2cb6c2eee1f88f06d8116c0ccfe86a37cbb5b13156689` |
| `chembl_bridge_rows.jsonl` | 885 | 1,183,419 | `893d8c2c9c6691dd46a440d3a6c55c76c25da30c73ddbd30c2cc064f7a79ffea` |
| `input_manifest.json` | - | 2,838 | `13db9f1aa88813a937ed5aa63a305f5f9b041672582e7fe358c3ce31009af3a0` |
| `validation_metrics.json` | - | 962 | `632db838f80603468fdb7284f7470974920298bf5476cfcf0b9cdc2a0cb4bcbd` |
| `output_manifest.json` | - | 2,207 | `6edd4d83adfbebf756de9bf3097944e70f257b1a1de24f0565823d465b8e45a1` |
| `persisted_readback.json` | - | 3,384 | `bffda29de180a185d13f04fc23aafe9ce2837af12db7b26e91d41f01327edfa9` |
| `calyx_bridge_corpus_stdout.json` | - | 673 | `39e4f1fbcb55b8018a31e1b3bfebbb95f4b27bed3000ba90f004991393bb5dfd` |
| `calyx_bridge_corpus_readback.json` | - | 5,505 | `d50c44cab39c975e71d85abe66ee63eeb666b0fb963df0aa91745541b22b747f` |

## Metrics

| Metric | Count |
|---|---:|
| #1245 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| ChEMBL molecule-search pair queries | 353 |
| ChEMBL queries with returned records | 339 |
| ChEMBL total records returned | 5,377 |
| ChEMBL pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Query HTTP status counts:

| HTTP status | Pair queries |
|---|---:|
| `200` | 353 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `chembl_pair_record_without_pair_match_still_blocked` | 339 |
| `chembl_pair_no_result_still_blocked` | 14 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `chembl_candidate_record_without_pair_match_still_blocked` | 487 |
| `chembl_candidate_no_result_still_blocked` | 45 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1245 persisted readback all true | true |
| #1245 Calyx readback all true | true |
| Query response for every pair key | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All ChEMBL hits have evidence rows | true |
| Evidence rows have source hashes | true |
| Evidence rows have pair terms | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1254-chembl-source-mining-20260704t213500z
vault_id: 01KWQHTFE770MJKC5T8AMCT780
vault_dir: /home/croyse/calyx/vaults/01KWQHTFE770MJKC5T8AMCT780
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 885 |
| Bridge terms | 509 |
| Graph nodes | 1,394 |
| Graph edges | 7,080 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1254 expanded external source coverage for the #1245 blocked remainder by
checking ChEMBL molecule-search records. ChEMBL returned 5,377 molecule records
across 339 pair queries, but no returned molecule record satisfied the physical
same-record two-term gate, so there were zero ChEMBL pair evidence rows. The
532 carried candidate rows remain blocked and now have explicit ChEMBL
no-result/no-pair-match status rows.

No efficacy, safety, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.

---

## 96_drugcentral_source_mining.md

# #1255 DrugCentral Source Mining

Status: complete for the DrugCentral source-mining pass.

This slice continued #1254 after the ChEMBL no-hit result by snapshotting
DrugCentral source tables and checking structured drug-drug interaction rows
plus same-structure equivalence mappings. A hit required a DrugCentral DDI row
whose two participants normalized to the pair terms, or both pair terms mapping
to the same DrugCentral structure id. Search count, single-term mappings, and
source-table presence alone were not treated as evidence.

Clinical boundary:

```text
DrugCentral source mining is drug/source triage only; interaction rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/candidate_chembl_status.jsonl
sha256: 761ef7b2c8f644fe20b2cb6c2eee1f88f06d8116c0ccfe86a37cbb5b13156689

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/chembl_pair_status.jsonl
sha256: 4ea75759362b16db5a3470605d6321d93f2301fa4954b60bd02dc81c5bd2861f

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/persisted_readback.json
sha256: bffda29de180a185d13f04fc23aafe9ce2837af12db7b26e91d41f01327edfa9

/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/calyx_bridge_corpus_readback.json
sha256: d50c44cab39c975e71d85abe66ee63eeb666b0fb963df0aa91745541b22b747f
```

Persisted source/API documentation:

| Source doc | Bytes | SHA-256 |
|---|---:|---|
| `drugcentral_download.html` | 14,175 | `a7ba9349a89f5986ed487c02688df47596c30f1f3721a199d852e7b59d76553a` |
| `drugcentral_active_download.html` | 9,095 | `193ac07195c08d0b52e0ff8a453892cb0597e447503ff4033dc6d40bae850f47` |
| `drugcentral_api_docs.html` | 943 | `4e18b81a68eee4e54babf520451ad6421ae4ba3cda16d50a5dcdcbbb31e3b80b` |
| `drugcentral_openapi.json` | 74,598 | `be98c09d44ca9279e47aa8d2ea56c0becbda8edbab3e3eab1add7c3125812e0f` |
| `drugcentral_schema.csv` | 3,321 | `8e8647fd00dfd4ac962086dce6b5d9ca36761c11242bfab03626eb7dcaae9ee1` |
| `drugcentral_counts.csv` | 148 | `6e73cf5f13c5420c43395dd9245e11249ca36aaab5e78ecf22d6c6dee4aeac1b` |

Runtime note: DrugCentral database access was supplied through the run
environment; credentials were not persisted in repo artifacts.

Source table snapshots:

| Table | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `ddi` | 7,621 | 775,132 | `9f9abc65ffbfca813962c1c6fa270b38f701f3c57d80838d5f5c049a9719ac9f` |
| `ddi_risk` | 6 | 146 | `7791f63f73a6160458ef01c2da61aecb013224a713e05a86f6fbb43b9598c7a4` |
| `structures` | 4,995 | 540,863 | `d0b59bbce25f3693563e6d4baf7b7cb38f2dbb77a0b65a0e76c7ee7bd33a293f` |
| `synonyms` | 23,369 | 978,738 | `a6db6cb7ccbabe4ec9eaeeded2c2015612d24e192b91d57d6007b3c8317252d6` |
| `identifier` | 82,230 | 2,610,309 | `658c8699871056ea7f8d770056f48d7481996caa01964dfa0a40e4d94d53db39` |
| `approval` | 3,915 | 150,580 | `f84fec5f2c321d5c321b3aee6f39935649673911bc35120332dbdde9b4f61c65` |
| `omop_relationship` | 42,307 | 4,211,849 | `9fe3ddb5cbdcf91998610cd19ee193c2e536f6b7e456f083df6963ea33e99271` |
| `act_table_full` | 20,978 | 4,531,558 | `4760de223d667361886927af6b3d46aa786b2b061503bd9acece239193d422bb` |

Source contract:

- Input scope was the 532 #1254 candidate rows still blocked after ChEMBL.
- Those candidates represented 353 unique pair keys.
- Structured DDI evidence required both pair terms to match the two DDI
  participants by exact normalized text or DrugCentral structure-id mapping.
- Same-structure evidence required both pair terms to resolve to the same
  DrugCentral structure id through structures, synonyms, or identifiers.
- All rows remained blocked behind the clinical boundary.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `drugcentral_source_rows.jsonl` | 10 | 7,472 | `cb3730a4d53cbd18704f89821e801f5f8c19a2e3a6d0bdb05b336458912036d8` |
| `drugcentral_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `drugcentral_pair_status.jsonl` | 353 | 529,705 | `07caa8201d7a9367864a52af4bae57afd9822e3e52aa69721e89f250cfe2ed81` |
| `candidate_drugcentral_status.jsonl` | 532 | 883,529 | `9dce3e78548d959b56b08f3a74f8e69ec936673f0d5b78924c387315c6eca099` |
| `drugcentral_bridge_rows.jsonl` | 895 | 1,285,878 | `eadfceb72720f9c0b67eebab52585ebc83b2fd63ad9d27be0f1d4008215f31e5` |
| `input_manifest.json` | - | 6,396 | `7bec2b5a9216fffe4608ba9ce30589202af5b89d91b9d52693ddb85a0521e4cb` |
| `validation_metrics.json` | - | 1,248 | `43c6588b5f75daf381ead3be51cf5374fe4da176b1976a05001f4f034881d34a` |
| `output_manifest.json` | - | 2,317 | `92e58d075a79e166cc3bfd05f24305640eadbc09949a3504beebd8e3b4353124` |
| `persisted_readback.json` | - | 3,552 | `661ef64c2daf25e5eb7483eeeb4754aa93fc2f83e223a7b8d114c14ed5fa9478` |
| `calyx_bridge_corpus_stdout.json` | - | 731 | `f2d034f296964cc44ffef482a8f78eb42cdc217e4791825af26d53cd480f2c6e` |
| `calyx_bridge_corpus_readback.json` | - | 5,696 | `c2c421392bb43469bbf7dc8f9be106c794b8c6697a39ceaf01c781ad225203bf` |

## Metrics

| Metric | Count |
|---|---:|
| #1254 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| DrugCentral DDI source rows checked | 7,621 |
| DrugCentral structure rows checked | 4,995 |
| DrugCentral synonym rows checked | 23,369 |
| DrugCentral identifier rows checked | 82,230 |
| DrugCentral pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `drugcentral_single_term_mappings_without_pair_match_still_blocked` | 189 |
| `drugcentral_no_term_mapping_still_blocked` | 164 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `drugcentral_candidate_single_term_mappings_without_pair_match_still_blocked` | 287 |
| `drugcentral_candidate_no_term_mapping_still_blocked` | 245 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1254 persisted readback all true | true |
| #1254 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All DrugCentral hits have evidence rows | true |
| DDI evidence rows have participant matches | true |
| Same-structure evidence rows have shared structure | true |
| Evidence rows have source hashes | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1255-drugcentral-source-mining-20260704t222500z
vault_id: 01KWQJX8XZJF7C1GF91699YM3Y
vault_dir: /home/croyse/calyx/vaults/01KWQJX8XZJF7C1GF91699YM3Y
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 895 |
| Bridge terms | 540 |
| Graph nodes | 1,435 |
| Graph edges | 7,160 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1255 expanded external source coverage for the #1254 blocked remainder by
checking DrugCentral structures, synonyms, identifiers, approvals, indications,
activity rows, and 7,621 structured DDI rows. No candidate pair satisfied the
structured DDI participant gate or same-structure equivalence gate, so there
were zero DrugCentral pair evidence rows. The 532 carried candidate rows remain
blocked and now have explicit DrugCentral no-term-mapping or single-term-only
status rows.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.

---

## 97_pharmgkb_source_mining.md

# #1256 PharmGKB Source Mining

Status: complete for the PharmGKB/ClinPGx source-mining pass.

This slice continued #1255 after the DrugCentral no-hit result by snapshotting
PharmGKB/ClinPGx source documentation and downloadable TSV archives, then
checking pharmacogenomic annotations, labels, variant annotations, chemical
aliases, drug aliases, and entity relationships for same-row two-term support.
A hit required one source row to contain both pair terms or mapped PharmGKB ids.
Single-term mappings, source-table presence, annotation existence, and label
existence were not treated as pair evidence.

Clinical boundary:

```text
PharmGKB source mining is pharmacogenomic/source triage only; clinical annotation, label, variant, or relationship rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/candidate_drugcentral_status.jsonl
sha256: 9dce3e78548d959b56b08f3a74f8e69ec936673f0d5b78924c387315c6eca099

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/drugcentral_pair_status.jsonl
sha256: 07caa8201d7a9367864a52af4bae57afd9822e3e52aa69721e89f250cfe2ed81

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/persisted_readback.json
sha256: 661ef64c2daf25e5eb7483eeeb4754aa93fc2f83e223a7b8d114c14ed5fa9478

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/calyx_bridge_corpus_readback.json
sha256: c2c421392bb43469bbf7dc8f9be106c794b8c6697a39ceaf01c781ad225203bf
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `clinpgx_api_root.html` | `https://api.pharmgkb.org/` | 4,172 | `92993534b52e3d12d02c2b6e66e580749fb862a2ed777ae95915f5f45de0ce64` |
| `clinpgx_downloads.html` | `https://www.clinpgx.org/downloads` | 2,505 | `ff5fe31c3191e57b278d53ede2f4a551102b8a5c5f017162e0f44987740aaddd` |
| `kg_registry_pharmgkb.html` | `https://kghub.org/kg-registry/resource/pharmgkb/pharmgkb.html` | 292,411 | `80cd262108973ee7533112935e9d524b906c75d8f30b1f574d4ff6a16266c408` |

Persisted PharmGKB archives:

| Archive | Bytes | SHA-256 |
|---|---:|---|
| `chemicals.zip` | 810,667 | `17ddecfbbf7be9ea44ecebb17632514ece6e0877f48d9100390e2c61b8007b80` |
| `clinicalAnnotations.zip` | 1,231,768 | `9c6512c54f3c9321effacb11178fb2ae1c45fa3f1710f08c3c365ee7537ced07` |
| `clinicalVariants.zip` | 74,345 | `68b6592a9039e0a6f0bbc0e15feffb691e66f008036aabb306ed734e4a0b7bda` |
| `drugLabels.zip` | 58,722 | `1be944921a8bbca9c1f273717322f5a8ebd5acb00dcc04cecda653592ffabd5b` |
| `drugs.zip` | 677,109 | `54939a6b4526845238d8e2139a1b59ad3382e64932dff21a1fd27c023f43b072` |
| `relationships.zip` | 2,375,103 | `1d4672930e8ef4c420ef840ca330517580886d8a8bfc56b0eef5a02a918c62be` |
| `variantAnnotations.zip` | 4,240,496 | `bfc1df607f95bfc08dd8e5af1ee78a231a08c0bf6061437ed53cf821c2f538ad` |

Source table snapshots:

| Table | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `chemicals.tsv` | 5,283 | 2,682,260 | `0518a95c92b8f9a5aa0d406492fa9e0324b2704a04c67d7f80ae2fd902f9cd22` |
| `clinicalVariants.tsv` | 5,190 | 362,165 | `5c98338a4c23c52590537776c9630650db7cf844c39b94ed5bc23300ea187414` |
| `clinical_ann_evidence.tsv` | 15,431 | 4,013,649 | `5fc2157933e78a02d0604b899e7db997e6f9d9f924c23f31dace59489803d118` |
| `clinical_annotations.tsv` | 5,186 | 852,045 | `f548f25c7b5930ca250e7640f5a3531cfcc6e89048b916a7f115ae390fcc66f4` |
| `drugLabels.tsv` | 1,401 | 206,852 | `9de09e002a31befd3bd342dc6af7660fc307aad318c5ccb6371a25693038746e` |
| `drugLabels.byGene.tsv` | 237 | 154,060 | `c90037d1468ceb9c7e47080f6e0464dc80cdc049010683a0b94d536a3c38cea4` |
| `drugs.tsv` | 3,756 | 2,222,607 | `6e7a8740fbad58347fabc0f4f5036221b0856f07fc805348f0019ffd5c2baba0` |
| `relationships.tsv` | 127,768 | 15,461,292 | `4bba8db8b80e2acdbfeddf5eba922052b0c7e5da968a1cf1cc4c9879598f74a1` |
| `var_drug_ann.tsv` | 12,963 | 7,093,584 | `742c08ac6d201e65e1aeed47193503bd7a48a4dcb4b32f683649b5a47cee6613` |
| `var_fa_ann.tsv` | 2,149 | 1,100,855 | `a674673b770239d46012eddc91a88c60f4f05cb4e91539fccb0c68c9fb06b8ed` |
| `var_pheno_ann.tsv` | 14,468 | 8,747,853 | `1345f02e1a12bf937a57da724a34332c6fbf200ac3b5eeb24bc8254aff2da43e` |

Source contract:

- Input scope was the 532 #1255 candidate rows still blocked after
  DrugCentral.
- Those candidates represented 353 unique pair keys.
- Alias mapping came from PharmGKB `drugs.tsv` and `chemicals.tsv` names,
  generic names, trade names, brand mixtures, cross-references, RxNorm ids,
  PubChem ids, and ATC ids.
- Same-row source evidence required both pair terms or their mapped PharmGKB ids
  to appear in one scanned TSV row.
- All rows remained blocked behind the clinical boundary.

Runtime notes:

- The live `https://api.pharmgkb.org/` root, ClinPGx downloads page, KG-Registry
  page, and concrete versioned data downloads were persisted.
- The stale `/v1/swagger-ui/index.html` API documentation path returned 404 at
  preflight time and was not included in the FSV contract.
- The TSV parser raised Python's CSV field-size limit because PharmGKB source
  rows exceed the default parser cap.
- The row scan was optimized to build normalized pair-term and PharmGKB-id
  membership once per source row, then produce detailed match objects only for
  row/pair candidates that satisfied both sides of the gate.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pharmgkb_source_rows.jsonl` | 21 | 17,580 | `e5ea879e444c5172edcf4a9b8d16368835aabdaba8a703e76b2958954f609626` |
| `pharmgkb_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `pharmgkb_pair_status.jsonl` | 353 | 510,805 | `58dc7d1c502f385649d41ff3bc469b0db0a373c12d991f2a9f744cecfdfe0d04` |
| `candidate_pharmgkb_status.jsonl` | 532 | 854,883 | `cc2b60f9d692b4dc74cf8c9e0742ae8d3e2f7cf7f707ca91d45e07bbc374ed0c` |
| `pharmgkb_bridge_rows.jsonl` | 906 | 1,299,927 | `da3d922c645989f5b9b639f945c4b9fdc652d1652e5e41ac87de9e2b65d79e99` |
| `input_manifest.json` | - | 9,114 | `e753058c8a3de94d8025532327fb89e9a5fa796b44ee45e985443cedcaa047b8` |
| `validation_metrics.json` | - | 1,305 | `19bf531047eae9a0e2e6210af4dcdcf363bc35cf8ef621085ad3fa59aff7d88f` |
| `output_manifest.json` | - | 2,316 | `8e5957fc9820728a996909d4aeb773fea82a2e320f5d8fb40099b11d35d43ed1` |
| `persisted_readback.json` | - | 3,474 | `6bb5b355ded3f1adf05b10e3186b1d1186612abe6d3556d1294b18850ad25cdf` |
| `calyx_bridge_corpus_stdout.json` | - | 713 | `8aabfe2bc0d577b8dc3ca7eea8e5db338e5a1ca1b146f817e9365e13cfccb805` |
| `calyx_bridge_corpus_readback.json` | - | 5,812 | `f70dae3dca15954d556a9d2ed1ecc7b2f54b0c77c01048fe9e586e7a630dd67d` |

## Metrics

| Metric | Count |
|---|---:|
| #1255 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| PharmGKB/ClinPGx source inventory rows | 21 |
| PharmGKB TSV tables scanned | 9 |
| Alias TSV tables scanned | 2 |
| PharmGKB pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Scanned TSV row counts:

| Table | Rows |
|---|---:|
| `clinical_annotations` | 5,186 |
| `clinical_ann_evidence` | 15,431 |
| `clinicalVariants` | 5,190 |
| `drugLabels` | 1,401 |
| `drugLabels_byGene` | 237 |
| `relationships` | 127,768 |
| `var_drug_ann` | 12,963 |
| `var_pheno_ann` | 14,468 |
| `var_fa_ann` | 2,149 |

Alias TSV row counts:

| Table | Rows |
|---|---:|
| `drugs` | 3,756 |
| `chemicals` | 5,283 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `pharmgkb_single_term_mappings_without_pair_match_still_blocked` | 161 |
| `pharmgkb_no_term_mapping_still_blocked` | 192 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `pharmgkb_candidate_single_term_mappings_without_pair_match_still_blocked` | 258 |
| `pharmgkb_candidate_no_term_mapping_still_blocked` | 274 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1255 persisted readback all true | true |
| #1255 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All PharmGKB hits have evidence rows | true |
| Evidence rows have both matches | true |
| Evidence rows have source hashes | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1256-pharmgkb-source-mining-20260704t230500z
vault_id: 01KWQMCNSKQFEM30T5CKV72CNV
vault_dir: /home/croyse/calyx/vaults/01KWQMCNSKQFEM30T5CKV72CNV
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 906 |
| Bridge terms | 550 |
| Graph nodes | 1,456 |
| Graph edges | 7,248 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1256 expanded external source coverage for the #1255 blocked remainder by
checking PharmGKB/ClinPGx drug, chemical, relationship, label, clinical
annotation, clinical variant, variant-drug annotation, variant-phenotype
annotation, and variant-functional annotation sources. No candidate pair
satisfied the same-row two-term or mapped-id source gate, so there were zero
PharmGKB pair evidence rows. The 532 carried candidate rows remain blocked and
now have explicit PharmGKB no-term-mapping or single-term-only status rows.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.

---

## 98_nsides_source_mining.md

# #1257 nSIDES TwoSIDES/OffSIDES Source Mining

Status: complete for the nSIDES TwoSIDES/OffSIDES source-mining pass.

This slice continued #1256 after the PharmGKB no-hit result by snapshotting
nSIDES/Tatonetti documentation and the current TwoSIDES and OffSIDES flat-file
archives. TwoSIDES was treated as the only pair-level adverse-effect source in
this pass. OffSIDES was treated as single-drug adverse-effect context only, not
pair proof.

Clinical boundary:

```text
nSIDES TwoSIDES/OffSIDES source mining is adverse-effect/source triage only; pair adverse-effect rows and single-drug adverse-effect rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/candidate_pharmgkb_status.jsonl
sha256: cc2b60f9d692b4dc74cf8c9e0742ae8d3e2f7cf7f707ca91d45e07bbc374ed0c

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/pharmgkb_pair_status.jsonl
sha256: 58dc7d1c502f385649d41ff3bc469b0db0a373c12d991f2a9f744cecfdfe0d04

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/persisted_readback.json
sha256: 6bb5b355ded3f1adf05b10e3186b1d1186612abe6d3556d1294b18850ad25cdf

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/calyx_bridge_corpus_readback.json
sha256: f70dae3dca15954d556a9d2ed1ecc7b2f54b0c77c01048fe9e586e7a630dd67d
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `nsides_home.html` | `https://nsides.io/` | 23,263 | `ecef6a0b1360726bc1a877715166388c8a849f1a6fea71194a84cb793db7e2a4` |
| `tatonetti_stm.html` | `https://tatonettilab.org/resources/tatonetti-stm.html` | 3,115 | `1cf7afa8da126f311fe1e24c289a87f5cbe6cfa1a962a838e928615cc679af52` |

Persisted archives:

| Archive | Bytes | SHA-256 |
|---|---:|---|
| `twosides.csv.gz` | 738,463,578 | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `offsides.csv.gz` | 68,762,346 | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Parsed source tables:

| Table | Rows | Header SHA-256 | Archive SHA-256 |
|---|---:|---|---|
| `TWOSIDES` | 42,920,391 | `d4aaaba48caff9cc697d80d858f9b43f48c11ef7ef0095de80f801c693eb47d2` | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `OFFSIDES` | 3,206,558 | `8d1c9e60327fc016bfd81ce74c5f1619290c469f638ef1a43450f9b1d290cb9c` | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Source contract:

- Input scope was the 532 #1256 candidate rows still blocked after PharmGKB.
- Those candidates represented 353 unique pair keys.
- TwoSIDES pair evidence required one TwoSIDES row where both pair drugs matched
  the row's two drug concept names by normalized exact or salt-stripped name
  key, in either order.
- OffSIDES context required one OffSIDES row matching one pair-side drug name by
  the same normalized/salt-stripped key. These rows were marked context only and
  never pair proof.
- All rows remained blocked behind the clinical boundary.

Runtime note:

- The first full run exposed an avoidable repeated source-hash read during
  OffSIDES matches. The miner was patched to cache source archive hashes once
  per scan and to reuse already-downloaded archives before the successful run.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `nsides_source_rows.jsonl` | 6 | 6,273 | `a75100db75fad772a7cfc48c2cc4d04dd8a0606a692c9be9595e82c73ab08ae5` |
| `twosides_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `offsides_single_drug_context.jsonl` | 853 | 1,556,599 | `4d5a6991457b0088b04e3aa4cf22f3eac780fe3c627d7d19cf35f8aa765f12dc` |
| `nsides_pair_status.jsonl` | 353 | 549,534 | `16626c6da02115bb1cb6a9a71d1f094bdba9ca799c59b69bad7bc62e8715ca2a` |
| `candidate_nsides_status.jsonl` | 532 | 908,364 | `70631c484ca28d8a20d08550ca0d15fabaa2eebfa3ab766ef2b43d4d8ccc4282` |
| `nsides_bridge_rows.jsonl` | 1,000 | 1,368,122 | `5c9b651d424ffec1e89e3857b4b226febbed1bbb93486590257ac3fc9547dfa1` |
| `input_manifest.json` | - | 5,194 | `f1300abbd4908118cbb345b4acb7045c4c9ed203ca68ac806459c82cdf2e09ee` |
| `validation_metrics.json` | - | 1,232 | `5467438ae31ebe6810a10f1b21d6c1e9e2023c6deccb30d7580ffdc722745d09` |
| `output_manifest.json` | - | 2,601 | `644e1095034650d89686ccf58164ea7e9d6e2d2776be2bff268e063164a7bc5d` |
| `persisted_readback.json` | - | 3,453 | `b0cbb727d64e4e3bab246d64e16d565bdbffa7e7a99a06dcbad0a8f163964fe4` |
| `calyx_bridge_corpus_stdout.json` | - | 731 | `bbf8c4d001aa88a0ac065d489b730543aae97a07db15257dcea8a78c757762b6` |
| `calyx_bridge_corpus_readback.json` | - | 5,872 | `f92138819ab44501f9a1a00e9fc22c940551af31716562a04659ec54be231fb6` |

## Metrics

| Metric | Count |
|---|---:|
| #1256 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| TwoSIDES source rows parsed | 42,920,391 |
| OffSIDES source rows parsed | 3,206,558 |
| TwoSIDES pair evidence rows | 0 |
| OffSIDES single-drug context sample rows | 853 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `nsides_offsides_single_drug_context_without_pair_hit_still_blocked` | 157 |
| `nsides_no_source_name_mapping_still_blocked` | 196 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `nsides_candidate_offsides_single_drug_context_without_pair_hit_still_blocked` | 241 |
| `nsides_candidate_no_source_name_mapping_still_blocked` | 291 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1256 persisted readback all true | true |
| #1256 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All TwoSIDES hits have evidence | true |
| Evidence rows have both matches | true |
| Evidence rows have source hashes | true |
| OffSIDES context rows are single-drug only | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1257-nsides-source-mining-20260704t235500z
vault_id: 01KWQPR6GMSZ5DD4FFSTDCV1WM
vault_dir: /home/croyse/calyx/vaults/01KWQPR6GMSZ5DD4FFSTDCV1WM
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 606 |
| Graph nodes | 1,606 |
| Graph edges | 7,782 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1257 expanded external source coverage for the #1256 blocked remainder by
checking 42,920,391 TwoSIDES drug-drug-effect rows and 3,206,558 OffSIDES
single-drug adverse-effect rows. No candidate pair satisfied the TwoSIDES
pair-level source gate, so there were zero TwoSIDES pair evidence rows. 157
pair keys had OffSIDES single-drug adverse-effect context for at least one
pair-side drug, and those rows are retained only as safety blocker/review
context.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.

---

## 99_rxnorm_canonicalization.md

# #1258 RxNorm/RxNav Canonicalization

Status: complete for the RxNorm canonicalization pass over the #1257 nSIDES
no-map remainder.

This slice used the official RxNav/RxNorm APIs to canonicalize the 152 unique
terms remaining after #1257 name matching, persisted every response body, and
rescanned the #1257 TwoSIDES/OffSIDES source archives by trusted RxCUIs. Exact
and normalized RxCUIs, plus ingredient relations derived from trusted RxCUIs,
were allowed for source matching. Approximate RxNav matches were retained only
as provisional manual-review inputs and were not trusted for pair matching.

Clinical boundary:

```text
RxNorm/RxNav canonicalization is identity/source-mapping support only; mapped RxCUIs, ingredients, approximate matches, TwoSIDES RxCUI rows, and OffSIDES RxCUI rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z
```

Sealed upstream #1257 inputs:

```text
/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/candidate_nsides_status.jsonl
sha256: 70631c484ca28d8a20d08550ca0d15fabaa2eebfa3ab766ef2b43d4d8ccc4282

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/nsides_pair_status.jsonl
sha256: 16626c6da02115bb1cb6a9a71d1f094bdba9ca799c59b69bad7bc62e8715ca2a

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/offsides_single_drug_context.jsonl
sha256: 4d5a6991457b0088b04e3aa4cf22f3eac780fe3c627d7d19cf35f8aa765f12dc

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/persisted_readback.json
sha256: b0cbb727d64e4e3bab246d64e16d565bdbffa7e7a99a06dcbad0a8f163964fe4

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/calyx_bridge_corpus_readback.json
sha256: f92138819ab44501f9a1a00e9fc22c940551af31716562a04659ec54be231fb6

/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/output_manifest.json
sha256: 644e1095034650d89686ccf58164ea7e9d6e2d2776be2bff268e063164a7bc5d
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `rxnorm_api_overview.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html` | 21,042 | `d037f07cac2e2f18225cff27f792c2d0133d945d67b175108d0866d4acfd49d8` |
| `find_rxcui_by_string.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.findRxcuiByString.html` | 26,687 | `169ad799d3153b19e5d56597b7a8d2850b10a69f7fa3a3edd9e4d0562e9bbbae` |
| `approximate_match.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getApproximateMatch.html` | 24,462 | `d58f956f8f11a13029151c41f4538362ef25be3047ef7f7b1b6e2f7511cc1888` |
| `related_by_type.html` | `https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getRelatedByType.html` | 24,658 | `9b0f92ac5831eb2549fb96e475254b6382a3532e0c87cf8ae8e1fc81edce37cf` |

Persisted source archives reused from #1257:

| Archive | Rows parsed | Bytes | SHA-256 |
|---|---:|---:|---|
| `twosides.csv.gz` | 42,920,391 | 738,463,578 | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `offsides.csv.gz` | 3,206,558 | 68,762,346 | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Source contract:

- Input scope was the 532 candidate rows and 353 unique pair keys still blocked
  after #1257.
- Each unique term was queried with RxNav `findRxcuiByString` using exact or
  normalized search, `getApproximateMatch`, and `getRelatedByType` for trusted
  exact/normalized RxCUIs.
- Exact/normalized RxCUIs and their related ingredient RxCUIs were trusted
  identity mappings for source rescans.
- Approximate matches remained provisional and did not create trusted pair
  evidence.
- TwoSIDES evidence required one TwoSIDES row containing trusted RxCUIs for
  both pair sides, in either drug position.
- OffSIDES evidence remained single-drug adverse-effect context only.
- Every output row remains blocked behind external identity, safety, outcome,
  falsification, and human-review gates.

Runtime note:

- The first materialization attempt exposed a real bridge-row text/term contract
  mismatch: row text did not include the `raw_docs` bridge term. The bridge row
  builder was patched to include all source and pair bridge terms in row text,
  then the full FSV root was rerun and materialized successfully.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `rxnorm_source_rows.jsonl` | 7 | 7,287 | `65c11cddb30eec8f5d294358c37aa788428989a8ed58b3adcd46527f334cc2e2` |
| `rxnav_api_response_rows.jsonl` | 376 | 210,097 | `bf546c59ea7ed07d7eeccc55ef0571633745615997327ccb03fff064be92e309` |
| `rxnorm_term_status.jsonl` | 152 | 400,296 | `8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7` |
| `rxnorm_twosides_pair_evidence.jsonl` | 757 | 1,372,373 | `7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17` |
| `rxnorm_offsides_single_drug_context.jsonl` | 1,088 | 1,796,596 | `cbb8980f79f05010ab4c515a5b1a9908a302cccc4d867d9f84d9e60c1c7bdc30` |
| `rxnorm_pair_status.jsonl` | 353 | 717,170 | `20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0` |
| `candidate_rxnorm_status.jsonl` | 532 | 1,039,327 | `cb9ca1d1a1dd8af195e4dd9830e1072c4c1bfe7e2197ef77f9ec26f2baeddba2` |
| `rxnorm_bridge_rows.jsonl` | 1,000 | 1,392,869 | `087257f34fe3fdf850fa569d31aa592a8c08d376a69827bda935dfcdc2bf10a9` |
| `input_manifest.json` | - | 6,411 | `2110ff5270725451f4518f4e7c25b4fa3e7798f7a72dc1109a78dcc3ad9e3e74` |
| `validation_metrics.json` | - | 1,860 | `b6b896a1f25851d821b4e9617691f49a3e206747d7187bc715d8fc30750d7de0` |
| `output_manifest.json` | - | 3,260 | `1c90ac1e495357da85a74a5c405423d59c914482aaae7036e3c6543216a0c48c` |
| `persisted_readback.json` | - | 4,357 | `0011f528f7af04e18154e83dc193c822a1171e9d753d3c67f5e4d53add04b55d` |
| `calyx_bridge_corpus_stdout.json` | - | 763 | `5c7cd657bff0ff88d3fe80d1c6d9dd9e032ecb404aac6a5adfa37ee4f3b8dc05` |
| `calyx_bridge_corpus_stderr.txt` | - | 336 | `4026c3ec984e71df637a49ae4d5fa9480ef973d30e1933c8397a118bdb3e048c` |
| `calyx_bridge_corpus_readback.json` | - | 3,834 | `521f420f3b8b5f821064c3a341be76dcdd5ef18cc11da8f3235b5ff893913a50` |

## Metrics

| Metric | Count |
|---|---:|
| #1257 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| Unique terms queried through RxNav | 152 |
| RxNav API response rows persisted | 376 |
| RxNav HTTP 200 responses | 376 |
| Trusted term mappings | 70 |
| Approximate-only provisional term mappings | 32 |
| No term mapping | 50 |
| TwoSIDES source rows parsed | 42,920,391 |
| OffSIDES source rows parsed | 3,206,558 |
| TwoSIDES RxCUI pair adverse-effect rows | 757 |
| Unique pair keys with TwoSIDES RxCUI rows | 7 |
| OffSIDES RxCUI single-drug context sample rows | 1,088 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Term status counts:

| Status | Term rows |
|---|---:|
| `rxnorm_term_trusted_mapping_still_blocked` | 70 |
| `rxnorm_term_approximate_only_provisional_still_blocked` | 32 |
| `rxnorm_term_no_mapping_still_blocked` | 50 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `rxnorm_twosides_rxcui_pair_hit_still_blocked` | 7 |
| `rxnorm_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked` | 29 |
| `rxnorm_approximate_only_provisional_still_blocked` | 163 |
| `rxnorm_unmapped_still_blocked` | 154 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `rxnorm_candidate_twosides_rxcui_pair_hit_still_blocked` | 16 |
| `rxnorm_candidate_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked` | 45 |
| `rxnorm_candidate_approximate_only_provisional_still_blocked` | 233 |
| `rxnorm_candidate_unmapped_still_blocked` | 238 |

TwoSIDES RxCUI pair evidence summary:

| Pair key | Rows | Max PRR | Example conditions |
|---|---:|---:|---|
| `etanercept szzs||sitagliptin` | 212 | 30.0 | Abdominal discomfort; Abdominal distension; Abdominal pain; Abdominal pain upper; Alopecia |
| `saxagliptin anhydrous||sitagliptin hydrochloride monohydrate` | 151 | 60.0 | Abdominal discomfort; Abdominal distension; Abdominal pain; Abdominal pain upper; Anaemia |
| `sitagliptin hydrochloride monohydrate||valacyclovir` | 124 | 40.0 | Abdominal discomfort; Abdominal pain; Abdominal pain upper; Abnormal dreams; Alanine aminotransferase increased |
| `infliximab dyyb||sitagliptin` | 123 | 40.0 | Abdominal pain; Anaemia; Anxiety; Arthralgia; Arthropathy |
| `etanercept szzs||ribavirin monophosphate` | 68 | 40.0 | Alanine aminotransferase increased; Anaemia; Arthralgia; Asthenia; Back pain |
| `etanercept||ribavirin monophosphate` | 68 | 40.0 | Alanine aminotransferase increased; Anaemia; Arthralgia; Asthenia; Back pain |
| `metformin||trametinib dimethyl sulfoxide` | 11 | 40.0 | Anaemia; Blood creatinine increased; Chills; Death; Dehydration |

Interpretation:

- The 757 rows are TwoSIDES adverse-effect source rows keyed by trusted RxCUIs.
- These rows are useful as safety/falsification blockers and review triage.
- They do not establish beneficial interaction, efficacy, safety clearance,
  causality, dosing, recommendation, clinical actionability, pair-interaction
  proof, or cure evidence.

Validation assertions:

| Assertion | Result |
|---|---|
| #1257 persisted readback all true | true |
| #1257 Calyx readback all true | true |
| 152 unique terms queried | true |
| Every RxNav response persisted | true |
| Every RxNav response had HTTP 200 status | true |
| Trusted pair matching used exact/normalized/related RxCUIs only | true |
| Approximate-only matches stayed provisional | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| TwoSIDES evidence rows have both pair RxCUI matches | true |
| Evidence rows carry source row hashes | true |
| OffSIDES context rows are single-drug only | true |
| Status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1258-rxnorm-canonicalization-20260705t000500z
vault_id: 01KWQR9X3RSFTATP35PGFPY62A
vault_dir: /home/croyse/calyx/vaults/01KWQR9X3RSFTATP35PGFPY62A
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 906 |
| Graph nodes | 1,906 |
| Graph edges | 10,278 |
| CSR persisted | true |
| Materializer index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault `CURRENT` and `MANIFEST` files present | true |
| Vault `cf/graph` files present | true |
| Vault `cf/graph` file count | 12,187 |
| Vault `cf/graph` total bytes | 11,443,976 |

## Result

#1258 converted a name-normalization no-map remainder into a RxNorm-grounded
identity map. It rescued 70 trusted term mappings and surfaced 757 TwoSIDES
RxCUI pair adverse-effect rows across 7 pair keys, plus 1,088 OffSIDES
single-drug adverse-effect context rows. The strongest operational value is
that those 7 pair keys now have concrete safety/falsification rows to validate
independently rather than remaining name-matching misses.

Every candidate remains blocked. No efficacy, safety clearance, treatment
guidance, dosing guidance, clinical recommendation, clinical actionability,
pair-interaction proof, or cure claim is made.

---

## 100_rxnorm_twosides_safety_validation.md

# #1259 RxNorm-Rescued TwoSIDES Safety Validation

Status: complete for the independent safety/falsification validation pass over
the seven #1258 RxCUI-rescued TwoSIDES pair-hit keys.

This slice read the sealed #1258 RxNorm canonicalization artifacts, verified
their hashes, grouped original pair keys by trusted RxCUI overlap, queried
independent safety/literature/label sources, and materialized the result into
native Calyx. The result is a blocker/review artifact only.

Clinical boundary:

```text
Independent validation of RxNorm-rescued TwoSIDES pair hits is safety, source, outcome, and falsification triage only; adverse-event rows, label co-mentions, literature co-mentions, and registry/source hits are blockers or review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z
```

Sealed #1258 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `rxnorm_twosides_pair_evidence.jsonl` | 757 | `7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17` |
| `rxnorm_pair_status.jsonl` | 353 | `20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0` |
| `candidate_rxnorm_status.jsonl` | 532 | `cb9ca1d1a1dd8af195e4dd9830e1072c4c1bfe7e2197ef77f9ec26f2baeddba2` |
| `rxnorm_term_status.jsonl` | 152 | `8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7` |
| `persisted_readback.json` | - | `0011f528f7af04e18154e83dc193c822a1171e9d753d3c67f5e4d53add04b55d` |
| `calyx_bridge_corpus_readback.json` | - | `521f420f3b8b5f821064c3a341be76dcdd5ef18cc11da8f3235b5ff893913a50` |
| `output_manifest.json` | - | `1c90ac1e495357da85a74a5c405423d59c914482aaae7036e3c6543216a0c48c` |

Independent source documentation snapshots:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `openfda_event_docs` | `https://open.fda.gov/apis/drug/event/` | 128,623 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_event_fields` | `https://open.fda.gov/apis/drug/event/searchable-fields/` | 127,160 | `a318fe3288d61ecc00d5ac891de26622bf3e0966f876c6ebed36bad918920203` |
| `openfda_label_docs` | `https://open.fda.gov/apis/drug/label/` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `openfda_query_syntax` | `https://open.fda.gov/apis/query-syntax/` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `dailymed_web_services` | `https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm` | 85,397 | `3fbe63342062c085fcbb85f1431f6c1614bbacc6b2ade5adc21de8eba3c4327e` |
| `dailymed_spls_api` | `https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm` | 91,702 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `europepmc_rest_docs` | `https://europepmc.org/RestfulWebService` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `ncbi_eutilities_intro` | `https://www.ncbi.nlm.nih.gov/books/NBK25497/` | 68,713 | `ffd2e3b9ae0e5a472cd0179faf8ab4810f6c17707c828cbe22121595699606fa` |
| `ncbi_eutilities_params` | `https://www.ncbi.nlm.nih.gov/books/NBK25499/` | 114,258 | `feeb9656287baeaf06a1a16e4b938c31c9a894f27dbb58311b018a14b2cd9756` |

Source contract:

- Input scope was exactly the 7 #1258 pair keys with
  `rxnorm_twosides_rxcui_pair_hit_still_blocked`.
- Expected #1258 hashes were verified before processing.
- Trusted RxCUI overlap on both pair sides formed canonical duplicate groups,
  while original pair keys and strict identity keys were retained.
- Queried independent sources: openFDA FAERS, openFDA drug labels, DailyMed SPL
  metadata, Europe PMC, and PubMed E-utilities.
- A query hit did not become evidence unless returned source fields contained
  both pair terms under the deterministic presence gate.
- All evidence remains blocked behind safety, outcome, falsification, and human
  review gates.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 9 | 11,489 | `831719fa41863ab691eb49aea9c639bf313043b78b82d45bc409995b39ee5f96` |
| `pair_scope.jsonl` | 7 | 35,421 | `c2958b87c75e45ac14164d25498b5dc01529a498a6b353c30a29ace77cae6aac` |
| `independent_query_rows.jsonl` | 35 | 188,922 | `736cfba3178cc6ae557bef94f95294a898365b1267f07ea3390e793cb00f914c` |
| `independent_evidence_rows.jsonl` | 1 | 2,526 | `6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca` |
| `pair_validation_rollups.jsonl` | 7 | 33,602 | `186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c` |
| `candidate_validation_status.jsonl` | 16 | 22,476 | `b6a3eeaa15add12f23b57ae2956c6dfddfadb78ab45788e7a4f4c1e0de843700` |
| `issue1259_bridge_rows.jsonl` | 68 | 96,303 | `e3b7552159c041239cff5fff4cbea35195901be42bfa6ba27a7aecd4c7d339e4` |
| `validation_metrics.json` | - | 1,687 | `93622d3c0de17a2760e528b220c76cfde1531cbbe0532200d458ca57166eb16b` |
| `output_manifest.json` | - | 7,058 | `2f48d9d64a8a973f684816f72cb2ddcc15fc20284e2d272c2a4ced3f10112751` |
| `persisted_readback.json` | - | 3,935 | `2a421bfd797355c8b4c777240b88a7c1d02c376005a53d50e86e85025dde92ad` |
| `calyx_bridge_corpus_stdout.json` | - | 793 | `8fd1b2e5e054536cf7370c91cd7cb80755f29ae111ae49373e319793909848ac` |
| `calyx_bridge_corpus_stderr.txt` | - | 324 | `bb3e375644b6b6d3b3ecee83f186063af42ca7b90880eefc7c048bd1a2fc83ff` |
| `calyx_bridge_corpus_readback.json` | - | 3,248 | `10462d52057fa323443d1e2ae8c0fe752c464b2424a1e726eb3dc1b6a1680ce1` |

## Metrics

| Metric | Count |
|---|---:|
| Pair scope rows | 7 |
| Candidate status rows touched | 16 |
| TwoSIDES RxCUI adverse-effect rows preserved | 757 |
| Canonical strict identity groups | 7 |
| Trusted-RxCUI overlap duplicate groups | 1 |
| Independent query rows | 35 |
| Independent evidence rows | 1 |
| Pair rollups | 7 |
| Bridge rows | 68 |

Query rows by source:

| Source | Query rows |
|---|---:|
| openFDA FAERS | 7 |
| openFDA label | 7 |
| DailyMed SPL metadata | 7 |
| Europe PMC | 7 |
| PubMed ESearch | 7 |

HTTP status counts:

| Status | Query rows |
|---|---:|
| 200 | 15 |
| 404 | 13 |

The 404 rows are persisted no-result responses from openFDA. A transient PubMed
429 was observed in an earlier run; fetch retry handling was patched and the
final run had no 429 rows.

Pair rollups:

| Pair key | Status | Independent evidence | TwoSIDES rows | Max PRR |
|---|---|---:|---:|---:|
| `etanercept szzs||ribavirin monophosphate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 68 | 40.0 |
| `etanercept szzs||sitagliptin` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 212 | 30.0 |
| `etanercept||ribavirin monophosphate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 68 | 40.0 |
| `infliximab dyyb||sitagliptin` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 123 | 40.0 |
| `metformin||trametinib dimethyl sulfoxide` | `independent_faers_safety_signal_still_blocked` | 1 | 11 | 40.0 |
| `saxagliptin anhydrous||sitagliptin hydrochloride monohydrate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 151 | 60.0 |
| `sitagliptin hydrochloride monohydrate||valacyclovir` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 124 | 40.0 |

Trusted-RxCUI overlap duplicate group:

```text
etanercept szzs||ribavirin monophosphate
etanercept||ribavirin monophosphate
```

Independent evidence row:

| Pair key | Source | Classification | Source ID | Reactions | Serious |
|---|---|---|---|---|---|
| `metformin||trametinib dimethyl sulfoxide` | openFDA FAERS | `safety_signal` / `faers_coreport_serious` | `24608768` | Off label use; Lower gastrointestinal haemorrhage | true |

Europe PMC returned hit counts for two pairs but no returned metadata record
passed the both-term evidence gate:

| Pair key | Hit count | Returned | Evidence rows |
|---|---:|---:|---:|
| `infliximab dyyb||sitagliptin` | 1 | 1 | 0 |
| `metformin||trametinib dimethyl sulfoxide` | 24 | 5 | 0 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `candidate_independent_faers_safety_signal_still_blocked` | 4 |
| `candidate_twosides_only_no_independent_confirmation_still_blocked` | 12 |

Validation assertions:

| Assertion | Result |
|---|---|
| Expected #1258 input hashes matched | true |
| Seven pair-scope rows | true |
| All pair-scope rows queryable | true |
| 757 TwoSIDES rows preserved | true |
| Pair rollup for every scoped pair | true |
| Query rows for every pair | true |
| Query rows have response hashes | true |
| Evidence rows have sources | true |
| Evidence rows blocked | true |
| Pair rollups blocked | true |
| Candidate statuses blocked | true |
| Clinical boundary present | true |
| Bridge rows <= 1,000 | true |
| Bridge terms present in text | true |

## Native Calyx Materialization

```text
name: issue1259-rxnorm-twosides-safety-validation-20260705t013000z-final
vault_id: 01KWQSYVD5WWDGTC0QFDR9651N
vault_dir: /home/croyse/calyx/vaults/01KWQSYVD5WWDGTC0QFDR9651N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 68 |
| Bridge terms | 56 |
| Graph nodes | 124 |
| Graph edges | 616 |
| CSR persisted | true |
| Materializer index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault `CURRENT` and `MANIFEST` files present | true |
| Vault `cf/graph` files present | true |
| Vault `cf/graph` file count | 743 |
| Vault `cf/graph` total bytes | 718,864 |

## Result

#1259 did not produce any treatment, efficacy, safety-clearance, dosing,
recommendation, clinical-actionability, pair-interaction-proof, or cure claim.
It did produce one independent safety blocker: a serious openFDA FAERS co-report
for `metformin||trametinib dimethyl sulfoxide` with reactions `Off label use`
and `Lower gastrointestinal haemorrhage`.

The remaining six pair rollups have TwoSIDES RxCUI adverse-effect source rows
but no independent verified evidence in this pass. All seven pair rollups and
all sixteen candidate statuses remain blocked.

---

## 101_metformin_trametinib_faers_case_validation.md

# #1260 Metformin-Trametinib FAERS Case Validation

Status: complete for the case-level validation of the #1259 serious FAERS
co-report blocker for `metformin||trametinib dimethyl sulfoxide`.

This slice read the sealed #1259 artifacts, re-fetched the exact openFDA FAERS
case by `safetyreportid:24608768`, queried identity/label/literature context
sources, and materialized the result into native Calyx. The result is a
blocked safety-review artifact only.

Clinical boundary:

```text
FAERS case-level validation is safety/source/falsification triage only; case reports, label text, RxNorm identity rows, and literature rows are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1260-metformin-trametinib-faers-case-20260705T020000Z
```

Sealed #1259 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `independent_evidence_rows.jsonl` | 1 | `6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca` |
| `pair_validation_rollups.jsonl` | 7 | `186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c` |
| `candidate_validation_status.jsonl` | 16 | `b6a3eeaa15add12f23b57ae2956c6dfddfadb78ab45788e7a4f4c1e0de843700` |
| `calyx_bridge_corpus_readback.json` | - | `10462d52057fa323443d1e2ae8c0fe752c464b2424a1e726eb3dc1b6a1680ce1` |

Persisted source documentation snapshots:

| Source doc | HTTP | SHA-256 |
|---|---:|---|
| `openfda_event_docs` | 200 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_label_docs` | 200 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `dailymed_spls_api` | 200 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `europepmc_rest_docs` | 200 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `ncbi_eutilities_intro` | 200 | `e0c8f8d9563afb18adaaa7a0bed1f9ca58b33ef39f9dc4775e9d1ebda9cf88b4` |
| `rxnorm_find_rxcui` | 200 | `169ad799d3153b19e5d56597b7a8d2850b10a69f7fa3a3edd9e4d0562e9bbbae` |
| `rxnorm_related` | 200 | `9b0f92ac5831eb2549fb96e475254b6382a3532e0c87cf8ae8e1fc81edce37cf` |

Source contract:

- Scope was exactly the #1259 independent FAERS evidence row for
  `metformin||trametinib dimethyl sulfoxide`, source ID `24608768`, and the
  four downstream candidate rows touched by that pair.
- Expected #1259 hashes were verified before processing.
- The exact FAERS query row was persisted separately from the interpreted case
  row.
- RxNorm identity rows were gathered for metformin, trametinib/trametinib DMSO,
  Mekinist, Eliquis, and apixaban.
- Label context was queried through openFDA labels and DailyMed SPL metadata/XML.
- Literature context was queried through Europe PMC and PubMed E-utilities.
- Every output row remains blocked behind case-level safety falsification and
  human review.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 7 | 8,535 | `6bece0dcb8f22e4af21608d69ba614d22d68df760ec741d20ac8aaa0d857a867` |
| `faers_query_rows.jsonl` | 1 | 1,053 | `6b2de6cfdad1ffbcd7b86427f54121972837b57a7e48155c8b55713191a8501a` |
| `faers_case_rows.jsonl` | 1 | 10,344 | `30f6908118eae47ba7fdec35a414766caea916a4762a910a45fc9aee7a1b3524` |
| `rxnorm_identity_rows.jsonl` | 12 | 17,222 | `59b797f289ea08380ee97bbaeec4e71c8f0305d5c9d96235048bf057acde2fbd` |
| `label_query_rows.jsonl` | 22 | 4,446,904 | `2735563b36974eaebedff9e2b94d2aab41fe453df620f9e7285f9d8a49eeadc1` |
| `label_evidence_rows.jsonl` | 154 | 848,168 | `551a10563115939917ab6bf8b54b99cebaf33794a5d3faed3139dfa7bae976a2` |
| `literature_query_rows.jsonl` | 10 | 173,531 | `bab78cbe58e3068d74e0f351d9d1a18f5d52918dbe809b5113c1b90f45dc8cff` |
| `literature_evidence_rows.jsonl` | 7 | 7,048 | `bf31398c4e1cf748c782b7f1a7390fa46c8d20db124cc471ad1f911369554585` |
| `case_rollups.jsonl` | 1 | 1,802 | `f1f045ed882eae25a8da0fda8025a35dd6b9a02f0b7db368cce5d89c127777a5` |
| `candidate_case_status.jsonl` | 4 | 4,552 | `cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656` |
| `issue1260_bridge_rows.jsonl` | 187 | 195,129 | `cfb2b6c7802ff025281c43441dd70b19d7b1b08135202840f7a56a10ff0eff18` |
| `validation_metrics.json` | - | 1,307 | `06971f05f0f9f9a70f606b605c2359854b3871ce4b855bc469033386693af194` |
| `output_manifest.json` | - | 6,021 | `b50fb116f79e30358ae42755fb121613c0bc196eeb1c3e4dbeb1c0f954dd967e` |
| `persisted_readback.json` | - | 4,990 | `c44de7d913ed74d44f91e5158ad547d314189f8b45ce1f6004fb59b2d2cf1ae0` |
| `calyx_bridge_corpus_stdout.json` | - | 873 | `4554af0e323d4f4a4f04e0d7f303798626313d7eda484ce3db0ade169e9ea788` |
| `calyx_bridge_corpus_stderr.txt` | - | 327 | `52480689a9534458b80e72c16a4da3b61e7b9c44704eb2dcc260c81a92f8fd8f` |
| `calyx_bridge_corpus_readback.json` | - | 1,869 | `a4f070383487e8ad0d0d2878a0cf8f79d534992d7aee6ab582ca4f1006815045` |

## Case Result

Exact FAERS query:

| Field | Value |
|---|---|
| URL | `https://api.fda.gov/drug/event.json?search=safetyreportid%3A24608768&limit=1` |
| HTTP status | 200 |
| Total | 1 |
| Returned | 1 |
| Raw response SHA-256 | `6820234835424c56b6834e2de7110944073ea2c1e1dc0955cbad8c432a43f284` |

Case rollup:

| Field | Value |
|---|---|
| Pair key | `metformin||trametinib dimethyl sulfoxide` |
| Source ID | `24608768` |
| Classification | `serious_faers_case_confounded_still_blocked` |
| Received date | `20241112` |
| Receipt date | `20241230` |
| Serious | true |
| Reactions | Off label use; Lower gastrointestinal haemorrhage |
| Drug count | 19 |
| Pair drugs present | true |
| Pair drugs are concomitant | true |
| Primary suspect drugs | Eliquis; Eliquis |
| Anticoagulant confounders | Eliquis; Eliquis |
| Candidate rows touched | 4 |

Reason codes:

- `serious_faers_report_found`
- `eliquis_primary_suspect_anticoagulant_confounder_present`
- `trametinib_metformin_concomitant_not_primary_suspect`
- `polypharmacy_case_report_not_pair_causality`
- `requires_human_review`

Interpretation: the case is a preserved safety blocker, not a validated
metformin-trametinib interaction. The case has both pair drugs, but they are
reported as concomitant. Eliquis appears twice as the primary suspect drug, and
the case has 19 total drugs.

Label context rows:

| Term | Rows |
|---|---:|
| `APIXABAN` | 47 |
| `ELIQUIS` | 47 |
| `METFORMIN` | 43 |
| `MEKINIST` | 6 |
| `TRAMETINIB` | 6 |
| `TRAMETINIB DIMETHYL SULFOXIDE` | 5 |

These are keyword safety-context rows only. They do not establish a pair
interaction or treatment/safety conclusion.

Literature context:

| Query | Total |
|---|---:|
| `metformin_trametinib` | 13 |
| `metformin_trametinib_bleeding` | 2 |
| `metformin_mekinist` | 0 |
| `metformin_trametinib_dimethyl_sulfoxide` | 0 |

Seven returned records passed the deterministic title/abstract both-term gate.
They are co-mention/context rows only, including preclinical oncology and drug
repositioning titles, and do not validate clinical actionability.

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected #1259 input hashes matched | true |
| One exact FAERS query row | true |
| Exact FAERS query total one | true |
| Exact FAERS query returned one | true |
| Exact FAERS raw response exists | true |
| One FAERS case row | true |
| FAERS case source ID matches | true |
| FAERS case has both pair drugs | true |
| FAERS case is serious | true |
| FAERS case has anticoagulant confounder | true |
| One case rollup row | true |
| Four candidate status rows | true |
| Case rollup blocked | true |
| Candidate statuses blocked | true |
| Bridge terms present in text | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1260-metformin-trametinib-faers-case-20260705t020000z
vault_id: 01KWQV29K8Y4KX17WTCCRFQ685
vault_dir: /home/croyse/calyx/vaults/01KWQV29K8Y4KX17WTCCRFQ685
rows: 187
bridge_terms: 55
graph_nodes_written: 242
graph_edges_written: 1918
csr_persisted: true
graph_file_count: 2163
graph_bytes: 1943124
```

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize exit zero | true |
| Materialize status ok | true |
| Row count matches read rows | true |
| Row SHA matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| Graph directory exists | true |
| Graph files present | true |
| CSR persisted | true |
| Index contains materialization name | true |
| Graph node count positive | true |
| Graph edge count positive | true |
| Domain counts match | true |
| Bridge metadata `source_dataset` present | true |

## Follow-Up State

This closes the #1259 independent FAERS signal at case-validation depth as a
confounded safety blocker. The next useful derived task is to feed FAERS role,
polypharmacy, and primary-suspect confounder features back into the
drug-combination ranker so serious co-reports are ranked by case quality rather
than raw co-presence.

---

## 102_faers_case_quality_ranker_overlay.md

# #1261 FAERS Case-Quality Ranker Overlay

Status: complete for the #1190 drug-combination ranker feedback pass using the
#1260 metformin-trametinib FAERS case validation.

This slice reads sealed #1190 combination-ranker artifacts and sealed #1260
case-validation artifacts, joins the #1260 confounded serious FAERS case onto
the metformin/trametinib family in #1190, emits case-quality/confounder features
and blocked ranker overlay rows, and materializes the overlay into native Calyx.

Clinical boundary:

```text
FAERS case-quality ranker overlay is safety/source/falsification triage only; case-quality rows, confounder features, and rank penalties are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1261-faers-case-quality-ranker-overlay-20260705T030000Z
```

Sealed inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| #1190 `candidate_pair_inputs.jsonl` | 1,750 | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1190 `drug_combination_hypotheses.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| #1190 `combination_safety_interaction_flags.jsonl` | 1,750 | `86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab` |
| #1190 `persisted_readback.json` | - | `9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2` |
| #1190 `calyx_bridge_corpus_readback.json` | - | `444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f` |
| #1260 `faers_query_rows.jsonl` | 1 | `6b2de6cfdad1ffbcd7b86427f54121972837b57a7e48155c8b55713191a8501a` |
| #1260 `faers_case_rows.jsonl` | 1 | `30f6908118eae47ba7fdec35a414766caea916a4762a910a45fc9aee7a1b3524` |
| #1260 `case_rollups.jsonl` | 1 | `f1f045ed882eae25a8da0fda8025a35dd6b9a02f0b7db368cce5d89c127777a5` |
| #1260 `candidate_case_status.jsonl` | 4 | `cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656` |
| #1260 `persisted_readback.json` | - | `c44de7d913ed74d44f91e5158ad547d314189f8b45ce1f6004fb59b2d2cf1ae0` |
| #1260 `calyx_bridge_corpus_readback.json` | - | `a4f070383487e8ad0d0d2878a0cf8f79d534992d7aee6ab582ca4f1006815045` |

Source contract:

- Input hashes were checked before processing.
- Pair normalization maps `trametinib dimethyl sulfoxide`, `trametinib`, and
  `Mekinist` to the ingredient-family key `trametinib`; metformin remains
  `metformin`.
- The #1260 case family key is `metformin||trametinib`.
- Direct #1260 candidate-status joins are preserved separately from the broader
  ingredient-family context row.
- The overlay does not mutate #1190 rows. It emits blocked overlay rows that can
  be consumed by a downstream ranker.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 11 | 10,500 | `1a54cadd9687ed6936c20942f5e05520d7afcdfc8140a2fb608e056eb0f4bbf6` |
| `case_quality_features.jsonl` | 5 | 11,639 | `22e9f8336dffa55349fa7c94ea2a14e85b22cbcef565fee15955ad1173b44c3c` |
| `ranker_case_quality_overlay.jsonl` | 5 | 10,120 | `d5f31d382afc25584577aab7b58b6c0268a71e5840fa170fde1467b0e8e0cbb8` |
| `family_case_quality_summary.jsonl` | 1 | 1,578 | `27a703ce5bf4a8eb0e19ce703742265db2120628ecc52ab310d832a1bb8cd533` |
| `issue1261_bridge_rows.jsonl` | 22 | 25,800 | `1609dfb625c34aecb67ad8ba56b50858b9c5db6290cc40337b79a3f06502756d` |
| `validation_metrics.json` | - | 998 | `d21541fed315baa35881578a476fc403cc6249bb6f880d3ee14bf69e5818ba72` |
| `output_manifest.json` | - | 8,846 | `859f130028c8a0b5f7ec15c0861b1a509bc1e930be335f0c5d5a681bf46d9180` |
| `persisted_readback.json` | - | 3,125 | `b4322712d8725a33ab9968ba22488f17298e5c22924e4e3888f88dded7274b3b` |
| `calyx_bridge_corpus_stdout.json` | - | 758 | `7ec8e3b27bd75cfe94d589209e4aa43c6cdbe69ec1aa0ce5ccc56d43be3a7a05` |
| `calyx_bridge_corpus_stderr.txt` | - | 319 | `446d0eb3e60ab2ae0cb5e31992f5f93c6e1c47a0b39d884e534277c871d874b7` |
| `calyx_bridge_corpus_readback.json` | - | 1,728 | `f05e25dfca9af51b4850c9db06d5713c92950d369a790d39687a1de84846ad45` |

## Metrics

| Metric | Count |
|---|---:|
| Source input rows | 11 |
| Case-quality feature rows | 5 |
| Ranker overlay rows | 5 |
| Family summary rows | 1 |
| Direct #1260 candidate-status joins | 4 |
| Ingredient-family context rows | 1 |
| Bridge rows | 22 |

Domain counts:

| Domain | Rows |
|---|---:|
| `issue1261_source_input` | 11 |
| `issue1261_case_quality_feature` | 5 |
| `issue1261_ranker_overlay` | 5 |
| `issue1261_family_summary` | 1 |

Case-quality class:

| Class | Rows |
|---|---:|
| `confounded_case_report_blocker` | 5 |

Overlay status:

| Status | Rows |
|---|---:|
| `blocked_case_quality_confounded_still_blocked` | 5 |

## Affected Ranker Rows

The overlay applies a deterministic 0.45 case-quality penalty. This is a
fail-closed ranker feature, not a measured clinical effect size.

| Pair ID | Pair | Disease | Original rank | Original score | Adjusted overlay score | Direct #1260 join |
|---|---|---|---:|---:|---:|---|
| `issue1190:85c287a7a7864b3621b8bd39` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Cardiofaciocutaneous syndrome 1 | 418 | 0.596529 | 0.146529 | true |
| `issue1190:0d987768bb01e33c82b5ebb9` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Cardiofaciocutaneous syndrome | 609 | 0.563515 | 0.113515 | true |
| `issue1190:ebdda04e878f80cb9f3c0de4` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Noonan syndrome | 736 | 0.546177 | 0.096177 | true |
| `issue1190:440d65be8fca3e11b9bd7dbf` | `TRAMETINIB DIMETHYL SULFOXIDE` + `METFORMIN` | Noonan syndrome 1 | 926 | 0.516483 | 0.066483 | true |
| `issue1190:c32560a52b99549f090012fe` | `Metformin` + `Trametinib` | Cancer | 1046 | 0.500000 | 0.050000 | false |

Summary:

```text
pair_family_key: metformin||trametinib
faers_source_id: 24608768
case_quality_class: confounded_case_report_blocker
case_quality_penalty_points: 0.45
affected_ranker_rows: 5
direct_issue1260_candidate_status_rows: 4
ingredient_family_context_rows: 1
all_rows_blocked: true
```

Reason codes added or carried forward:

- `serious_faers_report_found`
- `eliquis_primary_suspect_anticoagulant_confounder_present`
- `trametinib_metformin_concomitant_not_primary_suspect`
- `polypharmacy_case_report_not_pair_causality`
- `faers_case_quality_confounded_blocker`
- `faers_pair_concomitant_not_primary_suspect`
- `faers_anticoagulant_confounder_present`
- `faers_polypharmacy_case_report_not_pair_causality`

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected input hashes match | true |
| Affected ranker rows = 5 | true |
| Direct #1260 rows = 4 | true |
| Ingredient-family context rows = 1 | true |
| Family summary rows = 1 | true |
| Family key matches | true |
| All overlay rows blocked and no-promotion | true |
| Case-quality class is confounded | true |
| Penalty is positive | true |
| Bridge terms present in text | true |
| Bridge metadata `source_dataset` present | true |

## Native Calyx Materialization

```text
name: issue1261-faers-case-quality-ranker-overlay-20260705t030000z
vault_id: 01KWQVREZW2SGTRE38KS8G27MA
vault_dir: /home/croyse/calyx/vaults/01KWQVREZW2SGTRE38KS8G27MA
rows: 22
bridge_terms: 35
graph_nodes_written: 57
graph_edges_written: 172
csr_persisted: true
graph_file_count: 226
graph_bytes: 200586
```

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize exit zero | true |
| Materialize status ok | true |
| Row count matches read rows | true |
| Row SHA matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| Graph directory exists | true |
| Graph files present | true |
| CSR persisted | true |
| Index contains materialization name | true |
| Graph node count positive | true |
| Graph edge count positive | true |
| Domain counts match | true |
| Bridge metadata `source_dataset` present | true |

---

## 103_openfda_label_gate_validation.md

# #1241 openFDA Label Gate Validation

Status: complete for the #1236 openFDA Human Drug Label hit-validation pass.

This slice reads sealed #1236 label evidence/status artifacts, classifies each
persisted label source row through deterministic safety/interaction gates,
emits one gate-status row for every #1236 hit candidate row, and materializes
the validation overlay into native Calyx.

Clinical boundary:

```text
openFDA label gate validation is safety/source/falsification triage only; label safety rows, interaction-section rows, co-mentions, and false-positive context rows are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1241-openfda-label-gate-validation-20260705T040000Z
```

Sealed #1236 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `openfda_label_pair_evidence.jsonl` | 40 | `d2376732cea19e4f17d4f803e5e7c0b07cbd6ef4898fb7b527a6a1509c304ab3` |
| `candidate_openfda_label_status.jsonl` | 1,041 | `0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b` |
| `openfda_label_pair_status.jsonl` | 649 | `9a1680d5243a90ab52af7bb93b76f8e7677c7b7cef338282bf1cd781b5d7524a` |
| `persisted_readback.json` | - | `9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db` |
| `calyx_bridge_corpus_readback.json` | - | `5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e` |
| `output_manifest.json` | - | `c69cb405c3d36dce0536eb37361937aecaf4fd9d0d82d041a83b398053934420` |

Source contract:

- Do not re-query openFDA; classify the persisted #1236 source rows.
- One evidence gate row per #1236 label evidence row.
- One pair rollup per #1236 pair key with label evidence.
- One candidate gate-status row per #1236 hit candidate row.
- Preserve label safety/interaction language as review input only.
- Keep every row blocked behind independent safety, pair-interaction, outcome,
  falsification, and human-review gates.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 6 | 5,962 | `709379bd1120d09caed65555e83091b327cea037d34ef243307c1b6097c56f45` |
| `label_evidence_gate_rows.jsonl` | 40 | 103,733 | `d3a53039d3a5d866154f2576e95101f7b8e0bd9ac4cb87b6d15b312887ecc390` |
| `pair_label_gate_rollups.jsonl` | 9 | 14,393 | `0826cca2b76d539fbc6a90facd8bac467f7d0b556f80d37eda0f0fd2ba04d8bf` |
| `candidate_label_gate_status.jsonl` | 22 | 38,281 | `0cfcb6bfb3552c37435c00860e6e6648cf37b2fc7df7fd0ac6478d2e0876f400` |
| `issue1241_bridge_rows.jsonl` | 77 | 95,886 | `1aad095b99a8d85a08f3dc1c7cad0ace1f41be1623d37bb976bed24b77a718c7` |
| `validation_metrics.json` | - | 1,438 | `71429b14f5f12377b6d35c93e57b765b6cdbf3be8d5de9b6ce3163a6908e2cd3` |
| `output_manifest.json` | - | 5,879 | `e4ecba6303641e5c2b5b4b29205cb3d39ced329ef2d4f5f1fd9c113382130628` |
| `persisted_readback.json` | - | 3,093 | `e0e00ecba95f3eaf4451e107cf4b5f8f737c0824f29658fa040e07dc809eb83d` |
| `calyx_bridge_corpus_stdout.json` | - | 774 | `479c5e2fd34b10f18d4c3471ce68f5bc0967c56ea8bfe53cde5cb57707360d6b` |
| `calyx_bridge_corpus_stderr.txt` | - | 324 | `7bed985fec528148f2c187b0c24a2154dc305efe323ddc7a1bf4c8f78cd33e8f` |
| `calyx_bridge_corpus_readback.json` | - | 1,744 | `c6b759df3b56cae37d3954114c4005ee38fcd4f7f66c9f8b9d62adbbf3cf7eb1` |

## Metrics

| Metric | Count |
|---|---:|
| Source input rows | 6 |
| Label evidence gate rows | 40 |
| Pair rollup rows | 9 |
| Candidate gate-status rows | 22 |
| Bridge rows | 77 |

Evidence gate classifications:

| Classification | Rows |
|---|---:|
| `component_specific_safety_language_review_blocker` | 25 |
| `likely_false_positive_context_blocker` | 8 |
| `broad_label_comention_blocker` | 6 |
| `pair_interaction_language_review_blocker` | 1 |

Candidate gate statuses:

| Status | Candidate rows |
|---|---:|
| `blocked_component_safety_label_review_required` | 13 |
| `blocked_broad_label_comention_only` | 8 |
| `blocked_label_pair_interaction_review_required` | 1 |

Pair gate statuses:

| Status | Pair keys |
|---|---:|
| `blocked_component_safety_label_review_required` | 6 |
| `blocked_broad_label_comention_only` | 2 |
| `blocked_label_pair_interaction_review_required` | 1 |

## Pair Rollups

| Pair key | Candidate rows | Label evidence rows | Gate status | Evidence classification counts |
|---|---:|---:|---|---|
| `bepridil||ethosuximide` | 2 | 5 | `blocked_broad_label_comention_only` | broad 3; likely false-positive 2 |
| `bepridil||phenobarbital` | 1 | 5 | `blocked_label_pair_interaction_review_required` | broad 2; false-positive 2; pair-interaction review 1 |
| `cyclosporine||dextromethorphan hydrobromide` | 6 | 1 | `blocked_broad_label_comention_only` | broad 1 |
| `l methylfolate||nitrous oxide` | 1 | 5 | `blocked_component_safety_label_review_required` | component safety 3; false-positive 2 |
| `multivitamin||nitrous oxide` | 1 | 4 | `blocked_component_safety_label_review_required` | component safety 4 |
| `nisoldipine||phenytoin sodium` | 1 | 5 | `blocked_component_safety_label_review_required` | component safety 5 |
| `prednisolone||zolpidem` | 1 | 5 | `blocked_component_safety_label_review_required` | component safety 3; false-positive 2 |
| `sonidegib||tretinoin` | 8 | 5 | `blocked_component_safety_label_review_required` | component safety 5 |
| `streptomycin||zolpidem` | 1 | 5 | `blocked_component_safety_label_review_required` | component safety 5 |

The single `pair_interaction_language_review_blocker` row was:

```text
pair_key: bepridil||phenobarbital
source_issue1236_evidence_id: openfda-label-evidence:6d055a27620623c710d53278
openfda_label_id: 01c5574c-1056-49c6-af20-e950db3f4139
matched_section_fields: precautions; drug_interactions; drug_interactions_table
min_pair_token_distance: 25
direct_pair_section_count: 1
```

It is a review blocker, not pair-interaction proof or clinical clearance.

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected #1236 input hashes matched | true |
| Evidence gate rows = 40 | true |
| Candidate gate rows = 22 | true |
| Pair rollups = 9 | true |
| All evidence rows blocked | true |
| All candidate rows blocked | true |
| All pair rollups blocked | true |
| Every candidate has a rollup | true |
| Bridge terms present in text | true |
| Bridge metadata `source_dataset` present | true |

## Native Calyx Materialization

```text
name: issue1241-openfda-label-gate-validation-20260705t040000z
vault_id: 01KWQWAFVJASCQ0ASAMMNGM4P8
vault_dir: /home/croyse/calyx/vaults/01KWQWAFVJASCQ0ASAMMNGM4P8
rows: 77
bridge_terms: 101
graph_nodes_written: 178
graph_edges_written: 560
csr_persisted: true
graph_file_count: 653
```

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize exit zero | true |
| Materialize status ok | true |
| Row count matches read rows | true |
| Row SHA matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| Graph directory exists | true |
| Graph files present | true |
| CSR persisted | true |
| Index contains materialization name | true |
| Graph node count positive | true |
| Graph edge count positive | true |
| Domain counts match | true |
| Bridge metadata `source_dataset` present | true |

---

## 104_pubmed_structured_gate_validation.md

# #1239 PubMed Structured Gate Validation

Status: complete for the downstream gate-validation pass over the sealed #1238
PubMed structured extraction and candidate rollup artifacts.

This slice reads #1238 as sealed input, preserves counter-evidence fail-closed,
emits deterministic safety/outcome/falsification/human-review preflight rows,
and materializes a bounded validation overlay into native Calyx.

Clinical boundary:

```text
PubMed structured gate validation is safety/outcome/falsification preflight only; structured literature rows, counter-evidence rows, and missing-gate rows are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T012109Z
```

Sealed #1238 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `pubmed_structured_extraction.jsonl` | 510 | `10bc9ca90b97a089962bd85ee2f7881819668414859fdedb8040c86d78fa992e` |
| `candidate_pair_pubmed_structured_rollup.jsonl` | 301 | `e0c8db57ee492727fa525c9044dfcce85bf03acbfd7a03587030ab2d8393c3e6` |
| `candidate_pair_pubmed_structured_hits.jsonl` | 298 | `cb6ce01e363c6c320231cfaf01e59c9fafb891c6d437e1a9bc2f5ca924874c26` |
| `persisted_readback.json` | - | `14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413` |
| `calyx_bridge_corpus_readback.json` | - | `5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8` |
| `output_manifest.json` | - | `8fc6728117d993bb74ea3e1090ee94136f449ded4c5c89f733ce19fe0ba9ce74` |

Source contract:

- Verify the #1238 artifact hashes before processing.
- Emit one evidence gate row per #1238 structured extraction row.
- Emit one candidate gate-status row per #1238 candidate rollup row.
- Emit explicit missing/not-cleared rows for component safety,
  pair-interaction, outcome endpoint, falsification, and human review.
- Preserve all counter-evidence rows as falsification-review blockers.
- Keep every row blocked behind independent safety, outcome, falsification,
  and human-review gates.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 6 | 5,946 | `a4c71d4144737832c6522613b2a1a304047654f034f3d5507da8cb55f2f12b02` |
| `pubmed_structured_gate_evidence.jsonl` | 510 | 1,162,137 | `ec8137c141f37fcf1282e7bbb752ba5e100a215bfaf75dd77eab3ea7961d5db2` |
| `candidate_pubmed_gate_status.jsonl` | 301 | 810,904 | `67a0ff300092839addc6bd32b4922da954ee1b2469a905e5192895c041b838a7` |
| `pubmed_missing_gate_rows.jsonl` | 1,505 | 1,545,376 | `6ce1a596517ccbf52fe3cad5a7a8ef1970d95fd03f3266f7b8b295cce707fc02` |
| `gate_summary.jsonl` | 1 | 1,988 | `fbf31aa3ff8d8f3de8ff8d943b74406092ee258b48ae81ec6756c39fe035aa15` |
| `issue1239_bridge_rows.jsonl` | 1,000 | 1,165,572 | `89d23691fd2b0d8f24d4404dae9d30ec0d09ff74af12f3a7405672513609138d` |
| `validation_metrics.json` | - | 2,179 | `e5185c29d42195c8b498c20850b8624e6c15d2574d59dd5ebadcd363bc4ab33f` |
| `output_manifest.json` | - | 6,287 | `3da4ca41406bd38b42ec214ac16424aada832413ba8f54420b743deb1f6f6705` |
| `persisted_readback.json` | - | 3,527 | `c8661ab0546bc128fde69fcf35ff79bf7d290f9662063cddc2711f14e85153ac` |
| `calyx_bridge_corpus_stdout.json` | - | 806 | `1e6e3285098150ca46780263ef0823eb36ec27b014dcd570502de8ace4864a5e` |
| `calyx_bridge_corpus_stderr.txt` | - | 331 | `94dee7b1a64dfae4640f4fe60c6455ec491e3fc4b882d9760540a454c1f3152c` |
| `calyx_bridge_corpus_readback.json` | - | 4,491 | `a8acf5aae36a4fd94325d146933ee0d13c96420e1ab99c155238aac3866ac8a2` |

## Metrics

| Metric | Count |
|---|---:|
| Source input rows | 6 |
| PubMed structured evidence gate rows | 510 |
| Candidate gate-status rows | 301 |
| Missing/not-cleared gate rows | 1,505 |
| Counter-evidence evidence rows preserved | 224 |
| Bridge rows | 1,000 |

Required-gate accounting:

| Gate | Rows |
|---|---:|
| `component_safety` | 301 |
| `pair_interaction` | 301 |
| `outcome_endpoint` | 301 |
| `falsification` | 301 |
| `human_review` | 301 |

Every #1238 candidate rollup has exactly five required-gate rows. Source
language can mark a gate as review input, but no source-language row clears an
independent gate.

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected #1238 input hashes matched | true |
| Evidence gate rows = 510 | true |
| Candidate gate rows = 301 | true |
| Missing gate rows = 1,505 | true |
| Counter-evidence preserved = 224 | true |
| All evidence rows blocked | true |
| All candidate rows blocked | true |
| All missing-gate rows blocked | true |
| Every candidate has five required-gate rows | true |
| Bridge rows <= 1,000 | true |
| Bridge terms present in text | true |
| Bridge metadata `source_dataset` present | true |

## Native Calyx Materialization

```text
name: issue1239-pubmed-structured-gate-validation-20260705t012109z
vault_id: 01KWQXYT0KB77KMVZJ5A018QH5
vault_dir: /home/croyse/calyx/vaults/01KWQXYT0KB77KMVZJ5A018QH5
rows: 1,000
bridge_terms: 950
graph_nodes_written: 1,950
graph_edges_written: 6,000
csr_persisted: true
```

Materialization domain counts:

| Domain | Rows |
|---|---:|
| `issue1239_pubmed_gate_evidence` | 510 |
| `issue1239_candidate_gate_status` | 301 |
| `issue1239_missing_gate` | 182 |
| `issue1239_source_input` | 6 |
| `issue1239_gate_summary` | 1 |

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize stdout status ok | true |
| Bridge row count matches stdout | true |
| Bridge row SHA matches stdout | true |
| Graph node count matches written count | true |
| Graph edge count matches written count | true |
| CSR persisted | true |
| Index contains exactly one active materialization name | true |
| Index vault id matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| `cf/graph` SST files present | true |
| Vault-tree readback nonempty | true |
| Native manifest version readback ok | true |

The native manifest readback used:

```text
calyx readback vault-manifest --field version --vault /home/croyse/calyx/vaults/01KWQXYT0KB77KMVZJ5A018QH5
```

It returned:

```json
{"major":1,"minor":0}
```

During physical readback, `calyx readback --vault <vault> --show-manifest`
incorrectly routed this native vault through the shadow-manifest parser and
returned `CALYX_MANIFEST_CORRUPT`. That is tracked separately as #1262 and did
not invalidate the native vault-manifest readback above.

## Result

#1239 did not produce any efficacy, safety-clearance, dosing, recommendation,
clinical-actionability, pair-interaction-proof, or cure claim. It produced a
deterministic PubMed structured validation overlay: all 510 structured rows,
301 candidate rollups, and 1,505 required-gate rows remain blocked pending
independent safety, outcome, falsification, and human review.

---

## 105_clinicaltrials_gate_validation.md

# #1235 ClinicalTrials.gov Gate Validation

Status: complete for the #1232 ClinicalTrials.gov hit validation slice.

#1235 reads the sealed #1232 ClinicalTrials.gov v2 artifacts and classifies the
204 registry-hit rows into deterministic trial-context and gate-status rows.
The validator uses persisted raw API response bytes from #1232; it does not use
registry co-occurrence as efficacy, safety, pair-interaction, dosing, treatment
guidance, clinical actionability, recommendation, or cure evidence.

Clinical boundary:

```text
ClinicalTrials.gov trial-context validation is registry/source triage only; same-arm context, adverse-event modules, and outcome fields are blockers or review inputs, not efficacy, safety clearance, dosing guidance, treatment guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Implementation

Script:

```text
scripts/medicalsearch/issue1235_clinicaltrials_gate_validation.py
```

The script:

- verifies sealed #1232 artifact hashes;
- streams the persisted #1232 raw ClinicalTrials.gov response JSONL;
- extracts NCT ids, phase, status, arm labels, intervention names, outcomes,
  results modules, and adverse-event module counts;
- classifies trial context as same-arm, comparator-only, broad intervention
  list, registry context only, or likely self/salt false positive;
- emits one validation status row per #1232 pair hit;
- emits one not-cleared/missing gate row per pair/gate;
- writes a bounded 1,000-row bridge corpus for native Calyx materialization.

## FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1235-clinicaltrials-gate-validation-20260705T023240Z
```

Script capture:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `script.stdout.txt` | 13,573 | `5d8a18d6f21218017c63c720315038854e701c5c45aa0c7430fda223006b0683` |
| `script.stderr.txt` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `script.capture.txt` | 428 | `0339361dd874ca3070923fb715fec1885c743a7d62406f43c3b933c7adfd4474` |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1232 `clinicaltrials_pair_hits.jsonl` | 204 rows | `026cc1d5698f6b7b2fdcf2cd095cf0a605a804f1ed3f40ed0bd151342c006c38` |
| #1232 `clinicaltrials_study_evidence.jsonl` | 1,192 rows | `cb25d98c9fd5c623d63c31a9dcf7b7fbc55add65cc0a7b9d265fe83e5f875970` |
| #1232 `clinicaltrials_pair_status.jsonl` | 1,546 rows | `ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc` |
| #1232 `clinicaltrials_raw_responses.jsonl` | 480,925,442 bytes | `1f8d40df3bac91b30807e9eae785a70f6710ebccf8304b7c8e2eb678ee2b6e2d` |
| #1232 `persisted_readback.json` | - | `606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1` |
| #1232 `calyx_bridge_corpus_readback.json` | - | `ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a` |
| #1232 `output_manifest.json` | - | `ad6e55d387c2fe0f6558561ee403ffc4f2350a9bc5f81ad08f05e76afd123d1d` |

## Outputs

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 7 | 5,976 | `f68655a04364b82ee3c806d9a0f004ce69d8f575209b7e9fdca676ca09499bef` |
| `clinicaltrials_trial_context_rows.jsonl` | 1,192 | 4,476,223 | `c1660079cf090adc09ac7076008efaf51e939956167ca58e38a19350b2cd4e62` |
| `clinicaltrials_pair_validation_status.jsonl` | 204 | 349,687 | `9596b2e7e39c78714f2740073991880e8a76c6427e48704af9dedc8f53fdf4f0` |
| `clinicaltrials_missing_gate_rows.jsonl` | 816 | 718,100 | `ff00f66206a8091716e1bf559812abde986e1cec58c8ab2fd5ad83d08900b09d` |
| `issue1235_bridge_rows.jsonl` | 1,000 | 1,206,017 | `c5663f6a4c6b21ef4db1db0f37c3b33e5c763fbc550c2450c40e3b06a9dc6523` |
| `validation_metrics.json` | - | 10,552 | `c0c13899e960a7c5071703f3daea7f4e2927165129e9ccf9a6bf3120872fc5a2` |
| `input_manifest.json` | - | 7,381 | `b4e1a0c00e2ea6e614d3924f98f5059cfafcf20f9a65816a049cc9abefb27bd4` |
| `output_manifest.json` | - | 2,454 | `2bc60192f7cf82e3f7d8ad784d825a7874f930fffaf962936087ad098efb5b5d` |
| `persisted_readback.json` | - | 3,408 | `1a00de5e65343793805f25d49617cad949b919738e9d1e0a72c83df98a0a7848` |
| `calyx_bridge_corpus_stdout.json` | - | 717 | `dcbd79f9b7f4f6e461d0a81741add5f17053273e0d8a4619f175f1dbf3839131` |
| `calyx_bridge_corpus_stderr.txt` | - | 334 | `0345d4401da3e1bee00e71655a01486ff037031e46dbcc8a78905301c780fece` |
| `calyx_bridge_corpus.capture.txt` | - | 463 | `555793d1380b0e7a0820d613016ceac0ceb63faf690d7a4caffcdb090042fbc1` |
| `calyx_bridge_corpus_readback.json` | - | 8,432 | `1155e1a65c5fa78db98bcf04a94e22abcc69df6e566e667f4260e87752424442` |

## Metrics

| Metric | Count |
|---|---:|
| #1232 pair-hit rows checked | 204 |
| #1232 study-evidence rows checked | 1,192 |
| Trial-context rows | 1,192 |
| Pair validation status rows | 204 |
| Missing/not-cleared gate rows | 816 |
| Bridge rows | 1,000 |
| Unique pair keys | 67 |
| Unique NCT ids | 399 |
| Same-arm pair rows | 147 |
| Pairs with adverse-event context | 96 |
| Pairs with outcome fields | 204 |

Validation status counts:

| Status | Rows |
|---|---:|
| `blocked_no_pair_interaction` | 81 |
| `blocked_no_safety` | 66 |
| `registry_context_only` | 42 |
| `rejected_false_positive` | 15 |

Trial-context class counts:

| Class | Rows |
|---|---:|
| `same_arm_combination_context` | 515 |
| `likely_false_positive_self_or_salt_duplicate` | 539 |
| `comparator_only_cooccurrence` | 101 |
| `broad_intervention_list_context` | 37 |

Gate status counts:

| Gate status | Rows |
|---|---:|
| `not_cleared_registry_cooccurrence_not_synergy_or_pair_interaction_proof` | 204 |
| `missing_human_review_fail_closed` | 204 |
| `trial_outcome_fields_present_not_grounded_outcome_clearance` | 204 |
| `trial_adverse_event_context_present_not_safety_clearance` | 96 |
| `component_safety_evidence_missing_fail_closed` | 108 |

## Native Calyx Materialization

```text
name: issue1235-clinicaltrials-gate-validation-20260705t023240z
vault_id: 01KWR1YT96G56QJMYAWCJ7SV8X
vault_dir: /home/croyse/calyx/vaults/01KWR1YT96G56QJMYAWCJ7SV8X
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 496 |
| Graph nodes written | 1,496 |
| Graph edges written | 10,700 |
| CSR persisted | true |
| Active vault index contains name exactly once | true |
| Active index vault id matches | true |
| `CURRENT` present | true |
| `MANIFEST` present | true |
| Manifest JSON present | true |
| Graph SST present | true |
| Time-index SST present | true |
| Bridge-row SHA matches materializer stdout | true |
| Graph nodes match materializer readback | true |
| Graph edges match materializer readback | true |

## Assertions

`persisted_readback.json` records all assertions true:

- #1232 persisted and Calyx readback assertions are all true.
- All #1232 input hashes match expected values.
- All 204 #1232 pair-hit rows and all 1,192 study-evidence rows are checked.
- A raw persisted response was found for every #1232 hit pair.
- Every hit has exactly one #1235 pair status row.
- Every #1232 study-evidence row has a #1235 trial-context row.
- Validation statuses are restricted to blocked, registry-context, or rejected
  fail-closed classes.
- Registry co-occurrence is never counted as pair-interaction proof.
- Bridge rows are bounded at 1,000.

## Result

#1235 classifies the 204 #1232 ClinicalTrials.gov registry hits into
deterministic, fail-closed gate statuses. Same-arm context exists for 147 pair
rows, and adverse-event context exists for 96 rows, but no row clears
independent safety, pair-interaction/synergy, outcome, or human-review gates.
All rows remain blocked or rejected as likely false positives.

No efficacy claim, safety-clearance claim, pair-interaction proof, treatment
guidance, dosing guidance, recommendation, clinical-actionability claim, or
cure claim is made.

---

## 106_cdcdb_gate_validation.md

# #1233 CDCDB Gate Validation

Status: complete for the CDCDB-supported row validation slice.

#1233 reads the sealed #1231 CDCDB artifacts and classifies every CDCDB hit
into source-context, exact-pair versus multi-drug context, and fail-closed gate
status rows. CDCDB source context is not treated as synergy, pair-interaction
proof, safety clearance, efficacy, treatment guidance, dosing guidance,
clinical actionability, recommendation, or cure evidence.

Clinical boundary:

```text
CDCDB source-context validation is external combination-source triage only; ClinicalTrials.gov, Orange Book, patent, exact-pair, and multi-drug context rows are blockers or review inputs, not synergy, pair-interaction proof, efficacy, safety clearance, dosing guidance, treatment guidance, recommendation, clinical actionability, or cure evidence.
```

## Implementation

Script:

```text
scripts/medicalsearch/issue1233_cdcdb_gate_validation.py
```

The script:

- verifies sealed #1231 artifact hashes;
- reads the CDCDB hit rows and prior no-hit recheck rows;
- classifies CDCDB source context as ClinicalTrials.gov, patent, Orange Book,
  or mixed;
- classifies each hit as an exact two-drug source record, multi-drug context
  only, or both;
- emits one pair validation status row per CDCDB hit;
- emits one missing/not-cleared gate row per pair/gate;
- writes a bounded bridge corpus for native Calyx materialization.

## FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1233-cdcdb-gate-validation-20260705T025403Z
```

Script capture:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `script.stdout.txt` | 4,006 | `92d6d19beab9ec30410dabebbca4aa52e749d8d813d7b14969e35d3b9832ee17` |
| `script.stderr.txt` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `script.capture.txt` | 409 | `370bb849f8c82ee9b9697944d59e0058e0b84ddb6c6a343eb2bd1352057a4abd` |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1231 `candidate_external_combo_hits.jsonl` | 173 rows | `c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a` |
| #1231 `candidate_external_combo_status.jsonl` | 1,750 rows | `f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4` |
| #1231 `prior_no_hit_recheck_status.jsonl` | 1,682 rows | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |
| #1231 `cdcdb_source_combinations.jsonl` | 43,082 rows | `004b07ee5b308501d048a8a919d6ce7af9dcd086897b25b777c17d43d6a32f79` |
| #1231 `cdcdb_pair_index.jsonl` | 78,321 rows | `55d77873ae1c7cd1a5d670c82050de6b22134f419fafccdd3a7edf7742c75714` |
| #1231 `cdcdb_source_schema.json` | 6,587 bytes | `d5f27a3c635bd23984dd303fff882e61c42c9b6e828df324950bdc88349ac7f2` |
| #1231 `validation_metrics.json` | 9,600 bytes | `ebdc0863663e14ebb9bc96d84fe9d5787a4b14c279f5bb5ca34c37750c1f041d` |
| #1231 `persisted_readback.json` | 3,259 bytes | `e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825` |
| #1231 `calyx_bridge_corpus_readback.json` | 3,779 bytes | `2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db` |
| #1231 `output_manifest.json` | 2,866 bytes | `b62fdabdfff60d34ee17d3dc983c8f2192a91add3bb100775df3361ac58ddf29` |

## Outputs

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 10 | 8,784 | `fb1bb49b2a01018596b7c18443bde99d56678ea782b1786a93aee9dc930f0e89` |
| `cdcdb_source_context_rows.jsonl` | 611 | 932,950 | `5924817886ddba2828df1e33b4ab8c5141fb08024cab4371526934c26e72c6c2` |
| `cdcdb_pair_validation_status.jsonl` | 173 | 366,915 | `c0d3e03419dc1240260b18a81a9e9bf3601305773cb3ec99a6b217d81825f3b2` |
| `cdcdb_missing_gate_rows.jsonl` | 692 | 618,947 | `3ce4b35f88a385ca1aaf977690925981ab8c07eeb7d9b4e686b3bb572d53ec35` |
| `issue1233_bridge_rows.jsonl` | 784 | 915,177 | `9cbca17975fb44a719c74781d29452ae34335b40018834638c69c4e3bf5e4868` |
| `validation_metrics.json` | - | 1,751 | `6993bc18d3c8b9e15f71b3d73b6009c5d11e6bdc6a2770a1287ce0cf041cb94c` |
| `input_manifest.json` | - | 10,565 | `d0ba1de6dc342b16c6f333ecc17a8bfcaa4e364e3eb0f85daafcfe709cb215d8` |
| `output_manifest.json` | - | 2,366 | `26ef2f4b51cccb500d29ea4bbfdae596e5bed526702defdb8ed42444439cadb5` |
| `persisted_readback.json` | - | 3,288 | `c67ffc5baa8f5d8477ed942dd3e0183b12217c1f9a9c15f5631d10e652c1e676` |
| `calyx_bridge_corpus_stdout.json` | - | 678 | `ddb31ebe617614dc48d8e1ad178550d2eecdf80befc65f72870c87bf311dd998` |
| `calyx_bridge_corpus_stderr.txt` | - | 331 | `ffc24db945bc6f0d817741d26153e129e7726998301cef8b7c513d85eda99f2a` |
| `calyx_bridge_corpus.capture.txt` | - | 445 | `63bf7ed5c2ae5f7d550e5f87336d27ee97e75c550e6347ab0fed3f56a3a34441` |
| `calyx_bridge_corpus_readback.json` | - | 8,364 | `5eec4117b0c6dd4a58d9e2d2ba4d600043020721b2f9997a57ee54aa506effc4` |

## Metrics

| Metric | Count |
|---|---:|
| CDCDB hit rows checked | 173 |
| Prior no-hit recheck rows checked | 1,682 |
| Prior no-hit rows with CDCDB hit | 136 |
| Source-context rows | 611 |
| Pair validation status rows | 173 |
| Missing/not-cleared gate rows | 692 |
| Bridge rows | 784 |
| Unique pair keys | 85 |
| Pair rows with an exact two-drug CDCDB record | 63 |
| Pair rows with multi-drug context only | 110 |

Validation status counts:

| Status | Rows |
|---|---:|
| `blocked_no_safety` | 163 |
| `blocked_component_safety_review` | 10 |

CDCDB source context counts:

| Context | Rows |
|---|---:|
| `patent_context` | 67 |
| `mixed_clinicaltrialsgov_patents` | 55 |
| `clinicaltrials_registry_context` | 51 |

Pair record class counts:

| Class | Rows |
|---|---:|
| `multi_drug_context_only` | 110 |
| `two_drug_and_multi_drug_context` | 48 |
| `exact_two_drug_source_record` | 15 |

Gate status counts:

| Gate status | Rows |
|---|---:|
| `component_safety_missing_fail_closed` | 163 |
| `component_safety_flags_review_required_not_clearance` | 10 |
| `pair_interaction_evidence_missing_fail_closed` | 168 |
| `cdcdb_source_context_not_pair_interaction_or_synergy_proof` | 5 |
| `grounded_outcome_endpoint_missing_fail_closed` | 173 |
| `missing_human_review_fail_closed` | 173 |

## Native Calyx Materialization

```text
name: issue1233-cdcdb-gate-validation-20260705t025403z
vault_id: 01KWR35QT2C6GWHZNMX32PNKDK
vault_dir: /home/croyse/calyx/vaults/01KWR35QT2C6GWHZNMX32PNKDK
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 784 |
| Bridge terms | 358 |
| Graph nodes written | 1,142 |
| Graph edges written | 6,964 |
| CSR persisted | true |
| Active vault index contains name exactly once | true |
| Active index vault id matches | true |
| `CURRENT` present | true |
| `MANIFEST` present | true |
| Manifest JSON present | true |
| Graph SST present | true |
| Time-index SST present | true |
| Bridge-row SHA matches materializer stdout | true |
| Graph nodes match materializer readback | true |
| Graph edges match materializer readback | true |

## Assertions

`persisted_readback.json` records all assertions true:

- #1231 persisted readback assertions are all true and Calyx readback status is ok.
- All #1231 input hashes match expected values.
- All 173 CDCDB hit rows and all 1,682 prior no-hit rows are checked.
- The 136 prior no-hit CDCDB hits match #1231.
- Every CDCDB hit has exactly one #1233 pair status row and at least one
  source-context row.
- All statuses are fail-closed.
- CDCDB source context is never counted as synergy or pair-interaction proof.
- Bridge rows are bounded.

## Result

#1233 classifies all 173 CDCDB-supported rows into deterministic fail-closed
statuses. CDCDB provides source context for external combination documentation,
including 63 rows with at least one exact two-drug source record and 110 rows
that are multi-drug-context-only. All rows remain blocked on safety review,
pair-interaction/synergy, grounded outcome, and human-review gates.

No efficacy claim, safety-clearance claim, pair-interaction proof, treatment
guidance, dosing guidance, recommendation, clinical-actionability claim, or
cure claim is made.

---

## 107_safety_interaction_coverage_rollup.md

# #1228 Safety/Interaction Coverage Rollup

## Scope

#1228 aggregates the sealed #1190 drug-combination worklist and all follow-on
safety/interaction source-mining outputs into one coverage table. The goal is
accounting: every #1190 component drug and candidate pair receives either
source-backed context or an explicit fail-closed no-hit/not-cleared row.

This is safety and interaction triage only. Source rows, no-hit rows,
adverse-event rows, label text, registry context, literature context, and
identity mappings are blockers or review inputs, not safety clearance,
efficacy, treatment guidance, dosing guidance, recommendation, clinical
actionability, pair-interaction proof, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1228_safety_interaction_coverage_rollup.py
```

The script:

- verifies 57 sealed input artifacts by row count and SHA-256;
- reads the original #1190 component and pair universes;
- aggregates component safety context from initial safety rows, openFDA label
  gates, Europe PMC safety/counter review, openFDA FAERS, OffSIDES, and RxNorm
  mappings;
- aggregates pair source context from DrugComb, NCI ALMANAC, CDCDB,
  ClinicalTrials.gov, FDA/Orange Book/NDC/PubMed, openFDA labels, RxNorm,
  DailyMed, Europe PMC, FAERS, PubChem, ChEMBL, DrugCentral, PharmGKB, nSIDES,
  and case-quality overlays;
- emits explicit fail-closed gap rows for every #1190 candidate pair;
- writes a bounded bridge-corpus slice for native Calyx materialization.

## Real FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z/out/persisted_readback.json
sha256: 3ec89175f6cf47b144238a99dd0bb454527e7aa7723799edba6ae2e71b6774d4

/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z/out/calyx_bridge_corpus_readback.json
sha256: f2da81177e9e18fbde45d0cdcb22efbbb7cc4afd9d9ffda12eb7ff4017b80efb
```

Native Calyx materialization:

```text
name: issue1228-safety-interaction-coverage-rollup-20260705t032413z
vault_id: 01KWR4XV6G2AT5QXCPCEW3S2Q6
vault_dir: /home/croyse/calyx/vaults/01KWR4XV6G2AT5QXCPCEW3S2Q6
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,421 |
| Graph nodes | 2,421 |
| Graph edges | 7,446 |
| Graph SST files | 9,870 |
| Graph bytes | 9,322,240 |
| CSR persisted | true |
| Active vault index contains name | true |
| Active vault index vault id matches | true |
| Vault `CURRENT` / `MANIFEST` present | true |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `source_coverage_rows.jsonl` | 57 | `fd22d3800e5360b85c5c33dadec3838cd8b440524aaf9e40e0e0152d5e4eb18d` |
| `component_safety_coverage_rows.jsonl` | 277 | `b11d5d384c315e86666620fa02c1cdae219a6133e7ac410cd28e9e580cc0f6ae` |
| `component_safety_no_hit_rows.jsonl` | 113 | `b5f1558a9b14c1c438c2d0863fd0981702b31665d511243cb9632940efc681df` |
| `pair_interaction_coverage_rows.jsonl` | 1,750 | `cd36320b038f8af781f7a1a3c6c5a864c16352e24212841a83a2451589e0d614` |
| `pair_interaction_gap_rows.jsonl` | 1,750 | `15e65084a54e90b9e6d161b3f054437f17d6320289c41fdf12d74a9352eb89e0` |
| `issue1228_bridge_rows.jsonl` | 1,000 | `59897db9fde850c2353a5f093ae67f00416f6a5aa0615980975994d02ef91df7` |
| `validation_metrics.json` | - | `71a2665e773348e2d8c29605ec4ffeecd912f0c4c55b10f4a4b2614ae81096da` |
| `output_manifest.json` | - | `ae2c2df386a9ba96e7101a42aede7b0b468d8a30d06f0a55e9c3a1208ce6ab24` |

## Metrics

| Metric | Count |
|---|---:|
| Sealed source input files | 57 |
| #1190 component input rows | 1,023 |
| Unique #1190 component drugs | 277 |
| Component safety-context present, still blocked | 164 |
| Explicit component safety no-hit rows | 113 |
| #1190 candidate pair rows | 1,750 |
| Pair coverage rows | 1,750 |
| Pair gap rows | 1,750 |
| Pairs with source context, not clearance | 1,234 |
| Explicit pair source no-hit rows | 516 |
| Bridge rows materialized | 1,000 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Source hashes and row counts match expected sealed values | true |
| Component input rows = 1,023 | true |
| Candidate pair rows = 1,750 | true |
| Component coverage rows match unique drugs | true |
| Pair coverage rows = 1,750 | true |
| Pair gap rows = 1,750 | true |
| All component and pair rows remain blocked | true |
| Bridge rows are bounded to 1,000 | true |
| Bridge terms appear in bridge text | true |
| Materializer status is `ok` | true |
| Bridge-row SHA matches materializer stdout | true |
| Vault directory and graph SST files exist | true |
| Graph node/edge counts match materializer readback | true |

## Findings

- #1190's broad component universe is now explicitly accounted for: 277 unique
  drug terms from 1,023 component rows have coverage rows, and 113 have explicit
  no-hit fail-closed rows.
- #1190's candidate-pair universe is now explicitly accounted for: all 1,750
  candidate rows have pair coverage rows and gap rows.
- Source context is present for 1,234 candidate pairs, but every such row remains
  a review blocker rather than a clearance or pair-interaction proof.
- 516 candidate pairs have explicit no-hit rows for pair source context.
- The aggregate evidence is materialized in native Calyx vault
  `01KWR4XV6G2AT5QXCPCEW3S2Q6`.

## Conclusion

#1228 is satisfied as coverage accounting: every #1190 component and pair is
covered by source-backed context or explicit fail-closed missing/not-cleared
rows, with source hashes, native Calyx materialization, and readback evidence.

No treatment claim, efficacy claim, safety-clearance claim, pair-interaction
proof, clinical-actionability claim, recommendation, dosing guidance, or cure
claim is made.

---

## 108_target_match_fallacy_direction_gate.md

# #1269 Target-Match Fallacy Mechanistic Direction Gate

## Scope

#1269 fixed the target-match fallacy in the biomedical hypothesis engine. The
bug class was mechanistic direction loss: the pipeline could treat a
target-disease match as supportive without first proving whether disease biology
requires target inhibition, activation, or replacement/restoration, and without
distinguishing loss-of-function, gain-of-function, dosage-loss, and dosage-gain
evidence.

This is an engine correctness gate. It does not create a treatment claim,
efficacy claim, safety claim, dosing claim, recommendation, actionability
claim, pair-interaction proof, or cure evidence.

## Implementation

Primary module:

```text
crates/calyx-cli/src/cmd/mechanistic_direction.rs
```

Pipeline surfaces updated:

- `association-validation-gates` now admits Open Targets target-disease rows as
  positive benchmark evidence only when source-backed direction-on-target and
  direction-on-trait imply a required target modulation.
- `typed-association-miner` preserves mechanism-sensitive orientation and
  blocks gene-disease or drug-target candidates whose mechanism direction is
  missing, unrecognized, or internally conflicting.
- `hypothesis-falsification-sweep` treats required-modulation and observed-drug
  action conflicts as counter-evidence instead of support.
- Persisted JSON/JSONL reports now expose mechanistic direction counts,
  blocked rows/candidates, reason codes, inferred required target modulation,
  observed action modulation, mutation consequence, and source fields.

Mechanistic contract:

| Disease mechanism | Trait effect | Required target modulation |
|---|---|---|
| Gain of function / dosage gain | Risk | Inhibit |
| Gain of function / dosage gain | Protective | Activate |
| Loss of function / dosage loss | Risk | Replace or restore |
| Loss of function / dosage loss | Protective | Inhibit |

Drug-target action vocabulary is normalized from ChEMBL/DGIdb-style fields such
as `action_type`, `interactionTypes`, `directionality`, `moa`, and
`mechanism_of_action`. Ambiguous trait wording such as a bare `association`
does not imply risk or protection; it is blocked with an explicit reason code.

## Source Research

The implementation follows source-backed direction models:

- Open Targets direction-of-effect evidence combines direction on target and
  direction on trait.
- ChEMBL action types distinguish activating/agonist/positive-modulator actions
  from inhibiting/antagonist/blocker actions.
- DGIdb interaction directionality groups interactions as activating or
  inhibiting by mechanism.
- ClinGen dosage sensitivity distinguishes haploinsufficiency and
  triplosensitivity evidence.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\target_match_fallacy_final_20260707_110320
```

Manual FSV log:

```text
C:\code\Calyx-Dev\target\fsv\target_match_fallacy_final_20260707_110320\manual_fsv_log.txt
```

Source of truth:

```text
CLI persisted JSON/JSONL artifacts under the final FSV root. Verification used
separate file reads after command execution, not command return values alone.
```

Live source readbacks used by the final FSV:

| Source | Readback |
|---|---|
| Open Targets | `directionOnTarget=GoF`, `directionOnTrait=risk`, `score=1` |
| ChEMBL | `action_type=INHIBITOR`, `mechanism=TNF-alpha inhibitor` |

Primary readbacks:

| Artifact | Assertion |
|---|---|
| `out_validation/association_validation_report.json` | `gate_passed=true`, `blocked_direction_rows=0`, `inferred_required_direction_rows=1` |
| `out_miner_gene_disease/typed_association_miner_report.json` | one BRAF/cardiofaciocutaneous syndrome hypothesis; `required_target_modulation=inhibit`; `mutation_consequence=gain_of_function` |
| `out_miner_drug_target/typed_association_miner_report.json` | one adalimumab/TNF hypothesis; `observed_target_modulation=inhibit` |
| `out_falsification/falsification_sweep_report.json` | `support_evidence_count=2`, `counter_evidence_count=0` |

Boundary and edge-case readbacks:

| Case | Expected outcome | Persisted proof |
|---|---|---|
| Missing Open Targets direction | fail closed | command exit `2`; `mechanistic_direction_blocked_rows.jsonl` has `CALYX_MECH_TARGET_CONSEQUENCE_MISSING` |
| Invalid miner direction | fail closed | command exit `2`; `blocked_candidates.jsonl` records missing/unrecognized direction reasons |
| Falsification direction conflict | counter-evidence | `counter_evidence.jsonl` has `mechanistic_required_direction_conflict` |

Synapse readback also inspected the final FSV tree and confirmed:

```text
gate=True blocked=0 inferred=1
```

## Verification Commands

```text
cargo build -p calyx-cli
cargo check -p calyx-cli
cargo clippy -p calyx-cli --all-targets -- -D warnings
cargo test -p calyx-cli mechanistic_direction -- --nocapture
cargo test -p calyx-cli association_validation -- --nocapture
cargo test -p calyx-cli typed_association_miner -- --nocapture
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
git diff --check
```

All passed on the local authoring checkout before commit.

## GitHub State

Closed issues:

- #1269 epic: source-backed mechanistic direction gates.
- #1270 schema and report contract.
- #1271 Open Targets direction-of-effect gate.
- #1272 drug-target action normalization.
- #1273 mutation consequence and dosage mechanism screen.
- #1274 direction-aware typed mining and no unsafe reversal.
- #1275 falsification conflict gate.
- #1276 final FSV.

## Findings

- The root cause was not a single bad comparison; it was missing mechanistic
  direction as a required contract between validation, mining, and
  falsification.
- The fix makes mechanistic direction part of the persisted hypothesis surface,
  so later stages cannot silently reinterpret a target match as support.
- Unknown, unrecognized, ambiguous, or conflicting direction now creates a
  durable blocked-row artifact or counter-evidence artifact with reason codes.
- The system remains fail-closed: there are no permissive fallbacks for missing
  target mechanism or drug action direction.

## Conclusion

#1269 is complete. Target-disease and drug-target evidence now has to carry
source-backed mechanistic direction before it can support a hypothesis, and
direction conflicts are surfaced as falsification evidence.

---

## 109_biomedical_blindspot_audit.md

# #1277 Biomedical Blindspot Audit Gates

## Scope

#1277 through #1283 add a machine-readable blindspot audit after hypothesis
generation, mechanistic direction gating (#1269), and falsification. This stage
does not claim efficacy, safety, clinical actionability, treatment guidance,
dosing guidance, recommendation, pair-interaction proof, or cure evidence.

The audit closes the report-section blindspots that were still mostly prose:

- germline-versus-somatic context and synthetic-lethality inversion;
- drug lifecycle/viability for discontinued, withdrawn, failed, unavailable, or
  discredited molecules;
- external literature novelty and correlation to internal `novelty_score`;
- repeated-run/corpus/seed stability;
- benchmark-export readiness for Open Targets/Hetionet-style comparison;
- transcriptomic-reversal specificity for LINCS/CMap-style signatures.

## Implementation

Primary module:

```text
crates/calyx-cli/src/cmd/biomedical_blindspot_audit.rs
```

CLI:

```text
calyx biomedical-blindspot-audit \
  --hypotheses-report <typed/hunt report json> \
  --literature-audit <jsonl> \
  --stability-audit <jsonl> \
  --drug-lifecycle <jsonl> \
  --transcriptomic-audit <jsonl> \
  --out-dir <dir>
```

The command requires every source file. Missing files, malformed JSONL, or
source rows missing required identifiers fail closed with a structured
`CALYX_CLI_*` error. Biomedical failures do not disappear into stderr; they are
persisted as blocked or pending hypothesis rows with explicit reason codes.

Persisted outputs:

- `biomedical_blindspot_audit_report.json`
- `audited_hypotheses.jsonl`
- `ready_hypotheses.jsonl`
- `blocked_hypotheses.jsonl`
- `benchmark_export.jsonl`
- `metrics.json`

Representative reason codes:

| Code | Meaning |
|---|---|
| `CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK` | germline/constitutional disease plus synthetic-lethal cell-killing rationale |
| `CALYX_BLINDSPOT_DRUG_NOT_VIABLE` | drug lifecycle source says discontinued, withdrawn, terminated, suspended, failed, discredited, fraud-tainted, unavailable, or revoked |
| `CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING` | no external literature audit row matched the candidate |
| `CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING` | drug-disease candidate lacks patient/disease/variant-origin context |
| `CALYX_BLINDSPOT_REPRODUCIBILITY_LOW` | repeated-run frequency is below the configured stability threshold |
| `CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY` | transcriptomic reversal is generic mechanism-class signal rather than a specific reproducible signature |
| `CALYX_BLINDSPOT_TRANSCRIPTOMIC_NOT_REPRODUCIBLE_GOLD` | transcriptomic reversal lacks gold/reproducible/self-connected evidence |
| `CALYX_BLINDSPOT_BENCHMARK_FIELDS_MISSING` | row cannot be exported with at least disease plus target or drug fields |

## Source Research

The contract follows source-backed fields rather than free-text optimism:

- Open Targets direction-of-effect evidence separates direction on target and
  direction on trait.
- ChEMBL action and lifecycle fields distinguish positive/negative modulation,
  approved phase, and withdrawn/warning status.
- ClinGen dosage sensitivity separates haploinsufficiency and
  triplosensitivity mechanisms.
- PubMed/NCBI and Europe PMC are appropriate external literature-count sources
  for novelty audits, subject to their API/rate-limit contracts.
- LINCS/iLINCS only treats reproducible, self-connected (`gold`) signatures as
  strong transcriptomic evidence; generic mechanism-class reversals are weak
  priors.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\biomedical_blindspot_audit_final_20260707_180115
```

Manual FSV log:

```text
C:\code\Calyx-Dev\target\fsv\biomedical_blindspot_audit_final_20260707_180115\manual_fsv_log.txt
```

Source of truth:

```text
CLI persisted JSON/JSONL artifacts under each case directory. Verification
used separate file reads after command execution, not command return values.
The historical 726-row atlas JSONL was not present in this checkout; the docs
reference a prior `/home/croyse/...` vault path, so full 726-row
reclassification was not rerun here.
```

Happy-path persisted readback:

| Row | Status | Reason codes |
|---|---|---|
| `braf-trametinib-cfc` | `ready_for_human_review_after_blindspot_audit` | none |
| `olaparib-fanconi-brca2` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK` |
| `tarextumab-cadasil-notch3` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_DRUG_NOT_VIABLE` |
| `generic-hdac-reversal` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY`, `CALYX_BLINDSPOT_TRANSCRIPTOMIC_NOT_REPRODUCIBLE_GOLD` |

Happy-path counts:

```text
audited=4
ready=1
blocked=3
pending=0
benchmark_export_rows=4
```

Boundary and edge-case readbacks:

| Case | Expected outcome | Persisted proof |
|---|---|---|
| Missing patient/literature/stability context | pending, not ready | `blocked_hypotheses.jsonl` row has `pending_blindspot_evidence` plus `CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING`, `CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING`, `CALYX_BLINDSPOT_STABILITY_AUDIT_MISSING` |
| Low repeated-run stability | blocked | `blocked_hypotheses.jsonl` row has `CALYX_BLINDSPOT_REPRODUCIBILITY_LOW` and stability `0.333333333333333` |
| Malformed lifecycle source row | fail closed | command exit `2`, no report created, stderr JSON says `drug_lifecycle line 1 missing drug_name/name` |

Synapse readback independently reopened the final FSV tree and confirmed:

```text
audited=4
ready=1
blocked=3
pending=0
ready_ids=braf-trametinib-cfc
blocked_ids=generic-hdac-reversal,olaparib-fanconi-brca2,tarextumab-cadasil-notch3
```

## Verification Commands

```text
cargo fmt -p calyx-cli -- --check
cargo check -p calyx-cli
cargo test -p calyx-cli biomedical_blindspot -- --nocapture
cargo clippy -p calyx-cli --all-targets -- -D warnings
```

Manual FSV used the compiled `target\debug\calyx.exe` and then read back the
JSON/JSONL artifacts listed above.

## GitHub State

Issues completed by this entry:

- #1277 epic: biomedical blindspot hardening.
- #1278 germline-versus-somatic and synthetic-lethality context gate.
- #1279 drug viability and lifecycle gate.
- #1280 external literature novelty audit and novelty-score calibration metrics.
- #1281 reproducibility/stability and benchmark export audit surfaces.
- #1282 transcriptomic reversal specificity gate.
- #1283 final FSV and documentation.
- #1284 workspace dependency inheritance fix discovered during verification.

## Findings

- The root cause was that several skeptical-reviewer checks existed only in the
  paper/rejection prose, not as a required persisted contract.
- The fix adds a fail-closed audit surface after generation: missing source
  context becomes `pending_blindspot_evidence`; dangerous or contradicted
  context becomes `blocked_by_blindspot_audit`; only rows clearing every audit
  become `ready_for_human_review_after_blindspot_audit`.
- The audit also prevents generic class labels from being treated as concrete
  drug entities when an explicit drug name is present, avoiding spurious
  lifecycle misses on transcriptomic mechanism-class rows.

---

## index.md

# Medical Search — Discovery Findings Log (index)

This directory is the **append-only record of all biomedical association-mining work**: every
search, exploration, calibration, sweep, chain walk, and hypothesis is written here as a dated
`.md` file. Nothing about the discovery program lives only in chat or in a GitHub comment — it lands
here, traceably.

- **Strategy / plan:** [`../../docs2/BIOMEDICAL_DISCOVERY_STRATEGY.md`](../../docs2/BIOMEDICAL_DISCOVERY_STRATEGY.md)
- **Association-native doctrine:** [`../CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md`](../CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md)
- **Recording rule:** one file per atomic task (named `NN_<topic>.md`), using `_TEMPLATE.md`.
  Record what was run, the exact commands, the raw outputs/FSV evidence, and the honest conclusion
  (including refusals + per-sensor deficits — a refusal is a finding, not a failure).
- **GitHub:** every file here corresponds to a `[DISCOVERY]` issue; cross-link both ways.

## Honesty contract (binding)
Only record a result as "grounded" if it cleared the Calyx honesty gate
(`I(panel;outcome) ≥ H(outcome)`). Never assert a capability or a finding that was not actually
run and verified against stored artifacts. A discovered association is a **ranked, traceable
hypothesis**, never a verdict — it carries its full provenance chain and a sufficiency proof, and
still requires experimental confirmation.

## Files
| File | Task / topic | Status |
|---|---|---|
| `_TEMPLATE.md` | findings template | — |
| `01_anchors_at_ingest.md` | #868 anchors-at-ingest: thread typed anchors through streaming ingest | ✅ done + FSV |
| `02_anchored_reingest.md` | #869 anchored re-ingest of ~199k clinical-QA corpus (dedup + resume) | ✅ done + FSV (198,993 cx, chain ok) |
| `05_degraded_flag_fix.md` | #872 degraded flag ignores retrieval-only temporal sidecar absence | done + FSV |
| `06_calibration_fsv.md` | #873 planted signal calibration and gate readback | done + FSV |
| `07_power_gate_verify.md` | #874 assay power-calibration and entropy-floor gate verification | done + FSV |
| `08_blind_spot_sweep.md` | #875 blind-spot sweep/ranking log | implementation slice + synthetic FSV; corpus sweep pending |
| `09_domain_bridges.md` | #876 domain-pair bridge B-term ranking report | done + real clinical-corpus FSV; non-clinical corpus materialization tracked separately |
| `10_spectral_communities.md` | #877 spectral community and inter-community bridge report | done + real clinical-corpus FSV |
| `11_discovery_harness.md` | #878 gated discovery chain harness | done + real clinical-corpus 100-hop FSV |
| `12_probe_matrix.md` | #879 physical probe matrix harness and productive-combination log | done + real physical-vault FSV; large-corpus blockers split to #1000/#1001 |
| `13_chain_walks_synthetic.md` | #880 grounded chain-walk report for static/operator seeds | synthetic FSV + real clinical-corpus FSV; final repo gate pending |
| `13_chain_walks_spectral-bridge-1-src.md` | #880 real chain walk seed `spectral-bridge-1-src` | real corpus per-seed readback |
| `13_chain_walks_spectral-bridge-2-src.md` | #880 real chain walk seed `spectral-bridge-2-src` | real corpus per-seed readback |
| `13_chain_walks_spectral-bridge-3-src.md` | #880 real chain walk seed `spectral-bridge-3-src` | real corpus per-seed readback |
| `13_chain_walks_spectral-bridge-4-src.md` | #880 real chain walk seed `spectral-bridge-4-src` | real corpus per-seed readback |
| `13_chain_walks_operator-centrality-1.md` | #880 real chain walk seed `operator-centrality-1` | real corpus per-seed readback |
| `13_chain_walks_operator-centrality-2.md` | #880 real chain walk seed `operator-centrality-2` | real corpus per-seed readback |
| `14_hypothesis_evaluation.md` | #881 transparent multi-prompt hypothesis evaluation report | done + real GitHub Models evaluator FSV |
| `15_ranked_hypotheses.md` | #882 ranked traceable hypothesis list | done + real #881 evaluator ranking FSV |
| `16_refusal_driven_expansion.md` | #883 refusal-expansion planner and before/after verifier | done + real regrounding FSV |
| `17_discovery_vault_molecular.md` | #884 molecular discovery vault source-data preflight | preflight + FSV |
| `18_oracle_event_structuring.md` | #885 Oracle event/domain structuring for recurrence reverse-query | done + FSV |
| `19_nonclinical_bridge_corpora.md` | #994 real non-clinical bridge corpus materialization | done + molecular bridge FSV |
| `20_association_evidence_index.md` | #867 consolidated association evidence index | current result map + derived issue tree |
| `21_association_result_pack.md` | #1170 machine-readable association result pack | complete FSV |
| `22_cxid_source_expansion.md` | #1171 source expansion for association CxIds | complete FSV for current association-result surfaces |
| `23_biomedical_concept_normalization.md` | #1172 concept normalization for expanded evidence rows | bounded first-pass FSV + complete unresolved accounting |
| `24_typed_biomedical_overlay_graph.md` | #1173 typed biomedical overlay graph | complete FSV for first typed graph |
| `25_open_targets_validation_ingest.md` | #1174 Open Targets target-disease validation ingest | complete bounded FSV against Open Targets 26.06 |
| `26_molecular_vault_scaleout.md` | #1175 ChEMBL/BindingDB molecular vault scaleout | complete FSV for 53-row measured molecular/clinical vault |
| `27_pubtator_pubmed_relation_validation.md` | #1176 PubTator/PubMed relation validation | complete bounded FSV for 18 evidence-backed seed edges |
| `28_clinicaltrials_validation_ingest.md` | #1177 ClinicalTrials.gov intervention-condition evidence | complete bounded FSV for 13 trial-readiness seeds |
| `29_dgidb_drug_gene_validation.md` | #1178 DGIdb drug-gene and druggability evidence | complete bounded FSV for source-backed drug-target triage |
| `30_evidence_outcome_instrument_association_substrate.md` | #1196 Calyx DB evidence/outcome/instrument association substrate | complete FSV for accepted Aster Graph CF collection |
| `31_lincs_cmap_reversal_screen.md` | #1179 LINCS/CMap transcriptomic reversal screen | complete FSV for accepted Aster Graph CF collection |
| `32_lincs_perturbation_metadata_mapping.md` | #1199 LINCS/CMap perturbation metadata mapping | complete FSV for accepted Aster Graph CF collection |
| `33_graph_collection_lifecycle_cleanup.md` | #1197 graph collection lifecycle cleanup | complete FSV for accepted/tombstoned graph generations |
| `34_edge_range_readback.md` | #1198 PlainGraph edge range readback | complete FSV for all-edge collection-local readback |
| `37_metabolic_cardiovascular_hunt.md` | #1186 metabolic/cardiovascular drug-repurposing association hunt | complete FSV for typed graph, DGIdb/Open Targets context, safety flags, trial flags, and ranked hypothesis rows |
| `38_neuro_hunt.md` | #1187 neurodegeneration/neuropsychiatric association hunt | complete FSV for ranked neuro hypotheses, normalized evidence, Open Targets/DGIdb context, safety/trial flags, and explicit falsification status |
| `39_infectious_immunology_hunt.md` | #1188 infectious/immunology/inflammation association hunt | complete FSV for ranked infectious/immunology hypotheses, normalized evidence, family labels, Open Targets/DGIdb context, safety/trial flags, and explicit falsification status |
| `40_rare_disease_hunt.md` | #1189 rare-disease phenotype/gene/drug association hunt | complete FSV for HPO/Mondo source hashes, phenotype/gene links, ranked drug-target hypotheses, uncertainty rows, and native Calyx bridge-corpus materialization |
| `42_graph_csr_traversal_cache.md` | #1191 graph CSR/traversal cache for large readers | complete FSV for large #869 CSR readback and reader preference |
| `43_probe_matrix_scale_repair.md` | #1192 probe-matrix scale repair and real-vault readback | complete FSV for manifest repair, explicit stale policy, and real large-vault probe run |
| `44_gpu_sparse_association_acceleration.md` | #1194 GPU/sparse association acceleration decision | complete FSV for real #869 CSR input readback, CPU spectral profile, GPU telemetry, no-GPU decision, and fail-closed invalid-parameter case |
| `44_association_validation_gates.md` | #1182 known-positive/negative and time-split validation gates | complete FSV for fail-closed strict-threshold calibration and passing real-source gate |
| `45_all_pair_typed_association_miner.md` | #1183 bounded all-pair typed association miner | complete FSV for broad, chemical/disease, and gene/disease typed-pair scans |
| `46_hypothesis_falsification_sweep.md` | #1184 counter-evidence and falsification sweep | complete FSV for retained #1183 hypotheses and persisted source readback |
| `47_precision_oncology_validation.md` | #1180 precision-oncology validation sources | complete FSV for CIViC source bytes, parsed rows, mapped rows, and unresolved accounting |
| `48_oncology_deep_hunt.md` | #1185 oncology deep association hunt | complete FSV for oncology hypothesis atlas, source hashes, falsification state, and safety-trial flags |
| `49_drug_safety_triage.md` | #1181 drug safety/adverse-event triage | complete FSV for FDA label/event source readback, parsed safety rows, mapped candidate safety, and ranker block flags |
| `50_oracle_honesty_ci_low_gate.md` | #1204 Oracle honesty gate CI-low/calibration hardening | complete FSV for calibrated lower-bound sufficiency and fail-closed missing calibration |
| `51_discovery_chain_sufficiency_gate.md` | #1205 discovery-chain/chain-walk sufficiency gate hardening | complete FSV for calibrated assay-backed gates, missing/insufficient assay refusal, and accepted-hop evidence readback |
| `52_falsification_asserted_relation_gate.md` | #1206 falsification asserted-relation matching | complete FSV for structured endpoint matching, skipped unstructured rows, and real-source relation readback |
| `53_batch_ingest_provenance_gate.md` | #1211 batch-ingest source provenance gate | complete FSV for parser-preflight fail-closed enforcement and Base CF metadata readback |
| `54_oracle_event_fsv_snapshot_gate.md` | #1215 oracle-event FSV snapshot readback | complete FSV for current-snapshot Recurrence CF artifact write and broad batch FSV run |
| `55_ksg_mixed_discrete_estimator.md` | #1207 mixed continuous-discrete KSG estimator | complete FSV for planted-signal recovery, fail-closed small classes, and real labeled Assay CF readback |
| `56_ksg_subsample_ci.md` | #1208 KSG no-replacement subsample CI | complete FSV for duplicate-free subsamples, independent-control lower bound, planted-signal coverage, and small-sample fail-closed edge |
| `57_blind_spot_calibration.md` | #1209 blind-spot detector calibration | complete FSV for scale-normalized per-pair calibration, null-FDR control, and uncalibrated-pair fail-closed skip |
| `58_weighted_graph_csr.md` | #1213 weighted graph CSR evidence edges | complete FSV for persisted CSR weights, weighted reach/betweenness/spectral scoring, and invalid edge-value fail-closed cases |
| `60_hypothesis_evaluator_driver.md` | #1201 hypothesis evaluator driver | complete FSV for persisted evaluator runs, citation validation, aggregate readback, and fail-closed endpoint/malformed/citation cases |
| `62_discovery_manifest_redaction.md` | #1221 discovery-run manifest ledger redaction | complete FSV for long benign manifest provenance and true-secret rejection |
| `63_native_discovery_bridges.md` | #1220 native discovery bridge CLIs | complete FSV for real #1219 miner/falsification artifacts, downstream evaluator/ranker acceptance, and stale preflight fail-closed behavior |
| `64_hypothesis_evaluator_https_provider.md` | #1216 native HTTPS/OpenAI-compatible evaluator provider | complete FSV for HTTPS auth/redaction behavior, persisted evaluator output readback, and missing-auth fail-closed no-artifact proof |
| `65_association_native_doctrine_context_update.md` | #863/#860 association-native doctrine/context update | complete context update folding the builder handbook into repo doctrine and issue-state obligations |
| `66_binary_csr_persistence.md` | #1210 binary CSR persistence for PlainGraph | complete FSV for binary CSR physical readback, size reduction versus JSON, segmentation, and corruption fail-closed cases |
| `67_novelty_calibration_split.md` | #1226 novelty/calibration split for disease-hunt rankings | complete FSV for preserving original disease-hunt rows while splitting calibration/known-positive proof rows from novelty-prioritized research leads |
| `68_native_novelty_calibration_split.md` | #1227 native novelty/calibration splitter stage | complete FSV for native sealed-input splitter parity with #1226 plus stale-manifest fail-closed proof |
| `69_infectious_normalization_repair.md` | #1225 infectious/immunology concept normalization repair | complete FSV for deterministic high-value mappings, before/after unresolved accounting, required row examples, and new co-mention coverage |
| `70_neuro_normalization_repair.md` | #1222 neuro concept normalization repair | complete FSV for deterministic neuro mappings, typed overlay delta, and #1187 rerun count/hash deltas |
| `71_neuro_druggability_expansion.md` | #1224 neuropsychiatric target druggability expansion | complete FSV for target list, live DGIdb/ChEMBL source hashes, BindingDB accession scan, bridge rows, and explicit no-hit rows |
| `72_generated_candidate_falsification_sweep.md` | #1223 generated disease-hunt candidate falsification sweep | complete FSV for one flag per generated candidate, support/counter evidence, fail-closed safety/trial blocks, and native Calyx bridge-corpus materialization |
| `73_human_review_biomedical_hypothesis_atlas.md` | #1193 human-review biomedical hypothesis atlas | complete FSV for 1,877 hypothesis-only atlas rows, filters, evidence bundles, review statuses, and native Calyx bridge-corpus materialization |
| `74_drug_combination_hypotheses.md` | #1190 drug-combination and synergy hypothesis miner | complete FSV for 1,750 fail-closed combination rows, DrugComb exact-pair matches, safety/interaction flags, and native Calyx bridge-corpus materialization |
| `75_nci_almanac_external_synergy.md` | #1229 NCI ALMANAC external combination evidence ingest | complete FSV for ALMANAC source bytes, 306,365 atomic cell-line combo-score rows, 1,750 candidate joins, and native Calyx bridge-corpus materialization |
| `76_external_combo_source_expansion.md` | #1231 CDCDB external combination source expansion | complete FSV for CDCDB source bytes, schema fingerprint, 1,750 candidate rechecks, 1,682 prior no-hit rechecks, and native Calyx bridge-corpus materialization |
| `77_clinicaltrials_current_recheck.md` | #1232 ClinicalTrials.gov current direct source recheck | complete FSV for 1,546 remaining no-hit row queries, 1,618 API pages, 204 registry hits, and native Calyx bridge-corpus materialization |
| `78_fda_pubmed_source_mining.md` | #1234 FDA/PubMed source mining after ClinicalTrials.gov | complete FSV for FDA Orange Book/NDC source bytes, PubMed ESearch/ESummary rows, 301 literature co-mention hits, and native Calyx bridge-corpus materialization |
| `79_pubmed_source_text_validation.md` | #1237 PubMed source-text validation for #1234 co-mention hits | complete FSV for PubMed EFetch source records, 568 evidence validation rows, 301 candidate rollups, and native Calyx bridge-corpus materialization |
| `80_pubmed_structured_extraction.md` | #1238 PubMed structured relation/safety/outcome extraction | complete FSV for 510 structured extraction rows, 301 candidate rollups, blocked counter-evidence preservation, and native Calyx bridge-corpus materialization |
| `81_openfda_label_source_mining.md` | #1236 openFDA Human Drug Label source mining after FDA/PubMed recheck | complete FSV for 649 openFDA label queries, 40 label evidence rows, 22 candidate-row hits, 1,019 remaining no-hit rows, and native Calyx bridge-corpus materialization |
| `82_rxnorm_combination_product_mining.md` | #1240 RxNorm combination-product source mining after openFDA label recheck | complete FSV for 640 RxNorm pair-key query rows, zero concept hits, 1,019 remaining no-hit rows, and native Calyx bridge-corpus materialization |
| `83_dailymed_spl_title_source_mining.md` | #1242 DailyMed SPL title source mining after RxNorm recheck | complete FSV for 640 DailyMed pair-key query rows, zero SPL metadata hits, 1,019 remaining no-hit rows, and native Calyx bridge-corpus materialization |
| `84_europepmc_pair_search_mining.md` | #1243 Europe PMC pair-search source mining after DailyMed SPL title recheck | complete FSV for 640 Europe PMC pair-key queries, 598 verified co-mention evidence rows, 487 candidate-row hits, 532 remaining no-hit rows, and native Calyx bridge-corpus materialization |
| `85_europepmc_relation_validation.md` | #1244 Europe PMC source-text relation/safety/outcome validation | complete FSV for 598 evidence rows, 487 hit-candidate rollups, bounded relation classes, blocked safety/counter review rows, and native Calyx bridge-corpus materialization |
| `86_europepmc_safety_counter_review.md` | #1246 Europe PMC safety/counter falsification review | complete FSV for 363 scoped rollups, 310 evidence-review rows, safety/counter category separation, and native Calyx bridge-corpus materialization |
| `87_openfda_independent_safety_validation.md` | #1248 openFDA independent safety-source validation for Europe PMC safety/counter rollups | complete FSV for 202 pair-key openFDA label queries, 363 blocked no-hit rollup statuses, and native Calyx bridge-corpus materialization |
| `88_europepmc_endpoint_outcome_review.md` | #1247 Europe PMC endpoint/outcome review for relation-context rollups | complete FSV for 108 scoped rollups, 107 evidence-review rows, endpoint/outcome category separation, blocked validation status, and native Calyx bridge-corpus materialization |
| `89_clinicaltrials_endpoint_validation.md` | #1250 ClinicalTrials.gov endpoint validation for Europe PMC relation-context rollups | complete FSV for 72 pair-key registry queries, 108 blocked no-hit rollup statuses, and native Calyx bridge-corpus materialization |
| `90_europepmc_source_local_endpoint_expansion.md` | #1251 Europe PMC source-local endpoint expansion after ClinicalTrials no-hit | complete FSV for 107 bounded source-window reviews, 35 source-local endpoint rollups, blocked effect-validation status, and native Calyx bridge-corpus materialization |
| `91_effect_result_falsification_gate.md` | #1252 effect-result and falsification gate for source-local endpoint rollups | complete FSV for 33 source-local endpoint evidence reviews, 35 blocked rollup statuses, and native Calyx bridge-corpus materialization |
| `92_independent_effect_result_validation.md` | #1253 independent validation for #1252 effect-result candidates | complete FSV for 51 independent source queries, 24 blocked no-hit candidate rollups, and native Calyx bridge-corpus materialization |
| `93_openfda_faers_safety_expansion.md` | #1249 openFDA FAERS safety-source expansion after label no-hit | complete FSV for 202 FAERS pair queries, 45 adverse-event co-report evidence rows, 363 blocked rollup statuses, and native Calyx bridge-corpus materialization |
| `94_pubchem_synonym_source_mining.md` | #1245 PubChem synonym/equivalence source mining after Europe PMC no-hit | complete FSV for 152 PubChem term queries, zero pair-equivalence hits, 532 blocked candidate statuses, and native Calyx bridge-corpus materialization |
| `95_chembl_source_mining.md` | #1254 ChEMBL source mining after PubChem no-hit | complete FSV for 353 ChEMBL molecule-search queries, zero same-record pair hits, 532 blocked candidate statuses, and native Calyx bridge-corpus materialization |
| `96_drugcentral_source_mining.md` | #1255 DrugCentral source mining after ChEMBL no-hit | complete FSV for DrugCentral table snapshots, 7,621 DDI rows, zero structured pair hits, 532 blocked candidate statuses, and native Calyx bridge-corpus materialization |
| `97_pharmgkb_source_mining.md` | #1256 PharmGKB source mining after DrugCentral no-hit | complete FSV for PharmGKB/ClinPGx source docs, 7 downloaded archives, 11 TSV tables, zero same-row two-term hits, 532 blocked candidate statuses, and native Calyx bridge-corpus materialization |
| `98_nsides_source_mining.md` | #1257 nSIDES TwoSIDES/OffSIDES source mining after PharmGKB no-hit | complete FSV for 42,920,391 TwoSIDES rows, 3,206,558 OffSIDES rows, zero pair-level hits, 157 single-drug safety-context pair statuses, and native Calyx bridge-corpus materialization |
| `99_rxnorm_canonicalization.md` | #1258 RxNorm/RxNav canonicalization for nSIDES no-map remainder | complete FSV for 152 RxNav term queries, 376 persisted API responses, 757 TwoSIDES RxCUI pair adverse-effect rows, 1,088 OffSIDES context rows, and native Calyx bridge-corpus materialization |
| `100_rxnorm_twosides_safety_validation.md` | #1259 independent validation for RxNorm-rescued TwoSIDES pair safety hits | complete FSV for 7 pair rollups, 35 independent query rows, 1 serious FAERS co-report blocker, duplicate RxCUI-overlap grouping, and native Calyx bridge-corpus materialization |
| `101_metformin_trametinib_faers_case_validation.md` | #1260 metformin-trametinib FAERS case-level validation | complete FSV for exact FAERS case readback, confounded serious safety blocker classification, label/literature context rows, and native Calyx bridge-corpus materialization |
| `102_faers_case_quality_ranker_overlay.md` | #1261 FAERS case-quality/confounder overlay for combination ranker | complete FSV for 5 metformin/trametinib-family ranker overlays, 4 direct #1260 joins, 1 ingredient-family context row, and native Calyx bridge-corpus materialization |
| `103_openfda_label_gate_validation.md` | #1241 openFDA label-hit safety/interaction gate validation | complete FSV for 40 label evidence gate rows, 9 pair rollups, 22 blocked candidate statuses, and native Calyx bridge-corpus materialization |
| `104_pubmed_structured_gate_validation.md` | #1239 PubMed structured gate validation for #1238 rows | complete FSV for 510 PubMed structured gate rows, 301 candidate statuses, 1,505 missing/not-cleared gate rows, and native Calyx bridge-corpus materialization |
| `105_clinicaltrials_gate_validation.md` | #1235 ClinicalTrials.gov hit trial-context and gate validation | complete FSV for 204 registry-hit statuses, 1,192 trial-context rows, 816 missing/not-cleared gate rows, and native Calyx bridge-corpus materialization |
| `106_cdcdb_gate_validation.md` | #1233 CDCDB-supported combination source-context and gate validation | complete FSV for 173 CDCDB hit statuses, 611 source-context rows, 692 missing/not-cleared gate rows, and native Calyx bridge-corpus materialization |
| `107_safety_interaction_coverage_rollup.md` | #1228 aggregate component-safety and pair-interaction coverage rollup | complete FSV for 57 sealed source inputs, 277 component-drug coverage rows, 1,750 pair coverage rows, 1,750 pair gap rows, and native Calyx bridge-corpus materialization |
| `108_target_match_fallacy_direction_gate.md` | #1269 source-backed mechanistic direction gates for target-disease and drug-target evidence | complete FSV for Open Targets direction-of-effect gating, ChEMBL/DGIdb action normalization, mutation/dosage mechanism screening, conflict counter-evidence, and fail-closed blocked artifacts |
| `109_biomedical_blindspot_audit.md` | #1277-#1283 biomedical blindspot audit gates | complete FSV for germline/somatic context, drug lifecycle, literature novelty, stability, benchmark export, transcriptomic specificity, and fail-closed malformed-source behavior |
| (added as work proceeds) | | |

---

