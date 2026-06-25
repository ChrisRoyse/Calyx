# 13 - chain walks synthetic

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic grounded `AssocGraph` while #869 corpus ingest runs
- **Goal:** run grounded chain walks from static sweep seeds and operator-question seeds, preserving full provenance and extracting terminal A-B-C hypotheses.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue880-20260625T115442Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo"
scp issue880-20260625T115442Z.tar aiwonder:/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo.tar -C /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/chain_walks.rs`
- `crates/calyx-lodestar/tests/issue880_chain_walks_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z`
- Artifact: `/home/croyse/calyx/fsv/issue880-chain-walks-20260625T115442Z/issue880_chain_walks_readback.json`
- Artifact bytes: `13017`
- Artifact SHA256: `085e82acf830f7ec13dd850016ec913966926badcf298388eec86a50e515a455`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z`
- Artifact: `/home/croyse/calyx/fsv/issue880-chain-walks-final-20260625T115700Z/issue880_chain_walks_readback.json`
- Artifact bytes: `13017`
- Artifact SHA256: `085e82acf830f7ec13dd850016ec913966926badcf298388eec86a50e515a455`
- Readback scalar leaves:
  - `schema_version=1`
  - `seed_count=2`
  - `completed_chain_count=2`
  - `hypothesis_count=2`
  - `top_seed_id=static-top`
  - `top_a=b8180e3b18aacaa1d2b6823ac71505c6`
  - `top_b=4e9bfc1971e762585b85541a3b60217e`
  - `top_c=52b6d87820fd8013d5c945d766133424`
  - `top_rank_score=0.746999979019165`
  - `top_cross_domain_distance=2`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Static top-candidate seed and operator-question seed both run through the #878 grounded discovery harness.
- Terminal A-B-C hypotheses are extracted from accepted paths with `A=start`, `B=penultimate`, `C=terminal`.
- Seed provenance and selected-hop gate evidence are carried into hypothesis provenance.
- `max_hypotheses_per_seed` truncates after deterministic ranking.
- Empty seed list, duplicate seed IDs, and operator-question seeds without question text fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Unknown seed start nodes fail closed through `CALYX_GRAPH_UNKNOWN_NODE`.

## Findings (honest)
- Lodestar now has a serializable chain-walk report that runs `run_grounded_discovery_chain` once per seed.
- Seeds distinguish static sweep candidates from operator-supplied questions, preserving rationale and provenance.
- The synthetic FSV proves two completed grounded chains and two terminal A-B-C hypotheses persisted to disk and read back.
- This is not yet final #880 anchored-corpus acceptance. Real chain walks require #869 anchored ingest, #870 association graph weaving, #871 kernel grounding, and real top-candidate/operator seeds.

## Conclusion & next step
The #880 report/orchestration surface is ready for corpus use. Keep #880 open until real chain walks are run on aiwonder against the anchored graph and each real chain artifact is read back under `docs/medicalsearch/13_chain_walks_<seed>.md`.
