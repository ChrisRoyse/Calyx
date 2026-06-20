# 04 — Storage & Schema

Calyx's primary store is the custom column-family LSM in `calyx-aster`, NOT a SQL
database. There is no SQL engine anywhere in the runtime data path. SQLite appears
in exactly one place — the **`calyx migrate` CLI tool** — and there it is opened
**read-only** to import rows from an external SQLite/sqlite-vec source into an
Aster vault. This document is the consolidated schema reference for both: §1–§5
cover the Aster column families, on-disk artifacts, key/value encodings, vault
directory layout, and storage tiers; §6 covers the import-only SQLite schema; §7
classifies sacred/regenerable/ephemeral storage.

On-disk magic/version constants are detailed in
[06_aster_storage_engine.md](06_aster_storage_engine.md); this doc summarizes them
in one table (§2) and adds the directory layout (§3) and tier classification (§7).

**Source files covered:**
- `crates/calyx-aster/src/cf/family.rs` — `ColumnFamily`, `SlotFamilyKind`, `STATIC`, `keyspace_tag`
- `crates/calyx-aster/src/cf/key.rs` — big-endian key codecs, `XTermKind`, `OnlineKeyKind`, `ScalarId`
- `crates/calyx-aster/src/cf/mod.rs`, `cf/router.rs` — re-exports, `CfRouter`
- `crates/calyx-aster/src/storage_names.rs` — canonical file/dir names
- `crates/calyx-aster/src/vault.rs`, `vault/keyspace.rs`, `vault/encode.rs` (per 06)
- `crates/calyx-aster/src/manifest/mod.rs` — `VaultManifest`, `ManifestVersion`
- `crates/calyx-lodestar/src/kernel_index.rs`, `kernel_health.rs` — kernel artifact format
- `crates/calyx-cli/src/migrate/mod.rs`, `migrate/reader.rs`, `migrate/manifest.rs` — SQLite import tool
- `crates/calyx-cli/Cargo.toml` — `rusqlite.workspace = true`
- Cross-ref: `docs/dbprdplans/24_MEMORY_GC_RELIABILITY.md`

---

## 1. Column families (`cf/family.rs`)

### 1.1 What the "CF id" is

`ColumnFamily` is a Rust enum, not a numeric-keyed table. Two distinct numbering
schemes exist in the code; do not conflate them:

1. **`STATIC` index** = the position of a non-slot CF in
   `ColumnFamily::STATIC` (an array of length **33**). This index IS the CF's
   on-disk **keyspace tag byte** in a vault-scoped key (`keyspace_tag()` returns
   `vec![index as u8]` for non-slot CFs). This is the reversible discriminant
   used by `KeyspaceGuard::encode_key` / `decode_key` (`vault/keyspace.rs`).
2. **PRD CF code** — the numbering in `docs/dbprdplans/04_ASTER_STORAGE_FORMAT.md`
   (e.g. "Ledger=2, Assay=6") is the *design-doc* numbering and does NOT match the
   `STATIC` index. The implemented code uses the `STATIC` index as the on-disk tag,
   and `ColumnFamily::name()` (a directory string) as the durable CF identity. CFs
   are routed by enum/directory name (`CfRouter`, `parse_cf_dir_name`), never by a
   stored numeric code other than the keyspace tag.

The keyspace tag is what physically prefixes user keys on disk:
`vault_prefix(16 B) ‖ cf_tag ‖ user_key`. Non-slot tag = 1 byte (`STATIC` index).
Slot tag = `0xF0 ‖ slot_id_be(2) ‖ kind_byte` (`Quantized=0`, `Raw=1`). `STATIC.len()`
(33) stays below `0xF0`, so a static index can never collide with the slot marker.

### 1.2 Complete CF table

Columns below: **Idx** = `STATIC` index = on-disk keyspace tag byte (decimal);
**Dir** = `name()` directory under `cf/`; key/value encodings from `cf/key.rs`,
`cf/family.rs` doc comments, and `layers/*` (per 06). Big-endian throughout for
lexicographic range ordering. `CxId` = 16-byte BLAKE3 prefix; full hash = 32 bytes.

