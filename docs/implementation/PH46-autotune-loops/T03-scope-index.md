# PH46 · T03 — Index + quant scope tuner

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/scope_index.rs` (≤500) |
| **Depends on** | T01 (ConfigBandit), PH23 (HNSW index — tuned here), PH14 (TurboQuant — quant level tuned here) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/27 §5` |

## Goal

Implement `IndexScopeTuner`: the autotune layer for HNSW `ef`/`M`, DiskANN
beamwidth, SPANN posting cutoffs, and TurboQuant quant level per slot. Each
slot has an independent `ConfigBandit`; win = lower search p99 at the same or
better recall@k vs incumbent config. Quant level tuning is recall-gated: a
lower bit-width is only promoted if Assay's `bits_per_anchor` does not decrease
(information lossless, A25). Every promoted config is kept in the PH16 cache
and logged to the Anneal Ledger.

## Build (checklist of concrete, code-level steps)

- [ ] `struct IndexConfig { hnsw_ef: u32, hnsw_m: u32, diskann_beamwidth: u32, spann_cutoff: u32, quant_bits: u8 }` — `quant_bits` in `{4, 8, 16, 32}`; default `16`.
- [ ] `struct IndexScopeTuner { bandits: HashMap<SlotId, ConfigBandit>, assay: Arc<dyn AssayMetrics>, substrate: Arc<AnnealSubstrate>, cache: Arc<AutotuneCache> }`.
- [ ] `fn on_search(&mut self, slot_id: SlotId, p99_ns: u64, recall_k: f64, bits_per_anchor: f64)` — records result for current arm; if exploring, schedules shadow run.
- [ ] `fn quant_win_check(candidate: &IndexConfig, incumbent: &IndexConfig, bits_before: f64, bits_after: f64) -> bool` — candidate wins iff `p99 < incumbent_p99` AND `bits_after >= bits_before - 1e-6` (no information loss); `bits_before` and `bits_after` from Assay.
- [ ] `fn candidate_configs(slot_id: SlotId) -> Vec<IndexConfig>` — generates ≤8 candidates: vary `ef` in `{64, 128, 256}`, `M` in `{8, 16, 32}`, `quant_bits` in `{4, 8, 16}`; prune combinations exceeding VRAM budget.
- [ ] `fn get_incumbent_config(&self, slot_id: SlotId) -> IndexConfig` — from bandit or defaults.
- [ ] Quant downgrade requires `bits_after >= bits_before − 1e-6` (enforced in `quant_win_check`); quant upgrade (higher bits) is always allowed if latency improves.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: candidate with `ef=256` (better recall, higher p99) vs incumbent `ef=128` (lower p99) — `ef=128` wins on latency; incumbent unchanged.
- [ ] unit: quant downgrade from 16-bit to 8-bit with `bits_after = bits_before − 0.5` → `quant_win_check` returns false; incumbent unchanged.
- [ ] unit: quant downgrade from 16-bit to 8-bit with `bits_after ≈ bits_before` (within 1e-6) AND lower p99 → `quant_win_check` returns true; candidate promoted.
- [ ] proptest: for any `IndexConfig` sequence, `quant_bits` in the incumbent is always in `{4, 8, 16, 32}`.
- [ ] edge: `bits_per_anchor < 0.05` (decayed lens) → PH44 parks the lens before `IndexScopeTuner` can tune it; `on_search` for a `Parked` slot → no-op.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `AutotuneCache` CF for index config per slot + Ledger `AutotunePromote` entries.
- **Readback:** `calyx anneal autotune-report --scope index --slot 0` — prints current `ef`, `M`, `quant_bits`, trial count, last promotion.
- **Prove:** run 50 simulated searches for `slot_0` with arm B (`ef=128, quant_bits=8`) consistently beating arm A (`ef=64, quant_bits=16`) on latency AND with `bits_after ≈ bits_before`; confirm `autotune-report` shows arm B as incumbent; Ledger has `AutotunePromote` entry.

## Implementation Notes

- `scope_index.rs` is a thin facade; config/key serialization lives in `scope_index/types.rs`, and Ledger/bandit persistence adapters live in `scope_index/writer.rs` to preserve the ≤500-line gate.
- Slot keys are stable labels: `index:slot_0000`, hashed with the existing `shape_key_hash`/`bandit_key` path so `anneal_bandit` readback can find the physical row.
- PH16 cache rows use op `index`, shape `[slot_id]`, dtype `ann`, device `index:slot_####`; the config fields are duplicated into `BestConfig.extra` for human readback.
- `quant_win_check` is the quant-safety gate. Latency and recall gates are enforced by `IndexScopeTuner::on_search_for_arm` because the required signature carries only config + bits inputs.
- Quant-bit promotions that pass the win gate also require measured `QuantPromotionEvidence` and write structured Anneal Ledger `details` with before/after quant bits, bits-per-anchor, cosine error, guard FAR, and the slot hash as bytes. Missing or regressed evidence fails closed before cache, bandit, or Ledger mutation.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
