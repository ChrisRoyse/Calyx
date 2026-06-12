# Calyx dataset MANIFEST

This file documents the canonical catalog schema (PH69 T01, issue #551). The
**live catalog** — the Source of Truth — lives on aiwonder at
`$CALYX_DATASET_ROOT/MANIFEST.md` (default `/zfs/archive/calyx/datasets/MANIFEST.md`)
and is machine-written by `scripts/verify_dataset.sh register`. Do not hand-edit
rows there or here; this repo copy carries the schema and an example row only.

Each dataset additionally carries a `<name>/manifest.json` with per-file
`sha256`/`bytes`/`rows`, written atomically by the same `register` call. The
catalog row is the aggregate; `manifest.json` is the per-file detail; both are
re-verified from raw bytes by `scripts/verify_dataset.sh <name|ALL>`, which
fails closed with exact `CALYX_DATASET_*` codes on any drift.

Column semantics:

- **name** — dataset directory name under `$CALYX_DATASET_ROOT`.
- **source** — download URL or `huggingface:<org>/<repo> <config>`.
- **revision** — pinned upstream version: HF commit hash, release tag, or date.
  Never float on a branch; upstream datasets update silently.
- **sha256** — dataset digest: sha256 over the sorted lines
  `"<file_sha256>  <relpath>\n"` of every data file (everything in the dataset
  dir except `manifest.json`, hidden entries, and `*.tmp`). Any added, removed,
  or edited byte changes this value.
- **rows** — sum of record counts over counted files (`csv`/`tsv`: records
  minus one header row, `jsonl`: non-empty lines, `parquet`: metadata
  `num_rows`). `register --rows-from <globs>` restricts counting, e.g. to
  parquet splits when derived TSVs of the same records sit alongside.
- **bytes** — sum of data-file sizes in bytes.
- **license** — upstream license (A34: free sources only).
- **what it tests** — which lens family / intelligence metric consumes it.

<!-- template: name=dataset dir | source=URL or huggingface:<org>/<repo> <config> |
     revision=pinned commit/version | sha256=digest of sorted '<file_sha256>  <relpath>'
     lines over all data files | rows=sum of record counts of counted files |
     bytes=sum of data-file sizes | license=upstream license | tests=what it tests -->

| name | source | revision | sha256 | rows | bytes | license | tests |
|---|---|---|---|---|---|---|---|
| synthetic_fixture | self-test inline fixture | v1 | ed59b3298d0d2c5c56bfbb30c2438f87c54859c1ee785ea6020649421564e8d3 | 3 | 32 | n/a (synthetic) | verify_dataset.sh self-test (example row — live rows are on aiwonder) |
