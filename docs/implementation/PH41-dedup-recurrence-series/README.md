# PH41 — DedupPolicy TctCosine + Recurrence Series + Signature

**Stage:** S9 — Temporal & Dedup  ·  **Crate:** `calyx-aster` / `calyx-loom`  ·
**PRD roadmap:** A28, A29  ·  **Axioms:** A28, A29, A3, A26

## Objective

Implement multi-content-slot TCT cosine-`Gτ` deduplication configurable at vault
creation. `DedupPolicy { Off | Exact | TctCosine { required_slots, tau, action } }`
governs how `ingest_at(input, at: t)` responds when a near-duplicate is detected:
collapse it, link it, or — when `action = RecurrenceSeries` — append a timestamped
occurrence to a recurrence series. The recurrence signature (all content slots
agree + temporal lenses differ) is the detector that fires automatically, routing
the same action at a new time into the series. Dedup operates on content slots
only (temporal lenses E2/E3/E4 are excluded). Constellations with conflicting
anchors MUST NOT be merged. All merges are reversible and Ledger-logged. The
`dedup_audit` function exposes per-slot cosines and the full merge history.

## Dependencies

- **Phases:** PH37 (`Gτ` guard math + `GuardProfile` — provides the per-slot
  cosine gate and calibrated `τ`), PH09 (constellation CRUD + idempotent ingest
  — provides the `ingest` entry point that this phase extends)
- **Provides for:** PH42 (grounded recurrence wiring — needs the recurrence
  series and occurrence count stored here), PH72 (streaming ingest + time-travel
  depend on the recurrence series)

## Current state (build off what exists)

