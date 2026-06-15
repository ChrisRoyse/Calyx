# PH37 · T05 — No-flatten enforcement + average-passing / slot-failing rejection

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/guard.rs` (≤500), `crates/calyx-ward/tests/guard_no_flatten.rs` (≤500) |
| **Depends on** | T04 (this phase) |
| **Axioms** | A3, A12 |
| **PRD** | `dbprdplans/09 §2`, `09 §4` |

## Goal

Prove — structurally and by test — that no flattened-vector path exists: an
output that passes a cosine average across all slots but fails at least one
required slot is unconditionally rejected. This is the central anti-injection
property from `09 §2`: "an attack that fools the average can't fool every axis
at once."

## Build (checklist of concrete, code-level steps)

- [x] Add a module-level invariant comment in `guard.rs` documenting that slot
      vectors are scored independently and must not use an aggregate vector
      gate. Do not invent a Rust lint for this invariant; enforce it with
      source-readback tests.
- [x] Implement test-only `average_cosine_would_pass(&GuardVerdict) -> bool`:
      computes the mean of all per-slot cos values and returns `true` if mean
      is greater than or equal to mean tau. This function is used **only in
      tests** to demonstrate the attack scenario, never as a gate.
- [x] In test file `tests/guard_no_flatten.rs`, construct the canonical
      average-passing/slot-failing scenario:
      - Two required slots, τ = `[0.7, 0.7]` (both)
      - Cos scores = `[0.95, 0.45]` → average = 0.70 (≥ τ_avg = 0.70), but
        slot-2 fails (0.45 < 0.70)
      - Call `guard()` → assert `overall_pass == false`
      - Call `average_cosine_would_pass()` on the same verdict → assert `true`
      - This demonstrates the attack scenario is blocked
- [x] Add a source-level test that reads `guard.rs` bytes, requires the
      `INVARIANT A3` marker, and asserts no non-comment aggregate-vector gate
      markers (`concat`, `extend_from_slice`, `.append(`, `flat_map`) appear.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: the canonical average-passing/slot-failing case above — `guard()`
      returns `overall_pass == false` while `average_cosine_would_pass()` returns
      `true`; the `per_slot` vec shows cos=0.95/pass=true + cos=0.45/pass=false
- [x] unit: three slots where average cos=0.71 > mean-τ=0.70 but 2 of 3 slots
      fail individually; under `AllRequired` → overall fail; under `KofN{k:1}`
      → overall pass (1 slot passed)
- [x] proptest: for any slot-vector set where at least one slot cos < its τ,
      `AllRequired` guard always returns `overall_pass == false` regardless of
      the average
- [x] edge: identical produced and matched vectors on all slots → all cos=1.0 →
      overall pass regardless of τ (upper-bound sanity)
- [x] edge: exactly one slot at cos=0.0 with τ=0.7 → fail; average of remaining
      high-cos slots irrelevant
- [x] fail-closed: if somehow `per_slot` is empty under `AllRequired` and a
      code path tried to compute average — assert no panic (empty average handled
      as pass, not division-by-zero)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing the no-flatten attack
  verdict JSON, the source-readback result for `guard.rs`, and a SHA-256
  manifest.
- **Readback:** run the manual FSV fixture with
  `CALYX_WARD_NO_FLATTEN_FSV_DIR=$root`, then separately inspect the JSON and
  source-readback files with `xxd`, `sha256sum`, and `grep`.
- **Prove:** durable JSON shows `overall_pass=false` and
  `average_would_pass=true` in the canonical attack scenario; the source
  readback shows no non-comment concatenated-slot path; `guard.rs` line count
  is <=500.

## FSV readback (2026-06-09, issue #262)

- **Commit:** `3dbe1a6` (`Harden Ward no-flatten guard edges`)
- **aiwonder gates:** `git diff --check`, `cargo fmt --check`,
  `cargo check -p calyx-ward`, `cargo test -p calyx-ward -- --nocapture`,
  `cargo clippy -p calyx-ward --all-targets -- -D warnings`, and
  `bash scripts/linecount.sh` all returned 0.
- **Line counts:** `guard.rs` 413, `tests/guard_no_flatten.rs` 320,
  `tests/guard_kofn.rs` 182, `lib.rs` 22.
- **SoT root:** `/home/croyse/calyx/data/fsv-issue262-ph37-t05-20260609-3dbe1a6`
- **Hashes:**
  - `average-attack-ood-error.json`
    `bf659e423f42b4e207055e8be749d4491df996351c14ac8ec88c10c4dc588068`
  - `average-attack-verdict.json`
    `d6a71459b0224b1adfe8c118218af96d8811a1d333d171f324e3e52f34c761c4`
  - `three-slot-allrequired-verdict.json`
    `7790b429ad97d07f3f2024f1a97c1e1b037ba41621f79da64917cf71b96ea0ac`
  - `three-slot-kofn-one-verdict.json`
    `a9034729e109eff25df19896d3fe700bd7c62bdcba26848b1ae0a97147699caa`
  - `source-readback.json`
    `dd53971c3391b1908cc08edfdfc071e5b5b2965cf58f0e9a51f8d02e7baad3c1`
  - `no-flatten-fsv.log`
    `c7bd0734839fff64d813e2f23a6454da9979a6aa2e26a8cb63a4de6a7449d330`
  - `src/guard.rs`
    `20b69961aabf36810d2b59370c3f727b459c2025d7f14c5aa324914efda3c969`
  - `tests/guard_no_flatten.rs`
    `2272d914a01477530aed10bb9d550243d4c545adbd612fd2816b619a06f1e846`
- **Happy/attack readback:** `average-attack-verdict.json` contains
  `average_would_pass=true`, `overall_pass=false`, `failing_slots=[2]`,
  slot 1 `cos=0.949999988/pass=true`, and slot 2
  `cos=0.449999988/pass=false`.
- **OOD readback:** `average-attack-ood-error.json` contains
  `code=CALYX_GUARD_OOD` for the same average-pass/slot-fail attack.
- **Edge readbacks:** the three-slot `AllRequired` artifact contains
  `average_would_pass=true`, `overall_pass=false`, and failing slots `[2,3]`;
  the `KofN{k:1}` artifact contains `overall_pass=true` while preserving the
  same failed slot detail. `source-readback.json` contains
  `contains_a3_invariant=true`, `aggregate_vector_gate_markers=[]`, and
  `line_count=413`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