| Idx | CF | Dir | Key encoding (byte layout) | Value encoding |
|----:|----|-----|---------------------------|----------------|
| 0 | `Base` | `base` | `CxId` (16 B) | `ConstellationHeader` (102 B) + body (`vault/encode.rs`; see 06 §3.7) |
| 1 | `Collections` | `collections` | `b"coll\0" ‖ collection_name` | collection metadata |
| 2 | `Relational` | `relational` | `0x01 ‖ collection_id(8) ‖ pk_len ‖ pk` | `be_u16 ROW_SCHEMA_VERSION ‖ bincode(Row)` |
| 3 | `XTerm` | `xterm` | `CxId(16) ‖ SlotId_a be_u16 ‖ SlotId_b be_u16 ‖ XTermKind(1)` | cross-term value |
| 4 | `TemporalXTerm` | `temporal_xterm` | `CxId_a(16) ‖ CxId_b(16)` | temporal cross-term value |
| 5 | `Scalars` | `scalars` | `ScalarId be_u32 ‖ CxId(16)` | `f64` (be bit pattern) |
| 6 | `Anchors` | `anchors` | `CxId(16) ‖ AnchorKind(be_u16 tag [‖ len+utf8 for Label])` | `AnchorValue ‖ source ‖ ts` |
| 7 | `Assay` | `assay` | `(panel_version, corpus_shard, subject)` | `AssayRow` |
| 8 | `Ledger` | `ledger` | `seq` (`be_u64`) | hash-chained provenance entry (see [14](14_ledger_provenance.md)) |
| 9 | `Recurrence` | `recurrence` | `CxId(16) ‖ OccurrenceId be_u64` | recurrence occurrence / summary |
| 10 | `Graph` | `graph` | plain-graph rows: nodes / typed edges / reverse index / CSR projection | per-row graph value |
| 11 | `Online` | `online` | `OnlineKeyKind(1) ‖ seq_or_id be_u64` | typed online/adaptation state |
| 12 | `Reactive` | `reactive` | reactive trigger key | durable trigger audit / fired row |
| 13 | `AnnealRollback` | `anneal_rollback` | anneal key | rollback snapshots + live artifact pointers |
| 14 | `AnnealHealth` | `anneal_health` | anneal key | component health snapshot |
| 15 | `AnnealChecksums` | `anneal_checksums` | anneal key | base-shard checksum + restore metadata |
| 16 | `AnnealMistakes` | `anneal_mistakes` | anneal key | online mistake-closure log |
| 17 | `AnnealReplay` | `anneal_replay` | anneal key | surprise-prioritized replay-buffer snapshot |
| 18 | `AnnealHeads` | `anneal_heads` | anneal key | online head params + Fisher diagonals |
| 19 | `AnnealBandit` | `anneal_bandit` | anneal key | per-shape autotune bandit state |
| 20 | `AnnealSoak` | `anneal_soak` | anneal key | long-run soak metric samples + reports |
| 21 | `AnnealReport` | `anneal_report` | anneal key | intelligence report snapshots |
| 22 | `AnnealGrowth` | `anneal_growth` | anneal key | J-over-time growth-curve samples |
| 23 | `TimeIndex` | `time_index` | `be_u64(millis_utc) ‖ be_u64(seqno)` | 1-byte sentinel |
| 24 | `Document` | `document` | `0x02 ‖ collection_id(8) ‖ doc_id(16) ‖ path_segments` | leaf / branch cell / tombstone |
| 25 | `Kv` | `kv` | `0x03 ‖ collection_id(8) ‖ ns(8) ‖ key_len ‖ user_key` | `0x01 ‖ be_u64 expires_at_ms ‖ payload` (TTL on read) |
| 26 | `TimeSeries` | `timeseries` | `0x04 ‖ kind(0x00 point/0x01 rollup) ‖ collection_id(8) ‖ series(8) ‖ be ts/window` | point `be f64`; rollup `count(8) ‖ sum ‖ min ‖ max (f64)` |
| 27 | `Blob` | `blob` | chunk `0x05 ‖ 0x00 ‖ blob_id(16) ‖ be_u32 chunk_idx`; manifest `0x05 ‖ 0x01 ‖ blob_id(16)` | chunk bytes / manifest tuple |
| 28 | `IndexBtree` | `index_btree` | `0x10 ‖ collection_id(8) ‖ index_id(4) ‖ memcomparable(field_val) ‖ pk` | ∅ (existence is the signal) |
| 29 | `IndexInverted` | `index_inverted` | `0x11 ‖ collection_id(8) ‖ index_id(4) ‖ term_hash(8) ‖ pk` | `f32 be` (BM25 tf-weight); all-ones term_hash → `doc_count(8) ‖ avgdl(f32)` |
| 30 | `AnnealOperators` | `anneal_operators` | anneal key | learned-operator proposal records |
| 31 | `Kernel` | `kernel` | Lodestar grounding-kernel key | persisted kernel reports/indexes (see §2, [12](12_lodestar_kernel.md)) |
| 32 | `Guard` | `guard` | Ward subject key | Ward calibration profiles (see [13](13_ward_guard.md)) |

