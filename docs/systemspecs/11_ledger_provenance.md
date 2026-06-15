# 11. Ledger Provenance (calyx-ledger)

Append-only, hash-chained provenance ledger. Every write records a canonical
`LedgerEntry` whose BLAKE3 `entry_hash` chains to the predecessor's hash, so the
full history is tamper-evident. The crate provides the single append path, a
deterministic binary codec, payload redaction guardrails, periodic signed Merkle
checkpoints, chain verification with quarantine, deterministic answer
reproduction, metadata-only erasure tombstones, and an audit query surface.

This document describes only what the source does. Where behavior is not
expressed in the code, it is marked "Not determined from source".

See [05_aster_storage.md](05_aster_storage.md) for the storage engine that
hosts the `ledger` column family and drives group commit, and
[12_ward_guard.md](12_ward_guard.md) for the Guard subsystem whose decisions are
recorded as `Guard` entries.

## Source files covered

- `src/lib.rs` — module wiring and the public re-export surface.
- `src/entry.rs` — `LedgerEntry`, `SubjectId`, `ActorId`, `compute_entry_hash`.
- `src/kind.rs` — `EntryKind` and stable wire codes.
- `src/codec.rs` — `encode` / `decode` / `decode_header`, cursor-based parser.
- `src/append.rs` — `LedgerAppender`, `LedgerCfStore`, row stores, tip recovery.
- `src/verify.rs` — `verify_chain`, `VerifyResult`.
- `src/merkle.rs` — Merkle root, ed25519 signing, `MerkleExportBundle`.
- `src/checkpoint.rs` — `CheckpointScheduler`, `CheckpointPayload`, overlay store.
- `src/redaction.rs` — `RedactionPolicy`, secret scanner, `PayloadBuilder`.
- `src/group_commit.rs` — `DefaultLedgerHook`, staged rows, batch keys.
- `src/audit.rs` + `src/audit/mentions.rs` — `audit`, `get_provenance`,
  `get_answer_trace`, quarantine lookups.
- `src/reproduce.rs` + `src/reproduce/fusion.rs` — reproduce context, slot
  re-measurement, fusion replay, reproduce verdict entry.
- `src/tombstone.rs` + `src/tombstone/wire.rs` — erasure tombstones (JSON +
  bincode wire forms).

Crate size at time of writing: ~5,250 lines across `src/` (incl. tests).

---

## 1. Ledger column family and record layout

### 1.1 Storage contract (`LedgerCfStore`)

The `ledger` column family is abstracted by the `LedgerCfStore` trait
(`src/append.rs`):

| Method | Behavior |
|---|---|
| `scan() -> Vec<LedgerRow>` | Returns all rows sorted by sequence number. |
| `put_new(seq, bytes)` | Writes a new row. Implementations must reject overwrites. |
| `delete(seq)` | Default impl calls `reject_delete` → `CALYX_LEDGER_APPEND_ONLY_VIOLATION`. |
| `tombstone(seq)` | Default impl calls `reject_tombstone` → `CALYX_LEDGER_APPEND_ONLY_VIOLATION`. |

A `LedgerRow` is `{ seq: u64, bytes: Vec<u8> }` (`src/append.rs`). The CF key is
the big-endian `u64` sequence number; `ledger_batch_key(seq)` (group_commit.rs)
produces `seq.to_be_bytes().to_vec()` and the doc comment states it "must match
Aster `ledger_key`". See [05_aster_storage.md](05_aster_storage.md).

Three store implementations exist:

- `MemoryLedgerStore` — `BTreeMap<u64, Vec<u8>>`; `put_new` errors with
  `CALYX_LEDGER_APPEND_ONLY_VIOLATION` if `seq` exists.
- `DirectoryLedgerStore` — disk-backed, one file per row named
  `{seq:016x}.ledger`. `put_new` opens with `create_new(true)` (fail-closed on
  existing file → append-only violation), writes bytes, and calls `sync_all()`.
  `scan` reads `*.ledger` files, parses the hex stem back to `seq`, sorts by
  `seq`. Described in source as "manual FSV before Aster group-commit wiring".
- `OverlayLedgerStore` (checkpoint.rs) — read-only union of a base store plus
  pending prepared rows; `put_new` always errors append-only.

### 1.2 Logical record (`LedgerEntry`)

Defined in `src/entry.rs`:

| Field | Type | Meaning |
|---|---|---|
| `seq` | `u64` | Monotonic sequence number, gap-free from 0. |
| `prev_hash` | `[u8; 32]` | `entry_hash` of predecessor; `[0;32]` at seq 0. |
| `kind` | `EntryKind` | Event kind (§3). |
| `subject` | `SubjectId` | Primary object (tagged enum, §1.4). |
| `payload` | `Vec<u8>` | Kind-specific body (usually redacted JSON). |
| `actor` | `ActorId` | Causing service/agent/system (§1.5). |
| `ts` | `u64` | Monotonic timestamp (§2.3). |
| `entry_hash` | `[u8; 32]` | Canonical BLAKE3 hash of all prior fields. |

`HASH_BYTES = 32` (BLAKE3 output width). `LedgerEntry::new(...)` computes
`entry_hash` at construction; `LedgerEntry::verify()` recomputes and compares.

