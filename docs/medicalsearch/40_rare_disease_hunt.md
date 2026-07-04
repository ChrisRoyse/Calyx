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
