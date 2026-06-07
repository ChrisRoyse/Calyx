# PH48 · T06 — Integration FSV: growth rises on real corpus; gamed change fails held-out

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/tests/fsv_j_growth.rs` (≤500) |
| **Depends on** | T01–T05 |
| **Axioms** | A32, A2, A8 |
| **PRD** | `dbprdplans/27` (all) |

## Goal

Prove the three PH48 FSV gates in deterministic runnable tests: (1) `J` is
measured with a valid per-term breakdown; (2) growth curve rises on a synthetic
corpus under the autotune + mistake-closure loop; (3) a deliberately gamed
change (correlated lens that inflates `J` on the training set) is detected by
the Goodhart checker, reverted, and logged — the curve does not rise from a
gamed metric. This is the Stage 10 exit proof.

## Build (checklist of concrete, code-level steps)

- [ ] Test scenario `j_is_measured`: (a) create a synthetic vault with known metric values; (b) call `intelligence_report`; (c) assert `report.j > 0.0`; (d) assert all 8 term labels present in `format_report` output; (e) assert `dpi_headroom` is a finite `f64`; (f) assert `provisional_excluded` is printed; (g) assert top gradient action is present.
- [ ] Test scenario `growth_rises_on_corpus`: (a) create a synthetic vault with initially low `J`; (b) run 1000-step simulation: each step ingests 10 documents, runs `run_sleep_pass` (mistake-closure), runs one autotune bandit tick, records `GrowthSample`; (c) assert `growth_curve.is_rising(100) = true`; (d) assert `j_last > j_first`; (e) `growth-curve` ASCII output is non-empty.
- [ ] Test scenario `gamed_change_rejected`: (a) after establishing baseline `J`; (b) inject a `FakeCorrelatedLensAction` that adds a near-duplicate lens (corr `0.85` with existing lens L1) — bypasses differentiation gate in the test setup; (c) run `substrate.propose_change` with `GoodhartChecker` active; (d) assert `GoodhartReport.passed = false` (CrossLensAnomaly or HeldOutRegression); (e) assert promotion reverted (live panel unchanged); (f) assert Ledger has `GoodhartFailed` entry; (g) assert `growth_curve.is_rising` remains `true` (curve not poisoned by gamed metric).
- [ ] All scenarios seeded deterministic (`seed=0xDEADBEEF`); injected clock; no live TEI calls.
- [ ] No data deleted at any point: verify base CF SHA-256 unchanged throughout all scenarios.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `j_is_measured`: all 7 assertions (a–g) must pass.
- [ ] `growth_rises_on_corpus`: all 5 assertions (a–e) must pass.
- [ ] `gamed_change_rejected`: all 7 assertions (a–g) must pass.
- [ ] `no_data_deleted`: across all three scenarios, base CF SHA-256 is unchanged (single assertion at the end of the combined test run).
- [ ] Stage 10 exit: every phase gate (PH43–PH48) is satisfied by the chain of FSV tests; running all six phase FSV tests in sequence constitutes the Stage 10 exit proof.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_growth` CF, Ledger `GoodhartFailed` entry, `intelligence_report` output, base CF SHA-256.
- **Readback:** `calyx anneal intelligence-report`; `calyx anneal growth-curve --last 20`; `calyx readback ledger --kind Anneal --action GoodhartFailed --last 1`; `sha256sum $CALYX_HOME/vault/base/*.sst` before and after.
- **Prove:** run `cargo test --release fsv_j_growth` on aiwonder; all assertions green; `intelligence-report` shows valid `J`; `growth-curve` shows `is_rising=true`; Ledger shows `GoodhartFailed` for the gamed scenario; base CF SHA-256 unchanged. Attach all four readback outputs to the PH48 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `intelligence-report`, `growth-curve`, `GoodhartFailed` Ledger entry, and SHA-256 proof attached to PH48 GitHub issue
- [ ] Stage 10 exit: PH43–PH48 all FSV-proven; Calyx `SELFOPT` + `INTELLIGENCE` predicates satisfied per `03_PHASE_MAP.md`
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
