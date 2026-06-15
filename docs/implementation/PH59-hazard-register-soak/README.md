# PH59 — 25-hazard register FSV + soak

**Stage:** S13 — Resource, GC & Reliability Hardening  ·  **Crate:** cross-crate  ·
**PRD roadmap:** RESOURCE  ·  **Axioms:** A26

## Objective

Every hazard in PRD `24 §7` (the full 25-row register) has a passing byte-level FSV on
aiwonder. A 1e7-op soak shows RSS and VRAM bounded, no leak, no oscillation. No
green-checkmark harness counts — only the before/after byte readback from the actual SoT
(metric series, SST bytes, disk free, `dmesg`) is the verdict. Each hazard is driven,
the mitigation confirmed, and the evidence attached to the PH59 GitHub issue. This is the
final gate for Stage 13 and for the RESOURCE predicate of `BUILD_DONE`.

## Dependencies

- **Phases:** PH56 (bounded allocators — hazards 2, 8, 17), PH57 (VRAM budgeter — hazard 7),
  PH58 (GC reclaimers + watchdog — hazards 1, 3, 4, 5, 6, 21)
- **Also draws on:** PH14 (TurboQuant — hazards 10, 11), PH13 (CUDA — hazard 9),
  PH23 (HNSW — hazard 12), PH35/PH36 (Ledger — hazard 22), PH43 (Anneal — hazard 20),
  PH66 (restic — hazard 24), PH38 (Ward — no direct hazard but informs 22)
- **Provides for:** Stage 13 exit; RESOURCE predicate of BUILD_DONE

## Current state (build off what exists)

All mitigation infrastructure from PH56–PH58 and prior phases is in place by the time PH59
runs. PH59's job is purely FSV: drive each hazard's test scenario, read the bytes, record
evidence. Cross-crate: hazard test binaries live in `crates/calyx-aster/tests/hazard_*.rs`,
`crates/calyx-forge/tests/hazard_*.rs`, and a `crates/calyx-hazard-soak/` integration test
crate that orchestrates the full 25-hazard sweep and the final 1e7-op soak.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-hazard-soak/src/main.rs` | Hazard soak orchestrator; runs all 25 hazard probes + final soak |
| `crates/calyx-hazard-soak/src/hazards/resource.rs` | Hazards 1–8, 17, 18 (resource/operational) |
| `crates/calyx-hazard-soak/src/hazards/numerical.rs` | Hazards 9–12 (numerical/index) |
| `crates/calyx-hazard-soak/src/hazards/operational.rs` | Hazards 13–16, 19–21 (operational/concurrency) |
| `crates/calyx-hazard-soak/src/hazards/security.rs` | Hazards 22–25 (security/upgrade) |
| `crates/calyx-hazard-soak/src/soak.rs` | 1e7-op soak; RSS + VRAM bounded, no oscillation |
| `crates/calyx-hazard-soak/Cargo.toml` | Dev dependency on all calyx crates |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Hazards 1–5: compaction storm, flush stall, tombstone buildup, fsync spike, WAL bloat | PH56, PH58 |
| T02 | Hazards 6–8: MVCC version pile-up, VRAM OOM, heap OOM | PH56, PH57, PH58 |
| T03 | Hazards 9–12: NaN propagation, quant drift, codebook staleness, ANN corruption | PH13, PH14, PH23 |
| T04 | Hazards 13–16: hot-shard skew, lock contention, cache stampede, slow-lens HOL | PH09, PH56 |
| T05 | Hazards 17–21: disk full, ARC thrash, clock skew, Anneal thrash, panel explosion | PH56, PH58, PH43 |
| T06 | Hazards 22–25: secret leakage, nondeterminism, whole-host loss, upgrade skew | PH35, PH36, PH66 |
| T07 | Final 1e7-op soak — RSS/VRAM bounded, no leak, no oscillation | T01–T06 |
| T08 | doc-23 `compression_report(vault)` honest-numbers artifact + readback | PH14, PH42, Ward/Assay evidence |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

All 25 hazard probes pass their byte-level FSV on aiwonder:

```
cargo run --release --bin calyx-hazard-soak -- --all-hazards 2>&1 | tee /tmp/ph59_hazard_results.json
calyx readback --metric hazard_pass_count
```

- `hazard_pass_count == 25` (all 25 rows verified)
- `rss_bounded == true`, `vram_bounded == true`, `oscillation_detected == false` from the soak
- No OOM kill in `dmesg`; no panics in the log
- `ph59_hazard_results.json` attached to the PH59 GitHub issue

## Risks / landmines

- **Hazard 24 (whole-host loss):** DR drill requires restic restore — coordinate with PH66
  (infra stage); if PH66 is not complete, stub this hazard's FSV with a simulated restore
- **Hazard 22 (secret leakage):** scanning persisted bytes for secrets requires knowing what
  "secret" looks like; use a synthetic token `CALYX_TEST_SECRET_ABCD1234` injected into
  a reranker request, then scan all persisted files — it must not appear anywhere
- **Hazard 9 (NaN):** injecting NaN into GPU kernels may require a custom test shader on
  sm_120; ensure the NaN guard fires at the kernel boundary, not silently propagates
- **Determinism mode (hazard 23):** must run with `CALYX_DETERMINISM=1` env var; replay must
  be bit-parity within ≤ 1e-3 tolerance; floating-point non-associativity is the main risk
- **Hazard 19 (clock skew):** server-stamped monotonic seq means ordering is seq-based, not
  wall-clock; inject a 30-second wall-clock skew and verify no ordering inversion in seq space
