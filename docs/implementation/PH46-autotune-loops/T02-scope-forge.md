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

- [ ] `struct ForgeConfig { tile_m: u32, tile_n: u32, tile_k: u32, dtype: DType, batch_size: u32 }` — `DType` enum `{ Fp32, Fp16, Bf16, Fp8 }`; serializable as CBOR for `ConfigVariant`.
- [ ] `struct ForgeScopeTuner { bandits: HashMap<ShapeKey, ConfigBandit>, cache: Arc<AutotuneCache>, substrate: Arc<AnnealSubstrate> }` — `ShapeKey` is `(op_id, shape_bucketed, dtype, device_id)`.
- [ ] `fn on_op(&mut self, key: ShapeKey, elapsed_ns: u64, recall: f64)` — records the result for the current arm; if the bandit selects an explore arm, schedules a shadow run via `substrate.propose_change`.
- [ ] `fn candidate_configs(key: &ShapeKey) -> Vec<ForgeConfig>` — generates a bounded set of candidate tile/dtype configs for the given shape; at most 8 candidates per key.
- [ ] `fn get_incumbent(&self, key: &ShapeKey) -> ForgeConfig` — returns the current best config for the key; falls back to a safe default if no bandit exists yet.
- [ ] `fn promote(key: &ShapeKey, new_config: ForgeConfig, change_id: ChangeId)` — writes to `AutotuneCache` (PH16) + writes Ledger `action=AutotunePromote`.
- [ ] Shape bucketing: round each dim to next power of 2; caps at `65536` to limit key explosion.
- [ ] CPU↔GPU bit-parity ≤ 1e-3 is enforced by Forge (PH13); `ForgeScopeTuner` does NOT change the math semantics — only tile/batch/dtype within the parity-preserving range.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: two candidates for a `(gemm, 768x768, fp16, cuda)` key; candidate B has 20% lower latency and same recall; after 3 wins (hysteresis=3), `get_incumbent` returns B's config.
- [ ] unit: candidate with lower latency but lower recall (below tripwire) → NOT promoted; incumbent unchanged.
- [ ] proptest: for any sequence of `on_op` calls, `get_incumbent` always returns a valid `ForgeConfig` (no panic on empty bandit).
- [ ] edge: first `on_op` for a new key → bandit created with default config as arm 0; no crash; `get_incumbent` returns default.
- [ ] fail-closed: `AutotuneCache` write fails → `CALYX_FORGE_CACHE_WRITE_FAIL`; in-memory bandit state still updated; serving path unaffected.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `AutotuneCache` CF + Ledger `AutotunePromote` entries + `bandit-status` for Forge keys.
- **Readback:** `calyx anneal autotune-report --scope forge --last 5` — prints shape keys, current incumbent configs, trial counts, recent promotions.
- **Prove:** run 50 synthetic `on_op` calls for `(gemm, 768x768, fp16, cuda)` with arm B consistently winning; confirm `get_incumbent` returns arm B config; `autotune-report` shows the promotion entry with before/after latency.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
