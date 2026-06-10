# PH42 · T03 — Lodestar: frequency → kernel candidacy; time-window kernels

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/temporal_kernel.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH33 (kernel index + grounding gaps) · PH41 (frequency field) |
| **Axioms** | A29, A10, A11 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/08 §4b` |

## Goal

Wire frequency into Lodestar's kernel candidacy scoring: a constellation that
recurs frequently has demonstrated relevance by reality (A2) and is a natural
kernel candidate. Extend PH33's `kernel_answer` scoring to add a `frequency_bonus`
proportional to `log(frequency + 1)`. Implement the time-window kernel scope:
`kernel_for_window(vault, window) -> KernelResult` — the grounding kernel of "what
mattered then" (Lodestar `scope=TimeWindow`), built from constellations active
during the window.

## Build (checklist of concrete, code-level steps)

- [ ] Implement `frequency_kernel_bonus(frequency: u64) -> f32`:
  - `(frequency as f32 + 1.0).ln() / (FREQ_BONUS_MAX as f32 + 1.0).ln()` — normalized log bonus in `[0.0, 1.0]`
  - `FREQ_BONUS_MAX = 10_000u64` (configurable constant; constellations recurring > 10_000× get bonus = 1.0)
- [ ] Extend `KernelNodeScore` (PH33) with `frequency_bonus: f32`; total score += `FREQ_WEIGHT * frequency_bonus` where `FREQ_WEIGHT = 0.15` (tunable)
- [ ] Implement `apply_frequency_bonuses(kernel_graph: &mut KernelGraph, vault: &Vault)`:
  - for each node in the kernel graph: read `frequency` from base CF (O(1)); compute `frequency_kernel_bonus`; add to node score
  - re-sort kernel nodes by updated score
- [ ] Call `apply_frequency_bonuses` at the end of PH33's `build_kernel` (the kernel is rebuilt including frequency signal)
- [ ] Implement `kernel_for_window(vault: &Vault, window: &TimeWindow, k: usize) -> Result<KernelResult, CalyxError>`:
  - collect CxIds that have at least one occurrence `t_k ∈ window` (from recurrence series store)
  - build a sub-graph from those CxIds only (using PH33 kernel-graph logic on the subset)
  - return the top-k kernel nodes within the window
- [ ] `KernelResult` for window kernel carries `scope: KernelScope::TimeWindow { window }` in its metadata

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `frequency_kernel_bonus(0)` → 0.0; `frequency_kernel_bonus(1)` ≈ `ln(2)/ln(10001)` ≈ 0.076; `frequency_kernel_bonus(10_000)` = 1.0
- [ ] unit: kernel with two nodes — A (betweenness=0.8, freq=50) and B (betweenness=0.9, freq=1) — after `apply_frequency_bonuses` with `FREQ_WEIGHT=0.15`: A score = 0.8 + 0.15 * freq_bonus(50) ≈ 0.8 + 0.15*0.427 ≈ 0.864; B score = 0.9 + 0.15*0.075 ≈ 0.911; B still ranks higher
- [ ] unit: `kernel_for_window` with window [100, 300]: CxId-A has occurrences at [50, 150, 250], CxId-B has occurrences at [400, 500] → A is included, B is excluded
- [ ] unit: `kernel_for_window` result has `scope = TimeWindow { window: [100, 300) }`
- [ ] proptest: `frequency_kernel_bonus(n) ∈ [0.0, 1.0]` for all `n ∈ [0, u64::MAX]`
- [ ] edge: `kernel_for_window` with empty window (no CxIds active) → empty `KernelResult` without panic
- [ ] edge: `frequency = u64::MAX` → bonus = 1.0 (no overflow; `log` is monotone)
- [ ] fail-closed: `frequency` field missing from base CF (pre-PH41 constellation) → treat as `frequency = 0`; bonus = 0.0; log `CALYX_LODESTAR_MISSING_FREQUENCY` warning (not error)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** persisted kernel node weight list from `calyx readback kernel-weights --artifact <kernel-weights.json>`; persisted `KernelResult` from `kernel_for_window` via `calyx readback kernel-window --artifact <kernel-window.json>`
- **Readback:** (1) ingest CxId-X 50 times (frequency=50) and CxId-Y once; persist kernel-weight JSON, run `calyx readback kernel-weights --artifact <kernel-weights.json>`, and confirm X has higher weight than Y when betweenness scores are equal; (2) persist window-kernel JSON, run `calyx readback kernel-window --artifact <kernel-window.json>`, and confirm only CxIds with occurrences in the requested window appear
- **Prove:** X appears in kernel above Y (frequency bonus applied); window kernel contains only in-window CxIds; `scope = TimeWindow` in metadata

## Implementation notes

- Issue #389 implements this in `crates/calyx-lodestar/src/temporal_kernel.rs`.
- `NodeScore` now carries `frequency_bonus`; PH42 helpers add `FREQ_WEIGHT * frequency_bonus` to the PH33 score and re-sort the node list.
- `frequency_kernel_bonus` caps at `FREQ_BONUS_MAX=10_000` and returns a normalized log bonus in `[0.0, 1.0]`, including `u64::MAX -> 1.0`.
- `apply_frequency_bonuses` reads `recurrence.frequency` from the Aster base CF through the vault, not by scanning recurrence rows. Missing base/scalar frequency is a `CALYX_LODESTAR_MISSING_FREQUENCY` warning with bonus `0.0`; invalid scalar shape fails closed with `CALYX_LODESTAR_INVALID_FREQUENCY`.
- `kernel_for_window` scans persisted recurrence rows to find CxIds with an occurrence in the half-open `[start_secs, end_secs)` window and returns `KernelResult { scope: TimeWindow { .. } }`. `kernel_for_window_from_graph` applies the same active-CxId filter to an existing PH33 association graph.
- The ignored FSV trigger is `crates/calyx-lodestar/tests/temporal_kernel_fsv.rs` and writes `kernel-weights.json`, `kernel-window.json`, and `BLAKE3SUMS.txt` under `CALYX_LODESTAR_ISSUE389_FSV_DIR`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
