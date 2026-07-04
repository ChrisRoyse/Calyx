# #1181 Drug Safety / Adverse-Event Triage

## Scope

#1181 attaches public safety/adverse-event/contraindication evidence to the
drug-bearing #1185 oncology hypothesis atlas. This slice uses FDA public data
through openFDA:

- openFDA drug label API for label sections including boxed warnings,
  warnings, contraindications, drug interactions, and adverse reactions:
  `https://open.fda.gov/apis/drug/label/`
- openFDA drug adverse event API / FAERS for reported adverse-event counts:
  `https://open.fda.gov/apis/drug/event/`

The issue originally named `docs/medicalsearch/32_drug_safety_triage.md`; that
number is already occupied by LINCS/CMap perturbation metadata mapping. This
file keeps the append-only findings log order and cross-links #1181.

This is safety triage evidence only. FDA labels and FAERS/openFDA event counts
do not prove causality, incidence, prevalence, safety, efficacy,
clinical-actionability, treatment recommendation, or cure evidence. Missing
source coverage is a fail-closed ranker block, not an implication of safety.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z
```

Input atlas:

```text
/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl
```

Input readback:

| Role | Bytes | SHA-256 |
|---|---:|---|
| `oncology_hypothesis_atlas` | 23,618 | `455ea1b611e02b6d70d9a6dbbdb6af4faaa680a4521957b57f9166f8ee04c6e1` |

Run summary:

| Field | Count |
|---|---:|
| Drug terms queried | 14 |
| Drug terms with FDA label coverage | 11 |
| Drug terms with FAERS/openFDA event coverage | 11 |
| Parsed safety rows | 353 |
| Raw source files | 28 |
| Raw query manifest rows | 28 |
| Candidate-drug mappings | 26 |
| Candidate safety flag rows | 13 |
| Candidate rows with source-unavailable block | 2 |
| Candidate rows with high-risk label block | 11 |
| Candidate rows with FAERS serious/death-review block | 11 |

Fail-closed missing coverage:

| Drug term | Label | FAERS/openFDA events | Flag |
|---|---|---|---|
| AZ628 | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |
| JQ1 Compound | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |
| VTX-11e | missing | missing | `safety_label_unavailable_fail_closed`; `faers_event_unavailable_fail_closed` |

## Persisted Artifacts

| Artifact | Bytes | SHA-256 | Rows |
|---|---:|---|---:|
| `out/source_constraints.json` | 1,029 | `fd7e8f66e529ea740d96dafecde90b7e2f37e6ad48fee614a4ac804f756134ad` | - |
| `out/input_scope.json` | 1,445 | `03ae1c3871dde485fb0ee494026e8723365ac2b042f8e3abf04a27fcafa852fc` | - |
| `out/drug_safety_summary.json` | 848 | `967fda0a7a78121e6a460ecf7cb835e056f5cde3b4084a28544ebbb8d0757107` | - |
| `out/drug_safety_terms.jsonl` | 33,114 | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` | 14 |
| `out/parsed_safety_rows.jsonl` | 167,725 | `b56926bffe2c580c141a74e701c8f0535195d06f33d9c875812729e36f40d167` | 353 |
| `out/mapped_candidate_safety.jsonl` | 18,286 | `1f0eb4b787c708f5e87c5238d905ca5015b448db638866a2c4b538020dbc54e7` | 26 |
| `out/candidate_safety_flags.jsonl` | 12,305 | `862b83ad7d03f8288916e0323445269ca232247baafd2669fa9d387ea06cba80` | 13 |
| `out/raw_query_manifest.jsonl` | 7,426 | `5895c450130d3000a5ffe1ff6031b0f3d4243fdef4970180b3ce8862f0adc307` | 28 |
| `out/output_manifest.json` | 8,051 | `e69b1f5fe89948d10aa472296ae982e99ec431f7ef67fe081fccfaec5f7b3f83` | - |
| `out/persisted_readback.json` | 1,300 | `98ceeb5277f310f32f25f7ef74f15a1d2830aa0c178ccd979f10c67af7fbf52b` | - |

Raw FDA responses are persisted under:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/raw/openfda_label
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/raw/openfda_event
```

## Drug-Term Flags

| Drug term | Label | FAERS | FAERS total reports | Representative flags |
|---|---|---|---:|---|
| Cytarabine | yes | yes | 62,318 | boxed warning; contraindications; serious/death reports |
| Doxorubicin | yes | yes | 107,484 | boxed warning; contraindications; serious/death reports |
| Gemcitabine | yes | yes | 56,395 | contraindications; warnings; serious/death reports |
| Prednisolone | yes | yes | 193,614 | contraindications; warnings; serious/death reports |
| Selumetinib | yes | yes | 539 | contraindications; interactions; serious/death reports |
| Metformin | yes | yes | 425,794 | boxed warning; contraindications; serious/death reports |
| Vemurafenib | yes | yes | 4,046 | contraindications; interactions; serious/death reports |
| AZ628 | no | no | 0 | source unavailable fail-closed |
| Exemestane | yes | yes | 12,644 | contraindications; interactions; serious/death reports |
| JQ1 Compound | no | no | 0 | source unavailable fail-closed |
| Mirdametinib | yes | yes | 11 | contraindications; warnings; serious reports |
| Sirolimus | yes | yes | 13,671 | boxed warning; contraindications; serious/death reports |
| Trametinib | yes | yes | 6,718 | contraindications; interactions; serious/death reports |
| VTX-11e | no | no | 0 | source unavailable fail-closed |

The FAERS total is a report-count field from the openFDA event API, not an
incidence/prevalence or causality estimate.

## Candidate Ranker Flags

Every drug-bearing candidate receives:

```text
clinical_promotion_block_until_safety_review_complete
```

Additional ranker blocks are attached from the mapped drug evidence:

| Candidate | Therapies | Ranker blocks |
|---|---|---|
| `oncology-civic:11176` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1958` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:10138` | Metformin; Exemestane | high-risk label section; FAERS serious/death reports |
| `oncology-civic:7487` | Selumetinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1470` | Vemurafenib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1230` | Metformin; Trametinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1469` | Sirolimus; Mirdametinib | high-risk label section; FAERS serious/death reports |
| `oncology-civic:1743` | JQ1 Compound | safety source unavailable |

All candidate-level rows are in:

```text
/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/candidate_safety_flags.jsonl
```

## Conclusion

#1181 is complete for the current #1185 drug-hypothesis surface:

- public FDA label and FAERS/openFDA queries were persisted for all 14 therapy
  terms;
- safety terms, parsed safety rows, mapped candidate rows, and candidate
  ranker flags were written separately;
- source-unavailable cases fail closed;
- every drug-bearing candidate is blocked from clinical promotion until safety
  review and later validation clear.

This improves the association atlas by preventing candidate ranking from
silently treating missing or adverse safety evidence as harmless.
