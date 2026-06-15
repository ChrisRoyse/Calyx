# PH46 · T02 — Forge kernel scope tuner

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/scope_forge.rs` (≤500) |
| **Depends on** | T01 (ConfigBandit), PH16 (autotune config cache extended here) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4` |

## Goal

Implement `ForgeScopeTuner`: the autotune layer for Forge math kernels. For each
distinct `(op, shape, dtype, device)` workload encountered, maintains a
`ConfigBandit` over candidate kernel configs (matmul tile sizes, batch sizes,
`bf16`/`fp16`/`fp8` dtype, CUDA sm_120 launch params). On each A/B trial, the
shadow candidate runs in the background budget while the incumbent serves the
real request; win = candidate latency < incumbent latency with no recall
regression. The PH16 config cache is the persistence backend.

## Build (checklist of concrete, code-level steps)

- [x] `struct ForgeConfig { tile_m: u32, tile_n: u32, tile_k: u32, dtype: DType, batch_size: u32 }` — `DType` enum `{ Fp32, Fp16, Bf16, Fp8 }`; serializable as CBOR for `ConfigVariant`.
- [x] `struct ForgeScopeTuner { bandits: HashMap<ShapeKey, ConfigBandit>, cache: Arc<Mutex<AutotuneCache>>, ... }` — `ShapeKey` is `(op_id, shape_bucketed, dtype, device_id)`.
- [x] `fn on_op(&mut self, key: ShapeKey, elapsed_ns: u64, recall: f64)` — records the result for the current/pending arm and returns a `ForgeTuneDecision` naming the selected shadow arm.
- [x] `fn candidate_configs(key: &ShapeKey) -> Vec<ForgeConfig>` — generates a bounded set of candidate tile/dtype configs for the given shape; at most 8 candidates per key.
- [x] `fn get_incumbent(&self, key: &ShapeKey) -> ForgeConfig` — returns the current best config for the key; falls back to a safe default if no bandit exists yet.
- [x] Promotion writes to PH16 `AutotuneCache`, persists the per-key `ConfigBandit` into `anneal_bandit`, and writes Ledger `action=AutotunePromote` through `ForgePromotionWriter` implementations for `AnnealLedger`/`AnnealSubstrate`.
- [x] Shape bucketing: round each dim to next power of 2; caps at `65536` to limit key explosion.
- [x] CPU↔GPU bit-parity ≤ 1e-3 is enforced by Forge (PH13); `ForgeScopeTuner` does NOT change the math semantics — only tile/batch/dtype within the parity-preserving range.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two candidates for a `(gemm, 768x768, fp16, cuda)` key; candidate B has 20% lower latency and same recall; after 3 wins (hysteresis=3), `get_incumbent` returns B's config.
- [x] unit: candidate with lower latency but lower recall (below tripwire) → NOT promoted; incumbent unchanged.
- [x] proptest: for any sequence of `on_op` calls, `get_incumbent` always returns a valid `ForgeConfig` (no panic on empty bandit).
- [x] edge: first `on_op` for a new key → bandit created with default config as arm 0; no crash; `get_incumbent` returns default.
- [x] fail-closed: `AutotuneCache` write fails → `CALYX_FORGE_CACHE_WRITE_FAIL`; in-memory bandit state still updated; serving path unaffected.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** PH16 `AutotuneCache` JSON file, `anneal_bandit` CF row, and Ledger `AutotunePromote` entries.
- **Readback:** `calyx anneal autotune-report --scope forge --cache <json> --vault <dir> --last 5` — prints shape keys, current incumbent configs, trial counts, recent promotions.
- **Prove:** run deterministic synthetic `on_op` calls for `(gemm, 768x768, fp16, cuda)` with arm B consistently winning; confirm `get_incumbent` returns arm B config; `autotune-report` shows the promotion entry with before/after latency.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [x] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV

## Closeout evidence

- Tuner FSV root: `/home/croyse/calyx/data/fsv-issue413-forge-scope-20260611T1819Z`.
- Forge parity FSV root: `/home/croyse/calyx/data/fsv-issue413-forge-parity-20260611T1820Z`.
- Final gate: `scripts/linecount.sh && cargo fmt --all --check && cargo test -p calyx-anneal && cargo test -p calyx-cli && cargo clippy -p calyx-anneal -p calyx-cli --all-targets -- -D warnings`.
- Forge parity gate: `cargo test -p calyx-forge --features cuda --test cuda_parity -- --nocapture`.
- Manual readback confirmed cache row `forge:gemm:1024x1024:fp16:cuda`, incumbent arm `1`, `search_p99` `1000 -> 780`, recall `0.99 -> 0.99`, and fail-closed `CALYX_FORGE_CACHE_WRITE_FAIL`.
