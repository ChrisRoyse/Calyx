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

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07, commit 8dcddaa)

Shipped in `calyx-aster`:
- `vault.rs` — `AsterVault` implements `VaultStore`: `put`/`get(seq)`/`anchor`; content-addressed `CxId`; idempotent re-ingest (identical bytes → no-op `Ok(id)`; differing bytes under same CxId → `CALYX_ASTER_CORRUPT_SHARD`); explicit `Absent` slots (no zero-fill).
- `vault/encode.rs` (header + `encode_slot_vector`), `vault/ledger_stub.rs` (fixed-width PH35 placeholder row), `vault/anchor_codec.rs`, `vault/cf_codec.rs`, `vault/cursor.rs` (fail-closed reader), `vault/durable.rs` (WAL-integrated write path), `vault/router_bridge.rs`. CLI: `vault-demo`.

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.

Post-sweep clarification #327: the Stage 1 FSV evidence uses the current
`vault-demo`/readback paths plus direct `AsterVault` byte readbacks. The polished
`calyx ingest` / `calyx anchor` user-facing commands are PH62 interface work and
are not a PH09 implementation blocker.

### Resolved follow-up
- The PH35 Ledger-stub row now lives in `vault/ledger_stub.rs`; the real hash-chain still lands in PH35.

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

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`).

Put N constellations; restart the vault process (kill and reopen);
read each back byte-exact:

The original product-facing command sketch is `calyx ingest` + `calyx readback`;
the implemented Stage 1 proof uses `vault-demo`, `calyx readback`, and `xxd` over
the same vault CF bytes until PH62 expands the full CLI surface.

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