### 1.3 Physical wire layout (codec)

`encode` (`src/codec.rs`) produces a stable, padding-free big-endian layout. The
fixed header is `SEQ_LEN(8) + HASH_BYTES(32) = 40` bytes.

| Order | Field | Bytes |
|---|---|---|
| 1 | `seq` | 8 (BE u64) |
| 2 | `prev_hash` | 32 |
| 3 | `kind` wire code | 1 |
| 4 | `subject` tag | 1 |
| 5 | subject length | 2 (BE u16) |
| 6 | subject bytes | _len_ |
| 7 | payload length | 4 (BE u32) |
| 8 | payload bytes | _len_ |
| 9 | `actor` tag | 1 |
| 10 | actor length | 2 (BE u16) |
| 11 | actor bytes | _len_ |
| 12 | `ts` | 8 (BE u64) |
| 13 | `entry_hash` | 32 |

`encode` asserts subject ≤ `u16::MAX`, actor ≤ `u16::MAX`, payload ≤ `u32::MAX`.
`decode` parses via a bounds-checked `Cursor`, then calls `entry.verify()` and
fails with `CALYX_LEDGER_CORRUPT` on hash mismatch, trailing bytes, truncation,
invalid kind/subject/actor tags, non-UTF-8 actor, or a non-empty `System`
actor. `decode_unchecked` (crate-internal) skips the final hash check and is
used by `verify_chain` so it can classify a hash mismatch as `Broken` rather
than `Corrupt`. `decode_header(bytes)` reads only `seq` and `prev_hash` from the
first 40 bytes for fast chain-link probes.

Golden vector (codec test `codec_golden`): an entry `{seq=42,
prev_hash=[0x11;32], kind=Measure, subject=Cx([0x22;16]), payload="synthetic",
actor=Service("svc"), ts=99}` encodes to the constant `CODEC_GOLDEN_HEX` in
`src/codec.rs`.

### 1.4 Subject identifiers (`SubjectId`)

| Variant | Wire tag | Payload bytes |
|---|---|---|
| `Cx(CxId)` | 0 (`TAG_CX`) | 16-byte id |
| `Lens(LensId)` | 1 (`TAG_LENS`) | 16-byte id |
| `Kernel(Vec<u8>)` | 2 (`TAG_KERNEL`) | arbitrary |
| `Guard(Vec<u8>)` | 3 (`TAG_GUARD`) | arbitrary |
| `Query(Vec<u8>)` | 4 (`TAG_QUERY`) | arbitrary |

`Cx`/`Lens` decode requires exactly 16 bytes (`copy_16`). The canonical hash
input for a subject is `tag_byte || wire_bytes` (`canonical_bytes`).

### 1.5 Actor identifiers (`ActorId`)

| Variant | Wire tag | Bytes |
|---|---|---|
| `Agent(String)` | 0 (`TAG_AGENT`) | UTF-8 name |
| `Service(String)` | 1 (`TAG_SERVICE`) | UTF-8 name |
| `System` | 2 (`TAG_SYSTEM`) | empty (enforced) |

`ActorId::validate()` rejects names whose UTF-8 length exceeds
`MAX_ACTOR_ID_BYTES = 64` with `CALYX_LEDGER_ACTOR_TOO_LONG`. The canonical hash
input for an actor is `tag_byte || len_be_u64 || bytes` (`tagged_var`).

---

## 2. Hash-chain formula and the append algorithm

### 2.1 Entry hash preimage

`compute_entry_hash` (`src/entry.rs`) feeds every field into a single BLAKE3
hasher, each field length-prefixed by a big-endian `u64` via `frame`:

```text
frame(hasher, x) := hasher.update(be_u64(x.len())); hasher.update(x)

entry_hash = BLAKE3(
    frame( be_u64(seq) )                  // 8-byte value, length-prefixed
    frame( prev_hash )                    // 32 bytes
    frame( [kind.wire_code()] )           // 1 byte
    frame( subject_tag || subject_bytes ) // canonical_bytes(subject)
    frame( payload )
    frame( actor_tag || be_u64(actor_len) || actor_bytes ) // canonical_bytes(actor)
    frame( be_u64(ts) )
)
```

The length-prefix framing makes the encoding injective across field boundaries,
so changing any field changes the hash (property test
`changing_each_field_changes_hash`). Golden: the entry in `golden_entry()`
hashes to the `GOLDEN_HASH` constant in `src/entry.rs`.

The chain link is `entry[n].prev_hash == entry[n-1].entry_hash`, with
`entry[0].prev_hash == [0; 32]`.

### 2.2 Append / hash-chain link steps (`LedgerAppender`)

`LedgerAppender<S, C>` (`src/append.rs`) is "the single write path". It caches
the recovered tip: `next_seq`, `prev_hash`, `last_ts`, plus a `RedactionPolicy`.

`open` → `open_with_policy(store, clock, RedactionPolicy::default())` calls
`recover_tip` (§2.4).

`append(kind, subject, payload, actor)` = `prepare(...)` then
`commit_prepared(...)`. The `prepare` path (`prepare_at`):

1. `redaction_policy.check_payload_with_policy(&payload)` — reject secret-like
   payloads (§4).
