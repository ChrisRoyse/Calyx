# PH10 · T04 — kill -9 crash drill (3 points) + corrupt-shard FSV

| Field | Value |
|---|---|
| **Phase** | PH10 — Manifest + atomic swap + crash recovery |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/manifest/tests.rs` (≤500), `crates/calyx-cli/src/main.rs` |
| **Depends on** | T02 (replay reconstruction), T03 (open constructor) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §7` |

## Goal

The PH10 FSV gate: prove on aiwonder that the vault recovers byte-exact to the
last acked record after a `kill -9` at three specific crash points, and that
flipping a byte in a base CF SST causes `calyx readback` to return
`CALYX_ASTER_CORRUPT_SHARD` rather than silently returning wrong data.

Current status: the Stage 1 crash/corrupt-shard gate is FSV-signed-off on
aiwonder. `degraded_rebuildable=true` remains a PH44 self-heal responsibility;
PH10 verifies corrupt base shards fail closed and base reads do not silently
return wrong bytes.

## Build (checklist of concrete, code-level steps)

- [x] Add `calyx crash-drill --vault <path> --point <point>` CLI subcommand where
  `<point>` is one of: (1) `before-wal-fsync` — write WAL bytes but return before
  `sync_data()`; (2) `after-wal-before-commit` — WAL fsynced but
  `commit_batch` not yet called; (3) `after-commit-before-manifest` — `commit_batch`
  done, manifest write not yet started. Each point simulates the crash by
  exiting the process with `std::process::exit(1)` at the correct location.
- [x] For each crash point, the drill:
  1. Puts N known constellations (N-1 fully acked, the Nth mid-flight).
  2. Hits the crash point.
  3. Spawns `AsterVault::open` (recovery).
  4. `get` all N-1 constellations: byte-exact.
  5. `get` the Nth: must return Err (not found or stale derived).
- [x] Add `calyx corrupt-shard --vault <path> --cf base --byte-offset <N>` CLI
  subcommand that flips one byte at offset N in the first SST of the `base` CF.
- [x] After `corrupt-shard`, `calyx readback --cf base` must return
  `CALYX_ASTER_CORRUPT_SHARD` for the affected record (not silently skip it).
- [x] Keep PH10 fail-closed on corrupt base shards; PH44 owns setting
  `degraded_rebuildable=true` for rebuildable derived-CF corruption.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (subprocess): crash at point 1 → recovery → N-1 records found, Nth absent.
- [x] unit (subprocess): crash at point 2 → recovery → N-1 records found (WAL
  record for Nth exists but is orphaned — re-apply returns idempotent if committed,
  or absent if not).
- [x] unit (subprocess): crash at point 3 → recovery → all N records found (WAL
  record was fsynced and includes the Nth; commit_batch was called; manifest not
  yet written but WAL replay re-applies).
- [x] unit: corrupt SST byte → `CALYX_ASTER_CORRUPT_SHARD` on `calyx readback`.
- [x] edge (≥3): (1) crash with empty WAL → empty recovery, no error; (2) crash
  after manifest write → all N records durable; (3) corrupt byte in index section
  of SST → corrupt shard on open (not on read).
- [x] fail-closed: corrupt shard → `code == "CALYX_ASTER_CORRUPT_SHARD"`;
  `points_at_restore == true` or message contains `"restore"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment and base CF SST at `/home/croyse/calyx/test-vault/`.
- **Readback (crash drill):**
  ```
  calyx crash-drill --vault /home/croyse/calyx/test-vault --point after-wal-before-commit
  calyx recover --vault /home/croyse/calyx/test-vault
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
  Expected: `calyx recover` prints `last_recovered_seq = N`; `calyx readback`
  shows N constellation rows.
- **Readback (corrupt shard):**
  ```
  calyx corrupt-shard --vault /home/croyse/calyx/test-vault --cf base --byte-offset 100
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
  Expected: output contains `CALYX_ASTER_CORRUPT_SHARD` and a message pointing
  to the restore path. Screenshot posted to PH10 GitHub issue.
- **Prove:** (crash) before→after delta: WAL segment size unchanged; base SST
  contains exactly N-1 complete rows (point 1) or N rows (point 3). (corrupt)
  error code `CALYX_ASTER_CORRUPT_SHARD` is printed; no wrong data is silently
  returned.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH10 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
