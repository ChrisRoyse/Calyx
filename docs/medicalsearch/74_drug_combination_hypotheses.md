# #1190 Drug-Combination and Synergy Hypothesis Miner

## Scope

#1190 mines drug-pair hypotheses from the #1193 human-review atlas and blocks
promotion unless component safety, pair interaction, and external synergy/model
evidence are all present. This is a fail-closed triage layer: missing evidence
is a block, not a weak pass.

No row is efficacy, safety, clinical actionability, treatment guidance, dosing,
recommendation, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1190_drug_combination_miner.py
```

The script:

- reads the #1193 atlas and extracts specific drug-bearing components;
- groups components by disease/context and pairs the top components per group;
- scores target/pathway rationale for complementary targets or convergent
  pathway evidence;
- checks component safety rows and openFDA drug-interaction label sections from
  #1181;
- checks exact co-intervention trial rows from #1177;
- streams the DrugComb v1.4 summary table for exact drug-pair preclinical
  synergy matches;
- emits one safety/interaction flag per pair and blocks every pair missing any
  required evidence.

External source research:

- DrugComb Zenodo record: <https://zenodo.org/records/11102665>
- DrugComb downloaded file: `summary_table_v1.4.csv`
- NCI ALMANAC CellMiner source page: <https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html>
- NCI ALMANAC/figshare collection: <https://figshare.com/collections/Data_from_The_National_Cancer_Institute_ALMANAC_A_Comprehensive_Screening_Resource_for_the_Detection_of_Anticancer_Drug_Pairs_with_Enhanced_Therapeutic_Activity/6508832>

DrugComb is used here as preclinical/cell-line synergy evidence only. It is not
clinical efficacy.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/persisted_readback.json
sha256: 9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2

/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/calyx_bridge_corpus_readback.json
sha256: 444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f
```

Native Calyx materialization:

```text
name: issue1190-drug-combination-miner-20260704t130000z
vault_id: 01KWPKR3DMS0GX68YDCSCVP12T
vault_dir: /home/croyse/calyx/vaults/01KWPKR3DMS0GX68YDCSCVP12T
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 309 |
| Graph nodes | 1,309 |
| Graph edges | 10,000 |
| CSR persisted | true |
| Active vault index contains name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1193 atlas | 1,877 rows | `766cebc55e4fb6c49672bf09b2a5e02ebbd81da7dfdbb14a8766e4d66aa8098e` |
| #1181 drug safety terms | 14 rows | `a3840c545dc1c8efdf5a23fe944385147045640c2ba0561ff919c58ac0fa20f4` |
| #1181 parsed safety rows | 353 rows | `b56926bffe2c580c141a74e701c8f0535195d06f33d9c875812729e36f40d167` |
| #1181 mapped candidate safety | 26 rows | `1f0eb4b787c708f5e87c5238d905ca5015b448db638866a2c4b538020dbc54e7` |
| #1177 ClinicalTrials rows | 269 rows | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| DrugComb summary table v1.4 | 193,184,734 bytes | `e08e3d35bfa4bea011afe1b05b7025acde5c669c5e8d265866b5e12b08037a2f` |

DrugComb file integrity:

| Field | Value |
|---|---|
| Expected MD5 | `c11efbdcae4a860c2374c1505a66599b` |
| Observed MD5 | `c11efbdcae4a860c2374c1505a66599b` |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `combination_candidate_inputs.jsonl` | 1,023 | `be973631b5f16b4989db6d0cb8acad2fad0a369fad3c0c2b4932608573aada67` |
| `drug_component_safety_index.jsonl` | 14 | `53e296b2c27685c21e5280c7d70b0e7df2f2b66b72b369d95d9a95e2f75385a8` |
| `candidate_pair_inputs.jsonl` | 1,750 | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| `drugcomb_pair_matches.jsonl` | 28 | `9b075a80df9b6b252191d548ee68c93202e2ed9dfaa4430e226446aa7ad51a86` |
| `drug_combination_hypotheses.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| `combination_safety_interaction_flags.jsonl` | 1,750 | `86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab` |
| `blocked_combination_rows.jsonl` | 1,750 | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| `top_combination_review_queue.json` | - | `01e7a98b5f3685e8ea2f7d0615cac3c9ba49b63febedf213226f4feee26ca4b2` |
| `combination_bridge_rows.jsonl` | 1,000 | `fb4af52867fd7f7049ea268f7297db415202688388b38ebe1e35c73bd14b4ecd` |
| `validation_metrics.json` | - | `70c67ee66348531c2e8e99a5832028b5c9f8fbb5d208df69a0c15963a4d0d83a` |
| `persisted_readback.json` | - | `9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2` |
| `calyx_bridge_corpus_readback.json` | - | `444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f` |

## Metrics

| Metric | Count |
|---|---:|
| Component input rows | 1,023 |
| Candidate pair rows | 1,750 |
| Combination hypothesis rows | 1,750 |
| Safety/interaction flag rows | 1,750 |
| Blocked combination rows | 1,750 |
| Reviewable preclinical rows | 0 |
| DrugComb matched pair keys | 28 |
| Rows with DrugComb match | 47 |
| Rows with exact pair interaction evidence | 5 |
| Rows with both component safety rows | 23 |
| Component safety index rows | 14 |

Reason-code counts:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,703 |
| `overlapping_component_safety_flags_review_required` | 17 |

Disease-area counts:

| Area | Count |
|---|---:|
| rare disease | 1,470 |
| infectious/immunology/inflammation | 149 |
| neurodegeneration/neuropsychiatric | 66 |
| metabolic/cardiovascular/renal | 47 |
| oncology | 18 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Component inputs present | true |
| Candidate pairs present | true |
| Hypothesis rows match pairs | true |
| Flag rows match hypotheses | true |
| DrugComb source read | true |
| All rows have clinical boundary | true |
| Blocked rows present | true |
| Bridge rows <= 1,000 | true |
| No promoted clinical rows | true |

## Top Blocked Rows

These rows are useful because they name exactly what evidence is missing before
any combination can be reviewed.

| Rank | Pair | Disease/context | Status | Reason codes |
|---:|---|---|---|---|
| 1 | Metformin + Sitagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing |
| 2 | Metformin + Saxagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; pair interaction missing |
| 3 | Metformin + Alogliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; external synergy missing |
| 4 | Metformin + Linagliptin | Type 2 Diabetes Mellitus | blocked | component blocked/demoted; component safety missing; external synergy missing |
| 5 | Sunitinib + Everolimus | Hereditary pheochromocytoma-paraganglioma | blocked | component blocked/demoted; component safety missing; pair interaction missing |

## Findings

- The miner produced a real combination worklist, not a recommendation list:
  1,750 candidate pairs were persisted, and all 1,750 are blocked.
- DrugComb did add external preclinical evidence: 28 exact pair keys matched the
  candidate pairs, covering 47 rows. These remain preclinical and do not
  override safety/interaction blocks.
- The dominant blockers are missing component safety rows, missing exact
  drug-pair interaction evidence, and missing external synergy evidence. These
  are actionable data-ingest deficits, not clinical conclusions.
- The current safety substrate is too narrow for broad combination promotion:
  only 14 component safety rows are available for 1,023 drug components.
- The 1,000 highest-ranked blocked combination rows are now materialized in
  native Calyx vault `01KWPKR3DMS0GX68YDCSCVP12T`.

## Conclusion

#1190 is complete for the fail-closed combination miner slice: it builds
combination hypotheses, persists component safety/interaction/synergy evidence,
blocks missing evidence, and materializes the worklist into Calyx.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