2. `verify_tip()` — re-run `recover_tip` and confirm the store still matches the
   cached `(next_seq, prev_hash, last_ts)`; else `CALYX_LEDGER_CHAIN_BROKEN`.
3. `actor.validate()` (≤64 bytes).
4. `actor = redaction_policy.apply_to_actor(actor)` then `validate()` again.
5. `ts = next_ts_after(last_ts)` (§2.3).
6. Build `LedgerEntry::new(seq, prev_hash, kind, subject, payload, actor, ts)`
   (computes `entry_hash`), `encode` it, return `PreparedLedgerEntry { entry,
   bytes }`.

`prepare` does not mutate the store or tip. `commit_prepared(prepared)`:

1. Reject if `prepared.seq != next_seq` or `prepared.prev_hash != prev_hash`
   (`CALYX_LEDGER_CHAIN_BROKEN`).
2. `store.put_new(seq, bytes)` (fail-closed on overwrite).
3. Advance cached tip: `last_ts = ts`, `next_seq = seq + 1` (checked add),
   `prev_hash = entry_hash`. Return `LedgerRef { seq, hash }`.

`prepare_after(predecessor, ...)` builds the row that must follow an
uncommitted staged row, chaining `prev_hash = predecessor.entry_hash()` and
`seq = predecessor.seq() + 1`. This is used to stage a checkpoint immediately
after a data row in the same batch (§5).

### 2.3 Timestamp monotonicity

`next_ts_after(last_ts)`: read `clock.now()`; if `clock_ts <= last_ts`, use
`last_ts + 1` (checked, else `CALYX_LEDGER_CHAIN_BROKEN` "timestamp exhausted"),
otherwise use `clock_ts`. Timestamps are therefore strictly increasing across
the chain even with a non-monotonic or fixed clock.

### 2.4 Tip recovery (`recover_tip`)

`recover_tip` scans the store in seq order and validates, returning
`(next_seq, prev_hash, last_ts)`:

- Each `row.seq` must equal the running `next_seq`, else
  `CALYX_LEDGER_CHAIN_BROKEN` ("seq gap").
- `decode(row.bytes)` (full hash check); encoded `entry.seq` must equal
  `row.seq` (`CALYX_LEDGER_CORRUPT`).
- `entry.prev_hash` must equal the prior entry's hash, else
  `CALYX_LEDGER_CHAIN_BROKEN`.
- Advance `prev_hash = entry_hash`, `last_ts = entry.ts`, `next_seq += 1`.

An empty store recovers `(0, [0;32], 0)`.

---

## 3. EntryKinds

`EntryKind` (`src/kind.rs`) is a copyable enum with a stable one-byte
discriminant. `EntryKind::ALL` lists all ten in wire-code order;
`from_wire_code` returns `None` for codes ≥ 10.

| Variant | Wire code | `as_str()` | Notes from source |
|---|---|---|---|
| `Ingest` | 0 | `ingest` | Mapped from `WriteOp::Ingest`. |
| `Measure` | 1 | `measure` | Carries `RecordedSlot` provenance for reproduce. |
| `Assay` | 2 | `assay` | (kind only; no special handling in this crate). |
| `Kernel` | 3 | `kernel` | Linked from answer traces via `kernel_ref`/`kernel_id`. |
| `Guard` | 4 | `guard` | Linked via `guard_ref`/`guard_id`; supplies `guard_result`. |
| `Answer` | 5 | `answer` | Root of answer-trace and reproduce; subject is `Query(answer_id)`. |
| `Anneal` | 6 | `anneal` | (kind only). |
| `Migrate` | 7 | `migrate` | (kind only). |
| `Admin` | 8 | `admin` | Used for checkpoints, reproduce verdicts, vault admin. |
| `Erase` | 9 | `erase` | Carries an `ErasureTombstone` payload (§9). |

`WriteOp` → kind mapping (`ingest_kind_for`, group_commit.rs):
`Ingest→Ingest`, `VaultAdmin→Admin`, `Erase→Erase`.

`Assay`, `Anneal`, `Migrate` are valid wire codes but have no producer/consumer
logic inside this crate beyond hashing/codec/audit. Their semantics elsewhere
are Not determined from source.

---

## 4. Redaction and secret guardrails

`RedactionPolicy` (`src/redaction.rs`):

| Field | Type | Default | Effect |
|---|---|---|---|
| `store_raw_input` | `bool` | `false` | Keep raw/plaintext fields in `apply_to_payload`. |
| `redact_actor_name` | `bool` | `false` | Replace agent/service names with `"redacted"`. |

### 4.1 Payload scanner (`check_payload_with_policy`)

Runs before every append. Empty payload is allowed. If the payload parses as
JSON, `check_json_value` recurses; otherwise `check_text_tokens` scans the
lossy-UTF-8 text. It is fail-closed: anything matching a secret heuristic raises
`CALYX_LEDGER_SECRET_IN_PAYLOAD`.

- **Secret field names** (`is_secret_field`, after normalizing non-alphanumerics
  to `_` and lowercasing): exact `password`, `passwd`, `token`, `secret`, `key`,
  or any field ending `_password`/`_passwd`/`_token`/`_secret`/`_key`. Public-key
  fields are exempted first (`signer_pubkey`, `public_key`, `verifying_key`).
