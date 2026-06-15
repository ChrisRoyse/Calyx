# PH59 · T02 — Hazards 6–8: MVCC version pile-up, VRAM OOM, heap OOM

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/resource.rs` (≤500, continued) |
| **Depends on** | PH56 (bounded allocators + T07 soak), PH57 (VRAM budgeter), PH58 T01/T02 (long-reader watchdog) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` hazards 6–8 |

## Goal

Drive hazards 6 (MVCC version pile-up from long reader), 7 (VRAM OOM under concurrent TEI),
and 8 (heap OOM under sustained fuzz/soak), read the SoT bytes, prove each mitigation.
These three hazards are the most critical resource hazards — each has killed production
databases. Byte-level FSV on aiwonder is the verdict.

## Build (checklist of concrete, code-level steps)

**Hazard 6 — MVCC version pile-up (long reader):**
- [ ] `fn probe_h6_long_reader(vault: &mut Vault, watchdog: &SnapshotPinWatchdog) -> HazardResult`:
  - Open a reader at seq N; hold for `lease_duration + 100 ms`
  - Ingest 1e4 new constellations at seqs N+1 to N+10000
  - Verify `oldest_pinned_seq_gap ≥ 9999` (pile-up is occurring)
  - Let lease expire; verify `CALYX_READER_LEASE_EXPIRED` returned; verify `oldest_pinned_seq_gap` drops to < 10 after GC
  - Record `gc_bytes_freed_total` delta; verify `disk_free` is flat or improved after GC
  - Record `reader_lease_expired_total` increment (must be exactly 1)

**Hazard 7 — VRAM OOM:**
- [ ] `fn probe_h7_vram_oom(forge: &Forge, budgeter: &VramBudgeter) -> HazardResult`:
  - Set `CALYX_FORGE_VRAM_BUDGET` to 2 GiB
  - Dispatch 20 concurrent 200 MiB operations (total 4 GiB requested, budget 2 GiB)
  - Verify: at least 1 dispatch returns `CALYX_FORGE_VRAM_BUDGET` (not a panic/OOM kill)
  - Verify: `nvidia-smi memory.used` does not exceed `soft_cap + 512 MiB` at any sample
  - Verify: `forge_vram_budget_exceeded_total > 0`
  - Verify: `dmesg | grep -i oom` returns nothing (no OOM kill)

**Hazard 8 — Heap OOM:**
- [ ] `fn probe_h8_heap_oom(vault: &mut Vault) -> HazardResult`:
  - Run the 1e7-op soak (delegate to `soak.rs` T07) with RSS monitoring
  - Inject a burst: 1e5 writes at maximum size (4096-byte values)
  - Verify `CALYX_BACKPRESSURE` fires before RSS exceeds `memtable_cap + cache_cap + arena_cap + 20%`
  - Verify RSS trend from soak is < 1.0 bytes/op (no leak)
  - Read `/proc/self/status VmRSS` max; verify ≤ configured bound

- [ ] Aggregate into `target/ph59_hazards_6_8.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H6: `reader_lease_expired_total == 1`; `oldest_pinned_seq_gap` after GC < 100 (not 10000+); `gc_bytes_freed_total` delta > 0; `disk_free` flat or increased
- [ ] H7: `forge_vram_budget_exceeded_total >= 1`; `nvidia-smi max_memory_used ≤ soft_cap_mib + 512`; zero OOM kills in `dmesg`
- [ ] H8: `rss_trend < 1.0 bytes/op` from soak; `backpressure_events_total >= 1` during burst; `max_rss_bytes ≤ sum_of_caps × 1.2`
- [ ] edge: H6 — no live reader → `oldest_pinned_seq_gap == 0`; GC sweeps immediately; no version pile-up
- [ ] edge: H7 — zero VRAM budget (`CALYX_FORGE_VRAM_BUDGET=0`) → all dispatches return `CALYX_FORGE_VRAM_BUDGET` immediately; no crash
- [ ] fail-closed: H7 — no OOM kill anywhere in `dmesg` for the entire test session (checked at end)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_6_8.json`; `calyx readback` metrics; `sudo dmesg | grep -i oom`; `nvidia-smi --query-gpu=memory.used --format=csv,noheader`
- **Readback:**
  ```
  calyx readback --metric reader_lease_expired_total
  calyx readback --metric oldest_pinned_seq_gap
  calyx readback --metric forge_vram_budget_exceeded_total
  calyx readback --metric backpressure_events_total
  nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader,nounits
  sudo dmesg | grep -i oom
  cat target/ph59_hazards_6_8.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] for h in d))"
  ```
- **Prove:** all three hazards report `passed: true` in JSON; metrics within bounds; no OOM in dmesg. Attach `ph59_hazards_6_8.json` + all readback outputs to the PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
