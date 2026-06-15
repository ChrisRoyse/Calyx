# PH16 · T05 — FSV: two shapes converge to two cached configs

| Field | Value |
|---|---|
| **Phase** | PH16 — Autotune Config Cache |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/autotune_tests.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04 (this phase) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/13 §7` |

## Goal

Write the definitive FSV test for PH16: run the autotune explorer on two
distinct shapes (e.g. `[512,512,512]` and `[128,768,64]`) with a pool of 4
candidate configs each; assert that after `MAX_EXPLORE_ITERS` iterations each
shape has converged to a cached `BestConfig` and the two shapes' configs are
**distinct** (different tile sizes or backend variants). This is the byte-level
proof that the cache is working and shape-specific.

## Build (checklist of concrete, code-level steps)

- [x] `tests/autotune_tests.rs`: test `autotune_two_shapes_converge` — marked
  `#[cfg_attr(not(feature="cuda"), ignore)]`
- [x] Build `candidate_pool_A` for shape `[512,512,512]`: 4 `BestConfig` variants
  differing in `tile_m` (32, 64, 128, 256) but same `backend=Cuda`
- [x] Build `candidate_pool_B` for shape `[128,768,64]`: 4 variants with different
  tile sizes appropriate for the narrow shape
- [x] Run explorer for 20 iterations per shape on aiwonder:
  ```
  for iter in 0..20 {
      let config_a = next_candidate(&mut explorer, &key_a, &incumbent_a, &pool_a);
      let result_a = microbench("gemm", &config_a, &[512,512,512], Some(&ctx), 3)?;
      record_trial(&mut explorer, &key_a, &config_a, result_a);
      if let Some(old) = promote_if_winner(...) { cache.insert(key_a, config_a); ... }
      // same for shape B
  }
  ```
- [x] After the loop: `autotune(&cache, &key_a)` and `autotune(&cache, &key_b)` →
  assert both return `Some` config; assert the two configs differ (at least one field differs)
- [x] Print: `config_a=tile_m={X}`, `config_b=tile_m={Y}`, `X != Y` assertion
- [x] Write the final cache to `tests/autotune_cache_fsv.json` (persisted file for FSV readback)
- [x] Test `autotune_promotion_logged`: after running the above, read the `promotion_log.jsonl`
  and assert it contains at least one `Promoted` event
- [x] Test `autotune_promotion_reversible`: pick the last promoted key; call `rollback_promotion`;
  assert cache returns old config; assert log now has a `RolledBack` event

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `autotune_two_shapes_converge`: asserts two distinct keys in cache after 20 iters;
  configs differ; prints both configs
- [x] `autotune_promotion_logged`: asserts ≥1 `Promoted` event in log
- [x] `autotune_promotion_reversible`: rollback succeeds; log has `RolledBack`; cache = old config
- [x] proptest (CPU-only, no `#[cfg(feature="cuda")]`): `AutotuneCache` with 100 random
  `(key, config)` inserts → `get` returns the right config for all 100 keys (no hash collision)
- [x] edge (≥3): (1) both shapes converge to same tile_m by chance — test passes (distinctness
  is on the full `BestConfig`, not just tile_m; if identical, print a warning but don't fail);
  (2) explorer runs 0 iters → both keys absent from cache → `autotune` returns default;
  (3) cache file is read back from disk and `autotune` returns the same configs

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `autotune_tests::autotune_two_shapes_converge` on aiwonder RTX 5090
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda autotune_two_shapes_converge autotune_promotion -- \
    --nocapture 2>&1 | tee /tmp/ph16_fsv.txt

  grep -E "config_a|config_b|Promoted|RolledBack|converged|PASSED|FAILED" /tmp/ph16_fsv.txt

  # Read the persisted cache file:
  python3 -m json.tool \
    crates/calyx-forge/tests/autotune_cache_fsv.json 2>/dev/null | head -40
  ```
- **Prove:** `autotune_two_shapes_converge` PASSED; output contains two distinct
  `config_a=...` and `config_b=...` lines; `autotune_cache_fsv.json` contains two
  entries with different keys; `autotune_promotion_logged` PASSED; `autotune_promotion_reversible`
  PASSED; `/tmp/ph16_fsv.txt` attached to PH16 issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (autotune ops measured in PH12–PH15)
- [x] **Two shapes converge to two cached configs** (`autotune_two_shapes_converge` is the proof)
- [x] **Promotion is logged + reversible** (`autotune_promotion_logged` + `autotune_promotion_reversible`)
- [x] FSV evidence (`/tmp/ph16_fsv.txt` + `autotune_cache_fsv.json`) attached to PH16 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
