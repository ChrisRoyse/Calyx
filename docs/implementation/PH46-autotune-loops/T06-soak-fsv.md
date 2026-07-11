# PH46 · T06 — 1e6-query soak + FSV (p99 ↓ ≥20%, no recall regression, no oscillation)

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/soak_harness.rs` (≤500) · `crates/calyx-anneal/tests/fsv_soak.rs` (≤500) |
| **Depends on** | T01–T05 (all bandit + scope + A/B infrastructure) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/19 §4` |

## Goal

Implement `SoakHarness` and its corresponding FSV test: drive 1e6 synthetic
queries through the autotune loop on aiwonder and prove the FSV gate — `p99 ↓
≥20%` from baseline, `recall@10 ≥ recall@10_baseline` (no regression), and no
oscillation (p99 series is monotone-improving in the last 10k-query window).
The soak harness runs only in deterministic-seeded mode for explicit simulation.
`LiveTraffic` is reserved for a future independently measured vault-backed replay
provider and fails closed with `CALYX_ANNEAL_SOAK_LIVE_TRAFFIC_UNAVAILABLE` before
any tuner, metric, report, ledger, promotion, or storage mutation.

## Build (checklist of concrete, code-level steps)

- [ ] `struct SoakConfig { n_queries: u64, seed: u64, mode: SoakMode { Seeded, LiveTraffic }, p99_target_reduction: f64, min_recall: f64, oscillation_window: u64 }` — defaults: `n_queries=1_000_000`, `seed=0xABCDEF`, `p99_target_reduction=0.20`, `min_recall=recall@10_baseline`, `oscillation_window=10_000`.
- [ ] `struct SoakHarness { config: SoakConfig, forge_tuner: ForgeScopeTuner, index_tuner: IndexScopeTuner, loom_tuner: LoomScopeTuner, ab_runner: ABRunner, metrics: SoakMetrics }`.
- [ ] `fn run(&mut self, vault: &Vault) -> SoakReport` — drives `n_queries` queries; every query calls `on_op` + `on_search`; every 1000 queries emits a `MetricSample { p99_ns, recall_10, query_count }`; after all queries, computes `SoakReport`.
- [ ] `struct SoakReport { baseline_p99_ns: u64, final_p99_ns: u64, p99_reduction: f64, recall_baseline: f64, recall_final: f64, oscillation_detected: bool, promotions: Vec<ChangeId>, total_queries: u64 }`.
- [ ] `fn check_oscillation(samples: &[MetricSample], window: u64) -> bool` — returns `true` if any p99 in the last `window` queries is higher than the previous-window p99 by more than `5%`.
- [ ] FSV assertion in `fsv_soak.rs`: `report.p99_reduction >= 0.20` AND `report.recall_final >= report.recall_baseline` AND `!report.oscillation_detected`.
- [ ] Soak must complete within a configurable time budget; aiwonder FSV target: ≤2 hours for 1e6 queries.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (seeded): run 1000 queries with a synthetic vault where arm B is 30% faster; after 1000 queries, `p99_reduction ≥ 0.20` and `recall_final ≥ recall_baseline`.
- [ ] unit: `check_oscillation` with a strictly-decreasing p99 series → `false`; with a series that increases by 10% in the last window → `true`.
- [ ] proptest: for any `SoakConfig` with `n_queries ≥ 100` and a clearly-better arm B, `p99_reduction ≥ 0.0` (autotuning never makes things worse than baseline on a seeded corpus).
- [ ] edge: `n_queries=0` → `SoakReport` with all zeros, `oscillation_detected=false`; all arms have same latency → `p99_reduction=0.0` (no regression, soak passes vacuously).
- [ ] fail-closed: vault unavailable mid-soak → `CALYX_ASTER_CF_UNAVAILABLE`; partial `SoakReport` emitted with `total_queries` = number completed; no silent data loss.
- [ ] fail-closed: `LiveTraffic` without a vault-backed replay provider → `CALYX_ANNEAL_SOAK_LIVE_TRAFFIC_UNAVAILABLE`; no report, gate, metrics, promotions, ledger events, or soak rows.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `SoakReport` + Ledger A/B promotions + p99 metric series persisted to `anneal_soak` CF.
- **Readback:** `calyx anneal soak-report --last 1` — prints `baseline_p99_ns`, `final_p99_ns`, `p99_reduction`, `recall_final`, `oscillation_detected`, `promotions`.
- **Prove seeded simulation:** run the ignored seeded FSV on aiwonder and read back the persisted report and ledger bytes.
- **Prove LiveTraffic boundary:** use a durable Aster vault containing an unrelated fixture, attempt `LiveTraffic`, reopen the vault, and verify the fixture bytes and sequence are unchanged while `AnnealSoak` and Ledger remain empty.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `soak-report` output + Ledger snippet attached to PH46 GitHub issue proving `p99 ↓ ≥20%`, no recall regression, no oscillation
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