- **Token heuristic** (`check_text_tokens`): a no-whitespace printable run of ≥
  `SECRET_TOKEN_MIN = 40` chars, or any token ≥ 40 chars, is rejected unless it
  is an *allowed stable identifier* for its field.

`allowed_stable_identifier` whitelists, per field:
- source-metadata fields (`chunk_id`, `database_name`): alphanumeric +
  `_-.:/`, length ≤ `MAX_SOURCE_METADATA_LEN = 128`.
- `signature`: 128 hex chars.
- public-key fields: `MAX_HASH_OR_ID_LEN = 64` hex chars.
- `quant_slot_*`: hex, length ≤ `MAX_QUANT_SLOT_METADATA_LEN = 4096`.
- fields ending `_hash`/`_id`/`_sha256`/`_digest` and `hash`/`metadata`/
  `input_hash`/`root`/`weights_sha256`: hex, base58, or UUID, length ≤ 64.

### 4.2 Payload building (`apply_to_payload`, `PayloadBuilder`)

`PayloadBuilder` is a thin JSON object builder. `apply_to_payload` filters a
value with `filter_payload_value`/`keep_payload_field`:
- `input_ref` is rewritten to `{hash, redacted: true}` (`filter_input_ref`,
  drops the pointer).
- secret fields are dropped.
- raw fields (`raw`, `raw_bytes`, `raw_input`, `input_bytes`, `plaintext`, or
  ending `_raw`/`_bytes`) are kept only if `store_raw_input`.
- otherwise kept only if `ts`, `redacted`, or a stable-identifier field.

`redact_input_ref(input_ref)` returns a `RedactedInput { hash, redacted: true,
pointer: None }` preserving the content hash while dropping the pointer.

### 4.3 Actor redaction

`apply_to_actor`: when `redact_actor_name`, `Agent(_)→Agent("redacted")`,
`Service(_)→Service("redacted")`, `System→System`. Applied inside `prepare_at`
before hashing, so the redacted name is what is chained.

---

## 5. Group commit integration

`src/group_commit.rs` wires the appender into the storage engine's durable batch.

- `LedgerWriteBatch` trait: `put_ledger_row(key, value)`. `WriteBatch` is the
  in-memory test impl collecting `LedgerBatchRow { key, value }`.
- `DefaultLedgerHook<S, C>` wraps a `LedgerAppender` plus an optional
  `CheckpointScheduler`. Constructors: `new`, `with_checkpoint_config`
  (recovers a scheduler from the store), `with_checkpoint_scheduler`.

Flow (prepare-stage / commit-after-durable-write):

1. `stage(kind, subject, payload, actor)` calls `appender.prepare(...)` and
   returns a `StagedLedgerRow { key = ledger_batch_key(seq), value = bytes,
   ledger_ref, prepared, checkpoint_range_end }`. Staging does **not** advance
   the tip.
2. `stage_with_checkpoints(...)` stages the data row, then if a scheduler exists
   and `should_checkpoint(range_end)` (with `range_end = data_seq + 1`), builds
   an `OverlayLedgerStore` (base + the staged data row), computes the checkpoint
   over `range_start..range_end`, and stages a second `Admin` checkpoint row via
   `prepare_checkpoint_after`. Returns the staged rows in order.
3. Caller puts every staged row into the durable storage batch and commits it.
4. `commit_staged(staged)` calls `appender.commit_prepared` (advancing the tip)
   and, for a checkpoint row, `scheduler.advance_after_checkpoint(range_end)`.

