# PH16 · T03 — ε-greedy / Thompson explorer + promotion gate

| Field | Value |
|---|---|
| **Phase** | PH16 — Autotune Config Cache |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/autotune/explorer.rs` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/13 §7` |

## Goal

Implement the exploration policy that drives the autotune cache: at exploration
time, pick a candidate `BestConfig` using ε-greedy (exploit best known, explore
random with probability ε=0.1) or Thompson sampling (Beta-distribution posterior
over candidate win rates). Promote a candidate only when it shows a **measured**
win (not just a single lucky trial) — require it to beat the incumbent on ≥3
trials by a margin > 2%.

## Build (checklist of concrete, code-level steps)

- [x] `pub const EPSILON: f64 = 0.1;` and `pub const MIN_PROMOTE_MARGIN: f64 = 0.02;`
  and `pub const MIN_PROMOTE_TRIALS: u32 = 3;` — all constants, not magic numbers
- [x] `pub enum ExplorerPolicy { EpsilonGreedy, Thompson }`
- [x] `pub struct Explorer { policy: ExplorerPolicy, rng: ChaCha8Rng, candidate_stats: HashMap<AutotuneKey, Vec<BenchResult>> }`
- [x] `pub fn next_candidate(explorer: &mut Explorer, key: &AutotuneKey, incumbent: &BestConfig, candidate_pool: &[BestConfig]) -> BestConfig`
  — ε-greedy: with probability `EPSILON` pick a random candidate from `candidate_pool`;
  otherwise return `incumbent.clone()`;
  Thompson: model each candidate as a Beta distribution parameterized by
  `(wins+1, losses+1)` where wins/losses are counted from `candidate_stats`;
  sample each and pick the argmax
- [x] `pub fn record_trial(explorer: &mut Explorer, key: &AutotuneKey, config: &BestConfig, result: BenchResult)`
  — append `result` to `candidate_stats[key]`
- [x] `pub fn should_promote(explorer: &Explorer, key: &AutotuneKey, challenger: &BestConfig, incumbent: &BestConfig) -> bool`
  — requires at least `MIN_PROMOTE_TRIALS` trials for the challenger; checks that
  the challenger's mean GFLOP/s exceeds incumbent's mean by > `MIN_PROMOTE_MARGIN` (2%);
  returns `false` if either has fewer than `MIN_PROMOTE_TRIALS` results
- [x] `pub fn promote_if_winner(explorer: &mut Explorer, cache: &mut AutotuneCache, key: AutotuneKey, challenger: BestConfig, incumbent: BestConfig, clock: &dyn CalyxClock) -> Option<BestConfig>`
  — if `should_promote` → insert challenger into cache, return `Some(old incumbent)` (for rollback);
  else return `None`; uses `clock.now()` for the promotion timestamp (not `SystemTime::now()`)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `should_promote` with 3 trials where challenger beats incumbent by 3% → `true`
- [x] unit: `should_promote` with only 2 trials → `false` (not enough trials)
- [x] unit: `should_promote` with 3 trials where challenger beats by 1% → `false` (< 2% margin)
- [x] proptest: ε-greedy with ε=0.1 and pool of 10 candidates; over 1000 calls,
  exploitation fraction ≈ 90% (within ±5% — seeded RNG so exact count is deterministic)
- [x] edge (≥3): (1) empty candidate pool → return incumbent; (2) single candidate =
  incumbent → return incumbent; (3) Thompson with all candidates equal → random pick
- [x] fail-closed: `promote_if_winner` with `clock` that returns a fixed instant →
  the returned `BestConfig` is the old incumbent (enabling rollback)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `autotune_tests::epsilon_greedy_exploit_fraction` on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge autotune::explorer -- --nocapture 2>&1 \
    | grep -E "exploit_fraction|promote|PASSED|FAILED"
  ```
- **Prove:** `epsilon_greedy_exploit_fraction` PASSED printing `exploit_fraction=0.8XX`
  to `0.9XX` (within ±5% of 0.90); `should_promote` tests PASSED; absent: any
  promotion with < 3 trials

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached to PH16 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
