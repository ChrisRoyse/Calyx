# PH09 — Constellation CRUD + CxId + idempotent ingest

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A1, A15

## Objective

Implement the full vault write/read unit: `put(Constellation)` / `get(CxId, seq)`
/ `anchor(CxId, Anchor)`, content-addressed with blake3, idempotent re-ingest
(same bytes → same CxId → no-op), `Absent` slot handling, and WAL-integrated
group commit with a Ledger stub entry. After PH09, the vault round-trips
constellations to persistent bytes on disk (base + slot_* CFs), survives a vault
process restart, and the WAL is in the write path (not just in tests).

## Dependencies

- **Phases:** PH08 (MVCC+CfRouter disk bridge), PH05 (WAL group-commit batcher),
  PH07 (CF keys), PH04 (Constellation, Anchor, CxId, VaultStore trait)
- **Provides for:** PH10 (manifest captures the durable_seq after each write
  group), PH35 (Ledger real hash-chain replaces stub)

## Current state (build off what exists)

`vault.rs` has `AsterVault<C>` implementing `VaultStore` with `put`, `get`,
`anchor`, `snapshot`. It is well-tested but **entirely in-memory** —
`VersionedCfStore` holds all rows in a `HashMap`, with `serde_json` encoding.
The vault does NOT:
- Wire through the WAL (`GroupCommitBatcher::submit` is not called).
- Persist anything to disk (`CfRouter` is not used).
- Use the CF key binary encodings for storage (uses serde_json).
- Write to the `ledger` CF (writes a `LedgerRef` but the CF path is serde_json
  in the in-memory store).

**What remains:**
- Replace `serde_json` value encoding with the CF-native binary formats (base CF
  value = `ConstellationHeader` packed struct or bincode; slot CF value =
  `ArrowColumnChunk` for dense vectors; anchor CF value = packed `Anchor`).
- Wire `commit_batch` through the `GroupCommitBatcher` (WAL append) before the
  MVCC seq advances.
- Wire `commit_batch` through `CfRouter` (disk persistence).
- Prove idempotent re-ingest by reading from disk (not in-memory cache).
- Write a Ledger stub entry in the `ledger` CF (seq-keyed, placeholder bytes —
  real hash-chain in PH35).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/vault.rs` | `AsterVault` wired to WAL + CfRouter; binary CF encoding; idempotent ingest |
| `src/vault/encode.rs` | Binary pack/unpack for `ConstellationHeader`, `Anchor`, `LedgerRef` |
| `src/vault/ledger_stub.rs` | PH35 Ledger stub: write `seq -> [0u8; 32]` to `ledger` CF in group commit |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Binary CF encoding: ConstellationHeader, Anchor, SlotVector | — |
| T02 | WAL-integrated vault write path | T01, PH05 T03 |
| T03 | Idempotent ingest + Absent slot handling | T02 |
| T04 | Ledger stub entry in group commit | T02 |
| T05 | Vault put/get/anchor FSV (byte-exact on disk) | T02, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Put N constellations; restart the vault process (kill and reopen);
read each back byte-exact:

```
calyx ingest --vault /home/croyse/calyx/test-vault --input "hello world"
calyx readback --cf base --vault /home/croyse/calyx/test-vault
calyx readback --cf slot_00 --vault /home/croyse/calyx/test-vault
xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -4
```

Re-ingest the same input: the output CxId is identical; the SST does not grow.
Anchors land in `anchors` CF. Evidence posted to PH09 GitHub issue.

## Risks / landmines

- `serde_json` encoding produces variable-length bytes and is not byte-stable
  across library versions. Replace with a stable binary format (bincode or a
  hand-written fixed layout) for all values stored in CFs.
- WAL payload size: a `Constellation` with 15 dense slots × 512-d f32 = 30 KB
  raw. With PQ-8 quantization this drops to ~1 KB. For PH09 (pre-quantization),
  the WAL payload may be large; add an assertion that WAL record size ≤ 64 MiB.
- Idempotent dedup check must read from disk (via CfRouter), not from the
  in-memory HashMap, to correctly handle cold-open dedup.
