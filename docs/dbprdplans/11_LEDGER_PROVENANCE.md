# 11 — Ledger: Provenance & Witness

> **Living-system role:** self-knowledge / conscience — an honest, replayable record of everything it did and why (A31 — DOCTRINE §1b)

Implements A15. *"Full provenance tracking"* is a hard requirement, not a feature flag. Ledger makes every signal in Calyx traceable input→lens→vector→cross-term→signal→answer, and tamper-evident.


## 1. What must be provable

| Claim | Evidence Ledger stores |
|---|---|
| "This vector came from this input through this lens" | `(cx_id, slot_id, lens_id, weights_sha256, input_hash, ts)` |
| "This lens is worth 0.07 bits" | `(slot, anchor, bits, ci, estimator, corpus_shard_hash, ts)` |
| "This is the grounding kernel" | `(kernel_id, members_hash, mfvs_approx_factor, recall_ratio, graph_seq)` |
| "This output passed the guard" | `(cx_id, guard_id, per_slot_cos, tau, pass, ts)` |
| "This answer used these constellations" | answer path: ordered `(cx_id, hop, score, lens)` + fusion weights |
| "Nothing was tampered" | hash-chain + periodic Merkle checkpoint |

## 2. Structure (append-only, hash-chained)

```
LedgerEntry {
  seq: u64,                    // monotonic per vault
  prev_hash: Hash,            // chain link
  kind: Ingest | Measure | Assay | Kernel | Guard | Answer | Anneal | Migrate | Admin,
  subject: CxId | LensId | KernelId | GuardId | QueryId,
  payload: typed,             // the evidence row above
  actor: AgentId | ServiceId, // who/what caused it
  ts: Timestamp(UTC),         // server-stamped monotonic
  entry_hash: Hash = blake3(seq ‖ prev_hash ‖ kind ‖ subject ‖ payload ‖ actor ‖ ts),
}
```

- **Append-only**: no update/delete (DB-level trigger equivalent; LSM tombstones forbidden on `ledger` CF).
- **Hash-chain**: each entry binds the previous → any retro-edit breaks the chain.
- **Merkle checkpoint**: periodic root over `[seq_a, seq_b]` (absorbed from ContextGraph `context-graph-witness` `cert_merkle`), optionally signed (Ed25519) for export — reuses Leapable's ingest signing key pattern.

ContextGraph's witness chain + Leapable's audit log, unified as a core CF every write goes through.

## 3. Reproducibility

Lenses are content-addressed and frozen (A4), codebooks/panels immutable, and estimators record their corpus shard + parameters, so **any derived number is recomputable**:

```
reproduce(answer_id) ->
  re-measure inputs with the recorded lens_ids/weights_hashes
  re-run the recorded fusion with the recorded weights
  re-assert the same hits/scores within numerical tolerance
  -> PASS proves the answer was not fabricated; FAIL flags drift/corruption
```

The database analog of Leapable's "verify against bytes": the Ledger lets you *replay* a claim, not just read an assertion.

## 4. Provenance & privacy interplay

- `input_hash` always stored; **raw input bytes are optional** (may be redacted/absent) so Ledger proves lineage without retaining sensitive text (Leapable "never persist candidate text"). A redacted input still has a stable hash → provenance holds, content doesn't leak.
- Cross-vault answer paths record only `cx_id` references the querying vault is granted; default deny (A16).
- Ledger entries themselves are subject to the same redaction policy: payloads carry hashes/ids, not secret values (inherits aiwonder "never write a live secret" rule).

## 5. Audit & query surface

```
get_provenance(cx_id) -> full lineage (ingest → every measure → every signal)
get_answer_trace(answer_id) -> ordered path + fusion + guard + freshness
verify_chain(vault, [seq_a,seq_b]) -> {intact: bool, broken_at?}      // tamper check
merkle_root(vault, range) -> Hash                                      // export/attest
reproduce(answer_id) -> {reproduced: bool, max_drift}                  // replay a claim
audit(filter) -> [LedgerEntry]                                        // who/what/when
```

Filtered audit quarantine contract (#349): explicit `seq_range` overlap and any
matching/relevant result row inside a quarantined range fail closed with
`CALYX_LEDGER_CHAIN_BROKEN`; unrelated quarantined rows outside the filtered
result set do not poison the query. Audit/provenance readers must reject
physical ledger row keys whose sequence does not match the encoded
`LedgerEntry.seq`.

`get_provenance(cx_id)` matches typed `SubjectId::Cx(cx_id)` and explicit cx
payload fields (`cx_id`, `from_id`, `to_id`, `source_cx_id`, `target_cx_id`,
`nearest_cx`, `matched_cx_id`, `query_id`, `anchor_kernel_node_id`). Arbitrary
payload strings such as comments or notes are not provenance references.

## 6. Cost & placement

- Ledger is append-only and compresses well (zstd); hot recent entries on `hotpool`, archived ranges + Merkle checkpoints to `archive` cold tier and into restic.
- Writing a Ledger entry is part of the group-commit (`04 §5`) so provenance is never "added later" and can't be lost on crash — it's in the WAL with the data it describes.

## 7. Ledger API honesty rules

- Every "trusted"/"verified" surface in Calyx MUST be backed by a Ledger entry; a result that can't be traced is tagged `unprovenanced` and MUST NOT be labeled trusted.
- `verify_chain` failure is fail-closed: a broken chain quarantines the affected range and alerts (A16), it does not silently continue.

**One sentence:** Ledger makes Calyx auditable to the byte — every vector, bit, kernel, guard decision, and answer traceable to its grounded source and replayable to prove it was measured, not made up.

Source heritage: ContextGraph `context-graph-witness` (Merkle/sign), Leapable audit-log + Ed25519 ingest signing.
