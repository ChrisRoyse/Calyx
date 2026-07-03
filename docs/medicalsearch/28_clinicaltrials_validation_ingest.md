# 28 - ClinicalTrials.gov validation ingest

- **Issue:** #1177
- **Status:** Complete bounded FSV for ClinicalTrials.gov trial-readiness evidence.
- **FSV root:** `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z`

This is discovery and triage evidence only. A trial listing, phase, status, or results flag is not a cure, treatment recommendation, efficacy proof, causality proof, or clinical actionability claim.

## What changed

#1177 adds ClinicalTrials.gov registry evidence for selected drug-condition hypotheses and target-derived drug expansions. The run used the official ClinicalTrials.gov v2 API endpoint:

- `https://clinicaltrials.gov/api/v2/studies`
- Query form: `filter.advanced=AREA[InterventionName] <intervention> AND AREA[Condition] <condition>`
- `pageSize=25`
- `format=json`
- `countTotal=true`

The run also persisted:

- API version response: `apiVersion=2.0.5`, `dataTimestamp=2026-07-02T09:00:05`
- OpenAPI snapshot from `https://clinicaltrials.gov/api/oas/v2`
- Source metadata and query provenance for every seed

The public API docs page states that the CTG API specification is available as YAML. The stable live source used by this run was `/api/oas/v2`; `/api/oas/v2.yaml` returned 404 during the FSV probe and was not used as source truth.

## Persisted artifacts

| Artifact | Rows / entries | SHA256 |
|---|---:|---|
| `run_summary.json` | 1 | `7b8f09bed81dd8826eaf480b8ae5cbff806592864f9dcd177f0e3fca9b610cd8` |
| `persisted_readback.json` | 1 | `2e0ec5c1d11a8710dd082816c90734ead83555887f51fcc2715054cf36d194fe` |
| `final_file_manifest.json` | 29 files | `c3ad269f03d25012d4e40d3cdf9b56404c7e44261deead843a0aef3f7869e9e6` |
| `clinicaltrials_api_version.json` | 1 | `de8921a29236b6d6afb41a57d264c51ce14c653bbf0d4899501e410254f2b355` |
| `clinicaltrials_oas_v2.yaml` | 1 | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| `parsed/query_inputs.jsonl` | 13 | persisted in FSV root |
| `parsed/request_records.jsonl` | 15 | persisted in FSV root |
| `parsed/clinicaltrials_trial_rows.jsonl` | 269 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| `parsed/clinicaltrials_seed_summaries.jsonl` | 13 | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| `parsed/clinicaltrials_error_rows.jsonl` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

## Run summary

| Count | Value |
|---|---:|
| Query seeds | 13 |
| Total registry hits across seeds | 1,636 |
| Returned first-page trial rows | 269 |
| Exact intervention matches in returned rows | 260 |
| Returned rows with results available | 114 |
| Returned rows with stopped status | 33 |
| API / parse error rows | 0 |

Each seed was capped at a first page of 25 studies. `total_count` and `next_page_available` were persisted so truncated evidence is visible.

## Seed evidence

