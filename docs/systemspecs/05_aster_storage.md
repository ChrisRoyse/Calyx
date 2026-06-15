# 05. Aster Storage Engine (calyx-aster)

Aster is Calyx's embedded storage engine: a write-ahead log with group commit, a
bounded in-memory memtable, immutable mmap'd SSTables, per-column-family LSM
levels, an atomic JSON manifest with crash recovery, snapshot-isolated MVCC, a
content-addressed constellation ingest path, snapshot-safe compaction with
hot/cold tiering, and btree/inverted secondary indexes. Every claim below is
traced to a source file and the function/type that implements it.

This document derives entirely from the source under `crates/calyx-aster/src`.
Statements that the code does not determine are marked "Not determined from
source."

See also: [11_ledger_provenance.md](11_ledger_provenance.md) (the Ledger CF and
hash-chained provenance referenced by the ingest path),
[04_constellation_model.md](04_constellation_model.md) (the `Constellation`,
`CxId`, `SlotVector` types encoded here), and
[18_resource_governance.md](18_resource_governance.md) (memtable byte caps,
reader-lease gap accounting, disk-pressure guards).

## Source files covered

- `src/lib.rs` — crate module map.
- `src/wal/mod.rs`, `src/wal/record.rs`, `src/wal/segment.rs`, `src/wal/batch.rs` — WAL + group commit.
- `src/memtable/mod.rs`, `src/memtable/bounded.rs` — bounded memtable.
- `src/sst/mod.rs`, `src/sst/bloom.rs`, `src/sst/level.rs` — SSTable writer/reader, bloom filter, LSM level.
- `src/cf/mod.rs`, `src/cf/family.rs`, `src/cf/key.rs`, `src/cf/router.rs` — column families, key codecs, router.
- `src/storage_names.rs` — canonical on-disk file-name contract.
- `src/manifest/mod.rs` — atomic manifest + `recover_vault`.
- `src/mvcc/mod.rs`, `src/mvcc/store.rs`, `src/mvcc/lease.rs` — MVCC store, sequence allocator, reader leases, snapshots.
- `src/compaction/mod.rs`, `src/compaction/tiering.rs` — compaction, debt, scheduler, hot/cold tiering.
- `src/vault.rs`, `src/vault/store.rs`, `src/vault/commit.rs`, `src/vault/durable.rs`, `src/vault/encode.rs`, `src/vault/cf_codec.rs`, `src/vault/keyspace.rs` — vault store, CRUD/constellation ingest, durable commit, codecs.
- `src/index/btree.rs`, `src/index/inverted.rs` — secondary indexes.

---

## 1. On-disk file/directory layout

A durable vault is rooted at one directory. Writers and recovery share the same
layout (`src/vault/durable.rs`, `src/cf/router.rs`, `src/manifest/mod.rs`).

| Path (relative to vault root) | Producer | Contents |
|---|---|---|
| `wal/{index:020}.wal` | `wal::Wal` | WAL segment files (`src/wal/segment.rs::segment_path`). |
| `wal/.append.lock` | `wal::Wal::open`/`append_batch` | Exclusive append lock (`file_lock::FileLockGuard`). |
| `cf/<family>/` | `CfRouter`, `DurableVault` | Per-column-family SSTable directory. `<family>` is `ColumnFamily::name()`. |
| `cf/<family>/{seq:020}.sst` | `CfRouter::flush_cf` | LSM router memtable flush. |
| `cf/<family>/{seq:020}-{index:04}.sst` | `DurableVault::write_rows` | Durable group-commit batch checkpoint. |
| `cf/<family>/compacted-{seq:020}.sst` | compaction | Compaction output. |
| `CURRENT` | `ManifestStore::write_current` | UTF-8 pointer to the active `manifest-*.json`. |
| `MANIFEST` | `ManifestStore::write_current` | Mirror copy of the active manifest bytes. |
| `manifest-{seq:020}.json` | `ManifestStore::write_current` | Immutable manifest snapshot. |
| `panel/…`, `registry/…`, `codebooks/…` | caller | Content-addressed immutable refs verified by manifest. |
| `locks/durable.commit.lock`, `locks/recurrence.write.lock` | `DurableVault` | Cross-process write serialization. |

With a `TieringPolicy`, CF directories also exist under the hot and archive
tier roots (`<tier_root>/cf/<family>/`); see §9.

The canonical name shapes are enforced fail-closed by `src/storage_names.rs`. A
`*.sst` or `*.wal` file whose name does not classify into one of the canonical
shapes raises `CALYX_ASTER_CORRUPT_SHARD` rather than being silently skipped
during recovery/scan (`classify_sst`, `wal_segment_index`,
`unrecognized_name`).

| SST name class | Pattern | `SstName` variant | `class_rank` (sort) |
|---|---|---|---|
| Router flush | `{seq:020}.sst` | `Router { seq }` | 1 |
| Durable batch | `{seq:020}-{index:04}.sst` | `DurableBatch { seq, index }` | 2 |
| Compaction | `compacted-{seq:020}.sst` | `Compacted { seq }` | 3 |

`canonical_seq` accepts *exactly* 20 ASCII digits parseable as `u64` (a 20-digit
string above `u64::MAX` is rejected). `SstOrderKey` orders by `(seq, class_rank,
index)`, so within one seq the router flush precedes the durable batch which
precedes the compaction output.

---

## 2. WAL format and group-commit algorithm

### 2.1 Record framing (`src/wal/record.rs`)

| Constant | Value | Meaning |
|---|---|---|
| `MAGIC` | `u32::from_le_bytes(b"CXW1")` | WAL record magic (LE on disk: bytes `43 58 57 31`). |
| `HEADER_LEN` | 20 | Bytes of fixed header per record. |
| `MAX_RECORD_BYTES` | `64 * 1024 * 1024` | Hard cap on one payload; `encode` rejects larger. |

