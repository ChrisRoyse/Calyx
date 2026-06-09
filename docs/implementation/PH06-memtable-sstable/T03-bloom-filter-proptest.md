# PH06 · T03 — Bloom filter proptest (no false negatives)

| Field | Value |
|---|---|
| **Phase** | PH06 — Memtable + LSM SSTable writer/reader |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/sst/bloom.rs` (≤500) |
| **Depends on** | T02 (SST uses bloom) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §2` |

## Goal

Prove the bloom filter never produces false negatives: every key inserted
`may_contain` returns true. Also bound the false-positive rate: for a 10
bits/key filter with n keys, FPR < 1% (proven empirically with a random key set).
Harden the encode/decode round-trip with proptest. This is the no-false-negative
invariant that guards the SST point-lookup fast path.

## Build (checklist of concrete, code-level steps)

- [x] Verify current `BloomFilter` uses ≥10 bits per key; if it does not,
  increase the default bit density to 10 bits/key.
- [x] Add proptest: for any `Vec<Vec<u8>>` of keys (distinct, up to 1000),
  `BloomFilter::from_keys(keys)` → `may_contain(k) == true` for every `k` in
  keys (no false negatives, zero tolerance).
- [x] Add empirical FPR test: insert 10,000 random keys (seeded RNG,
  `rand::SeedableRng::seed_from_u64(0xDEAD_BEEF)`), generate 100,000 random
  probe keys that are guaranteed not in the insert set, assert FPR < 1%.
- [x] Add proptest: `BloomFilter::encode` + `BloomFilter::decode` round-trips: the
  re-decoded filter `may_contain` all originally inserted keys.
- [x] Add test: `BloomFilter::from_keys([])` (empty) → `may_contain(b"any_key")`
  may be true or false but must not panic.
- [x] Add fail-closed test: `BloomFilter::decode` on a 0-byte slice returns `None`
  (not a panic).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert keys `[b"alpha", b"beta", b"gamma"]`; all three return `true`
  from `may_contain`; `b"delta"` may return true or false but must not return
  false for any inserted key.
- [x] proptest: `∀ distinct key sets`: no false negatives (zero tolerance).
- [x] edge (≥3): (1) empty key set → no panic; (2) single key → `may_contain`
  returns true; (3) encode/decode with max 10,000 keys → lossless.
- [x] fail-closed: `decode(b"")` → `None` (not panic, not corrupt filter).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster bloom` output on aiwonder.
- **Readback:** `cargo test -p calyx-aster bloom -- --nocapture 2>&1`
- **Prove:** Proptest output shows ≥100 cases passed with 0 false-negative
  counterexamples. The FPR test prints `fpr = <N> / 100000 = <rate>` and asserts
  `rate < 0.01`. Screenshot of terminal output posted to PH06 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
