# PH59 · T03 — Hazards 9–12: NaN propagation, quant drift, codebook staleness, ANN corruption

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/numerical.rs` (≤500) |
| **Depends on** | PH13 (CUDA backend + NaN guard), PH14 (TurboQuant + QJL seed versioning), PH23 (HNSW + degraded flag) |
| **Axioms** | A26, A16, A25 |
| **PRD** | `dbprdplans/24 §7` hazards 9–12; `24 §5` |

## Goal

Drive hazards 9 (NaN/Inf propagation through GPU kernels), 10 (quantization drift beyond
tolerance), 11 (codebook/rotation staleness — TurboQuant is data-oblivious, QJL seed
versioned), and 12 (ANN/kernel index corruption + rebuild), read the SoT bytes, prove each
mitigation. TurboQuant is largely N/A for codebook drift (data-oblivious rotation), but
QJL seed version must be FSV-proven. Defends intelligence integrity at the numerical layer.

## Build (checklist of concrete, code-level steps)

**Hazard 9 — NaN/Inf propagation:**
- [ ] `fn probe_h9_nan_propagation(forge: &Forge) -> HazardResult`:
  - Construct an embedding vector with one NaN element at position 0
  - Submit to `Forge` CUDA matmul kernel
  - Verify `CALYX_FORGE_NUMERICAL_INVARIANT` is returned (kernel-boundary NaN guard fires)
  - Verify the NaN does not propagate into any persisted SST slot column (readback the slot CF — must be absent or contain the error-flagged entry, not a NaN value)
  - Record `calyx_nan_guard_trips_total` counter increment

**Hazard 10 — Quantization drift:**
- [ ] `fn probe_h10_quant_drift(forge: &Forge, vault: &Vault) -> HazardResult`:
  - Embed 1000 synthetic constellations with known cosine similarity structure
  - Apply TurboQuant at the chosen bit-width; compare inner-product before/after
  - Verify `|ip_before - ip_after| / ip_before ≤ distortion_bound` (from `23 §4.4`) for all pairs
  - Verify `recall@10` of quantized HNSW ≥ `recall@10` of full-precision × 0.95 (within intelligence contract)
  - Record the before/after cosine distribution for FSV attachment

**Hazard 11 — Codebook/rotation staleness:**
- [ ] `fn probe_h11_codebook_staleness(forge: &Forge) -> HazardResult`:
  - Re-quant the same input with the same QJL seed → verify bit-identical output (seed versioned, deterministic)
  - Re-quant with a different seed → verify output differs (seed matters)
  - Verify `re_quant_parity == true` for same-seed, `false` for different-seed
  - TurboQuant (data-oblivious rotation) has no codebook → assert `codebook_staleness_na == true` in result

**Hazard 12 — ANN/kernel index corruption:**
- [ ] `fn probe_h12_ann_corruption(vault: &mut Vault) -> HazardResult`:
  - Flip 8 bytes in the HNSW graph file at a known offset (simulate bit-rot)
  - Open the vault; attempt a search; verify `degraded` flag is set (not a panic)
  - Verify background rebuild starts (Anneal degraded-flag handler from PH44)
  - After rebuild: verify search returns correct results (readback: recall vs brute-force ≥ target)
  - Verify no data loss in base CF (only graph index was corrupt, base is WAL-protected)
  - Record `calyx_ann_degraded_rebuilds_total`

- [ ] Aggregate into `target/ph59_hazards_9_12.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H9: `nan_guard_trips_total` increments by exactly 1; slot CF does not contain NaN bytes (verify with `xxd` on the slot file or `calyx readback --slot` command); `CALYX_FORGE_NUMERICAL_INVARIANT` in response
- [ ] H10: `|ip_before - ip_after| ≤ distortion_bound` for all 1000 pairs (seeded, deterministic assertion); `recall@10_quantized ≥ recall@10_full × 0.95`
- [ ] H11: same-seed re-quant is bit-identical (byte comparison of output buffers); different-seed produces different bytes (collision probability negligible with seeded PRG)
- [ ] H12: `degraded` flag in vault metadata after byte-flip; recall after rebuild ≥ target; `ann_degraded_rebuilds_total == 1`
- [ ] edge: H9 — entire vector of NaN → `CALYX_FORGE_NUMERICAL_INVARIANT`; no output bytes written
- [ ] edge: H10 — quantize at the minimum bit-width (1 bit); verify distortion is measured (even if large); contract fires `CALYX_QUANT_DRIFT_EXCEEDED` if beyond bound
- [ ] fail-closed: H12 — corrupt graph, degraded mode; search still returns results (degraded, not failed completely); rebuilds to full recall

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_9_12.json`; `calyx readback --metric nan_guard_trips_total`; `xxd /hotpool/vault_*/sst/slot_0_<seq>.sst | grep -i 'nan\|ff ff ff ff'` (IEEE 754 NaN pattern); `calyx readback --metric ann_degraded_rebuilds_total`
- **Readback:**
  ```
  calyx readback --metric nan_guard_trips_total
  calyx readback --metric ann_degraded_rebuilds_total
  xxd /hotpool/vault_test/sst/slot_0_latest.sst | head -20   # must not contain NaN
  cat target/ph59_hazards_9_12.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] for h in d))"
  ```
- **Prove:** all four hazards report `passed: true`; `nan_guard_trips_total >= 1`; slot SST bytes do not contain the IEEE 754 NaN pattern (`0x7FC00000` for f32); `ann_degraded_rebuilds_total == 1` after the byte-flip probe. Attach JSON + readback + xxd snippet to PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (Forge-touching probes)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
