# #1250 ClinicalTrials.gov Endpoint Validation

Status: complete for the ClinicalTrials.gov registry pass.

This slice checked the #1247 Europe PMC endpoint/outcome review rollups against
the current ClinicalTrials.gov v2 API as an independent registry endpoint
source. A registry hit required both candidate terms in the same study
intervention text; an endpoint hit additionally required protocol or results
outcome fields in the matched study. No pair cleared that source gate.

Clinical boundary:

```text
ClinicalTrials.gov endpoint validation is registry documentation triage only; not efficacy, safety, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/candidate_europepmc_endpoint_outcome_rollup.jsonl
sha256: d13703c71a7e1dedf595ee4b21fd0fbf5535d33c7487bebfd3717426e9aea90c

/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/europepmc_endpoint_outcome_evidence_review.jsonl
sha256: edfc474cbe8b98a3bb3a17452a7a34cca7d67ae34cd4b810e609fbb9c4c74f64
```

Source contract:

- Input rollups: 108.
- Unique pair keys queried: 72.
- Source: current ClinicalTrials.gov v2 API.
- Query mode: `query.intr` with both candidate names.
- A source hit required both candidate terms in the same drug-intervention text.
- An endpoint hit required protocol or results outcome fields in that matched study.
- A no-hit is not negative efficacy evidence; it only means this targeted registry
  query did not find a same-intervention endpoint record.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/clinicaltrials_oas_v2.yaml` | 80,983 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `raw/clinicaltrials_api.html` | 94,295 | `da0916c3c6c7549118c989f03d8870f213428cd97e9fd0420d9aedc57677c616` |
| `raw/clinicaltrials_about_api.html` | 94,295 | `da0916c3c6c7549118c989f03d8870f213428cd97e9fd0420d9aedc57677c616` |

Source URLs:

- https://clinicaltrials.gov/api/oas/v2
- https://clinicaltrials.gov/data-api/api
- https://clinicaltrials.gov/data-api/about-api

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `clinicaltrials_endpoint_query_responses.jsonl` | 72 | 1,554,295 | `347f29d788af7c97cf4b259427492bea0d88747a2809ccaaeb6e1a9fe36d8822` |
| `clinicaltrials_endpoint_pair_status.jsonl` | 72 | 98,200 | `bfd61154e643adb9c852f5c0ee031db018e6986026ec400863784d677f1806c8` |
| `clinicaltrials_endpoint_rollup_status.jsonl` | 108 | 171,164 | `7cc7fd9b1ed55a41dcd02ccca37dd72647dae877f524c0c02d049d6771425b84` |
| `clinicaltrials_endpoint_study_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `clinicaltrials_endpoint_bridge_rows.jsonl` | 108 | 156,862 | `f117521406dc4280986b5ccbaac33c327662a4df3ceeb3278b2a9d479b2dbb3b` |
| `input_manifest.json` | - | 3,602 | `1ae93f1fbef2735401c344c42082f270a171272531efb7101b5e3609da5c3c18` |
| `validation_metrics.json` | - | 1,264 | `347770274a8530955fa15411669bd95a946477d5eb7bc1e6e8d81c2a15cbb36f` |
| `output_manifest.json` | - | 2,449 | `844615d7b830d76bf7c8468826a6a64b609c17e724194b28d8231450e324bfd7` |
| `persisted_readback.json` | - | 3,544 | `376633adc3bf8a5163c06f2a4d85bd5843b87bfe40789ddab36bbcd7a26739d0` |
| `calyx_bridge_corpus_stdout.json` | - | 706 | `4313a17bb9144a66b544df81dd5798bc7981147ca8f3e6d6a0a4d1a54df83e6e` |
| `calyx_bridge_corpus_readback.json` | - | 5,811 | `37e0f59f466acb5a1b246a97476db92b30faf2f0b383e5f42a19337d9fb016e4` |

## Metrics

| Metric | Count |
|---|---:|
| #1247 rollup rows checked | 108 |
| Unique pair keys | 72 |
| Queryable pair keys | 72 |
| ClinicalTrials.gov query response rows | 72 |
| Total API pages read | 72 |
| Responses with any returned study | 3 |
| Pair statuses with registry no-hit | 72 |
| Rollup statuses with registry no-hit | 108 |
| Study-evidence rows | 0 |
| Endpoint study-evidence rows | 0 |
| Result endpoint study-evidence rows | 0 |

#1247 category counts carried through:

| #1247 category | Rollups |
|---|---:|
| `clinical_endpoint_language_review` | 59 |
| `endpoint_or_outcome_language_review` | 21 |
| `mechanistic_endpoint_context_review` | 19 |
| `pharmacokinetic_or_exposure_endpoint_review` | 4 |
| `combination_or_coexposure_context_review` | 3 |
| `preclinical_or_cell_endpoint_language_review` | 2 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1247 persisted readback all true | true |
| #1247 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1247 rollup | true |
| Pair status values allowed | true |
| Rollup status values allowed | true |
| Endpoint hits have endpoint study evidence | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1250-clinicaltrials-endpoint-validation-20260704t230000z
vault_id: 01KWQAY8DYNXAJ5W90MVP2CBTE
vault_dir: /home/croyse/calyx/vaults/01KWQAY8DYNXAJ5W90MVP2CBTE
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 108 |
| Bridge terms | 152 |
| Graph nodes | 260 |
| Graph edges | 864 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1250 found no ClinicalTrials.gov same-intervention endpoint hits for the 72
unique #1247 pair keys. Three registry queries returned at least one study, but
none placed both candidate terms in the same drug-intervention text with
endpoint fields. All 108 rollups remain
`clinicaltrials_registry_no_hit_still_blocked`.

This does not falsify the source-text endpoint language from #1247; it only
records a targeted independent registry no-hit. No efficacy claim, safety
claim, treatment guidance, recommendation, clinical actionability, dosing
guidance, or cure claim is made.
