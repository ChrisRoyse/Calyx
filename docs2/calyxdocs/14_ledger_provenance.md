# 14 — Ledger: Hash-Chained Provenance, Checkpoints, and Verification

calyx-ledger is the append-only, hash-chained, tamper-evident provenance record. Every
trusted signal in Calyx (measurement, assay, kernel, guard verdict, answer, anneal,
migration, admin, erase) is recorded as one canonical `LedgerEntry`. Each entry seals the
previous via a BLAKE3 chain hash; periodic Admin "checkpoint" entries carry a Merkle root
over a sequence range, optionally signed with ed25519-dalek for export/attestation.

**Source files covered:**

- `crates/calyx-ledger/src/lib.rs`
- `crates/calyx-ledger/src/entry.rs`
- `crates/calyx-ledger/src/kind.rs`
- `crates/calyx-ledger/src/codec.rs`
- `crates/calyx-ledger/src/append.rs` (+ `src/append/tests.rs`)
- `crates/calyx-ledger/src/checkpoint.rs`
- `crates/calyx-ledger/src/merkle.rs`
- `crates/calyx-ledger/src/verify.rs`
- `crates/calyx-ledger/src/group_commit.rs`
- `crates/calyx-ledger/src/redaction.rs`
- `crates/calyx-ledger/src/audit.rs` (+ `src/audit/mentions.rs`)
- `crates/calyx-ledger/src/reproduce.rs` (+ `src/reproduce/fusion.rs`)
- `crates/calyx-ledger/src/tombstone.rs` (+ `src/tombstone/wire.rs`)
- `crates/calyx-ledger/Cargo.toml`
- Cross-ref: `crates/calyx-aster/src/cf/key.rs` (`ledger_key`), `crates/calyx-core/src/error.rs`, `crates/calyx-core/src/model/signal.rs` (`LedgerRef`)

Cross-references: see [04_storage_and_schema.md](04_storage_and_schema.md) and
[06_aster_storage_engine.md](06_aster_storage_engine.md) for the `ledger` column family,
[05_core.md](05_core.md) for `LedgerRef`/error taxonomy, [12_lodestar_kernel.md](12_lodestar_kernel.md)
and [13_ward_guard.md](13_ward_guard.md) for the Kernel/Guard entries this records.

Dependencies (`Cargo.toml`): `bincode`, `blake3`, `calyx-core`, `ed25519-dalek`, `serde`,
`serde_json`, `ulid`; dev: `proptest`.

---

## 1. The ledger entry

### 1.1 `LedgerEntry` (`entry.rs`)

Canonical append-only entry. `#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]`.

| Field | Type | Meaning |
|---|---|---|
| `seq` | `u64` | Monotonic per-ledger sequence number, starts at 0, no gaps. |
| `prev_hash` | `[u8; 32]` | `entry_hash` of seq−1; `[0;32]` for seq 0 (genesis). |
| `kind` | `EntryKind` | Event kind (§1.3). |
| `subject` | `SubjectId` | Tagged primary object (§1.2). |
| `payload` | `Vec<u8>` | Evidence row (usually JSON; tombstones use a binary wire form). |
| `actor` | `ActorId` | Who/what caused the entry (§1.2). |
| `ts` | `u64` | Server-stamped, strictly monotonic timestamp (§4.2). |
| `entry_hash` | `[u8; 32]` | Canonical BLAKE3 hash over all preceding fields (§2). |

`HASH_BYTES = 32`. `LedgerEntry::new(...)` computes `entry_hash` via `compute_entry_hash`.
`LedgerEntry::verify()` recomputes and compares `entry_hash` (returns `bool`).

### 1.2 Subject and actor identifiers

`SubjectId` enum (`entry.rs`), wire tag in parens:

| Variant | Payload | `wire_tag` | `wire_bytes` |
|---|---|---|---|
| `Cx(CxId)` | 16-byte id | `0` (`TAG_CX`) | `id.as_bytes()` (16) |
| `Lens(LensId)` | 16-byte id | `1` (`TAG_LENS`) | `id.as_bytes()` (16) |
| `Kernel(Vec<u8>)` | opaque | `2` (`TAG_KERNEL`) | raw bytes |
| `Guard(Vec<u8>)` | opaque | `3` (`TAG_GUARD`) | raw bytes |
| `Query(Vec<u8>)` | opaque (answer/query/checkpoint id) | `4` (`TAG_QUERY`) | raw bytes |

`ActorId` enum:

| Variant | `wire_tag` | `wire_bytes` |
|---|---|---|
| `Agent(String)` | `0` (`TAG_AGENT`) | UTF-8 of the string |
| `Service(String)` | `1` (`TAG_SERVICE`) | UTF-8 of the string |
| `System` | `2` (`TAG_SYSTEM`) | empty (`&[]`) |

