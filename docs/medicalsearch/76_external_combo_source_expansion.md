# #1231 External Combination Source Expansion

## Scope

#1231 expands the #1190/#1229 drug-combination evidence search beyond
DrugComb v1.4 and NCI ALMANAC by adding CDCDB, an open drug-combination source
snapshot. The run rechecks every #1190/#1229 candidate pair and separately
rechecks the 1,682 rows that remained `no_external_hit` after #1229.

CDCDB evidence is source-attributed combination documentation from
ClinicalTrials.gov, FDA Orange Book, and patent-derived records. It is not
synergy proof, efficacy proof, safety proof, treatment guidance, dosing,
recommendation, or cure evidence. Rows with CDCDB hits remain blocked until
component safety, pair interaction, grounded outcome, and human-review gates are
separately satisfied.

## Implementation

Script:

```text
scripts/medicalsearch/issue1231_external_combo_sources.py
```

The script:

- reads the CDCDB Figshare metadata and CSV archive;
- checksums the physical archive and records Figshare API file metadata;
- fingerprints every CSV schema in the CDCDB archive;
- parses `all_combs_unormalized.csv` into source-combination rows;
- derives normalized pair keys across all drug groups and aliases;
- joins CDCDB pair evidence to every #1190/#1229 candidate pair;
- emits deterministic `exact_hit`, `normalized_hit`, or `no_external_hit`
  status for every candidate and every prior no-hit row;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- CDCDB Figshare dataset:
  <https://springernature.figshare.com/articles/dataset/CSV_version_of_CDCDB_from_12_4_2022/19582069>
- Figshare API metadata:
  <https://api.figshare.com/v2/articles/19582069>
- Direct archive URL:
  <https://ndownloader.figshare.com/files/34785670>
- Data descriptor:
  <https://www.nature.com/articles/s41597-023-02303-8>
- License recorded by the run: `CC0`

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/persisted_readback.json
sha256: e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825

/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/calyx_bridge_corpus_readback.json
sha256: 2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db
```

Native Calyx materialization:

```text
name: issue1231-external-combo-sources-20260704t150500z
vault_id: 01KWPTB0JDE1NCSHB4477BH7HA
vault_dir: /home/croyse/calyx/vaults/01KWPTB0JDE1NCSHB4477BH7HA
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 357 |
| Graph nodes | 1,357 |
| Graph edges | 8,508 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| CDCDB `12.04.2022.zip` | 64,277,993 bytes | `02e6151bfcce9617d47260ad8d60570432e21de1c7330ca1f946c44faf169d3c` |
| Figshare API metadata | 3,377 bytes | `c5f066523101409c1781eb9fdb0a405879474c3fbfc5ec2db52aa1bc21d0872b` |
| #1190 candidate pairs | 1,750 rows | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1229 prior external status | 1,750 rows | `e4dbb332db9c4b656db83452a2f0debac9748c4c0cadb4b98c9280cfbcc6a4b7` |

CDCDB file integrity:

| Check | Value |
|---|---|
| Archive MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Figshare supplied MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Figshare computed MD5 | `2af17e658987b6c32b3d95f3a7c5ed7e` |
| Bytes match Figshare API | true |
| Download URL matches Figshare API | true |
| CDCDB schema fingerprint | `b287a134e9aa7d578db38dad09af1a05a4612303fff968c98e29ff3e2360e8b5` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `cdcdb_source_combinations.jsonl` | 43,082 | 38,714,138 | `004b07ee5b308501d048a8a919d6ce7af9dcd086897b25b777c17d43d6a32f79` |
| `cdcdb_pair_index.jsonl` | 78,321 | 127,436,740 | `55d77873ae1c7cd1a5d670c82050de6b22134f419fafccdd3a7edf7742c75714` |
| `candidate_external_combo_status.jsonl` | 1,750 | 2,641,087 | `f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4` |
| `prior_no_hit_recheck_status.jsonl` | 1,682 | 2,449,119 | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |
| `candidate_external_combo_hits.jsonl` | 173 | 595,639 | `c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,314,779 | `dde19c92a5e657298fef463a8345ba262a9e99507e481546db02b664c6229a14` |
| `input_manifest.json` | - | 9,796 | `b6f52fe6ca249d9696acddf17e0da1bee94865e40e3716f5b43497b9d4c2f864` |
| `output_manifest.json` | - | 2,866 | `b62fdabdfff60d34ee17d3dc983c8f2192a91add3bb100775df3361ac58ddf29` |
| `validation_metrics.json` | - | 9,600 | `ebdc0863663e14ebb9bc96d84fe9d5787a4b14c279f5bb5ca34c37750c1f041d` |
| `persisted_readback.json` | - | 3,259 | `e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825` |
| `calyx_bridge_corpus_stdout.json` | - | 679 | `f93a69be19de85986ebbb64b5dd643ddca3983ca682f1e3cb64b7809ebb3bb2d` |
| `calyx_bridge_corpus_readback.json` | - | 3,779 | `2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db` |

## Metrics

| Metric | Count |
|---|---:|
| CDCDB source combination rows parsed | 43,082 |
| CDCDB unique normalized pair keys | 78,321 |
| #1190/#1229 candidate rows rechecked | 1,750 |
| #1229 prior no-hit rows rechecked | 1,682 |
| Candidate rows with CDCDB hit | 173 |
| Prior no-hit rows with CDCDB hit | 136 |
| Candidate rows with any external evidence after CDCDB | 204 |
| Remaining prior no-hit rows after CDCDB | 1,546 |

CDCDB source-combination row counts:

| Source type | Rows |
|---|---:|
| `clinicaltrials.gov` | 28,322 |
| `orangebook` | 551 |
| `patents` | 14,209 |

CDCDB status counts over all candidate rows:

| Status | Count |
|---|---:|
| `exact_hit` | 62 |
| `normalized_hit` | 111 |
| `no_external_hit` | 1,577 |

Overall external-evidence status after DrugComb + ALMANAC + CDCDB:

| Status | Count |
|---|---:|
| `exact_hit` | 109 |
| `normalized_hit` | 95 |
| `no_external_hit` | 1,546 |

Reason-code counts after the CDCDB recheck:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,682 |
| `external_combo_source_hit_not_clearance` | 173 |
| `external_combo_source_missing_fail_closed` | 1,546 |
| `external_synergy_recheck_required_not_a_pass` | 21 |
| `overlapping_component_safety_flags_review_required` | 17 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| CDCDB source rows present | true |
| CDCDB pair keys present | true |
| Joined rows match prior status rows | true |
| Prior no-hit recheck rows match prior no-hit rows | true |
| Deterministic status for every candidate | true |
| Deterministic status for every prior no-hit | true |
| All joined rows carry boundary | true |
| All CDCDB hits have source summaries | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains final name exactly once | true |
| Graph/time-index SST files are present | true |

## Findings

- CDCDB adds a third external combination-evidence source to the #1190/#1229
  candidate universe.
- CDCDB contributes 173 candidate hits: 62 exact-name hits and 111 normalized
  hits.
- CDCDB resolves 136 of the 1,682 #1229 prior no-hit rows to source-attributed
  external-combination evidence, reducing the remaining no-hit set to 1,546.
- The `external_synergy_evidence_missing_fail_closed` reason is intentionally
  preserved on the CDCDB-only rows. CDCDB is not a synergy, safety, dosing, or
  outcome gate.
- The 1,000-row bridge corpus is materialized in native Calyx vault
  `01KWPTB0JDE1NCSHB4477BH7HA`.

## Conclusion

#1231 is complete for the CDCDB source-expansion slice: the additional open
source was downloaded, checksummed, schema-fingerprinted, normalized to
association pair keys, joined to all #1190/#1229 candidates, and materialized
into a native Calyx bridge-corpus vault.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
