# PH21 · T01 — CapabilityCard struct + ProbeSet

| Field | Value |
|---|---|
| **Phase** | PH21 — Capability cards / profile |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile.rs` (≤500) |
| **Depends on** | PH20 T01 (Registry + LensSpec) |
| **Axioms** | A6, A17 |
| **PRD** | `dbprdplans/05 §5` |

## Goal

Define the `CapabilityCard` struct and `ProbeSet` type that `profile()` operates
on. Every field is faithfully present from `05 §5`; `signal` and
`differentiation` are `Option<f32>` until Assay (Stage 5) is wired. All
fields are serializable to a stable JSON representation.

## Build (checklist of concrete, code-level steps)

- [x] `CapabilityCard` struct (derive `Debug`, `Clone`, `Serialize`,
  `Deserialize`):
  ```
  pub struct CapabilityCard {
      pub lens_id: LensId,
      pub name: String,
      pub signal: Option<f32>,          // bits about anchors — None until Assay (PH29)
      pub proxy_signal: f32,            // Registry probe estimate, never trusted as Assay
      pub differentiation: Option<f32>, // max pairwise corr vs panel — None until Assay
      pub proxy_differentiation: f32,   // Registry probe estimate, never trusted as Assay
      pub spread: SpreadMetrics,
      pub separation: f32,              // silhouette score in [-1, 1]
      pub cost: CostMetrics,
      pub coverage: f32,                // fraction of probe inputs non-degenerate
      pub collapsed: bool,              // true if participation_ratio < COLLAPSE_THRESHOLD
  }
  ```
- [x] `SpreadMetrics` struct: `participation_ratio: f32`, `stable_rank: f32`.
- [x] `CostMetrics` struct: `ms_per_input: f32`, `vram_mb_estimated: u32`,
  `batch_ceiling: u32`.
- [x] `COLLAPSE_THRESHOLD: f32 = 0.05` — a lens with `participation_ratio < 0.05`
  is considered collapsed (empirically from ContextGraph embedder probe suite).
- [x] `ProbeSet` struct: `inputs: Vec<Input>`, `labels: Option<Vec<String>>`
  (labels needed for silhouette and signal; if None, silhouette is skipped,
  signal is None).
- [x] `ProbeSet::min_size() -> usize { 50 }` — document as minimum for
  meaningful spread metrics.
- [x] Serde test: `CapabilityCard` with known values round-trips to/from JSON
  byte-exactly.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: construct a `CapabilityCard` with all fields populated; serialize
  to JSON; deserialize; assert equality.
- [x] unit: `collapsed` field is `true` when `participation_ratio < 0.05`,
  `false` when `≥ 0.05`.
- [x] unit: `signal: None` serializes as `"signal": null` in JSON (not absent
  key, not 0.0).
- [x] edge (≥3): (1) `ProbeSet` with 0 inputs is valid to construct (but
  `profile` will return a coverage=0.0 card); (2) `SpreadMetrics` with both
  fields = 0.0 → `collapsed=true`; (3) `CostMetrics` with vram_mb_estimated=0
  (CPU-only lens).
- [x] fail-closed: N/A — struct is pure data; no error paths here.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** serde JSON output of a `CapabilityCard` with known values
- **Readback:** `cargo test -p calyx-registry capability_card -- --nocapture 2>&1`
- **Prove:** test output shows the JSON representation with `"signal":null`,
  `"collapsed":false` for a healthy spread, `"collapsed":true` for low-ratio;
  screenshot attached to PH21 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH21 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