`ActorId::validate()` returns `CALYX_LEDGER_ACTOR_TOO_LONG` when `wire_bytes().len() >
MAX_ACTOR_ID_BYTES` (= 64).

### 1.3 `EntryKind` (`kind.rs`)

`#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]`. The one-byte
`wire_code()` is stable and is the byte hashed/encoded for the kind. `EntryKind::ALL` lists
all 10 in wire-code order. `from_wire_code(u8) -> Option<Self>`; `as_str()` gives the lowercase
label; `Display` uses `as_str`.

| Variant | `wire_code` | `as_str` | What it records (per plan §1) |
|---|---|---|---|
| `Ingest` | 0 | `ingest` | Input ingested into a vault. |
| `Measure` | 1 | `measure` | Vector measurement: `(cx_id, slot_id, lens_id, weights_sha256, input_hash, ts)`. |
| `Assay` | 2 | `assay` | Signal-bit estimate: `(slot, anchor, bits, ci, estimator, corpus_shard_hash, ts)`. |
| `Kernel` | 3 | `kernel` | Grounding kernel snapshot. |
| `Guard` | 4 | `guard` | Guard pass/fail verdict (`per_slot_cos, tau, pass, ts`). |
| `Answer` | 5 | `answer` | Answer path + fusion weights + guard/freshness refs. |
| `Anneal` | 6 | `anneal` | Self-tuning step. |
| `Migrate` | 7 | `migrate` | Schema/data migration. |
| `Admin` | 8 | `admin` | Admin events; also used for **checkpoint** rows and **reproduce** result rows. |
| `Erase` | 9 | `erase` | Erasure tombstone (§7). |

The payload schema per kind is **not** type-enforced by this crate (payload is `Vec<u8>`);
schemas are conventions read back by `audit`/`reproduce` from JSON keys. Document treats those
keys as conventions, not guarantees.

---

## 2. The hash chain (exact preimage)

### 2.1 Entry-hash preimage (`compute_entry_hash`, `entry.rs`)

The hash is BLAKE3 over **length-framed** fields, in fixed order. Each field is fed through
`frame(hasher, bytes)`, which does:

```
hasher.update( (bytes.len() as u64).to_be_bytes() )   // 8-byte big-endian length prefix
hasher.update( bytes )                                  // then the bytes
```

The framed fields, in exact order:

```
entry_hash = BLAKE3(
    frame( seq.to_be_bytes() )            // 8 bytes
    frame( prev_hash )                    // 32 bytes
    frame( [kind.wire_code()] )           // 1 byte
    frame( subject.canonical_bytes() )    // tag byte ++ subject.wire_bytes()
    frame( payload )                      // raw payload bytes
    frame( actor.canonical_bytes() )      // tag ++ u64-BE len ++ actor.wire_bytes()
    frame( ts.to_be_bytes() )             // 8 bytes
)
```

`subject.canonical_bytes()` = `tagged_slice(tag, wire_bytes)` = `[tag] ++ wire_bytes` (NO inner
length — the outer `frame` length covers it).

`actor.canonical_bytes()` = `tagged_var(tag, wire_bytes)` = `[tag] ++ (len as u64 BE) ++
wire_bytes` (an inner length **is** present, distinguishing it from the subject framing).

Length-framing prevents field-boundary ambiguity: changing any single field changes the hash
(proptest `changing_each_field_changes_hash`). The golden vector `entry_hash_golden`
(`entry.rs` tests) pins the formula: entry `(seq=1, prev=[0;32], Ingest,
Cx([1;16]), payload="test", Service("svc"), ts=1_785_000_000)` →
`21f5ff34d085ba094e9e831a734fc4bbfd7d8ecaab1138a805a96bc46c17ae88`.

### 2.2 How each entry seals the previous

`prev_hash` is the previous entry's `entry_hash` and is itself framed into the new entry's
hash. So `entry_hash[n]` transitively binds every prior entry. Genesis (`seq 0`) uses
`prev_hash = [0; 32]`. Any retroactive edit to entry *k* changes `entry_hash[k]`, which
breaks the `prev_hash` link at *k+1* and is detected by verification (§5).

---

## 3. Binary codec (`codec.rs`)

`encode(&LedgerEntry) -> Vec<u8>` — deterministic, padding-free layout (offsets in bytes):

| Offset | Field | Width |
|---|---|---|
| 0 | `seq` (BE) | 8 |
| 8 | `prev_hash` | 32 |
| 40 | `kind.wire_code()` | 1 |
| 41 | `subject.wire_tag()` | 1 |
| 42 | subject len (`u16` BE) | 2 |
| 44 | subject `wire_bytes` | var |
| … | payload len (`u32` BE) | 4 |
| … | payload bytes | var |
| … | `actor.wire_tag()` | 1 |
| … | actor len (`u16` BE) | 2 |
| … | actor `wire_bytes` | var |
| … | `ts` (BE) | 8 |
| … | `entry_hash` | 32 |

