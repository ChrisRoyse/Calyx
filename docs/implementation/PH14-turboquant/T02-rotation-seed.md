# PH14 · T02 — Content-addressed `RotationSeed` + rotation matrix construction

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/quant/rotation.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/23 §4.1`, `dbprdplans/24 §7 row 11` |

## Goal

Implement the versioned, content-addressed `RotationSeed` type and the structured
Hadamard+diagonal random-sign rotation construction. The seed is the sole identity
of a quantizer instance — re-creating a `TurboQuantCodec` with the same `SeedId`
must produce bit-identical encoded bytes (replay-safe). Seed construction is
deterministic given the seed bytes; the algorithm version is embedded so old-
version seeds remain decodable after an algorithm upgrade.

## Build (checklist of concrete, code-level steps)

- [x] `src/quant/rotation.rs`: `pub struct RotationSeed { pub id: SeedId, pub version: u8, pub dim: usize, pub diagonal: Vec<f32> }`
  — `diagonal` is the random Rademacher diagonal (`±1.0` drawn from the seed);
  the rotation is `R = H_d · diag(diagonal)` where `H_d` is the Walsh-Hadamard
  transform of dimension `d` (applied in-place, O(d log d), no stored matrix)
- [x] `pub fn new_seed(dim: usize, entropy: &[u8]) -> RotationSeed`
  — `entropy` can be any bytes (e.g. `SystemTime` or an explicit caller-supplied
  value for tests); derive `diagonal` via `ChaCha8Rng::from_seed(sha256(entropy || dim_le_u64))`
  → sample `dim` values of `±1.0`; `id = sha256(diagonal_bytes || version_u8 || dim_le_u64)`
- [x] `pub fn apply_rotation(seed: &RotationSeed, vec: &mut [f32])` — apply
  Walsh-Hadamard transform in-place (butterfly O(d log d)); then elementwise
  multiply by `seed.diagonal`; asserts `vec.len() == seed.dim` else panics with dim message
- [x] `pub fn apply_rotation_batch(seed: &RotationSeed, vecs: &mut [f32], n: usize)` —
  applies `apply_rotation` to each row of an `n × dim` matrix in-place
- [x] `pub const CURRENT_SEED_VERSION: u8 = 1;` — bump this if the construction
  algorithm changes; decoders must check and return `ForgeError::SeedVersionMismatch` on mismatch
- [x] `serde::{Serialize, Deserialize}` on `RotationSeed`; `id` serializes as hex string

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `new_seed(128, b"test_entropy_1")` produces a deterministic `id`
  (same call twice → identical `id` bytes); print first 8 hex chars of id
- [x] unit: `apply_rotation` on dim-4 vector `[1,0,0,0]` produces a vector with
  `‖result‖ ≈ 1.0` (rotation is isometric, within 1e-5)
- [x] proptest: `apply_rotation` preserves L2 norm (within 1e-5) for random dim-32 vectors
- [x] proptest: `new_seed(d, entropy1) != new_seed(d, entropy2)` for distinct entropy
  bytes (with overwhelming probability — assert ids differ)
- [x] edge (≥3): (1) `dim=1` (trivial rotation — diagonal = ±1); (2) `dim=768` (real
  embedding dim — no panic, runs in < 1 ms); (3) `version` mismatch on deserialized
  seed → `ForgeError::SeedVersionMismatch`
- [x] fail-closed: `apply_rotation` with `vec.len() != seed.dim` → panic with
  `"dimension mismatch: expected {dim} got {n}"` (this is a programming error, not a user error)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `turboquant_tests::rotation_isometric` + `rotation_seed_deterministic` on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge quant::rotation -- --nocapture 2>&1 \
    | grep -E "id=|norm=|PASSED|FAILED"
  ```
- **Prove:** `rotation_seed_deterministic` prints same `id=XXXXXXXX` twice;
  `rotation_isometric` prints `norm=1.000000` (within 1e-5); absent: any panic
  or norm deviation > 1e-5

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
