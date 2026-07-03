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
