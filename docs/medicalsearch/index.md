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
| `42_graph_csr_traversal_cache.md` | #1191 graph CSR/traversal cache for large readers | complete FSV for large #869 CSR readback and reader preference |
| `43_probe_matrix_scale_repair.md` | #1192 probe-matrix scale repair and real-vault readback | complete FSV for manifest repair, explicit stale policy, and real large-vault probe run |
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
| (added as work proceeds) | | |
