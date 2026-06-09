# PH23 · T01 — `Index` trait + module skeleton

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/lib.rs` (≤500), `crates/calyx-sextant/src/index/mod.rs` (≤500) |
| **Depends on** | — (first card; PH20 for `SlotId`/`LensId` imports, PH13 for `DistanceMetric`) |
| **Axioms** | A16, A4 |
| **PRD** | `dbprdplans/10 §3` |

## Goal

Define the `Index` trait that every per-slot index (HNSW, inverted, dual) must
implement, and wire the crate root. The trait is the seam that lets Stage 17
swap in DiskANN without touching fusion code above it.

## Build (checklist of concrete, code-level steps)

- [x] Add `calyx-sextant` to workspace `Cargo.toml`; add deps `calyx-core`,
      `calyx-registry` (for `SlotId`, `LensId`), `calyx-forge` (for
      `DistanceMetric`), `parking_lot`, `thiserror`
- [x] Create `crates/calyx-sextant/src/index/mod.rs` with:
  ```rust
  pub trait Index: Send + Sync {
      fn insert(&mut self, id: CxId, vec: &[f32]) -> Result<(), CalyxError>;
      fn search(&self, query: &[f32], k: usize, ef: usize) -> Result<Vec<(CxId, f32)>, CalyxError>;
      fn remove(&mut self, id: CxId) -> Result<bool, CalyxError>;
      fn len(&self) -> usize;
      fn is_empty(&self) -> bool { self.len() == 0 }
      fn rebuild(&mut self) -> Result<(), CalyxError>;
      fn dim(&self) -> usize;
  }
  ```
- [x] `CalyxError` variants for this phase: `CALYX_SEXTANT_DIM_MISMATCH`,
      `CALYX_SEXTANT_INDEX_EMPTY`, `CALYX_SEXTANT_EF_TOO_SMALL` (all fail-closed
      per A16, each carrying a `remediation: String` field)
- [x] `crates/calyx-sextant/src/lib.rs`: `pub mod index; pub use index::Index;`
      + crate-level doc comment citing `10 §3`
- [x] `cargo check` + `clippy -D warnings` green with empty stub impls

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `Index` is object-safe — `let _: Box<dyn Index>;` compiles
- [x] unit: error variants carry `remediation` string — assert non-empty on each
      new `CalyxError` arm
- [x] edge: zero-dim vector → `CALYX_SEXTANT_DIM_MISMATCH`
- [x] edge: `search` on empty index → `CALYX_SEXTANT_INDEX_EMPTY`
- [x] fail-closed: passing `ef=0` to a stub impl → `CALYX_SEXTANT_EF_TOO_SMALL`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo check` + `clippy -D warnings` output on aiwonder
- **Readback:** `cargo check -p calyx-sextant 2>&1 | tail -3` → `Finished` line
- **Prove:** before this card — `calyx-sextant` does not compile as a workspace
  member; after — `cargo check` prints `Finished` with zero warnings

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
