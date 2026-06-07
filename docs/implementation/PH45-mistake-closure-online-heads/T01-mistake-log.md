# PH45 · T01 — MistakeLog (append, rate metric)

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/learn/mistake_log.rs` (≤500) |
| **Depends on** | — (first card) |
| **Axioms** | A4, A14, A15 |
| **PRD** | `dbprdplans/12 §3`, `dbprdplans/03 §6` |

## Goal

Implement `MistakeLog`: an append-only log that records every observed
contradiction between a trusted prediction and an observed outcome. Each entry
captures the constellation ID, predicted value, observed value, relevant anchor,
logical timestamp, and computed surprise (`|predicted − observed|`). The log
exposes a `mistake_rate()` metric (mistakes per N queries in a rolling window)
that feeds into the `J` objective's `w6 · mistake_rate` term in PH48.

## Build (checklist of concrete, code-level steps)

- [ ] `struct MistakeEntry { cx_id: CxId, predicted: f64, observed: f64, anchor: AnchorId, ts: LogicalTime, surprise: f64 }` where `surprise = (predicted − observed).abs()`.
- [ ] `struct MistakeLog { cf: MistakeCf, window_size: usize, clock: Arc<dyn Clock> }` — persisted in `anneal_mistakes` CF in Aster.
- [ ] `fn append(&mut self, cx_id, predicted, observed, anchor) -> Result<MistakeRef, CalyxError>` — computes `surprise`, appends to CF under a monotonic key, returns a `MistakeRef` (seq + surprise score).
- [ ] `fn mistake_rate(&self, window: usize) -> f64` — reads the last `window` entries; returns `mistakes_with_high_surprise / window` where "high surprise" means `surprise > threshold` (configurable, default `0.3`).
- [ ] `fn recent(&self, n: usize) -> Vec<MistakeEntry>` — returns last `n` entries in insertion order; used by `ReplayBuffer` to seed priority queue on restart.
- [ ] All entries `serde::{Serialize, Deserialize}`; stored as CBOR in the CF value.
- [ ] Never modifies base data, never touches frozen lens weights; `MistakeLog` is purely append-only.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: append three entries with `(predicted=0.9, observed=0.1)`, `(0.8, 0.7)`, `(0.5, 0.5)` → `surprises = [0.8, 0.1, 0.0]`; `mistake_rate(3)` with threshold `0.3` = `1/3 = 0.333…`.
- [ ] unit: `recent(2)` returns last 2 in insertion order; `recent(100)` on a 3-entry log returns all 3.
- [ ] proptest: for any sequence of appends, `mistake_rate(window)` is in `[0.0, 1.0]`.
- [ ] edge: empty log → `mistake_rate(10) = 0.0`; `recent(5)` returns empty vec; `window=0` → `CALYX_ANNEAL_INVALID_WINDOW`.
- [ ] fail-closed: CF write failure → `CALYX_ASTER_CF_UNAVAILABLE`; in-memory append still tracked for this session.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_mistakes` CF rows.
- **Readback:** `calyx readback anneal mistakes --last 5` (or `xxd` CF at the last 5 seq keys) — prints `cx_id`, `predicted`, `observed`, `surprise`, `ts`.
- **Prove:** call `append(cx_id="cx_1", predicted=0.9, observed=0.1, anchor="a_1")`; `readback mistakes --last 1` shows `surprise=0.8`; `mistake_rate(1)` = `1.0` (surprise `0.8 > 0.3` threshold).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