`encode` asserts subject ≤ `u16::MAX`, actor ≤ `u16::MAX`, payload ≤ `u32::MAX`.
`HEADER_LEN = 40` (seq+prev_hash). Golden: `codec_golden` pins `CODEC_GOLDEN_HEX`.

- `decode(&[u8]) -> Result<LedgerEntry>` decodes and calls `verify()`; hash mismatch →
  `CALYX_LEDGER_CORRUPT`. Trailing bytes, truncation, bad tags, non-UTF8 actor, non-16-byte
  Cx/Lens, and `System` actor with non-empty bytes all fail closed as corrupt.
- `decode_unchecked` decodes without re-verifying the embedded hash (used by `verify.rs` so it
  can recompute and report the mismatch itself).
- `decode_header(&[u8]) -> Result<(u64, [u8;32])>` reads only seq+prev_hash for fast link
  checks (needs ≥ `HEADER_LEN` bytes).

---

## 4. Append path and durability (`append.rs`)

### 4.1 `LedgerCfStore` trait

The minimal append-only column-family contract:

| Method | Behavior |
|---|---|
| `scan() -> Result<Vec<LedgerRow>>` | All rows sorted by seq. |
| `put_new(seq, &[u8]) -> Result<()>` | Insert new row; implementations MUST reject overwrite. |
| `delete(seq)` (default) | Calls `reject_delete` → `CALYX_LEDGER_APPEND_ONLY_VIOLATION`. |
| `tombstone(seq)` (default) | Calls `reject_tombstone` → same error. |

`LedgerRow { seq: u64, bytes: Vec<u8> }`. Implementations:

- `MemoryLedgerStore` — `BTreeMap<u64, Vec<u8>>`; `put_new` rejects existing seq with
  append-only violation. `insert_raw` is a test backdoor that bypasses the check.
- `DirectoryLedgerStore` — disk-backed; row file `{seq:016x}.ledger` under `root`. `put_new`
  uses `OpenOptions::create_new(true)` (refuses overwrite → append-only violation on
  `AlreadyExists`) then `write_all` + **`sync_all()`** (fsync) for durability. `scan` reads all
  `*.ledger` files, parses hex seq from the stem, sorts by seq. I/O errors map to
  `disk_pressure`. Comment marks it "manual FSV before Aster group-commit wiring."
- `OverlayLedgerStore` (`checkpoint.rs`) — read-only merge of a base store plus pending
  prepared rows, used to compute a checkpoint Merkle root including not-yet-committed rows;
  `put_new` always errors. Divergent bytes for the same seq → `CALYX_LEDGER_CORRUPT`.

### 4.2 `LedgerAppender<S, C>`

The single write path. Fields: `store: S`, `clock: C` (`calyx_core::Clock`), `next_seq`,
`prev_hash`, `last_ts`, `redaction_policy`.

- `open(store, clock)` / `open_with_policy(store, clock, policy)` — calls `recover_tip` to
  rebuild `(next_seq, prev_hash, last_ts)` by scanning all rows and validating: no seq gap
  (else `CALYX_LEDGER_CHAIN_BROKEN`), `decode` each (verifies hash), key seq == encoded seq
  (else `CALYX_LEDGER_CORRUPT`), and each `prev_hash` matches the prior `entry_hash`.
- `prepare(kind, subject, payload, actor) -> PreparedLedgerEntry` — builds the next row
  **without** mutating store or tip. Steps in `prepare_at`:
  1. `redaction_policy.check_payload_with_policy(&payload)` (rejects secrets, §6).
  2. `verify_tip()` — re-`recover_tip` and confirm it equals the in-memory tip; mismatch
     (concurrent writer / external mutation) → chain-broken.
  3. `actor.validate()`, then `redaction_policy.apply_to_actor(actor)`, then validate again.
  4. `ts = next_ts_after(last_ts)`: `clock.now()` if strictly greater than `last_ts`,
     else `last_ts + 1` (monotonic; overflow → chain-broken).
  5. `LedgerEntry::new(...)` (computes hash), `encode`.
- `prepare_after(predecessor, …)` — chains onto an uncommitted prepared row (seq+1,
  `prev_hash = predecessor.entry_hash()`); used to stage a checkpoint behind a data row.
- `commit_prepared(&prepared) -> Result<LedgerRef>` — rejects if `prepared.seq != next_seq`
  or `prev_hash != self.prev_hash` (chain-broken), then `store.put_new`, then advances
  `last_ts`, `next_seq += 1`, `prev_hash = entry_hash`. Returns `LedgerRef`.
