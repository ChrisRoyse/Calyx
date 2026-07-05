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
