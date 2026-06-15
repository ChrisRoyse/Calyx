# PH48 · T06 — Integration FSV: growth rises on real corpus; gamed change fails held-out

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/tests/fsv_j_growth.rs` (≤500), `crates/calyx-anneal/tests/fsv_j_growth/support.rs` (≤500) |
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

## Implementation Notes

- Implemented as ignored aiwonder FSV test
  `crates/calyx-anneal/tests/fsv_j_growth.rs` with fixture/support helpers
  under `crates/calyx-anneal/tests/fsv_j_growth/support.rs`.
- The test writes reusable evidence under `CALYX_ISSUE428_FSV_ROOT`, including
  `intelligence-fixture.json`, `ph48_j_growth_goodhart.json`, and
  `base-sha256.json`.
- Independent manual readback uses the existing CLI surfaces:
  `calyx anneal intelligence-report`, `calyx anneal growth-curve`,
  `calyx readback ledger --kind Anneal --action GoodhartFailed`, and
  `calyx readback --cf base`.

## Build (checklist of concrete, code-level steps)

- [x] Test scenario `j_is_measured`: (a) create a synthetic vault with known metric values; (b) call `intelligence_report`; (c) assert `report.j > 0.0`; (d) assert all 8 term labels present in `format_report` output; (e) assert `dpi_headroom` is a finite `f64`; (f) assert `provisional_excluded` is printed; (g) assert top gradient action is present.
- [x] Test scenario `growth_rises_on_corpus`: (a) create a synthetic vault with initially low `J`; (b) run 1000-step simulation: each step ingests 10 documents, runs `run_sleep_pass` (mistake-closure), runs one autotune bandit tick, records `GrowthSample`; (c) assert `growth_curve.is_rising(100) = true`; (d) assert `j_last > j_first`; (e) `growth-curve` ASCII output is non-empty.
- [x] Test scenario `gamed_change_rejected`: (a) after establishing baseline `J`; (b) inject a correlated lens candidate that dominates training-set gain; (c) run `GoodhartChecker` active; (d) assert `GoodhartReport.passed = false` with held-out, cross-lens, and `Gτ` violations; (e) assert promotion reverted (live panel unchanged); (f) assert Ledger has `GoodhartFailed` entry; (g) assert persisted growth remains the non-gamed rising curve.
- [x] All scenarios seeded deterministic (`seed=0xDEADBEEF`); injected clock; no live TEI calls.
- [x] No data deleted at any point: verify base CF SHA-256 unchanged throughout all scenarios.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `j_is_measured`: all 7 assertions (a–g) must pass.
- [x] `growth_rises_on_corpus`: all 5 assertions (a–e) must pass.
- [x] `gamed_change_rejected`: all 7 assertions (a–g) must pass.
- [x] `no_data_deleted`: across all three scenarios, base CF SHA-256 is unchanged (single assertion at the end of the combined test run).
- [ ] Stage 10 exit: every phase gate (PH43–PH48) is satisfied by the chain of FSV tests; running all six phase FSV tests in sequence constitutes the Stage 10 exit proof.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_growth` CF, Ledger `GoodhartFailed` entry, `intelligence_report` output, base CF SHA-256.
- **Readback:** `calyx anneal intelligence-report --vault`; `calyx anneal growth-curve --vault --last 20`; `calyx readback ledger --kind Anneal --action GoodhartFailed --last 1`; `calyx readback --cf base`; `calyx readback --cf anneal_growth`; `calyx readback --wal`; `sha256sum` over the readback files.
- **Prove:** run `cargo test -p calyx-anneal --test fsv_j_growth -- --ignored --nocapture` on aiwonder; all assertions green; `intelligence-report` shows valid `J`; `growth-curve` shows `is_rising=true`; Ledger shows `GoodhartFailed` for the gamed scenario; base CF SHA-256 unchanged. Attach the readback outputs and hashes to the PH48 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence: `intelligence-report`, `growth-curve`, `GoodhartFailed` Ledger entry, and SHA-256 proof attached to PH48 GitHub issue
- [ ] Stage 10 exit: PH43–PH48 all FSV-proven; Calyx `SELFOPT` + `INTELLIGENCE` predicates satisfied per `03_PHASE_MAP.md`
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