**Total static column families: 33** (indices 0–32). Note the `STATIC` array order
is NOT alphabetical and NOT the enum declaration order — `Document`, `Kv`,
`TimeSeries`, `Blob`, `IndexBtree`, `IndexInverted`, `AnnealOperators`, `Kernel`,
`Guard` were appended after the original 24, so their tag bytes are 24–32.
**Changing the array order would re-number every tag and break on-disk keys** —
the test `every_static_cf_dir_name_round_trips` guards the dir-name mapping.

Plus a **parameterized family of slot CFs** (not in `STATIC`), one per
`(SlotId, SlotFamilyKind)`:

| CF | Dir | Key | Value |
|----|-----|-----|-------|
| `Slot{slot, Quantized}` | `slot_NN` (zero-padded 2-digit, `{:02}`) | `CxId` (16 B) | quantized `SlotVector` (be bit patterns, see 06 §3.7) |
| `Slot{slot, Raw}` | `slot_NN.raw` | `CxId` (16 B) | raw f32 sidecar `SlotVector` |

Slot CF tag: `0xF0 ‖ slot_id_be(2) ‖ kind(0=Quantized,1=Raw)`.

### 1.3 Key-codec auxiliary enums (`cf/key.rs`)

| Enum / type | Variants → code | Notes |
|---|---|---|
| `XTermKind` | `Concat=0`, `Interaction=1`, `Agreement=2`, `Delta=3` | 1-byte tail of `xterm_key` |
| `OnlineKeyKind` | `MistakeLog=0`, `ReplayBuffer=1`, `HeadState=2`, `DeltaJQueue=3` | 1-byte head of `online_key` |
| `ScalarId(u32)` | — | `be_u32` head of `scalar_key` |
| `AnchorKind` tag (`be_u16`) | `TestPass=0`, `TieFormed=1`, `Thumbs=2`, `Reward=3`, `SpeakerMatch=4`, `StyleHold=5`, `Recurrence=6`, `Label=7` | `Label` adds `be_u64(len) ‖ utf8` |
| `KeyRange{start, end:Option}` | — | `contains`: `start ≤ key < end`; `end=None` = unbounded |

Content addressing: `full_content_hash(parts)` = BLAKE3 over length-delimited
parts (each part prefixed with `be_u64(len)`); `cx_id_from_full_hash` takes the
first 16 bytes; `verify_cx_hash_prefix` fails closed (`CALYX_ASTER_CORRUPT_SHARD`)
on prefix mismatch. `CX_ID_BYTES = 16`, `FULL_HASH_BYTES = 32`.