The legacy direct path `LedgerGroupCommitHook::on_commit` is intentionally
**disabled**: it always returns `CALYX_LEDGER_GROUP_COMMIT_FAILED`
("direct ... is disabled; use stage_with_checkpoints and commit_staged after
durable storage commit"). This forces ledger state to advance only through the
durable staged path (referenced as issue #652). `group_commit_failed` wraps
stage/commit errors with the same code.

`ledger_batch_key(seq) = seq.to_be_bytes()` — must match Aster's `ledger_key`
(see [05_aster_storage.md](05_aster_storage.md)).

---

## 6. Merkle checkpoints and signatures

### 6.1 Merkle root computation (`src/merkle.rs`)

Domain-separated BLAKE3 binary Merkle tree over entry hashes:

```text
leaf_hash(h)        = BLAKE3( "leaf" || h )
combine_hash(l, r)  = BLAKE3( "node" || l || r )

merkle_root_of_hashes(hashes):
    if empty: return MERKLE_EMPTY_ROOT  (= [0; 32])
    level = [ leaf_hash(h) for h in hashes ]
    while level.len() > 1:
        if level.len() is odd: duplicate the last element
        level = [ combine_hash(pair[0], pair[1]) for each adjacent pair ]
    return level[0]
```

Odd levels duplicate the final node before pairing (classic Bitcoin-style
duplication). `merkle_root(store, range)`: validates `start <= end` (else
`CALYX_LEDGER_CORRUPT`), returns `MERKLE_EMPTY_ROOT` for an empty range,
collects in-range rows into a `BTreeMap` (duplicate seq → corrupt), then for
each `seq` in range full-`decode`s the row (verifies hash), checks encoded
`seq`, and pushes `entry_hash`. The ordered `entry_hash` list feeds
`merkle_root_of_hashes`.

### 6.2 Signature scheme (ed25519)

`SIGNATURE_BYTES = 64`. Signing domain `MERKLE_SIGNING_DOMAIN =
b"calyx-ledger-root-v1"`. The signed message is:

```text
signing_message(range, root) =
    "calyx-ledger-root-v1"      (20 bytes)
    || be_u64(range.start)      (8)
    || be_u64(range.end)        (8)
    || root                     (32)
```

`sign_root(range, root, signing_key)` builds an `ed25519_dalek::SigningKey`
from the 32-byte seed and signs that message → 64-byte signature.
`verify_signature(bundle)` requires both `signature` and `signer_pubkey`, rebuilds
the message from the bundle's range+root, and returns whether
`VerifyingKey::verify` succeeds (false on missing fields or bad key bytes).

### 6.3 Export bundle (`MerkleExportBundle`)

| Field | Type | Notes |
|---|---|---|
| `range_start` | `u64` | inclusive |
| `range_end` | `u64` | exclusive |
| `root` | `[u8; 32]` | Merkle root |
| `signature` | `Option<[u8; 64]>` | custom serde (`signature_serde`) |
| `signer_pubkey` | `Option<[u8; 32]>` | ed25519 verifying key |

`unsigned(range, root)` leaves signature/pubkey `None`; `signed(range, root,
key)` fills both. Custom serde serializes the signature as an optional byte
slice and rejects deserialized signatures whose length ≠ 64.

### 6.4 Checkpoint scheduling (`src/checkpoint.rs`)

`CheckpointConfig { interval_entries: u64, sign_key: Option<[u8;32]> }`.
`DEFAULT_CHECKPOINT_INTERVAL = 1_000`. `validate()` rejects
`interval_entries == 0` with `CALYX_LEDGER_CORRUPT`.

`CheckpointScheduler` tracks `range_start` and `next_checkpoint_at` (initially
`interval_entries`).

- `should_checkpoint(current_seq)` = `current_seq >= next_checkpoint_at &&
  range_start < current_seq`.
- `prepare_checkpoint_after(appender, store, predecessor, range_end_seq)`:
  `range = range_start..range_end_seq`; `root = merkle_root(store, range)`;
  builds a `CheckpointPayload::from_root(range, root, sign_key)` (signed bundle
  if a key is set), then `appender.prepare_after(predecessor, Admin,
  checkpoint_subject(), payload.encode(), ActorId::System)`. The checkpoint
  subject is `Query(b"checkpoint_v1")`.
- `advance_after_checkpoint(range_end_seq)`: `range_start = range_end_seq + 1`
  (checked), `next_checkpoint_at = range_start + interval_entries` (saturating).
- `recover(config, store)`: replays all `Admin` rows whose payload decodes as a
  checkpoint (`decode_optional`), calling `advance_from_payload` to rebuild
  `range_start`/`next_checkpoint_at`.

`CheckpointPayload` (JSON, stored as the Admin entry payload):

| Field | Type | Notes |
|---|---|---|
| `tag` | `String` | must equal `CHECKPOINT_TAG = "checkpoint_v1"` |
| `range_start` | `u64` | |
| `range_end` | `u64` | |
| `root` | `String` | hex of 32-byte root |
| `signature` | `Option<String>` | 128 hex chars (skipped if `None`) |
| `signer_pubkey` | `Option<String>` | 64 hex chars (skipped if `None`) |

`decode` validates `tag`, `range_start <= range_end`, `root` parses to 32 bytes,
and optional hex lengths (signature 64 bytes / 128 chars, pubkey 32 bytes / 64
chars); all failures → `CALYX_LEDGER_CORRUPT`. Hex is encoded/decoded by the
module's own `hex`/`parse_hex_array` helpers (lowercase).

`OverlayLedgerStore::new(base, pending)` builds a read-only union, using
`insert_unique` which deduplicates identical bytes but rejects divergent bytes
for the same seq (`CALYX_LEDGER_CORRUPT` "divergent ledger bytes").

---

## 7. Verify-chain and quarantine

### 7.1 `verify_chain(store, range)` (`src/verify.rs`)

`VerifyResult`:

| Variant | Fields | Meaning |
|---|---|---|
| `Intact` | `count: u64` | Range verified; `count` entries checked. |
| `Broken` | `at_seq, expected, found` | Hash-link or entry-hash mismatch. |
| `Corrupt` | `at_seq, reason` | Missing/undecodable/seq-mismatched row. |

`quarantine_seq()` returns `Some(at_seq)` for `Broken`/`Corrupt`, `None` for
`Intact`. This is the seq an operator quarantines.

Steps:

1. Reject `start > end` (`CALYX_LEDGER_CORRUPT`); `start == end` → `Intact{0}`.
2. Snapshot all rows into a `BTreeMap`.
3. `expected_prev_hash`: if `start == 0`, `[0; 32]`. Else decode the row at
   `start-1` (via `decode_unchecked`), require its `seq`, require `verify()`
   true; any failure yields a `Corrupt{at_seq: start, reason}` (so a broken
   predecessor poisons the range start).
4. For each `seq` in range: missing row → `Corrupt`; `decode_unchecked` error →
   `Corrupt`; encoded-seq mismatch → `Corrupt`; `entry.prev_hash !=
   expected_prev` → `Broken`; recomputed entry hash != stored `entry_hash` →
   `Broken`. Otherwise advance `expected_prev = entry_hash`, `count += 1`.
5. Return `Intact { count }`.

Using `decode_unchecked` lets a tampered entry hash surface as `Broken` (with
`expected`/`found`) rather than as a decode `Corrupt`.

### 7.2 Quarantine in the audit surface

`QuarantineLookup::contains_quarantined(range) -> bool`. `QuarantineSet`
(`src/audit.rs`) holds non-empty `Range<u64>`s (`from_ranges` rejects
`start >= end` with `CALYX_LEDGER_CHAIN_BROKEN`) and answers true on any
overlap (`ranges_overlap`: `left.start < right.end && right.start < left.end`).

All query functions reject quarantined data fail-closed:
`ensure_seq_not_quarantined(seq)` checks `seq..seq+1`;
`ensure_range_not_quarantined` raises `CALYX_LEDGER_CHAIN_BROKEN`
("ledger range a..b is quarantined") when a non-empty range overlaps. `audit`,
`get_provenance`, and `get_answer_trace` all gate scanned rows through these
checks so a quarantined seq cannot be returned in results.

There is no automatic quarantine writer in this crate: `verify_chain` reports
`quarantine_seq()` and callers construct a `QuarantineSet`. The wiring that
persists quarantine ranges is Not determined from source (outside calyx-ledger).

---

## 8. Reproduce / bit-parity

### 8.1 Context build (`build_reproduce_context`, `src/reproduce.rs`)

Given an `answer_id` (`QueryId = Vec<u8>`), scan and decode all rows, find the
`Answer` entry whose subject is `Query(answer_id)`, parse its JSON payload, then:
- read inline `recorded_slots` from the answer payload, and
- for each `seq` in `measure_refs`, look up that entry, require it is a
  `Measure` entry (else `CALYX_LEDGER_CORRUPT`), and append its
  `RecordedSlot`.

Result is a `ReproduceContext { answer_id, ledger_entries, recorded_slots }`.

`RecordedSlot` fields: `cx_id`, `slot_id`, `lens_id`, `weights_sha256 ([u8;32])`,
`input_hash ([u8;32])`, optional `corpus_shard_hash`, `forge_seed (u64)`,
optional inline `input`. `forge_seed` is mandatory — a missing one is
`CALYX_REPRODUCE_NONDETERMINISTIC`.

### 8.2 Slot re-measurement (`remeasure_slots_with_input_resolver`)

For each recorded slot, deterministically:

1. `lookup_frozen_lens(registry, lens_id, weights_sha256)` — the registry's
   frozen weights hash must equal the recorded one, else
   `CALYX_LENS_FROZEN_VIOLATION`.
2. `activate_forge_determinism(forge, forge_seed)` — pins deterministic Forge
   execution to the recorded seed.
3. `resolver.resolve_input(slot)` — `InlineInputResolver` uses the slot's inline
   `input`; an external resolver supplies content-addressed bytes.
4. `verify_input_hash`: `BLAKE3(input.bytes) == slot.input_hash`, else
   `CALYX_LEDGER_CORRUPT`.
5. `registry.measure_frozen(lens_id, input)` → `SlotVector`, producing a
   `RemeasuredSlot`.

### 8.3 Fusion replay and verdict (`src/reproduce/fusion.rs`)

`reproduce(store, registry, forge, answer_id)` builds the context, re-measures,
and calls `reproduce_from_remeasured`:

1. Read the answer payload, parse `original_hits: Vec<HitRef>` and
   `fusion_weights: FusionWeights`.
2. `rerun_fusion(remeasured, fusion_weights)` recomputes hits.
3. `assert_within_tolerance(original_hits, reproduced_hits,
   REPRODUCE_TOLERANCE)` → `(reproduced, max_drift)`.
4. `append_reproduce_entry` records the verdict.

`rerun_fusion`: for each remeasured slot that participates (mode-dependent),
require a dense score vector (`as_dense`) whose length equals the candidate
count (else `CALYX_LEDGER_CORRUPT`), compute a per-slot weight, then for each
candidate add a contribution:
- `SingleLens`: raw dense score (`single_slot` selects the one participating
  slot).
- `Rrf` / `WeightedRrf`: `weight / ((rank + 1) + RRF_K)` with `RRF_K = 60.0`;
  `Rrf` weight is `1.0`, `WeightedRrf` uses the per-slot weight (default `0.0`,
  zero-weight slots skipped).

Fused scores are sorted descending (ties broken by `cx_id` string), truncated to
`k`. `assert_within_tolerance`: empty/empty → `(true, 0.0)`; differing lengths
or a missing cx_id → `(false, 1.0)`; otherwise `max_drift` is the largest
absolute per-cx score delta and reproduced = `max_drift <= tol`.
`REPRODUCE_TOLERANCE = 1.0e-3`. `assert_reproduced` converts a `false` verdict
into `CALYX_REPRODUCE_DRIFT_EXCEEDED`.

### 8.4 Reproduce verdict entry

`append_reproduce_entry(store, answer_id, result)` recovers the tip (its own
`recover_tip`), sets `ts = last_ts + 1`, builds a JSON payload tagged
`REPRODUCE_PAYLOAD_TAG = "reproduce_v1"` with fields `answer_id` (hex),
`reproduced`, `max_drift`, `original_hits`, `reproduced_hits`, `ts`, runs it
through `RedactionPolicy::check_payload`, then appends an `Admin` entry whose
subject is `Query(answer_id)` and actor is `Service("calyx-reproduce")`, writing
via `store.put_new`.

`ReproduceResult { reproduced: bool, max_drift: f64, original_hits, reproduced_hits }`.
Bit-parity here is defined as score reproduction within tolerance, not byte
identity of the result vectors.

---

## 9. Erasure tombstones

`src/tombstone.rs` adds metadata-only `Erase` entries (right-to-erasure marker
that never stores erased content).

`ErasureScope`: `Vault`, `Cx(CxId)`, or `Subject(SubjectId)`.
`ErasureTombstone { seq, vault_id, scope, actor, erased_at: Ts, records_deleted:
usize }`. `ledger_subject()`: `Cx→Cx`, `Subject→that subject`,
`Vault→Guard(scope_digest(...))` where `scope_digest` is
`BLAKE3("calyx-ledger-erasure-tombstone-subject-v1" || vault_id || scope_tag ||
id?)`.

`write_tombstone(tombstone, ledger)` requires `ledger.next_seq() ==
tombstone.seq` (else `CALYX_LEDGER_CHAIN_BROKEN`) and appends an `Erase` entry
with the tombstone payload and actor. `tombstone_from_entry` decodes an `Erase`
entry's payload and cross-checks that payload `seq` and `actor` match the ledger
entry (else `CALYX_LEDGER_CORRUPT`). `find_tombstone` / `is_tombstoned` scan for
a tombstone matching `(vault_id, scope)`.

Two payload encodings, both decoded by `from_ledger_payload`:
- **Wire** (`src/tombstone/wire.rs`, preferred): magic `b"ETB1"` + bincode
  (`config::standard`) of `WireTombstone`; rejects trailing bytes. Scopes/actors
  map to compact enums; vault id is the 16-byte ULID.
- **Compact JSON** (`CompactTombstone`): fields `q,v,c/sc/sl/sk/sg/sq,a,t,n`;
  exactly one scope field may be set (`set_scope` rejects multiples). Actor is
  `"A:<id>"` / `"S:<id>"` / `"Y"`. Used for `as_json_value` (CLI readback) and
  accepted as legacy input.

The `ledger` CF's `delete`/`tombstone` row operations remain forbidden (§1.1);
"tombstone" here means an appended `Erase` provenance record, not row deletion.
See [12_ward_guard.md](12_ward_guard.md) for how erasure is enforced at the
guard layer.

---

## 10. Audit query surface

`src/audit.rs`. All three functions decode physical rows
(`decode_physical_row`, which also checks key seq == encoded seq) and gate on
quarantine.

### 10.1 `audit(cf_reader, quarantine, filter)`

`AuditFilter` parameters:

| Parameter | Type | Semantics |
|---|---|---|
| `kind` | `Option<EntryKind>` | exact kind match |
| `actor` | `Option<ActorId>` | exact actor match |
| `ts_range` | `Option<(u64, u64)>` | `start <= ts < end` |
| `seq_range` | `Option<(u64, u64)>` | `start <= seq < end` (half-open) |

If `seq_range` is set, the whole range is quarantine-checked up front, and rows
outside the range are skipped before decode. Each matching row is then
individually quarantine-checked before being collected.

### 10.2 `get_provenance(cf_reader, quarantine, cx_id)`

Decodes all (non-quarantined) entries and returns those that *mention* the
`cx_id`. `entry_mentions_cx` (`src/audit/mentions.rs`) matches when the subject
is `Cx(cx_id)`, or when the JSON payload contains the cx id string in a known
cx-bearing field: `cx_id`, `from_id`, `to_id`, `source_cx_id`, `target_cx_id`,
`nearest_cx`, `matched_cx_id`, `query_id`, `anchor_kernel_node_id` (recursing
through nested objects/arrays).

### 10.3 `get_answer_trace(cf_reader, quarantine, answer_id)`

Reconstructs a provenance trace for an answer:

1. Collect `Answer` entries whose subject is `Query(answer_id)`, sorted by seq;
   each must not be quarantined. Empty → `unprovenanced_trace("answer_trace.missing")`.
2. Prefer the latest payload with `"complete": true`; else fall back to the last
   payload (and assemble a partial path from each payload's hops).
3. Build the hop `path` from the payload's `path` array (or `hop_index` entry),
   sorted by `hop`. `trace_hop` reads `hop`/`hop_index`, `cx_id`/`to_id`,
   optional `from_id`, `score`/`hop_score`, optional `lens_id`, and
   `ledger_ref.seq`/`ledger_seq` (defaulting to the entry seq).
4. Resolve `kernel_entry` and `guard_entry` via `kernel_ref`/`guard_ref`
   (seq + optional hash match) or by `kernel_id`/`guard_id` subject match.
5. Extract `fusion_weights`, `guard_result` (guard entry payload or
   `guard_result` field), and `freshness_ts`/`freshness_ts_millis`.

`AnswerTrace` fields: `answer_entry`, `kernel_entry`, `guard_entry`, `path:
Vec<AnswerTraceHop>`, `fusion_weights`, `guard_result`, `freshness_ts`,
`complete: bool`, `warnings: Vec<CalyxWarning>`. `complete` requires a complete
payload, a `path` field, contiguous hops (`hop == index`), and `expected_hops`
matching the path length. `is_trusted() = complete && warnings.is_empty()`.
Warnings (`CalyxWarning::unprovenanced`) are pushed for
`answer_trace.partial_or_unmarked`, `answer_trace.kernel_unprovenanced`,
`answer_trace.guard_unprovenanced`.

`AnswerTraceHop`: `cx_id`, `from_cx_id`, `hop: u32`, `score: f32`, `lens_id`,
`ledger_seq`.

---

## 11. Constants and error conditions

### 11.1 Constants

| Constant | Value | File |
|---|---|---|
| `HASH_BYTES` | 32 | entry.rs / merkle.rs |
| `SIGNATURE_BYTES` | 64 | merkle.rs |
| `MAX_ACTOR_ID_BYTES` | 64 | entry.rs |
| Subject tags | Cx=0, Lens=1, Kernel=2, Guard=3, Query=4 | entry.rs |
| Actor tags | Agent=0, Service=1, System=2 | entry.rs |
| `ROW_EXT` | `"ledger"` | append.rs |
| `MERKLE_EMPTY_ROOT` | `[0; 32]` | merkle.rs |
| `MERKLE_SIGNING_DOMAIN` | `b"calyx-ledger-root-v1"` | merkle.rs |
| `CHECKPOINT_TAG` | `"checkpoint_v1"` | checkpoint.rs |
| `DEFAULT_CHECKPOINT_INTERVAL` | 1000 | checkpoint.rs |
| `SECRET_TOKEN_MIN` | 40 | redaction.rs |
| `MAX_HASH_OR_ID_LEN` | 64 | redaction.rs |
| `MAX_QUANT_SLOT_METADATA_LEN` | 4096 | redaction.rs |
| `MAX_SOURCE_METADATA_LEN` | 128 | redaction.rs |
| `REPRODUCE_TOLERANCE` | 1.0e-3 | fusion.rs |
| `REPRODUCE_PAYLOAD_TAG` | `"reproduce_v1"` | fusion.rs |
| `RRF_K` | 60.0 | fusion.rs |
| `PAYLOAD_MAGIC` | `b"ETB1"` | tombstone/wire.rs |
| `DIGEST_DOMAIN` | `b"calyx-ledger-erasure-tombstone-subject-v1"` | tombstone.rs |

### 11.2 Error conditions

All error codes are `CalyxError` constructors from `calyx-core`
(`crates/calyx-core/src/error.rs`).

| Code | Raised when |
|---|---|
| `CALYX_LEDGER_CORRUPT` | Bad codec bytes, hash mismatch on `decode`, invalid tags, seq mismatch, JSON/hex parse failures, divergent overlay bytes, missing/invalid checkpoint payload, fusion vector shape mismatch, tombstone payload mismatch. |
| `CALYX_LEDGER_CHAIN_BROKEN` | Seq gap, tip changed (`verify_tip`), prev_hash mismatch in recovery, prepared/next mismatch, seq/ts exhaustion, quarantined range queried, empty quarantine range, tombstone seq mismatch. |
| `CALYX_LEDGER_APPEND_ONLY_VIOLATION` | `put_new` over existing seq, `reject_delete`, `reject_tombstone`, overlay store write. |
| `CALYX_LEDGER_SECRET_IN_PAYLOAD` | Secret field name or long token in payload. |
| `CALYX_LEDGER_ACTOR_TOO_LONG` | Actor id > 64 UTF-8 bytes. |
| `CALYX_LEDGER_GROUP_COMMIT_FAILED` | Direct `on_commit` called; stage/commit failure wrapped. |
| `CALYX_LENS_FROZEN_VIOLATION` | Registry frozen weights hash ≠ recorded `weights_sha256`. |
| `CALYX_REPRODUCE_NONDETERMINISTIC` | Measure payload missing `forge_seed`. |
| `CALYX_REPRODUCE_DRIFT_EXCEEDED` | `assert_reproduced` on a non-reproduced verdict. |
| `CALYX_DISK_PRESSURE` | `DirectoryLedgerStore` filesystem errors (create/read/write/sync). |

---

## 12. Cross-references

- [05_aster_storage.md](05_aster_storage.md) — the `ledger` CF host, durable
  group-commit batch, and the `ledger_key` byte format that `ledger_batch_key`
  must match.
- [12_ward_guard.md](12_ward_guard.md) — Guard decisions recorded as `Guard`
  entries and guard-layer erasure enforcement behind erasure tombstones.