- `append(...)` = `prepare` then `commit_prepared`.
- Accessors: `next_seq()`, `prev_hash()`, `last_ts()`, `scan_entries()` (decode all),
  `store()`, `store_mut()`, `into_store()`.

`PreparedLedgerEntry` holds `{ entry, bytes }` and exposes `seq()`, `entry_hash()`,
`prev_hash()`, `ts()`, `bytes()`, `ledger_ref() -> LedgerRef { seq, hash: entry_hash }`.

### 4.3 Group commit and Aster persistence (`group_commit.rs`)

In production the ledger row is written as part of Aster's group-commit so provenance lives in
the WAL with the data it describes (plan §6).

- `LedgerWriteBatch` trait: `put_ledger_row(key, value) -> Result<()>`. `WriteBatch` is the
  in-memory test impl exposing `ledger_rows() -> &[LedgerBatchRow { key, value }]`.
- `ledger_batch_key(seq) = seq.to_be_bytes().to_vec()` — **must match** Aster's
  `crates/calyx-aster/src/cf/key.rs::ledger_key(seq)` (= `seq.to_be_bytes()`), so ledger rows
  sort by big-endian seq in `ColumnFamily::Ledger` (CF code 2,
  `calyx-aster/src/vault/cf_codec.rs`).
- `WriteOp { Ingest, VaultAdmin, Erase }` and `ingest_kind_for(op)` map storage ops to
  `EntryKind::{Ingest, Admin, Erase}`.
- `DefaultLedgerHook<S, C>` wraps a `LedgerAppender` plus optional `CheckpointScheduler`.
  Constructors: `new`, `with_checkpoint_config` (recovers scheduler from store),
  `with_checkpoint_scheduler`.
  - `stage(...)` → one `StagedLedgerRow { key, value, ledger_ref, prepared,
    checkpoint_range_end }` (does not advance the tip).
  - `stage_with_checkpoints(...)` → the data row plus, if `scheduler.should_checkpoint`, a
    second Admin checkpoint `StagedLedgerRow` prepared over an `OverlayLedgerStore`.
  - `commit_staged(&row)` → `commit_prepared`, then `advance_after_checkpoint` if the staged
    row carried a `checkpoint_range_end`.
