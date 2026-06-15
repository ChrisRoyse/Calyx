# PH53 · T05 — Blob layer: chunked payload + manifest

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/layers/blob.rs` (≤500) |
| **Depends on** | T01, T02 (Layer trait) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/04 §2/§3`, `dbprdplans/20 §2` |

## Goal

Implement the blob paradigm layer for storing and streaming large payloads.
Payloads are split into fixed-size chunks (each a CF row); a manifest row
records the chunk count, total byte count, content hash, cold-tier flag, and
v2 creation timestamp used by PH58 retention.
The manifest is written **last**, in its own WAL flush after all chunk rows
are durable — so a partial-write failure leaves no live manifest, and a
subsequent `get_blob` will see no blob (not corrupt data). Cold-tier sidecar
on ZFS archive is flagged in the manifest; physical cold offload in PH11.

## Build (checklist of concrete, code-level steps)

- [ ] Define blob key schema (discriminant `0x05`):
  ```
  chunk_key    = 0x05 | 0x00 | collection_id (8B BE) | blob_id (16B) | chunk_idx (u32 BE)
  manifest_key = 0x05 | 0x01 | collection_id (8B BE) | blob_id (16B)
  chunk_value  = raw bytes (≤256 KiB per chunk)
  manifest_val_v2 = total_bytes (u64 BE) | chunk_count (u32 BE) | content_hash (32B blake3) | cold_tier (u8 bool) | created_at_ms (u64 BE)
  ```
  v1 manifests without `created_at_ms` remain readable and are skipped by
  TTL-based blob retention rather than guessed.
- [ ] Define `BLOB_CHUNK_SIZE: usize = 262_144` (256 KiB). Configurable per
  vault but immutable after first write.
- [ ] Implement `blob_put(col: &Collection, blob_id: BlobId, data: &[u8]) -> Result<()>`:
  - Split `data` into `ceil(len / BLOB_CHUNK_SIZE)` chunks.
  - Write all chunk rows in group-commit WAL batch; fsync.
  - Compute `content_hash = blake3(data)`.
  - Write manifest row in a **second** WAL batch (ensures chunks are durable
    before manifest is visible); Ledger stub in this batch.
  - Fail closed with `CALYX_BLOB_TOO_LARGE` if `data.len() > 1 GiB`.
- [ ] Implement `blob_get(col: &Collection, blob_id: BlobId) -> Result<Option<Vec<u8>>>`:
  - Read manifest; if absent → `None`.
  - Read all `chunk_count` chunk rows; concatenate.
  - Verify `blake3(result) == manifest.content_hash`; on mismatch →
    `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] Implement `blob_delete(col: &Collection, blob_id: BlobId) -> Result<()>`:
  - Write tombstones for all chunk rows and manifest in one WAL batch.
- [ ] Implement `blob_stream_chunks(col: &Collection, blob_id: BlobId) -> impl Iterator<Item=Result<Vec<u8>>>`:
  - Lazy chunk iterator for large blobs without full in-memory load.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `blob_put(id=b1, data=b"hello world" * 10)` → `blob_get(b1)` returns
  exact bytes; `blake3` matches.
- [ ] unit: payload spanning 3 chunks (size = `BLOB_CHUNK_SIZE * 2 + 1`) →
  chunk count = 3 in manifest; `blob_get` reassembles correctly.
- [ ] proptest: `blob_get(blob_put(id, data)) == Some(data)` for arbitrary
  `data` up to 2 MiB.
- [ ] edge (≥3): (1) empty payload → 0 chunks, manifest with `total_bytes=0`;
  `blob_get` returns `Some(b"")`; (2) `blob_get` on absent `blob_id` → `None`;
  (3) flip one byte in a chunk SST row → `CALYX_ASTER_CORRUPT_SHARD` on `blob_get`;
  (4) `blob_put` > 1 GiB → `CALYX_BLOB_TOO_LARGE`.
- [ ] fail-closed: manifest absent (simulated partial write) → `blob_get` returns
  `None`, not partial data.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/blob/` SST shard (chunk rows + manifest rows).
- **Readback:**
  ```
  dd if=/dev/urandom of=/tmp/testblob bs=1M count=2
  calyx blob put  --vault /home/croyse/calyx/test-vault --collection blobs --id b1 --file /tmp/testblob
  calyx blob get  --vault /home/croyse/calyx/test-vault --collection blobs --id b1 --out /tmp/out
  cmp /tmp/testblob /tmp/out && echo "blob round-trip OK"
  xxd /home/croyse/calyx/test-vault/cf/blob/000001.sst | head -8
  ```
- **Prove:** `cmp` exits 0 (byte-exact round-trip); `xxd` shows `0x05 | 0x00`
  chunk rows and `0x05 | 0x01` manifest row; manifest `content_hash` matches
  `b3sum /tmp/testblob`. Evidence posted to PH53 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
