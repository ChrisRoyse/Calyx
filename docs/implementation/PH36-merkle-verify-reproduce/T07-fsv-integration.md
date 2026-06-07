# PH36 · T07 — FSV integration: flip-byte tamper test + reproduce bit-parity test

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/tests/fsv_integration.rs` (≤500) |
| **Depends on** | T02, T05, T06 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §3`, `11 §5`, `11 §7` |

## Goal

Produce the two byte-level FSV proofs required by the PH36 exit gate and attach
them as evidence on the GitHub issue. These tests are the definitive gate —
not a harness assertion, but actual bytes read back on aiwonder proving the
claims. Test 1: flip one ledger byte and confirm `verify_chain` detects the
break at exactly the right seq. Test 2: run `reproduce(answer_id)` on a real
answer and confirm bit-parity ≤ 1e-3.

## Build (checklist of concrete, code-level steps)

- [ ] `fn test_tamper_detected_at_exact_seq()` —
  1. Write 20 entries to an in-process vault using `smoke_ingest_n_constellations(20)`.
  2. Choose `target_seq = 11` (hard-coded for determinism).
  3. Read the raw bytes of the ledger-CF row at `seq=11`; flip byte at offset 8
     (first byte of `prev_hash`) using the test CF writer's raw-edit interface.
  4. Call `verify_chain(range: 0..20)`.
  5. `assert!(matches!(result, VerifyResult::Broken { at_seq: 11, .. }))`.
  6. `assert!(is_quarantined(manifest, 11))`.
  7. `assert!(matches!(get_provenance(cx_id_for_seq_11), Err(CALYX_LEDGER_CHAIN_BROKEN)))`.
- [ ] `fn test_reproduce_bit_parity()` —
  1. Ingest one synthetic `Constellation` (known input bytes, fixed MockClock,
     fixed forge seed `0xDEAD_BEEF`).
  2. Run a synthetic search query over it, write an Answer entry.
  3. Call `reproduce(answer_id)`.
  4. `assert!(result.reproduced)`.
  5. `assert!(result.max_drift <= 1e-3)`.
  6. Print both original and reproduced score vectors to stdout for `xxd` readback.
- [ ] Both tests must be `#[ignore]` unless run with `--include-ignored` on
  aiwonder (they touch disk; they are FSV tests, not unit tests).
- [ ] A `Makefile` target or shell helper `scripts/fsv_ph36.sh` that runs both
  ignored tests, pipes stdout through `xxd`, and prints a human-readable summary
  `"PH36 FSV PASS: tamper detected at seq=11; reproduce max_drift=0.000XYZ"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (always-run): `assert_within_tolerance` with `max_drift = 1e-3` →
  `true`; with `max_drift = 1e-3 + epsilon` → `false` (boundary condition).
- [ ] unit (always-run): `VerifyResult::Broken { at_seq: 11, .. }` pattern-match
  compiles correctly (sanity check).
- [ ] ignored FSV test 1: tamper detected at seq=11 — described above.
- [ ] ignored FSV test 2: reproduce bit-parity ≤ 1e-3 — described above.
- [ ] edge (≥3): flip the `entry_hash` field itself (offset = end-32 bytes) →
  `verify_chain` detects at `seq=11` (hash self-check fails); flip a byte in
  `seq=0` entry → `VerifyResult::Broken { at_seq: 0 }`; verify intact chain
  of 20 entries → `VerifyResult::Intact { count: 20 }`.
- [ ] fail-closed: `fsv_ph36.sh` exits non-zero if either FSV test prints anything
  other than expected; the CI analogue on aiwonder treats non-zero exit as
  a blocking failure for merge.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout + stderr from `cargo test -p calyx-ledger -- --include-ignored
  --nocapture fsv_integration 2>&1` on aiwonder
- **Readback:**
  1. `cargo test … | tee /tmp/ph36_fsv.log`
  2. `grep "BROKEN at seq=11" /tmp/ph36_fsv.log` → must match.
  3. `grep "max_drift=" /tmp/ph36_fsv.log` → extract value; confirm ≤ 1e-3.
  4. `xxd /tmp/ph36_fsv.log | grep -A2 "max_drift"` — screenshot this for the issue.
- **Prove:**
  - Test 1: output contains `CALYX_LEDGER_CHAIN_BROKEN at seq=11` — tamper
    detected at the **right seq**, not before or after. Screenshot in issue.
  - Test 2: output contains `reproduced=true, max_drift=<value>` where value
    is at most 0.001. Both original and reproduced score bytes printed and
    visible in the `xxd` dump. Screenshot in issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the reproduce golden set (Forge determinism mode)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue —
      **both** the tamper-detection screenshot and the reproduce bit-parity screenshot
      must be present; the phase is not DONE without them
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
