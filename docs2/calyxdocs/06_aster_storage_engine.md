# 06 — Aster Storage Engine (calyx-aster)

`calyx-aster` is the storage substrate of Calyx: an ordered, transactional, MVCC
keyspace built as an LSM tree (memtable + immutable SSTables + WAL) with column
families, hot/cold tiering, atomic manifest/versioning, crash recovery, and a set
of FoundationDB-style key-encoding "layers" (relational, document, KV,
time-series, blob, graph) plus the native constellation/slot/anchor families.
Largest crate in the workspace (232 `.rs` files).

This doc documents WHAT THE CODE DOES. Where the planning doc
`docs/dbprdplans/04_ASTER_STORAGE_FORMAT.md` describes intent that is not (yet)
matched by code (DiskANN, SPANN, PQ/Float8 quantization, zstd compression), it is
flagged in [§11 Gaps](#11-gaps--not-covered). On-disk magic/version constants here
also feed [04_storage_and_schema.md](04_storage_and_schema.md).

**Source files covered:**
- `crates/calyx-aster/src/lib.rs`
- `crates/calyx-aster/src/wal/mod.rs`, `wal/record.rs`, `wal/batch.rs`, `wal/segment.rs`
- `crates/calyx-aster/src/memtable/bounded.rs`, `memtable/mod.rs`
- `crates/calyx-aster/src/sst/mod.rs`, `sst/bloom.rs`, `sst/level.rs`, `sst/arrow.rs`
- `crates/calyx-aster/src/mmap_col.rs`
- `crates/calyx-aster/src/mvcc/store.rs`, `mvcc/mod.rs`, `mvcc/lease.rs`, `mvcc/read_barrier.rs`
- `crates/calyx-aster/src/manifest/mod.rs`, `manifest/quarantine.rs`
- `crates/calyx-aster/src/compaction/mod.rs`, `compaction/tiering.rs`, `compaction/scan.rs`
- `crates/calyx-aster/src/cf/family.rs`, `cf/key.rs`, `cf/router.rs`, `cf/mod.rs`
- `crates/calyx-aster/src/storage_names.rs`
- `crates/calyx-aster/src/vault.rs`, `vault/encode.rs`, `vault/durable.rs`
- `crates/calyx-aster/src/gc/*`, `timetravel/*`, `txn/*`, `dedup/*`, `index/*`, `layers/*`, `collection/*`, `resource/*`, `pressure.rs`, `residency.rs`, `retention.rs`, `security/*`, `redaction.rs`, `erase.rs`

---

## 1. Storage architecture

### 1.1 Module tree (top-level `pub mod` from `src/lib.rs`)

`cf`, `collection`, `compaction`, `dedup`, `erase`, `gc`, `index`, `layers`,
`ledger_view`, `manifest`, `memtable`, `mmap_col`, `mvcc`, `olap`, `plain_column`,
`plain_graph`, `pressure`, `recurrence`, `redaction`, `residency`, `resource`,
`retention`, `security`, `sst`, `storage_names`, `stream`, `stride_fsv`,
`supply_chain`, `timetravel`, `txn`, `vault`, `wal`. Private: `file_lock`.
`lib.rs` re-exports only the dedup compression helpers
(`CompressionRatio`, `Domain`, `DomainCompressionStats`, `compression_ratio`,
`domain_compression_stats`). The `lib.rs` doc-comment calls the crate a
"skeleton".

### 1.2 LSM design

Aster is a leveled LSM keyspace. Per column family the write/read components are:

| Component | Type | File |
|---|---|---|
| Mutable in-memory table | `BoundedMemtable` (alias `Memtable`), `BTreeMap<Vec<u8>,Vec<u8>>` with byte cap | `memtable/bounded.rs` |
| Frozen handoff | `FrozenMemtable` (immutable `BTreeMap`) | `memtable/bounded.rs` |
| On-disk sorted table | SSTable (`write_sst` / `SstReader`), mmap-backed | `sst/mod.rs` |
| Per-CF newest-first stack | `SstLevel` (single level) | `sst/level.rs` |
| Router | `CfRouter` — owns one memtable + one `SstLevel` per CF | `cf/router.rs` |
| Compaction view | `CompactionCatalog` / `CompactionSnapshot` with atomic swap | `compaction/mod.rs` |

Note: `SstLevel` models a single per-CF level (`files: Vec<PathBuf>`, newest at
index 0 via `push` inserting at front). There is no multi-level (L0/L1/…)
fan-out implemented inside `SstLevel`; the `level: u8` on `SstShard` only
increments on compaction for naming/debt purposes.

### 1.3 Write path (`AsterVault` / `CfRouter` / `DurableVault`)

`AsterVault<C>` (`vault.rs`) is the single-vault store. A durable write group
(`write_cf_batch` / `commit_rows` → `stage_constellation_rows`):

1. Build `Vec<WriteRow{cf,key,value}>`.
2. `ensure_memtable_admission` / `CfRouter::ensure_batch_admitted` — fail closed
   with `CALYX_BACKPRESSURE` if any single row exceeds the memtable byte cap
   (a row that can never fit), BEFORE the WAL append (`cf/router.rs`).
3. `DurableVault::append_batch` (`vault/durable.rs`): `encode_write_batch(rows)`
   → `GroupCommitBatcher::submit(payload)` → WAL append + fsync; returns the
   allocated seq. (Test-only failpoint `fail_next_wal_append`.)
4. `VersionedCfStore::commit_batch` applies rows to the CF router memtables and
   pushes versioned values at the allocated MVCC seq (`mvcc/store.rs`).
5. Memtable high-water → `CfRouter::flush_cf` writes an SSTable and pushes it on
   the CF's `SstLevel`. Backpressure on a full memtable triggers an inline flush
   and retry (`record_memtable_absorbed` / `record_memtable_rejected`).

`CfRouter::put` flow: write to memtable; on `CALYX_BACKPRESSURE` flush the CF and
retry once; if `ack.flush_triggered` (high-water), flush after the write.

### 1.4 Read path

`AsterVault::read_cf_at(seq, cf, key)` → `VersionedCfStore::read_at` resolves the
visible version for the pinned seq (MVCC, §4). For the non-MVCC raw LSM path,
`CfRouter::get` checks the memtable first, then `SstLevel::get` which opens each
SST newest-first and returns the first hit (Bloom-gated). `range`/`iter_cf` merge
memtable + level into a `BTreeMap` (newest wins). `CompactionSnapshot::get`
iterates shards in reverse, newest first.

### 1.5 Column families

`ColumnFamily` (`cf/family.rs`) is the CF enum. Static (non-slot) families are in
`ColumnFamily::STATIC` (an array of 33; recovery/manifest order). Slot CFs are
parameterized: `Slot { slot: SlotId, kind: SlotFamilyKind }` where
`SlotFamilyKind ∈ {Quantized, Raw}`. Directory names from `ColumnFamily::name()`:

| CF | dir name | key (logical) |
|---|---|---|
| `Base` | `base` | `CxId` → ConstellationHeader |
| `Collections` | `collections` | `b"coll\0" ‖ name` → collection metadata |
| `Relational` | `relational` | `0x01 ‖ collection_id ‖ pk` |
| `Document` | `document` | `0x02 ‖ collection_id ‖ doc_id ‖ path` |
| `Kv` | `kv` | `0x03 ‖ collection_id ‖ ns ‖ key` |
| `TimeSeries` | `timeseries` | `0x04 ‖ … ‖ ts/window` |
| `Blob` | `blob` | `0x05 ‖ … ‖ blob_id ‖ chunk` |
| `Slot{Quantized}` | `slot_NN` | `CxId` → quantized SlotVector |
| `Slot{Raw}` | `slot_NN.raw` | `CxId` → raw f32 sidecar |
| `XTerm` | `xterm` | `(CxId, SlotId_a, SlotId_b, XTermKind)` |
| `TemporalXTerm` | `temporal_xterm` | `(CxId_a, CxId_b)` |
| `Scalars` | `scalars` | `(ScalarId, CxId)` → f64 |
| `Anchors` | `anchors` | `(CxId, AnchorKind)` → AnchorValue |
| `Assay` | `assay` | `(panel_version, corpus_shard, subject)` |
| `Ledger` | `ledger` | `seq` → hash-chained entry |
| `Kernel` | `kernel` | Lodestar kernel reports/indexes |
| `Guard` | `guard` | Ward calibration profiles |
| `Recurrence` | `recurrence` | `(CxId, OccurrenceId)` |
| `Graph` | `graph` | nodes/edges/reverse/CSR |
| `Online` | `online` | typed online state |
| `Reactive` | `reactive` | reactive trigger audit |
| `AnnealRollback…AnnealOperators` (14 CFs) | `anneal_*` | Anneal state |
| `TimeIndex` | `time_index` | `be(millis) ‖ be(seqno)` → sentinel |
| `IndexBtree` | `index_btree` | `0x10 ‖ collection_id ‖ index_id ‖ field_val ‖ pk` |
| `IndexInverted` | `index_inverted` | `0x11 ‖ collection_id ‖ index_id ‖ term_hash ‖ pk` → f32 |

`ColumnFamily::keyspace_tag()` / `parse_keyspace_tag()` give a reversible CF tag
for vault-scoped composite keys: non-slot CFs encode to their single-byte index
in `STATIC`; slot CFs encode to `SLOT_KEYSPACE_TAG (0xF0) ‖ slot_id_be(2) ‖ kind_byte`
(`Quantized=0`, `Raw=1`). `STATIC.len()` stays below `0xF0` so there is no
collision. `is_slot`, `is_raw_slot`, `slot_id`, `slot(SlotId)`, `slot_raw(SlotId)`
are the accessors/constructors.

Key codecs (`cf/key.rs`) are all big-endian for range-scan ordering. `CxId` is a
16-byte BLAKE3 prefix (`CX_ID_BYTES = 16`, `FULL_HASH_BYTES = 32`). Functions:
`base_key`, `slot_key`, `xterm_key`, `temporal_xterm_key`, `scalar_key`,
`anchor_key`, `ledger_key`, `recurrence_key`, `online_key`, the matching
`*_prefix_range`/`*_range` builders, `prefix_range`, `full_content_hash` (BLAKE3
over length-delimited parts: each part prefixed with `be_u64(len)`),
`cx_id_from_full_hash` (first 16 bytes), `verify_cx_hash_prefix`.
`XTermKind ∈ {Concat=0, Interaction=1, Agreement=2, Delta=3}`;
`OnlineKeyKind ∈ {MistakeLog=0, ReplayBuffer=1, HeadState=2, DeltaJQueue=3}`;
`ScalarId(u32)`; `KeyRange{start, end:Option}` (`contains`: `start ≤ key < end`).
`anchor_key` encodes `AnchorKind` as a `be_u16` tag (`TestPass=0 … Recurrence=6`,
`Label=7` followed by `be_u64(len) ‖ utf8`).

---

## 2. Write-ahead log (`wal/`)

### 2.1 Record format — "CXW1" (`wal/record.rs`)

`MAGIC = "CXW1"` as little-endian `u32` (`u32::from_le_bytes(*b"CXW1")`).
`HEADER_LEN = 20`, `MAX_RECORD_BYTES = 64 MiB`.

```
WAL record (all little-endian):
  +0   u32   magic   = "CXW1"
  +4   u64   seq             (1-based vault WAL sequence)
  +12  u32   len             (payload length, ≤ 64 MiB)
  +16  u32   crc             (crc32fast over seq_le ‖ len_le ‖ payload)
  +20  [u8; len]  payload    (encode_write_batch output)
```

CRC: `crc32fast::Hasher` over `seq.to_le_bytes()`, `len.to_le_bytes()`, then the
payload. `decode_at` returns `DecodeStatus::{Complete, Eof, Torn{offset,message}}`.
A record is **Torn** (and the tail truncated) on: partial header (`<20` bytes,
non-zero), bad magic, `len > MAX_RECORD_BYTES`, partial payload (UnexpectedEof),
or CRC mismatch. `Eof` is a clean 0-byte read at a record boundary.

The WAL payload is a `WriteRow` batch (`vault/encode.rs` `encode_write_batch`):
`be_u32(row_count)` then per row `cf_tag(u8) ‖ be_u32(key_len) ‖ key ‖ be_u32(value_len) ‖ value`.

### 2.2 Segments (`wal/segment.rs`)

Segment files are named `{index:020}.wal` (20-digit zero-padded). `list_segments`
sorts by index and **fails closed** (`CALYX_ASTER_CORRUPT_SHARD`) if indexes are
non-contiguous (a missing segment would silently drop committed writes) or if a
`*.wal` file has a non-canonical name (`storage_names::wal_segment_index`).

### 2.3 Writer & group-commit (`wal/mod.rs`, `wal/batch.rs`)

`Wal` holds the active segment file in append mode. `WalOptions`:
`max_segment_bytes` (default 64 MiB), `group_commit_window` (default
`DEFAULT_GROUP_COMMIT_WINDOW = 2 ms`). `Wal::open` acquires a `.append.lock`
file lock, replays/truncates a torn tail, and sets `next_seq` to last seq + 1
(or 1). `append_batch` re-checks for external appends, encodes each record, rotates
the segment if `offset + bytes > max_segment_bytes` (fsync-before-rotate via
`sync_all`), writes all records, then a single `sync_data()` (fsync) before
returning `AppendAck{seq, segment_path, start_offset, end_offset}`. One fsync per
batch is the group-commit unit.

`GroupCommitBatcher` (`wal/batch.rs`) is the fsync-backed coalescing wrapper: a
dedicated thread receives `BatchOp::{Append,Flush,TipSeq}` over an mpsc channel.
On the first `Append` it opens a deadline of `group_commit_window` and drains
further requests until the deadline/timeout/disconnect, then calls
`Wal::append_batch` once for all coalesced payloads (one fsync). `validate_window`
**rejects any window > 2 ms** with `CALYX_DISK_PRESSURE` ("group_commit_window
exceeds 2 ms limit"). Public API: `submit(payload)->AppendAck`, `flush_sync()`,
`tip_seq()->u64`.

### 2.4 fsync policy

Every batch append issues `sync_data()`; segment rotation issues `sync_all()`;
newly created segments fsync the parent directory on Unix (`sync_parent`, no-op on
non-Unix). The torn-tail truncation path also `set_len` + `sync_data`. WAL is the
source of truth for un-checkpointed writes.

### 2.5 Replay & recycling

`replay_dir` / `replay_dir_locked` decode every segment in order; on the first
torn record it truncates that segment to the torn offset, removes all later
segments, and returns `ReplayOutcome{records, torn_tail: Some(TornTail)}`.
`TornTail{segment_path, offset, code, message}` carries `code =
CalyxErrorCode::AsterTornWal.code()`; `TornTail::error()` builds the catalogued
`AsterTornWal` error. `WalSegmentStatus`, `segment_inventory`,
`total_segment_bytes`, `durable_tip_seq`, and `recycle_durable_segments`
(`WalRecycleReport`) support the GC WAL recycler: a non-active segment whose
`last_seq ≤ newest_durable_seq` is recyclable; recycling truncates it to 0 and
fsyncs, bounded by `min(max_segments, fsync_budget)` per pass.

---

## 3. Memtable, SSTable, columnar & mmap formats

### 3.1 Memtable (`memtable/bounded.rs`)

`BoundedMemtable` (aliased `Memtable`): `Mutex<BTreeMap<Vec<u8>,Vec<u8>>>` with a
hard `cap_bytes` and a `high_water_bytes` (default 80% of cap,
`high_water_for = cap*4/5`). `ENTRY_OVERHEAD_BYTES = 4`; `entry_size = key.len +
value.len + 4`. `write(key,value,seq)->WriteAck` fails closed with
`CALYX_BACKPRESSURE` if the row alone exceeds cap, or if projected used bytes
exceed cap (replacing an existing key reclaims its bytes first). `WriteAck{seq,
accepted_bytes, used_bytes, cap_bytes, high_water_bytes, flush_triggered}`;
`flush_triggered` is true at/above high-water (and always true when cap==0).
`MemtableUsage` mirrors the byte accounting. `freeze()` → `FrozenMemtable`;
`flush_to_sst(path)` writes a sorted SSTable; `reset_after_flush(bytes)`
decrements the atomic usage counter. Iteration is key-ordered (BTreeMap).

### 3.2 SSTable format — "CXS1" v2 (`sst/mod.rs`)

`MAGIC = b"CXS1"`, current `VERSION = 2`, `LEGACY_VERSION = 1` (still readable),
`HEADER_LEN = 32`, `RECORD_HEADER_LEN = 12`, `INDEX_ENTRY_FIXED_LEN = 12`.
All multi-byte fields little-endian. Layout:

```
SSTable (CXS1):
  HEADER (32 B):
    +0  [u8;4] magic = "CXS1"
    +4  u32    version = 2
    +8  u32    entries
    +12 u64    index_offset
    +20 u64    bloom_offset
    +28 u32    body_crc   (crc32 over bytes[32..]; v2 only)
  RECORDS  (offset 32 .. index_offset), each:
    +0  u32    key_len
    +4  u32    value_len
    +8  u32    record_crc  (crc32 over key ‖ value)
    +12 key bytes ‖ value bytes
  INDEX  (index_offset .. bloom_offset), one per entry, key-sorted:
    +0  u32    key_len
    +4  u64    record_offset
    +12 key bytes
  BLOOM  (bloom_offset .. EOF): encoded BloomFilter
```

`write_sst(path, entries)` requires strictly sorted, unique keys
(`ensure_sorted`, else `CALYX_ASTER_CORRUPT_SHARD`); writes to `path.sst.tmp`,
`sync_all`, then atomic `rename` + parent fsync. `SstReader::open` mmaps the file
(`MmapColumn`), validates header magic/version, bounds-checks offsets, verifies
the v2 body CRC, and decodes the index and Bloom filter — every failure path is a
fail-closed `CALYX_ASTER_CORRUPT_SHARD`. `get(key)` consults the Bloom filter,
then binary-searches the index and reads + CRC-verifies the record. `range`,
`iter`, `bloom_may_contain` round out the API. `SstSummary{path, entries, bytes,
index_offset, bloom_offset}`; `SstEntry{key, value}`.

### 3.3 Bloom filter (`sst/bloom.rs`)

Deterministic per-SST filter: `bit_count = max(64,
next_power_of_two(max(1,n)*16))`, `hash_count = 3`. Hashing is BLAKE3 over
`key ‖ round_le_u32`, first 8 bytes mod `bit_count`. Encoded as
`le_u64(bit_count) ‖ le_u32(hash_count) ‖ le_u32(byte_len) ‖ bits`. Tested
false-positive rate < 1% at 10k keys; no false negatives by construction.

### 3.4 SstLevel (`sst/level.rs`)

Per-CF newest-first `Vec<PathBuf>` (`push` inserts at front). `get` returns the
first (newest) SST hit. `range`/`iter` merge across files into a `BTreeMap` where
the **oldest** value is kept (`or_insert`) because files are iterated newest-first
and `or_insert` does not overwrite — so the first-seen (newest) value wins.

### 3.5 Arrow column chunk — "CXA1" v1 (`sst/arrow.rs`)

Column-major f32 chunk for SIMD-friendly slot scans:

```
Arrow chunk (CXA1, little-endian):
  +0  [u8;4] magic = "CXA1"
  +4  u32    version = 1
  +8  u32    n_rows
  +12 u32    dim
  +16 ...    f32 values, COLUMN-MAJOR: for col in 0..dim { for row in 0..n_rows { f32_le } }
```

`HEADER_LEN = 16`. `encode_column_chunk(rows)` (all rows same non-zero dim);
`decode_column_shape` → `ArrowColumnView` (zero-copy column slices,
`column_bytes`/`column_values`/`value(col,row)`); `decode_column_chunk` →
`ArrowChunkView` (row-major materialized, `row(i)`). Length and shape are
validated exactly (`bytes.len() == HEADER_LEN + n_rows*dim*4`).

### 3.6 mmap (`mmap_col.rs`)

`MmapColumn` is a read-only `memmap2::Mmap` over an immutable file (backs every
`SstReader`). `open` rejects missing/empty files (`CALYX_NOT_FOUND`). Accessors:
`read_slice(offset,len)` (bounds-checked), `read_f32_slice(offset,count)`
(checks f32 alignment of both offset and base pointer, returns `&[f32]` via
`from_raw_parts`), `as_bytes`, `file_len`, `path`, plus `prefetch`/`drop_pages`
(`madvise WILL_NEED`/`DONT_NEED`, Unix-only, best-effort). Error codes:
`CALYX_NOT_FOUND`, `CALYX_IO_ERROR`, `CALYX_BOUNDS_EXCEEDED`. Doc note: file must
not be truncated while mapped (SIGBUS risk); ZFS columns should use
`primarycache=metadata`.

### 3.7 Constellation base value encoding (`vault/encode.rs`)

The `base` CF value is the constellation header + body. `HEADER_LEN = 102`,
`IDENTITY_HASH_LEN = 32`. `ConstellationHeader` fields:
`cx_id(16) ‖ vault_id ULID(16) ‖ be_u32 panel_version ‖ be_u64 created_at ‖
u8 modality_tag ‖ u8 flags_bits ‖ be_u16 n_slots ‖ be_u16 n_anchors ‖
be_u64 ledger_seq ‖ input_hash(32) ‖ 12 zero bytes` (= 102 bytes).
`encode_constellation_base` appends: BLAKE3 identity hash (32 B), input-ref tail
(`u8 redacted ‖ pointer tag/string`), slot list (`be_u16 count`, then per slot
`be_u16 slot_id ‖ blake3(encode_slot_vector)`), scalars (`be_u32 count`, then
`len-prefixed key ‖ be_u64 f64.to_bits`), anchors (`be_u32 count`, len-prefixed
`encode_anchor`), provenance hash (32 B), then string metadata. Modality tags
`Text=0 … Molecule=9`; flags bitfield `ungrounded|degraded<<1|novel_region<<2|redacted_input<<3`.

`encode_slot_vector` tag byte: `Dense=0 (be_u32 dim ‖ be_u32 f32.to_bits…)`,
`Absent=1 (reason tag 0..5)`, `Sparse=2 (be_u32 dim ‖ be_u32 n ‖ {be_u32 idx ‖
be_u32 val}…)`, `Multi=3 (be_u32 token_dim ‖ be_u32 n_tokens ‖ tokens…)`. Floats
are stored as big-endian IEEE-754 bit patterns (`f32::to_bits`/`f64::to_bits`),
not raw f32 LE — i.e. the slot CF value is NOT the Arrow CXA1 layout.

---

## 4. MVCC snapshots (`mvcc/`)

### 4.1 Sequence allocator (`mvcc/lease.rs`)

`SeqAllocator{current:AtomicU64, allocated:AtomicBool}`: `new(start)` makes the
next write `start+1`. `allocate()` does `fetch_add(1)+1` and marks allocated;
`current()` reads the latest committed seq; `set_start_seq(seq)` is permitted only
before the first allocation (else `CALYX_BACKPRESSURE` "cannot reset MVCC start
seq after allocation") — used during recovery; `advance_to_at_least(seq)` CAS-bumps
the seq after reading externally committed durable rows. The vault sequence is a
single vault-wide monotonic counter; a write advances it and all CFs share it.

### 4.2 Versioned store & visibility (`mvcc/store.rs`)

`VersionedCfStore` keys a `BTreeMap<(ColumnFamily, Vec<u8>), Vec<VersionedValue{seq,value}>>`
(append-only per-key version chain) plus a `SeqAllocator`, an optional `CfRouter`,
read barriers, a `LeaseRegistry`, `ResourceCounters`, and a `SnapshotGcReclaimer`.
`commit_batch(rows)` writes rows to the router and pushes a `VersionedValue` per
row at one freshly-allocated seq (atomic across any number of CFs).
`restore_batch(seq, rows)` re-inserts durable rows at their original seq during
recovery before live writes begin.

Visibility (`visible_version`): scan the version chain **in reverse** and return
the first version with `seq ≤ snapshot.seq`, unless it is a tombstone.
Tombstones: `TOMBSTONE_VALUE = b"\0CALYX_ASTER_TOMBSTONE_V1"`,
`tombstone_value()` / `is_tombstone_value()`. A visible tombstone reads as
absent. This is snapshot isolation: a reader pinned at seq S sees exactly the
latest committed version ≤ S of every key across every CF.

Read API (all take a `Snapshot` + `&dyn Clock`, enforce lease liveness, and check
read barriers): `read_at`, `seq_for_key_at`, `read_batch(&[CfRead])`, `scan_cf_at`,
`scan_cf_range_at`, `scan_cf_range_page_at(after_key, limit)`. `CfRead{cf,key}`.

### 4.3 Reader leases & freshness (`mvcc/lease.rs`, `mvcc/store.rs`)

`ReaderLease{id, pinned_seq, issued_at, max_age_ms}` with
`expires_at = issued_at + max_age_ms`, `is_expired_at`, `ensure_live_at`
(`CALYX_READER_LEASE_EXPIRED` on expiry). `Snapshot{seq, freshness, lease}`.
`pin_snapshot(freshness, clock, max_age_ms)` pins at `current_seq` and registers
the lease for oldest-pinned-seq accounting; `pin_snapshot_at(seq, …)` pins at an
explicit historical seq (time-travel) so version GC cannot reclaim ≤ seq while
held; `release_lease(id)`; `lease_view(now)`. `AsterVault` exposes `pin_reader`,
`pin_reader_at`, `release_reader`. Internal vault reads use a transient lease id 0
with `DEFAULT_LEASE_MS = 5_000` and `Freshness::FreshDerived`.

`Freshness` governs DERIVED structures (ANN/xterm/kernel/guard) carrying a build
seq: `FreshDerived` requires `derived_seq ≥ pinned_seq`; `StaleOk{max_lag}` allows
lag up to `max_lag`. `ensure(pinned, derived)` returns `CALYX_STALE_DERIVED` on
violation.

`snapshot_gc_tick(clock, max_gap_seqs)` aborts expired readers, checks the
pinned-seq gap, and returns `SnapshotGcTick{aborted_readers, gap_alert, metrics}`.

### 4.4 Read barriers (`mvcc/read_barrier.rs`)

`ReadBarrier` blocks reads over a `(ColumnFamily, KeyRange)`. `base_corrupt(id,
range)` constructs one for `ColumnFamily::Base` with code
`CALYX_ASTER_BASE_CORRUPT` and remediation "restore from restic/snapshot"
(A16: a corrupt base shard fails reads closed rather than returning wrong data).
`first_blocking(barriers, cf, key)` returns the blocking error if any.
`AsterVault::install_read_barrier`/`remove_read_barrier`/`read_barriers`.

---

## 5. Crash recovery, manifest & versioning (`manifest/`, `vault/durable.rs`)

### 5.1 Manifest format & format version (`manifest/mod.rs`)

The manifest is **JSON** (`serde_json::to_vec_pretty`), not a binary format.
Files in the vault root: `CURRENT` (text pointer), `MANIFEST` (mirror copy of the
current manifest bytes), and immutable `manifest-{seq:020}.json` files
(`MANIFEST_PREFIX="manifest-"`, `MANIFEST_SUFFIX=".json"`).

Format version constant: `ManifestVersion{major:u16, minor:u16}` with
`SUPPORTED_MANIFEST_MAJOR = 1`, `SUPPORTED_MANIFEST_MINOR = 0`
(`ManifestVersion::current()` = `1.0`). `validate()` refuses any **unknown major**
with error code `CALYX_FORMAT_VERSION_UNSUPPORTED` (remediation: "refuse unknown
format major; migrate through a compatible reader"). This is the Aster
format-version gate referenced by [04_storage_and_schema.md](04_storage_and_schema.md).

`VaultManifest` fields: `version`, `manifest_seq:u64` (must be ≥ 1),
`durable_seq:u64` (highest seq durably checkpointed to SSTs), `panel_ref:ImmutableRef`,
`registry_ref:Option<ImmutableRef>`, `codebook_refs:Vec<ImmutableRef>`,
`temporal_policy:Option`, `dedup_policy:Option`, `retention_horizon`,
`degraded_rebuildable:bool`, `quarantines:Vec<QuarantineRecord>`. `ImmutableRef{
logical_path, blake3_hex}` is content-addressed: path must be vault-relative,
not escape the vault (no `..`/root/prefix), not point at a control file, and the
hash must be 64 lowercase hex chars (32-byte BLAKE3). `panel_ref` must be under
`panel/`, `registry_ref` under `registry/`, codebooks under `codebooks/`
(deduplicated).

### 5.2 Atomic manifest swap (`ManifestStore`)

`write_current(manifest)`: validate, then `write_atomic` (write `*.tmp`,
`sync_all`, `rename`, parent fsync) for the `manifest-NNNN.json`, the `MANIFEST`
mirror, and finally `CURRENT` → the pointer filename. `load_current` reads
`CURRENT`, validates the pointer filename shape, reads the pointed manifest,
decodes + validates, and **verifies every immutable ref's BLAKE3 hash against the
on-disk bytes** (`verify_immutable_refs`) — a mismatch is
`CALYX_ASTER_CORRUPT_SHARD`. `append_quarantine` bumps `manifest_seq` and rewrites.
`ManifestWrite{manifest_path, mirror_path, current_path, pointer}`.

### 5.3 Recovery algorithm (`recover_vault` + `DurableVault::recover_batches`)

`recover_vault(vault_dir)`:
1. `ManifestStore::load_current()` — load + validate manifest, verify immutable
   refs (panel/registry/codebooks) by hash. If `CURRENT` is absent, recovery
   falls through to a WAL-only path (`DurableVault::recover_batches`).
2. `replay_dir(vault_dir/"wal")` — replay all WAL segments, truncating any torn
   tail (§2.5).
3. Keep only WAL records with `seq > manifest.durable_seq` (records already
   checkpointed into SSTs are not replayed).
4. `last_recovered_seq` = last surviving WAL record seq, else `manifest.durable_seq`.
5. Return `RecoveryOutcome{manifest, wal_records, torn_tail, last_recovered_seq,
   degraded_rebuildable}`.

`DurableVault::recover_batches` then materializes batches:
- With a manifest: `read_manifested_batches` reads SSTs across all tier roots,
  classifying names via `storage_names::classify_sst`. `DurableBatch{seq,index}`
  and `Compacted{seq}` files with `seq ≤ durable_seq` are loaded (router-flush
  `Router{seq}` SSTs are recovered separately by `CfRouter::load_existing`),
  ordered by `(seq, index+row_offset)`. WAL records (seq > durable_seq) are decoded
  via `decode_write_batch` and appended.
- Without a manifest: replay the WAL directly into batches.

On open (`AsterVault::open_with_clock`): recover batches, recover the ledger hook,
open the `CfRouter` (loads existing SSTs), build `VersionedCfStore` at
`last_recovered_seq`, `restore_batch` every recovered batch at its original seq,
`set_start_seq(last_recovered_seq)`, then open `DurableVault`. `VaultRecoveryReport{
last_recovered_seq, torn_tail}` is exposed via `recovery_report()` — the torn-tail
diagnostic surfaces on normal open.

### 5.4 Canonical file names (`storage_names.rs`)

Single fail-closed authority for `*.sst`/`*.wal` names in Aster-owned dirs. SST
name classes (`SstName`): `Router{seq}` = `{seq:020}.sst` (router memtable flush);
`DurableBatch{seq,index}` = `{seq:020}-{index:04}.sst` (group-commit batch);
`Compacted{seq}` = `compacted-{seq:020}.sst`. `classify_sst` returns `Ok(None)`
for non-`.sst` files, `Ok(Some)` for canonical names, and a typed
`CALYX_ASTER_CORRUPT_SHARD` error for any `*.sst` that matches no canonical shape
(refusing to silently skip it). `sst_order_key`/`SstOrderKey{seq, class_rank,
index}` give chronological order (`Router=1 < DurableBatch=2 < Compacted=3` within
a seq). `wal_segment_index` parses `{index:020}.wal`. `parse_cf_dir_name` maps a
`cf/<name>` dir back to a `ColumnFamily`, round-tripping through `name()` so a
mis-padded slot dir is rejected.

### 5.5 Quarantine (`manifest/quarantine.rs`)

`QuarantineRecord` ranges of seqs poisoned by corruption; `is_quarantined`,
`is_vault_seq_quarantined`, `manifest.quarantines`. `read_base_shard(path,key)`
reads a base SST through the fail-closed `SstReader`.

---

## 6. Compaction & hot/cold tiering (`compaction/`)

### 6.1 Catalog & snapshot-safe compaction (`compaction/mod.rs`)

`SstShard{cf, path, level:u8, bytes}`. `CompactionCatalog{active:RwLock<Arc<Vec<SstShard>>>}`
with `pin_snapshot()->CompactionSnapshot` (an `Arc` clone; old pinned views
survive a swap → concurrent reads during rebuild). `compact_cf(cf, output_path,
throttle)`:
1. Pin a snapshot, collect input shards for `cf`.
2. `compact_shards(cf, inputs, output_path, throttle)`.
3. On success build the output `SstShard` at `level = max(input levels)+1`, swap
   the active list (drop old `cf` shards, add the compacted one) atomically.

`compact_shards`:
1. `CompactionDebt::measure` over inputs.
2. **Skip** if `< 2` input files, or if `throttle.max_input_bytes` is set and
   pending bytes exceed it (`CompactionResult::Skipped{debt}`).
3. Merge all input SSTs into a `BTreeMap` (later files overwrite — newest wins),
   write a single output SST (`write_sst`).
4. Compute `write_amp_milli = output_bytes*1000 / logical_bytes` and return
   `CompactionResult::Compacted(CompactionReport{…})`.

`CompactionReport` fields: `cf, input_files, input_paths, input_bytes,
output_bytes, logical_bytes, write_amp_milli, reclaimed_input_files(=0),
debt_before, debt_after, output_path, staging_parent`.

### 6.2 Debt & scheduler

`CompactionDebt{pending_bytes, target_bytes, score_milli}` where
`score_milli = pending_bytes*WRITE_AMP_SCALE / target_bytes`,
`WRITE_AMP_SCALE = 1_000`, `DEFAULT_COMPACTION_TARGET_BYTES = 64 MiB`.
`CompactionThrottle{max_input_bytes:Option<u64>}` (`unlimited` / `max_input_bytes`).

`CompactionScheduler::start(catalog, options)` runs a background thread:
`CompactionSchedulerOptions{interval_ms=10_000, debt_trigger_score_milli=1_000,
max_write_amp_milli=2_000, backoff_factor=2, max_interval_ms=60_000, output_root,
tiering_policy}`. Each tick: sleep `interval_ms`; for each CF, if `debt.score_milli
≥ debt_trigger_score_milli` compact it; if the resulting `write_amp_milli >
max_write_amp_milli`, multiply the interval by `backoff_factor` (capped at
`max_interval_ms`) — anti-storm backoff. Output path is placed via the tiering
policy or `output_root/<cf>/compacted-{id:020}.sst`. The cadence is fixed; a
`FIXME(PH46)` notes the intended Anneal adaptive hook is not yet wired.

### 6.3 Hot/cold tiering (`compaction/tiering.rs`)

`StorageTier ∈ {Hot, Cold}`. `TieringPolicy{hot_root, archive_root, active_slots,
current_panel_version}`; `TieringPolicy::aiwonder(...)` defaults to
`/zfs/hot/calyx` and `/zfs/archive/calyx` (falling back to `$CALYX_HOME/<hot|archive>`
or `/home/croyse/calyx/...` when the ZFS path is absent). `place_cf(cf,
panel_version)->TierPlacement{tier, root, cf_dir=cf/<name>}`;
`absolute_dir() = root.join(cf_dir)`. `place_current_cf` uses
`current_panel_version`.

Cold-vs-hot rule (`is_cold`):
- Always **Hot**: `Base`, `Ledger`, `Anchors`, `Graph`, `Reactive`, and all
  `Anneal*` CFs.
- Always **Cold**: any raw slot CF (`is_raw_slot`).
- A quantized slot CF is **Cold** iff `panel_version < current_panel_version`
  (retired version) OR its slot is not in `active_slots`.
- Everything else: Hot.

`write_tiered_sst(cf, panel_version, file_name, entries)` refuses a non-canonical
SST file name (would be invisible to fail-closed recovery scans), creates the tier
dir, writes the SST, returns `TierWrite{placement, path, bytes, staging_parent}`.
`tier_roots()` returns one root when hot==archive, else both.
`scan.rs`: `catalog_from_vault_dir`, `catalog_from_vault_tiers` build a
`CompactionCatalog` by scanning canonical SSTs across tier roots.

---

## 7. Garbage collection (`gc/`)

Six GC subsystems (public types only; thresholds in parentheses). All are
incremental/bounded with anti-storm controls.

| GC | Reclaims | Key public types / defaults |
|---|---|---|
| `snapshot_gc` | abandoned/expired reader leases, oldest-pinned-seq gap | `SnapshotGcReclaimer`, `SnapshotGcTick`, `SnapshotGcCounters`; `DEFAULT_READER_LEASE_MS=5_000`, `DEFAULT_MAX_PINNED_SEQ_GAP=1_000_000` |
| `wal_recycler` | durable, non-active WAL segments (truncate to 0) | `WalRecycleReport`; `DEFAULT_MAX_RECYCLE_PER_TICK=8`, `DEFAULT_FSYNC_BUDGET_PER_TICK=8`, `DEFAULT_FSYNC_P99_ALERT_US=10_000` |
| `compaction_gc` | tombstone-heavy SSTs (drive compaction) | `TombstoneInventory`, `CompactionGcResult`, `CompactionCadence`; tombstone-ratio trigger ≈ `0.50` |
| `panel_version_gc` | retired panel versions (hot→cold, prune) | `PanelVersionRecord`, `RetentionPolicy` (keep 2 hot, cold-tier first) |
| `orphan_reconciler` | dangling slot/index↔base mismatches | `OrphanReport`, `OrphanRepairResult` |
| `ann_gc` | tombstoned ANN graph nodes (copy-on-write rebuild) | `SharedAnnIndex<T>`, `AnnGcResult`; rebuild interval ≈ `600_000` ms, max tombstone ratio ≈ `0.25` |

(Exact constant values live in `gc/<sub>.rs`; the snapshot-GC hook is invoked from
`VersionedCfStore::snapshot_gc_tick`.)

---

## 8. Key-encoding layers & secondary indexes

### 8.1 FoundationDB-style layers (`layers/`)

Each paradigm is a key-encoding layer over the ordered keyspace, with a leading
discriminant byte:

| Layer | Discriminant | Key | Value |
|---|---|---|---|
| Relational (`relational.rs`) | `0x01` | `0x01 ‖ collection_id(8) ‖ pk` | `be_u16 ROW_SCHEMA_VERSION ‖ bincode(Row)` |
| Document (`document.rs`) | `0x02` | `0x02 ‖ collection_id(8) ‖ doc_id(16) ‖ path` | Leaf / Tombstone / Branch cell |
| KV (`kv.rs`) | `0x03` | `0x03 ‖ collection_id(8) ‖ ns(8) ‖ user_key` | `KV_VALUE_VERSION(0x01) ‖ be_u64 expires_at_ms ‖ payload` (TTL checked on read) |
| TimeSeries (`timeseries.rs`) | `0x04` | `0x04 ‖ kind(point=0x00/rollup=0x01) ‖ collection_id(8) ‖ series(8) ‖ be ts/window` | point `be f64`; rollup `count(8) ‖ sum(f64) ‖ min(f64) ‖ max(f64)` |
| Blob (`blob.rs`) | `0x05` | chunk `0x05 ‖ 0x00 ‖ blob_id(16) ‖ be_u32 chunk_idx`; manifest `0x05 ‖ 0x01 ‖ blob_id(16)` | chunk bytes; manifest `total_bytes(8) ‖ chunk_count(4) ‖ blake3(32) ‖ cold_tier(1) ‖ created_at_ms(8)` |

Blob: `BLOB_CHUNK_SIZE = 262_144` (256 KiB), `MAX_BLOB_BYTES = 1 GiB`; chunks
committed before the manifest (content-addressed durability). Time-series rollup
windows are minute/hour/day (`NANOS_PER_MINUTE` etc.), updated atomically per write.

### 8.2 Secondary indexes (`index/`)

`IndexId(u32)` (4-byte BE in keys). `IndexSpec{index_id, name, kind, on_field,
field_type}`.
- **B-tree** (`index_btree`, discriminant `0x10`): key
  `0x10 ‖ collection_id(8) ‖ index_id(4) ‖ memcomparable(field_val) ‖ pk`, empty
  value. Field encodings are order-preserving: `I64` sign-flipped then BE; `U64`
  plain BE; `F64` IEEE total-order transform then BE; `Bool` one byte; `Text`/`Bytes`
  first 64 bytes (`MAX_INDEXED_BYTES = 64`) escape-terminated memcomparable.
- **Inverted** (`index_inverted`, discriminant `0x11`): posting key
  `0x11 ‖ collection_id(8) ‖ index_id(4) ‖ term_hash(8) ‖ pk`, value `f32 BE`
  (BM25 tf-weight). A reserved all-ones (`u64::MAX`) term hash stores stats
  (`doc_count(8) ‖ avgdl(f32)`). `BM25_K1=1.2`, `BM25_B=0.75`.

`btree.rs`, `inverted.rs`, `inverted_maintenance.rs`, `maintenance.rs`,
`rebuild/*`, `terms.rs` provide build/maintain/rebuild logic.

---

## 9. Dedup, time-travel, transactions

### 9.1 Dedup / idempotency (`dedup/`)

`DedupPolicy ∈ {Off, Exact, TctCosine(TctCosineConfig)}` (re-validated against the
panel on vault open). `DedupResult ∈ {New(CxId), DedupMerge{into,occurrence},
ExactDuplicate(CxId)}`. Content addressing: `CxId::from_input(bytes,
panel_version, salt)` and `full_content_hash` (BLAKE3 over length-delimited parts).
Recurrence signature detection (`signature.rs`) compares temporal-slot cosine and
event time. Compression aggregates (re-exported in `lib.rs`): `CompressionRatio`,
`Domain`, `DomainCompressionStats`, `compression_ratio`,
`domain_compression_stats`. Anchor-conflict threshold `ANCHOR_VECTOR_TAU = 0.70`.
Error codes include `CALYX_DEDUP_DPI_EXCEEDED`, `CALYX_DEDUP_ANCHOR_CONFLICT`,
`CALYX_RECURRENCE_SLOT_MISSING`.

### 9.2 Time-travel (`timetravel/`)

`TimeIndex` CF rows are `be_u64(millis_utc) ‖ be_u64(seqno)` → 1-byte sentinel.
`AsterVault::as_of(t_millis)` → `TimeTravelSnapshot` resolves the greatest seqno
with `millis ≤ t`, pins a reader lease there, releases on drop. `RetentionHorizon
∈ {Rolling{min_age}, Absolute{horizon_millis}, None}` lower-bounds `as_of`
(`CALYX_TIMETRAVEL_BEFORE_HORIZON` when violated); `safe_to_gc_before_millis`
feeds version GC. `TimeIndexEntry{millis, seqno}`.

### 9.3 Transactions (`txn/`)

`TxnHandle` is a vault-scoped serial transaction manager (one active txn per
vault, condvar-gated) with state `Idle | Active{started_at, cost_cap_ms}`.
`CrossModelTxn` batches writes across paradigm layers (`put_record`, `put_doc`,
`kv_set`, `ts_write`, `blob_put`, …); validation enforces schema, dedup policy,
and index maintenance (`validation.rs`). All layer writes for one logical change
commit in one MVCC batch (data key + index key in one transaction). Error codes:
`CALYX_TXN_TIMEOUT`, `CALYX_TXN_COST_CAP`.

---

## 10. Resource accounting, pressure, residency, redaction, erase

- `resource/` — `ResourceCounters` (atomic `memtable_absorbed_total`,
  `memtable_rejected_total`, `disk_pressure_events_total`), `BackpressureStatus`,
  `LeaseRegistry`/`LeaseView`, `MemtableStatus`/`MemtableCfStatus`,
  `collect_resource_status`/`ResourceStatus`/`VramBudgetStatus`.
- `pressure.rs` — `DiskPressureGuard` (high-water ratio default 0.85),
  `DiskSample`; `CALYX_DISK_PRESSURE` admission gate with spill request.
- `residency.rs` — immutable data-residency pin (`Residency{dataset_root,
  allow_off_dataset}`, sidecar `residency.json`); `authorize(target)` fails closed
  with `CALYX_RESIDENCY_VIOLATION` and writes a path-digest-only Ledger audit
  entry (no raw paths in the ledger).
- `retention.rs` — per-collection TTL/rollup sweep (`RetentionPolicy`,
  `RetentionStore`).
- `security/` — `LensStoreGuard` (cross-vault vector guard), ZFS encryption probe.
- `redaction.rs` — input PII modes (`InputMode ∈ {Full, HashOnly([u8;32]),
  Redacted}`, `redact_to_hash` = BLAKE3, `CALYX_PII_REDACTION_REQUIRED`). Per
  product policy, local-only/opt-in redaction is intentional, not a defect.
- `erase.rs` — lawful erasure (`EraseScope ∈ {Vault, Cx(CxId), Subject(SubjectId)}`,
  `EraseResult`, `EraseRegistry`); distinct from compaction.

### 10.1 Error taxonomy (codes surfaced by this crate)

`CALYX_ASTER_TORN_WAL`, `CALYX_ASTER_CORRUPT_SHARD`, `CALYX_ASTER_BASE_CORRUPT`,
`CALYX_FORMAT_VERSION_UNSUPPORTED`, `CALYX_BACKPRESSURE`, `CALYX_DISK_PRESSURE`,
`CALYX_STALE_DERIVED`, `CALYX_READER_LEASE_EXPIRED`, `CALYX_NOT_FOUND`,
`CALYX_IO_ERROR`, `CALYX_BOUNDS_EXCEEDED`, `CALYX_VAULT_KEYSPACE_MISMATCH`,
`CALYX_QUOTA_EXCEEDED`, `CALYX_ENCRYPTION_FAILED`, `CALYX_DECRYPTION_FAILED`,
`CALYX_VAULT_KEY_MISSING`, `CALYX_RESIDENCY_VIOLATION`,
`CALYX_TIMETRAVEL_BEFORE_HORIZON`, `CALYX_TXN_TIMEOUT`, `CALYX_TXN_COST_CAP`,
`CALYX_PII_REDACTION_REQUIRED`, plus the dedup codes above. (Many are constructed
via `calyx-core::CalyxError`; see [05_core.md](05_core.md).)

---

## 11. Gaps / not covered

- **Quantization / compression are not implemented in this crate.** Slot CF
  values are stored as big-endian IEEE bit patterns (`vault/encode.rs`); the PRD's
  PQ-8 / Float8 / binary quantization and zstd raw-sidecar compression are not
  present here. "Quantized" vs "Raw" slot CFs differ only by directory/tier
  policy, not by an encoding in this code.
- **ANN / DiskANN / SPANN indexes are not in calyx-aster.** The `idx/` directory
  tree, HNSW/DiskANN graphs, and sparse posting lists from the PRD live in
  `calyx-sextant` ([09_sextant_search.md](09_sextant_search.md)); aster only holds
  the `Kernel`/`Guard` CFs and the secondary B-tree/inverted indexes (§8.2). ANN
  GC here operates on a `SharedAnnIndex<T>` abstraction, not an on-disk ANN format.
- **Single-level SstLevel.** `SstLevel` is one newest-first stack per CF; multi-
  level LSM fan-out (L0/L1/…) is not implemented; `level:u8` is metadata only.
- **Manifest is JSON, not binary.** No binary manifest magic; the format gate is
  the `ManifestVersion` major/minor (1.0).
- **Compaction cadence is fixed.** `FIXME(PH46)` — the Anneal adaptive cadence
  hook is not wired; the scheduler uses fixed interval + write-amp backoff.
- `lib.rs` self-describes as a "skeleton"; several modules (`olap`, `plain_column`,
  `plain_graph`, `stream`, `stride_fsv`, `supply_chain`, `ledger_view`) were not
  deep-read for this doc and may contain partial implementations.

See also [04_storage_and_schema.md](04_storage_and_schema.md) (consolidated
on-disk format), [05_core.md](05_core.md) (CxId/SlotVector/error types),
[09_sextant_search.md](09_sextant_search.md) (ANN), and
[01_system_overview.md](01_system_overview.md).
