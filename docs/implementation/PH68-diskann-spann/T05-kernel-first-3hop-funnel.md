# PH68 · T05 — Kernel-first 3-hop funnel for huge vaults (1e8+)

| Field | Value |
|---|---|
| **Phase** | PH68 — DiskANN dense + SPANN sparse |
| **Stage** | S17 — Scale: DiskANN + SPANN |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/funnel.rs` (≤500) |
| **Depends on** | T02 (this phase — DiskAnnSearch), T03 (this phase — SpannSearch), PH33 (kernel index + kernel_answer — provides the kernel cx set and its ANN), PH24 (RRF/KernelFirst fusion strategy enum) |
| **Axioms** | A10, A11, A16, A32 |
| **PRD** | `dbprdplans/10 §2` (KernelFirst strategy), `dbprdplans/10 §8` (KernelFirst@1e8 p99 < 25 ms), `dbprdplans/04 §3` |

## Goal

Implement the kernel-first 3-hop funnel that keeps huge-vault (1e8+) queries
sublinear: **hop 1** — ANN over the kernel-of-regions (tiny set, in-RAM, from
PH33); **hop 2** — DiskANN beam search within the identified regions; **hop 3** —
DiskANN/SPANN final cx retrieval within each region candidate. `KernelFirstSearch`
is the `SearchStrategy` behind the `KernelFirst` fusion mode in `10 §2`. The
3-hop funnel is the mechanism that delivers `KernelFirst@1e8 p99 < 25 ms`.

> **Scale boundary:** the 3-hop funnel is designed for 1e8+ cx server vaults only.
> For vaults < 1e7 cx the query planner bypasses it and uses direct HNSW (PH23).
> Any code path that activates the funnel on an embedded vault is a bug.

## Build (checklist of concrete, code-level steps)

- [ ] Define `FunnelParams { n_kernel_probe: usize, n_region_beam: usize, n_cx_beam: usize, n_regions_to_expand: usize }`: all four are Anneal-tunable; sensible defaults `{8, 32, 64, 4}`
- [ ] Define `KernelFirstSearch`: holds a reference to the kernel ANN (in-RAM, from PH33 `KernelIndex`), a region-level `DiskAnnSearch` (built over region centroids on disk), and the cx-level `DiskAnnSearch` / `SpannSearch` per slot
- [ ] Implement **hop 1**: `fn probe_kernel(&self, query: &[f32], params: &FunnelParams) -> Vec<KernelRegionId>` — ANN search over the kernel index returns `n_kernel_probe` candidate kernel regions; the kernel index is always in-RAM (tiny, `≈1%` of vault, per PH33 design)
- [ ] Implement **hop 2**: `fn expand_regions(&self, kernel_hits: &[KernelRegionId], query: &[f32], params: &FunnelParams) -> Vec<RegionCandidate>` — for each kernel hit, beam search the region-level DiskANN graph (stored at `idx/regions.ann/`) with beamwidth `n_region_beam`; collect `n_regions_to_expand` top regions by score
- [ ] Implement **hop 3**: `fn search_within_regions(&self, regions: &[RegionCandidate], query: &[f32], k: usize, params: &FunnelParams) -> Result<Vec<(u32, f32)>, CalyxError>` — for each candidate region, issue DiskANN/SPANN beam search restricted to that region's cx partition; merge results, return top-k by score
- [ ] Implement `fn search(&self, query: &[f32], k: usize, params: &FunnelParams) -> Result<Vec<(u32, f32)>, CalyxError>` composing hops 1–3; tag each returned hit with `provenance: FunnelPath { kernel_region, region, cx }` (for `explain=true` in `10 §5`)
- [ ] Guard: if `vault.cx_count < FUNNEL_MIN_VAULT_SIZE` (compile-time constant, default `1e7 as u64`), return `CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL` — caller (query planner) falls back to direct DiskANN; this prevents funnel overhead on small vaults
- [ ] Return `CALYX_INDEX_KERNEL_UNAVAILABLE` if the kernel index has not been built (PH33 dependency not satisfied); `CALYX_INDEX_IO` on any disk read failure during hop 2/3

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 3-hop funnel over a synthetic 1e5-cx vault (small enough for test, large enough to exercise multi-region dispatch) with 100 planted regions, 3 kernel nodes, seed=13 — search `k=10` returns 10 results; all cx_ids < 1e5; no duplicate ids
- [ ] unit: `FunnelPath` provenance — with `explain=true`, each result carries a non-null `kernel_region` field pointing to a valid kernel id; proves hop 1 was used
- [ ] unit: `FUNNEL_MIN_VAULT_SIZE` guard — vault with 5e6 cx returns `CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL` (not a panic, not a silent result)
- [ ] unit: recall check — for a planted query whose nearest-10 are all in region 3, and region 3 is reachable from the nearest kernel node, funnel recall@10 ≥ 0.9 (synthetic, deterministic, seed=21)
- [ ] proptest: for any `n_kernel_probe ∈ [1, kernel_size]`, funnel returns `min(k, total_cx)` results with distinct ids and non-decreasing scores (seed `55u64`)
- [ ] edge: `n_regions_to_expand = 0` → `CALYX_INDEX_INVALID_PARAMS`
- [ ] edge: kernel index empty (0 nodes) → `CALYX_INDEX_KERNEL_UNAVAILABLE`
- [ ] fail-closed: region-level DiskANN file missing → `CALYX_INDEX_IO`; result is not a silent empty Vec but an error

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the 3-hop funnel execution trace on the real 1e8-cx vault on
  `hotpool` NVMe; the SLO measurement in T06 is the definitive gate. This card's
  FSV proves the 3-hop code path is exercised (not bypassed) and returns correct
  results before T06 wires the full soak.
- **Readback:**
  ```
  calyx search \
    --vault /zfs/hot/calyx/ph68-1e8 \
    --strategy KernelFirst \
    --explain \
    --query "synthetic test query" \
    --k 10
  # Output must contain "funnel_path: { kernel_region: ..., region: ..., cx: ... }"
  # proving the 3-hop path was taken, not a fallback
  ```
- **Prove:** the `explain` output for at least one result contains a non-null
  `kernel_region` field; the result cx_ids are readable via `calyx readback`;
  evidence attached to PH68 issue; full p99 SLO proof delegated to T06

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (calyx search --explain output showing 3-hop funnel_path) attached to the PH68 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
