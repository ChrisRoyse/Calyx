# PH35 — Hash-chain append-only CF (in group-commit)

**Stage:** S7 — Ledger Provenance  ·  **Crate:** `calyx-ledger`  ·
**PRD roadmap:** P7  ·  **Axioms:** A15, A16

## Objective

Build an append-only, hash-chained `ledger` column family whose entries are
written in the same WAL group-commit as the data mutations they describe. Every
signal in Calyx — ingest, measure, assay, kernel, guard, answer, anneal,
migrate, admin, erase — produces a chained entry so provenance can never be
lost on crash and can never be retroactively forged. This is the foundational
"conscience" of the living system (A31 / `11 §1`).

## Dependencies

- **Phases:** PH09 (constellation CRUD + write path — group-commit hooks),
  PH05 (WAL group-commit + fsync), PH07 (CF key codecs — `ledger_key` +
  `ledger_range` already exist in `calyx-aster/src/cf/key.rs`),
  PH04 (`calyx-core` structs — `LedgerRef` already defined in
  `calyx-core/src/model/signal.rs`)
- **Provides for:** PH36 (Merkle + verify_chain + reproduce), PH61
  (crypto-shred requires Ledger `Erase` kind), PH67 (DR restore chain-checks)

## Current state (build off what exists)

`calyx-ledger` now has PH35 T01-T02 implemented: `EntryKind`, `LedgerEntry`,
`SubjectId`, `ActorId`, deterministic `entry_hash`, deterministic binary
`encode`/`decode`/`decode_header`, and `CALYX_LEDGER_CORRUPT` fail-closed decode
errors. T01 evidence is at
`/home/croyse/calyx/data/fsv-issue242-ledger-entry-20260608`; T02 evidence is at
`/home/croyse/calyx/data/fsv-issue243-ledger-codec-20260608`. The remaining
PH35 tasks build the appender, redaction policy, group-commit hook,
actor/timestamp wiring, and WAL smoke FSV on top of those primitives. The
following scaffolding already exists and must be reused:

- `calyx-core/src/model/signal.rs`: `LedgerRef { seq: u64, hash: [u8; 32] }`
- `calyx-aster/src/cf/key.rs`: `ledger_key(seq: u64) -> Vec<u8>` (big-endian
  `seq`), `ledger_range(start, end) -> KeyRange`
- `calyx-aster/src/cf/family.rs`: `ColumnFamily::Ledger` variant (enumerated
  alongside base/slot/anchors)
- PH09 group-commit path in `calyx-aster`: hook points are the target wiring
  site; PH35 adds the ledger side

The `kind` discriminant set and `entry_hash` formula are implemented; the
append-only enforcement rule remains greenfield until T03.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-ledger/src/entry.rs` | `LedgerEntry` struct; `SubjectId`/`ActorId`; `entry_hash` computation (`blake3`); serde |
| `crates/calyx-ledger/src/codec.rs` | deterministic binary `encode`/`decode`/`decode_header`; fail-closed `CALYX_LEDGER_CORRUPT` parsing |
| `crates/calyx-ledger/src/append.rs` | `LedgerAppender`: seq-counter, `append(entry) -> LedgerRef`, append-only enforcement (no update/delete), tombstone prohibition |
| `crates/calyx-ledger/src/kind.rs` | `EntryKind` enum with all 10 variants; `Display` / serde |
| `crates/calyx-ledger/src/redaction.rs` | `RedactionPolicy`: ensure payloads carry hashes/ids only, never raw secret values; `check_no_secret` validator |
| `crates/calyx-ledger/src/group_commit.rs` | `LedgerGroupCommitHook`: trait impl wiring a ledger append into PH09's group-commit batch so the ledger entry and the data write share one WAL record |
| `crates/calyx-ledger/src/lib.rs` | Crate root; re-exports |
| `crates/calyx-ledger/src/tests/` | Unit + proptest + FSV-support tests (may be split into `entry_tests.rs`, `append_tests.rs`, `group_commit_tests.rs`) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `LedgerEntry` struct + `EntryKind` enum + `entry_hash` | — |
| T02 | Binary codec (encode/decode) round-trip | T01 |
| T03 | `LedgerAppender`: seq counter + append-only enforcement | T02 |
| T04 | Redaction policy: no secrets in payload | T03 |
| T05 | Group-commit hook: ledger entry in same WAL batch as data write | T03 |
| T06 | Actor-stamp + server-stamped monotonic timestamp wiring | T05 |
| T07 | Integration smoke: PH09 constellation write → chained ledger entry in WAL | T05, T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Every constellation write has a corresponding chained ledger entry **in the
same WAL group-commit record** as the data it describes (read the WAL bytes
with `xxd`); the chain links verify (`prev_hash` of entry N matches
`entry_hash` of entry N-1); no entry's payload contains a raw secret value.

Exact readback sequence on aiwonder:
1. `calyx readback --vault <vault> --cf ledger --range 0..10` → prints seq,
   prev_hash, entry_hash for each row; confirm `hash[n] == prev_hash[n+1]`.
2. `xxd <CALYX_HOME>/vault/<vault>/wal/wal-*.bin | grep -A2 "ledger"` →
   confirm ledger entry bytes appear **before** the WAL commit record that
   also contains the base-CF write.
3. `calyx scan --cf ledger --seq 1` → inspect payload JSON; confirm no field
   contains a raw bearer token or secret string.

## Risks / landmines

- **Seq counter must be durable**: the counter must be recovered from the last
  `ledger` CF row on restart, not from in-memory state; losing it breaks the
  chain after a crash.
- **Tombstone prohibition**: LSM compaction in aster must be told the `ledger`
  CF is append-only — tombstones (delete markers) must be rejected at the write
  path, not silently allowed and then missed at read time.
- **Clock injection**: use the `Clock` trait everywhere — never `SystemTime::now()`
  in logic — so tests can inject a deterministic monotonic clock.
- **≤500-line hard limit** per `.rs` file: if `entry.rs` grows (large payload
  variants), split payload types into `payload.rs`.
