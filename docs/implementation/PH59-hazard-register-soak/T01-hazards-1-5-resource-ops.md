# PH59 · T01 — Hazards 1–5: compaction storm, flush stall, tombstone buildup, fsync spike, WAL bloat

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/resource.rs` (≤500, partial) |
| **Depends on** | PH56 (bounded allocators), PH58 (GC reclaimers) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` hazards 1–5 |

## Goal

Drive each of hazards 1–5 from PRD `24 §7`, read the SoT bytes before/after, prove the
mitigation holds. Each hazard probe is a deterministic scenario that triggers the hazard
condition, then verifies the mitigation output via `calyx readback` or direct metric read —
not a green test harness. All five must pass their byte-level FSV on aiwonder.

## Build (checklist of concrete, code-level steps)

**Hazard 1 — Write amplification / compaction storm:**
- [ ] `fn probe_h1_compaction_storm(vault: &mut Vault) -> HazardResult` — run a write-heavy workload at 2× normal rate for 30 s; record `write_amp` metric before and after; verify `write_amp ≤ configured_target` (e.g., 10); verify serving p99 did not breach SLO during compaction; record `compaction_debt` trend (must not grow unbounded)

**Hazard 2 — Memtable flush stall:**
- [ ] `fn probe_h2_flush_stall(vault: &mut Vault) -> HazardResult` — write at the bounded memtable cap rate for 60 s; verify `memtable_used_bytes` stays ≤ `cap`; verify write acks keep flowing (count acks per second, must not drop to 0); heap RSS stays bounded

**Hazard 3 — Tombstone buildup:**
- [ ] `fn probe_h3_tombstone_buildup(vault: &mut Vault) -> HazardResult` — ingest 1e5 CxIds, delete 70% of them; run `CompactionGcReclaimer` 5 times; verify `tombstone_ratio ≤ 0.1` after sweep; record `tombstone_ratio` series

**Hazard 4 — fsync latency spike:**
- [ ] `fn probe_h4_fsync_spike(vault: &mut Vault) -> HazardResult` — inject a slow-disk simulation (use Linux `blkdebug` or throttle via `cgroups` I/O limit); write 100 constellations; verify write acks degrade gracefully (no data loss, acks eventually arrive); verify `fsync_p99_us` metric spikes then recovers; no data loss on readback

**Hazard 5 — WAL bloat:**
- [ ] `fn probe_h5_wal_bloat(vault: &mut Vault) -> HazardResult` — write 1e4 entries without triggering flush (hold flush suppressed); verify `wal_bytes_active` grows; release flush; verify `wal_bytes_active` drops after WAL recycler runs; crash+recover; verify all acked writes present on readback; WAL bounded post-recovery

- [ ] Each `HazardResult` has fields `{ hazard_id: u8, passed: bool, evidence: serde_json::Value }` — serialize to JSON
- [ ] Aggregate into `target/ph59_hazards_1_5.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H1: `write_amp ≤ 10.0` (read from `calyx_write_amp` metric); `serving_p99_during_compaction ≤ serving_p99_baseline × 2`
- [ ] H2: `ack_rate_min > 0` (write acks never fully stall); `memtable_used_bytes max ≤ memtable_cap`
- [ ] H3: `tombstone_ratio_final ≤ 0.1` (read from `calyx_tombstone_ratio` after 5 sweeps)
- [ ] H4: all 100 acked writes readable after slow-disk injection (byte-exact readback); `fsync_p99_us` returns to < 10 ms after throttle lifted
- [ ] H5: crash+recover with `kill -9`; all acked writes present; `wal_bytes_active ≤ 2 × max_segment_size` post-recovery
- [ ] edge: H4 with disk fully stalled (0 MB/s) → acks blocked but no data loss; database does not corrupt; returns when I/O resumes
- [ ] fail-closed: H2 write flood → `CALYX_BACKPRESSURE` fires at least once (counter > 0); no OOM kill

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_1_5.json` produced on aiwonder; Prometheus series for `write_amp`, `tombstone_ratio`, `fsync_p99_us`, `wal_bytes_active`, `memtable_used_bytes`
- **Readback:**
  ```
  calyx readback --metric write_amp
  calyx readback --metric tombstone_ratio
  calyx readback --metric fsync_p99_us
  calyx readback --metric wal_bytes_active
  calyx readback --metric memtable_used_bytes
  cat target/ph59_hazards_1_5.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] for h in d))"
  ```
- **Prove:** `all passed == true` in the JSON; each metric within its bound. Attach `ph59_hazards_1_5.json` + readback output to the PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
