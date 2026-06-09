# PH24 · T01 — `Hit` struct + `Query` struct

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/hit.rs` (≤500), `crates/calyx-sextant/src/query.rs` (≤500) |
| **Depends on** | PH23 T01 (`SlotId`, `Index`), PH03 (`CxId`), PH35 stub (`LedgerRef`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/10 §1`, `dbprdplans/10 §5` |

## Goal

Define the two central data types for Sextant: `Hit` (every result row, always
provenance-carrying) and `Query` (the full lens-aware query model). These types
are the stable API surface that PH25–PH26 and all downstream crates depend on;
get them right here so later phases only add, never restructure.

> Post-stage correction: PH24 reserved the guarded-search surface, but the real
> `InRegionOnly(GuardProfile)` implementation was deferred to Ward blindspot
> #276. Stage 4 did not ship final guarded search; PH38 T06 owns the live code
> and FSV.

## Build (checklist of concrete, code-level steps)

- [x] `crates/calyx-sextant/src/hit.rs`:
  ```rust
  pub struct PerLensEntry {
      pub slot: SlotId,
      pub rank: u32,
      pub raw_score: f32,
      pub weight: f32,
      pub contribution: f32,  // weight * (1/(rank+60)) for RRF; weight*raw for SingleLens
  }

  pub struct GuardInfo {
      pub pass: bool,
      pub per_slot_cos: Vec<(SlotId, f32)>,
  }

  pub struct FreshnessTag {
      pub built_at_seq: u64,
      pub stale_by: Option<u64>,
  }

  pub enum FreshnessPolicy { FreshDerived, StaleOk { seq_lag: u64 } }

  pub struct Hit {
      pub cx_id: CxId,
      pub fused_score: f32,
      pub per_lens: Vec<PerLensEntry>,
      pub cross_terms_used: Vec<CxId>,
      pub guard: Option<GuardInfo>,
      pub provenance: LedgerRef,
      pub freshness: FreshnessTag,
  }
  ```
- [x] `crates/calyx-sextant/src/query.rs`:
  ```rust
  pub enum QueryInput { Text(String), Vector(Vec<f32>), Anchor(CxId) }
  pub enum LensSelection { Auto, Explicit(Vec<SlotId>) }
  pub enum FusionStrategy { SingleLens(SlotId), Rrf, WeightedRrf(String), KernelFirst, Pipeline }
  pub struct RerankSpec { pub endpoint: String, pub top_k_candidates: usize }

  pub struct Query {
      pub vault: VaultId,
      pub input: QueryInput,
      pub lenses: LensSelection,
      pub fusion: FusionStrategy,
      pub filters: Vec<Predicate>,           // scalar/anchor/metadata predicates
      pub guard: GuardMode,
      pub freshness: FreshnessPolicy,
      pub k: usize,
      pub rerank: Option<RerankSpec>,
      pub explain: bool,
  }
  ```
- [x] Historical placeholder only: final `InRegionOnly(GuardProfile)` guarded
      search is PH38 T06 / #276, not PH24 FSV evidence
- [x] `Predicate` type: opaque for now (`pub struct Predicate(pub String)`) —
      full predicate parser is Stage 12; this stub compiles and round-trips
- [x] Derive `Debug`, `Clone`, `serde::Serialize`, `serde::Deserialize` on all
      public structs

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `Hit` serde round-trip — serialize to JSON, deserialize, assert all
      fields equal (use a hand-crafted `Hit` with known `LedgerRef::stub`)
- [x] unit: `Query` serde round-trip for each `FusionStrategy` variant
- [x] unit: `PerLensEntry` contribution formula: for RRF, `contribution == weight
      * 1.0 / (rank as f32 + 60.0)` — assert with f32 tolerance 1e-6
- [x] edge: `Hit` with empty `per_lens` compiles and serializes without panic
- [x] edge: `FreshnessPolicy::StaleOk { seq_lag: 0 }` is distinct from
      `FreshDerived` (not unified)
- [x] fail-closed: deserializing a `Hit` JSON with a missing `provenance` field →
      serde returns `Err`, not a zero/default `LedgerRef`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-sextant hit_query -- --nocapture` on aiwonder
- **Readback:** `cargo test -p calyx-sextant hit_query -- --nocapture 2>&1`
- **Prove:** serde round-trip test prints `round_trip_ok=true` for both `Hit`
  and `Query`; contribution formula test prints `expected=NNN got=NNN ok=true`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
