# Stage 1 — Aster Storage Core (PH05–PH11)

> **STATUS: ✅ DONE** (2026-06-07, commit `8dcddaa`; FSV-signed-off on aiwonder
> — 87 `calyx-aster` + 6 `calyx-cli` tests green; crash-drill recovered to
> last-acked seq with `CALYX_ASTER_TORN_WAL`; corrupt-shard failed closed with
> `CALYX_ASTER_CORRUPT_SHARD`). Satisfies PRD `CORE` (`19 §5`). Evidence: GitHub
> issue #23; FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.
> All seven Stage-1 follow-ups have since been triaged: **#1, #4, #6, and #7 are
> resolved in code** (commits `75975a9`, `3e6c03d`, plus the
> `CompactionDebt::measure` proptest in `compaction/tests.rs`); the rest are
> explicit deferrals or module-placement cosmetics — see "Stage-1 follow-ups" at
> the bottom. Pre-Lodestar hardening #333 adds SST v2 full-body CRC validation,
> parent-directory fsync after SST rename, manifest immutable-ref hash readback,
> compacted-SST recovery, WAL-authoritative post-append commit semantics, and
> real deadline-based WAL group-commit coalescing. Evidence root:
> `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`.
> Sweep residual #337 adds normal-open torn-tail recovery diagnostics plus a
> core compaction write-amp bound regression; evidence root:
> `/home/croyse/calyx/data/fsv-issue337-aster-durability-residuals-20260608`.

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
- **Post-sweep note.** #333 makes the group-commit batcher wait until the
  configured deadline for near-following submissions instead of draining only
  the queue that was already present.
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
- **Post-sweep note.** #333 writes SST v2 files with a full-body CRC covering
  record, index, and bloom sections; legacy v1 SSTs remain readable, and corrupt
  v2 section bytes fail closed on open.
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

## PH10 — Manifest + atomic swap + crash recovery — ✅ DONE (recovery unified; minor deferrals)
- **Objective.** Atomic manifest pointer; recovery replays WAL past the last
  durable manifest to the last fsync'd record; corrupt base fails closed.
- **Deps.** PH09.
- **Deliverables.** `MANIFEST` + `CURRENT` atomic `rename()`; recovery routine;
  immutable codebook/panel references.
- **Key tasks.** manifest versioning; recovery ordering (manifest→WAL replay);
  corrupt base → fail-closed read. Derived-CF degraded/rebuildable flags are
  explicitly deferred to PH44 self-heal and are not PH10 acceptance criteria.
- **Post-sweep note.** #333 verifies immutable panel/codebook refs by content
  hash while loading a manifest, treats compacted SSTs as durable recovery
  inputs, and preserves WAL-authoritative success if a post-WAL checkpoint
  fails after the durable append. #337 exposes torn-tail diagnostics through
  `AsterVault::recovery_report()` on normal cold open.
- **FSV gate.** crash drill (`kill -9` at several points) → recover **byte-exact
  to last-acked**; flip a base-shard byte → read fails closed
  (`CALYX_ASTER_CORRUPT_SHARD`), points at restore.
- **Axioms/PRD.** A15, A16, `04 §7`.

## PH11 — Compaction + hot/cold tiering — ✅ DONE (durable tiering wired; debt proptest landed)
- **Objective.** Background, snapshot-safe compaction; tiering hot (NVMe) vs
  cold (archive HDD); raw-f32 sidecars cold.
- **Deps.** PH10.
- **Deliverables.** leveled/tiered compaction (throttled, debt-metered),
  tiering policy (active slots hot, `*.raw`/retired/old-panels cold), staging
  inside the destination dataset (avoid `EXDEV`), plus durable/router flush,
  recovery, catalog scan, and compaction output paths routed through
  `VaultOptions::tiering_policy`.
- **Key tasks.** concurrent-read-safe compaction; adaptive cadence hook (Anneal
  later); cold-tier writer to `/zfs/archive/calyx`.
- **Post-sweep note.** #333 makes compacted `compacted-*.sst` files recoverable
  even when original shard files are absent. #337 adds direct compaction report
  coverage that asserts the default write-amp ceiling (`<= 2000` milli) on a
  deterministic two-shard merge.
- **FSV gate.** compaction runs with concurrent reads → no partial reads; cold
  slots physically on archive (verified by path); write-amp ≤ target on a soak.
