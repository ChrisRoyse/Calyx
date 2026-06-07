# PH56 · T06 — Disk-pressure guard — `CALYX_DISK_PRESSURE`, spill cold to archive

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/pressure.rs` (≤500) |
| **Depends on** | T04 (bounded memtable + backpressure established) · T05 (mmap/cold exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §6`, `24 §7b`, `24 §7` hazard 17 |

## Goal

Implement a disk-pressure guard that monitors `hotpool` NVMe free space and halts write
acceptance before the single NVMe fills to corruption. At the 85% high-water mark, stop
accepting new writes (return `CALYX_DISK_PRESSURE`, fail closed) and trigger a spill of cold
data to the `archive` dataset. Also enforce the operational hygiene that staged temp files are
written inside the destination dataset (avoiding `EXDEV` on ZFS rename — PRD `24 §7b`). This
defends hazard 17 (disk full on `hotpool`).

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct DiskPressureGuard { hotpool_path: PathBuf, high_water_ratio: f64, clock: Arc<dyn Clock> }` in `pressure.rs`
- [ ] Implement `DiskPressureGuard::check(&self) -> Result<DiskStatus, CalyxError>` — calls `statvfs` (via `nix::sys::statvfs::statvfs`) to get `f_bavail` and `f_blocks`; computes `used_ratio = 1.0 - (f_bavail as f64 / f_blocks as f64)`; if `used_ratio >= high_water_ratio` returns `CALYX_DISK_PRESSURE` with `used_ratio` in the error payload
- [ ] Implement `DiskPressureGuard::is_under_pressure(&self) -> bool` — thin wrapper over `check()`; returns true if `CALYX_DISK_PRESSURE` would fire; used by write acceptors
- [ ] Add `CALYX_DISK_PRESSURE` to `calyx-core` error catalog if not present; remediation: "hotpool NVMe at high-water; writes halted; spill cold data to archive or delete stale artifacts"
- [ ] Implement `SpillTrigger::request_spill(&self)` — sends a channel message to the janitor/GC background task to move cold SST files from `hotpool` to `archive`; non-blocking (fire and forget); logs the request via `tracing`
- [ ] Enforce temp-file hygiene: add `TempFile::in_dataset(destination_dir: &Path) -> TempFile` helper that creates temp files inside the destination directory (not `/tmp`), so ZFS `rename` is atomic (no `EXDEV`)
- [ ] Wire `DiskPressureGuard::is_under_pressure` into the memtable `write()` path (checked before every write accept)
- [ ] Emit Prometheus counter `calyx_disk_pressure_events_total` on each `CALYX_DISK_PRESSURE` firing

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock `statvfs` returning 90% used → `is_under_pressure()` returns true; `check()` returns `CALYX_DISK_PRESSURE` with `used_ratio ≈ 0.90` in payload
- [ ] unit: mock 80% used → `is_under_pressure()` returns false; `check()` returns `Ok(DiskStatus::Ok { used_ratio: 0.80 })`
- [ ] unit: mock exactly `high_water_ratio` (85%) → `is_under_pressure()` returns true (boundary is inclusive)
- [ ] unit: `TempFile::in_dataset(dir)` creates file inside `dir`; verify `parent() == dir` (no `/tmp` escape)
- [ ] unit: `SpillTrigger::request_spill` sends message on channel — verify receiver gets the `SpillRequest` message within a sync test
- [ ] edge: `statvfs` fails (e.g., invalid path) → returns `CALYX_IO_ERROR`, does not panic; caller treats this as "pressure unknown → reject" (fail closed)
- [ ] edge: `f_blocks == 0` (unusual FS) → `used_ratio = 0.0` (treat as empty, not divide-by-zero)
- [ ] fail-closed: `DiskPressureGuard` wired into memtable; mock 90% full; attempt write → `CALYX_DISK_PRESSURE` returned; mock drops to 70%; write succeeds

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `disk_free` bytes on `hotpool` (`df -h /hotpool` or `zfs list -o name,avail hotpool`) and `calyx_disk_pressure_events_total` counter
- **Readback:** fill `hotpool` to 86% with a test file; `calyx readback --metric disk_pressure_events_total` must show > 0; attempt a write — must receive `CALYX_DISK_PRESSURE`; delete the test file; write must succeed
- **Prove:** before the test, `disk_pressure_events_total == 0`; after filling to 86%, the counter increments and writes are rejected cleanly (no data corruption, no OOM). `df -h /hotpool` shows ≤ 100% used at all times.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
