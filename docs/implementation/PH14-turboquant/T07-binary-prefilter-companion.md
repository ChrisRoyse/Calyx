# PH14 T07 - Binary Prefilter Companion

## Goal

Implement the Binary 1-bit companion to TurboQuant: a `QuantLevel::Bits1`
codec that stores signs of rotated coordinates and a Hamming/popcount prefilter
that ranks candidates before the full TurboQuant estimator.

## Build

- Add `src/quant/binary.rs` with `BinaryCodec { seed: RotationSeed }`.
- Encode by applying `apply_rotation`, storing `rotated[i] > 0.0` as
  low-bit-first packed sign bits, and preserving `seed_id`.
- Decode as a coarse unit direction: expand sign bits to `+/-1/sqrt(dim)` and
  apply the inverse rotation back into the vector domain.
- Add `hamming_dot_estimate(a, b)` using
  `1 - 2 * popcount(a.bits XOR b.bits) / dim`.
- Add `binary_prefilter(query, candidates, keep)` returning top indices by
  Hamming estimate with deterministic lower-index tie-breaks.
- Fail closed on non-finite inputs, wrong level, dimension mismatch, seed
  mismatch, malformed length, and non-zero padding bits.

## Tests

- Fixed unit vector produces deterministic packed bytes and expected length.
- Hamming self estimate is exactly `1.0`.
- Hamming estimate for a vector and its negation is exactly `-1.0`.
- Prefilter recall is at least `0.80` on deterministic dim-128 random trials.
- Edges: dim=1, `keep >= candidates.len()`, and partial final byte.
- Fail closed: non-finite input and mismatched seed ids return catalog errors.

## FSV

Run on aiwonder:

```bash
cargo test -p calyx-forge quant::binary -- --nocapture
```

Read back the log and prove:

- `binary_hamming_self_is_one` prints `hamming=1.000000`.
- `binary_prefilter_recall` prints `recall=...` and every recall is `>= 0.80`.
- Encoded fixed bytes are printed in hex.
- Wrong seed reports `seed_id mismatch in hamming_dot_estimate`.

## Done

- `cargo check`, `cargo clippy -D warnings`, and `cargo test` pass on
  aiwonder.
- All `.rs` files remain under 500 lines.
- FSV evidence is attached to issue #89.
