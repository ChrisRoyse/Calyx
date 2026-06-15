# PH68 · T06 — Anneal autotune of beamwidth/posting-cutoff + 1e8-cx SLO soak FSV

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/autotune.rs` (≤500) |
| **Depends on** | T02 (this phase — DiskAnnSearch, beamwidth param), T03 (this phase — SpannSearch, n_probe/posting-cutoff param), T05 (this phase — KernelFirstSearch, FunnelParams), PH46 (Anneal autotune loops — bandit + tripwire infrastructure) |
| **Axioms** | A14, A16, A32 |
| **PRD** | `dbprdplans/10 §8` (KernelFirst@1e8 p99 < 25 ms), `dbprdplans/27_STAGE17_SCALE.md` (FSV gate), `dbprdplans/10 §2` (Anneal wires beamwidth/posting-cutoff) |

## Goal

Implement `BwPostcutoffTuner`: the Anneal hook that observes per-query p99 latency
and recall@10 for DiskANN beamwidth and SPANN posting-cutoff (n_probe), uses the
Anneal bandit (PH46) to propose incremental adjustments, and fires a tripwire + auto-
revert if recall@10 drops below the floor. Then execute the **definitive FSV soak**:
a 1e8-cx server vault on aiwonder's `hotpool` NVMe, 1000 queries, `KernelFirst`
strategy, measured p99 < 25 ms. This is the PH68 phase exit gate.

> **Scale boundary:** the 1e8-cx soak is a **server-only** FSV run on aiwonder.
> The autotune unit tests use synthetic small vaults. No laptop promise.

## Build (checklist of concrete, code-level steps)

- [ ] Define `TunerObservation { query_latency_us: u64, recall_at_10: f32, beamwidth: usize, posting_cutoff: usize }`: one observation per search call; latency injected via the `Clock` trait (never `SystemTime::now()` in logic)
- [ ] Implement `BwPostcutoffTuner`: registers two Anneal bandit arms for each tunable — `(beamwidth, [min, max, step])` and `(posting_cutoff, [min, max, step])`; `fn observe(&mut self, obs: TunerObservation)` pushes to a sliding window (last 512 observations); `fn maybe_adjust(&mut self) -> Option<TunerAdjustment>` returns a proposed `(beamwidth, posting_cutoff)` when the window is full and the bandit has a confident direction
- [ ] Tripwire (A14): if any proposed adjustment causes `recall_at_10 < RECALL_FLOOR` (configurable, default `0.85`), immediately revert both params to the pre-adjustment values; write a Ledger entry: `{ event: "diskann_tuner_revert", reason: "recall_below_floor", old_bw, new_bw, recall_observed }` (Ledger stub until PH35, real after)
- [ ] Anti-oscillation: a param direction change within `HYSTERESIS_WINDOW` (default 50 observations) is blocked; prevents beamwidth thrashing under bursty load
- [ ] Expose `fn register_with_anneal(tuner: BwPostcutoffTuner, anneal: &mut AnnealEngine)` that wires the observer callback into the Anneal autotune loop from PH46
- [ ] Criterion benchmark `bench_diskann_1e6`: builds a 1e6-cx vault in a temp dir, runs 100 search queries with beamwidth=64, measures wall latency, reports p50/p99/p999; used as a fast regression check before the full 1e8 FSV soak
- [ ] Test helper `build_synthetic_vault(n_cx: usize, dim: usize, n_slots: usize, seed: u64, vault_path: &Path)`: reused across T01–T06 tests; deterministic; ≤200 lines, placed in `crates/calyx-sextant/src/index/testutil.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `BwPostcutoffTuner` with injected clock — push 512 observations with `recall_at_10=0.92`, all latencies within SLO; `maybe_adjust()` returns `None` or a non-regressing adjustment; assert no Ledger revert event
- [ ] unit: tripwire fires — inject 512 observations where the last 50 have `recall_at_10=0.80` (below `RECALL_FLOOR=0.85`); assert `maybe_adjust()` returns a revert (params reset to pre-drop values) and a Ledger entry is emitted with `event: "diskann_tuner_revert"`
- [ ] unit: anti-oscillation — alternate beamwidth direction every 20 observations; assert `maybe_adjust()` never recommends a direction flip within the 50-observation hysteresis window
- [ ] unit: criterion `bench_diskann_1e6` runs without panic; p99 is a finite positive duration (don't assert specific numbers — CI is aiwonder only)
- [ ] proptest: for any sliding window of 512 observations with `recall_at_10 ∈ [0.9, 1.0]`, tuner never fires the revert tripwire (seed `77u64`)
- [ ] edge: tuner with 0 observations → `maybe_adjust()` returns `None`
- [ ] edge: `posting_cutoff` proposed below `min` → clamped to `min`, not `CALYX_*` error
- [ ] fail-closed: Anneal engine unavailable (None handle) → `BwPostcutoffTuner` operates in standalone mode (adjustments recorded but not applied); emits `CALYX_ANNEAL_UNAVAILABLE` warning log, does not panic

## FSV (read the bytes on aiwonder — the truth gate)

This card owns the **PH68 phase exit gate** — the definitive billion-scale SLO proof.

- **SoT:** live p99 latency measurement on a 1e8-cx server vault on `hotpool` NVMe
  (`/zfs/hot/calyx/ph68-1e8/`) and Ledger autotune entries
- **Readback (soak):**
  ```bash
  # Step 1: build the 1e8-cx vault (one-time, may take hours on aiwonder)
  calyx build-bench-vault \
    --vault /zfs/hot/calyx/ph68-1e8 \
    --n-cx 100000000 \
    --dim 512 \
    --slots 6 \
    --seed 42

  # Step 2: verify disk layout
  ls -lh /zfs/hot/calyx/ph68-1e8/idx/slot_00.ann/graph.cda
  ls -lh /zfs/hot/calyx/ph68-1e8/idx/slot_00.sparse/centroids.spn

  # Step 3: run the soak — 1000 queries, KernelFirst, measure p99
  calyx bench search \
    --vault /zfs/hot/calyx/ph68-1e8 \
    --strategy KernelFirst \
    --n 1000 \
    --report p50,p99,p999 \
    --seed 42

  # Required output: p99 < 25000 µs (25 ms) — the KernelFirst@1e8 SLO

  # Step 4: verify autotune logged at least one event
  calyx anneal status \
    --vault /zfs/hot/calyx/ph68-1e8 \
    --tuner bw_postcutoff
  # Must show: current beamwidth, current posting_cutoff, ≥1 Ledger event
  ```
- **Prove (all required):**
  1. `ls` shows `graph.cda` and `centroids.spn` are non-zero — DiskANN graph +
     SPANN lists physically on disk (`04 §3`).
  2. `calyx bench search` p99 result < 25 ms — `KernelFirst@1e8 p99 < 25 ms` SLO
     (`10 §8`) met on aiwonder hardware.
  3. `calyx anneal status` shows at least one autotune Ledger event — Anneal
     actually ran the tuner during the soak.
  4. No recall regression: `calyx bench recall --vault ph68-1e8 --n 200 --k 10`
     prints `recall@10 ≥ 0.85` — tripwire floor not breached post-autotune.
  5. Full output (bench report + anneal status + recall) attached as screenshot /
     text to the PH68 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] **1e8-cx soak on aiwonder: p99 < 25 ms** — the `KernelFirst@1e8` SLO (`10 §8`) met
- [ ] DiskANN graph + SPANN lists physically on disk at correct paths (verified with `ls` + `xxd`)
- [ ] Anneal autotune logged ≥1 Ledger event during soak; no oscillation; no recall revert
- [ ] FSV evidence (full bench output + anneal status + recall measurement + ls/xxd) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
