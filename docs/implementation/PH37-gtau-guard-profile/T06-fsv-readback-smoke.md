# PH37 · T06 — FSV harness — per-slot verdict readback + anti-flatten smoke test

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/guard_ph37_fsv.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A3, A12, A16 |
| **PRD** | `dbprdplans/09 §1`, `09 §2`, `09 §4` |

## Goal

Provide the complete FSV harness for PH37: a single runnable test binary on
aiwonder that covers the phase's full exit gate — per-slot verdict readback,
the average-passing/slot-failing rejection proof, `CALYX_GUARD_OOD` emission,
and the anti-flatten source check. The output of this test run is the evidence
attached to the PH37 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [x] Write `tests/guard_ph37_fsv.rs` with `#[test] fn fsv_per_slot_verdict_readback`
      that:
      - Constructs a `GuardProfile` with slots `["content", "style"]`,
        τ = `{"content": 0.72, "style": 0.65}`, policy `AllRequired`,
        novelty `NewRegion`; `calibration: None`
      - Provides produced vecs (seeded `f32` arrays, seed=42) and matched vecs
        (seeded, seed=7); both pre-normalized
      - Calls `guard()`; prints `GuardVerdict` as `{:?}` and as JSON
        (`serde_json::to_string_pretty`)
      - Asserts `per_slot.len() == 2`; asserts each `SlotVerdict` has finite
        `cos` in `[-1.0, 1.0]`
- [x] Write `#[test] fn fsv_average_passing_slot_failing_rejected` with the
      exact scenario from T05: cos=`[0.95, 0.45]`, τ=`[0.70, 0.70]`; assert
      `overall_pass == false` and `average_cosine_would_pass(..) == true`;
      print both values to stdout
- [x] Write `#[test] fn fsv_ood_code_emitted` — call `guard()` in a failing
      scenario; capture `WardError::Ood { .. }`; print `format!("{}", err)`;
      assert the formatted string contains `"CALYX_GUARD_OOD"`
- [x] Write `#[test] fn fsv_no_flatten_source_check` — read
      `concat!(env!("CARGO_MANIFEST_DIR"), "/src/guard.rs")` as a string;
      require the `INVARIANT A3` marker and assert no non-comment line contains
      aggregate-vector gate markers (`concat`, `extend_from_slice`, `.append(`,
      `flat_map`); print line count; assert ≤ 500
- [x] Write `#[test] fn fsv_guard_profile_serde_roundtrip` — construct full
      `GuardProfile` with `CalibrationMeta` populated; round-trip via
      `serde_json`; assert equality; print JSON to stdout
- [x] Test data is deterministic and hand-constructed (no `SystemTime`, no live
      network, no random dependency added)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `fsv_per_slot_verdict_readback` — prints per-slot cos/tau/pass;
      JSON output parseable; assertion green
- [x] unit: `fsv_average_passing_slot_failing_rejected` — prints
      `overall_pass=false` and `average_would_pass=true` to stdout
- [x] unit: `fsv_ood_code_emitted` — formatted error string contains
      `CALYX_GUARD_OOD`
- [x] unit: `fsv_no_flatten_source_check` — guard.rs ≤ 500 lines; `INVARIANT
      A3` present; aggregate-vector gate markers absent in non-comment source
      lines
- [x] unit: `fsv_guard_profile_serde_roundtrip` — original == deserialized;
      JSON printed includes all required keys

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue263-ph37-t06-<date>/` containing the
  captured cargo log, per-slot verdict JSON, anti-flatten source-check readback,
  and SHA-256 manifest. Stdout is only one captured artifact, not the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue263-ph37-t06-<date>
  mkdir -p "$root"
  cargo test -p calyx-ward -- --nocapture 2>&1 | tee "$root/ph37-fsv.log"
  grep -E "CALYX_GUARD_OOD|overall_pass|per_slot|average_would_pass" "$root/ph37-fsv.log"
  xxd -g 1 "$root/per-slot-verdict.json" | head -32
  sha256sum "$root"/* | sort
  wc -l crates/calyx-ward/src/guard.rs
  ```
- **Prove:** grep output contains `CALYX_GUARD_OOD`, `overall_pass: false`,
  `average_would_pass: true`; `wc -l` shows ≤ 500; all tests marked `ok` in
  cargo output; `xxd` shows durable JSON bytes with per-slot `(cos,tau,pass)`;
  attach the root path, hashes, and readback excerpts to the PH37 GitHub issue

## FSV readback (2026-06-09, issue #263)

- **Commit:** `4cde3b7` (`Add Ward PH37 FSV readback fixture`)
- **aiwonder gates:** `git diff --check`, `cargo fmt --check`,
  `cargo check -p calyx-ward`, `cargo test -p calyx-ward -- --nocapture`,
  `cargo clippy -p calyx-ward --all-targets -- -D warnings`, and
  `bash scripts/linecount.sh` all returned 0.
- **Line counts:** `guard.rs` 413, `tests/guard_no_flatten.rs` 320,
  `tests/guard_ph37_fsv.rs` 342, `tests/guard_kofn.rs` 182, `lib.rs` 22.
- **SoT root:** `/home/croyse/calyx/data/fsv-issue263-ph37-t06-20260609-4cde3b7`
- **Hashes:**
  - `per-slot-verdict.json`
    `74ba82f6a90b0337c91f60ccdbf7c27e614800fe92477d63e0d79df92fa90f83`
  - `average-attack-verdict.json`
    `d6a71459b0224b1adfe8c118218af96d8811a1d333d171f324e3e52f34c761c4`
  - `ood-error.json`
    `bf659e423f42b4e207055e8be749d4491df996351c14ac8ec88c10c4dc588068`
  - `invalid-vector-verdict.json`
    `e29b66c58091b7a6f562a526fbb345cdc6301b4fb68ef6993a98bb70c38c163e`
  - `source-readback.json`
    `dd53971c3391b1908cc08edfdfc071e5b5b2965cf58f0e9a51f8d02e7baad3c1`
  - `profile-roundtrip.json`
    `27416b7be9f45bc2911cba684c6c1b5fdbcc4a1ab1588c9329f215b2a0bfa751`
  - `ph37-fsv.log`
    `306400e484b5f7611cbd3a9db074a23f664ff262a29dc3157a789e6f3fd87e7a`
  - `tests/guard_ph37_fsv.rs`
    `a5c7b6be4db0f5204e39953d0e11007cd50bdf123f9fe3f839bc476c955f05c8`
  - `src/guard.rs`
    `20b69961aabf36810d2b59370c3f727b459c2025d7f14c5aa324914efda3c969`
- **Readback verdict:** `per-slot-verdict.json` shows two passing slots:
  slot 1 `cos=0.8/tau=0.72/pass=true`, slot 2
  `cos=0.7/tau=0.65/pass=true`.
- **Failure/OOD readbacks:** `average-attack-verdict.json` shows
  `average_would_pass=true`, `overall_pass=false`, and failing slot `[2]`;
  `ood-error.json` contains `code=CALYX_GUARD_OOD`; `invalid-vector-verdict.json`
  shows a zero-vector input with `tau=0.0` still fails closed and quarantines.
- **Source/profile readbacks:** `source-readback.json` contains
  `contains_a3_invariant=true`, `aggregate_vector_gate_markers=[]`, and
  `line_count=413`; `profile-roundtrip.json` contains `roundtrip_equal=true`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] Forge-touching note: PH37 `guard()` uses Forge `CpuBackend`; no Ward CUDA
      dispatch exists in PH37. CPU↔GPU backend parity remains PH13 evidence,
      not a hidden PH37 claim.
- [x] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
