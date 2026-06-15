# PH53 · T10 — Columnar/OLAP scan+aggregate root op

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/olap/` |
| **Issue** | #586 |

## Goal

Serve the columnar/OLAP root operation required by A19: scan Arrow/SoA column
bytes and compute count/sum/min/max/avg with optional group-by. The operator
must be cost-capped and fail closed on invalid plans, non-finite values, corrupt
chunks, and over-limit scans.

## Storage Surface

The current Aster column surface is slot-column materialization:

- `slot-column-manifest.json` records snapshot, row count, dimension, cx ids, and
  chunk hash.
- `slot-column.cxa1` stores f32 values in column-major Arrow-compatible layout:
  `CXA1 | version | rows | dim | col0 rows | col1 rows | ...`.

The OLAP scan path reads the manifest and mmaps `slot-column.cxa1` directly. The
standalone scan API does not need a vault handle, which proves the aggregate is
answered from column bytes rather than the row CF after materialization.

## FSV

Issue #586 FSV uses five deterministic rows in one dense slot:

```text
[10, 1, 0], [20, 2, 0], [30, 3, 1], [-5, 4, 1], [15, 5, 1]
```

For value column 0, the hand result is count 5, sum 70, min -5, max 30, avg 14.
Group-by column 2 yields group 0 sum 30/count 2 and group 1 sum 40/count 3.

Close evidence must include:

- the JSON report under `/home/croyse/calyx/data/fsv-issue586-*`;
- xxd readback of `slot-column.cxa1` showing `CXA1` and column-major payload;
- sha256 of manifest/chunk/test-output;
- edge cases for empty slot, invalid column, row limit, and group limit.