---

## 2. On-disk artifacts — consolidated format table

Every persistent artifact Calyx writes, its magic/version, and where it lives
under the vault dir. Detailed byte layouts are in
[06_aster_storage_engine.md](06_aster_storage_engine.md) (§2–§5); the kernel
artifacts are in [12_lodestar_kernel.md](12_lodestar_kernel.md). All Aster binary
artifacts use little-endian header fields; **logical keys** inside SSTs/WAL
payloads are big-endian (§1).

| Artifact | Magic / version constant | Format | Path under vault dir | Owner / source |
|---|---|---|---|---|
| WAL record | `MAGIC = "CXW1"` (LE u32); `HEADER_LEN=20`, `MAX_RECORD_BYTES=64 MiB` | binary, crc32fast per record | `wal/{index:020}.wal` | `wal/record.rs`, `wal/segment.rs` |
| SSTable | `MAGIC = b"CXS1"`; `VERSION=2` (`LEGACY_VERSION=1` readable); `HEADER_LEN=32` | binary, body crc32 (v2) + per-record crc + Bloom | `cf/<family>/{seq:020}.sst`, `…-{index:04}.sst`, `compacted-{seq:020}.sst` | `sst/mod.rs` |
| Arrow column chunk | `MAGIC = b"CXA1"`; `VERSION=1`; `HEADER_LEN=16` | binary, column-major f32 LE | written by columnar slot-scan path (`sst/arrow.rs`) | `sst/arrow.rs` |
| Vault manifest | `ManifestVersion{major=1, minor=0}` (`SUPPORTED_MANIFEST_MAJOR=1`) | **JSON** (`serde_json::to_vec_pretty`) | `CURRENT` (pointer), `MANIFEST` (mirror), `manifest-{seq:020}.json` | `manifest/mod.rs` |
| Tombstone marker | `TOMBSTONE_VALUE = b"\0CALYX_ASTER_TOMBSTONE_V1"` | value sentinel (in-SST/memtable) | inside any CF's SST/memtable as a row value | `mvcc/store.rs` |
| Lodestar kernel index | `FORMAT_VERSION: u32 = 1` (`kernel_index.rs`) | JSON snapshot (`KernelIndexSnapshot`) | `idx/kernel/<kernel_id>/index.json` (via `KernelStore`) | `calyx-lodestar/kernel_index.rs` |
| Lodestar kernel artifact | `KERNEL_ARTIFACT_FORMAT_VERSION: u32 = 1` (`kernel_health.rs`) | JSON snapshot (`KernelArtifactSnapshot`) | `idx/kernel/<kernel_id>/kernel.json` (sibling of `index.json`) | `calyx-lodestar/kernel_health.rs` |
| Residency pin | — (struct sidecar) | JSON sidecar | `residency.json` (vault root) | `aster/residency.rs` |
| Immutable refs | content-addressed (BLAKE3 hex, 64 chars) | bytes, verified on manifest load | `panel/…`, `registry/…`, `codebooks/…` (manifest-referenced) | `manifest/mod.rs` `ImmutableRef` |

**Format-version gate.** The only enforced refusal-on-unknown-major gate is
`ManifestVersion::validate()` → `CALYX_FORMAT_VERSION_UNSUPPORTED` (manifest), and
the kernel artifacts' `format_version != 1` → `LodestarError::KernelIndexCodec`.
SSTables accept `VERSION 1` and `2`; WAL/Arrow check magic+exact version. There is
no global "schema version" number for the Aster store other than these per-format
constants.

Canonical SST/WAL name classes (`storage_names.rs`, fail-closed authority):
`Router{seq}` = `{seq:020}.sst`; `DurableBatch{seq,index}` = `{seq:020}-{index:04}.sst`;
`Compacted{seq}` = `compacted-{seq:020}.sst`; WAL = `{index:020}.wal`. Any `*.sst`
/ `*.wal` name not matching → `CALYX_ASTER_CORRUPT_SHARD` (never silently skipped).

