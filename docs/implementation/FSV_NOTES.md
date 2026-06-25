# FSV Notes

FSV tools print source-of-truth bytes for a human or agent to inspect. They do
not emit verdicts such as pass/fail. A passing test is a claim; the bytes are the
verdict.

## Readback Convention

Every FSV gate should name the readback command that proves it. Current readback
surfaces include file bytes, vault trees, column-family rows, WAL records, and
SST levels. The current CLI uses explicit `--vault` and typed subcommands for
higher-level Aster/Ledger surfaces:

```bash
calyx readback --hex <file>
calyx readback --vault-tree <dir>
calyx readback --cf <cf-name> --vault <vault-root>
calyx readback --cf ledger --vault <vault-root> [--seq <n>]
calyx readback cx-list --vault <vault-root>
calyx readback recurrence-series --vault <vault-root> --cx-id <CxId>
calyx readback periodic-recall --vault <vault-root> (--hour <0-23> | --day <0-6>) [--hour <0-23>] [--day <0-6>]
calyx readback dedup-audit --vault <vault-root> --cx-id <CxId>
calyx readback dedup-undo --vault <vault-root> --token <json>
calyx verify-chain --vault <vault-root> --range <start>..<end>
calyx readback --wal --vault <vault-root>
calyx readback --cf <cf-name> --level <level-dir>
```

Recent recurrence concurrency evidence (#621) uses the same surfaces against
the persistent aiwonder root
`/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`:

```bash
calyx readback recurrence-series --vault <root>/direct-append/vault --cx-id <CxId>
calyx readback --cf recurrence --vault <root>/direct-append/vault
calyx readback --cf base --vault <root>/direct-append/vault
calyx readback --wal --vault <root>/direct-append/vault
calyx verify-chain --vault <root>/direct-append/vault --range 0..1
```

The same readbacks are repeated for `<root>/ingest/vault` and
`<root>/failed-retry/vault`, with `BLAKE3SUMS.txt` verifying the persisted SST,
WAL, manifest, and JSON artifact bytes.

Later phases extend `readback` for metrics and higher-level engine artifacts.
The command stays observational unless explicitly named as an action subcommand
such as `dedup-undo`: it prints bytes, rows, or listings and exits. The agent
compares those bytes to the expected state and records evidence in the GitHub
issue.

## Bounded Output Protocol (#831)

Codex terminals have repeatedly crashed while agents generated or streamed large
FSV final-readback JSON, registry catalogs, and one-line Calyx JSON log records.
Treat terminal output volume as a reliability constraint until the host-side
root cause is proven.

Keep full evidence on aiwonder under the FSV root. In the Codex terminal and in
issue comments, emit only:

- artifact path;
- byte count;
- SHA256;
- command exit status;
- exact scalar or boolean leaves needed to prove the state.

Do not `cat`, `Get-Content -Raw`, `grep`, `tail`, or broad-`jq` a full
readback, registry, catalog, or log artifact into Codex output. `grep` and
`tail` are unsafe for one-line JSON producers because a single matching line can
be the whole catalog or report. Write the full readback to a file first, then
read back `wc -c`, `sha256sum`, and a bounded projection such as:

```bash
jq '{gate:.gate,total:.total,matched:.matched}' "$FSV/final-readback.json"
```

For nested reports, select scalar leaves rather than whole sections:

```bash
jq '.lenses[] | {name,bits:.bits_about,admitted}' "$FSV/assay-report.json"
```

The full artifact remains the source of truth; the terminal summary is only a
bounded pointer to the bytes.

Use `scripts/fsv_bounded.py` when a command might emit a large JSON/log/readback
payload. It captures stdout/stderr to files and prints only exit status, path,
bytes, SHA256, and selected scalar JSON leaves:

```bash
python3 scripts/fsv_bounded.py capture \
  --stdout "$FSV/final_readback.json" \
  --stderr "$FSV/final_readback.stderr" \
  --field accepted=.accepted \
  -- calyx readback some-large-surface --vault "$VAULT"

python3 scripts/fsv_bounded.py summarize "$FSV/final_readback.json" \
  --field accepted=.accepted \
  --field row_count=.row_count
```

Prefer this helper over `tee` in FSV scripts. `tee` persists bytes but still
streams them into the Codex terminal.

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
