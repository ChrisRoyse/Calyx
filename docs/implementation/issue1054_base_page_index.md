# Issue 1054: Indexed Base CF Paging for cx-list --limit

## Root Cause

`calyx readback cx-list --limit <n>` applied the limit after `latest_cf_rows(Base)`, so a one-row read still walked every Base SST and WAL record. On the large aiwonder vault this loaded 198,993 Base rows from 99,498 SST files before returning one row. That made bounded readback look successful while doing unbounded storage work.

## Design

The source of truth for bounded paging is now the vault-local `base_page_index_v1/` directory:

- `manifest.json` records the index format/version, ledger head height and tip hash, Base SST count, WAL record count, total/live/tombstone counts, page size, and page SHA-256 values.
- `page-XXXXXXXX.json` files store sorted Base keys with value SHA-256, tombstone state, and the physical SST or WAL source needed to verify selected rows.
- `cx-list --limit` reads this index and fails closed with `CALYX_BASE_PAGE_INDEX_MISSING`, `CALYX_BASE_PAGE_INDEX_STALE`, or `CALYX_BASE_PAGE_INDEX_CORRUPT` instead of falling back to a full Base scan.
- `--rebuild-base-page-index` is the explicit full-scan path. It streams WAL records instead of materializing the whole WAL and emits progress JSONL while writing the checked index.
- Indexed reads verify the current ledger head, page SHA, tombstone state, value SHA, and selected physical source bytes. WAL sources are read by exact segment/sequence/offset range rather than replaying the whole WAL.

## Operator Usage

```bash
calyx readback cx-list --vault <vault> --limit 1 --rebuild-base-page-index --progress-jsonl stderr
calyx readback cx-list --vault <vault> --limit 1 --progress-jsonl stderr
```

Use `--base-page-index-page-size <n>` only when deliberately changing page sizing. The default is 1024 index entries per page.

## Full State Verification

aiwonder FSV root: `/home/croyse/calyx/fsv/issue1054-base-page-index-20260630T225402Z`

Real-vault source of truth: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/base_page_index_v1/manifest.json` plus its page files and `ledger_head/current.json`.

Observed real-vault state after streaming rebuild:

- `manifest.json` SHA-256: `60438b60f8d8ebc13a04d621d62e8f2cd4c515bed5076acb8ffe9f2b7ff5b1a5`
- Ledger head SHA-256: `83fde0df9963b57ee6fbf7232afd76dfad7c808a338d2e3462d76560810c4871`
- Ledger head height: `647374`
- Total/live entries: `198993` / `198993`
- Base SST files: `99498`
- WAL records streamed during rebuild: `652079`
- Pages: `195`; files in index directory: `196` including manifest
- Rebuild plus limit read: rc `0`, elapsed `47.42s`, max RSS `114768 KB`
- Separate indexed read without rebuild: rc `0`, elapsed `0.01s`, max RSS `23008 KB`
- The separate read stdout SHA matched the rebuild command stdout SHA: `35a7caa61581e69e5b9192845e8cc6c2d00a7d2b582bf433322471622c1568a7`.
- First page SHA in manifest matched the physical page SHA: `45a2e54d8561f281c9079667c05376f2078ad19411e71ee0a8e31ae4423f0f31`.

Boundary cases under the same FSV root:

- Missing real-vault index before rebuild: rc `2`, code `CALYX_BASE_PAGE_INDEX_MISSING`, stdout empty, index remained absent.
- Empty synthetic vault: explicit rebuild/read returned rc `0`, stdout `[]`, and persisted an index manifest with zero entries/pages.
- Stale synthetic ledger head: rc `2`, code `CALYX_BASE_PAGE_INDEX_STALE`, manifest remained unchanged.
- Corrupt synthetic page SHA: rc `2`, code `CALYX_BASE_PAGE_INDEX_CORRUPT`, page bytes and manifest remained available for inspection.
