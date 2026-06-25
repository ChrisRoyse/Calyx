# 11 - discovery harness

- **Issue:** #878   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic grounded `AssocGraph` while #869 corpus ingest runs
- **Goal:** turn the discovery chain loop into a real, deterministic Lodestar harness that gates every probed association and persists a traceable chain log.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue878-20260625T105843Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo"
scp issue878-20260625T105843Z.tar aiwonder:/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo.tar -C /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/discovery_chain.rs`
- `crates/calyx-lodestar/tests/issue878_discovery_chain_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z`
- Artifact: `/home/croyse/calyx/fsv/issue878-discovery-chain-20260625T105843Z/issue878_discovery_chain_readback.json`
- Artifact bytes: `4473`
- Artifact SHA256: `338319e9e6563f9d9b9326c13d8dc27644ada26da0b8c8e0ba59854a0542f184`
- Readback scalar leaves:
  - `schema_version=1`
  - `accepted_count=2`
  - `gate_pass_count=2`
  - `refused_count=1`
  - `termination=frontier_exhausted`
  - `refusal_codes=CALYX_DISCOVERY_UNGROUNDED`
  - `accepted_to_count=2`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Strong ungrounded edge is refused with `CALYX_DISCOVERY_UNGROUNDED` and does not enter the selected chain.
- Visited-loop candidate is logged and refused with `CALYX_DISCOVERY_VISITED_LOOP`.
- Branch pruning keeps only the top gate-PASS candidate when `branch_width=1`; the unselected gate-PASS candidate remains in the log.
- `branch_width=0` fails closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Unknown start node fails closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- The harness now exists as a serializable Lodestar engine over `calyx_paths::AssocGraph`.
- Every probed candidate row carries the source branch, edge score, path score, novelty score, groundedness distance, gate verdict, and provenance strings.
- The default grounded gate keeps only candidates with an anchor reachable inside the configured groundedness radius and confidence floor.
- The synthetic FSV proves persisted chain-log bytes for the code path, including one explicit refusal.
- This is not yet the final #878 anchored-corpus acceptance. The real multi-hop biomedical run remains gated on #869 anchored ingest plus #870 association graph weaving and #871 kernel grounding.

## Conclusion & next step
The implementation slice is ready for downstream corpus use, but #878 must remain open until the harness is run against the real anchored corpus and its real chain log is read back from aiwonder. Next dependency path: finish #869, build #870 graph, ground #871 kernel, then run the #878 100-hop harness on the anchored graph.
