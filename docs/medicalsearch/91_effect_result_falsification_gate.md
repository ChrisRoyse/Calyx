# #1252 Effect-Result Falsification Gate

Status: complete for the source-local effect-result/falsification extraction
pass.

This slice continued the #1251 Europe PMC/PMC bounded-window endpoint pass by
extracting structured effect-result, magnitude, comparator, safety, and counter
language from the source-local endpoint rows. All rows remain blocked pending
independent effect-result validation, safety/falsification review, and human
review.

Clinical boundary:

```text
Effect-result and falsification extraction is source-text triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/europepmc_source_local_endpoint_rollup_status.jsonl
sha256: 5af58d0b05c2fb03374ff0d74f3fceb300a8c40def32f455be8478a5d241f7c4

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/europepmc_source_local_endpoint_evidence_review.jsonl
sha256: c26882e64fc669d458f132684eab28df0ee15470a399c52982a9bf34b5b15154

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/persisted_readback.json
sha256: d57506da4b7e0d718d7f6866a1a156169e7be7cbddc8e870be5f001f693d4032

/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/calyx_bridge_corpus_readback.json
sha256: c341675c0311714c69fa8383533cb6b56d772f43eaf6baae6a183f68bc760025
```

Source contract:

- Input scope was the 35 #1251 rollups with
  `europepmc_source_local_endpoint_context_hit_still_blocked`.
- Evidence scope was the 33 #1251 source-local endpoint evidence rows.
- Every evidence row had to preserve pair-term verification from the bounded
  `source_text_window`.
- Result extraction is lexical triage only; direction, magnitude, comparator,
  safety, and counter strings are not validated endpoint results.
- Every row carries the clinical boundary and remains blocked.

## Method

The extractor:

- loaded the sealed #1251 rollup/evidence rows and upstream readbacks;
- retained only the #1251 source-local endpoint rows;
- classified evidence windows into result candidates with magnitude and
  direction, direction-only candidates, counter/safety-blocked candidates, or
  endpoint context without result assertion;
- emitted one rollup status row for every scoped rollup;
- preserved safety/counter language as blocking evidence;
- wrote a 68-row bridge-corpus slice for native Calyx materialization;
- verified all persisted rows and bridge rows by readback.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `effect_result_evidence_review.jsonl` | 33 | 164,687 | `faa3c1527ed9e79ecb53841e0a10cada8d0c2b1462c449cc2f2b0dbba453095c` |
| `effect_result_rollup_status.jsonl` | 35 | 67,008 | `43c11b2c653ff0772e6ef382ceafa6b1a84d5a10debba491586fc8107309a8b1` |
| `effect_result_bridge_rows.jsonl` | 68 | 93,327 | `ab4ec55c480e5fe65c61b5e2ae1202744b8b83909c567bca52d1f9b5425e42ce` |
| `input_manifest.json` | - | 2,206 | `d0417b8f8f9707c54e06eb8a139c3105fe73202090361460a04a585caa3d1910` |
| `validation_metrics.json` | - | 18,034 | `76fae92687a0abd10343ea87adcf772efe91a0fb6f248d22e8949fb77827fecd` |
| `output_manifest.json` | - | 1,721 | `6e4968b909fa78b4722a5fa8f47c926a7156dd6f78781acd4197c74d2edec2ad` |
| `persisted_readback.json` | - | 2,632 | `4a9e67714abb8fae8f26b021f5d4829cfd2a3aba4b76cd5777c184215116cf3e` |
| `calyx_bridge_corpus_stdout.json` | - | 724 | `0c4433c226ddaacf6896681395829912188d51db21e275c907e2b8456d12a201` |
| `calyx_bridge_corpus_readback.json` | - | 5,110 | `cd80e2ebcfc7819b6a1322a96a0560464bf14b614d04ef28a6ce23ca8a898756` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped source-local endpoint rollups | 35 |
| Result evidence rows | 33 |
| Rollup status rows | 35 |
| Evidence rows with direction and magnitude | 7 |
| Evidence rows with direction only | 11 |
| Evidence rows with comparator language | 12 |
| Evidence rows with safety/counter language | 1 |
| Rollups with direction and magnitude | 9 |
| Rollups with safety/counter block | 1 |

Evidence status counts:

| Status | Evidence rows |
|---|---:|
| `counter_or_safety_language_blocks_effect_result_candidate` | 1 |
| `effect_result_candidate_direction_only_still_blocked` | 11 |
| `effect_result_candidate_with_magnitude_and_direction_still_blocked` | 7 |
| `endpoint_context_without_result_assertion_still_blocked` | 14 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `counter_or_safety_language_blocks_rollup` | 1 |
| `effect_result_candidate_direction_only_still_blocked` | 15 |
| `effect_result_candidate_with_magnitude_and_direction_still_blocked` | 9 |
| `endpoint_context_without_result_assertion_still_blocked` | 10 |

Primary model-system counts for endpoint-context evidence:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 30 |
| `in_vitro_or_cell_system` | 2 |
| `unclear` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1251 persisted readback all true | true |
| #1251 Calyx readback all true | true |
| Rollup status for every scoped rollup | true |
| Result rows cover status pair keys | true |
| Evidence statuses allowed | true |
| Rollup statuses allowed | true |
| All result rows have pair-term verification | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1252-effect-result-falsification-gate-20260704t235000z
vault_id: 01KWQC80E88YJSQZ52MNXKQKPG
vault_dir: /home/croyse/calyx/vaults/01KWQC80E88YJSQZ52MNXKQKPG
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 68 |
| Bridge terms | 129 |
| Graph nodes | 197 |
| Graph edges | 676 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1252 produced a structured effect-result/falsification triage layer for the 35
#1251 source-local endpoint rollups. Nine rollups have direction plus magnitude
language and 15 have direction-only language, but every row remains blocked
because source-window text is not an independent endpoint-result, safety,
falsification, or human-review gate.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.