- **Axioms/PRD.** `04 §6`, `24 §3` (anti-storm), A26.

---

## Stage 1 exit — ✅ achieved
Aster round-trips byte-exact, survives `kill -9` to last-acked, serves
consistent MVCC snapshots, ingests idempotently, and tiers hot/cold — all proven
by reading the persisted bytes on aiwonder. Durable tiering follow-up #295
physically proved hot base/active-slot SSTs under `hot/cf`, inactive-slot and
compacted SSTs under `archive/cf`, and no misplaced inactive-slot files under
the vault root at `/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608`.
This is PRD `CORE` (`19 §5`), satisfied at commit `8dcddaa` plus the #295
post-sweep hardening commit; #333 adds the pre-Lodestar durability hardening
listed above, with aiwonder evidence at
`/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`.

---

## Stage-1 follow-ups (functional gate passed; architectural debt to resolve)

Stage 1 is **functionally complete and FSV-signed-off**. A forensic
code-vs-card audit (2026-06-07) found seven divergences from the PH10/PH11
cards; their current disposition follows. None block downstream stages.

1. ✅ **RESOLVED — recovery paths unified (PH10).** `AsterVault::open` now
   recovers through the manifest-anchored `recover_vault` and calls
   `set_start_seq(recovery.last_recovered_seq)` (`vault/durable.rs::open` →
   `manifest::recover_vault`, `vault.rs` → `set_start_seq`), so WAL replay is
   bounded by the manifest's `durable_seq` instead of replaying the whole log.
   Fixed in commit `75975a9` ("Unify Aster vault manifest recovery"), with new
   `vault/recovery_tests.rs` coverage.
2. **Accepted non-work (cosmetic, PH10).** Recovery logic still lives in
   `manifest/mod.rs::recover_vault` rather than a split-out `manifest/recovery.rs`
   — module placement only; no behavioural impact and no live issue required.
3. **Deferred to PH44 T01/T03/T06 (PH10).** `degraded_rebuildable` still has no
   code path that sets it true on a corrupt derived CF; the degrade/self-heal
   path lands with Anneal self-heal, background rebuild, and corrupt-index FSV.
4. ✅ **RESOLVED — durable / `CfRouter` / `CompactionScheduler` unified
   (PH09/PH10/PH11).** The write/flush/compaction paths are now wired together
   via `vault/compaction_bridge.rs` (`VaultCompactionScheduler`) plus
   `compaction/scan.rs` (`catalog_from_vault_dir` SST discovery) and
   `vault/commit.rs`; durable SSTs are registered in a `CompactionCatalog` and
   the scheduler triggers on debt. Fixed in commit `3e6c03d` ("Resolve Aster
   durable compaction follow-up"), with `vault/compaction_tests.rs` coverage.
   Post-sweep #295 extends this through `VaultOptions::tiering_policy`: durable
   checkpoint SSTs, router flush SSTs, manifest recovery scans, vault catalog
   discovery, one-shot compaction output, and scheduler output all resolve hot
   vs archive roots through the same policy.
5. ✅ **RESOLVED — derived materialized slot-column sidecar (PH06), #341.**
   `sst/arrow.rs` remains the `CXA1` Arrow-compatible dense chunk primitive, and
   #341 plus the post-sweep SoA hardening adds `vault/slot_column.rs` to
   materialize dense `slot_NN` row-CF bytes into a separate dimension-contiguous
   column-major `slot-column.cxa1` plus `slot-column-manifest.json` sidecar.
   Live slot CF values intentionally stay row-encoded via
   `vault/encode.rs::encode_slot_vector`; row-level slot vectors remain the
   CRUD/recovery source of truth for Aster. Evidence root:
   `/home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58`.
6. ✅ **RESOLVED — dedicated ledger stub (PH09).** The PH35 ledger-stub row now
   lives in a dedicated `vault/ledger_stub.rs` (commit `3e6c03d`); the real
   hash-chain still lands in PH35.
7. ✅ **RESOLVED — debt-meter proptest (PH11).** `compaction/tests.rs` contains
   `compaction_debt_matches_scaled_pending_bytes`, a proptest that verifies
   `CompactionDebt::measure` tracks pending bytes, target bytes, and scaled debt
   over generated shard byte sizes.
