# PH23 · T04 — Dual-index scaffold for asymmetric slots

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/dual.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH20 (asymmetric slot definitions) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/03 §4` |

## Goal

Provide a `DualHnswIndex` that wraps two `HnswGraph` instances (side `a` and
side `b`) for asymmetric/causal slots. Directional boost multipliers live in the
index config, not in fusion, so the `Index` trait surface is unchanged. This
scaffold is the foundation for the "what caused X?" vs "what did X cause?"
traversal mode (`10 §4`).

## Build (checklist of concrete, code-level steps)

- [x] `DualHnswIndex` struct:
  ```rust
  pub struct DualHnswIndex {
      a: HnswGraph,   // e.g. cause-side
      b: HnswGraph,   // e.g. effect-side
      boost_a: f32,   // directional weight multiplier, default 1.0
      boost_b: f32,
  }
  ```
- [x] Implement `Index` for `DualHnswIndex`:
      - `insert` routes by a caller-supplied `Direction` enum
        (`Direction::A | Direction::B`); insert into the appropriate sub-index
      - `search` takes a `Direction` hint embedded in the query (or defaults to
        `A`); applies `boost_a`/`boost_b` to raw scores before returning
      - `remove` removes from both sub-indexes (idempotent if absent in one)
      - `len` returns `a.len() + b.len()` (may count duplicates if the same cx
        is in both; document this)
      - `dim` asserts `a.dim == b.dim`, returns either
- [x] `Direction` enum in `index/mod.rs` (not a separate file — small)
- [x] `CALYX_SEXTANT_DIRECTION_MISMATCH` error for `dim` conflict between a and b
- [x] Config struct `DualHnswConfig { m, ef_construction, boost_a, boost_b, dim }`
      with a `Default` impl (m=16, ef=200, boosts=1.0)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert 10 nodes into side A, 10 into side B → `len() == 20`
- [x] unit: search Direction::A returns only side-A cx ids
- [x] unit: search Direction::B with `boost_b=2.0` → scores are ×2 vs baseline
- [x] proptest: `remove` from a dual index never panics for any valid `CxId`
- [x] edge: construct with mismatched dims → `CALYX_SEXTANT_DIRECTION_MISMATCH`
- [x] edge: insert with `Direction::A` then search `Direction::B` on side with no
      insertions → `CALYX_SEXTANT_INDEX_EMPTY`
- [x] fail-closed: `boost_a = 0.0` → search still runs, scores are 0.0, no NaN,
      no panic (zero-boost is valid — PH26 planner may set it)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant dual -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant dual -- --nocapture 2>&1`
- **Prove:** before — no `DualHnswIndex` type; after — all dual tests pass,
  directional score multipliers produce the expected numeric output (printed in
  the `--nocapture` run and matched against golden constants)

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
