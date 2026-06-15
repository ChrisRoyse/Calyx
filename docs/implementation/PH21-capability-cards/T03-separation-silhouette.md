# PH21 · T03 — Separation metric (silhouette)

| Field | Value |
|---|---|
| **Phase** | PH21 — Capability cards / profile |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile/separation.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A6 |
| **PRD** | `dbprdplans/05 §5` |

## Goal

Compute the silhouette score over labeled probe embeddings: "does this lens
cluster the outcome axis cleanly?" Silhouette ∈ [-1, 1]; high positive = clean
separation; near zero = random; negative = wrong clustering. If labels are
absent, return 0.0 and document that silhouette was skipped.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn silhouette_score(embeddings: &[Vec<f32>], labels: &[String]) -> Result<f32>`:
  - validate `embeddings.len() == labels.len()`; if not →
    `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` with "silhouette requires matching
    embeddings and labels lengths".
  - if `embeddings.len() < 2` → return `Ok(0.0)` (undefined; skip).
  - group embeddings by label.
  - for each embedding `i`:
    - `a_i = mean cosine distance to all embeddings in same label group`.
    - `b_i = min over all other groups of mean cosine distance to that group`.
    - `s_i = (b_i - a_i) / max(a_i, b_i)`.
  - return `mean(s_i)` over all embeddings.
  - complexity is O(N^2) — acceptable for probe sets ≤ 500; for larger sets,
    subsample to 500.
- [x] `pub fn separation_metric(embeddings: &[Vec<f32>], labels: Option<&[String]>) -> Result<f32>`:
  - if labels is `None` → return `Ok(0.0)`.
  - call `silhouette_score(embeddings, labels)`.
- [x] Use cosine distance `d = 1 - cos(u, v)` = `1 - dot(u, v)` for
  unit-normed vectors (no sqrt needed).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two tight clusters in 2-D, well-separated → silhouette > 0.5.
- [x] unit: two completely interleaved clusters (random permutation) → silhouette
  near 0.0 (within ±0.15 for seed=42).
- [x] unit: single label (all same class) → silhouette = 0.0 (a_i is defined
  but b_i is undefined → return 0.0).
- [x] unit: `labels = None` → returns `Ok(0.0)`.
- [x] edge (≥3): (1) N=2 same label → 0.0 (no b_i); (2) N=2 different labels
  → silhouette ∈ [-1, 1]; (3) all embeddings identical → a_i = 0, b_i = 0
  → silhouette = 0.0 (avoid 0/0).
- [x] fail-closed: mismatched lengths → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** unit test output on aiwonder
- **Readback:** `cargo test -p calyx-registry silhouette -- --nocapture 2>&1`
- **Prove:** test output shows well-separated cluster silhouette > 0.5 and
  interleaved cluster ≈ 0.0; both printed with 4 decimal places; screenshot
  attached to PH21 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH21 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
