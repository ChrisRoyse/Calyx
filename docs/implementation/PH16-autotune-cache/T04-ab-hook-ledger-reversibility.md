# PH16 · T04 — A/B-on-live hook + Ledger stub + reversibility

| Field | Value |
|---|---|
| **Phase** | PH16 — Autotune Config Cache |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/autotune.rs` (≤500) — additions to T01 |
| **Depends on** | T01, T03 (this phase) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/13 §7` |

## Goal

Add the A/B-on-live hook (a callback invoked during live traffic to collect latency
samples for the challenger config without disrupting the incumbent), the Ledger stub
(append-only `promotion_log.jsonl` recording every promote/rollback event), and the
`rollback_promotion(key)` API making every promotion reversible. This satisfies A14
(Anneal contract: promotions are measured, A/B'd, reversible).

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct PromotionEvent { pub key: AutotuneKey, pub old_config: BestConfig, pub new_config: BestConfig, pub timestamp_ns: u64, pub action: PromotionAction }`
  `pub enum PromotionAction { Promoted, RolledBack }`; serde, Clone, Debug
- [ ] `pub fn log_promotion(event: &PromotionEvent, log_path: &Path) -> Result<(), ForgeError>`
  — append one JSON line (JSONL format) to `log_path`; create file if not exists;
  uses `OpenOptions::append(true)` + `create(true)`; each line ends with `\n`;
  // TODO(PH35): replace with real Ledger chain entry
- [ ] `pub fn rollback_promotion(cache: &mut AutotuneCache, log: &Path, key: &AutotuneKey, clock: &dyn CalyxClock) -> Result<Option<BestConfig>, ForgeError>`
  — read the last `Promoted` event for `key` from `log_path` (scan JSONL from end);
  if found → `cache.rollback(key, event.old_config.clone())`; log a `RolledBack` event;
  return `Some(new_config)` (the demoted config); if no prior promotion → return `None`
- [ ] `pub struct AbHook { pub rate: f64 }` — `rate` fraction of live dispatches that
  run the challenger instead; `pub fn should_use_challenger(hook: &AbHook, rng: &mut ChaCha8Rng) -> bool`
  → `rng.gen::<f64>() < hook.rate`
- [ ] `pub fn autotune(cache: &AutotuneCache, key: &AutotuneKey) -> BestConfig`
  — public API; if key found in cache → return clone; else return `BestConfig::default_for(key)`
  where `default_for` chooses `BackendKind::Cuda` if CUDA feature enabled, else `Cpu`,
  with default tile sizes

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `log_promotion` writes one JSONL line; re-read and deserialize → `PromotionEvent` equal
- [ ] unit: `rollback_promotion` after a `Promoted` event → cache entry = old config; log
  contains a `RolledBack` event
- [ ] unit: `autotune` on an absent key → returns default config (not an error)
- [ ] proptest: `AbHook { rate: 0.1 }` → over 1000 calls (seeded), `should_use_challenger`
  rate ≈ 10% (within ±2%)
- [ ] edge (≥3): (1) `rollback_promotion` on key with no prior promotion → `None`, no error;
  (2) log file in non-existent directory → `CacheError` with path in detail;
  (3) two promotions for same key → rollback only reverts the most recent one
- [ ] fail-closed: malformed JSONL line in log → `CacheError` with line number and content

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `autotune_tests::promotion_logged_and_reversible` on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge autotune_tests::promotion -- --nocapture 2>&1 \
    | grep -E "Promoted|RolledBack|PASSED|FAILED"

  # Inspect the log file created during the test:
  cat /tmp/calyx_promotion_test.jsonl
  ```
- **Prove:** test PASSED; `/tmp/calyx_promotion_test.jsonl` contains at least one
  `"action":"Promoted"` line and one `"action":"RolledBack"` line; the final cache
  entry for the key equals the `old_config` from the promotion event

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (JSONL log content) attached to PH16 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
