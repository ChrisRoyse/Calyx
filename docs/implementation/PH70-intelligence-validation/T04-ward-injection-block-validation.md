# PH70 · T04 — Ward injection-block validation — ≥99% at calibrated FAR

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-ward` |
| **Files** | `scripts/validate_ward_guard.sh` (≤500) |
| **Depends on** | PH69 T08 (prompt-injection corpus + VoxCeleb1 verified); PH38 (τ calibration conformal + novelty→new-region) |
| **Axioms** | A2, A12, A15 |
| **PRD** | `28 §2` (Ward row), `28 §3` rows 6, 10 |

## Goal

Prove Ward's injection-blocking intelligence claim on real adversarial data on
aiwonder: injection-block rate ≥99% at calibrated FAR, measured on the
prompt-injection/jailbreak corpus. Read per-slot cosine scores and guard-verdict
counts from the persisted `guard_verdicts` CF or metric file on aiwonder — not a
return value.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_ward_guard.sh`:
      (1) Calibrate τ: ingest a clean hold-out set (benign text, no injections)
          into an Aster vault; run `calyx ward calibrate --far 0.01` (1% FAR target);
          read the calibrated τ value from the `ward_tau` CF row; write to
          `/zfs/hot/calyx/metrics/ward_tau.txt`.
      (2) Run injection corpus: for each example in `prompt_injection` dataset,
          run through Ward's `Gτ` guard; collect verdicts (blocked / passed).
      (3) Compute block rate = blocked_injection_count / total_injection_count;
          write to `/zfs/hot/calyx/metrics/ward_block_rate.txt`.
      (4) Confirm FAR on the benign validation set remains ≤ calibrated target;
          write to `/zfs/hot/calyx/metrics/ward_far.txt`.
      (5) Assert block_rate ≥ 0.99 (99%); fail-closed if not.
      (6) Read per-slot cosine values from the `guard_verdicts` CF for ≥10 injections
          to confirm the bytes are persisted.
- [ ] Fail-closed: block_rate < 0.99 → exits 1,
      `CALYX_FSV_WARD_BLOCK_RATE_BELOW_99PCT: rate=<actual>`.
- [ ] Verify valid-novelty inputs are routed to new-region, not blocked; write to
      `/zfs/hot/calyx/metrics/ward_novelty_routed.txt`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: a synthetic 100-example set (50 clean, 50 adversarial, fixed seed);
      calibrate τ on the clean set; run adversarial set; assert block count ≥ 49
      (≥98% as a floor for the unit test).
- [ ] proptest: property that raising τ monotonically increases the block rate on
      a fixed adversarial corpus (no false-increases).
- [ ] edge (≥3):
      (1) all-benign injection corpus (zero adversarial) → block rate = 0/0;
          script exits 1, `CALYX_FSV_NO_ADVERSARIAL_EXAMPLES`;
      (2) τ calibrated to FAR=0 (block everything) → block rate = 100% but FAR
          also rises; script reports both metrics;
      (3) novel (OOD) benign input → routed to new-region, not blocked; verified
          in `ward_novelty_routed.txt`.
- [ ] fail-closed: calibration corpus empty → exits 1,
      `CALYX_WARD_CALIBRATION_EMPTY_CORPUS`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/ward_tau.txt`,
  `ward_block_rate.txt`, `ward_far.txt`, `ward_novelty_routed.txt` on aiwonder;
  `guard_verdicts` CF in the Aster vault.
- **Readback:**
  ```
  cat /zfs/hot/calyx/metrics/ward_tau.txt
  cat /zfs/hot/calyx/metrics/ward_block_rate.txt
  cat /zfs/hot/calyx/metrics/ward_far.txt
  python3 -c "rate=float(open('/zfs/hot/calyx/metrics/ward_block_rate.txt').read()); print('PASS' if rate>=0.99 else 'FAIL', rate)"
  calyx readback --cf guard_verdicts --vault calyx_ph70_validation | head -10
  ```
- **Prove:** before: metric files absent; after: `ward_block_rate.txt` contains a
  float ≥ 0.99; `ward_tau.txt` contains a finite positive float; `ward_far.txt`
  contains a float ≤ 0.01; Python assertion prints `PASS`; per-slot cosine values
  present in `guard_verdicts` CF readback.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + screenshot of ward_block_rate.txt and
      ward_tau.txt showing ≥99% block at calibrated τ) attached to the PH70 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
