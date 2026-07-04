# #1232 ClinicalTrials.gov Current Recheck

## Scope

#1232 performs a direct current ClinicalTrials.gov v2 API recheck over the
1,546 #1231 rows that still had no external combination-source hit after
DrugComb, NCI ALMANAC, CDCDB, and the #1231 CDCDB join.

The recheck persists one raw API response per pair, follows pagination, and
derives deterministic status rows only when both candidate drug names are found
in returned study intervention text. Trial-registry co-occurrence is not
efficacy proof, safety proof, dosing guidance, treatment guidance,
recommendation, clinical actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1232_clinicaltrials_recheck.py
```

The script:

- reads the #1231 prior no-hit recheck rows;
- filters to the 1,546 rows still `no_external_hit` after CDCDB;
- downloads and fingerprints the current ClinicalTrials.gov v2 OpenAPI spec;
- queries the current v2 API with `query.intr` for each remaining pair;
- follows `nextPageToken` pagination for every pair;
- stores raw API responses and page hashes;
- emits deterministic `exact_hit` or `no_external_hit` rows;
- writes one study-evidence row per matched NCT study;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted source:

- ClinicalTrials.gov v2 API docs: <https://clinicaltrials.gov/data-api/api>
- API overview: <https://clinicaltrials.gov/data-api/about-api>
- OpenAPI spec: <https://clinicaltrials.gov/api/oas/v2>
- Studies endpoint: <https://clinicaltrials.gov/api/v2/studies>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/persisted_readback.json
sha256: 606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1

/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/calyx_bridge_corpus_readback.json
sha256: ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a

/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/vault_supersession_readback.json
sha256: 91ad37fbcac9c838a730c74dae66b13e5f7870b56414927f9eae433a9c8dbd40
```

Native Calyx materialization:

```text
name: issue1232-clinicaltrials-current-recheck-20260704t154500z
vault_id: 01KWPWANC80TEH4HZWJM337ZX0
vault_dir: /home/croyse/calyx/vaults/01KWPWANC80TEH4HZWJM337ZX0
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 305 |
| Graph nodes | 1,305 |
| Graph edges | 8,838 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

Supersession readback:

| Item | Value |
|---|---|
| Stale non-paginated vault | `01KWPVN28JV96ZDPG12D0C5WA9` |
| Final paginated vault | `01KWPWANC80TEH4HZWJM337ZX0` |
| Old vault absent from active index | true |
| Old vault has supersession record | true |
| Supersession points to final vault | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| ClinicalTrials.gov OpenAPI v2 spec | 80,983 bytes | `ba31adaea67e6bb09ff77af5c0c11daf36d5aff7f5d6cbed89f0aab04b297aea` |
| #1231 prior no-hit recheck rows | 1,682 rows | `1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `clinicaltrials_raw_responses.jsonl` | 1,546 | 480,925,442 | `1f8d40df3bac91b30807e9eae785a70f6710ebccf8304b7c8e2eb678ee2b6e2d` |
| `clinicaltrials_pair_status.jsonl` | 1,546 | 2,178,478 | `ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc` |
| `clinicaltrials_pair_hits.jsonl` | 204 | 747,137 | `026cc1d5698f6b7b2fdcf2cd095cf0a605a804f1ed3f40ed0bd151342c006c38` |
| `clinicaltrials_study_evidence.jsonl` | 1,192 | 1,638,764 | `cb25d98c9fd5c623d63c31a9dcf7b7fbc55add65cc0a7b9d265fe83e5f875970` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,299,985 | `9f93b312d2bb7e96b779291b4579b9f635e3780ce113799f0e41b06e8e9656f9` |
| `input_manifest.json` | - | 1,734 | `b9ab3f1acdfcb89a9c36cf0de17fe030031fe68364e04794c3f455fbd2403087` |
| `output_manifest.json` | - | 2,378 | `ad6e55d387c2fe0f6558561ee403ffc4f2350a9bc5f81ad08f05e76afd123d1d` |
| `validation_metrics.json` | - | 9,890 | `31042216dacc7142351cd4a97336979442f213e89ae591b416fa43d74db3640b` |
| `persisted_readback.json` | - | 2,823 | `606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1` |
| `calyx_bridge_corpus_stdout.json` | - | 711 | `38b47e193ac6590d0f7e411160a42bed4f65488d32370925d181b5fc96179f2a` |
| `calyx_bridge_corpus_readback.json` | - | 3,287 | `ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a` |
| `stale_vault_supersession_stdout.json` | - | 3,689 | `094252fc130057ff3448a3b5414cf88e0b99b8037458a19abffeaf802cc208c0` |
| `vault_supersession_readback.json` | - | 4,875 | `91ad37fbcac9c838a730c74dae66b13e5f7870b56414927f9eae433a9c8dbd40` |

## Metrics

| Metric | Count |
|---|---:|
| Remaining no-hit rows queried | 1,546 |
| Raw API response rows | 1,546 |
| API pages read | 1,618 |
| Max API pages for one pair | 31 |
| Responses with at least one returned study | 285 |
| Responses with pagination | 10 |
| Candidate rows with current ClinicalTrials.gov hit | 204 |
| Matched NCT study evidence rows | 1,192 |
| Unique matched NCT ids | 399 |
| Remaining no-hit rows after current ClinicalTrials.gov | 1,342 |

Status counts:

| Status | Count |
|---|---:|
| `exact_hit` | 204 |
| `no_external_hit` | 1,342 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Raw response for every remaining no-hit row | true |
| Deterministic status for every row | true |
| All joined rows carry boundary | true |
| All hits have matched studies | true |
| All study rows have NCT ids | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains final name exactly once | true |
| Stale non-paginated vault absent from active index | true |
| Stale vault has supersession record | true |

## Findings

- Current direct ClinicalTrials.gov registry recheck adds 204 external
  combination-source hits to the 1,546 rows that remained no-hit after CDCDB.
- The remaining external no-hit set drops from 1,546 to 1,342.
- All 204 hits are `exact_hit` under this parser because both candidate drug
  names appear in the returned study intervention text.
- This is registry documentation only. It does not clear efficacy, safety,
  dosing, outcome, or clinical actionability gates.
- The final paginated bridge corpus is materialized in native Calyx vault
  `01KWPWANC80TEH4HZWJM337ZX0`; the earlier non-paginated trial vault
  `01KWPVN28JV96ZDPG12D0C5WA9` is superseded and no longer active.

## Conclusion

#1232 is complete for the current ClinicalTrials.gov direct recheck slice:
1,546 remaining no-hit candidate rows were queried against the current API,
1,618 API pages were persisted and checksummed, 204 source-attributed registry
hits were found, and the bounded bridge corpus was materialized into Calyx with
post-supersession readback.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