---

## 3. Vault directory layout on disk

Path construction is rooted at one `vault_dir` (the `<vault>.calyx` directory the
CLI/daemon passes to `AsterVault::open`). Confirmed from `cf/router.rs`
(`vault_dir.join("cf").join(cf.name())`), `manifest/mod.rs` (`CURRENT`/`MANIFEST`/
`manifest-*.json`), `wal` replay paths (`vault_dir.join("wal")`), `residency.rs`,
`migrate/manifest.rs`, the lodestar `KernelStore` (`idx/kernel/...`), and the
sextant `idx/` paths (`vault.join("idx")...`, owned by `calyx-sextant`).

| Path (relative to vault dir) | Contents | Owning subsystem |
|---|---|---|
| `cf/` | root of all column-family directories | `calyx-aster` `CfRouter` |
| `cf/<family>/` | one dir per CF (`base/`, `ledger/`, `slot_00/`, `slot_00.raw/`, `anneal_*/`, `index_btree/`, …); holds `*.sst` files | `calyx-aster` |
| `cf/<family>/{seq:020}.sst` | router memtable-flush SSTable | `sst` / `CfRouter::flush_cf` |
| `cf/<family>/{seq:020}-{index:04}.sst` | durable group-commit batch SSTable | `vault/durable.rs` |
| `cf/<family>/compacted-{seq:020}.sst` | compaction output SSTable | `compaction/` |
| `wal/` | write-ahead log directory | `calyx-aster` `wal/` |
| `wal/{index:020}.wal` | WAL segment (contiguous indices, fail-closed on gap) | `wal/segment.rs` |
| `wal/.append.lock` | exclusive append file-lock | `file_lock.rs` / `wal/mod.rs` |
| `CURRENT` | text pointer → current `manifest-*.json` | `manifest/mod.rs` |
| `MANIFEST` | mirror copy of current manifest bytes | `manifest/mod.rs` |
| `manifest-{seq:020}.json` | immutable versioned manifest snapshots | `manifest/mod.rs` |
| `*.tmp` | atomic-write temp files (renamed into place) | `manifest`, `sst` |
| `panel/` | manifest-referenced immutable panel artifacts | manifest `panel_ref` |
| `registry/` | manifest-referenced immutable registry artifact | manifest `registry_ref` |
| `codebooks/` | manifest-referenced immutable codebook artifacts | manifest `codebook_refs` |
| `idx/kernel/<kernel_id>/index.json` | Lodestar kernel index snapshot (`FORMAT_VERSION=1`) | `calyx-lodestar` |
| `idx/kernel/<kernel_id>/kernel.json` | Lodestar kernel artifact snapshot | `calyx-lodestar` |
| `idx/...` (e.g. `idx/graph.cda`, `idx/slot_00.sparse`) | ANN / sparse index artifacts | `calyx-sextant` (see [09](09_sextant_search.md)) |
| `residency.json` | data-residency pin sidecar | `aster/residency.rs` |
| `migration-manifest.json` | SQLite-migration sidecar (see §6) | `calyx-cli/migrate` |
| `migration-panel.json` | migration panel template sidecar | `calyx-cli/migrate` |
| `migration-backfill-scheduler.json` | migration backfill scheduler state | `calyx-cli/migrate` |

When hot/cold tiering is configured (`compaction/tiering.rs`), CF dirs are split
across a `hot_root` and an `archive_root`, each with its own `cf/` subtree (default
roots `/zfs/hot/calyx` and `/zfs/archive/calyx`; see 06 §6.3). Cold-only artifacts
are raw slot CFs and retired-panel quantized slot CFs.

---

## 4. Value-encoding summary

The constellation `base` value, slot-vector encoding, and layer values are fully
specified in [06 §3.7 and §8](06_aster_storage_engine.md). Key facts for schema
consumers:

