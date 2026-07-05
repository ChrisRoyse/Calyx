# #1233 CDCDB Gate Validation

Status: complete for the CDCDB-supported row validation slice.

#1233 reads the sealed #1231 CDCDB artifacts and classifies every CDCDB hit
into source-context, exact-pair versus multi-drug context, and fail-closed gate
status rows. CDCDB source context is not treated as synergy, pair-interaction
proof, safety clearance, efficacy, treatment guidance, dosing guidance,
clinical actionability, recommendation, or cure evidence.

Clinical boundary:

```text
CDCDB source-context validation is external combination-source triage only; ClinicalTrials.gov, Orange Book, patent, exact-pair, and multi-drug context rows are blockers or review inputs, not synergy, pair-interaction proof, efficacy, safety clearance, dosing guidance, treatment guidance, recommendation, clinical actionability, or cure evidence.
```

## Implementation

Script:

```text
scripts/medicalsearch/issue1233_cdcdb_gate_validation.py
```

The script:

- verifies sealed #1231 artifact hashes;
- reads the CDCDB hit rows and prior no-hit recheck rows;
- classifies CDCDB source context as ClinicalTrials.gov, patent, Orange Book,
  or mixed;
- classifies each hit as an exact two-drug source record, multi-drug context
  only, or both;
- emits one pair validation status row per CDCDB hit;
- emits one missing/not-cleared gate row per pair/gate;
- writes a bounded bridge corpus for native Calyx materialization.

## FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1233-cdcdb-gate-validation-20260705T025403Z
```

Script capture:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `script.stdout.txt` | 4,006 | `92d6d19beab9ec30410dabebbca4aa52e749d8d813d7b14969e35d3b9832ee17` |
| `script.stderr.txt` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `script.capture.txt` | 409 | `370bb849f8c82ee9b9697944d59e0058e0b84ddb6c6a343eb2bd1352057a4abd` |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1231 `candidate_external_combo_hits.jsonl` | 173 rows | `c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a` |
| #1231 `candidate_external_combo_status.jsonl` | 1,750 rows | `f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4` |
| #1231 `prior_no_hit_recheck_status.jsonl` | 1,682 rows | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |
| #1231 `cdcdb_source_combinations.jsonl` | 43,082 rows | `004b07ee5b308501d048a8a919d6ce7af9dcd086897b25b777c17d43d6a32f79` |
| #1231 `cdcdb_pair_index.jsonl` | 78,321 rows | `55d77873ae1c7cd1a5d670c82050de6b22134f419fafccdd3a7edf7742c75714` |
| #1231 `cdcdb_source_schema.json` | 6,587 bytes | `d5f27a3c635bd23984dd303fff882e61c42c9b6e828df324950bdc88349ac7f2` |
| #1231 `validation_metrics.json` | 9,600 bytes | `ebdc0863663e14ebb9bc96d84fe9d5787a4b14c279f5bb5ca34c37750c1f041d` |
| #1231 `persisted_readback.json` | 3,259 bytes | `e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825` |
| #1231 `calyx_bridge_corpus_readback.json` | 3,779 bytes | `2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db` |
| #1231 `output_manifest.json` | 2,866 bytes | `b62fdabdfff60d34ee17d3dc983c8f2192a91add3bb100775df3361ac58ddf29` |

## Outputs

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 10 | 8,784 | `fb1bb49b2a01018596b7c18443bde99d56678ea782b1786a93aee9dc930f0e89` |
| `cdcdb_source_context_rows.jsonl` | 611 | 932,950 | `5924817886ddba2828df1e33b4ab8c5141fb08024cab4371526934c26e72c6c2` |
| `cdcdb_pair_validation_status.jsonl` | 173 | 366,915 | `c0d3e03419dc1240260b18a81a9e9bf3601305773cb3ec99a6b217d81825f3b2` |
| `cdcdb_missing_gate_rows.jsonl` | 692 | 618,947 | `3ce4b35f88a385ca1aaf977690925981ab8c07eeb7d9b4e686b3bb572d53ec35` |
| `issue1233_bridge_rows.jsonl` | 784 | 915,177 | `9cbca17975fb44a719c74781d29452ae34335b40018834638c69c4e3bf5e4868` |
| `validation_metrics.json` | - | 1,751 | `6993bc18d3c8b9e15f71b3d73b6009c5d11e6bdc6a2770a1287ce0cf041cb94c` |
| `input_manifest.json` | - | 10,565 | `d0ba1de6dc342b16c6f333ecc17a8bfcaa4e364e3eb0f85daafcfe709cb215d8` |
| `output_manifest.json` | - | 2,366 | `26ef2f4b51cccb500d29ea4bbfdae596e5bed526702defdb8ed42444439cadb5` |
| `persisted_readback.json` | - | 3,288 | `c67ffc5baa8f5d8477ed942dd3e0183b12217c1f9a9c15f5631d10e652c1e676` |
| `calyx_bridge_corpus_stdout.json` | - | 678 | `ddb31ebe617614dc48d8e1ad178550d2eecdf80befc65f72870c87bf311dd998` |
| `calyx_bridge_corpus_stderr.txt` | - | 331 | `ffc24db945bc6f0d817741d26153e129e7726998301cef8b7c513d85eda99f2a` |
| `calyx_bridge_corpus.capture.txt` | - | 445 | `63bf7ed5c2ae5f7d550e5f87336d27ee97e75c550e6347ab0fed3f56a3a34441` |
| `calyx_bridge_corpus_readback.json` | - | 8,364 | `5eec4117b0c6dd4a58d9e2d2ba4d600043020721b2f9997a57ee54aa506effc4` |

## Metrics

| Metric | Count |
|---|---:|
| CDCDB hit rows checked | 173 |
| Prior no-hit recheck rows checked | 1,682 |
| Prior no-hit rows with CDCDB hit | 136 |
| Source-context rows | 611 |
| Pair validation status rows | 173 |
| Missing/not-cleared gate rows | 692 |
| Bridge rows | 784 |
| Unique pair keys | 85 |
| Pair rows with an exact two-drug CDCDB record | 63 |
| Pair rows with multi-drug context only | 110 |

Validation status counts:

| Status | Rows |
|---|---:|
| `blocked_no_safety` | 163 |
| `blocked_component_safety_review` | 10 |

CDCDB source context counts:

| Context | Rows |
|---|---:|
| `patent_context` | 67 |
| `mixed_clinicaltrialsgov_patents` | 55 |
| `clinicaltrials_registry_context` | 51 |

Pair record class counts:

| Class | Rows |
|---|---:|
| `multi_drug_context_only` | 110 |
| `two_drug_and_multi_drug_context` | 48 |
| `exact_two_drug_source_record` | 15 |

Gate status counts:

| Gate status | Rows |
|---|---:|
| `component_safety_missing_fail_closed` | 163 |
| `component_safety_flags_review_required_not_clearance` | 10 |
| `pair_interaction_evidence_missing_fail_closed` | 168 |
| `cdcdb_source_context_not_pair_interaction_or_synergy_proof` | 5 |
| `grounded_outcome_endpoint_missing_fail_closed` | 173 |
| `missing_human_review_fail_closed` | 173 |

## Native Calyx Materialization

```text
name: issue1233-cdcdb-gate-validation-20260705t025403z
vault_id: 01KWR35QT2C6GWHZNMX32PNKDK
vault_dir: /home/croyse/calyx/vaults/01KWR35QT2C6GWHZNMX32PNKDK
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 784 |
| Bridge terms | 358 |
| Graph nodes written | 1,142 |
| Graph edges written | 6,964 |
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

- #1231 persisted readback assertions are all true and Calyx readback status is ok.
- All #1231 input hashes match expected values.
- All 173 CDCDB hit rows and all 1,682 prior no-hit rows are checked.
- The 136 prior no-hit CDCDB hits match #1231.
- Every CDCDB hit has exactly one #1233 pair status row and at least one
  source-context row.
- All statuses are fail-closed.
- CDCDB source context is never counted as synergy or pair-interaction proof.
- Bridge rows are bounded.

## Result

#1233 classifies all 173 CDCDB-supported rows into deterministic fail-closed
statuses. CDCDB provides source context for external combination documentation,
including 63 rows with at least one exact two-drug source record and 110 rows
that are multi-drug-context-only. All rows remain blocked on safety review,
pair-interaction/synergy, grounded outcome, and human-review gates.

No efficacy claim, safety-clearance claim, pair-interaction proof, treatment
guidance, dosing guidance, recommendation, clinical-actionability claim, or
cure claim is made.
