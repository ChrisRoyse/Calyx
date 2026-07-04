# #1246 Europe PMC Safety/Counter Review

Status: complete.

This slice reviewed the #1244 Europe PMC rollups that carried bounded
source-text safety, adverse, counter, or negative language. It separated those
signals into review categories while keeping every row blocked from clinical
promotion.

Clinical boundary:

```text
Europe PMC safety/counter review is literature triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/candidate_europepmc_relation_rollup.jsonl
sha256: 730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef

/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/europepmc_source_text_relation_validation.jsonl
sha256: ab2cdc88971890fe6e036fe11f3a46c1e779e9c2a52cae429f5c16a2966c33d2
```

Source contract:

- The input filter was `source_text_validation_status == source_text_safety_or_counter_review_required_still_blocked`.
- Scoped #1244 rollups: 363.
- The stage did not fetch new source data; it read #1244 source-text windows and spans.
- Output categories are review flags only, not safety findings or clinical advice.

## Method

The reviewer:

- emitted one rollup review row for each scoped #1244 rollup;
- emitted evidence-review rows for linked #1244 source-text validation rows;
- separated fatality/mortality, contraindication/avoidance, toxicity,
  adverse/safety, counter/negative, and generic risk language;
- preserved source ids, source-window text, source hashes, and source relation
  classes;
- kept every row blocked pending independent safety/falsification and human
  review;
- wrote a 673-row bridge-corpus slice for native Calyx materialization.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `europepmc_safety_counter_evidence_review.jsonl` | 310 | 5,133,255 | `2906d957d06b38ce9ef1f33dc2b42f9984adf9055a1295d9d598ce7fb4a65089` |
| `candidate_europepmc_safety_counter_rollup.jsonl` | 363 | 780,131 | `b7fddaf84c0c9a09dcbb8a6e59e505a6fe225fe4dce5f0167cd165d4b7b06255` |
| `europepmc_safety_counter_bridge_rows.jsonl` | 673 | 1,002,138 | `c6aa943c39c87e55750f81f8ba7637b7207317dfed096c0d4271c10f94ba73e9` |
| `input_manifest.json` | - | 2,059 | `3e264a939191dafc39215860736402b07d36d0fdfd7cd782c838565d373df41b` |
| `validation_metrics.json` | - | 16,470 | `68cc65a7e9db9daeff2b990905cfc0d39ac86c66b76469521e7aa47082afcd45` |
| `output_manifest.json` | - | 1,851 | `3d53fa444de3dbf043c72e85c7bcd45f768df723f2d4b0b4cb28813e2c49d17e` |
| `persisted_readback.json` | - | 2,907 | `3e0239ed1b237441da320ad503ac50804e3498da74dcffca5e8e37b2a889faab` |
| `calyx_bridge_corpus_stdout.json` | - | 737 | `ba7720baec1d6971ad4c23cf109538c73de9ef79af1fb70d29e1ca95b5ea9b9b` |
| `calyx_bridge_corpus_readback.json` | - | 4,709 | `15f05e9bfd789f766de867916aa95af33e915613ef145336fb1ec805c0f45da3` |

## Metrics

| Metric | Count |
|---|---:|
| Scoped #1244 rollups | 363 |
| Evidence-review rows | 310 |
| Rollup-review rows | 363 |
| Rollups with fatality/mortality language | 125 |
| Rollups with contraindication/avoidance language | 63 |
| Rollups with toxicity language | 170 |
| Rollups with counter/negative language | 330 |
| Rollups with safety/adverse language | 340 |

Evidence category counts:

| Category | Rows |
|---|---:|
| `contraindication_or_avoidance_language_review` | 22 |
| `counter_negative_and_safety_language_review` | 68 |
| `counter_negative_language_review` | 40 |
| `fatality_or_mortality_language_review` | 92 |
| `safety_adverse_language_review` | 20 |
| `toxicity_language_review` | 68 |

Rollup category counts:

| Category | Rows |
|---|---:|
| `contraindication_or_avoidance_language_review` | 40 |
| `counter_negative_and_safety_language_review` | 82 |
| `counter_negative_language_review` | 23 |
| `fatality_or_mortality_language_review` | 125 |
| `safety_adverse_language_review` | 15 |
| `toxicity_language_review` | 78 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1244 persisted readback all true | true |
| #1244 Calyx readback all true | true |
| Rollup review for every scoped rollup | true |
| Reviewed pair ids match scope | true |
| Rollup reviews have evidence | true |
| Evidence reviews have source windows | true |
| Evidence reviews carry the clinical boundary | true |
| Rollup reviews carry the clinical boundary | true |
| Evidence reviews remain blocked | true |
| Rollup reviews remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1246-europepmc-safety-counter-review-20260704t203000z
vault_id: 01KWQ8BK8XQK3AN77PMBEF3WRN
vault_dir: /home/croyse/calyx/vaults/01KWQ8BK8XQK3AN77PMBEF3WRN
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 673 |
| Bridge terms | 1,274 |
| Graph nodes | 1,947 |
| Graph edges | 7,350 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1246 accounted for all 363 #1244 rollups requiring safety/counter review and
separated 310 linked evidence rows into review categories. The result is a
blocked safety/falsification triage layer for downstream human and independent
validation gates.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, recommendation, clinical actionability, or cure claim is made.
