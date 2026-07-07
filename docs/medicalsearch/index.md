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
