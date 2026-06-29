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

## 2026-06-29 real anchored-corpus completion

Issue #877 is now run against the real #869 physical Aster association graph. The 2026-06-25
implementation slice proved the report shape on a planted graph; the 2026-06-29 work removed the
real blocker:

- Root cause: the existing spectral path was an in-memory dense Laplacian path, which is impossible
  for the real graph (`198,993` nodes means roughly 39.6B dense entries before eigen work).
- Production gap: there was no `calyx spectral-communities <vault>` command, no persisted artifact
  under the physical vault, and no separate readback from a source-of-truth file.
- Performance bug found during real FSV: the projected Ritz matrix Jacobi solve used a fixed
  `256`-rotation cap. The real 32-vector Lanczos projection hit `CALYX_SPECTRAL_NOT_CONVERGED`
  before writing an artifact. The fix scales the projected eigensolver budget by matrix size.

Research used:

- Exa: `large sparse graph spectral clustering matrix-free Lanczos Laplacian Fiedler vector best practices`.
- Zhuzhunashvili/Knyazev, "Preconditioned Spectral Clustering..." notes that Lanczos/LOBPCG can be
  matrix-free and only need matrix-vector products, which is the right memory model for large graph
  Laplacians: <https://ar5iv.labs.arxiv.org/html/1708.07481>.
- Dall'Amico/Couillet/Tremblay, "A Unified Framework for Spectral Clustering in Sparse Graphs"
  reiterates the Fiedler-vector basis for two-community Laplacian reconstruction and sparse-graph
  caveats: <https://jmlr.csail.mit.edu/papers/volume22/20-261/20-261.pdf>.
- SciPy/ARPACK docs model the same operational contract: accept a sparse matrix or linear operator,
  use Lanczos-family methods, and fail explicitly on non-convergence:
  <https://docs.scipy.org/doc/scipy/reference/generated/scipy.sparse.linalg.eigsh.html>.

Implementation changes:

- Added `calyx spectral-communities <vault>` with tunable eigen/centrality iteration parameters.
- Source of truth: `<vault>/idx/spectral_communities/<blake3(report_json)>/report.json`.
- Persistence is atomic, refuses overwriting a different explicit output, reads bytes back, decodes
  the report, and emits report byte count + SHA256.
- Replaced dense adjacency/Laplacian allocation with a symmetric sparse graph and matrix-free
  shifted-Laplacian matvec.
- Parallelized sparse row matvecs with Rayon and log `rayon_threads`.
- Removed O(V*E) degree scoring in Lodestar by precomputing degree counts in one edge pass.
- Scaled the projected dense Jacobi budget as `max(256, 16 * n^2)` for the small Ritz matrix.

Exact local verification:

```bash
cargo fmt --all -- --check
cargo test -p calyx-cli cmd::spectral_communities::tests --target-dir target\issue877-cli-spectral3 --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue877_spectral_communities_tests --target-dir target\issue877-lodestar-spectral3 --jobs 32 -- --nocapture
cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-mincut --all-targets --target-dir target\issue877-clippy3 --jobs 32 -- -D warnings
git diff --check
bash scripts/linecount.sh
```

Local results:

- CLI spectral command tests: 5 passed, 0 failed.
- Lodestar issue #877 tests: 4 passed, 0 failed.
- rustfmt check: exit 0.
- clippy: exit 0.
- `git diff --check`: exit 0.
- Touched file line counts: `spectral_communities.rs=287`, CLI tests `195`, mincut spectral `318`,
  mincut linalg `236`, Lodestar spectral report `302`.
- Repo-wide linecount still fails on legacy files tracked by #954, not on #877-touched files.

Real FSV command:

```bash
root=/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547
CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 \
  /usr/bin/time -v "$root/target/release/calyx" spectral-communities \
  corpus-anchored-869-20260625T080546Z \
  --eigen-k 3 \
  --eigen-max-iter 64 \
  --centrality-max-iter 512 \
  --centrality-tol 0.00001 \
  --max-bridge-candidates 32 \
  --max-centrality-candidates 32
```

Real FSV source-of-truth readback:

- FSV root: `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547`.
- Source commit: `3dd4a2d42612e5855f83cb8305bcccef2b6dd079`.
- Source of truth:
  `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/spectral_communities/8c043084b902047a931952143821764d55dc25227ffadaad67d744fe5ee58cb2/report.json`.
- Before state: `idx/spectral_communities` was missing.
- After state: one persisted report at the path above.
- Report bytes: `42836741`.
- Report SHA256: `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7`.
- Graph readback: `node_count=198993`, `edge_count=2435817`.
- Report readback: `schema_version=1`, `member_count=198993`, `community_count=2`,
  `bridge_candidate_count=32`, `centrality_candidate_count=32`.
- Communities: community `0` has `33871` members; community `1` has `165122` members.
- Eigenvalues: `[0.122558594, 1.0655823, 2.615509]`.
- Spectral gap: `0.9430237`.
- Top bridge:
  `71a2dcaac4464a1943e5c17ecc5b9c4e -> 5f94d150f749709e0367ffcc4a6b2255`,
  rank `0.8651129`.
- Top centrality proposer: `5f94d150f749709e0367ffcc4a6b2255`, rank `0.9356725`, degree `116`.
- Runtime evidence: elapsed `10.60s`, max RSS `8103272 KB`, CPU `627%`, logged
  `rayon_threads=32`.

Boundary/edge FSV against the real vault:

- Invalid tolerance: `--centrality-tol 0` exited `2` with `CALYX_CLI_USAGE_ERROR`; before/after
  source-of-truth SHA remained
  `4dec84d08ae12ef67908ba43f46d2a16082530d026f931e9ed11d3f5e936b4f7`.
- Invalid eigen count: `--eigen-k 1` exited `2` with `CALYX_CLI_USAGE_ERROR`; before/after
  source-of-truth SHA remained unchanged.
- Too few Lanczos iterations: `--eigen-max-iter 1` opened the real graph, then exited `2` with
  `CALYX_SPECTRAL_NOT_CONVERGED`; before/after source-of-truth SHA remained unchanged.

Evidence files under the FSV root:

- `happy2_stderr.log` - real run graph load, thread count, persistence path, time/memory.
- `happy2_stdout.json` - CLI JSON output.
- `happy2_readback_summary.json` - independent disk read/decode/hash summary.
- `before_spectral_state_rerun.log` and `after_spectral_state.log` - source-of-truth state.
- `edge_case_summary.json` and `edge_*_before_state.log` / `edge_*_after_state.log` - edge
  source-of-truth verification.

Honest caveats:

- This report is a ranked graph-structural hypothesis surface, not a biomedical verdict.
- The real vault still logged `plain-graph: persisted CSR missing for collection=default, scanning
  graph edge rows`; #877 is complete, but a follow-up issue should materialize the physical CSR so
  large graph readers do not depend on row scans.
- This path is CPU/Rayon today. It does not use Forge CUDA kernels because Calyx does not yet have a
  GPU sparse Laplacian eigensolver backend.

## Conclusion
Issue #877 acceptance is complete: real spectral communities and inter-community bridge candidates
were computed from the physical anchored corpus graph and verified by a separate read of the persisted
vault artifact.
