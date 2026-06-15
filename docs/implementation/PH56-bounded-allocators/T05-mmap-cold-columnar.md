# PH56 T05 - mmap cold/columnar access - OS page cache, never full vault in heap

| Field | Value |
|---|---|
| **Phase** | PH56 - Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 - Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mmap_col.rs` (`<=500`) |
| **Depends on** | T04 bounded memtable; PH11 SSTable/compaction layout |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24` sections 1 and 5; `dbprdplans/04` section 3 |

## Goal

Provide `MmapColumn`, a memory-mapped accessor for cold and columnar Aster data
(SST slot columns, ANN graph files, panel codebooks) where the OS page cache is
the cache and Calyx never holds the full vault in heap. Opened column files are
mapped read-only; access is a pointer dereference; eviction is managed by the
kernel. This removes a class of heap-OOM failures on large vaults and enables
streaming for VRAM in PH57.

## Build

- [x] Define `struct MmapColumn { mmap: memmap2::Mmap, path: PathBuf, file_len: usize }` in `mmap_col.rs`.
- [x] Implement `MmapColumn::open(path: &Path) -> Result<Self>` with read-only `memmap2::MmapOptions::new().map(&file)`.
- [x] Return `CALYX_NOT_FOUND` for nonexistent or empty files and `CALYX_IO_ERROR` for mmap/open failures.
- [x] Implement `read_slice(offset, len)` with checked bounds and `CALYX_BOUNDS_EXCEEDED` on violation.
- [x] Implement `read_f32_slice(offset, count)` with checked byte length and f32 alignment.
- [x] Implement `prefetch(offset, len)` with best-effort `MADV_WILLNEED`.
- [x] Implement `drop_pages(offset, len)` with best-effort `MADV_DONTNEED` through `memmap2::UncheckedAdvice`.
- [x] Add a `primarycache=metadata` ZFS advisory note.
- [x] Document that mapped files are immutable and must not be truncated while live.
- [x] Wire `MmapColumn::open` into SST reads, materialized slot-column reads, and OLAP slot-column aggregate scans.

## Tests

- [x] Unit: 1024 known bytes round-trip through `read_slice(0, 1024)`.
- [x] Unit: four known f32 values round-trip through `read_f32_slice(0, 4)`.
- [x] Unit: `read_slice(1020, 8)` on a 1024-byte file returns `CALYX_BOUNDS_EXCEEDED`.
- [x] Unit: `read_f32_slice(3, 1)` returns `CALYX_BOUNDS_EXCEEDED`.
- [x] Unit: nonexistent path returns `CALYX_NOT_FOUND`.
- [x] Edge: zero-length file returns `CALYX_NOT_FOUND`.
- [x] Edge: `prefetch` and `drop_pages` are nonfatal on valid and invalid ranges.
- [x] Reader integration: SST, slot-column, and OLAP focused tests pass on aiwonder.

## FSV

**Source of truth:** bytes and RSS on aiwonder:

- `/home/croyse/calyx/data/fsv-issue472-mmap-20260614T173632Z/cold-column-1g.bin`
- `/home/croyse/calyx/data/fsv-issue472-mmap-20260614T173632Z/f32-column.bin`
- `/home/croyse/calyx/data/fsv-issue472-mmap-20260614T173632Z/empty-column.bin`
- `/home/croyse/calyx/data/fsv-issue472-mmap-20260614T173632Z/issue472-mmap-fsv-readback.json`

Evidence captured on 2026-06-14:

- Implementation commits: `5fde017` and `677bc1c`.
- aiwonder gates passed: `cargo fmt --all -- --check`, tracked `.rs` line-count gate, `cargo check -p calyx-aster`, focused `mmap_col`, `slot_column`, `sst`, `olap` tests, and `cargo clippy -p calyx-aster --all-targets -- -D warnings`.
- Cold file logical size: `1073741824` bytes.
- Cold file sparse disk use: `41472` bytes.
- Read length: `1048576` bytes.
- First MiB hash: `1c59b8670027384143781a8a8bff2f3b44bd8818d0f53b13b064c2375a1afe38`.
- Readback JSON hash: `5330c3c893297406759053b9ace51c0024740172f8decc166cf197f979043f32`.
- RSS before/open/read: `4841472` / `4845568` / `5902336` bytes.
- RSS delta after 1 MiB read: `1060864` bytes, under the `2097152` byte limit.
- f32 readback: `[1.0, 2.0, 3.0, 4.0]`.
- Edge readbacks: bounds and alignment returned `CALYX_BOUNDS_EXCEEDED`; missing and empty returned `CALYX_NOT_FOUND`; `prefetch` and `drop_pages` were called nonfatally.

Separate aiwonder SoT reads:

- `stat` showed `cold-column-1g.bin` as a regular 1 GiB file and `f32-column.bin` as 16 bytes.
- `head -c 1048576 cold-column-1g.bin | sha256sum` matched the JSON `slice_sha256` and `expected_sha256`.
- `xxd -g1 -l 128 cold-column-1g.bin` showed the deterministic byte pattern beginning `07 26 45 64 83 a2 c1 e0`.
- `xxd -g4 -l 16 f32-column.bin` showed `0000803f 00000040 00004040 00008040`.

## Done When

- [x] `cargo check`, `clippy -D warnings`, and focused tests are green on aiwonder.
- [x] Rust files are `<=500` lines.
- [x] FSV evidence is attached to GitHub issue #472 / PR evidence.
- [x] No PH56 anti-pattern: no full cold column read into heap; no trust or intelligence-theory changes.
