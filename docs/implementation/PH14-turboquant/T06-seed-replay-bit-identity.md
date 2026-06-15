# PH14 · T06 — Seed replay bit-identity + cosine error bound

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/turboquant_tests.rs` (≤500) — additional tests added to same file |
| **Depends on** | T02, T05 (this phase) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/24 §7 row 11`, `dbprdplans/23 §4.1` |

## Goal

Prove that **re-quantizing with the recorded seed is bit-identical** — the defining
replay-safety property that makes TurboQuant suitable for Calyx's content-addressed
storage. Two separate `TurboQuantCodec` instances constructed from the same `SeedId`
must produce byte-for-byte identical `QuantizedVec::bytes` for the same input.
This is the FSV gate for PH14 at `24 §7 row 11`.

## Build (checklist of concrete, code-level steps)

- [x] Test `seed_replay_bit_identical`: given a fixed `RotationSeed` (from
  `new_seed(128, b"replay_test_seed")`), construct two independent `TurboQuantCodec`
  instances from that seed; encode the same 128-dim vector `v` with both; assert
  `codec1.encode(v).bytes == codec2.encode(v).bytes` (exact byte equality);
  print first 16 bytes as hex with label `seed_replay_bytes=XXXXXXXX...`
- [x] Test `seed_id_is_stable_across_reconstruction`: serialize `seed` to JSON,
  deserialize into a new `RotationSeed`, construct a codec, encode `v`; assert
  bytes == original encode bytes (JSON serde does not alter the seed)
- [x] Test `different_seeds_produce_different_encodings`: two seeds from different entropy
  → `encode(v).bytes != encode(v).bytes` (with overwhelming probability for random v;
  assert they differ on at least 1 byte)
- [x] Test `cosine_error_within_epsilon`: same 1000-pair trial as T05 but now reads the
  `SeedId` from a persisted file `tests/golden/turboquant_seed_v1.json` (generated once,
  committed); ensures the FSV result is stable across code changes — if the seed format
  changes, the file must be regenerated and the issue updated
- [x] Ensure `tests/golden/turboquant_seed_v1.json` is generated and committed as part
  of this card: run `cargo test generate_golden_seed -- --nocapture` once on aiwonder,
  commit the output file

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `seed_replay_bit_identical` asserts exact byte equality (not approximate)
- [x] proptest: `encode(encode_input, seed).bytes.len()` is stable across 100 runs
  with the same seed (no non-determinism in byte packing)
- [x] proptest: different input vectors → different encoded bytes (with same seed)
  for random non-identical f32 vectors
- [x] edge (≥3): (1) seed version bumped → `SeedVersionMismatch`; (2) `seed_id` from
  JSON matches original id bytes exactly; (3) dim=768 replay bit-identical
- [x] fail-closed: decoding with a different seed (wrong `seed_id`) → `ForgeError::QuantError
  { detail: "seed_id mismatch" }` (never silently produces wrong output)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `turboquant_tests::seed_replay_bit_identical` output bytes on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge seed_replay -- --nocapture 2>&1 \
    | grep -E "seed_replay_bytes|PASSED|FAILED"

  # Also verify the committed golden seed file exists:
  ls -la crates/calyx-forge/tests/golden/turboquant_seed_v1.json
  ```
- **Prove:** `seed_replay_bytes=XXXXXXXX` printed twice with identical values (exact
  match visible in output); `seed_replay_bit_identical` PASSED; golden seed file
  present with non-zero bytes; absent: any mismatch between the two hex strings

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] **CPU↔GPU bit-parity ≤ 1e-3 on the golden set** — TurboQuant encode uses
      `CpuBackend.gemm` (rotation); if GPU path is used, parity must hold
- [x] **re-quant with the recorded seed is bit-identical** — `seed_replay_bit_identical` is the proof
- [x] FSV evidence (seed_replay_bytes hex + test output) attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
