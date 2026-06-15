# PH09 · T01 — Binary CF encoding: ConstellationHeader, Anchor, SlotVector

| Field | Value |
|---|---|
| **Phase** | PH09 — Constellation CRUD + CxId + idempotent ingest |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/encode.rs` (≤500) |
| **Depends on** | PH04 (Constellation, Anchor, SlotVector types) |
| **Axioms** | A1, A15 |
| **PRD** | `dbprdplans/04 §4/§5` |

## Goal

Implement stable, versioned binary codecs for `ConstellationHeader` (the `base` CF
value), `Anchor` (the `anchors` CF value), and the `SlotVector` variants (the
`slot_*` CF values). Replace `serde_json` in the vault write/read path with these
codecs. The encoding must be deterministic and forward-stable: the same struct
always produces the same bytes; a stored byte sequence always decodes to the same
struct.

## Build (checklist of concrete, code-level steps)

- [x] Define `ConstellationHeader`: `cx_id (16B) | vault_id (16B) | panel_version
  (u32 BE) | created_at (u64 BE) | modality (u8) | flags_bits (u8) | n_slots (u16
  BE) | n_anchors (u16 BE) | ledger_seq (u64 BE) | input_hash (32B)` = 102 bytes
  fixed. (Variable-length fields like `scalars` map are encoded as separate CF rows.)
- [x] Implement `encode_header(cx: &Constellation) -> Vec<u8>` and
  `decode_header(bytes: &[u8]) -> Result<ConstellationHeader>`. On decode, fail
  closed with `CALYX_ASTER_CORRUPT_SHARD` if bytes.len() < 102.
- [x] Define `AnchorEncoding`: `kind_tag (u16 BE) | kind_extra (var) |
  value_tag (u8) | value_bytes (var) | source_len (u32 BE) | source_utf8 (var) |
  observed_at (u64 BE) | confidence_bits (u64 BE as f64 raw)`.
- [x] Implement `encode_anchor(anchor: &Anchor) -> Vec<u8>` and
  `decode_anchor(bytes: &[u8]) -> Result<Anchor>`.
- [x] Define `SlotVectorEncoding`: `tag (u8) { 0=Dense, 1=Absent, 2=Sparse }`;
  for Dense: `dim (u32 BE) | data (dim * 4B f32 BE)`; for Absent: `reason (u8)`;
  for Sparse: `n_terms (u32 BE) | [(term_id: u32 BE, weight: f32 BE), ...]`.
- [x] Implement `encode_slot_vector(sv: &SlotVector) -> Vec<u8>` and
  `decode_slot_vector(bytes: &[u8]) -> Result<SlotVector>`.
- [x] Add `encode_ledger_stub(seq: Seq) -> Vec<u8>`: returns `[0u8; 32]` (32 zero
  bytes — PH35 replaces with real hash-chain entry).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `encode_header` on a known `Constellation` produces bytes with
  `cx_id` bytes at offset 0..16 and `panel_version` BE at offset 32..36.
- [x] proptest: `decode_header(encode_header(cx)) == ConstellationHeader { .. }`.
- [x] proptest: `decode_anchor(encode_anchor(a)) == a`.
- [x] proptest: `decode_slot_vector(encode_slot_vector(sv)) == sv` for all three
  variants.
- [x] edge (≥3): (1) `Absent` with each `AbsentReason` variant; (2) `Dense` with
  dim=0 → error or empty data; (3) `Sparse` with 0 terms.
- [x] fail-closed: truncate encoded bytes by 1 → `CALYX_ASTER_CORRUPT_SHARD` on decode.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster vault::encode` on aiwonder.
- **Readback:** `cargo test -p calyx-aster vault -- --nocapture 2>&1 | tail -10`
- **Prove:** Proptest shows ≥100 passing cases for each codec; the unit test
  prints the known golden bytes and asserts them. Screenshot posted to PH09
  GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH09 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
