# 09 - domain bridges

- **Issue:** #876   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic `AssocGraph` bridge members while #869 corpus ingest runs
- **Goal:** rank Swanson-style B-term bridge candidates per domain pair by graph frequency, degree centrality, grounded confidence, and provenance.

## 2026-06-29 real-corpus completion update
Source of truth:
- Real graph/properties: aiwonder vault `corpus-anchored-869-20260625T080546Z`, ULID `01KVYX0KYVBQSGVC6N2S00FX6J`.
- Persisted report: `<vault>/idx/domain_bridges/<blake3(report_bytes)>/report.json`.
- FSV summary artifact: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/issue876_real_fsv_summary.json`.
- Local readback copy: `target/issue876-fsv/issue876_real_fsv_summary.json`.
- Synapse readback key: `calyx-dev/issue876/real-fsv-summary`.

Root cause found:
- The earlier slice only ranked supplied bridge inputs; there was no production CLI that mined domain-pair bridges from a physical Aster graph and persisted a readbackable artifact.
- Metadata scopes were root-only filters, so two source-dataset scopes were disjoint and could not expand to shared graph neighbours.
- The physical graph reader reconstructed topology by scanning edge rows and did not use persisted CSR when available.
- `domain_bridges::max_degree` was effectively `O(V * E)` because it called `AssocGraph::in_degree` for every node. On the real #869 graph (`198,993` nodes, `2,435,817` edges) this stalled before bridge scoring logs.

Research inputs:
- Rediscovering Don Swanson: B-terms are implicit linking information; raw B-term counts are not robust by themselves, and ranking should use statistical and graph/network properties. <https://pmc.ncbi.nlm.nih.gov/articles/PMC5771422/>
- LBD systematic review: term ranking/thresholding is needed to prune noisy associations and rank by significance/interestingness. <https://pmc.ncbi.nlm.nih.gov/articles/PMC7924697/>
- Henry and McInnes indirect-association ranking work: shared linking terms and ranking measures need empirical filtering/evaluation, not unbounded enumeration. <https://bmcbioinformatics.biomedcentral.com/articles/10.1186/s12859-019-2989-9>

Implemented:
- `calyx domain-bridges <vault>` opens the latest physical Aster graph, mines scoped bridge candidates, persists an atomic JSON report, reads it back, byte-compares it, and prints report plus artifact hash/counts.
- `Scope::FilterReachable` starts from real metadata roots and expands through bounded outgoing graph hops, so source-dataset scopes can produce shared bridge candidates without inventing data.
- `PhysicalAsterAssocSnapshot` exposes real topology and decoded node metadata through the Lodestar `AssocStore`.
- Physical graph loading now prefers persisted CSR when present and logs an explicit row-scan path when CSR is missing.
- Degree scoring now uses a single `O(V + E)` degree precompute and computes it lazily only after shared bridge members exist.
- Failure modes are fail-closed: missing roots, no shared bridge members, refused-only candidates, corrupt CSR, invalid params, and artifact overwrite mismatch all return errors with context instead of fallback data.

Local gates:
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests --target-dir target\issue876-lodestar-final3 --jobs 32 -- --nocapture`: 5 passed.
- `cargo test -p calyx-cli cmd::domain_bridges::tests --target-dir target\issue876-cli-final3 --jobs 32 -- --nocapture`: 5 passed.
- `cargo test -p calyx-aster physical_assoc_graph_prefers_persisted_csr_projection --target-dir target\issue876-aster-final3 --jobs 32 -- --nocapture`: passed.
- `cargo test -p calyx-lodestar --test ph34_scope_tests materialize_all_domain_subgraph_time_tenant_filter --target-dir target\issue876-scope-final2 --jobs 32 -- --nocapture`: passed.
- `cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target\issue876-cli-roundtrip2 --jobs 32`: passed.
- `cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-aster --all-targets --target-dir target\issue876-clippy-final --jobs 32 -- -D warnings`: pass.
- Post-split verification:
  - `cargo fmt --all -- --check`: pass.
  - `git diff --check`: pass.
  - `cargo clippy -p calyx-cli -p calyx-lodestar -p calyx-aster --all-targets --target-dir target\issue876-clippy-final2 --jobs 32 -- -D warnings`: pass.
  - `cargo test -p calyx-lodestar --test issue876_domain_bridges_tests --target-dir target\issue876-lodestar-final4 --jobs 32 -- --nocapture`: 5 passed.
  - `cargo test -p calyx-cli cmd::domain_bridges::tests --target-dir target\issue876-cli-final4 --jobs 32 -- --nocapture`: 5 passed.
  - `cargo test -p calyx-aster physical_assoc_graph_prefers_persisted_csr_projection --target-dir target\issue876-aster-final4 --jobs 32 -- --nocapture`: 1 passed.
  - `cargo test -p calyx-lodestar --test ph34_scope_tests materialize_all_domain_subgraph_time_tenant_filter --target-dir target\issue876-scope-final4 --jobs 32 -- --nocapture`: 1 passed.
  - `cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target\issue876-cli-roundtrip4 --jobs 32`: 1 passed.
  - `bash -lc "wc -l ..."` for #876 touched split files: `plain_graph/mod.rs=481`, `aster_bridge.rs=414`, `domain_bridges.rs=401`, `plain_graph/assoc_graph.rs=82`, `aster_bridge/physical.rs=137`, `domain_bridges/mining.rs=149`.
  - `bash scripts/linecount.sh`: still fails on legacy files outside #876; #954 was reopened with the current failing file list.

