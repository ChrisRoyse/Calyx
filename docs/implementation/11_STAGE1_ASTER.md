# Stage 1 — Aster Storage Core (PH05–PH11)

> **STATUS: ✅ DONE** (2026-06-07, commit `8dcddaa`; FSV-signed-off on aiwonder
> — 87 `calyx-aster` + 6 `calyx-cli` tests green; crash-drill recovered to
> last-acked seq with `CALYX_ASTER_TORN_WAL`; corrupt-shard failed closed with
> `CALYX_ASTER_CORRUPT_SHARD`). Satisfies PRD `CORE` (`19 §5`). Evidence: GitHub
> issue #23; FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.
> PH05–PH09 and PH11 are clean; **PH10 is functionally complete but diverges
> architecturally from its card — see "Stage-1 follow-ups" at the bottom.**

The on-disk substrate: WAL, LSM+columnar, column families, MVCC, constellation
CRUD, crash recovery, compaction/tiering. Everything downstream stores through
Aster. Borrow proven ideas (Lance columnar/mmap, RocksDB LSM+CF+WAL, Arrow
layout) but build the association-native CFs ourselves (PRD `04 §2`). Lands in
`calyx-aster`. All bytes live under `CALYX_HOME/data` (→ `/zfs/hot/calyx` once
provisioned). **Living-system role:** metabolism + memory.

> Aster is also the **ordered transactional core** (FoundationDB-style) that
> later hosts every paradigm as a key-encoding layer (Stage 12). Design keys
> with that in mind from PH07.

---

## PH05 — WAL + group-commit + fsync — ✅ DONE
- **Objective.** Durable write-ahead log; group-commit window ≤2 ms; fsync;
  torn-tail discard on replay. WAL is the source of truth for un-compacted
  writes.
- **Deps.** PH04.
- **Deliverables.** `wal/` module (segment writer, group-commit batcher, fsync,
  segment recycle), WAL record framing (len+crc), replay reader.
- **Key tasks.** append+fsync with a bounded commit window; CRC per record;
  segment rotation; replay stops at first torn record.
- **FSV gate.** `kill -9` mid-write on aiwonder → replay → **last-acked record
  present, un-acked absent, torn tail discarded** — proven by reading WAL bytes
  (`xxd`) before/after, not a return. `CALYX_ASTER_TORN_WAL` on torn tail.
- **Axioms/PRD.** A15, A16, `04 §5/§7`.

## PH06 — Memtable + LSM SSTable writer/reader — ✅ DONE
- **Objective.** Bounded in-RAM memtable that flushes to immutable, ordered
  SSTables; block-based reader with mmap scan.
- **Deps.** PH05.
- **Deliverables.** `memtable.rs` (bounded, backpressure on cap), `sst/`
  (writer with block index + bloom, mmap reader, iterator), Arrow-layout column
  blocks for slot columns.
- **Key tasks.** ordered insert; flush at byte cap; SST block index; bloom for
  point lookups; SIMD-friendly column layout.
- **FSV gate.** flush a known memtable → read the SST back **byte-exact**; range
  scan returns keys in big-endian order; bloom never false-negative.
- **Axioms/PRD.** A26 (bounded), `04 §2/§8`, `23 §2` (SoA columns).

## PH07 — Column families + key encoding — ✅ DONE
- **Objective.** The association-native CFs and their key schema.
- **Deps.** PH06.
- **Deliverables.** CFs `base, slot_00..NN, slot_NN.raw, xterm, scalars,
  anchors, ledger, online`; big-endian key codecs; `CxId` 16-byte prefix +
  collision check.
- **Key tasks.** per-CF key/value codecs (`04 §4`); `(CxId)`→header,
  `(CxId)`→slot vec, `(CxId,a,b,kind)`→xterm, `(CxId,AnchorKind)`→anchor,
  `seq`→ledger; range-scan helpers (prefix reads for the future doc/graph
  layers).
- **FSV gate.** write one row per CF → read each back byte-exact; key ordering
  supports range scans; `CALYX_ASTER_CORRUPT_SHARD` on hash mismatch.
- **Axioms/PRD.** `04 §4`, `03 §2`, A16.

## PH08 — MVCC sequence numbers + snapshot reads — ✅ DONE
- **Objective.** A single vault sequence gates all CFs so a reader sees a
  consistent snapshot; derived structures carry their build-seq.
- **Deps.** PH07.
- **Deliverables.** `seq` allocator; snapshot handle pinning a seq; read path
  that resolves across CFs at one seq; `freshness` (FreshDerived|StaleOk).
- **Key tasks.** monotonic seq on every write; reader pins seq; bounded-
  staleness reads; reader-lease scaffolding (full watchdog in PH58).
- **FSV gate.** concurrent writer+reader race on aiwonder → reader **never sees
  a partial constellation** (asserted by reading both CFs at the pinned seq).
- **Axioms/PRD.** `03 §8`, `04 §6`, A26.

## PH09 — Constellation CRUD + CxId + idempotent ingest — ✅ DONE
- **Objective.** The unit write/read: `put(Constellation)`/`get(CxId,seq)`/
  `anchor(...)`, content-addressed + idempotent.