- **`base` value** = 102-byte `ConstellationHeader` + body (identity hash, input
  ref, slot list, scalars, anchors, provenance hash, string metadata).
- **Slot values** store floats as **big-endian IEEE-754 bit patterns**
  (`f32::to_bits`/`f64::to_bits`), NOT the LE column-major Arrow `CXA1` layout.
  "Quantized" vs "Raw" slot CFs differ only by directory + tier policy in this
  crate — there is no quantization codec in `calyx-aster` (06 §11 gap).
- **Layer values** carry per-layer version prefixes: Relational `be_u16
  ROW_SCHEMA_VERSION ‖ bincode`; KV `0x01 ‖ be_u64 expires_at_ms ‖ payload`.

---

## 5. MVCC / sequence model (schema-relevant)

Every CF shares one vault-wide monotonic `seq` (`SeqAllocator`). A row is a
version `(seq, value)` in an append-only chain; a reader pinned at seq `S` sees the
latest version ≤ `S` per key across all CFs (snapshot isolation). Deletes are
tombstone rows (§2). The manifest's `durable_seq` marks the highest seq
checkpointed into SSTs; WAL records with `seq > durable_seq` are replayed on
recovery. See [06 §4–§5](06_aster_storage_engine.md).

---

## 6. SQLite — import-only migration tool (`calyx-cli` `migrate`)

### 6.1 SQLite is NOT Calyx's store

**SQLite is read-only import scaffolding, not part of Calyx's own storage.** The
runtime store is the Aster LSM (§1–§5). The single `rusqlite` dependency
(`calyx-cli/Cargo.toml`: `rusqlite.workspace = true`) is used only by the
`calyx migrate` command (`crates/calyx-cli/src/migrate/`), which reads an external
SQLite database and writes constellations into a brand-new Aster vault. Connections
are opened with `OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`
(`reader.rs::open_sqlite`) — Calyx **never creates, writes, or migrates** a SQLite
schema; it issues no `CREATE TABLE`/`INSERT`/`UPDATE`. There are no PRAGMAs other
than read-only schema introspection (`PRAGMA table_info(<table>)`).

The workspace `rusqlite` feature set determines whether SQLite is statically
bundled (the `bundled` feature in the workspace `Cargo.toml`); the migrate code is
agnostic to it. The only other `rusqlite` references in the tree are in
`calyx-cli/src/leapable/` shadow-harness test/verifier code that likewise reads
external Leapable SQLite sources for round-trip verification.

### 6.2 Recognized source schemas (read, not created)

`reader.rs::source_schema` sniffs two external layouts via `PRAGMA table_info`:

**(a) `CalyxFixture`** — a `chunks` table with these columns (read by
`SELECT rowid, chunk_id, database_name, content, embedding FROM chunks`):

| Column | Type (as read) | Notes |
|---|---|---|
| `rowid` | INTEGER | SQLite implicit rowid; → `ChunkRow.row_num` (must be ≥ 0) |
| `chunk_id` | TEXT | UTF-8 validated |
| `database_name` | TEXT | UTF-8 validated |
| `content` | BLOB/TEXT | raw bytes; BLAKE3 → content hash / CxId |
| `embedding` | BLOB | `768 × f32` little-endian = `3072` bytes exactly (`GTE_EMBEDDING_DIM=768`); NaN/Inf rejected |

**(b) `LeapableVec`** — a sqlite-vec layout joined across four tables. Required
columns (validated by `has_leapable_vector_tables`):

| Table | Required columns | Role |
|---|---|---|
| `chunks` | `id`, `text` | chunk identity + content (`c.text` → content bytes) |
| `database_metadata` | `database_name` | source DB name (one row, `ORDER BY id LIMIT 1`) |
| `embeddings` | `id`, `chunk_id` | maps chunk → embedding row |
| `vec_embeddings_rowids` | `id`, `chunk_id`, `chunk_offset` | offset into the packed vector blob |
| `vec_embeddings_vector_chunks00` | `vectors` | packed `f32` blob; slice `[offset*3072 .. +3072]` |

