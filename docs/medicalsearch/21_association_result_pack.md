# 21 - Association result pack

- **Issue:** #1170
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for the current association result pack.
- **FSV root:** `/home/croyse/calyx/fsv/issue1170-association-result-pack-20260703T154220Z`

## Bottom line

The current association-mining outputs are now joined into one machine-readable result pack.

- `1,949` result rows.
- `10` source issue surfaces: #875, #876, #877, #878, #880, #881, #882, #883, #884, #994.
- #1171 source expansion joined where applicable: `2,612` source-expanded CxIds available.
- `0` missing required source refs.
- `2` missing non-required refs are the #883 TREC-COVID regrounding target rows, which are outside the #1171 biomedical #869 source-expansion vault and carry their own stored provenance.

This pack is still hypothesis evidence, not biomedical truth or treatment guidance. It is the machine-readable substrate for #1172 concept normalization and downstream validation.

## Persisted Artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `association_result_pack.jsonl` | One row per candidate/hypothesis/bridge/proposer/result surface | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |
| `association_result_pack.json` | JSON object form of the same rows | `25f0d1d7166ff7125b2ca04080d39eb3442a894a259fd35bcad6c09a73107f85` |
| `readback_summary.json` | Input hashes, row counts, issue counts, source-expansion link | `65a6f1b3b0c5d66991c5be760f10a24c9791e645d4273ba956bd8216afe8b5d5` |
| `persisted_readback.json` | Separate readback from persisted JSONL | `319c111346937b8b8b92680eec0a98d40139e9f466a48a32a326edafbffdb1fa` |

## Row Counts

| Row type | Rows |
|---|---:|
| `blind_spot_candidate` | `128` |
| `domain_bridge_candidate` | `7` |
| `spectral_bridge_candidate` | `32` |
| `spectral_centrality_candidate` | `32` |
| `discovery_accepted_hop` | `1,600` |
| `chain_walk_hypothesis` | `48` |
| `hypothesis_evaluation` | `48` |
| `ranked_hypothesis` | `44` |
| `probe_matrix_reground_summary` | `2` |
| `molecular_bridge_row` | `8` |

Issue counts:

| Issue | Rows |
|---:|---:|
| #875 | `128` |
| #876 | `7` |
| #877 | `64` |
| #878 | `1,600` |
| #880 | `48` |
| #881 | `48` |
| #882 | `44` |
| #883 | `2` |
| #884 | `4` |
| #994 | `4` |

## Readback

Separate persisted readback:

```json
{
  "jsonl_row_count": 1949,
  "jsonl_sha256": "50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37",
  "json_sha256": "25f0d1d7166ff7125b2ca04080d39eb3442a894a259fd35bcad6c09a73107f85",
  "missing_required_source_ref_count": 0,
  "missing_nonrequired_source_ref_count": 2
}
```

The pack builder verified every declared input artifact hash before writing outputs. Any missing file or hash mismatch would have aborted the run.

## Source Expansion Join

Rows with biomedical #869 CxIds carry compact source refs from #1171:

- `cx_id`
- `source_dataset`
- `source_id`
- `source_sha256`
- `source_file`
- `source_line`
- `text_sha256`
- `text_len`
- `text_snippet`

The full source text remains in #1171:

```text
/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/complete_cxid_source_expansion.jsonl
```

## Closeout

#1170 acceptance is met:

- JSONL/JSON outputs persist one row per current candidate/hypothesis/bridge/proposer/result surface.
- Each row carries source issue, source artifact path, source artifact SHA256, source CxIds, score fields, gate/verdict fields, provenance or parent evidence, and FSV root.
- Readback summary includes row counts per source issue and hashes.
- The pack builder failed closed on missing/hash-mismatched declared inputs.

Next execution should use `association_result_pack.jsonl` plus #1171 source rows for #1172 concept normalization.
