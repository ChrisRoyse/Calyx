# PH71 · T02 — Dual-write ingest + recall-parity comparator (V0 gate)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/dual_write.rs` (≤500), `crates/calyx-cli/src/leapable/recall_comparator.rs` (≤500) |
| **Depends on** | T01 (shadow harness), PH64 (migration tool / 1-slot constellation writer) |
| **Axioms** | A18, A15, A4, A16 |
| **PRD** | `dbprdplans/15 §5 V0`, `15 §4` |

## Goal

Implement the dual-write ingest path and the recall-parity comparator that together
constitute the **V0 FSV gate**: ingest writes every chunk to both `sqlite-vec` and
Calyx atomically; Ask reads `sqlite-vec` (authoritative in V0) and compares the
Calyx result; the gate passes when Calyx recall ≥ baseline (`sqlite-vec` recall) on
a real Vault, proven by byte readback. Neither the comparator log nor any other
structure ever persists candidate text (PRD `15 §4`).

## Build (checklist of concrete, code-level steps)

- [ ] `DualWriteIngest::ingest(chunk_id: ChunkId, text_hash: [u8;32], vector:
      &[f32], metadata: &ChunkMeta) -> Result<IngestReceipt, CalyxError>`:
      writes to `sqlite-vec` first (existing path), then to Calyx as a 1-slot
      `Constellation` (reuse PH64's `MigrateWriter`). On Calyx write failure →
      `CALYX_SHADOW_WRITE_FAILED`; SQLite write is NOT rolled back (V0 invariant:
      `sqlite-vec` is authoritative). `IngestReceipt` carries both `sqlite_rowid`
      and `CxId`.
- [ ] Enforce `chunk_id` and `database_name` are passed through verbatim as
      `ConstellationMeta` fields — these are code-contract names (PRD `15 §4`);
      no transformation, no aliasing.
- [ ] Enforce **never-persist-candidate-text**: `DualWriteIngest` accepts only
      `text_hash: [u8;32]` and `vector: &[f32]`; if a caller passes raw text bytes
      to any public fn in this module → compile error (type-level enforcement via
      `TextHash` newtype). Reranker candidate text is request-scoped only.
- [ ] `RecallComparator::compare(query_vec: &[f32], top_k: usize) ->
      Result<ParityReport, CalyxError>`: runs Ask against both `sqlite-vec` and
      Calyx, computes recall@k (intersection / k), returns `ParityReport { sqlite_recall,
      calyx_recall, delta, query_hash: [u8;32] }`. Never stores the query text —
      only the hash.
- [ ] `ParityReport::gate_passes(&self) -> bool`: `self.calyx_recall >=
      self.sqlite_recall` (parity ≥ baseline). Logs WARN if delta < 0; logs INFO if
      delta ≥ 0.05. `gate_passes() == false` → `CALYX_RECALL_PARITY_BELOW_BASELINE`
      with the two recall values in the error body.
- [ ] `DualWriteIngest::batch_ingest(chunks: &[ChunkRecord]) -> Result<Vec<IngestReceipt>, CalyxError>`:
      wraps single-chunk ingest in a loop; partial failure → returns all successes +
      one `CALYX_SHADOW_WRITE_FAILED` per failed chunk (not an all-or-nothing abort,
      since SQLite side must remain authoritative).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: ingest 5 chunks with known `chunk_id` values (`c001`–`c005`), known
      32-byte text hashes, known 4-d test vectors (seed 0xCAFE_BABE) → `IngestReceipt`
      set has both `sqlite_rowid` and distinct `CxId` for each; `CxId` byte-stable
      across re-ingest (idempotent, from PH09 contract).
- [ ] unit: `RecallComparator::compare` on 5-chunk Vault with query vector identical
      to chunk `c003`'s vector → `sqlite_recall` = 1.0, `calyx_recall` = 1.0,
      `gate_passes()` = true.
- [ ] proptest: for any batch of n ∈ [1, 50] chunks (seed deterministic), every
      `IngestReceipt.chunk_id` matches the corresponding input `chunk_id` byte-exact
      (code-contract preservation invariant).
- [ ] edge (≥3):
      (a) Calyx WAL full (injected error) → `CALYX_SHADOW_WRITE_FAILED`; SQLite row
          written and readable;
      (b) query vector all-zeros → comparator returns `CALYX_INVALID_VECTOR` (zero
          norm guard);
      (c) `top_k = 0` → `CALYX_INVALID_TOP_K`.
- [ ] fail-closed: attempt to pass raw `&str` text to `DualWriteIngest` via the
      public API → compile error (enforced by `TextHash` newtype, not a runtime
      assert — confirmed by `cargo check`).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Calyx Vault directory (`vault.calyx/`) on aiwonder after a real
  ingest of a real Leapable Vault's chunks; the `PARITY_REPORT` log file written by
  `RecallComparator`; the `sqlite-vec` `.db` file (authoritative, unmodified by
  Calyx in V0).
- **Readback:**
  ```
  # 1. Ingest a real Vault (use a copy, never the live DB):
  calyx leapable dual-write --sqlite real_vault_copy.db --calyx vault.calyx

  # 2. Byte-readback: confirm Calyx constellations match source SQLite rows:
  calyx readback --vault vault.calyx --verify-against real_vault_copy.db
  # must print: N constellations, all chunk_ids match, all text_hashes match

  # 3. Recall-parity gate:
  calyx leapable recall-compare --sqlite real_vault_copy.db --calyx vault.calyx \
      --queries queries.jsonl
  # must print: calyx_recall >= sqlite_recall for every query; gate=PASS
  ```
- **Prove:** before this card: `vault.calyx/` has 0 constellations (T01 state).
  After: N constellations present (N = chunk count of the real Vault); every
  `chunk_id` in Calyx matches its source SQLite row verbatim; the comparator
  `ParityReport` shows `calyx_recall >= sqlite_recall` on all test queries. The
  `.db` `mtime` must be unchanged (dual-write never modifies the SQLite primary
  path in V0). This is the **V0 FSV gate**.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `calyx readback --verify-against` output + comparator parity
      report (screenshot / log) attached to the PH71 GitHub issue showing V0 gate
      PASS on a real Vault on aiwonder
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `text_hash`-only API confirmed: no raw text in any persistent struct (grep
      `persisted_text` returns 0 hits in this module)