On-disk record layout:

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 4 | magic | `MAGIC` little-endian |
| 4 | 8 | seq | `u64` LE |
| 12 | 4 | len | `u32` LE payload length |
| 16 | 4 | crc | `u32` LE CRC32 |
| 20 | len | payload | raw bytes |

The CRC (`payload_crc`) is `crc32fast` over `seq.to_le_bytes() ‖
len.to_le_bytes() ‖ payload` (header magic/crc fields excluded).

`decode_at` returns `DecodeStatus::Eof` on a zero-byte read, `Complete` on a
verified record, or `Torn { offset, message }` for: a partial header
(`read < HEADER_LEN`), bad magic, a `len` exceeding `MAX_RECORD_BYTES`, a partial
payload (`UnexpectedEof`), or a CRC mismatch.

### 2.2 Segment naming and rotation (`src/wal/segment.rs`, `src/wal/mod.rs`)

Segments are `{index:020}.wal`. `list_segments` parses indexes via
`wal_segment_index`, sorts them, and `validate_contiguous` raises
`CALYX_ASTER_CORRUPT_SHARD` if the sorted indexes have a gap (a missing segment
would silently drop committed writes). `WalOptions` defaults:
`max_segment_bytes = 64 MiB`, `group_commit_window = 2 ms`
(`DEFAULT_GROUP_COMMIT_WINDOW`). `rotate_if_needed` `sync_all()`s the active
segment then opens `active_index + 1` when `offset + incoming_bytes >
max_segment_bytes` (and `offset != 0`).

### 2.3 Append path (`Wal::append_batch`)

1. Acquire `wal/.append.lock` (`FileLockGuard`).
2. `refresh_after_external_appends_locked`: if another process appended
   (segment count or active-segment length changed), replay to recompute
   `next_seq` and reopen the active segment.
3. For each payload: assign `seq = next_seq`; `record::encode`;
   `rotate_if_needed`; seek to end; `write_all`; bump `next_seq` and
   `active_len`.
4. After all payloads, a single `file.sync_data()` fsyncs the whole batch before
   returning the `AppendAck { seq, segment_path, start_offset, end_offset }`
   list. (`append` is the one-payload wrapper.)

On Unix, newly created segments also fsync their parent directory
(`sync_parent`); on non-Unix this is a no-op (`#[cfg(not(unix))]`).

### 2.4 Group commit (`src/wal/batch.rs`)

`GroupCommitBatcher::new` spawns a thread holding the `Wal` behind a `Mutex`.
`validate_window` rejects any window `> 2 ms` with `CALYX_DISK_PRESSURE`. Steps
in `run_batcher`:

1. Block on `receiver.recv()` for the first request.
2. If the first op is not `Append` (it is `Flush` or `TipSeq`), service it alone
   and loop.
3. Otherwise set `deadline = now + group_commit_window` and drain further
   requests via `recv_timeout` until the deadline, a timeout, or disconnect.
4. `flush_requests` collects all `Append` payloads and calls
   `Wal::append_batch` once (one fsync for the coalesced group), then routes one
   `AppendAck` back to each submitter; `Flush` replies after the batch;
   `TipSeq` replies with `durable_tip_seq` (computed once and cached per flush).
   On `append_batch` error, every request in the group receives the cloned error.

`submit`, `flush_sync`, and `tip_seq` are the public client methods; each sends a
one-shot reply channel and blocks for the response.

---

## 3. Memtable (`src/memtable/bounded.rs`)

`BoundedMemtable` (aliased `Memtable`) is an ordered `Mutex<BTreeMap<Vec<u8>,
Vec<u8>>>` with a hard byte cap and a high-water flush signal.

| Field | Type | Meaning |
|---|---|---|
| `entries` | `Mutex<BTreeMap<Vec<u8>,Vec<u8>>>` | Ordered key→value rows. |
| `cap_bytes` | `usize` | Hard admission cap. |
| `high_water_bytes` | `usize` | Flush-trigger threshold (`min(cap)`). |
| `used_bytes` | `AtomicUsize` | Running charged size. |

Constants/formulas: `ENTRY_OVERHEAD_BYTES = 4`; `entry_size = key.len() +
value.len() + 4` (saturating); default high-water `high_water_for(cap) = cap*4/5`
(80%).

