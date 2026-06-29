# 11 - discovery harness

- **Issue:** #878   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / 2026-06-29   **Vault/panel:** synthetic slice, then real anchored corpus `corpus-anchored-869-20260625T080546Z`
- **Goal:** turn the discovery chain loop into a real, deterministic Lodestar harness that gates every probed association and persists a traceable chain log.

## 2026-06-29 real anchored-corpus completion

### Research used
- Exa + direct web research confirmed the right shape is bounded beam expansion with traceable paths and explicit pruning/gating:
  - Think-on-Graph describes iterative beam search over knowledge graphs with explicit reasoning-path traceability: <https://arxiv.org/abs/2307.07697>.
  - NetworkX's beam-search docs define beam width as keeping only the best `w` neighbors by an application heuristic: <https://networkx.org/documentation/stable/reference/algorithms/generated/networkx.algorithms.traversal.beamsearch.bfs_beam_edges.html>.
  - OpenTelemetry logging specs reinforce structured, correlated records for downstream inspection: <https://opentelemetry.io/docs/specs/otel/logs/>.

### Root cause fixed
- The previous #878 slice was library-only: it had no physical CLI surface, no vault source-of-truth artifact, and no real anchored-corpus run.
- The chain engine also rebuilt the anchor index for every candidate. That was harmless on toy graphs but wrong for real corpus anchor sets. The engine now builds the anchor ID/index sets once per run.
- The CLI now supports `--anchor-file` so large audited anchor sets can be supplied explicitly instead of relying on shell-length-limited inline arguments.

### Implemented source
- `crates/calyx-cli/src/cmd/discovery_chain.rs`
- `crates/calyx-cli/src/cmd/discovery_chain/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`
- `crates/calyx-lodestar/src/discovery_chain.rs`

### Exact final FSV commands
```bash
# Windows authoring checkout
cargo test -p calyx-cli cmd::discovery_chain::tests --target-dir target\issue878-cli-discovery3 --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests --target-dir target\issue878-lodestar-discovery3 --jobs 32 -- --nocapture
cargo fmt --all -- --check
cargo clippy -p calyx-cli -p calyx-lodestar --all-targets --target-dir target\issue878-clippy3 --jobs 32 -- -D warnings

# aiwonder final archived source
git archive --format=tar -o issue878-real-discovery-fullanchors-20260629-061500.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo"
scp issue878-real-discovery-fullanchors-20260629-061500.tar aiwonder:/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo.tar -C /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/repo && CARGO_INCREMENTAL=0 cargo build -p calyx-cli --release --jobs 32"

# Anchor file source of truth: every member in the #877 spectral report.
python3 - <<'PY'
import json, pathlib
root=pathlib.Path('/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500')
report_path=pathlib.Path('/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/spectral_communities/8c043084b902047a931952143821764d55dc25227ffadaad67d744fe5ee58cb2/report.json')
report=json.loads(report_path.read_text())
members=[row['cx_id'] for row in report['members']]
(root/'anchors_all_members_from_spectral.txt').write_text('\n'.join(members)+'\n')
PY

CALYX_HOME=/home/croyse/calyx RAYON_NUM_THREADS=32 /usr/bin/time -v \
  ./target/release/calyx discovery-chain corpus-anchored-869-20260625T080546Z \
  --start 71a2dcaac4464a1943e5c17ecc5b9c4e \
  --start c0fff9e919bfa23e0b7aaea7b6f341fd \
  --start 76cdb0f7234f9e0b25cc8fea8daf2434 \
  --start 2b597f47ce5d9a4101918d06272cb294 \
  --start c1607869690bf88eba8c5d85150aab22 \
  --start 07d28562a4aed115a0bffa444d3e8685 \
  --start 475e94270655af8c265fa7f9fa70595f \
  --start 5f94d150f749709e0367ffcc4a6b2255 \
  --anchor-file /home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/anchors_all_members_from_spectral.txt \
  --max-hops 100 --branch-width 16 --probe-width 16 \
  --max-groundedness-distance 1 --min-gate-confidence 0.25 --novelty-weight 0.35
```

### Source-of-truth readback
- Source of truth: `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/discovery_chains/4f5dbf9acb8beac9f287837fddaf58338a5c5515368ae3d66ee3b678532b9820/chain.json`
- Artifact bytes: `36554831`
- Artifact SHA256: `376d0649da579a1fe623a21b63cf58338ef6c93ccb284c549f2f6e8e9b2c0885`
- Graph read from artifact: `198993` nodes, `2435817` edges.
- Chain read from artifact: `start_count=8`, `anchor_count=198993`, `candidate_count=25472`, `accepted_hop_count=1600`, `gate_pass_count=21936`, `refused_count=3536`, `max_hop_seen=100`, `termination=max_hops`.
- The last accepted hop is at hop `100`; the final 10 hop buckets each have `16` accepted branches.
- `node_metadata_count=2244`, with real corpus metadata including `source_dataset`, `source_sha256`, `download_uri`, license, and retrieval timestamp.
- Runtime evidence: elapsed `7.51s`, max RSS `8095584 KB`, CPU `442%`, and the command logged `rayon_threads=32`.

### Boundary / edge-case FSV
Source-of-truth root before and after each edge case stayed at `chain_json_count 6`; the happy artifact path above stayed present.

- Edge 1: `--max-hops 0`
  - Exit `2`
  - Error code `CALYX_CLI_USAGE_ERROR`
  - Message: `--max-hops must be >= 1`
- Edge 2: malformed anchor file row `not-a-cxid`
  - Exit `2`
  - Error code `CALYX_CLI_USAGE_ERROR`
  - Message includes the exact anchor file path and line `:1`.
- Edge 3: unknown start `00000000000000000000000000000000`
  - Exit `2`
  - Error code `CALYX_GRAPH_UNKNOWN_NODE`
  - No discovery-chain artifact was created or mutated.

Raw evidence folder:
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stdout.json`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stderr.log`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_readback_summary.json`
- `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/edge_case_summary.json`

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

## Conclusion
#878 is complete. The harness now has a physical CLI, writes a content-addressed traceable chain log into the real vault, reads that source of truth back, supports large explicit anchor files, and has real anchored-corpus FSV proving the 100-hop gated chain loop reached `termination=max_hops`.