- **Fail-closed legacy gate** (issue #652): the crate-private `LedgerGroupCommitHook::on_commit`
  is disabled — it always returns `CALYX_LEDGER_GROUP_COMMIT_FAILED` and touches neither the
  batch nor the tip, forcing callers through `stage_with_checkpoints` + `commit_staged` after a
  durable storage commit. Tests confirm no row/file/tip mutation on the rejected path.

---

## 5. Verification (`verify.rs`)

`verify_chain(store: &dyn LedgerCfStore, range: Range<u64>) -> Result<VerifyResult>`.

`VerifyResult` enum:

| Variant | Fields | Meaning |
|---|---|---|
| `Intact` | `count: u64` | All entries in range link and hash correctly. |
| `Broken` | `at_seq, expected: [u8;32], found: [u8;32]` | Chain link or entry-hash mismatch. |
| `Corrupt` | `at_seq, reason: String` | Missing/undecodable row or key/seq mismatch. |

`VerifyResult::quarantine_seq()` returns `Some(at_seq)` for `Broken`/`Corrupt`, `None` for
`Intact`.

Algorithm:

1. Reject `range.start > range.end` (`CALYX_LEDGER_CORRUPT`); empty range → `Intact{count:0}`.
2. `scan()` all rows into a `BTreeMap<seq, bytes>`.
3. Compute `expected_prev` for the first seq (`expected_prev_hash`): if `start == 0`, use
   `[0;32]`; otherwise look up `start-1`, decode it (`decode_unchecked`), confirm its key seq,
   confirm `entry.verify()`; any failure → `Corrupt{at_seq:start, …}`.
4. For each `seq` in range, in order:
   - missing row → `Corrupt`;
   - `decode_unchecked` failure → `Corrupt`;
   - key seq ≠ encoded seq → `Corrupt`;
   - `entry.prev_hash != expected_prev` → `Broken{expected:expected_prev, found:prev_hash}`;
   - recomputed `compute_entry_hash` ≠ stored `entry_hash` → `Broken`;
   - else set `expected_prev = entry.entry_hash`, `count += 1`.
5. Return `Intact{count}`.

Note: `verify_chain` returns `Ok(VerifyResult::Broken/Corrupt)` for tamper findings (a normal
result, not an `Err`). On detection the policy (plan §7) is to quarantine the affected range;
the audit surface (§8) enforces this via `QuarantineSet`/`CALYX_LEDGER_CHAIN_BROKEN`.

---

## 6. Redaction and secret guardrails (`redaction.rs`)

`RedactionPolicy { store_raw_input: bool, redact_actor_name: bool }` (both `false` by default).

- `check_payload(&[u8])` / `check_payload_with_policy(&self, &[u8])` — empty payload OK.
  Parses payload as JSON; if not JSON, scans as text tokens. Rejects:
  - secret-like JSON keys: `password`, `passwd`, `token`, `secret`, `key`, or any key ending
    `_password`/`_passwd`/`_token`/`_secret`/`_key` (but **not** public-key fields
    `signer_pubkey`/`public_key`/`verifying_key`) → `CALYX_LEDGER_SECRET_IN_PAYLOAD`;
  - long high-entropy tokens (`SECRET_TOKEN_MIN = 40`, or any ≥40-char no-whitespace printable
    run) unless allowed for the field as a stable identifier.
  - Allowed stable-identifier fields (`field_allows_stable_identifier`): `hash`, `metadata`,
    `input_hash`, `root`, `signature` (128-hex), `weights_sha256`, the public-key fields
    (64-hex), source-metadata (`chunk_id`/`database_name`), `quant_slot_*` (≤4096-hex), or
    suffixes `_hash`/`_id`/`_sha256`/`_digest` (≤64 chars, hex/base58/uuid).
- `redact_input_ref(&InputRef) -> RedactedInput { hash, redacted: true, pointer: None }` —
  keeps the content hash, drops the raw pointer (lineage holds, content does not leak).
- `apply_to_payload(&PayloadBuilder)` — projects JSON keeping only allowed id/hash/`ts`/
  `redacted` fields; `input_ref` is reduced to `{hash, redacted:true}`; raw fields (`raw`,
  `*_raw`, `*_bytes`, `plaintext`, …) kept only if `store_raw_input`.
- `apply_to_actor(actor)` — when `redact_actor_name`, replaces `Agent`/`Service` names with
  `"redacted"`; `System` unchanged.

`PayloadBuilder` (JSON object builder): `object()`, `from_value`, `insert_value`,
`insert_str`, `insert_u64`, `value()`. `RedactedInput { hash:[u8;32], redacted:bool,
pointer:Option<String> }`.

---

## 7. Erasure tombstones (`tombstone.rs`, `tombstone/wire.rs`)

GDPR-style erase recorded as an `EntryKind::Erase` ledger entry (metadata only — carries
identifiers and counts, never erased content).

- `ErasureScope` enum: `Vault`, `Cx(CxId)`, `Subject(SubjectId)`.
- `ErasureTombstone { seq:u64, vault_id:VaultId, scope:ErasureScope, actor:ActorId,
  erased_at:Ts, records_deleted:usize }`.
- `ledger_subject()` maps scope to a `SubjectId`; `Vault` scope uses `SubjectId::Guard` of a
  BLAKE3 `scope_digest` (domain `b"calyx-ledger-erasure-tombstone-subject-v1"` ‖ vault id ‖
  scope tag ‖ id).
- Payload encoding: `as_ledger_payload()` uses the binary wire form (`wire.rs`):
  4-byte magic `b"ETB1"` ++ `bincode` (`config::standard()`) of `WireTombstone`. Decode accepts
  either the wire form or a legacy compact-JSON (`CompactTombstone`). Trailing bytes → corrupt.
- `write_tombstone(&tombstone, &mut appender)` — requires `appender.next_seq() == tombstone.seq`
  (else chain-broken) and appends an `Erase` entry.
- `tombstone_from_entry(&entry)` returns `None` for non-`Erase`; validates payload seq == entry
  seq and payload actor == entry actor (else corrupt). `find_tombstone` / `is_tombstoned`
  scan for a tombstone matching `(vault_id, scope)`.

Note: this is a ledger-row tombstone (a recorded erase event), distinct from LSM
delete-tombstones, which remain forbidden on the `ledger` CF (§4.1).

---

## 8. Audit and provenance query surface (`audit.rs`, `audit/mentions.rs`)

- `AuditFilter { kind:Option<EntryKind>, actor:Option<ActorId>, ts_range:Option<(u64,u64)>,
  seq_range:Option<(u64,u64)> }` (half-open ranges).
- `QuarantineLookup` trait: `contains_quarantined(range) -> Result<bool>`. `QuarantineSet`
  (`from_ranges`, rejects empty ranges) implements it by half-open overlap test.
- `audit(cf_reader, quarantine, filter)` — scans rows, applies filter, and **fails closed**
  with `CALYX_LEDGER_CHAIN_BROKEN` if a matching row (or an explicit `seq_range`) overlaps a
  quarantined range (#349). Decoders reject rows whose key seq ≠ encoded seq.
- `get_provenance(cf_reader, quarantine, cx_id) -> Vec<LedgerEntry>` — entries that mention the
  cx, via typed `SubjectId::Cx` or specific JSON payload fields only (`entry_mentions_cx` /
  `is_cx_payload_field`: `cx_id`, `from_id`, `to_id`, `source_cx_id`, `target_cx_id`,
  `nearest_cx`, `matched_cx_id`, `query_id`, `anchor_kernel_node_id`).
- `get_answer_trace(cf_reader, quarantine, answer_id) -> AnswerTrace`. `AnswerTrace` fields:
  `answer_entry`, `kernel_entry`, `guard_entry` (all `Option<LedgerEntry>`), `path:
  Vec<AnswerTraceHop>`, `fusion_weights: Option<FusionWeights>`, `guard_result: Option<Value>`,
  `freshness_ts: Option<u64>`, `complete: bool`, `warnings: Vec<CalyxWarning>`. `is_trusted()`
  = `complete && warnings.is_empty()`. Incomplete/unlinked traces add `unprovenanced` warnings
  rather than fabricating links. `AnswerTraceHop { cx_id, from_cx_id:Option, hop:u32, score:f32,
  lens_id:Option, ledger_seq:u64 }`.

---

## 9. Checkpoints (`checkpoint.rs`) and Merkle export (`merkle.rs`)

### 9.1 Cadence and scheduler

`DEFAULT_CHECKPOINT_INTERVAL = 1_000`. `CHECKPOINT_TAG = "checkpoint_v1"`.

`CheckpointConfig { interval_entries:u64, sign_key:Option<[u8;32]> }`. `new(interval)`,
`with_sign_key([u8;32])`, `Default` = interval 1000, no key. `validate()` rejects
`interval_entries == 0` (`CALYX_LEDGER_CORRUPT`).

`CheckpointScheduler { config, range_start, next_checkpoint_at }`:

- `new(config)` → `range_start = 0`, `next_checkpoint_at = interval_entries`.
- `recover(config, store)` — replays existing Admin rows whose payload is a `checkpoint_v1`
  `CheckpointPayload`, advancing the scheduler past each recorded range.
- `should_checkpoint(current_seq)` = `current_seq >= next_checkpoint_at && range_start <
  current_seq`.
- `prepare_checkpoint_after(appender, store, predecessor, range_end_seq)` — computes
  `merkle_root(store, range_start..range_end_seq)`, builds a `CheckpointPayload`
  (`from_root`), and prepares an `Admin` entry whose subject is
  `SubjectId::Query(b"checkpoint_v1")` and actor `System`.
- `advance_after_checkpoint(range_end_seq)` — `range_start = range_end_seq + 1`,
  `next_checkpoint_at = range_start + interval` (saturating).

Because the checkpoint row itself consumes a sequence number and `range_start` jumps to
`range_end + 1`, checkpoints with interval 5 over a continuous stream fire at seq 5, 11, 17,…
(verified by `checkpoint_tests.rs::scheduler_writes_periodic_admin_checkpoints`).

### 9.2 `CheckpointPayload`

Serde struct stored as JSON in the Admin entry payload:

| Field | Type | Notes |
|---|---|---|
| `tag` | `String` | Must equal `"checkpoint_v1"`. |
| `range_start` | `u64` | Inclusive start of the checkpointed range. |
| `range_end` | `u64` | Exclusive end. |
| `root` | `String` | Hex of the 32-byte Merkle root. |
| `signature` | `Option<String>` | Hex of the 64-byte ed25519 signature (skipped if `None`). |
| `signer_pubkey` | `Option<String>` | Hex of the 32-byte ed25519 verifying key (skipped if `None`). |

`from_root(range, root, sign_key)` builds a signed or unsigned `MerkleExportBundle` then hex-
encodes. `encode()` = `serde_json::to_vec`. `decode()` validates the tag, `range_start <=
range_end`, root hex length, and optional signature (64-byte)/pubkey (32-byte) hex lengths.
`decode_optional` returns `None` for non-checkpoint payloads (lets the scheduler skip non-
checkpoint Admin rows). `root_bytes()` parses the hex root to `[u8;32]`.

### 9.3 Merkle root (`merkle.rs`)

`MERKLE_EMPTY_ROOT = [0;32]`. Domain-separated BLAKE3:

```
leaf_hash(entry_hash)      = BLAKE3( b"leaf" ++ entry_hash )
combine_hash(left, right)  = BLAKE3( b"node" ++ left ++ right )
```

`merkle_root_of_hashes(&[[u8;32]])`: empty → `MERKLE_EMPTY_ROOT`; else map each entry hash
through `leaf_hash`, then repeatedly combine adjacent pairs; **odd levels duplicate the last
node** before pairing; terminates at a single root.

`merkle_root(store, range)`: rejects `start > end` (corrupt); empty range → `MERKLE_EMPTY_ROOT`;
collects in-range rows (duplicate seq → corrupt), requires every seq present (missing → corrupt),
decodes each (`decode`, verifies hash), confirms encoded seq, and feeds `entry_hash` values to
`merkle_root_of_hashes`.

### 9.4 Signing scheme (ed25519-dalek)

`MERKLE_SIGNING_DOMAIN = b"calyx-ledger-root-v1"`, `SIGNATURE_BYTES = 64`.

Signing message (`signing_message`):

```
message = MERKLE_SIGNING_DOMAIN
        ++ range.start.to_be_bytes()   // 8 BE
        ++ range.end.to_be_bytes()     // 8 BE
        ++ root                        // 32
```

- `sign_root(range, &root, &signing_key[32]) -> [u8;64]` — `SigningKey::from_bytes`
  (ed25519-dalek; the 32-byte key is the raw ed25519 secret seed) then `key.sign(message)`.
- `MerkleExportBundle { range_start:u64, range_end:u64, root:[u8;32],
  signature:Option<[u8;64]>, signer_pubkey:Option<[u8;32]> }`. `unsigned(range, root)` leaves
  both `None`; `signed(range, root, key)` sets `signature` and records the **verifying key**
  (`signing_key.verifying_key().to_bytes()`) as `signer_pubkey`. A custom serde module
  serializes the 64-byte signature as a byte slice and validates length on decode.
- `verify_signature(&bundle) -> bool` — false if signature or pubkey is `None` or the pubkey is
  invalid; otherwise reconstructs the message from the bundle's range/root and checks
  `VerifyingKey::verify`.

Key handling: there is **no key generation, storage, rotation, or loading** in this crate. The
signing key is a caller-supplied `[u8;32]` passed via `CheckpointConfig::with_sign_key`; the
public key is embedded per-checkpoint for self-contained export verification. Signing is
optional — default checkpoints are unsigned (`signature`/`signer_pubkey` = `None`).

---

## 10. Reproducibility (`reproduce.rs`, `reproduce/fusion.rs`)

`reproduce(answer_id)` replays a recorded answer to prove it was measured, not fabricated.

- `ReproduceLensRegistry` trait: `frozen_weights_sha256(lens_id) -> [u8;32]`,
  `measure_frozen(lens_id, &Input) -> SlotVector`. `ForgeBackend` trait:
  `activate_determinism(seed)`. `ReproduceInputResolver` trait: `resolve_input(&RecordedSlot)
  -> Input`; `InlineInputResolver` uses the slot's inline `input` (else corrupt).
- `QueryId = Vec<u8>`. `ReproduceContext { answer_id, ledger_entries:Vec<LedgerEntry>,
  recorded_slots:Vec<RecordedSlot> }`.
- `RecordedSlot { cx_id, slot_id, lens_id, weights_sha256:[u8;32], input_hash:[u8;32],
  corpus_shard_hash:Option<[u8;32]>, forge_seed:u64, input:Option<Input> }`.
  `RemeasuredSlot { cx_id, slot_id, lens_id, input_hash, forge_seed, vector:SlotVector }`.
- `build_reproduce_context(cf_reader, answer_id)` — finds the `Answer` entry with matching
  `SubjectId::Query`, reads `recorded_slots` and `measure_refs` (seq list) from its JSON
  payload, and pulls the referenced `Measure` entries.
- `remeasure_slots[_with_input_resolver]` — for each slot: `lookup_frozen_lens` (registry hash
  must equal recorded `weights_sha256`, else `CALYX_LENS_FROZEN_VIOLATION`),
  `activate_forge_determinism(forge_seed)`, resolve input, `verify_input_hash`
  (`blake3(input.bytes) == input_hash`, else corrupt), then `measure_frozen`. Missing
  `forge_seed` → `CALYX_REPRODUCE_NONDETERMINISTIC`.
- Fusion replay (`fusion.rs`): `FusionMode { SingleLens, Rrf, WeightedRrf }`,
  `FusionWeights { mode, k:usize, candidates:Vec<CxId>, weights:Vec<SlotWeight>,
  single_slot:Option<SlotId> }`, `SlotWeight { slot_id, weight }`, `HitRef { cx_id, score }`.
  `rerun_fusion` re-fuses dense slot scores: SingleLens uses raw scores; RRF/WeightedRRF use
  `weight / (rank + 1 + RRF_K)` with `RRF_K = 60.0`; sorts by score then cx id, truncates to
  `k`. `REPRODUCE_TOLERANCE = 1.0e-3`; `assert_within_tolerance(original, reproduced, tol)`
  returns `(within, max_drift)` (drift 1.0 if lengths/ids differ).
- `ReproduceResult { reproduced:bool, max_drift:f64, original_hits:Vec<HitRef>,
  reproduced_hits:Vec<HitRef> }`. `reproduce(...)` appends a result row to the ledger via
  `append_reproduce_entry` — an `Admin` entry, subject `SubjectId::Query(answer_id)`, actor
  `Service("calyx-reproduce")`, JSON payload tagged `REPRODUCE_PAYLOAD_TAG = "reproduce_v1"`
  (passes redaction check). `assert_reproduced(&result)` → `CALYX_REPRODUCE_DRIFT_EXCEEDED`
  when `!reproduced`.

---

## 11. Error taxonomy (`calyx-core/src/error.rs`)

| Constructor | Code | Raised when |
|---|---|---|
| `ledger_chain_broken` | `CALYX_LEDGER_CHAIN_BROKEN` | seq gap, prev-hash break, tip changed, seq/ts exhausted, quarantined range, tombstone seq mismatch. |
| `ledger_corrupt` | `CALYX_LEDGER_CORRUPT` | decode failure, hash mismatch, bad tags/lengths, missing/duplicate row, invalid range, bad payload JSON. |
| `ledger_append_only_violation` | `CALYX_LEDGER_APPEND_ONLY_VIOLATION` | overwrite, delete, or tombstone attempt on a `ledger` row. |
| `ledger_secret_in_payload` | `CALYX_LEDGER_SECRET_IN_PAYLOAD` | secret-like field/token in payload. |
| `ledger_actor_too_long` | `CALYX_LEDGER_ACTOR_TOO_LONG` | actor id > 64 UTF-8 bytes. |
| `ledger_group_commit_failed` | `CALYX_LEDGER_GROUP_COMMIT_FAILED` | staging/commit failure or disabled `on_commit`. |
| `lens_frozen_violation` | `CALYX_LENS_FROZEN_VIOLATION` | reproduce: registry weights hash ≠ recorded hash. |
| `reproduce_nondeterministic` | `CALYX_REPRODUCE_NONDETERMINISTIC` | Measure payload missing `forge_seed`. |
| `reproduce_drift_exceeded` | `CALYX_REPRODUCE_DRIFT_EXCEEDED` | `assert_reproduced` on a non-reproduced result. |

---

## 12. Constants reference

| Constant | Value | File |
|---|---|---|
| `HASH_BYTES` | 32 | entry.rs / merkle.rs |
| `SIGNATURE_BYTES` | 64 | merkle.rs |
| `MAX_ACTOR_ID_BYTES` | 64 | entry.rs |
| `HEADER_LEN` | 40 | codec.rs |
| `DEFAULT_CHECKPOINT_INTERVAL` | 1_000 | checkpoint.rs |
| `CHECKPOINT_TAG` | `"checkpoint_v1"` | checkpoint.rs |
| `MERKLE_EMPTY_ROOT` | `[0;32]` | merkle.rs |
| `MERKLE_SIGNING_DOMAIN` | `b"calyx-ledger-root-v1"` | merkle.rs |
| `REPRODUCE_TOLERANCE` | 1.0e-3 | reproduce/fusion.rs |
| `REPRODUCE_PAYLOAD_TAG` | `"reproduce_v1"` | reproduce/fusion.rs |
| `RRF_K` | 60.0 | reproduce/fusion.rs |
| `SECRET_TOKEN_MIN` | 40 | redaction.rs |
| `PAYLOAD_MAGIC` | `b"ETB1"` | tombstone/wire.rs |
| `DIGEST_DOMAIN` | `b"calyx-ledger-erasure-tombstone-subject-v1"` | tombstone.rs |

---

## Gaps / not covered

- **Payload schemas are conventions, not types.** `payload: Vec<u8>` is opaque to the chain;
  the per-kind JSON keys (Measure/Assay/Kernel/Guard/Answer fields in plan §1) are enforced
  only by readers (`audit`, `reproduce`), not at append time. The plan's typed payload table is
  aspirational relative to the code.
- **No key management.** Signing keys are caller-supplied `[u8;32]`; no generation, storage,
  rotation, or the plan's "reuse Leapable ingest signing key pattern" appears in this crate.
- **`DirectoryLedgerStore`** is documented in-code as "manual FSV before Aster group-commit
  wiring"; production durability is via the Aster `ledger` CF group-commit. zstd compression
  and hot/archive tiering described in plan §6 are Aster-side, not in this crate.
- **`LedgerGroupCommitHook::on_commit` is intentionally a disabled stub** (returns an error);
  the only live commit path is `stage`/`stage_with_checkpoints` + `commit_staged`.
- **Plan API name drift:** `verify_chain` returns a `VerifyResult` enum (Intact/Broken/Corrupt),
  not the plan's `{intact, broken_at?}`; `EntryKind` adds `Erase` beyond the plan's list.
