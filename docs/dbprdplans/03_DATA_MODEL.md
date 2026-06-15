# 03 — Data Model

The logical model. Physical layout is `04`; concrete Rust types are `18`.

## 0. Collections — one container, any model (general data layer, `20 §3`)

A **Vault** holds one or more **Collections**. A Collection behaves as whatever data model its workload needs, and intelligence is opt-in per collection (progressive enhancement, A19):

```
Collection {
  name, mode: Records | Documents | KV | TimeSeries | Blob | Constellations,
  schema: Option<Schema>,          // SchemaFull | SchemaLess
  panel: Option<PanelRef>,         // None/0 lenses = plain store; ≥1 lens = constellation store
  indexes: Vec<SecondaryIndex>,    // btree | inverted | ANN | kernel
  dedup: DedupPolicy,              // Off | Exact | TctCosine{required_slots, tau, action} — A28, set at creation (25)
  temporal: TemporalPolicy,        // E2/E3/E4 retrieval boost (AP-60) + event/recurrence understanding — A27 (25)
  retention, txn_policy, tenant,
}
```

`dedup` and `temporal` are **set at creation** (founder's requirement): e.g. `TctCosine{ action: RecurrenceSeries }` + default `TemporalPolicy` makes the collection automatically capture recurring events compactly with periodic understanding, no application code (`25`).

- **0 lenses** → a plain, fast store: relational table, document store, KV namespace, time-series, blob bucket — key-encoding layers over Aster (FoundationDB-style; `04`, `20`).
- **≥1 lens** → records become **constellations**, gain the full Association Engine (DDA, bits, kernel, `Gτ`, Oracle).
- A single transaction/query can span collections and modes (relational filter → graph hop → vector similarity → aggregate), one pass, one source of truth.

The objects below (Panel, Constellation, etc.) are the structure **inside a Constellations-mode collection**; plain-mode collections store typed records/documents/KV/series/blobs with secondary indexes and the same Ledger provenance.

## 1. Object hierarchy

```
Vault                      // one project's database (any number of collections)
 ├─ Collection(mode)       // behaves as relational/doc/KV/TS/blob/constellations
 ├─ Panel(version)         // (constellation collections) the active set of Slots; versioned
 │   └─ Slot               // one named lens-filled position
 ├─ Lens (frozen)          // registered instrument; many Vaults may share by id
 ├─ Constellation (TCT)    // THE record (constellation mode): one input × Panel
 │   ├─ SlotVector[]       // per-slot dense/sparse measurement
 │   ├─ CrossTerm[]        // derived, lazily materialized (Loom)
 │   ├─ Scalar[]           // typed numeric aggregates
 │   ├─ Anchor[]           // grounded real-outcome labels
 │   └─ Provenance         // Ledger pointer (hash-chained)
 ├─ Kernel(version)        // Lodestar output: the ≈1% grounding set + recall metrics
 ├─ GuardProfile(version)  // Ward: per-slot τ thresholds, calibration provenance
 └─ Index[]                // per-slot ANN, sparse inverted, kernel index, scalar btree
```

## 2. Identifiers

| Entity | ID | Construction |
|---|---|---|
| Vault | `VaultId` | ULID, stable for life of the vault |
| Lens | `LensId` | `blake3(name ‖ weights_sha256 ‖ corpus_hash ‖ output_shape)[..16]` — **content-addressed**: identical lens ⇒ identical id across vaults |
| Slot | `SlotId` | small interned `u16` index into the Panel, + a stable `slot_key: &str` (e.g. `"sem-self"`) |
| Constellation | `CxId` | `blake3(input_bytes ‖ panel_version ‖ vault_salt)` — **content-addressed input**: re-ingesting the same input is idempotent and dedups |
| CrossTerm | `(CxId, SlotId_a, SlotId_b, kind)` | derived key; never stored if `a==b` |
| Anchor | `(CxId, anchor_kind)` | one per outcome axis |
| Kernel | `KernelId` | `(VaultId, panel_version, corpus_shard_hash, ts)` |

Content-addressing is load-bearing: makes ingest idempotent (A15 friendliness), dedups across users in a shared lens, makes provenance hashes stable.

## 3. The Constellation record (logical)

> **Physical form:** a constellation is stored as one **co-located, self-organizing array bundle** — all lens vectors + scalars + anchors + cross-terms + bits + guard + provenance grouped under one `CxId`, structurally invariant to the number of lenses N (add a lens = append one block). Full layout, array math, and compression in `23`.

```
Constellation {
  cx_id: CxId,
  vault_id: VaultId,
  panel_version: u32,
  created_at: Timestamp(UTC),          // monotonic, server-stamped (never client time)
  input_ref: InputRef,                  // hash + optional pointer to raw bytes (may be redacted/absent)
  modality: Modality,                   // text | code | image | audio | video | structured | mixed
  slots: Map<SlotId, SlotVector>,       // dense or sparse, per Slot.shape
  metadata: Map<String, String>,        // verbatim source ids (chunk_id, database_name, ...)
  scalars: Map<ScalarId, f64>,          // BFS depth, churn, coverage Δ, blame age, repo health, ...
  anchors: Vec<Anchor>,                 // grounded outcomes (may be empty → "ungrounded" flag)
  cross_terms: CrossTermPolicy,         // lazy | eager(subset) | none  (Loom decides; see 06)
  provenance: LedgerRef,                // hash-chain entry (see 11)
  flags: { ungrounded, degraded, novel_region, redacted_input },
}
```

### SlotVector
```
SlotVector =
  | Dense  { dim: u32, data: QuantizedOrF32 }      // L2-normalized unless lens declares otherwise
  | Sparse { dim: u32, entries: Vec<(idx:u32, val:f32)> }   // SPLADE/keyword/lexical lenses
  | Multi  { token_dim: u32, tokens: Vec<[f32]> }  // ColBERT/late-interaction lenses (MaxSim)
```
- Dense vectors stored quantized (PQ/Float8/binary) per the Slot's quant policy, raw f32 recoverable within the quant error bound; the quant codebook is a Slot-level artifact (see `13`).
- A missing slot is **explicit** (`SlotState::Absent{reason}`), never a zero vector (A16).

### Anchor (grounding)
```
Anchor {
  kind: AnchorKind,           // test_pass | tie_formed | thumbs | label | reward | speaker_match | style_hold
  value: AnchorValue,         // bool | enum | f64 | one-hot
  source: AnchorSource,       // oracle id (docker test, human label, reality reward)
  observed_at: Timestamp,
  confidence: f32,            // 1.0 for deterministic oracles
}
```
Anchors are the only objects that "touch reality" (A2). Assay computes bits *about* anchors; Lodestar grounds the kernel *at* anchored constellations; Ward calibrates `τ` *against* anchored outcomes.

## 4. Panel & Slot

```
Slot {
  slot_id: SlotId,
  slot_key: String,                  // stable human/agent name, e.g. "want-cause"
  lens_id: LensId,
  shape: SlotShape,                  // Dense(d) | Sparse(d) | Multi(token_d)
  modality: Modality,
  asymmetry: Option<Asymmetry>,      // None | Dual{a_index, b_index}  (cause/effect, paraphrase/context)
  quant: QuantPolicy,                // None | PQ{m,nbits} | Float8 | Binary
  axis: Option<AxisTag>,             // optional grouping (Polis-style 11-axis taxonomy)
  bits_about: Map<AnchorKind, f32>,  // Assay output (signal); refreshed by Anneal
  added_at_panel_version: u32,
  state: Active | Parked | Retired,  // Parked = kept, not searched (low signal); Retired = tombstoned
}

Panel { version: u32, slots: Vec<Slot>, created_at, kernel_ref, guard_ref }
```

- **Asymmetric slots** store two vectors per constellation (e.g. `want-cause` as cause-view and effect-view) and own two ANN indexes; directional queries pick the right one with a boost (absorbed from ContextGraph E5/E8/E10).
- Panel evolution is **append-mostly**: a new lens bumps `panel_version`; old constellations remain valid under their version; backfill is lazy (A5). A Slot is never deleted, only `Retired` with a tombstone, so historical constellations stay interpretable.

## 5. Cross-terms (logical; engine in `06`)

A cross-term is an association-between-associations derived from two slots of the **same** constellation:

| Kind | Definition | Use |
|---|---|---|
| `Concat` | `[v_a ‖ v_b]` (typed, reversible) | a richer joint key for a region |
| `Interaction` | element/blockwise product or low-rank bilinear `v_aᵀ W v_b` | learned interaction signal |
| `Agreement` | `cos(v_a, v_b)` scalar | cross-lens consistency (anomaly/blind-spot detection) |
| `Delta` | `v_a − v_b` (compatible shapes) | directional contrast |

Cross-terms are **derived objects**: identified by `(CxId, a, b, kind)`, never duplicated for `a==b`, materialized only when Loom's policy says the pair carries non-redundant bits (Assay-gated). The combinatorial `C(N,2)` count is the *upper bound* (A8); the *materialized* count is `≪ C(N,2)`, governed by `n_eff`.

## 6. Outcome/reward & online state

Supports self-learning (A14) and the JEPA-style "predict consequences" loop:

```
MistakeLog {                 // append-only; one row per wrong trusted prediction
  cx_id, predicted, observed, anchor_kind, ts, panel_version
}
ReplayBuffer { ... }         // sampled constellations for online head updates
OnlineHeadState { ... }      // small learned heads (predictor, calibrator) — versioned, never frozen-lens
```
Mirror ContextGraph `mistake_log`/`replay_buffer`/`online_head_state`. Frozen lenses never change (A4); only small online heads and indexes adapt.

## 7. Multi-tenancy & isolation

- One **Vault = one tenant boundary.** Cross-vault reads require an explicit grant; default deny (A16).
- Lenses are **shareable by content-id** (A4) — many vaults reference the same `LensId` and its weights/codebook on disk once, but vectors and constellations are per-vault, never mixed (inherits Leapable "never mix vectors across models").
- Redaction: `input_ref` may be hash-only (raw bytes absent/redacted) while slot-vectors persist — supports Leapable's "never persist candidate text" reranker rule and privacy posture.

## 8. Consistency model (summary; detail in `04`/`17`)

- **Single-writer-per-vault, MVCC reads.** A vault's writes are serialized; readers see a consistent snapshot (LSM sequence number / ZFS-friendly). A Vault is single-tenant local state, so single-writer-per-vault is natural and sufficient — it improves on the `sqlite-vec` Vault (concurrent MVCC reads during writes) without PostgreSQL's multi-writer control-plane machinery. (Control plane stays on PostgreSQL; Calyx never needs cross-vault multi-writer transactions.)
- **Durability:** WAL + fsync group-commit; crash recovery replays to last consistent sequence (`04`).
- **Index/derived freshness:** ANN, cross-terms, kernel, guard are *derived*; may lag the base constellation by a bounded, queryable staleness, rebuilt by background tasks (Anneal) — reads declare whether they require fresh-derived or accept bounded-stale.

## 9. What replaces what in Leapable

Calyx replaces **only** the Vault (SQLite) side. The PostgreSQL control plane is untouched.

| Leapable today | Calyx object |
|---|---|
| **Vault** SQLite `chunks` + `sqlite-vec` 768-d vector | Constellation with a 1-slot panel → grows to N-slot |
| Vault-local `provenance` / `audit` rows | Ledger |
| Vault `knowledge_nodes` / KG edges | cross-term `Agreement`/`Delta` graph + entity lens slot |
| Vault reranker confidence | a scalar + a `Gτ` reading |
| `creator_databases`, `queries`, billing, outbox (PostgreSQL central) | **NOT a Calyx object — stays in PostgreSQL, unchanged** |

The migration replaces the Vault file format only ("Vault = local Calyx engine"); the served PostgreSQL control plane pointing at those Vaults is unchanged, doing exactly what it does today.
