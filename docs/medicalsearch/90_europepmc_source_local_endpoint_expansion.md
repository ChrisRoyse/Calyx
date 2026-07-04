# #1251 Europe PMC Source-Local Endpoint Expansion

Status: complete for the Europe PMC/PMC bounded-window pass.

This slice continued endpoint-source validation after the #1250
ClinicalTrials.gov no-hit by rereading the sealed #1247 Europe PMC/PMC source
windows. It required both candidate terms to appear in the same bounded source
window before emitting source-local endpoint context. All rows remain blocked
pending independent effect-result validation, safety/falsification, and human
review.

Clinical boundary:

```text
Europe PMC source-local endpoint expansion is source-text triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z/out/clinicaltrials_endpoint_rollup_status.jsonl
sha256: 7cc7fd9b1ed55a41dcd02ccca37dd72647dae877f524c0c02d049d6771425b84

/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z/out/clinicaltrials_endpoint_pair_status.jsonl
sha256: bfd61154e643adb9c852f5c0ee031db018e6986026ec400863784d677f1806c8

/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/europepmc_endpoint_outcome_evidence_review.jsonl
sha256: edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64
```

Source contract:

- Input #1250 rollups: 108.
- Input #1250 pair statuses: 72.
- Input #1247 evidence windows: 107.
- A source-local endpoint hit required both candidate terms in the same bounded
  `source_text_window` plus endpoint/outcome/PK-exposure context.
- Whole-article co-occurrence or one-term windows remained blocked.
- Direction, comparator, magnitude, and cohort/model strings are review fields
  only, not validated results.

## Method

The extractor:

- emitted one evidence-review row for every #1247 evidence window in the #1250
  pair-key scope;
- verified exact and normalized pair-term presence inside each bounded source
  window;
- separated source-local endpoint context, source-local pair context without
  endpoint language, and non-source-local windows;
- extracted endpoint type, effect-direction language, magnitude strings,
  comparator strings, cohort/model strings, mechanism, PK/exposure, dose, and
  trial-design strings;
- emitted one status row per #1250 rollup;
- kept every row blocked pending independent effect-result validation,
  safety/falsification, and human review;
- wrote a 215-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_source_local_endpoint_evidence_review.jsonl` | 107 | 662,973 | `c26882e64fc669d458f132684eab28df0ee15470a399c52982a9bf34b5b15154` |
| `europepmc_source_local_endpoint_rollup_status.jsonl` | 108 | 223,982 | `5af58d0b05c2fb03374ff0d74f3fceb300a8c40def32f455be8478a5d241f7c4` |
| `europepmc_source_local_endpoint_bridge_rows.jsonl` | 215 | 324,864 | `69bca1a9419fd6bce8a18ab7aae6d1aa221d4ce828725671ff912ed602679655` |
| `input_manifest.json` | - | 2,784 | `e614bdae68dacd263467c0e4d653c2b5093f61078293d2569d26242e2747fdef` |
| `validation_metrics.json` | - | 19,087 | `1fa89b31e86130f707bb20509990a7594fc36c1a036473843b616ea816874f34` |
| `output_manifest.json` | - | 1,879 | `626f50c4a6e6a79ac520602d83ccfcfdcda2e094471ff58c2d953ad03d04925c` |
| `persisted_readback.json` | - | 2,907 | `d57506da4b7e0d718d7f6866a1a156169e7be7cbddc8e870be5f001f693d4032` |
| `calyx_bridge_corpus_stdout.json` | - | 774 | `19c400979bd91af72283b112b8e3eb4014a2e810ec492cfab0ea778a4064410d` |
| `calyx_bridge_corpus_readback.json` | - | 5,337 | `c341675c0311714c69fa8383533cb6b56d772f43eaf6baae6a183f68bc760025` |

## Metrics

| Metric | Count |
|---|---:|
| #1250 rollups checked | 108 |
| #1250 pair statuses | 72 |
| Evidence-review rows | 107 |
| Rollup-status rows | 108 |
| Source-local pair-context evidence rows | 43 |
| Source-local endpoint-context evidence rows | 33 |
| Rollups with source-local pair context | 47 |
| Rollups with source-local endpoint context | 35 |
| Endpoint evidence rows with magnitude language | 9 |
| Endpoint evidence rows with comparator language | 12 |

Evidence status counts:

| Status | Evidence rows |
|---|---:|
| `europepmc_source_local_endpoint_context_hit_still_blocked` | 33 |
| `europepmc_source_local_pair_context_mechanistic_or_pk_only_still_blocked` | 10 |
| `europepmc_source_window_not_pair_local_still_blocked` | 64 |

Rollup status counts:

| Status | Rollups |
|---|---:|
| `europepmc_source_local_endpoint_context_hit_still_blocked` | 35 |
| `europepmc_source_local_pair_context_without_endpoint_still_blocked` | 12 |
| `europepmc_no_source_local_pair_endpoint_hit_still_blocked` | 61 |

Endpoint type counts:

| Endpoint type | Evidence rows |
|---|---:|
| `clinical_endpoint_language` | 23 |
| `endpoint_or_outcome_language` | 8 |
| `pharmacokinetic_or_exposure_language` | 1 |
| `preclinical_or_cell_endpoint_language` | 1 |

Effect-direction language counts:

| Direction label | Evidence rows |
|---|---:|
| `benefit_or_improvement_language` | 19 |
| `mixed_or_ambiguous_effect_language` | 2 |
| `no_explicit_effect_direction_language` | 9 |
| `worsening_progression_or_increase_language` | 3 |

Primary model-system counts for endpoint-context evidence:

| Model/system | Evidence rows |
|---|---:|
| `human_clinical_or_patient` | 30 |
| `in_vitro_or_cell_system` | 2 |
| `unclear` | 1 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1250 persisted readback all true | true |
| #1250 Calyx readback all true | true |
| Rollup status for every #1250 rollup | true |
| Evidence rows cover pair-status keys | true |
| Endpoint rollup keys have endpoint evidence | true |
| Evidence statuses allowed | true |
| Rollup statuses allowed | true |
| Source-local endpoint evidence has both terms in bounded window | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1251-europepmc-source-local-endpoint-expansion-20260704t234000z
vault_id: 01KWQBKV63T2EPVWKJS8AJMVM0
vault_dir: /home/croyse/calyx/vaults/01KWQBKV63T2EPVWKJS8AJMVM0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 215 |
| Bridge terms | 342 |
| Graph nodes | 557 |
| Graph edges | 2,148 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1251 identified 35 #1250 rollups with bounded Europe PMC/PMC source-window
endpoint context where both candidate terms appear in the same source window.
Nine endpoint-context evidence rows carry magnitude strings and 12 carry
comparator language. These are source-text review fields only; none are
validated effect results or clinical claims.

No efficacy claim, safety claim, treatment guidance, recommendation, clinical
actionability, dosing guidance, or cure claim is made.
