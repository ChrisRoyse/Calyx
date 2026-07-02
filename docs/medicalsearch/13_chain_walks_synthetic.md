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

## 2026-07-02 real corpus addendum

- **Issue:** #880   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- **Goal:** run the grounded chain-walk report against the anchored clinical corpus graph, using real static bridge seeds plus operator-question seeds, and persist/read back the full A-B-C hypothesis artifact.

## What was run (exact commands)
```bash
# aiwonder source-of-truth checkout
cargo test -p calyx-cli cmd::chain_walks -- --nocapture
cargo test -p calyx-cli chain_walks_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
$(rustup which rustfmt) --edition 2024 --check \
  crates/calyx-cli/src/cmd/chain_walks.rs \
  crates/calyx-cli/src/cmd/chain_walks/tests.rs
git diff --check -- \
  crates/calyx-cli/src/cmd/chain_walks.rs \
  crates/calyx-cli/src/cmd/chain_walks/tests.rs \
  crates/calyx-cli/src/cmd/mod.rs \
  crates/calyx-cli/src/cmd/tests/token_roundtrip.rs

# real corpus run
root=/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z
CALYX_HOME=/home/croyse/calyx /usr/bin/time -f "elapsed_sec=%e" \
  -o "$root/chain_walks.rerun.time" \
  target/debug/calyx chain-walks corpus-anchored-869-20260625T080546Z \
    --seed-file "$root/chain_walk_seeds.json" \
    --anchor-file "$root/kernel_members.anchors.txt" \
    --max-hops 100 \
    --branch-width 3 \
    --probe-width 16 \
    --max-groundedness-distance 3 \
    --min-gate-confidence 0.25 \
    --novelty-weight 0.35 \
    --max-hypotheses-per-seed 8 \
    --min-terminal-confidence 0.25 \
    --out "$root/real_chain_walks.json" \
    > "$root/chain_walks.rerun.stdout.json" \
    2> "$root/chain_walks.rerun.stderr.log"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-cli/src/cmd/chain_walks.rs`
- `crates/calyx-cli/src/cmd/chain_walks/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`

Focused aiwonder checks:
- `cargo test -p calyx-cli cmd::chain_walks -- --nocapture`: 2 passed, 0 failed.
- `cargo test -p calyx-cli chain_walks_round_trips_through_tokens -- --nocapture`: 1 passed, 0 failed.
- `cargo check -p calyx-cli`: exit 0.
- `rustfmt --edition 2024 --check` on the new chain-walks files: exit 0.
- `git diff --check` on the touched #880 files: exit 0.
- `cargo fmt -p calyx-cli -- --check` was also attempted, but it stops on the unrelated `crates/calyx-cli/src/cmd/probe_matrix/tests/bounded.rs` rustfmt diff tracked outside this slice.

aiwonder real-corpus FSV:
- FSV root: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z`
- Input vault: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`
- Seed file: `chain_walk_seeds.json`
- Seed count: `6`
- Seed SHA256: `f0df2880e1925e5eb7c470466932dfe5762d723b6d46d787cfda2a9f4cfe8a96`
- Anchor file: `kernel_members.anchors.txt`
- Anchor count: `21954`
- Anchor SHA256: `1eb62eee08ebe849ee214c4fc731ae59fbc2de1e0702af79d18b40fc91bf7735`
- Report artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- Report SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- Readback artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/readback_summary.json`
- Readback SHA256: `7cc3485e1bb9201f04db2c4ce3a48ea8eb0a1323c878bb78122bdb75c0fdc14b`
- Runtime sidecar: `elapsed_sec=56.63`

Readback scalar leaves:
- `graph.nodes=198993`
- `graph.edges=2435817`
- `graph.node_metadata_count=166`
- `schema_version=1`
- `seed_count=6`
- `completed_chain_count=6`
- `hypothesis_count=48`
- `seed_kind_counts=static_candidate:4, operator_question:2`

Per-seed readback:
| Seed | Kind | Accepted hops | Candidates | Gate pass | Refused | Hypotheses | Top A | Top B | Top C |
|---|---|---:|---:|---:|---:|---:|---|---|---|
| `spectral-bridge-1-src` | static | 165 | 2608 | 1314 | 1294 | 8 | `71a2dcaac4464a1943e5c17ecc5b9c4e` | `472cc62939d0298289886c288ea30269` | `d586f81bca5cf4aca40b42d2f7b65091` |
| `spectral-bridge-2-src` | static | 153 | 2416 | 1130 | 1286 | 8 | `c0fff9e919bfa23e0b7aaea7b6f341fd` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `spectral-bridge-3-src` | static | 159 | 2512 | 1196 | 1316 | 8 | `76cdb0f7234f9e0b25cc8fea8daf2434` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `spectral-bridge-4-src` | static | 135 | 2128 | 1038 | 1090 | 8 | `2b597f47ce5d9a4101918d06272cb294` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `operator-centrality-1` | operator | 108 | 1696 | 793 | 903 | 8 | `5f94d150f749709e0367ffcc4a6b2255` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |
| `operator-centrality-2` | operator | 153 | 2416 | 1146 | 1270 | 8 | `5194ee0bbcc455a10b1b453735169a83` | `a8cc65ec3a9ae0d9febc02a22c107009` | `ccf62e2fb59b20a8ca50febd517f5c9b` |

The first attempted command omitted `CALYX_HOME` and failed before writing the report with `CALYX_CLI_USAGE_ERROR` / `CALYX_HOME is required for vault commands`. That stderr was retained as `chain_walks.stderr.log`; the successful rerun above produced the artifact and readback.

## Findings (honest)
- The corpus run completed all 6 real seeds and persisted 48 terminal A-B-C hypotheses.
- The report was read back from disk independently, and the report SHA256 in the readback matches the persisted artifact.
- Terminal node metadata sampled from the report carries stored `source_dataset=medmcqa`, `download_uri=hf://openlifescienceai/medmcqa`, `license=mit`, `retrieval_ts=2026-06-23T20:00:00Z`, `source_id`, and `source_sha256`.
- These are ranked, traceable hypotheses only. They are not biomedical verdicts and still need #881 evaluation over grounded evidence.

## Conclusion & next step
The #880 real-corpus chain-walk artifact is complete for the anchored clinical corpus and is ready to feed #881 hypothesis evaluation. Per-seed readbacks are recorded in:
- `13_chain_walks_spectral-bridge-1-src.md`
- `13_chain_walks_spectral-bridge-2-src.md`
- `13_chain_walks_spectral-bridge-3-src.md`
- `13_chain_walks_spectral-bridge-4-src.md`
- `13_chain_walks_operator-centrality-1.md`
- `13_chain_walks_operator-centrality-2.md`

Repository-wide closeout remains coupled to the unrelated calyx-cli formatting/linecount cleanup being handled separately; the #880 files themselves have focused check, readback, and artifact evidence.
