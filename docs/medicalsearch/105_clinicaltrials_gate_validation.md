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
