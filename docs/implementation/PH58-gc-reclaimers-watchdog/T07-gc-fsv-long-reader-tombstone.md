# PH58 · T07 — GC FSV: long reader aborted → version GC'd, disk flat; tombstone ratio bounded

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster`, `calyx-anneal` |
| **Files** | `crates/calyx-aster/tests/soak_ph58.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 (all GC infrastructure complete) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §3`, `24 §4`, `24 §7` hazard 3, hazard 6 |

## Goal

Prove on aiwonder — by reading the actual metric bytes and disk usage — that:
(1) a long reader is aborted on lease expiry → its old version is GC'd → disk usage is flat;
(2) a delete-heavy workload produces a bounded tombstone ratio after GC sweeps;
(3) logs and build artifacts remain bounded after janitor runs.
This is the phase FSV gate. The byte-level readback is the verdict.

## Build (checklist of concrete, code-level steps)

- [ ] Write `soak_ph58.rs` with three distinct sub-tests, each runnable independently via `cargo test soak_ph58::long_reader` etc.

**Sub-test 1 — long_reader_aborted_version_reclaimed:**
- [ ] Open a vault; ingest 1e4 constellations at seqs 1–10000; start a long reader at seq 5000 (lease duration = 200 ms)
- [ ] While the reader is open: ingest 1e4 more constellations at seqs 10001–20000; verify `oldest_pinned_seq_gap` ≥ 15000
- [ ] Wait for lease to expire (advance mock clock by 250 ms); run `check_and_abort_expired()`; verify `CALYX_READER_LEASE_EXPIRED` returned to the reader
- [ ] Run `SnapshotGcReclaimer::run_once()`; collect `gc_bytes_freed_total` before and after; verify `delta > 0`
- [ ] Read `df -h /hotpool` before and after GC; serialize disk_free_before and disk_free_after to `target/ph58_long_reader.json`

**Sub-test 2 — tombstone_ratio_bounded:**
- [ ] Ingest 5e4 constellations; delete 3e4 of them; run `tombstone_ratio()` → assert > 0.5
- [ ] Run `CompactionGcReclaimer::run_once()` 3 times (rate-limited); verify `tombstone_ratio` ≤ 0.1 after 3 passes
- [ ] Collect `calyx_tombstone_ratio` series; serialize to `target/ph58_tombstone.json`

**Sub-test 3 — logs_and_artifacts_bounded:**
- [ ] Create synthetic log files and build artifact dirs in `$CALYX_HOME/logs` and `$CALYX_HOME/target/test_artifacts`
- [ ] Run `Janitor::run_tick()`; verify `du` of log dir ≤ `log_max_bytes`; verify artifact dirs pruned to `keep_releases`
- [ ] Serialize `janitor_bytes_freed_total` to `target/ph58_janitor.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] sub-test 1: `gc_bytes_freed_total` after long-reader abort > 0 (version actually reclaimed); `disk_free_after >= disk_free_before` (disk flat or improved)
- [ ] sub-test 1: `reader_lease_expired_total == 1` (exactly one reader aborted)
- [ ] sub-test 2: `tombstone_ratio ≤ 0.1` after 3 compaction passes (verified from `ph58_tombstone.json`)
- [ ] sub-test 2: serving (concurrent read ops during compaction) p99 < 2× baseline (anti-storm worked)
- [ ] sub-test 3: `janitor_bytes_freed_total > 0`; `du $CALYX_HOME/logs ≤ log_max_bytes`
- [ ] soak: zero panics across all three sub-tests (wrapped in `std::panic::catch_unwind`)
- [ ] edge: run all three sub-tests sequentially; total run time < 5 minutes on aiwonder (GC must not be excessively slow)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph58_long_reader.json`, `target/ph58_tombstone.json`, `target/ph58_janitor.json` produced on aiwonder; and the three Prometheus metric readbacks
- **Readback:**
  ```
  cargo test --release --test soak_ph58 -- --nocapture 2>&1 | tee /tmp/ph58_soak.log
  calyx readback --metric reader_lease_expired_total
  calyx readback --metric gc_bytes_freed_total
  calyx readback --metric tombstone_ratio
  calyx readback --metric janitor_bytes_freed_total
  df -h /hotpool
  ```
- **Prove:** from `ph58_long_reader.json`: `disk_free_after >= disk_free_before` (version GC'd, disk flat); `reader_lease_expired_total >= 1`; from `ph58_tombstone.json`: final `tombstone_ratio <= 0.1`; from `ph58_janitor.json`: `janitor_bytes_freed_total > 0`. Attach all three JSON files + the `calyx readback` + `df` output as FSV evidence to the PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
