# PH27 · T06 — `abundance_report` skeleton + storage O(n·n_eff) FSV

> **Status: superseded by final Stage 5 implementation.** The PH27 skeleton
> landed first; PH29/PH30 then replaced provisional `n_eff` and DPI ceiling
> fields with computed report data. Final byte readback is recorded under
> `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/abundance.rs` (≤500) |
| **Depends on** | T04 (agreement_graph, weave, xterm CF writes) · T03 (materialized_count) |
| **Axioms** | A8, A9 |
| **PRD** | `dbprdplans/06 §1`, `06 §2`, `06 §8` |

## Goal

Implement the skeleton `abundance_report` that exposes the four honest numbers —
N (active lenses), C(N,2) (upper bound), materialized (actual xterm CF row
count), and n_eff placeholder (set to `N` until PH29 computes stable rank) —
plus the DPI ceiling placeholder (set to `f32::NAN` with a `provisional` tag
until PH28/PH30 compute it). This honest dashboard is required from PH27
onwards; downstream phases fill in the real numbers. The FSV for this card
proves that materialized count is ≪ C(N,2) at rest.

> **Honesty invariant:** `abundance_report` MUST always print all four numbers.
> It MUST mark n_eff and DPI ceiling as `provisional` when they are stubs.
> It MUST NOT suppress or omit `C(N,2)` — the honest bound is load-bearing.

## Build (checklist of concrete, code-level steps)

- [x] Define `AbundanceReport`:
  ```rust
  pub struct AbundanceReport {
      pub vault_id: VaultId,
      pub n_active_lenses: usize,         // N
      pub cn2_upper_bound: u64,           // C(N,2) = N*(N-1)/2
      pub materialized_xterms: u64,       // actual xterm CF row count
      pub n_eff: NeffEstimate,            // Provisional(N as f32) until PH29
      pub dpi_ceiling_bits: DpiCeiling,   // Provisional(f32::NAN) until PH28/PH30
      pub meaning_compression_yield: f32, // materialized_xterms / n_constellations
      pub computed_at_seq: u64,
  }
  ```
- [x] Define `NeffEstimate`: `Provisional(f32)` | `Computed { value: f32, ci_low: f32, ci_high: f32 }`
- [x] Define `DpiCeiling`: `Provisional` | `Computed { bits: f32, anchor: AnchorKind }`
- [x] Implement `abundance_report(vault, forge, clock) -> Result<AbundanceReport, CalyxError>`:
  - count active lenses from the registry panel
  - compute `C(N,2)` exactly: `N*(N-1)/2` as `u64`
  - count materialized xterm CF rows (Agreement + any Interaction/Concat eagerly stored)
  - set n_eff to `Provisional(N as f32)` with a comment `// refined in PH29`
  - set dpi_ceiling_bits to `Provisional` with a comment `// refined in PH28/PH30`
  - compute `meaning_compression_yield = materialized_xterms as f32 / n_constellations as f32`
- [x] `Display` impl for `AbundanceReport` that prints all four numbers clearly, marks provisional values with `[provisional]`, and never hides `C(N,2)`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: vault with N=13 active lenses, 100 constellations, all Agreement scalars only → `cn2_upper_bound = 78`, `materialized_xterms = 7800` (78 per constellation × 100); `n_eff = Provisional(13.0)`
- [x] unit: vault with 0 active lenses → `cn2_upper_bound = 0`, `materialized_xterms = 0`; no panic
- [x] proptest: `materialized_xterms <= cn2_upper_bound * n_constellations` always (materialized never exceeds the upper bound)
- [x] edge: `meaning_compression_yield` is `f32::NAN` when `n_constellations = 0` (no panic, just NaN in the output)
- [x] fail-closed: vault with corrupted xterm CF → `CALYX_ASTER_CORRUPTION` propagated; never returns a silent zero

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `abundance_report` output for a test vault with N=13 lenses and 50 constellations ingested
- **Readback:**
  ```
  cat /home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final/stage5-readback.json
  ```
  Expected output:
  ```
  N (active lenses):      13
  C(N,2) upper bound:     78
  Materialized xterms:    3900  (78 Agreement scalars × 50 constellations)
  n_eff:                  13.0  [provisional]
  DPI ceiling:            [provisional]
  ```
- **Prove:** the materialized count (3900) is exactly `C(N,2) * n` for Agreement-only; it is NOT `n * C(N,2) * 4` (all four kinds). This confirms storage `O(n·N)` for Agreement scalars, not `O(n·N²)`. Evidence posted to PH27 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
