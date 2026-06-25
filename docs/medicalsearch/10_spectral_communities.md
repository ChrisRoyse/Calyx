# 10 - spectral communities

- **Issue:** #877   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `AssocGraph` while #869 corpus ingest runs
- **Goal:** expose latent agreement-graph communities through Fiedler bisection and rank inter-community bridge edges plus eigenvector-centrality proposers.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue877-20260625T114455Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo"
scp issue877-20260625T114455Z.tar aiwonder:/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo.tar -C /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/spectral_communities.rs`
- `crates/calyx-lodestar/tests/issue877_spectral_communities_tests.rs`
- `crates/calyx-lodestar/src/error.rs` conversion for `CALYX_SPECTRAL_*` errors
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue877_spectral_communities_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z`
- Artifact: `/home/croyse/calyx/fsv/issue877-spectral-communities-20260625T114455Z/issue877_spectral_communities_readback.json`
- Artifact bytes: `4859`
- Artifact SHA256: `2d469e91a7518d9fc3abfb77d7c0e3cf96545ace1abed1325b8af818158e7c90`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z`
- Artifact: `/home/croyse/calyx/fsv/issue877-spectral-communities-final-20260625T114900Z/issue877_spectral_communities_readback.json`
- Artifact bytes: `4859`
- Artifact SHA256: `2d469e91a7518d9fc3abfb77d7c0e3cf96545ace1abed1325b8af818158e7c90`
- Readback scalar leaves:
  - `schema_version=1`
  - `node_count=6`
  - `edge_count=13`
  - `community_count=2`
  - `bridge_candidate_count=1`
  - `centrality_candidate_count=6`
  - `spectral_gap=0.4025992155075073`
  - `top_bridge_src=962248c9b37cc067dad060792ca1e865`
  - `top_bridge_dst=1279c8633841c89a2f8ccb64620effcb`
  - `top_bridge_rank_score=0.9499999284744263`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Planted two-clique graph partitions into two three-member communities through the Fiedler vector.
- Ranked inter-community bridge candidate is the single cross-community association edge.
- Eigenvector-centrality proposer list is emitted independently from bridge-edge ranking.
- `max_bridge_candidates` and `max_centrality_candidates` truncate after deterministic ranking.
- `eigen_k < 2` and zero candidate limits fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- One-node graphs fail closed through `CALYX_SPECTRAL_GRAPH_TOO_SMALL`.

## Findings (honest)
- Lodestar now has a serializable spectral-community report over `AssocGraph`.
- The report reuses `calyx-mincut` Laplacian eigenmaps, Fiedler bisection, spectral gap, and eigenvector centrality; it does not hand-roll spectral math.
- Bridge candidates are graph-structural hypotheses only. They are ranked by edge weight, endpoint centrality, and endpoint frequency, and carry provenance strings.
- Centrality candidates are independent proposers ranked by eigenvector centrality, degree, and frequency.
- This is not yet final #877 anchored-corpus acceptance. The real community partition and inter-community biomedical bridge list require #869 anchored ingest, #870 association graph weaving, and #871 kernel grounding.

## Conclusion & next step
The #877 implementation/report surface is ready for corpus use. Keep #877 open until the real anchored association graph exists and the spectral report is run on aiwonder against that graph with persisted readback evidence.
