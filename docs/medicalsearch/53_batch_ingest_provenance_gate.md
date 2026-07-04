# 53 - #1211 Batch ingest source-provenance gate

## Scope

#1211 hardens streaming batch ingest so a JSONL row cannot enter Calyx without
minimum source provenance. This is the bulk path used by biomedical ingestion;
unguarded rows would become graph nodes without a source dataset, checksum,
license, retrieval timestamp, or stable locator.

This change proves source-traceability enforcement only. It does not validate a
biomedical association, treatment, safety decision, clinical action, or cure.

## Code Change

`crates/calyx-cli/src/cmd/ingest/batch.rs` now validates every non-blank batch
line during parser preflight, before opening the vault or initializing
measurement state.

Required non-empty metadata keys:

- `source_dataset`
- `source_sha256`
- `license`
- `retrieval_ts`
- at least one locator: `source_url`, `doi`, `pmid`, or `pmcid`

Missing or blank required provenance returns `CALYX_CLI_USAGE_ERROR` with the
line number and missing key. Extra provenance keys remain allowed and are
stored verbatim on the constellation metadata map.

The enforcement is intentionally presence-based, matching the sibling
`bridge_corpus` validation semantics for required source metadata.

## Test Updates

Batch test fixtures now write provenance-bearing rows instead of bare
`{"text": ...}` rows, except for explicit negative tests that prove the new gate
fails closed.

Focused coverage added:

- valid row ingests and its required metadata is read back from the persisted
  Base CF constellation;
- missing metadata object fails in parser preflight;
- blank `source_dataset` fails in parser preflight;
- missing `source_sha256` fails in parser preflight;
- no locator (`source_url`/`doi`/`pmid`/`pmcid`) fails in parser preflight;
- missing `license` fails in parser preflight;
- missing `retrieval_ts` fails in parser preflight;
- missing provenance against a non-existent vault fails before the vault path is
  created.

## Verification

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli batch_ -- --nocapture
cargo test -p calyx-cli batch_ingest_requires_and_persists_source_provenance -- --nocapture
cargo test -p calyx-cli batch_provenance_edge_cases_fail_in_parser_preflight -- --nocapture
cargo test -p calyx-cli missing_batch_provenance_fails_before_vault_open -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1211-batch-provenance-20260704T044300Z\issue1211-batch-provenance-gate
```

FSV artifacts:

| Artifact | Source of truth |
|---|---|
| `valid-row-base-cf-provenance-readback.json` | Aster Base CF readback via `vault.get(cx_id, snapshot)` after batch ingest flush |
| `parser-preflight-provenance-edge-cases.json` | `validate_batch_file` parser preflight errors |
| `missing-provenance-fails-before-vault-open.json` | missing-vault path existence checked after rejected `ingest_batch_streaming` |

Readback summary:

```json
{
  "valid_row": {
    "cx_id": "acf75bde73422e0100269e945b6f2891",
    "stored_metadata_keys": [
      "license",
      "retrieval_ts",
      "source_dataset",
      "source_sha256",
      "source_url"
    ]
  },
  "edge_cases": 6,
  "missing_vault_path_exists_after_error": false
}
```

During FSV, a broad `cargo test -p calyx-cli batch_` run with
`CALYX_FSV_ROOT` set exposed an unrelated older oracle-event FSV branch that
reuses a pre-index-rebuild snapshot and fails with
`CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE`. The same broad batch suite passes
without that test's FSV write branch enabled, and the three #1211 FSV-emitting
tests pass with `CALYX_FSV_ROOT` set.

## Conclusion

#1211 closes the ungrounded-row hole in the streaming batch ingest parser. Bulk
biomedical corpus rows now fail closed before vault open unless they carry the
minimum provenance needed to trace associations back to their source.