Real aiwonder FSV:
- Isolated source: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/repo`.
- Built binary: `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/target/debug/calyx`.
- FSV summary bytes: `11015`.
- FSV summary SHA256: `d667518cfafe5f051813df61aa6bac99e9fbfdcaf6f7232de50953c13769c200`.
- Real graph readback logs: `nodes=198993`, `edges=2435817`, `node_props rows=198993`.
- Persisted CSR was absent in this older vault, so the logged source of truth was the physical edge-row scan; the new CSR path is covered by the physical CSR readback test.

Manual happy path:
- Trigger: `metadata:source_dataset=pubmedqa` vs `metadata:source_dataset=medxpertqa`, `--scope-radius 1`, `--max-evidence-hops 2`, `--kernel-target-fraction 0.10`.
- Before: output report absent.
- After: output report present, bytes `11165`, SHA256 `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04`.
- Readback state: `schema_version=1`, `input_count=7`, `pair_count=1`, `candidate_count=7`, `refused_count=0`.
- Top candidate readback: `cx_id=0fa503037d5d87b51187abe53d1df67c`, `gate=CALYX_DOMAIN_BRIDGE_GATE_PASS`, `confidence=0.33333334`, `distance=2`, provenance includes real metadata (`source_dataset=medmcqa`, `license=mit`, `download_uri=hf://openlifescienceai/medmcqa`).

Manual edge cases:
- Tight target fraction (`0.02`): before output absent, after output absent, rc `2`, stderr contained `produced no shared bridge members`.
- Missing metadata roots (`metadata:source_dataset=not_real_876`): before output absent, after output absent, rc `2`, stderr contained `has no source-of-truth root nodes`.
- Strict gate (`--min-gate-confidence 0.99`): before output absent, after output absent, rc `2`, stderr contained `had only refused bridge candidates`.

Honest limitation:
- The current real #869 vault contains clinical-QA source datasets (`pubmedqa`, `medxpertqa`, `medqa`, `medmcqa`) only. This closes the production bridge-mining/root-cause work and proves real clinical domain-pair behavior, but it does not prove clinical x molecular/legal/finance bridge acceptance because those non-clinical corpora are not yet materialized in a physical vault. The missing materialized-corpus state is tracked in #994.

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
