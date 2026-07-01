# Issue 1053 - Large Vault Verify Progress

## Problem

On the aiwonder clinical corpus vault, two read-only inspection commands could appear stuck:

- `calyx verify-chain <vault>` materialized the full Ledger CF before checking any rows.
- `calyx readback cx-list --vault <vault> --limit 1 --include-slots` decoded Base rows first, then loaded whole per-slot CF maps for slot payload readback.

Both paths hid the active phase from operators. They also made it hard to distinguish slow physical reads from a broken process.

## Fix

- `verify-chain <vault>` now streams physical Ledger rows in batches and emits optional JSONL progress to stderr or a requested file with `--progress-jsonl`.
- `verify-chain` accepts `--time-budget-ms` and fails closed with `CALYX_CLI_TIMEOUT` if the command exceeds the requested budget at a checked phase.
- Direct absolute vault paths resolve directly, so physical readback does not require `CALYX_HOME` when the path exists.
- `readback cx-list` accepts `--progress-jsonl` and `--time-budget-ms`.
- `readback cx-list --include-slots` no longer materializes every `slot_NN` CF. It resolves concrete slot payloads by the constellation provenance sequence and keeps Base `Absent` placeholders as `payload_source: "base_absent"` without touching slot SSTs.

Progress and diagnostics stay off stdout. Command data remains parseable stdout JSON, while long-running status goes to stderr or the explicit progress path.

## Operational Notes

Use bounded diagnostics when inspecting the real clinical vault:

```bash
calyx verify-chain /path/to/vault \
  --progress-jsonl stderr \
  --time-budget-ms 180000 \
  --batch-size 8192

calyx readback cx-list \
  --vault /path/to/vault \
  --limit 1 \
  --include-slots \
  --progress-jsonl stderr \
  --time-budget-ms 170000
```

For a known constellation, prefer the targeted form:

```bash
calyx readback cx-list \
  --vault /path/to/vault \
  --cx-id <cx_id> \
  --include-slots \
  --progress-jsonl stderr \
  --time-budget-ms 30000
```

`cx-list --limit` no longer has to scan the whole Base CF after `docs/implementation/issue1054_base_page_index.md`: bounded reads use the persisted Base page index and fail closed when that index is missing, stale, or corrupt.

## FSV

Issue 1053 was verified on aiwonder against:

`/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`

The manual evidence root is:

`/home/croyse/calyx/fsv/issue1053-verify-progress-20260630T215357Z`

The evidence includes command stdout/stderr, exit codes, elapsed times, ledger-head readback, and before/after state for happy path plus edge cases.