`calyx-aster` has WAL, memtable, SSTable, column families, MVCC, constellation
CRUD, manifest, compaction, and PH41 T01 `DedupPolicy` manifest persistence in
place. #379 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`.
PH41 T02 #380 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`: the
bounded content-slot cosine gate now runs on top of the persisted policy and
prints byte readback through `calyx readback dedup-check`, including fail-closed
runtime validation for calibrated tau and constructor-bypassed empty
`required_slots`.
PH41 T03 #381 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`: the
dedup engine now checks shared anchors before cosine, returns `AnchorConflict`
for opposite `SpeakerMatch`, incompatible `StyleHold`, and exclusive-tag
conflicts, and writes reciprocal `dedup:contested_with:<CxId>` rows through the
durable `online` CF/WAL path. Exact/same-CxId anchor conflicts now fail closed
instead of matching through the exact/self path.
PH41 T04 #382 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`: the Aster
`ingest_at` facade stores caller-provided event time in base rows, interim
recurrence Online CF rows, and Ledger payloads; exact duplicates write Ledger
without a second base row; anchor conflicts store a new candidate plus reciprocal
contested rows; invalid negative event time fails closed with no base/ledger
rows.
PH41 T05 #383 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`: Aster
now owns the dedicated `recurrence` CF, Loom exposes `SeriesStore` as the
facade, recurrence ingests update base `recurrence.frequency` and recurrence
rows in one commit, CLI `readback recurrence-series` reads SST+WAL bytes, and
the FSV fixture proves happy-path 5 occurrences, empty series, max-count rollup,
oversized context fail-closed, and WAL append failure atomicity. The `Gτ` guard
(PH37) is in `calyx-ward`.
PH41 T06 #384 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`:
the signature detector now distinguishes same-action/new-time recurrence from
same-time exact duplicate, routes valid signatures into recurrence occurrence
appends, fails closed on missing temporal signature slots, and records
`recurrence_signature`, `same_action`, and `new_time` in Ledger payloads.
Post-T06 hardening #623 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue623-recurrence-fallback-20260610-1dc61cf`:
the recurrence signature detector now falls back to event-time deltas when the
configured temporal signature slot vectors are absent, so valid same-action
events still append to the recurrence series instead of silently becoming exact
duplicates. Artifact hashes: `dedup-ingest-at-readback.json` BLAKE3
`da862cb17a3a0877f216305fa4a5fb5ee4bdff5f04e2686bb884ca30568b7c45` and
`BLAKE3SUMS.txt` BLAKE3
`325f522e71d67a6ae6e7a94681b532403774b2a0eb0ddad39d631b935e1e134d`.
PH41 T07 #385 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue385-dedup-audit-20260610-cc9f57b`: it adds
Ledger-chain-verified `dedup_audit`, vault/target-bound reversible `dedup_undo`,
restore snapshots in merge Ledger payloads, recurrence tombstone undo, and CLI
readbacks for `dedup-audit`, `dedup-undo`, and `cx-list`.
PH41 T08 #386 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue386-dedup-invariants-20260610-5fdab01`: it
adds the five-fixture dedup invariant readback for near-distinct separation,
anchor-conflict separation, reversible recurrence undo across all three base
rows, temporal-slot exclusion from agreement, and frequency count 10. Artifact
hashes: `dedup-invariants-readback.json` BLAKE3
`f568a21145a811671c79f2cba56b08eee36b6536fa64dbd598ee73d5d527e140` and
`BLAKE3SUMS.txt` BLAKE3
`fdda61062034e8d10c4a99e509166e7338b9bc62d6454d8ed3c66fefea33eb87`.
PH41 public recurrence read API follow-up #578 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue578-periodic-recall-20260610-240de5a`: Loom
now exposes public `recurrence_series`, `periodic_fit`, and `periodic_recall`
read APIs, CLI `readback periodic-recall` reads the recurrence CF/WAL-backed
series surface, public fit input is sorted before cadence estimation, joint
hour/day recall must match an observed bucket, and empty recall queries fail
closed with `CALYX_TEMPORAL_INVALID_PERIOD`. Artifact hashes:
`periodic-recall-readback.json` BLAKE3
`7973b14e446ddd9d1901648d5dd66cf1afac2fbc9a6806b191f4bb0682921c79` and
`BLAKE3SUMS.txt` BLAKE3
`7f4af4acb4f507c5e70afb3128f04692d8673fcbabe8aa552d417a2734a09c4e`.
PH41 concurrency hardening #621 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`:
recurrence writes now take the recurrence lock, and every durable commit takes a
process+OS `durable.commit.lock`, refreshes WAL/MVCC/Ledger state before
staging, and then appends/checkpoints with a monotonic local sequence. This
serializes direct appends, recurrence-policy ingests, dedup undo, anchor updates
touching recurrence metadata, and ordinary durable writes that could otherwise
advance WAL between a stale handle's refresh and commit. The FSV fixture opens
one durable `AsterVault` per worker before the barrier, then separately reads
recurrence/base/WAL bytes plus Ledger `verify-chain`: direct append returns and stores IDs 0..15
with frequency 16, recurrence ingest stores IDs 0..12 with frequency 13, and a
negative event time fails closed before retrying as ID 0.
Artifact hashes: `recurrence-concurrency-readback.json` BLAKE3
`91e0ad19b81589f49591a9ed65ee6efb3c656a82ebc545a27c62820d1cfa96d8` and
`BLAKE3SUMS.txt` BLAKE3
`e1bb5a412ca31e1e8d27d18bd1410ee8c65260389a63bceac078ea01cfd027af`.
PH41 WAL recovery/open serialization #624 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue624-wal-recovery-lock-20260610-1e4b34c`:
WAL replay/open now takes the same `.append.lock` as append before any torn-tail
truncation, while already-locked append refreshes use an internal replay helper.
The FSV fixture holds `.append.lock` over a partial record, proves replay does
not truncate it (`partial_len_before_replay=47`,
`locked_len_after_replay_attempt=47`), completes the record, and then reads two
records (`seq=1` payload `acked`, `seq=2` payload `completed`) from a 54-byte
WAL segment with no torn tail. Artifact hashes: `wal-recovery-lock-readback.json`
BLAKE3 `1c2c255e517691660f8ba45c78b625dd5c4d6eb68b5d7609a69cc8bf2b5bff84`,
WAL segment BLAKE3
`95c91a000e2c7fc7cba16196d7bbda74f7849e7c29d6c66a42b5dc46ac93e5d8`, and
`BLAKE3SUMS.txt` BLAKE3
`81d2d5d6790221315f1cfcbf1331fbc68668bb0b9d4bed26c2befd75d7099c3d`.
PH41 durable dedup policy validation parity #617 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue617-dedup-panel-validation-20260610-07884d9`:
`VaultOptions::panel` now routes supplied and recovered `DedupPolicy` values
through panel-aware temporal/dedup-excluded slot validation before durable
creation or reopen can proceed. The readback proves public creation rejects
required slot 5 with `CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED` and no
`CURRENT`/`MANIFEST` bytes, while a legacy invalid manifest is rejected on cold
open once panel metadata is supplied. Artifact hashes:
`dedup-policy-readback.json` BLAKE3
`9e7636d173dd188b52f3aa232c70fe279e18ad89988a179ec4296e1287ce7423` and
`BLAKE3SUMS.txt` BLAKE3
`8c20d63213e87c210385f69ad8d144d4c81397e433e433e177161222151659d0`.
PH41 WAL write-failure error-code contract #622 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`.
The decision is to keep `CALYX_DISK_PRESSURE` as the stable PRD 18 storage-write
failure code and not add `CALYX_WAL_WRITE_ERROR`. The readback proves the
injected WAL append failure returns `CALYX_DISK_PRESSURE`, leaves base and
Ledger rows byte-identical, leaves recurrence and online CFs empty, and keeps
the snapshot at 1. Artifact hashes: `recurrence-wal-failure-readback.json`
BLAKE3 `7af2b0050766d69d1fad37a896e896766fcf920b9ad510a017171ee1558e24ff`
and `BLAKE3SUMS.txt` BLAKE3
`5c23c502836168d8642cc0ad9bcf839af3a19ca5d8ac3f4e092d896dff6a1506`.
PH41 recurrence rollup tombstone/reclaim integration #620 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue620-recurrence-reclaim-20260610-209f843`.
The readback appends seven occurrences with `max_occurrences=3`, rolls ids 0-3
into a summary, compacts recurrence, reclaims eight input SSTs, prunes
tombstones from the active SST, and cold-reopens with active ids 4/5/6 and
frequency 7. WAL bytes intentionally retain historical recurrence records until
the general WAL recycler. Artifact hashes: `recurrence-reclaim-readback.json`
BLAKE3 `c893925939e3fa0f9c2247c63c85f7eb162f94ce3cd7043f49bdc03b06409710`,
active recurrence SST BLAKE3
`878892e318a277654a835008620eb728f0641403f0a5f934560ed55b26913479`, WAL
segment BLAKE3 `8e6c0e9b295e6d543bcac38657e5952ef137540e2525cadcd3a79d59e8b3f941`,
and `BLAKE3SUMS.txt` BLAKE3
`46daedec8313759540c29130d6fcc880e40fad9e48f83bc98f63a47e62a2e2fe`.
PH41 follow-ups #627 CLI compact recovery-safe naming, #628 dedup undo after
rolled recurrence summary FSV, and #626 anchor-conflict never-merge property
coverage are closed and FSV-backed.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/dedup/mod.rs` | `DedupPolicy`, `DedupAction`, `TctCosineConfig`, `DedupResult` types |
| `crates/calyx-aster/src/dedup/policy.rs` | Policy validation, anchor-conflict check, content-slot selection (excl. E2/E3/E4) |
| `crates/calyx-aster/src/dedup/engine.rs` | `check_dedup(new_cx, vault, policy) -> DedupDecision`; per-slot cosine, required-slots pass logic |
| `crates/calyx-aster/src/dedup/ingest_at.rs` | `ingest_at(vault, input, at: t) -> New(CxId) | DedupMerge{into, occurrence}` |
| `crates/calyx-loom/src/recurrence/mod.rs` | `RecurrenceSeries`, `Occurrence { t_k, context }`, public read API exports, bounded rollup/retention (A26) |
| `crates/calyx-loom/src/recurrence/series_store.rs` | CF-backed store: append occurrence, read series, cadence scalar, periodic recall readback |
| `crates/calyx-loom/src/recurrence/periodic.rs` | Public `recurrence_series`, `periodic_fit`, and `periodic_recall` read APIs |
| `crates/calyx-loom/src/recurrence/signature.rs` | Recurrence signature detector: content-slots-agree + temporal-slots-differ |
| `crates/calyx-aster/src/dedup/audit.rs` | `dedup_audit(vault, cx) -> DedupAuditReport { per_slot_cos, merges, reversible }` |
| `crates/calyx-cli/src/recurrence_readback.rs` | CLI `readback recurrence-series` and `readback periodic-recall` source-of-truth surfaces |
| `crates/calyx-aster/src/dedup/*_tests.rs` | Dedup unit and FSV-style regression tests split by module |
| `crates/calyx-loom/src/recurrence/tests.rs` | All recurrence series FSV tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `DedupPolicy` types + vault-creation config | DONE / FSV #379 |
| T02 | Dedup engine: per-slot cosine gate (content-only, excl. E2/E3/E4) | DONE / FSV #380 |
| T03 | Anchor-conflict guard (MUST NOT merge conflicting anchors) | DONE / FSV #381 |
| T04 | `ingest_at(input, at: t)` → `New | DedupMerge{into, occurrence}` | DONE / FSV #382 |
| T05 | Recurrence series store (one event, many `t_k` occurrences; bounded, A26) | DONE / FSV #383 |
| T06 | Recurrence signature detector (content-agree + temporal-differ) | DONE / FSV #384 |
| T07 | `dedup_audit` (per-slot cos, reversible, Ledger-logged) | DONE / FSV #385 |
| T08 | FSV: near-but-distinct NOT merged; conflicting-anchor stays separate; recurring → series (reversible) | DONE / FSV #386 |

## Closed PH41 follow-ups

#578 public recurrence read APIs, #621 recurrence concurrency hardening, and
#617 durable policy validation parity are implemented and FSV-backed. #622
settled the WAL-failure-code contract as `CALYX_DISK_PRESSURE`, and #620
implements recurrence rollup tombstone/physical reclaim; both are FSV-backed.
CLI compact recovery safety #627, dedup-undo-after-rollup FSV #628, and
anchor-conflict property work #626 are also closed and FSV-backed:

| Issue | Scope |
|---|---|
| #627 | CLI compact must use durable recovery-safe SST naming |
| #628 | Dedup undo after rolled recurrence summary needs dedicated FSV |
| #626 | Anchor-conflict pairs never appear in the same `DedupMerge` property/regression |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

The original five T01-T08 gates passed in #386; #578 adds the public recurrence
read API/readback surface for those bytes:
1. **Near-but-distinct NOT merged:** ingest two constellations with content cosine
   just below calibrated τ → `New` returned for both → two separate CxIds exist in
   CF (`calyx readback cx-list`).
2. **Conflicting-anchor stays separate:** ingest two constellations with identical
   content slots but opposite `SpeakerMatch` anchors → `New` returned for second →
   two separate CxIds; `dedup_audit` shows anchor-conflict-blocked (`xxd` the CF).
3. **Recurring event → one event + time series (reversible):** ingest the same
   constellation 3× at different timestamps → one CxId + `RecurrenceSeries` with 3
   occurrences → `calyx readback recurrence-series <CxId>` shows 3 `t_k` entries →
   call `dedup_audit` → merge history reversible → apply reversal → original 3
   separate CxIds restored byte-for-byte.
4. **Temporal slots excluded from dedup agreement:** content slots above τ still
   merge even when temporal slot cosine is low, and `dedup_audit` reports only
   the content slot in `per_slot_cos`.
5. **Frequency count accurate:** ten same-content recurrence ingests read back as
   `frequency = 10` and `occurrence_count = 10`.

## Risks / landmines

- **Temporal slots must be excluded from dedup agreement:** the `required_slots`
  in `TctCosineConfig` must never include `SlotId`s corresponding to E2/E3/E4. Add
  an explicit filter at policy construction time, not just convention.
- **Anchor-conflict check before cosine check:** check anchor compatibility first;
  if anchors conflict, skip cosine comparison and return `New`. Never compare
  cosines on a pair that will be refused anyway.
- **Bounded recurrence series (A26):** the series store must enforce a max
  occurrence count and a retention window; unbounded growth is a resource hazard.
  Rollup policy (collapse old occurrences into a summary) must be implemented.
- **Reversibility constraint:** every merge must write a Ledger entry containing
  enough information to reconstruct the original constellations. Test reversal
  byte-for-byte before merge.
- **`ingest_at` is the single ingest entry point:** all temporal ingests go through
  `ingest_at`; the existing `ingest` in PH09 becomes a thin wrapper with `at = now`.
