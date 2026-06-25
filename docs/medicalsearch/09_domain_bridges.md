# 09 - domain bridges

- **Issue:** #876   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `AssocGraph` bridge members while #869 corpus ingest runs
- **Goal:** rank Swanson-style B-term bridge candidates per domain pair by graph frequency, degree centrality, grounded confidence, and provenance.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue876-20260625T113117Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo"
scp issue876-20260625T113117Z.tar aiwonder:/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo.tar -C /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/domain_bridges.rs`
- `crates/calyx-lodestar/tests/issue876_domain_bridges_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z`
- Artifact: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260625T113117Z/issue876_domain_bridges_readback.json`
- Artifact bytes: `3410`
- Artifact SHA256: `929677a08b383594b9b2158dae8a8ecca3b941439e94cc0314ab3327473ffdba`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=4`
  - `pair_count=2`
  - `candidate_count=3`
  - `refused_count=1`
  - `top_cx_id=cd67bd26d28afed81d52aee947746077`
  - `top_rank_score=0.4300000071525574`
  - `top_degree=1`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- B-term candidates are grouped by domain pair.
- Ranking combines graph frequency, graph degree, supplied centrality, and gate confidence.
- Gate-refused bridge members are counted but not returned as candidates.
- `max_per_pair` truncates after deterministic ranking.
- Non-finite centrality fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Bridge IDs absent from the graph fail closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- Lodestar now has a serializable domain-bridge report for B-term candidate mining.
- The report consumes real `bridges(scope_a, scope_b)` output shape indirectly: candidate `CxId`s are validated against the graph, then scored using graph frequency and degree.
- The synthetic FSV proves two domain-pair reports, three ranked candidates, and one gate refusal persisted to disk and were read back.
- This is not yet the final #876 anchored-corpus acceptance. The real bridge candidate list requires running scoped kernels and bridges on the actual anchored association graph after #869/#870/#871.

## Conclusion & next step
The #876 ranking/report surface is ready. Keep #876 open until the real anchored corpus graph exists, scoped kernels can be built, `bridges(scope_a, scope_b)` is run for real domain pairs, and ranked B-term candidates are read back with real sufficiency evidence.