Join (read query in `stream_leapable_rows`):
`chunks c JOIN embeddings e ON e.chunk_id=c.id JOIN vec_embeddings_rowids r ON r.id=e.id JOIN vec_embeddings_vector_chunks00 vc ON vc.rowid=r.chunk_id`.
`validate_leapable_source` fails closed if `count(chunks) != count(joined vectors)`
or any `chunk_offset` is NULL/negative/out of the backing blob.

If neither matches, the tool reports which `CalyxFixture` column is missing
(`errors::schema`). No schema is ever written back.

### 6.3 Migration sidecar (`migrate/manifest.rs`) — JSON, written to the vault

The tool persists its own JSON sidecars in the target vault dir (NOT SQLite):

`MigrationManifest` (`migration-manifest.json`, `schema_version: u32 = 1`):

| Field | Type | Meaning |
|---|---|---|
| `schema_version` | u32 | sidecar schema version (currently `1`) |
| `vault_id` | String | ULID; derived from `content_address("calyx-ph64-vault-id-v1" ‖ seed)` |
| `vault_salt_hex` | String | hex of `content_address("calyx-ph64-vault-salt-v1" ‖ seed)` |
| `sqlite_path_digest` | String | hex BLAKE3 of source path |
| `panel_template` | String | default `"text-default"` |
| `panel_version` | u32 | default panel version |
| `base_slot_id` | u16 | default `0` |
| `base_lens_id` | String | GTE lens id (`--gte-lens-id`, default `default_base_lens_id()`) |
| `source_rows` / `migrated_rows` | usize | progress counters |
| `created_at_ms` | u64 | wall-clock ms |
| `scheduler_file` | String | `"migration-backfill-scheduler.json"` |

Sidecar files: `migration-manifest.json`, `migration-panel.json`,
`migration-backfill-scheduler.json` (all `vault_dir.join(...)`).

### 6.4 Migration steps (`migrate/mod.rs`)

Subcommands: `migrate vault | backfill | verify | status | readback`.
`migrate vault <sqlite.db> <vault.calyx>` flow:

1. `open_sqlite` (read-only) → `row_count` (`SELECT COUNT(*) FROM chunks`).
2. `stream_rows` reads all `ChunkRow`s (fixture or Leapable layout); decode +
   validate each 768-d f32 embedding.
3. `MigrationManifest::load_or_create` (idempotent — reuses an existing manifest).
4. `ensure_unique_cx_ids` — fail closed if two rows map to the same content
   `CxId` (`errors::schema`).
5. `--dry-run`: build a `Constellation` per row, write nothing, report counts.
6. Else `open_vault` (`AsterVault::new_durable`, `VaultOptions::default`), then per
   `--batch-size` (default 100) batch: `row_exists_and_matches` → skip duplicate,
   else `vault.put(adapter.constellation(row))`. `vault.flush()` at the end.
7. Optional `--backfill-default-panel` (`backfill_default_panel`,
   `BackfillMode::RealTei` or `--offline-backfill` → `OfflineDeterministic`).
8. Optional `--verify` (`verify_migration`; byte-exact content readback; any
   mismatch → `CALYX_ASTER_CORRUPT_SHARD`).
9. Write updated manifest; emit `MigrateVaultReport` JSON.

`verify`/`status`/`readback`/`backfill` re-open the existing vault + manifest and
read back; none writes SQLite.

---

## 7. Storage-tier classification (sacred / regenerable / ephemeral)

Derived from `compaction/tiering.rs` (hot/cold), `manifest/mod.rs`
(`degraded_rebuildable`), the GC table in
`docs/dbprdplans/24_MEMORY_GC_RELIABILITY.md §3`, and `06 §6.3`/`§7`. This is a
*durability/recoverability* classification (distinct from hot/cold placement).

