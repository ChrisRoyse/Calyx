# 54 - #1215 Oracle-event FSV snapshot readback

## Scope

#1215 fixes an FSV-only failure in
`batch_ingest_structures_oracle_recurrence_for_reverse_query`. The normal test
body passed, but when `CALYX_FSV_ROOT` was set the artifact-writing branch reused
an earlier snapshot after the ingest/search-index path had advanced the vault.
That requested historical state from a latest-only recovered vault and failed
with `CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE`.

This is a verification-path fix. It does not establish biomedical efficacy,
safety, clinical actionability, or a cure.

## Code Change

The FSV branch now captures a fresh `fsv_snapshot = vault.snapshot()`
immediately before scanning Recurrence CF for the artifact. The report records
both snapshots:

- `initial_readback`: the snapshot used for the first Base/Recurrence assertions;
- `fsv_readback`: the current snapshot used for FSV artifact readback.

This keeps the FSV source of truth as persisted Aster CF bytes without asking a
latest-only recovery path for historical state it cannot serve.

## Verification

Passed:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z'
cargo test -p calyx-cli cmd::ingest::oracle_event_tests::batch_ingest_structures_oracle_recurrence_for_reverse_query -- --nocapture
```

Also passed:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z'
cargo test -p calyx-cli batch_ -- --nocapture
```

Repository gates:

```bash
cargo fmt -p calyx-cli
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

FSV artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1215-oracle-event-fsv-20260704T042100Z\issue885_oracle_event_readback.json
```

Readback summary:

```json
{
  "issue": 885,
  "fixed_by_issue": 1215,
  "recurrence": {
    "cf_rows": 1,
    "occurrences": 1,
    "first_t_secs": 1700000000
  },
  "reverse_query": {
    "cause_count": 1,
    "first_action_or_event": "What treats type 2 diabetes?",
    "first_domain": "endocrinology",
    "first_provisional": false,
    "first_confidence": 0.5
  },
  "snapshots": {
    "initial_readback": 4,
    "fsv_readback": 5
  }
}
```

## Conclusion

The oracle-event FSV branch now reads from a valid current snapshot and emits its
artifact under `CALYX_FSV_ROOT`. The broad batch suite also passes with FSV
artifact writing enabled, closing the caveat found during #1211 verification.
