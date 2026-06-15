# PH12 · T05 — Golden-vector fixtures + numpy reference validation

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/cpu_kernels.rs` (≤500), `crates/calyx-forge/tests/golden/` (fixture files) |
| **Depends on** | T02, T03, T04 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §2/§6`, `dbprdplans/19 §4` |

## Goal

Build a seeded golden-vector fixture set (numpy-generated on aiwonder, committed
as binary + JSON sidecar) and write the Rust test that reads those fixtures and
asserts the CPU backend produces outputs within tolerance. This is the shared
ground truth that PH13 (CUDA parity) compares against — the fixtures are immutable
once committed; a change requires a new fixture version.

## Build (checklist of concrete, code-level steps)

- [x] Python generator script `tests/golden/generate_golden.py` (not a Rust file;
  run once on aiwonder with pinned numpy 1.26.x):
  — seed `numpy.random.default_rng(seed=0xCALYX12)` (hex literal in comment);
  — generate: 64 random dim-128 f32 vectors (`vectors_128d.bin`, row-major f32
  LE), a 128×64 matmul input pair (`gemm_A.bin`, `gemm_B.bin`), expected outputs
  (`gemm_C_ref.bin`, `cosine_ref.bin`, `topk_ref.bin`) computed via numpy's
  `np.dot`, `scipy.spatial.distance.cosine`, and `np.argsort`;
  — sidecar `golden_manifest.json`: `{ "seed": "0xCALYX12", "numpy_version":
  "1.26.x", "n_vecs": 64, "dim": 128, "gemm_m": 128, "gemm_k": 64, "gemm_n": 32 }`
- [x] Commit the generated `.bin` and `.json` files under `tests/golden/` (they are
  small: 64×128×4 ≈ 32 KB each); document that they must never be regenerated
  without bumping `"seed_version"` in the manifest
- [x] `tests/cpu_kernels.rs`: `fn load_golden_f32(name: &str) -> Vec<f32>` reads
  `tests/golden/<name>.bin` as LE f32 bytes; panics with filename in message on IO error
- [x] Test `golden_gemm_matches_numpy`: load `gemm_A`, `gemm_B`, `gemm_C_ref`;
  run `CpuBackend.gemm(...)`; assert max `|computed[i] - ref[i]| ≤ 1e-4` with
  a message printing the worst offender's index and values
- [x] Test `golden_cosine_matches_numpy`: load `vectors_128d`, `cosine_ref`;
  run `CpuBackend.cosine_batch(query=vectors_128d[0], candidates=vectors_128d[1..], ...)`
  assert max `|computed[i] - ref[i]| ≤ 1e-5`
- [x] Test `golden_topk_matches_numpy`: load `vectors_128d`, `topk_ref` (top-8
  indices for the first query); assert `CpuBackend.topk` returns same index order
- [x] All three golden tests print on failure: seed, numpy version, worst error,
  the index, and the expected vs actual values — enough to diagnose drift without
  re-running numpy

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `load_golden_f32("gemm_A")` returns a vec of length exactly `128 * 64`
- [x] unit: golden_manifest.json deserializes and `seed == "0xCALYX12"`
- [x] proptest: `load_golden_f32` → all values are finite (no NaN/Inf in committed fixtures)
- [x] edge (≥3): (1) request a non-existent golden file → panic with filename in
  message (not silent); (2) truncated binary file (odd byte count) → error message
  includes "unexpected EOF"; (3) cosine of the first vector against itself →
  result ≥ 0.9999 (sanity bound, not exact 1.0 due to float precision)
- [x] fail-closed: any golden test that fails tolerance prints `CALYX_FORGE_GOLDEN_MISMATCH`
  (a non-CALYX_* prefix is acceptable here since this is a test-only sentinel, but
  the message must include the op name, index, expected, and actual)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cpu_kernels.rs` golden tests + the committed `.bin` files on aiwonder
- **Readback:**
  ```bash
  # Confirm fixtures exist and are non-empty:
  ls -la crates/calyx-forge/tests/golden/
  xxd crates/calyx-forge/tests/golden/cosine_ref.bin | head -3

  # Run golden tests:
  cargo test -p calyx-forge golden -- --nocapture 2>&1 | grep -E "PASSED|FAILED|worst"
  ```
- **Prove:** `golden_gemm_matches_numpy`, `golden_cosine_matches_numpy`,
  `golden_topk_matches_numpy` all PASSED; `xxd` shows non-zero bytes in first line
  of `cosine_ref.bin`; absent: any "FAILED" or tolerance breach message

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set — this card *establishes* the
      golden set; PH13 T03 verifies CUDA agrees with these same `.bin` files
- [x] FSV evidence (readback `xxd` + test output / screenshot) attached to the PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
