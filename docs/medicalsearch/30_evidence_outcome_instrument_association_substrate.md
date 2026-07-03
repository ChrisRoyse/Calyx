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