| Seed | Total hits | Returned | Exact intervention | Results | Stopped | Max score | Status counts |
|---|---:|---:|---:|---:|---:|---:|---|
| `metformin_type2_diabetes` | 732 | 25 | 25 | 7 | 0 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:21, RECRUITING:1, UNKNOWN:2 |
| `sitagliptin_type2_diabetes` | 335 | 25 | 24 | 12 | 3 | 5.6 | COMPLETED:18, TERMINATED:2, UNKNOWN:4, WITHDRAWN:1 |
| `vildagliptin_type2_diabetes` | 133 | 25 | 24 | 0 | 0 | 4.6 | COMPLETED:21, UNKNOWN:4 |
| `linagliptin_type2_diabetes` | 93 | 25 | 22 | 19 | 1 | 5.6 | COMPLETED:21, RECRUITING:1, TERMINATED:1, UNKNOWN:2 |
| `etanercept_psoriasis` | 86 | 25 | 24 | 15 | 2 | 5.6 | COMPLETED:21, TERMINATED:1, UNKNOWN:2, WITHDRAWN:1 |
| `adalimumab_psoriasis` | 79 | 25 | 23 | 13 | 0 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:21, UNKNOWN:3 |
| `metformin_breast_cancer` | 55 | 25 | 25 | 8 | 7 | 5.2 | ACTIVE_NOT_RECRUITING:1, COMPLETED:10, RECRUITING:3, TERMINATED:6, UNKNOWN:4, WITHDRAWN:1 |
| `alogliptin_type2_diabetes` | 41 | 25 | 25 | 16 | 1 | 5.6 | COMPLETED:21, RECRUITING:1, UNKNOWN:2, WITHDRAWN:1 |
| `metformin_prostate_cancer` | 33 | 25 | 25 | 5 | 10 | 5.2 | ACTIVE_NOT_RECRUITING:1, COMPLETED:7, NOT_YET_RECRUITING:1, RECRUITING:3, TERMINATED:6, UNKNOWN:3, WITHDRAWN:4 |
| `infliximab_psoriasis` | 30 | 25 | 25 | 14 | 1 | 5.6 | ACTIVE_NOT_RECRUITING:1, COMPLETED:19, NOT_YET_RECRUITING:1, RECRUITING:1, TERMINATED:1, UNKNOWN:2 |
| `metformin_colorectal_cancer` | 13 | 13 | 13 | 4 | 5 | 5.2 | COMPLETED:5, TERMINATED:4, UNKNOWN:3, WITHDRAWN:1 |
| `adalimumab_sarcoidosis` | 4 | 4 | 3 | 1 | 3 | 4.2 | COMPLETED:1, TERMINATED:1, WITHDRAWN:2 |
| `infliximab_sarcoidosis` | 2 | 2 | 2 | 0 | 0 | 4.4 | COMPLETED:2 |

## Highest trial-readiness rows

The score is a registry-readiness score only. It rewards exact intervention match, condition match, active/completed status, result availability, and later phase; it penalizes stopped status.

| Seed | NCT | Status | Phase | Results | Score | Sponsor | Title |
|---|---|---|---|---:|---:|---|---|
| `metformin_type2_diabetes` | `NCT00751114` | COMPLETED | PHASE4 | true | 5.6 | Sanofi | Evaluation of Insulin Glargine Versus Sitagliptin in Insulin-naive Patients |
| `linagliptin_type2_diabetes` | `NCT02350478` | COMPLETED | PHASE4 | true | 5.6 | Medical University of Graz | Effects of Linagliptin on Endothelial Function |
| `sitagliptin_type2_diabetes` | `NCT00751114` | COMPLETED | PHASE4 | true | 5.6 | Sanofi | Evaluation of Insulin Glargine Versus Sitagliptin in Insulin-naive Patients |
| `sitagliptin_type2_diabetes` | `NCT00885638` | COMPLETED | PHASE4 | true | 5.6 | Lund University | Effects of Dipeptidyl Peptidase-4 Inhibition on Hormonal Responses to Meal Ingestion |
| `alogliptin_type2_diabetes` | `NCT02771093` | COMPLETED | PHASE4 | true | 5.6 | Takeda | An Exploratory Study of the Effects of Trelagliptin and Alogliptin on Glucose Variability |
| `adalimumab_psoriasis` | `NCT00735787` | COMPLETED | PHASE4 | true | 5.6 | Abbott | Controlled Study of Humira in Subjects With Chronic Plaque Psoriasis of the Hand |
| `etanercept_psoriasis` | `NCT02749370` | COMPLETED | PHASE4 | true | 5.6 | Amgen | Study to Evaluate the Efficacy of Etanercept Treatment in Adults Who Failed Therapy |
| `infliximab_psoriasis` | `NCT00686595` | COMPLETED | PHASE4 | true | 5.6 | Merck Sharp & Dohme LLC | A Study to Evaluate the Switch From Etanercept to Infliximab in Subjects With Moderate-to-Severe Psoriasis |

## FSV

The FSV readback proved:

- Raw ClinicalTrials.gov responses were persisted under `raw/`.
- Parsed query inputs, request records, NCT rows, seed summaries, and error rows were persisted under `parsed/`.
- The run failed closed with zero API/parse error rows, not by treating errors as no evidence.
- A final manifest was written after the readback file existed, proving the readback hash separately.

## Next

Trial registry evidence should now feed:

- #1181 safety/adverse-event triage, because trials with results need safety extraction before any intervention promotion.
- #1182 known-positive/negative and time-split validation gates, because diabetes and psoriasis rows provide strong known-positive calibration material.
- #1184 counter-evidence sweep, because stopped statuses are not failures by themselves but must be reviewed before ranking.

The useful claim unlocked by #1177 is: selected drug-condition hypotheses now have persisted trial-readiness evidence. This still does not prove that any candidate is effective, safe, actionable, or curative.
