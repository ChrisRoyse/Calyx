# PH56 T06 - Disk-Pressure Guard

| Field | Value |
|---|---|
| Phase | PH56 - Bounded caches/queues/memtables + arenas/pools |
| Stage | S13 - Resource, GC & Reliability Hardening |
| Crate | `calyx-aster` |
| Primary file | `crates/calyx-aster/src/pressure.rs` |
| Depends on | T04 bounded memtable + T05 mmap/cold columnar access |
| PRD | `docs/dbprdplans/24_MEMORY_GC_RELIABILITY.md` sections 6, 7b, hazard 17 |
| Issue | GitHub #473 |

## Delivered

- `DiskPressureGuard` samples the configured hotpool path with Unix `statvfs` through `nix::sys::statvfs`.
- `check()` computes `used_ratio = 1.0 - f_bavail / f_blocks`, treats `f_blocks == 0` as `0.0`, and returns `CALYX_DISK_PRESSURE` when `used_ratio >= high_water_ratio`.
- statvfs failures return module-local `CALYX_IO_ERROR`; write admission fails closed on that error.
- Durable write admission checks the guard before WAL append, MVCC commit, and router mutation.
- `SpillTrigger::request_spill()` sends a non-blocking channel request and logs via `tracing`.
- `TempFile::in_dataset(destination_dir)` creates staging files under the destination dataset, not `/tmp`.
- `calyx_disk_pressure_events_total` is emitted in Prometheus resource metrics and increments when `CALYX_DISK_PRESSURE` fires.

## Tests

- Mock 90 percent used -> `CALYX_DISK_PRESSURE`, payload includes `used_ratio=0.900000`.
- Mock 80 percent used -> accepted with `DiskStatus::Ok`.
- Mock exact 85 percent high-water -> rejected; boundary is inclusive.
- Mock `f_blocks == 0` -> `used_ratio = 0.0`.
- Mock statvfs failure -> `CALYX_IO_ERROR`; boolean wrapper fails closed.
- Temp file parent is the destination dataset and is removed on drop.
- Spill request reaches the receiver channel.
- Linux write-path regression proves pressure rejects before WAL append and a later 70 percent sample allows the write.

## aiwonder Gates

Branch: `issue473-ph56-disk-pressure`
Implementation commit: `47ef6e7`

Ran on aiwonder from `/home/croyse/calyx/repo`:

- `cargo fmt --all -- --check`
- corrected tracked `.rs` line gate: no file over 500 lines
- `cargo check -p calyx-aster`
- `cargo test -p calyx-aster pressure -- --nocapture` - 15 passed
- `cargo test -p calyx-aster metrics_text_renders_prometheus_conventions -- --nocapture` - 1 passed
- `cargo test -p calyx-aster --test issue473_disk_pressure -- --nocapture` - 1 passed, 1 ignored FSV driver
- `cargo clippy -p calyx-aster --all-targets -- -D warnings`

## FSV Evidence

Evidence root:

`/home/croyse/calyx/data/fsv-issue473-disk-pressure-20260614T180237Z`

`/home/croyse/calyx/data` is a symlink to `/zfs/hot/calyx`, so the vault and evidence files were on the hotpool dataset.

Source of truth reads:

- `df-before.txt`: hotpool mounted as `hotpool/calyx`; available bytes `1610551787520`, use `1%`.
- `df-after.txt`: same hotpool mount and available bytes `1610551787520`, proving the synthetic test did not fill or corrupt the pool.
- `metrics-before.prom`: `calyx_disk_pressure_events_total{vault="issue473"} 0`, `calyx_wal_bytes{vault="issue473"} 0`.
- `metrics-after-pressure.prom`: `calyx_disk_pressure_events_total{vault="issue473"} 1`, `calyx_wal_bytes{vault="issue473"} 73`.
- `issue473-disk-pressure-readback.json`: pressure error code `CALYX_DISK_PRESSURE`; invalid path error code `CALYX_IO_ERROR`; spill request path equals the vault path; temp parent matched dataset; temp file removed on drop.
- WAL readback: `wal_before_reject = 73`, `wal_after_reject = 73`; rejected `blocked-key` and `blocked-value` strings were absent from the WAL.
- Final WAL bytes were `152` only after pressure was cleared and `clear-key -> clear-value` was accepted.

The FSV used a safe threshold-crossing proof instead of physically filling the 1.6 TB hotpool: the real `statvfs` sample reported `used_ratio=0.001982`, and the pressure guard was configured with `high_water_ratio=0.000000` to force the same comparison path without consuming hundreds of GB of NVMe.
