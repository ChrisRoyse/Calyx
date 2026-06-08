# FSV Notes

FSV tools print source-of-truth bytes for a human or agent to inspect. They do
not emit verdicts such as pass/fail. A passing test is a claim; the bytes are the
verdict.

## Readback Convention

Every FSV gate should name the readback command that proves it. Current readback
surfaces include file bytes, vault trees, column-family rows, WAL records, and
SST levels:

```bash
calyx readback --hex <file>
calyx readback --vault-tree <dir>
calyx readback --cf <vault-root> <cf-name> [--prefix <hex-prefix>]
calyx readback --wal <vault-root>
calyx readback --level <vault-root> <cf-name>
```

Later phases extend `readback` for Ledger entries, metrics, and higher-level
engine artifacts. The command stays observational: it prints bytes, rows, or
listings and exits. The agent compares those bytes to the expected state and
records evidence in the GitHub issue.

## Synapse Mapping

Use Synapse as the perception and action surface for FSV:

1. `reality_baseline`: record the visible/process/file context before action.
2. `act_run_shell`: execute the trigger and the readback command on aiwonder.
3. `reality_audit` or a fresh readback: inspect the source-of-truth delta.
4. `find` / `read_text`: locate exact values in terminal output or files.
5. `capture_screenshot`: preserve GUI/Grafana/J-curve states when text is not
   enough.

For async operations, register a `reflex_register` watcher for the expected
source-of-truth state, then perform the same readback when it appears. For
dashboards, use screenshot plus AI vision in the already-open Chrome session.

FSV evidence belongs on the GitHub issue: command, source-of-truth path, expected
bytes/state, actual readback, edge cases, and cleanup proof for synthetic data.
