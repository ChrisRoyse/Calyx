# PH09 · T03 — Idempotent ingest + Absent slot handling

| Field | Value |
|---|---|
| **Phase** | PH09 — Constellation CRUD + CxId + idempotent ingest |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault.rs` (≤500) |
| **Depends on** | T02 (WAL-integrated write), T01 (binary encoding) |
| **Axioms** | A1, A15 |
| **PRD** | `dbprdplans/04 §5`, `dbprdplans/03 §3` |

## Goal

Prove — on disk — that re-ingesting the same input bytes and panel version is
idempotent: the WAL does not grow, the SST does not change, the same CxId is
returned, and the vault seq does not advance. Also prove that explicit `Absent`
slots are stored correctly in the `slot_*` CF and round-trip through get. This
is the dedup short-circuit from PRD `04 §5`: `cx_id = blake3(input ‖
panel_version ‖ salt)[0..16]; if exists → idempotent return`.

## Build (checklist of concrete, code-level steps)

- [x] In `AsterVault::put`, the dedup check must read from `CfRouter::get(Base,
  base_key(cx_id))` (disk), not only from the in-memory version chain. This
  handles cold-open dedup (vault was restarted between two identical ingest calls).
- [x] On dedup hit: compare `ConstellationHeader` decoded from the stored bytes to
  the incoming constellation; if they match (same ingest identity — same cx_id,
  panel_version, modality, input_hash), return `Ok(cx_id)` without touching the
  WAL or MVCC seq.
- [x] On hash collision (same CxId, different ingest identity): return
  `CALYX_ASTER_CORRUPT_SHARD` (unchanged from current behavior).
- [x] Write a test: `put(cx)` where `cx.slots` contains an `Absent { reason:
  LensUnavailable }` for slot 3; `get` returns `Absent { reason:
  LensUnavailable }` for slot 3 from disk (not None).
- [x] Write a test: idempotent re-ingest reads from disk: put → flush → new vault
  (cold open) → put same bytes again → returns same CxId; WAL has exactly 1
  record; SST unchanged.
- [x] Write a test: idempotent re-ingest without flush (dedup from in-memory
  cache): put → put same → seq unchanged.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `Absent` slot round-trips through flush + cold-open + `get`.
- [x] unit: cold-open idempotent ingest: WAL record count unchanged on second put.
- [x] unit: warm (in-memory) idempotent ingest: seq unchanged.
- [x] proptest: for any constellation, `put(cx); put(cx)` → seq after second put
  == seq after first put.
- [x] edge (≥3): (1) all slots Absent → constellation still stored and retrieved;
  (2) hot-add a real slot value after Absent in a second put → not idempotent
  (seq advances, new row written); (3) same CxId, different modality → collision →
  `CALYX_ASTER_CORRUPT_SHARD`.
- [x] fail-closed: `CALYX_ASTER_CORRUPT_SHARD` on hash collision.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment byte count before and after second ingest of same input.
- **Readback:**
  ```
  wc -c /home/croyse/calyx/test-vault/wal/00000000000000000000.wal
  calyx ingest --vault /home/croyse/calyx/test-vault --input "hello world"
  calyx ingest --vault /home/croyse/calyx/test-vault --input "hello world"
  wc -c /home/croyse/calyx/test-vault/wal/00000000000000000000.wal
  ```
- **Prove:** WAL byte count is identical before and after the second ingest (no
  new WAL record written). `calyx readback --cf base` shows exactly 1 row for
  the input. SST file size unchanged. Screenshot posted to PH09 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH09 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
