# 12 - probe matrix

- **Issue:** #879   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic probe provider while #869 corpus ingest runs
- **Goal:** make probe variation reusable by materializing the fusion x phrasing x length x lens-emphasis matrix and logging which combinations surface unique grounded hits.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue879_probe_matrix_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue879-20260625T110753Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo"
scp issue879-20260625T110753Z.tar aiwonder:/home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo.tar -C /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z cargo test -p calyx-lodestar --test issue879_probe_matrix_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/probe_matrix.rs`
- `crates/calyx-lodestar/tests/issue879_probe_matrix_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue879_probe_matrix_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z`
- Artifact: `/home/croyse/calyx/fsv/issue879-probe-matrix-20260625T110753Z/issue879_probe_matrix_readback.json`
- Artifact bytes: `6012`
- Artifact SHA256: `921ea0a7efeef4d7661781228796e9f9aec589248d1e44783d1e98b7c1657b61`
- Readback scalar leaves:
  - `schema_version=1`
  - `variant_count=5`
  - `productive_count=1`
  - `top_productive_fusion=WeightedRrf`
  - `refusal_count=1`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Matrix count is the expected cross-product of kernel-first, RRF, weighted-RRF profiles, single-lens slots, pipeline, phrasing, and length axes.
- A grounded hit shared by multiple probes is not counted as unique productivity.
- A unique grounded hit surfaced only by weighted-RRF is recorded as the productive combination.
- An ungrounded hit is not counted as a productive unique grounded hit.
- Refusals are logged alongside probe records.
- Empty frontier and non-finite hit score fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The probe matrix now exists as a serializable Lodestar planner and run log.
- The planner uses Calyx's existing fusion vocabulary: kernel-first, RRF, weighted-RRF, single-lens, and pipeline.
- Lens emphasis reuses Sextant `RrfProfile` values and explicit `SlotId` single-lens probes.
- The synthetic FSV proves durable readback of a matrix run with one productive weighted-RRF combination and one refusal.
- This is not yet the final #879 anchored-corpus acceptance. Productive real biomedical combinations can only be logged after #869/#870/#871 provide the real anchored search/graph substrate.

## Conclusion & next step
The reusable #879 probe-matrix surface is ready for the discovery harness. Keep #879 open until the matrix runs against the anchored corpus and logs productive fusion x phrasing x lens combinations from real Calyx search results.
