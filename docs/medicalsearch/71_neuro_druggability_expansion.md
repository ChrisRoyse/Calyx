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