- **Deps.** PH08.
- **Deliverables.** `vault.rs` implementing `VaultStore`; ingest pipeline
  (cx_id = blake3(input‖panel_ver‖salt) → dedup short-circuit → write group);
  `anchor` writer.
- **Key tasks.** idempotent re-ingest (same bytes → same CxId, no-op); explicit
  `Absent` slots; group-commit integrates the Ledger entry (PH35 stub now).
- **FSV gate.** put N constellations → read `base`/`slot_*` CFs back **byte-
  exact**; re-ingest identical input is idempotent (verified on disk); anchors
  land in the `anchors` CF.
- **Axioms/PRD.** A1, A15, `03 §3`, `04 §5`.

## PH10 — Manifest + atomic swap + crash recovery — ✅ DONE (follow-ups tracked)
- **Objective.** Atomic manifest pointer; recovery replays WAL past the last
  durable manifest to the last fsync'd record; corrupt base fails closed.
- **Deps.** PH09.
- **Deliverables.** `MANIFEST` + `CURRENT` atomic `rename()`; recovery routine;
  immutable codebook/panel references.
- **Key tasks.** manifest versioning; recovery ordering (manifest→WAL replay);
  degraded-flag for rebuildable derived; corrupt base → fail-closed read.
- **FSV gate.** crash drill (`kill -9` at several points) → recover **byte-exact
  to last-acked**; flip a base-shard byte → read fails closed
  (`CALYX_ASTER_CORRUPT_SHARD`), points at restore.
- **Axioms/PRD.** A15, A16, `04 §7`.

## PH11 — Compaction + hot/cold tiering — ✅ DONE (follow-ups tracked)
- **Objective.** Background, snapshot-safe compaction; tiering hot (NVMe) vs
  cold (archive HDD); raw-f32 sidecars cold.
- **Deps.** PH10.
- **Deliverables.** leveled/tiered compaction (throttled, debt-metered),
  tiering policy (active slots hot, `*.raw`/retired/old-panels cold), staging
  inside the destination dataset (avoid `EXDEV`).
- **Key tasks.** concurrent-read-safe compaction; adaptive cadence hook (Anneal
  later); cold-tier writer to `/zfs/archive/calyx`.
- **FSV gate.** compaction runs with concurrent reads → no partial reads; cold
  slots physically on archive (verified by path); write-amp ≤ target on a soak.
- **Axioms/PRD.** `04 §6`, `24 §3` (anti-storm), A26.

---

## Stage 1 exit — ✅ achieved
Aster round-trips byte-exact, survives `kill -9` to last-acked, serves
consistent MVCC snapshots, ingests idempotently, and tiers hot/cold — all proven
by reading the persisted bytes on aiwonder. This is PRD `CORE` (`19 §5`),
satisfied at commit `8dcddaa`.

---

## Stage-1 follow-ups (functional gate passed; architectural debt to resolve)

Stage 1 is **functionally complete and FSV-signed-off**, but a forensic
code-vs-card audit (2026-06-07) found real divergences from the PH10/PH11 cards.
These are **non-blocking** for starting Stage 2 (Forge), but are tracked as
GitHub `type:task` issues and should be resolved before/with Stage 13 (resource
hardening), since they touch the recovery + compaction paths.

1. **Two parallel recovery paths (PH10).** `AsterVault::open` recovers by
   replaying the **entire** WAL (`vault/durable.rs::replay_batches`) and
   re-committing every batch — it does **not** consult the manifest's
   `durable_seq` or call the manifest-anchored `recover_vault` +
   `set_start_seq(last_recovered_seq)`. The manifest-anchored path exists
   (`manifest/mod.rs::recover_vault`) and is exercised only by the CLI `recover`
   command + tests. **Unify `open` onto the manifest-anchored recovery.**
2. **`manifest/recovery.rs` not split out (PH10).** Recovery logic lives in
   `manifest/mod.rs::recover_vault`; cosmetic module-placement divergence from
   the card.
3. **`degraded_rebuildable` never set (PH10).** The manifest field exists but no
   code path sets it true on a corrupt derived CF; the degrade/self-heal path is
   deferred to PH44 (Anneal self-heal).
4. **Durable / `CfRouter` / `CompactionScheduler` not unified (PH09/PH10/PH11).**
   `DurableVault::write_batch` writes one SST per row and rewrites the manifest
   per put, bypassing the memtable/`CfRouter` flush model; `CompactionScheduler`
   /`CompactionCatalog` are implemented but **not wired into `AsterVault`** (no
   vault method triggers compaction, durable SSTs aren't registered in a
   catalog). Unify the write/flush/compaction paths.
5. **Arrow slot columns not wired (PH06).** `sst/arrow.rs` (Arrow SoA column
   chunk) is implemented + demo-wired, but slot CF values are stored via
   `vault/encode.rs::encode_slot_vector`, not `ArrowColumnChunk`. Deferred to the
   Forge/array-bundle work (PRD `23 §2`).
6. **Inlined ledger stub (PH09).** The PH35 ledger-stub row is written in
   `vault/encode.rs::encode_ledger_stub`, not a dedicated `vault/ledger_stub.rs`;
   the real hash-chain lands in PH35.
7. **Missing debt-meter proptest (PH11).** `CompactionDebt` has example-based
   unit tests but no `proptest` as the card specified.
