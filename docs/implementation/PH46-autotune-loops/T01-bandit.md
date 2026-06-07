# PH46 · T01 — Bandit (ε-greedy/Thompson, hysteresis, arm selection)

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/bandit.rs` (≤500) |
| **Depends on** | — (first card; used by all scope tuners T02–T04) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/19 §4` |

## Goal

Implement `ConfigBandit`: a multi-armed bandit over a discrete set of config
candidates for a given `(op, shape, dtype, device, recall_target)` key. Supports
both ε-greedy (explore with probability ε, exploit best-known with `1−ε`) and
Thompson sampling (sample from Beta posterior over each arm's win rate). Hysteresis
prevents oscillation: an arm must win N consecutive A/B trials before being
promoted as the new incumbent. All arm selection is reproducible given a seed.

## Build (checklist of concrete, code-level steps)

- [ ] `enum BanditPolicy { EpsilonGreedy { epsilon: f64 }, Thompson }`.
- [ ] `struct Arm { config: ConfigVariant, wins: u32, trials: u32, consecutive_wins: u32 }` — `ConfigVariant` is an opaque `Vec<u8>` blob (each scope encodes its own config); win rate = `wins / trials.max(1)`.
- [ ] `struct ConfigBandit { policy: BanditPolicy, arms: Vec<Arm>, incumbent_idx: usize, hysteresis_wins: u32, rng_seed: u64 }` — `hysteresis_wins` default `3`.
- [ ] `fn select_arm(&mut self) -> usize` — ε-greedy: with prob `ε` pick a uniform random arm (seeded RNG), else pick arm with highest win rate; Thompson: sample `Beta(wins+1, trials-wins+1)` for each arm, pick argmax; returns arm index.
- [ ] `fn record_result(&mut self, arm_idx: usize, won: bool)` — increments `wins` if won, `trials` always; increments `consecutive_wins` if won (resets to 0 on loss); if `consecutive_wins >= hysteresis_wins`: update `incumbent_idx = arm_idx`, reset all `consecutive_wins`.
- [ ] `fn incumbent(&self) -> &Arm` — returns current best arm.
- [ ] `fn add_arm(&mut self, config: ConfigVariant)` — appends new arm with zero stats; used when a new candidate config is synthesized.
- [ ] Persist `ConfigBandit` state to `anneal_bandit` CF keyed by the shape key hash; reload on restart.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (ε-greedy): with ε=0.0 (pure exploit), `select_arm` always returns incumbent (arm with highest win rate); with ε=1.0 (pure explore), distribution is uniform over arms.
- [ ] unit (hysteresis): arm 1 wins 2 consecutive times (`hysteresis_wins=3`) → incumbent unchanged; wins 3rd time → incumbent = arm 1.
- [ ] unit (Thompson): with all arms at `(1,1)` (uniform Beta), seeded at `42`, `select_arm` returns the same arm across calls with the same seed.
- [ ] proptest: after any sequence of `record_result` calls, `incumbent_idx < arms.len()` (always valid index).
- [ ] edge: single arm → `select_arm` always returns index 0; zero arms → `CALYX_ANNEAL_BANDIT_EMPTY`; `hysteresis_wins=0` → incumbent updates on first win (no hysteresis).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_bandit` CF row for the shape key.
- **Readback:** `calyx anneal bandit-status --key <shape_key>` — prints `incumbent`, `arm_count`, per-arm `win_rate`, `consecutive_wins`.
- **Prove:** run bandit for 50 rounds with one clearly-better arm (synthetic A/B where arm 1 wins 80% of the time, arm 0 wins 20%); after 50 rounds, `incumbent` is arm 1; `bandit-status` shows arm 1 win_rate > arm 0 win_rate; state persists after a simulated restart (reload from CF).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