`write(key, value, seq)`: computes projected bytes (subtracting any existing
row's charge, adding the new charge); if `accepted_bytes > cap` or `next_bytes >
cap`, returns `CALYX_BACKPRESSURE` (fail closed). Otherwise inserts and returns a
`WriteAck { seq, accepted_bytes, used_bytes, cap_bytes, high_water_bytes,
flush_triggered }`. `flush_trigger_for` returns true when `cap_bytes == 0` or
`used_bytes >= high_water_bytes`. `reset_after_flush` saturating-subtracts
flushed bytes from `used_bytes`. `freeze()` consumes the table into a
`FrozenMemtable` for handoff; `flush_to_sst` writes the snapshot via
`sst::write_sst`.

---

## 4. SSTable format (`src/sst/mod.rs`, `src/sst/bloom.rs`)

### 4.1 Magic / versions / sizes

| Constant | Value |
|---|---|
| `MAGIC` | `b"CXS1"` |
| `LEGACY_VERSION` | 1 (header lacks body CRC verification) |
| `VERSION` | 2 (current; body CRC verified) |
| `HEADER_LEN` | 32 |
| `RECORD_HEADER_LEN` | 12 |
| `INDEX_ENTRY_FIXED_LEN` | 12 |

### 4.2 File layout

A file is `header ‖ records ‖ index ‖ bloom`, written by `write_sst` which
requires the input strictly sorted by key (`ensure_sorted`, else
`CALYX_ASTER_CORRUPT_SHARD`). It writes to `path.with_extension("sst.tmp")`,
`sync_all`s, then `fs::rename`s into place and `sync_parent`s — atomic publish.

**Header (32 bytes, `write_header`):**

| Offset | Size | Field | Encoding |
|---|---|---|---|
| 0 | 4 | magic | `CXS1` |
| 4 | 4 | version | `u32` LE (2) |
| 8 | 4 | entries | `u32` LE |
| 12 | 8 | index_offset | `u64` LE |
| 20 | 8 | bloom_offset | `u64` LE |
| 28 | 4 | body_crc | `u32` LE CRC32 of bytes `[32..]` |

**Record (per row, `write_record`):**

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | key_len `u32` LE |
| 4 | 4 | value_len `u32` LE |
| 8 | 4 | record crc `u32` LE (CRC32 over key‖value) |
| 12 | key_len | key |
| 12+key_len | value_len | value |

**Index entry (`write_index`):** `key_len (u32 LE) ‖ record_offset (u64 LE) ‖
key`. `read_index` requires the running offset to land exactly on `bloom_offset`
(else `SST index length mismatch`).

**Bloom (`BloomFilter::encode`):** `bit_count (u64 LE) ‖ hash_count (u32 LE) ‖
byte_len (u32 LE) ‖ bits`.

### 4.3 Reader (`SstReader`)

`open` mmaps the file (`MmapColumn`), reads/validates the header, parses the
index, and decodes the bloom. `read_header` fails closed on: missing header, bad
magic, unsupported version (`!= 2 && != 1`), out-of-bounds offsets (`index_offset
< 32`, `index_offset > len`, `bloom_offset < index_offset`, `bloom_offset >
len`), and — for `version >= 2` — a body-CRC mismatch.

`get(key)`: bloom `may_contain` short-circuit, then `binary_search` the in-memory
index, then `read_record` (which re-verifies the per-record CRC). `range(start,
end)` uses `partition_point` for `[start, end)`; `iter` returns all rows.

### 4.4 Bloom filter (`src/sst/bloom.rs`)

`from_keys`: `bit_count = max(64, next_power_of_two(max(1, n)*16))`,
`hash_count = 3`. `bit_index` hashes `key ‖ round.to_le_bytes()` with BLAKE3 and
takes `u64(first 8 bytes) % bit_count`. `may_contain` requires all 3 bits set;
no false negatives by construction. `decode` rejects buffers `< 16` bytes, zero
`bit_count`/`hash_count`, or a `bits` length that disagrees with
`bit_count.div_ceil(8)`.

### 4.5 LSM level (`src/sst/level.rs`)

`SstLevel` is an ordered `Vec<PathBuf>`; `push` inserts at index 0 so iteration
is **newest-first**. `get` returns the first match scanning newest→oldest (newest
wins). `range`/`iter` merge into a `BTreeMap` with `or_insert` so the
newest-first traversal makes the newest value win on duplicate keys.

---

## 5. Column families (`src/cf/family.rs`)

`ColumnFamily` is an enum. Non-slot variants are listed in `ColumnFamily::STATIC`
(31 entries); `Slot { slot: SlotId, kind: SlotFamilyKind }` is parameterized.
`SlotFamilyKind` is `Quantized` or `Raw` (raw f32 sidecar). Selected variants and
their documented key→value contracts (from the doc comments on `family.rs`):

| Variant | `name()` | Documented key → value |
|---|---|---|
| `Base` | `base` | `CxId → ConstellationHeader`/base record |
| `Collections` | `collections` | `b"coll\0" ‖ name → Collection metadata` |
| `Relational` | `relational` | `0x01 ‖ collection_id ‖ pk_len ‖ pk → row` |
| `Document` | `document` | `0x02 ‖ collection_id ‖ doc_id ‖ path → leaf/tombstone` |
| `Kv` | `kv` | `0x03 ‖ collection_id ‖ ns ‖ key_len ‖ user_key → version‖expires_at‖payload` |
| `TimeSeries` | `timeseries` | `0x04 ‖ tag ‖ collection_id ‖ series ‖ ts → point/rollup` |
| `Blob` | `blob` | `0x05 ‖ tag ‖ collection_id ‖ blob_id ‖ chunk_idx → chunk/manifest` |
| `Slot{Quantized}` | `slot_{NN}` | `CxId → quantized slot vector` |
| `Slot{Raw}` | `slot_{NN}.raw` | `CxId → raw f32 sidecar` |
| `XTerm` | `xterm` | `(CxId, a, b, kind) → cross-term value` |
| `TemporalXTerm` | `temporal_xterm` | `(CxId_a, CxId_b) → temporal cross-term` |
| `Scalars` | `scalars` | `(ScalarId, CxId) → f64` |
| `Anchors` | `anchors` | `(CxId, AnchorKind) → AnchorValue+source+ts` |
| `Assay` | `assay` | `(panel_version, corpus_shard, subject) → AssayRow` |
| `Ledger` | `ledger` | `seq → hash-chained provenance entry` |
| `Recurrence` | `recurrence` | `(CxId, OccurrenceId) → occurrence/summary` |
| `Graph` | `graph` | plain-collection graph rows (nodes/edges/CSR) |
| `Online` | `online` | typed online/adaptation state |
| `Reactive` | `reactive` | reactive trigger audit/fired rows |
| `Anneal*` (13 CFs) | `anneal_*` | Anneal subsystem state snapshots |
| `TimeIndex` | `time_index` | `BE_u64(millis_utc) ‖ BE_u64(seqno) → sentinel` |
| `IndexBtree` | `index_btree` | `0x10 ‖ collection_id ‖ index_id ‖ field_val ‖ pk → ∅` |
| `IndexInverted` | `index_inverted` | `0x11 ‖ collection_id ‖ index_id ‖ term_hash ‖ pk → f32_be` |

There are **two distinct CF tagging schemes**, both round-trip tested:

1. **Keyspace tag** (`ColumnFamily::keyspace_tag` / `parse_keyspace_tag`, used by
   per-vault keyspace isolation): non-slot CFs encode to a single byte = their
   index in `STATIC`; slot CFs encode to `SLOT_KEYSPACE_TAG (0xF0) ‖ slot_id_be(2)
   ‖ kind_byte (0=Quantized,1=Raw)`.
2. **WAL/batch tag** (`src/vault/cf_codec.rs::cf_tag` / `decode_cf`): a fixed
   per-CF `u8`. Notable values: `Base=0`, `Anchors=1`, `Ledger=2`, `XTerm=3`,
   `Scalars=4`, `Online=5`, slot quantized = `16 + slot` (16..=63), slot raw =
   `64 + slot` (64..=111), `AnnealBandit=112`, `TimeIndex=116`,
   `Collections=117`…`Blob=122`, `IndexBtree=123`, `IndexInverted=124`,
   `AnnealOperators=125`, `Reactive=126`. Uniqueness and round-trip are asserted
   by `every_static_cf_tag_round_trips_uniquely`.

`parse_cf_dir_name` (`src/storage_names.rs`) is the inverse of `name()` for
recovery and fails closed on unknown or non-canonical directory names.

### 5.1 Key encoding (`src/cf/key.rs`)

All composite keys are big-endian so lexicographic byte order matches natural
order. `CX_ID_BYTES = 16`, `FULL_HASH_BYTES = 32`.

| Function | Key bytes |
|---|---|
| `base_key(cx)` / `slot_key(cx)` | `CxId` (16B) |
| `xterm_key(cx,a,b,kind)` | `CxId ‖ a_be(2) ‖ b_be(2) ‖ kind_code(1)` |
| `temporal_xterm_key(a,b)` | `CxId_a ‖ CxId_b` (32B) |
| `scalar_key(scalar,cx)` | `scalar_be(4) ‖ CxId` |
| `anchor_key(cx,kind)` | `CxId ‖ encode_anchor_kind(kind)` |
| `ledger_key(seq)` | `seq_be(8)` |
| `recurrence_key(cx,occ)` | `CxId ‖ occ_be(8)` |
| `online_key(kind,id)` | `kind_code(1) ‖ id_be(8)` |

`XTermKind` codes: `Concat=0, Interaction=1, Agreement=2, Delta=3`.
`OnlineKeyKind` codes: `MistakeLog=0, ReplayBuffer=1, HeadState=2,
DeltaJQueue=3`. `encode_anchor_kind` emits a `u16` BE discriminant
(`TestPass=0 … Recurrence=6`, `Label=7` followed by length-delimited UTF-8).

`prefix_range(prefix)` builds the `[prefix, prefix_upper_bound)` range where
`prefix_upper_bound` increments the last non-`0xff` byte (returns `None` —
unbounded — for an all-`0xff` prefix). `cx_prefix_range` and friends reuse it.
`full_content_hash` BLAKE3-hashes a sequence of length-delimited parts
(`len_be(8) ‖ part`); `cx_id_from_full_hash` takes the first 16 bytes as the
`CxId`; `verify_cx_hash_prefix` re-checks that a stored `CxId` equals the hash
prefix or returns `CALYX_ASTER_CORRUPT_SHARD`.

### 5.2 Per-vault keyspace isolation (`src/vault/keyspace.rs`)

`KeyspaceGuard::encode_key(cf, user_key) = vault_prefix(16) ‖ cf.keyspace_tag()
‖ user_key`. The prefix is the full 16-byte vault ULID (chosen over 8 bytes to
avoid prefix-aliasing collisions). `decode_key` fails closed
(`CALYX_VAULT_KEYSPACE_MISMATCH`) if a key lacks this vault's prefix, so one
vault's read path cannot structurally reach another's key range.

### 5.3 Router (`src/cf/router.rs`)

`CfRouter` owns per-CF `Memtable`, `SstLevel`, and a next-file counter.
`DEFAULT_MEMTABLE_BYTES = 8 MiB` (used when the cap is 0). `open_with_tiering`
creates `cf/` (and per-tier `cf/` roots), `ensure_cf`s every `STATIC` family,
then `load_existing`. `put`: write to the memtable; on `CALYX_BACKPRESSURE`,
`flush_cf` and retry once (counting absorbed/rejected on `ResourceCounters`); if
the ack reports `flush_triggered`, flush. `flush_cf` swaps in a fresh memtable,
freezes the old one, and writes `{seq:020}.sst`. `get` checks the memtable then
the level; `range`/`iter_cf` merge level + memtable into a `BTreeMap` (memtable
overwrites, i.e. wins). `load_existing` scans all CF roots, parses dir names,
sorts SSTs by `SstOrderKey`, and seeds the next-file counter from the max
`Router{seq}` only (durable/compaction names use disjoint shapes).

---

## 6. MVCC, sequences, and snapshots (`src/mvcc/`)

### 6.1 Sequence allocator (`src/mvcc/lease.rs`)

`SeqAllocator` is an `AtomicU64` `current` plus an `allocated` flag. `allocate()`
sets `allocated` and returns `fetch_add(1)+1` (next write is `start+1`).
`set_start_seq` is rejected (`CALYX_BACKPRESSURE`) once any allocation happened —
recovery must set the start before live writes. `advance_to_at_least` CAS-bumps
`current` up to an externally observed seq.

### 6.2 Reader leases and freshness

`ReaderLease { id, pinned_seq, issued_at, max_age_ms }`. `expires_at =
issued_at + max_age_ms` (saturating); `ensure_live_at(now)` returns
`CALYX_READER_LEASE_EXPIRED` once `now >= expires_at`. `Freshness` is
`FreshDerived` or `StaleOk { max_lag }`; `ensure(pinned, derived)` returns
`CALYX_STALE_DERIVED` when a derived structure lags the pinned seq beyond what
the policy allows. `Snapshot { seq, freshness, lease }` bundles them.

### 6.3 Versioned store (`src/mvcc/store.rs`)

`VersionedCfStore` holds `rows: RwLock<BTreeMap<(ColumnFamily, Vec<u8>),
Vec<VersionedValue>>>` where each `VersionedValue { seq, value }` appends to a
per-key version chain. A tombstone is the sentinel value
`b"\0CALYX_ASTER_TOMBSTONE_V1"` (`TOMBSTONE_VALUE`; `tombstone_value()` /
`is_tombstone_value`).

**Commit** (`commit_batch`): takes the row-table write lock, optionally pushes
all rows through the `CfRouter` (so the LSM/memtable mirrors the in-memory
table), allocates one seq via `SeqAllocator::allocate`, and appends `{seq,
value}` to each key's chain. The whole group shares one seq — atomic across any
number of CFs. `restore_batch` re-inserts a durable batch at its *original* seq
during recovery without allocating.

**Visibility** (`visible_version`): scan the version chain in reverse, take the
first `version.seq <= snapshot.seq`, and drop it if it is a tombstone. So a read
at seq S sees the newest non-tombstone version committed at or before S — classic
snapshot isolation. `read_at`, `read_batch`, `seq_for_key_at`, `scan_cf_at`,
`scan_cf_range_at`, and `scan_cf_range_page_at` all first `ensure_snapshot_live`
(lease not expired) and `ensure_unbarriered` (no read barrier blocks the key),
then resolve visible values. The paged scan walks the `BTreeMap` range with a
`Bound::Excluded`/`Included` lower bound and stops once it has `limit` rows or
passes the CF/range upper bound.

**Read barriers** (`ReadBarrier`, `install_read_barrier`/`remove_read_barrier`):
a registered barrier makes matching CF/key reads fail closed (`first_blocking`),
used to fence off corrupt or quarantined ranges.

**Snapshot pinning**: `pin_snapshot` registers a `ReaderLease` in the
`LeaseRegistry` for oldest-pinned-seq gap accounting; `pin_snapshot_at` pins a
historical seq for time-travel; `snapshot_gc_tick` aborts expired readers and
checks the lease gap on a scheduler cadence.

---

## 7. CRUD and constellation ingest path (`src/vault/`)

`AsterVault<C: Clock>` is the single-vault store. A durable vault holds a
`VersionedCfStore` (with a `CfRouter`) plus a `DurableVault` (WAL + manifest).
`DEFAULT_LEASE_MS = 5000`.

### 7.1 Constellation `put` (`src/vault/store.rs`, `VaultStore::put`)

1. Reject if `constellation.vault_id != self.vault_id`
   (`CALYX_VAULT_ACCESS_DENIED`); `validate_schema()`.
2. Under the durable commit lock, read the existing `Base` row at the latest
   snapshot.
   - If present and byte-identical to the re-encoded base → return the `CxId`
     (idempotent no-op).
   - If present with the *same identity* (`same_constellation_identity`) but
     different bytes → decode both and run `check_anchor_conflict`; a conflict
     raises `CALYX_ASTER_CORRUPT_SHARD`, otherwise the duplicate is accepted.
   - If present with a different identity → `CALYX_ASTER_CORRUPT_SHARD` ("CxId
     collision or non-idempotent duplicate").
3. Stage rows: a Ledger row (via the ledger hook `stage_ingest`, which also sets
   `constellation.provenance`; or a `ledger_stub` row when no hook), the `Base`
   row (`encode_constellation_base`), one `slot(slot)` row per slot vector
   (`encode_slot_vector`), and one `Anchors` row per anchor (`encode_anchor`).
4. `commit_rows_locked` (§7.3), then `commit_staged` the ledger entry.

`get(id, snapshot)` reads the `Base` row, decodes the header/scalars/anchors,
then `read_batch`es each slot CF; if a quantized slot row fails to decode it
falls back to the `slot_raw` sidecar (`CALYX_ASTER_CORRUPT_SHARD` if that is also
missing). `anchor(id, anchor)` re-reads, appends the anchor, and commits updated
`Base` + `Anchors` rows under the recurrence write lock.

Raw CF access bypasses constellation encoding: `write_cf` / `write_cf_batch`
(commit via the WAL/MVCC path) and `read_cf_at` / `scan_cf_at` /
`scan_cf_range_at` / `scan_cf_range_page_at` (snapshot reads).
`stage_constellation_rows` is the shared batch builder used by bulk ingest.

### 7.2 Base record encoding (`src/vault/encode.rs`)

`ConstellationHeader` is a fixed **102-byte** header (`HEADER_LEN = 102`):

| Offset | Size | Field |
|---|---|---|
| 0 | 16 | cx_id |
| 16 | 16 | vault_id (ULID bytes) |
| 32 | 4 | panel_version (BE) |
| 36 | 8 | created_at (BE) |
| 44 | 1 | modality tag |
| 45 | 1 | flags bits |
| 46 | 2 | n_slots (BE) |
| 48 | 2 | n_anchors (BE) |
| 50 | 8 | ledger_seq (BE) |
| 58 | 32 | input_hash |
| 90 | 12 | reserved zero padding |

`encode_constellation_base` appends, after the header: a 32-byte identity hash
(BLAKE3 over a header copy with `n_anchors`/`ledger_seq` zeroed plus slot hashes,
scalars, and metadata), the input-ref tail (`redacted` byte + optional pointer
string), slot count + per-slot `(slot_id_be, blake3(slot_vector))`, scalar count
+ `(string, f64 bits BE)` pairs, anchor count + length-prefixed anchor blobs, the
32-byte provenance hash, and string metadata. Decoding fails closed on trailing
bytes (`trailing bytes after constellation metadata`).

`modality_tag`: `Text=0, Code=1, Image=2, Audio=3, Video=4, Structured=5,
Mixed=6, Protein=7, Dna=8, Molecule=9`. `flags_bits` packs
`ungrounded|degraded<<1|novel_region<<2|redacted_input<<3`.

`encode_slot_vector` tags: `Dense=0` (`dim_be(4)` then `f32` bits BE each),
`Absent=1` (absent reason), `Sparse=2` (`dim`, count, `(idx, f32 bits)` pairs),
`Multi=3` (`token_dim`, count, then each token's f32 bits). Unknown tags →
`CALYX_ASTER_CORRUPT_SHARD`.

### 7.3 Durable commit path (`src/vault/commit.rs`)

`commit_rows` runs `commit_rows_locked` under the durable commit lock
(`locks/durable.commit.lock`); the lock also triggers `refresh_from_durable` when
another writer's WAL tip is ahead of this process's seq.

`commit_rows_locked`:
1. Empty batch → commit nothing (no seq bump, no time-index stamp).
2. Otherwise predict `predicted = current_seq + 1`, build a `TimeIndex` row
   (`timetravel::entry_row(clock.now(), predicted)` → `BE_u64(millis) ‖
   BE_u64(seqno)`), and append it to the rows so the `(millis → seqno)` mapping
   commits **atomically** with the data.

`commit_prepared_rows`:
1. `ensure_memtable_admission` — fail closed (`CALYX_BACKPRESSURE`) before any
   WAL write if a row can never fit in an empty memtable.
2. No durable backend → commit straight to MVCC.
3. With a durable backend: `ensure_disk_write_allowed` (disk-pressure guard),
   then `durable.append_batch(rows)` (WAL group commit) yielding `durable_seq`.
4. Commit the same rows to MVCC. On MVCC/router failure, `restore_batch` at
   `durable_seq`, write the durable checkpoint, and return `durable_seq` (the WAL
   record is the source of truth).
5. Assert `mvcc_seq == durable_seq` (else `CALYX_ASTER_CORRUPT_SHARD`), then
   `stage_checkpoint_batch` for later SST materialization.
6. The time-index prediction is re-checked against the committed seq and fails
   loud on divergence.

### 7.4 WAL batch payload (`src/vault/encode.rs`)

`encode_write_batch`: `row_count (u32 BE)`, then per row `cf_tag(u8) ‖ key
(u32-len-prefixed) ‖ value (u32-len-prefixed)`. `decode_write_batch` is the
inverse via `decode_cf`. This is the payload carried in each WAL record (§2.1).

### 7.5 Durable batch materialization (`src/vault/durable.rs`)

`write_rows(seq, rows)` groups rows by CF (sorted by `cf.name()`), sorts each
group by key, and writes one `{seq:020}-{first_index:04}.sst` per CF (the
`first_index` ties the SST file back to the row's position in the batch).
`stage_checkpoint_batch` accumulates `(seq, rows)`; `flush` drains them via
`write_rows` then `write_manifest` advances `durable_seq`.

---

## 8. Manifest and crash recovery (`src/manifest/mod.rs`, `src/vault/durable.rs`)

### 8.1 Manifest format and constants

| Constant | Value |
|---|---|
| `CURRENT_FILE` | `CURRENT` |
| `MANIFEST_FILE` | `MANIFEST` |
| `MANIFEST_PREFIX` / `MANIFEST_SUFFIX` | `manifest-` / `.json` |
| `SUPPORTED_MANIFEST_MAJOR.MINOR` | `1.0` |

`VaultManifest` (serde-JSON, `serde_json::to_vec_pretty`) fields:
`version {major,minor}`, `manifest_seq` (must be ≥1), `durable_seq`, `panel_ref`,
optional `registry_ref`, `codebook_refs`, optional `temporal_policy`/
`dedup_policy`, `retention_horizon`, `degraded_rebuildable`, `quarantines`.
`ImmutableRef { logical_path, blake3_hex }` is content-addressed; `validate`
requires a vault-relative path that does not escape (`..`, root, prefix), is not a
control file, and a 64-hex-char (32-byte) BLAKE3 digest. `validate` also requires
`panel_ref` under `panel/`, `registry_ref` under `registry/`, and each
`codebook_ref` under `codebooks/` (deduped).

### 8.2 Atomic manifest swap (`ManifestStore::write_current`)

1. `validate()` the manifest.
2. `write_atomic` the bytes to `manifest-{seq:020}.json`, then to the `MANIFEST`
   mirror, then write the pointer string to `CURRENT`. `write_atomic` =
   write to `*.tmp`, `sync_all`, `fs::rename`, fsync parent dir (Unix).

`load_current` reads `CURRENT`, validates the pointer name
(`valid_manifest_filename`), reads the pointed manifest, decodes+validates it,
and `verify_immutable_refs` re-hashes every `panel/registry/codebook` file
against its recorded BLAKE3 (mismatch → `CALYX_ASTER_CORRUPT_SHARD`).
`append_quarantine` adds a `QuarantineRecord` and bumps `manifest_seq`.

### 8.3 Recovery algorithm (`recover_vault` + `DurableVault::recover_batches`)

`recover_vault(vault_dir)`:
1. `load_current()` the manifest (fails closed on corruption/hash mismatch).
2. `replay_dir(vault_dir/wal)` — WAL replay (truncating a torn tail, §2.1/§2.3).
3. Keep only WAL records with `seq > manifest.durable_seq` (records already
   materialized into manifested SSTs are dropped — manifest-first ordering).
4. `last_recovered_seq` = last surviving WAL record's seq, else `durable_seq`.
   Return `RecoveryOutcome { manifest, wal_records, torn_tail,
   last_recovered_seq, degraded_rebuildable }`.

`DurableVault::recover_batches`:
- If `CURRENT` exists: `recover_vault`, then `read_manifested_batches` reads
  every `DurableBatch`/`Compacted` SST with `seq <= durable_seq` from all
  (tiered) CF roots, reconstructing each batch's rows ordered by their original
  in-batch index; then append the surviving WAL records (decoded via
  `decode_write_batch`). Router-flush SSTs are skipped here (the `CfRouter`
  recovers those itself).
- If `CURRENT` is absent (no manifest yet): replay the WAL alone;
  `last_recovered_seq` = last WAL record seq (or 0).

`AsterVault::open_with_clock` ties it together: recover batches, recover the
ledger hook, open the `CfRouter`, build the `VersionedCfStore` at
`last_recovered_seq`, `restore_batch` every recovered batch at its original seq,
`set_start_seq(last_recovered_seq)`, then open the live `DurableVault`. The
`VaultRecoveryReport { last_recovered_seq, torn_tail }` is exposed via
`recovery_report()`.

---

## 9. Compaction and tiering (`src/compaction/`)

### 9.1 Compaction (`src/compaction/mod.rs`)

`SstShard { cf, path, level, bytes }`. `CompactionCatalog` holds the active
shard set in `RwLock<Arc<Vec<SstShard>>>`; `pin_snapshot` clones the `Arc` so an
old `CompactionSnapshot` survives a compaction swap (snapshot-safe). A snapshot
`get(cf, key)` scans matching shards **newest→oldest** (`.rev()`) and returns the
first hit.

`compact_shards(cf, inputs, output, throttle)`:
1. Measure `CompactionDebt` (below). If `< 2` inputs → `Skipped`. If a throttle
   `max_input_bytes` is set and exceeded → `Skipped`.
2. Merge all input SST rows into a `BTreeMap` (later/larger files overwrite — the
   iteration order makes newer shards win), producing sorted unique entries.
3. `write_sst` the merged output at `level = max(input levels) + 1`.
4. Return a `CompactionReport` with input/output/logical bytes and
   `write_amp_milli = output_bytes * 1000 / max(1, logical_bytes)`.

`CompactionCatalog::compact_cf` then atomically swaps the active set: keep all
shards of other CFs, drop this CF's inputs, push the single compacted output.

`CompactionDebt::measure(shards, target)`: `pending_bytes = Σ shard.bytes`,
`score_milli = pending_bytes * 1000 / max(1, target)`.
`DEFAULT_COMPACTION_TARGET_BYTES = 64 MiB`, `WRITE_AMP_SCALE = 1000`.

### 9.2 Scheduler

`CompactionScheduler::start` runs a background thread.
`CompactionSchedulerOptions` defaults: `interval_ms = 10000`,
`debt_trigger_score_milli = 1000`, `max_write_amp_milli = 2000`,
`backoff_factor = 2`, `max_interval_ms = 60000`. Each tick: for each CF whose
`debt.score_milli >= debt_trigger`, `compact_cf`; if the resulting
`write_amp_milli > max_write_amp_milli`, multiply the interval by `backoff_factor`
(capped at `max_interval_ms`) — anti-storm backoff. (A `FIXME(PH46)` notes the
fixed cadence is to be replaced by an Anneal adaptive hook.)

### 9.3 Tiering (`src/compaction/tiering.rs`)

`TieringPolicy { hot_root, archive_root, active_slots, current_panel_version }`.
`StorageTier` is `Hot`/`Cold`. `place_cf(cf, panel_version)` resolves a
`TierPlacement { tier, root, cf_dir }` (`cf_dir = cf/<name>`). `is_cold` rules:

- Always **Hot**: `Base, Ledger, Anchors, Graph, Reactive`, and the Anneal
  durability/state CFs (`AnnealChecksums, AnnealMistakes, AnnealReplay,
  AnnealHeads, AnnealBandit, AnnealSoak, AnnealReport, AnnealGrowth,
  AnnealOperators`).
- Always **Cold**: any raw slot sidecar (`is_raw_slot`).
- A quantized slot is **Cold** when `panel_version < current_panel_version` or
  the slot is not in `active_slots`; otherwise Hot.

`write_tiered_sst` refuses non-canonical file names (`classify_sst` must
recognize them) before writing. `aiwonder()` is a preset rooting hot at
`/zfs/hot/calyx` and archive at `/zfs/archive/calyx` (falling back to
`$CALYX_HOME` or `/home/croyse/calyx`).

---

## 10. Secondary indexes (`src/index/`)

### 10.1 Btree index (`src/index/btree.rs`)

Discriminant `DISC_BTREE_INDEX = 0x10`; CF `index_btree`. Key:
`0x10 ‖ collection_id (8B BE) ‖ index_id (4B BE) ‖ field_val_encoded ‖ pk_bytes`;
value is **empty** (existence is the signal). The 13-byte prefix is `PREFIX_BYTES
= 1 + 8 + 4`. `MAX_INDEXED_BYTES = 64`.

`field_val_encoded` is **memcomparable** (byte order = value order). Per type:

| Type | Encoding |
|---|---|
| `Bool` | single byte `0x00`/`0x01` |
| `I64` | `(value as u64) XOR 0x8000_0000_0000_0000`, 8B BE (negatives sort first) |
| `U64` | plain 8B BE |
| `Timestamp` | non-negative i64 ns as 8B BE (negative → invalid-input) |
| `F64` | IEEE-754 total-order bits (`f64_order_bits`) 8B BE; `-0.0`→`+0.0`; NaN rejected |
| `Text` | first 64 bytes, escape-terminated memcomparable |
| `Bytes` | first 64 bytes, escape-terminated memcomparable |

Memcomparable encoding (`encode_memcomparable`): each `0x00` byte → `0x00 0xff`;
value terminated by `0x00 0x01` (`ESC=0x00`, `ESC_LITERAL=0xff`, `ESC_TERM=0x01`).
This is self-delimiting and prefix-correct (a byte-prefix sorts first). NULL is
not indexable and fails closed (`CALYX_INVALID_ARGUMENT`).

Queries (`btree_range_at`, `btree_range`, `btree_point`, `btree_count`) pin one
MVCC snapshot (`vault.latest_seq()` unless given), scan the index-key range, and
for each entry verify the data row is still live at the snapshot
(`pk_is_live` dispatches on `CollectionMode`: Relational/KV/TimeSeries point
reads, Documents prefix-scan). Stale entries (index present, row gone) are
skipped and logged (`CALYX_INDEX_STALE_ENTRY`). `range_bounds` makes `lte`
inclusive by taking the lexicographic upper bound of the value's scan prefix.

### 10.2 Inverted index (`src/index/inverted.rs`)

Discriminant `DISC_INVERTED_INDEX = 0x11`; CF `index_inverted`. Posting key:
`0x11 ‖ collection_id (8B BE) ‖ index_id (4B BE) ‖ term_hash (8B BE) ‖ pk_bytes`;
posting value = BM25 term-frequency component as `f32` BE (4 bytes,
`POSTING_VALUE_BYTES`). A reserved key with `term_hash = u64::MAX`
(`STATS_TERM_HASH`) and no pk stores `doc_count (u64 BE) ‖ avgdl (f32 BE)` (12
bytes, `STATS_VALUE_BYTES`); decode rejects non-finite/negative `avgdl`.

BM25 constants: `BM25_K1 = 1.2`, `BM25_B = 0.75`. `bm25_tf_weight(tf, doc_len,
avgdl) = tf / (tf + k1*(1 - b + b*doc_len/avgdl))`; `bm25_idf(N, df) = ln((N - df
+ 0.5)/(df + 0.5) + 1)`. `inverted_put` stores per-term posting rows;
`inverted_match`/`inverted_match_at` scan a term prefix, filter to rows whose
Relational data row is live at the snapshot, and return `(pk, weight)` sorted
descending. `inverted_bm25[_and]` sums `weight * idf` across query terms with
`Or`/`And` (all-terms-required) modes and an optional `limit`. Index construction
requires `kind == Inverted` and a `Text` field type (`index_for`).

---

## 11. On-disk magic numbers, versions, and sizes (summary)

| Where | Constant | Value |
|---|---|---|
| WAL record | `MAGIC` | `b"CXW1"` (LE u32) |
| WAL record | header / max payload | 20 bytes / 64 MiB |
| WAL segment | default max bytes | 64 MiB |
| WAL group commit | window | 2 ms (hard cap) |
| SST | `MAGIC` | `b"CXS1"` |
| SST | `VERSION` / `LEGACY_VERSION` | 2 / 1 |
| SST | header / record-header / index-entry | 32 / 12 / 12 bytes |
| Bloom | hash_count | 3 (BLAKE3-keyed) |
| Manifest | version | major 1, minor 0 |
| Constellation header | `HEADER_LEN` | 102 bytes |
| MVCC tombstone | sentinel | `b"\0CALYX_ASTER_TOMBSTONE_V1"` |
| Keyspace | vault prefix | 16 bytes (full ULID) |
| Keyspace slot tag | `SLOT_KEYSPACE_TAG` | `0xF0` |
| Btree index | discriminant / max indexed bytes | `0x10` / 64 |
| Inverted index | discriminant / stats term hash | `0x11` / `u64::MAX` |
| Memtable | `ENTRY_OVERHEAD_BYTES` / default high-water | 4 / 80% of cap |
| Router | default memtable cap | 8 MiB |
| Compaction | default target / write-amp scale | 64 MiB / 1000 |

All multi-byte WAL/SST/bloom framing integers are **little-endian**; composite CF
keys and the constellation header/value encodings are **big-endian**.

---

## 12. Error handling and fail-closed posture

Aster maps storage faults to typed `CalyxError` codes (defined in
`crates/calyx-core/src/error.rs`):

| Code | Raised by (examples) |
|---|---|
| `CALYX_ASTER_TORN_WAL` | `TornTail::error` for a truncated/corrupt WAL tail. |
| `CALYX_ASTER_CORRUPT_SHARD` | SST magic/version/CRC/offset failures, non-canonical file/CF names, manifest decode/hash mismatch, unknown CF/modality/slot tags, seq divergence, identity collisions. |
| `CALYX_DISK_PRESSURE` | I/O errors, oversized group-commit window, disk-pressure guard, batcher channel closure. |
| `CALYX_BACKPRESSURE` | Memtable byte-cap rejection, admission failure, `set_start_seq` after allocation, poisoned locks. |
| `CALYX_STALE_DERIVED` | Derived-structure freshness violations; missing constellation at snapshot. |
| `CALYX_READER_LEASE_EXPIRED` | Reads against an expired snapshot lease. |
| `CALYX_VAULT_KEYSPACE_MISMATCH` | A CF key lacking this vault's keyspace prefix. |
| `CALYX_VAULT_ACCESS_DENIED` | A `put` for a constellation of another vault. |
| `CALYX_INVALID_ARGUMENT` | NULL/typed-mismatch index inputs; btree query on unsupported collection mode. |

The recurring design rule across `storage_names.rs`, `wal/segment.rs`, `sst`, and
`manifest`: a parse/CRC/name failure is **never** a silent skip — it is a typed
error, because silently excluding a file from replay/scan would drop committed
writes.

---

## 13. What is NOT covered / known gaps (from source)

- **Single-level LSM.** `SstLevel` is a flat newest-first file list; there is no
  multi-level (L0..Ln) leveled-compaction structure. The `SstShard.level`/
  compaction-output `level = max+1` field tracks generation count, not an
  on-disk level layout. Point/range reads scan every file in the family.
- **No block/value compression in the SST writer.** `write_record` stores raw
  key/value bytes; the only space-saving structures are the bloom filter and the
  CF-level dedup/quantization (the `dedup`/quantization modules, not the SST
  format itself).
- **Compaction does not yet honor MVCC version retention or tombstone GC inside
  the merge.** `compact_shards` collapses each key to one value via a `BTreeMap`;
  `reclaimed_input_files` is hardcoded `0` and old input files are not deleted by
  `compact_cf` (the swap drops them from the active set only).
- **Scheduler cadence is fixed** with an explicit `FIXME(PH46)` to replace it
  with an Anneal adaptive hook.
- **LEGACY SST version 1** is still accepted on read but never written; v1 skips
  body-CRC verification.
- **Directory fsync is Unix-only.** `sync_parent` is a no-op on non-Unix targets
  (the `#[cfg(not(unix))]` arms), so directory-entry durability after rename is
  not guaranteed on Windows.
- Many sibling modules in the crate (`dedup`, `erase`, `gc`, `redaction`,
  `retention`, `recurrence`, `supply_chain`, `olap`, `plain_graph`,
  `plain_column`, `stream`, `txn`, `layers/*`, `security`, `residency`) are part
  of the same crate but outside this document's storage-engine scope; they are
  referenced only where the core engine calls into them.
