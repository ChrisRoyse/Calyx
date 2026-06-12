# PH71 - T07 - Issue #612 flipped-read latency and widened control-plane FSV

| Field | Value |
|---|---|
| Phase | PH71 - V0 shadow + V1 flip + V2 calyx-only |
| Stage | S19 - Leapable Vault Swap |
| Crate | `calyx-cli` |
| Files | `crates/calyx-cli/src/leapable/issue612_fsv.rs` (<=500), `crates/calyx-cli/tests/leapable_issue612.rs` (<=500) |
| GitHub issue | `#612` |
| Depends on | T03 read flip, T06 control-plane snapshot contract |

## Goal

Close the PH71 blind spot from issue #612 with a verifier that reads physical
latency sample files and PostgreSQL dump bytes instead of accepting a test verdict.
The verifier proves two things:

- flipped Calyx reads do not regress p99 latency beyond 105% of the sqlite-vec
  baseline p99
- the control-plane snapshot covers `creator_databases`, `queries`, `billing`,
  `marketplace`, and `outbox`, with byte-identical before/after dumps

## Command

```bash
calyx leapable issue612-fsv \
  --baseline-latency baseline_latency.json \
  --flipped-latency flipped_latency.json \
  --pg-before pg_before \
  --pg-after pg_after \
  --out evidence.json
```

Latency inputs are JSON files with this shape:

```json
{
  "path": "sqlite-vec",
  "samples_us": [1000, 1001, 1002]
}
```

The PG snapshot directories must contain these dump files:

- `creator_databases.dump`
- `queries.dump`
- `billing.dump`
- `marketplace.dump`
- `outbox.dump`

## Fail-Closed Cases

- `CALYX_LATENCY_SAMPLE_EMPTY`: either latency sample file has no samples
- `CALYX_LATENCY_REGRESSION`: flipped p99 is greater than baseline p99 * 1.05
- `CALYX_PG_SNAPSHOT_INCOMPLETE`: any required dump file is absent from either
  before or after
- `CALYX_PG_STATE_CHANGED`: any required dump file differs byte-for-byte between
  before and after

The verifier writes no evidence artifact on any fail-closed path.

## FSV

Run on aiwonder against a fresh root under `/home/croyse/calyx/data`. For closure
evidence, manually read the source-of-truth files after the command runs:

- `evidence.json`
- `issue612-fsv-readback.json`
- `BLAKE3SUMS.txt`
- `baseline_latency.json`
- `flipped_latency.json`
- `pg_before/*.dump`
- `pg_after/*.dump`
- edge-case directories under `edges/`

Known deterministic fixture values used by the ignored FSV test:

- baseline samples are `1000..1099`, so p99 is `1098`
- flipped samples are `900..999`, so p99 is `998`
- allowed flipped p99 is `1152.9`
- all five required PG dump files are identical on the happy path

The manual readback must include `sha256sum` or BLAKE3 hashes for the evidence,
latency sample files, and dump files, plus a direct JSON read showing:

```json
{
  "baseline_p99_us": 1098,
  "flipped_p99_us": 998,
  "matched_tables": 5,
  "all_hashes_match": true
}
```

Edges must show before and after artifact state as absent for empty latency,
latency regression, missing marketplace, and changed outbox cases.

## Done When

- `cargo check -p calyx-cli`, `cargo test -p calyx-cli`, and
  `cargo clippy -p calyx-cli --all-targets -- -D warnings` pass on aiwonder
- every `.rs` source/test file stays at or below 500 lines
- ignored FSV test writes the artifact root on aiwonder
- manual readback confirms the p99 math, widened PG table set, byte-identical
  dumps, and fail-closed edges
- GitHub issue #612 has a RESOLVED comment with the evidence root and hashes
