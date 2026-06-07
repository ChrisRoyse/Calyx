# PH58 · T06 — Janitor — build artifacts, logs, temp files, datasets; disk-pressure guard

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/janitor.rs` (≤500) |
| **Depends on** | T05 (GC patterns established), PH43 (Anneal background task infrastructure) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7b` |

## Goal

Implement the bounded background janitor (managed by Anneal) that prunes operational buildup on
the single-NVMe `hotpool`: old `cargo` build artifacts, rotated/compressed logs, temp/scratch
files, and stale datasets — each with a configured TTL/size cap. Logs rotate by size+age
(`tracing-appender`/logrotate pattern), zstd-compress, then drop on TTL; bounded total log
bytes per service. Staged temp files written inside the destination dataset (avoid `EXDEV`,
PRD `24 §7b`). Datasets on cold `archive`, unused pruned per MANIFEST. Janitor is rate-limited
below serving, reversible within the recovery window, and Ledger-logged. Nothing accumulates
silently.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct JanitorConfig { log_max_bytes: u64, log_ttl: Duration, build_artifact_keep_releases: usize, temp_ttl: Duration, dataset_prune_by_manifest: bool }` in `janitor.rs`
- [ ] Implement `Janitor::new(config: JanitorConfig, clock: Arc<dyn Clock>) -> Self`
- [ ] Implement `Janitor::prune_logs(&self) -> GcResult` — scans `$CALYX_HOME/logs/` and per-service log dirs; compresses files older than `log_rotation_age` with zstd (`zstd::encode_all`); deletes compressed files older than `log_ttl`; enforces `log_max_bytes` total across all services; returns `bytes_freed`
- [ ] Implement `Janitor::prune_build_artifacts(&self) -> GcResult` — lists `$CALYX_HOME/target/` release dirs by mtime; keeps last `build_artifact_keep_releases`; removes older ones; never removes the currently running binary's artifacts (check `/proc/self/exe`)
- [ ] Implement `Janitor::prune_temp_files(&self) -> GcResult` — scans `$CALYX_HOME/*/.tmp/` and dataset-local temp dirs; removes files older than `temp_ttl`; verifies temp files are inside their destination dataset (no cross-mount strays); reports `CALYX_IO_ERROR` if a stray temp crosses a ZFS dataset boundary
- [ ] Implement `Janitor::prune_datasets(&self, manifest: &DatasetManifest) -> GcResult` — removes dataset dirs not in the MANIFEST; raw datasets older than their parsed/quantized form and not needed by any current test run
- [ ] Implement `Janitor::run_tick(&self, disk_pressure: &DiskPressureGuard)` — calls all prune methods in priority order (temp first, logs second, artifacts third, datasets fourth); stops early if disk_pressure drops below high-water; rate-limited (max 100 MB/s I/O during prune); Ledger-logs each prune action (stub entry until PH35)
- [ ] Emit Prometheus: `calyx_janitor_bytes_freed_total`, `calyx_janitor_log_bytes`, `calyx_janitor_artifact_bytes`, `calyx_janitor_temp_bytes`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: create 5 log files; 3 older than `log_ttl`, 2 recent; `prune_logs()` deletes exactly 3; `bytes_freed` matches sum of their sizes
- [ ] unit: log rotation — create 10 MB log file older than `log_rotation_age`; `prune_logs()` compresses it (output is valid zstd); compressed size < 10 MB
- [ ] unit: `prune_build_artifacts` with `keep_releases=2`; 5 artifact dirs by mtime → 3 removed; current binary dir not removed (mock `/proc/self/exe`)
- [ ] unit: `prune_temp_files` — 4 temp files older than `temp_ttl`, 2 recent → 4 removed; a temp file at `/tmp/stray` (outside dataset) → `CALYX_IO_ERROR` reported, file NOT deleted (cannot safely remove strays)
- [ ] unit: `prune_datasets` with MANIFEST listing 3 datasets; 5 dirs present → 2 removed; MANIFEST entries preserved
- [ ] edge: `log_max_bytes=0` → all log files deleted on every tick (zero-byte budget enforces maximum aggression)
- [ ] edge: disk pressure already below high-water before janitor starts → `run_tick` still runs full prune cycle (proactive maintenance)
- [ ] fail-closed: zstd compression fails (corrupt file) → skip that file, log `CALYX_IO_ERROR`, continue with others; no panic; `bytes_freed` reflects only successfully pruned files

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_janitor_bytes_freed_total` counter and `du -sh $CALYX_HOME/logs $CALYX_HOME/target` before/after janitor run on aiwonder
- **Readback:**
  ```
  du -sh $CALYX_HOME/logs $CALYX_HOME/target
  calyx readback --metric janitor_bytes_freed_total
  calyx readback --metric janitor_log_bytes
  ```
- **Prove:** let logs accumulate to 2× `log_max_bytes`; run janitor; verify `du` of `$CALYX_HOME/logs` drops to ≤ `log_max_bytes`; `janitor_bytes_freed_total` increments; let `target/` accumulate 4 release dirs with `keep_releases=2`; verify 2 removed; `du` decreases. Attach readback output to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
