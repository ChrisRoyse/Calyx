# 04 - Kernel build

- **Issue:** #871   **Phase:** CPU-safe pre-corpus slice   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic kernel artifact / corpus pending #869 and #870
- **Goal:** Verify the existing kernel-build, recall-gate, persisted artifact, and kernel-health readback path before running it on the anchored corpus.

## What was run (exact commands)

```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue871_kernel_build_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder archived-source FSV
git archive --format=tar -o issue871-20260625T123814Z-base.tar HEAD
git diff --cached --binary > issue871-20260625T123814Z.patch
ssh aiwonder "rm -rf /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z && mkdir -p /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo"
scp issue871-20260625T123814Z-base.tar aiwonder:/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo-base.tar
scp issue871-20260625T123814Z.patch aiwonder:/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/issue871.patch
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo-base.tar -C /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && git init -q && git apply /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/issue871.patch"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z; cd \"$root/repo\" && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue871_kernel_build_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV

Implementation source:
- `crates/calyx-lodestar/tests/issue871_kernel_build_tests.rs`

The synthetic FSV path:
- Builds a three-node directed cycle with `target_fraction=1.0`.
- Anchors the selected DFVS member.
- Runs `build_kernel_pipeline`.
- Builds a `KernelIndex`.
- Runs `kernel_recall_gate` with `min_recall_ratio=0.95`.
- Persists both `index.json` and `kernel.json` through `FsKernelStore`.
- Reads `kernel.json` through `read_kernel_artifact`.
- Reads health fields through `kernel_health`, which reads the persisted artifact instead of recomputing.

Expected scalar leaves from the happy readback:
- `source_graph.node_count=3`
- `source_graph.edge_count=3`
- `member_count=1`
- `kernel_graph_count=3`
- `groundedness_fraction=1.0`
- `recall_ratio=1.0`
- `tau_star_estimate=1`
- `tau_star_exact=true`
- `health.recall.pass_mode=passed`
- `health.grounded_fraction=1.0`

Boundary and edge behavior covered:
- Recall below A10 gate fails closed with `CALYX_KERNEL_RECALL_BELOW_GATE`.
- Missing kernel embedding fails closed with `CALYX_KERNEL_EMBEDDING_MISSING`.
- Empty held-out corpus fails closed with `CALYX_RECALL_EMPTY_CORPUS`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z`
- Patch bytes: `11026`
- Base archive bytes: `28753920`
- Happy artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/happy/issue871_kernel_build_readback.json`
- Happy artifact bytes: `1094`
- Happy artifact SHA256: `13ce0a6f7b704fd018a440ddd51150e562cbaafdce84f03a47b356bc56836743`
- Happy scalar leaves: `source_nodes=3`, `source_edges=3`, `kernel_file_bytes=1561`, `index_file_bytes=219`, `member_count=1`, `kernel_graph_count=3`, `groundedness_fraction=1.0`, `recall_ratio=1.0`, `tau_star_estimate=1`, `tau_star_exact=true`, `health_recall_pass_mode=passed`, `health_recall_n_queries=1`
- Recall-fail artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/edges/issue871_kernel_recall_fail.json`
- Recall-fail artifact bytes: `166`
- Recall-fail artifact SHA256: `7ff7c84e9276cc791fe570b1811871ff635e57792702cbd90213f287147c3526`
- Recall-fail scalar leaves: `error_code=CALYX_KERNEL_RECALL_BELOW_GATE`, `kernel_member=01010101010101010101010101010101`, `full_top_expected=09090909090909090909090909090909`
- Error artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-20260625T123814Z/edges/issue871_kernel_build_errors.json`
- Error artifact bytes: `106`
- Error artifact SHA256: `937ce758db81eb847e5a8f6dee3f015e58a05425fd2ecaba73b2f1aad5c70b41`
- Error scalar leaves: `missing_embedding=CALYX_KERNEL_EMBEDDING_MISSING`, `empty_corpus=CALYX_RECALL_EMPTY_CORPUS`
- aiwonder tests: 3 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder final live-checkout FSV after dev push:
- Dev commit: `28feb5dd`
- FSV root: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z`
- Happy artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/happy/issue871_kernel_build_readback.json`
- Happy artifact bytes: `1094`
- Happy artifact SHA256: `13ce0a6f7b704fd018a440ddd51150e562cbaafdce84f03a47b356bc56836743`
- Happy scalar leaves: `source_nodes=3`, `source_edges=3`, `kernel_file_bytes=1561`, `index_file_bytes=219`, `member_count=1`, `kernel_graph_count=3`, `groundedness_fraction=1.0`, `recall_ratio=1.0`, `tau_star_estimate=1`, `tau_star_exact=true`, `health_recall_pass_mode=passed`, `health_recall_n_queries=1`
- Recall-fail artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/edges/issue871_kernel_recall_fail.json`
- Recall-fail artifact bytes: `166`
- Recall-fail artifact SHA256: `7ff7c84e9276cc791fe570b1811871ff635e57792702cbd90213f287147c3526`
- Recall-fail scalar leaves: `error_code=CALYX_KERNEL_RECALL_BELOW_GATE`, `kernel_member=01010101010101010101010101010101`, `full_top_expected=09090909090909090909090909090909`
- Error artifact: `/home/croyse/calyx/fsv/issue871-kernel-build-final-20260625T124200Z/edges/issue871_kernel_build_errors.json`
- Error artifact bytes: `106`
- Error artifact SHA256: `937ce758db81eb847e5a8f6dee3f015e58a05425fd2ecaba73b2f1aad5c70b41`
- Error scalar leaves: `missing_embedding=CALYX_KERNEL_EMBEDDING_MISSING`, `empty_corpus=CALYX_RECALL_EMPTY_CORPUS`
- aiwonder live tests: 3 passed, 0 failed, 0 ignored.
- aiwonder live `cargo fmt --all -- --check`: exit 0.
- aiwonder live `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

## Findings (honest)

- The existing kernel pipeline, recall gate, artifact write/read, and health readback are sufficient to record the #871 acceptance metrics once #869 and #870 produce the real anchored association graph.
- This is not final #871 acceptance. No real anchored corpus kernel was built yet; the real MFVS members, groundedness fraction, recall ratio, and `tau_star` must still be read from the live Calyx source-of-truth bytes.

## Conclusion & next step

After #869 completes and #870 materializes the real association graph, run this pattern against the live corpus graph and record the real kernel artifact, `groundedness_fraction`, recall gate pass/fail, and `tau_star` values here.
