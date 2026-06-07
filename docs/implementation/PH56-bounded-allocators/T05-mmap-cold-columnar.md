# PH56 · T05 — mmap cold/columnar access — OS page cache, never full vault in heap

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mmap_col.rs` (≤500) |
| **Depends on** | T04 (bounded memtable established) · PH11 (SSTable/compaction layout exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §5`, `04 §3` |

## Goal

Provide `MmapColumn` — a memory-mapped accessor for cold and columnar Aster data (SST slot
columns, ANN graph files, panel codebooks) where the OS page cache (ZFS ARC) is the cache and
Calyx never holds the full vault in heap. Opened column files are mmap'd read-only; access is a
pointer dereference; eviction is managed by the kernel. This eliminates a class of heap-OOM
failures on large vaults (hazard 8) and enables streaming for VRAM (PH57 uses pinned-host
double-buffering over these mmap'd columns).

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct MmapColumn { mmap: memmap2::Mmap, path: PathBuf, file_len: usize }` in `mmap_col.rs`; use `memmap2` crate (no-std-compat, widely used in RocksDB/LanceDB patterns)
- [ ] Implement `MmapColumn::open(path: &Path) -> Result<Self, CalyxError>` — opens file read-only, calls `memmap2::MmapOptions::new().map(&file)`; if file empty or nonexistent returns `CALYX_NOT_FOUND`; if mmap fails returns `CALYX_IO_ERROR` with OS error string
- [ ] Implement `MmapColumn::read_slice(&self, offset: usize, len: usize) -> Result<&[u8], CalyxError>` — bounds-checks `offset + len <= file_len`; returns slice; `CALYX_BOUNDS_EXCEEDED` on violation
- [ ] Implement `MmapColumn::read_f32_slice(&self, offset: usize, count: usize) -> Result<&[f32], CalyxError>` — alignment-checked cast (offset must be 4-byte aligned); wraps `read_slice`
- [ ] Implement `MmapColumn::prefetch(&self, offset: usize, len: usize)` — calls `libc::madvise(MADV_WILLNEED)` on the range; non-fatal if madvise fails (best-effort)
- [ ] Implement `MmapColumn::drop_pages(&self, offset: usize, len: usize)` — calls `libc::madvise(MADV_DONTNEED)` to release pages under pressure; non-fatal
- [ ] Add `primarycache=metadata` advisory note in a doc-comment (operator must set this on the SST ZFS dataset to avoid double-caching in ARC + mmap)
- [ ] Wire `MmapColumn::open` into the SST reader in `calyx-aster` for cold slot columns

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: write 1024 known bytes to a temp file; `MmapColumn::open` + `read_slice(0, 1024)` returns the exact bytes; verified with `assert_eq!(slice, &expected[..])`
- [ ] unit: `read_f32_slice` on a file containing 4 known f32 values → slice matches `[1.0_f32, 2.0, 3.0, 4.0]` byte-exactly
- [ ] unit: `read_slice(offset=1020, len=8)` on a 1024-byte file → `CALYX_BOUNDS_EXCEEDED` (1028 > 1024)
- [ ] unit: `read_f32_slice` at odd offset (e.g., offset=3) → `CALYX_BOUNDS_EXCEEDED` (alignment violation)
- [ ] unit: open a nonexistent path → `CALYX_NOT_FOUND`
- [ ] edge: zero-length file → `open` returns `CALYX_NOT_FOUND` (empty mmap is meaningless)
- [ ] edge: `prefetch` and `drop_pages` on a valid range → no panic (madvise may return ENOSYS on some kernels; must not fail-hard)
- [ ] fail-closed: truncate a file after open (file shrinks); `read_slice` at a now-invalid offset → OS SIGBUS would occur — document in SAFETY comment; in practice we re-check `file_len` at open time; operator must not truncate live files

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** heap RSS reported by `/proc/self/status VmRSS` during a large vault read, compared to vault size on disk
- **Readback:** `calyx readback --metric rss_bytes` while reading a 10 GB vault — `rss_bytes` must remain << vault size (page cache handles it, not heap); `zfs list -o name,used` shows vault on disk; `free -h` shows ARC growing (not heap)
- **Prove:** open a 1 GB cold column file, read 1 MB of it; `rss_bytes` delta is < 2 MB (only the 1 MB page plus overhead, not 1 GB). Compare before/after with `calyx readback --metric rss_bytes`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
