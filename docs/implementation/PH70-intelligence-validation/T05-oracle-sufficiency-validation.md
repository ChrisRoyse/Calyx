# PH70 · T05 — Oracle sufficiency validation — SWE-bench ≈0.46 deficit

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-oracle` |
| **Files** | `scripts/validate_oracle_sufficiency.sh` (≤500) |
| **Depends on** | PH69 T04 (SWE-bench Lite verified, 300×8); PH49 (consequence prediction + sufficiency gate) |
| **Axioms** | A2, A20, A15 |
| **PRD** | `28 §2` (Oracle row), `28 §3` row 3 |

## Goal

Prove Oracle sufficiency on SWE-bench Lite on aiwonder: a form-only panel (no code
execution, no test runner — surface features only) produces `I(panel;oracle)` with
approximately the 0.46 deficit reported in the paper, triggering the sufficiency-
refusal gate. Read the persisted `oracle_sufficiency` metric from the Aster vault
on aiwonder — not a return value. The sufficiency-refusal must fire (refuse to
predict) because `I < H(Y)`.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_oracle_sufficiency.sh`:
      (1) Ingest SWE-bench Lite (300 instances) into an Aster vault; the grounded
          anchor is `test_pass_fail` (FAIL_TO_PASS tests: did the patch make them
          pass? — a deterministic binary oracle).
      (2) Build a form-only panel: text lenses on `problem_statement` + `hints_text`
          only; no code-execution lens, no test-runner lens.
      (3) Run `calyx oracle sufficiency_report --vault calyx_ph70_validation`;
          write output to `/zfs/hot/calyx/metrics/oracle_sufficiency.json`.
      (4) Read `I(panel;oracle)` from the JSON; assert it is below `H(Y)` (entropy
          of the binary pass/fail label ≈ 1.0 bit for balanced classes); the deficit
          ≈ 0.46 bits is the expected value from the paper.
      (5) Assert sufficiency-refusal fires: `oracle.refuse_when_insufficient` returns
          true; read the `oracle_refused` flag from the persisted metric.
      (6) Run `oracle.calibration_capped_at_oracle_self_consistency`: verify that
          predicted confidence is capped at the oracle's own self-consistency value.
      (7) Optionally run `reverse_query` on a known cause (one known-pass instance
          whose features imply the patch) and verify it recovers a matching instance.
- [ ] Fail-closed: `I(panel;oracle)` ≥ `H(Y)` (unexpected) → exits 1,
      `CALYX_FSV_ORACLE_PANEL_UNEXPECTEDLY_SUFFICIENT`.
- [ ] Sufficiency-refusal not fired → exits 1,
      `CALYX_FSV_ORACLE_REFUSAL_DID_NOT_FIRE`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: a synthetic 20-instance binary-oracle fixture (10 pass, 10 fail, seed=42)
      with a form-only panel that has near-zero MI with the oracle; assert
      sufficiency-refusal fires; assert `I(panel;oracle)` < `H(Y)`.
- [ ] proptest: property that a panel with known high MI (> H(Y)) does NOT trigger
      refusal; a panel with known low MI (< H(Y)) does trigger refusal.
- [ ] edge (≥3):
      (1) all-pass oracle (H(Y)=0) → entropy = 0, no information needed,
          sufficiency-refusal does NOT fire (correct behavior);
      (2) `oracle_self_consistency` = 0.5 → calibration capped at 0.5;
      (3) `reverse_query` on a non-existent instance_id → exits 1,
          `CALYX_ORACLE_INSTANCE_NOT_FOUND`.
- [ ] fail-closed: SWE-bench Lite not ingested (empty vault) → exits 1,
      `CALYX_FSV_EMPTY_CORPUS`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/oracle_sufficiency.json` on aiwonder;
  `oracle_sufficiency` CF row in the Aster vault.
- **Readback:**
  ```
  cat /zfs/hot/calyx/metrics/oracle_sufficiency.json
  python3 -c "
  import json
  d = json.load(open('/zfs/hot/calyx/metrics/oracle_sufficiency.json'))
  print('I(panel;oracle):', d['panel_mi'])
  print('H(Y):', d['oracle_entropy'])
  print('deficit:', d['oracle_entropy'] - d['panel_mi'])
  print('refused:', d['oracle_refused'])
  print('PASS' if d['oracle_refused'] and (d['oracle_entropy'] - d['panel_mi']) > 0.3 else 'FAIL')
  "
  calyx readback --cf oracle_sufficiency --vault calyx_ph70_validation | head -5
  ```
- **Prove:** before: metric file absent; after: `oracle_sufficiency.json` contains
  `oracle_refused: true`; deficit ≈ 0.46 (accept 0.3–0.65 as reasonable range
  given the form-only panel); Python assertion prints `PASS`; sufficiency CF row
  present.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + screenshot of oracle_sufficiency.json showing
      `oracle_refused: true` and deficit ≈ 0.46) attached to the PH70 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
