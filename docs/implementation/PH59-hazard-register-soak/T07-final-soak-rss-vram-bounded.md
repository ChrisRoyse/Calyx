# PH59 · T07 — Final 1e7-op soak — RSS/VRAM bounded, no leak, no oscillation

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/soak.rs` (≤500), `crates/calyx-hazard-soak/src/main.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 (all 25 hazards passing) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` (all 25 rows); `24 §8` (observability) |

## Goal

The final integration: run all 25 hazard probes in sequence, then run a 1e7-op soak that
exercises every subsystem simultaneously — writes/reads/queries/GC/compaction/VRAM/anneal —
and prove on aiwonder that RSS and VRAM remain bounded, there is no leak, and no oscillation
occurs. This is the Stage 13 exit gate and the RESOURCE predicate of `BUILD_DONE`. The
`calyx-hazard-soak` binary is the FSV tool; it reads bytes, not green checkmarks.

## Build (checklist of concrete, code-level steps)

**`soak.rs` — the 1e7-op integrated soak:**
- [ ] Implement `run_integrated_soak(n_ops: u64, seed: u64) -> SoakReport` — issues `n_ops` operations sampled by weight: 40% writes, 25% reads, 15% ANN searches, 10% GC ticks, 5% VRAM dispatches, 5% Anneal ticks; uses `SmallRng::seed_from_u64(seed)` for determinism
- [ ] Sample every 5000 ops: `rss_kib` (from `/proc/self/status VmRSS`), `vram_mib` (from `VramBudgeter::stats().allocated_bytes`), `tombstone_ratio` (from `CompactionGcReclaimer`), `wal_bytes_active`, `oldest_pinned_seq_gap`; store in `Vec<SoakSample>`
- [ ] After `n_ops`: compute `rss_trend` (linear regression slope over samples), `vram_trend`, `rss_max`, `vram_max`, `oscillation_detected` (flag if `tombstone_ratio` or `oldest_pinned_seq_gap` oscillates > threshold)
- [ ] Assert in-soak: `rss_trend < 1.0 bytes/op`; `vram_max <= soft_cap_bytes`; `oldest_pinned_seq_gap` never exceeds `max_gap_seqs`
- [ ] Serialize `SoakReport` to `target/ph59_final_soak.json`

**`main.rs` — hazard soak orchestrator:**
- [ ] Implement `main()` that: parses `--all-hazards` flag; runs T01–T06 probes in order; prints pass/fail per hazard; then runs the integrated soak (T07); aggregates into `target/ph59_hazard_results.json` with `hazard_pass_count`, `soak_rss_bounded`, `soak_vram_bounded`, `soak_oscillation_detected`
- [ ] Print a final summary line: `STAGE13 EXIT GATE: hazard_pass_count=25 rss_bounded=true vram_bounded=true oscillation=false`
- [ ] Exit code 0 if all pass; exit code 1 if any fail (for CI-free agent-invoked check)
- [ ] Add `criterion` benchmark `bench_hazard_soak_throughput` for the 1e4-op soak (smaller, fast version) measuring ops/s

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] soak: `rss_trend < 1.0 bytes/op` (from `ph59_final_soak.json`; `trend_bytes_per_op` field)
- [ ] soak: `vram_max_mib ≤ soft_cap_mib` (verified from JSON)
- [ ] soak: `oscillation_detected == false` (tombstone_ratio and seq_gap are monotonically decreasing or flat, not oscillating)
- [ ] soak: `hazard_pass_count == 25` in the orchestrator output (all probes from T01–T06 passed)
- [ ] soak: zero panics / OOM kills (checked via `std::panic::catch_unwind` + `sudo dmesg | grep -c oom == 0`)
- [ ] soak: run time < 30 minutes on aiwonder (practical bound; GC and VRAM operations must not be excessively slow)
- [ ] edge: run with `--seed 0xDEADBEEF` vs `--seed 0xCALYX59` — both complete successfully (seed-independent correctness)
- [ ] criterion: `bench_hazard_soak_throughput` on 1e4 ops shows ≥ 10k ops/s on aiwonder (sanity bound)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_final_soak.json` and `target/ph59_hazard_results.json` produced on aiwonder; `calyx readback` metrics; `sudo dmesg | grep -c oom`
- **Readback:**
  ```
  cargo run --release --bin calyx-hazard-soak -- --all-hazards --seed 0xCALYX59 2>&1 | tee /tmp/ph59_full_run.log
  cat target/ph59_hazard_results.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('hazards_passed:', d['hazard_pass_count'], 'rss_bounded:', d['soak_rss_bounded'], 'vram_bounded:', d['soak_vram_bounded'], 'oscillation:', d['soak_oscillation_detected'])"
  cat target/ph59_final_soak.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('rss_trend:', d['trend_bytes_per_op'], 'vram_max_mib:', d['vram_max_mib'], 'rss_max_mib:', d['rss_max_mib'])"
  sudo dmesg | grep -c oom
  ```
- **Prove:**
  - `hazard_pass_count == 25`
  - `soak_rss_bounded == true` and `trend_bytes_per_op < 1.0`
  - `soak_vram_bounded == true` and `vram_max_mib ≤ soft_cap_mib`
  - `soak_oscillation_detected == false`
  - `dmesg oom count == 0`

  Attach `ph59_hazard_results.json` + `ph59_final_soak.json` + the full `/tmp/ph59_full_run.log` as FSV evidence to the PH59 GitHub issue. This evidence is the RESOURCE predicate of `BUILD_DONE`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
