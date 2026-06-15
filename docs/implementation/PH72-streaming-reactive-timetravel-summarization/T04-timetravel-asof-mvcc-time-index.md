# PH72 · T04 — `as_of(t)` over MVCC time-keyed snapshots (time-index in group-commit)

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/timetravel/mod.rs` (≤500), `crates/calyx-aster/src/timetravel/time_index.rs` (≤500) |
| **Depends on** | PH08 (MVCC sequence numbers + snapshot reads), PH05 (WAL + group-commit) |
| **Axioms** | A15, A16 |
| **PRD** | `17 §8`, `17 §7.2` |

## Goal

Add a `TimeIndex` column family that maps wall-clock milliseconds to MVCC sequence
numbers, written atomically within the same WAL group-commit as each data write.
Expose `as_of(vault, t: Timestamp) -> Result<TimeTravelSnapshot, CalyxError>` which
resolves `t` to the highest seqno ≤ `t` via the time-index and opens an MVCC
snapshot at that seqno. The caller can then read any CF at that snapshot as if it
were `now`. The time-index is the single source of truth for time→seqno mapping;
no secondary scan of WAL entries is needed at query time.

## Build (checklist of concrete, code-level steps)

- [ ] `TimeIndexCf`: a new column family `time_index` with key layout `big_endian_u64(millis_utc) || big_endian_u64(seqno)` → value `0u8` (keys are the data; value is a sentinel); big-endian ordering lets a range scan find `floor(t)` seqno cheaply
- [ ] `TimeIndex::write_entry(batch: &mut WriteBatch, ts_millis: u64, seqno: u64)` — appends the key to the WAL group-commit batch (called from the group-commit path in `vault.rs`); must be called inside the same `WriteBatch` as the data keys, never in a separate commit
- [ ] `TimeIndex::resolve(vault, t_millis: u64) -> Result<u64, CalyxError>` — performs `seek_for_prev(key(t_millis, u64::MAX))` on the `time_index` CF to find the greatest entry with `millis ≤ t_millis`; returns the `seqno` encoded in that key; returns `CALYX_TIMETRAVEL_NO_DATA` if the vault has no entries before `t` (empty result, not a panic)
- [ ] `TimeTravelSnapshot { seqno: u64, resolved_at_millis: u64, vault_ref: &Vault }` — wraps a pinned MVCC snapshot opened at `seqno`; exposes `get_cx(cx_id) -> Result<Constellation, CalyxError>` and `scan_cfs(cf, from, to) -> Iterator` over the snapshot at that seqno
- [ ] `fn as_of(vault: &Vault, t: Timestamp, clock: &dyn Clock) -> Result<TimeTravelSnapshot, CalyxError>` — calls `TimeIndex::resolve`; if resolution succeeds opens `vault.snapshot_at(seqno)`; returns `TimeTravelSnapshot`; injects `clock` for reproducibility in tests
- [ ] Modify `vault.rs` group-commit path: after computing the new `seqno`, call `TimeIndex::write_entry(batch, clock.now_millis(), seqno)` before `batch.write()` — atomicity guaranteed by the single `WriteBatch` (A15)
- [ ] `TimeTravelSnapshot` implements `Drop` that releases the MVCC seqno pin so GC can proceed (no leaked pins, A26)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: ingest constellation C1 at `FakeClock(t=1000ms)`, C2 at `t=2000ms`; `as_of(t=1500ms)` → returns snapshot that contains C1 but NOT C2; `get_cx(C2.id)` on that snapshot → `CALYX_CX_NOT_FOUND`
- [ ] unit: `as_of(t=2000ms)` → snapshot contains both C1 and C2; `get_cx(C1.id)` and `get_cx(C2.id)` both succeed
- [ ] unit: `as_of(t=2001ms)` after C2 is mutated at `t=3000ms` → snapshot returns the pre-mutation bytes for C2 (not the mutated state); assert byte equality against the original ingest bytes
- [ ] proptest: `∀ n_ingests ∈ [1, 20]` with monotonically increasing fake timestamps: `as_of(t_k)` for each `k` returns exactly `k` constellations; the `k+1`th is absent
- [ ] edge: `as_of(t=0ms)` (before any ingest) → `CALYX_TIMETRAVEL_NO_DATA` (not a stale-data silent return)
- [ ] edge: `as_of` on a vault with exactly one ingest at `t=500ms`; query with `t=499ms` → `CALYX_TIMETRAVEL_NO_DATA`; query with `t=500ms` → returns that one constellation
- [ ] edge: `TimeTravelSnapshot` dropped without explicit release → no panic; seqno pin released on drop; subsequent GC call does not error
- [ ] fail-closed: `TimeIndex::resolve` called on a vault with a corrupt `time_index` CF (injected bad key) → `CALYX_STORAGE_CF_CORRUPT` (not a silent wrong-seqno)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `time_index` CF rows on disk; the constellation bytes returned by `as_of` at two different timestamps
- **Readback:** `calyx readback time-index --vault $VAULT_PATH` → prints the `(millis, seqno)` pairs in order; `calyx readback as-of --vault $VAULT_PATH --t-millis <T1>` → prints the constellation list at that time; `xxd` the time-index CF segment to confirm big-endian key layout
- **Prove:** ingest C1 at `t1`, C2 at `t2 > t1`; time-index CF has two entries (verify via `calyx readback time-index`); `as_of(t1)` returns only C1 — the C2 bytes are absent (xxd the returned snapshot output and confirm C2's CxId is not present); `as_of(t2)` returns both — both CxIds present; the time-index entry for `t1` encodes exactly the seqno written for C1's group-commit (byte-compare seqno field)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH72 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
