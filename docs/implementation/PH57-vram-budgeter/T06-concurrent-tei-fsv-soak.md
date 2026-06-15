# PH57 · T06 — Concurrent TEI FSV soak — dispatch over budget → split/queue/fail, p99 holds

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/soak_ph57.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 (all VRAM infrastructure complete) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2`, `13 §5` |

## Goal

Prove on aiwonder — by reading nvidia-smi and the metric bytes — that under concurrent TEI
container load (all 3 TEI instances at :8088/:8089/:8090 running), a Forge dispatch that
exceeds the VRAM budget results in split/queue/`CALYX_FORGE_VRAM_BUDGET` (no silent OOM), and
search p99 latency SLO is maintained. This is the phase FSV gate: nvidia-smi output + latency
series + counter evidence, not a green test harness.

## Build (checklist of concrete, code-level steps)

- [ ] Write `soak_ph57.rs` integration test; at test start verify TEI is responsive on :8088 (HTTP GET `/health`); if not, skip test with a message (test is aiwonder-only)
- [ ] Implement `background_tei_load(n: u32)` — fires `n` concurrent embed requests to :8088/:8089/:8090 (round-robin) using `reqwest::blocking`; measures p50/p99 latency
- [ ] Implement `forge_load(n: u32, bytes_per_dispatch: usize)` — fires `n` Forge dispatches, each requesting `bytes_per_dispatch` of VRAM; collects `Ok/Err` outcomes
- [ ] Set `CALYX_FORGE_VRAM_BUDGET` to a value that forces contention (e.g., 8 GiB when TEI takes ~16 GiB); run 100 concurrent TEI requests + 50 concurrent Forge dispatches of 2 GiB each
- [ ] Assert: at least 1 Forge dispatch returns `CALYX_FORGE_VRAM_BUDGET` or is split (counted via `VramStats::splits_total + failed_total >= 1`)
- [ ] Assert: no `cudaErrorMemoryAllocation` propagates as a panic or `unwrap` — all errors are structured `CALYX_*` codes
- [ ] Collect `nvidia-smi --query-gpu=memory.used,power.draw --format=csv,noheader` every 2 s; serialize to `target/ph57_soak_vram.json`; assert `memory.used` never exceeds 31 GiB (leaving 1 GiB driver headroom)
- [ ] Collect TEI p99 latency; assert p99 < 2× baseline (measured before Forge load starts)
- [ ] Criterion benchmark `bench_admission_overhead` — measure overhead of `AdmissionController::decide` call: < 1 µs per decision

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] soak: `forge_vram_budget_exceeded_total > 0` after the run (verified from dumped `VramStats`)
- [ ] soak: `memory.used` in nvidia-smi series ≤ 31 GiB at all samples (never full OOM territory)
- [ ] soak: TEI p99 ≤ 2× baseline (read from latency series in `ph57_soak_vram.json`)
- [ ] soak: zero panics / unwrap failures (checked via `std::panic::catch_unwind` wrapper in the test harness)
- [ ] soak: `power.draw` ≤ 600 W at all samples (Anneal throttle working)
- [ ] edge: run with only 1 TEI container alive (kill :8089/:8090 in test); budgeter still enforces cap; round-robin degrades gracefully
- [ ] fail-closed: no silent OOM; every over-budget dispatch gets a `CALYX_*` code; `dmesg | grep -i oom` returns nothing after the test

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph57_soak_vram.json` produced on aiwonder containing nvidia-smi time series + latency samples; Prometheus `calyx_forge_vram_budget_exceeded_total`
- **Readback:**
  ```
  cargo test --release --test soak_ph57 -- --nocapture 2>&1 | tee /tmp/ph57_soak.log
  cat target/ph57_soak_vram.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('max_vram_mib=', max(d['memory_used_mib']), 'max_power_w=', max(d['power_draw_w']), 'tei_p99_ms=', d['tei_p99_ms'])"
  calyx readback --metric forge_vram_budget_exceeded_total
  sudo dmesg | grep -i oom
  ```
- **Prove:** `max_vram_mib ≤ 31744` (31 GiB); `forge_vram_budget_exceeded_total ≥ 1`; `dmesg` shows no OOM kill; `tei_p99_ms ≤ 2 × baseline`. Attach `ph57_soak_vram.json` and the readback output to the PH57 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
