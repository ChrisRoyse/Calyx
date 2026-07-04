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