| Tier | Meaning | Artifacts | Evidence |
|---|---|---|---|
| **Sacred** (never regenerable; loss = data loss) | source of truth; must survive crash; archived never deleted | WAL (`wal/*.wal`), `Ledger` CF (append-only hash chain, archived never purged), `Base` CF + slot CFs (the constellations), manifest (`CURRENT`/`MANIFEST`/`manifest-*.json`) + immutable refs (`panel/`, `registry/`, `codebooks/`), `Anchors` CF | WAL = un-checkpointed source of truth (06 §2.4); Ledger "archival, never deleted" (24 §3); manifest verifies immutable-ref hashes on load |
| **Regenerable** (rebuildable from sacred data; `degraded` flag + background rebuild) | derived indexes/caches that can be recomputed | `Kernel`/`Guard` CFs + `idx/kernel/*.json` (Lodestar/Ward, rebuildable), `IndexBtree`/`IndexInverted` (secondary indexes, rebuilt from base — `index/rebuild/*`), ANN/sparse `idx/*` (sextant), `Assay` cache, `XTerm`/`TemporalXTerm` (lazily materialized cross-terms), compacted SSTs, raw slot sidecars (`slot_NN.raw`, re-derivable) | 24 §7 hazard 12 "ANN/kernel index corruption → rebuildable from base; `degraded` flag + background rebuild"; manifest `degraded_rebuildable` flag |
| **Ephemeral** (in-memory; lost on restart by design; not on the durable critical path) | bounded in-RAM working state | `BoundedMemtable` rows (flushed to SST or replayed from WAL), LRU/TTL caches (lazy cross-terms, query plans, autotune configs, kernel results), arena/slab transient buffers, reader-lease registry, VRAM-resident blocks | 24 §1 (bounded memtables, bounded caches, arenas — "lost"/reclaimed; never the durable record) |

Anti-storm and reclaim policy (24 §3): tombstones/overwritten keys → compaction;
old MVCC versions → snapshot GC (reader-lease watchdog); WAL → recycler once durable;
retired-lens columns and old panel/codebook versions → version GC (cold-tier first,
keep 2 hot); orphan slots/indexes → reconciler. The Ledger is the one CF explicitly
exempt from deletion (audit). See [06 §7](06_aster_storage_engine.md) for the six
GC subsystems and their constants.

---

## Gaps / not covered

- **No SQL store of Calyx's own.** SQLite is read-only import scaffolding for
  `calyx migrate` only; Calyx writes no SQL. (Stated explicitly per the assignment.)
- **PRD CF codes vs `STATIC` indices diverge.** The design-doc numbering (Ledger=2,
  Assay=6) is not the implemented on-disk tag. This doc uses the code's `STATIC`
  index (Ledger=8, Assay=7) as the authoritative on-disk keyspace-tag byte.
- **No global Aster schema-version integer.** Format versioning is per-artifact
  (manifest `1.0`, SST `v2`, WAL/Arrow `1`, kernel `1`); only the manifest major
  and kernel `format_version` are refusal-on-unknown gates.
- **Slot "quantized" vs "raw" is policy-only** in `calyx-aster` (directory + tier),
  not a distinct on-disk encoding (06 §11).
- ANN/DiskANN/SPANN on-disk index formats under `idx/` are owned by
  `calyx-sextant`, not detailed here — see [09_sextant_search.md](09_sextant_search.md).
- Value byte layouts for `XTerm`, `TemporalXTerm`, `Reactive`, and the `Anneal*`
  CFs were inferred from doc comments + key codecs; their exact value serializers
  live in the owning crates (anneal/loom/oracle) and were not byte-traced here.

See [06_aster_storage_engine.md](06_aster_storage_engine.md) (engine internals),
[12_lodestar_kernel.md](12_lodestar_kernel.md) (kernel artifacts),
[14_ledger_provenance.md](14_ledger_provenance.md) (ledger CF), and
[09_sextant_search.md](09_sextant_search.md) (ANN indexes).
