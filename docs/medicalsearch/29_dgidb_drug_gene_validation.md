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
