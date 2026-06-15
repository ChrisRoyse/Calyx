# 02. Source Code Map

Source files covered: derived from the root `Cargo.toml`, `rust-toolchain.toml`, the
`fuzz/` cargo-fuzz package, and the `Cargo.toml` + `src/lib.rs` of all 18 workspace
crates under `crates/`. Module lists are enumerated from each crate's `mod`/`pub mod`
declarations and the file/directory names directly under `src/`. Deeper nested files
(submodules inside the listed module directories) are summarized at the module level
rather than per-file.

LOC values are approximate (`wc -l` over each crate's `src/**/*.rs`, rounded). Roles
are derived from each crate's `lib.rs` top-level `//!` doc comment.

---

## 1. Workspace layout

The workspace (`[workspace] members = ["crates/*"]`) contains 18 crates. Deep-dive
documents are cross-referenced in the right-hand role column where one exists.

| Crate | Path | LOC (approx) | Role |
|---|---|---|---|
| calyx-core | crates/calyx-core | 5,100 | Core Calyx identifiers, model contracts, and shared types (leaf). See 04. |
| calyx-aster | crates/calyx-aster | 43,700 | Aster storage engine for column families and WAL. See 05. |
| calyx-forge | crates/calyx-forge | 13,400 | Forge math runtime for CPU, CUDA, and quantized kernels. See 06. |
| calyx-registry | crates/calyx-registry | 13,700 | Registry runtimes for frozen Calyx lenses. See 07. |
| calyx-sextant | crates/calyx-sextant | 13,400 | Sextant search and navigation for retrieval. See 08. |
| calyx-loom | crates/calyx-loom | 3,800 | Loom DDA cross-term and agreement-graph engine. See 09. |
| calyx-assay | crates/calyx-assay | 5,600 | Assay signal-bit measurement, panel sufficiency, persistence contracts. See 09. |
| calyx-paths | crates/calyx-paths | 500 | Path and graph traversal over association networks. See 10. |
| calyx-mincut | crates/calyx-mincut | 1,100 | Directed graph primitives for grounding kernels. See 10. |
| calyx-lodestar | crates/calyx-lodestar | 5,300 | Lodestar grounding-kernel discovery and maintenance. See 10. |
| calyx-ledger | crates/calyx-ledger | 5,200 | Append-only Ledger provenance primitives. See 11. |
| calyx-ward | crates/calyx-ward | 4,800 | Ward guard profiles for per-slot cosine policy enforcement. See 12. |
| calyx-anneal | crates/calyx-anneal | 19,700 | Anneal self-optimization contracts for reversible tuning loops. See 13. |
| calyx-oracle | crates/calyx-oracle | 9,300 | Oracle consequence prediction and completion primitives. See 14. |
| calyx-cli | crates/calyx-cli | 26,800 | `calyx` command-line binary; readback/ops/validation dispatch. See 15. |
| calyx-mcp | crates/calyx-mcp | 800 | MCP interface for agent-facing Calyx operations. See 16. |
| calyxd | crates/calyxd | 4,300 | `calyxd` daemon: chain-verify metrics, CUDA/VRAM probes, MCP socket. See 16. |
| calyx-testkit | crates/calyx-testkit | 200 | Reusable deterministic test scaffolding for Calyx crates. |

Layering summary (see section 3 for the full graph):

- Leaf: `calyx-core` (no `calyx-*` deps), `calyx-testkit` (core only).
- Low: `calyx-paths`, `calyx-ledger`, `calyx-forge` (core only); `calyx-mincut` (core + paths).
- Mid: `calyx-aster`, `calyx-ward`, `calyx-loom`, `calyx-sextant`, `calyx-lodestar`,
  `calyx-registry`, `calyx-anneal`, `calyx-assay`, `calyx-oracle`.
- Top (binaries): `calyx-cli` (`calyx`), `calyxd` (`calyxd`), `calyx-mcp` (`calyx-mcp`).

---

## 2. Per-crate module tree

For each crate, the modules below are the `mod`/`pub mod` declarations in `lib.rs`
together with the `*.rs` files and submodule directories directly under `src/`. Where
a name appears both as `foo.rs` and `foo/` (e.g. `vault.rs` + `vault/`), Rust treats
the `.rs` as the module root and the directory as its submodules — listed once.
Deeper nested files inside each module directory are summarized at the module level.

### calyx-core (`crates/calyx-core/src/`)
Doc: "Core Calyx identifiers, model contracts, and shared types."

| Module | Description |
|---|---|
| alloc/ | Arena/allocation primitives. |
| cache/ | Shared caching primitives. |
| cold_start.rs | Cold-start handling for fresh state. |
| consent.rs | Consent records and policy types. |
| cosine.rs | Cosine-similarity primitives. |
| enums.rs | Shared enumerations. |
| error.rs | Core error taxonomy. |
| ids.rs | Calyx identifier types. |
| model/ | Model contracts and shared model types. |
| security.rs | Security-related shared types. |
| temporal.rs | Temporal/recurrence shared types. |
| time.rs | Time primitives. |
| traits.rs | Shared trait contracts. |

### calyx-aster (`crates/calyx-aster/src/`)
Doc: "Aster storage engine skeleton for Calyx column families and WAL."

| Module | Description |
|---|---|
| cf/ | Column-family definitions and access. |
| collection/ | Collection-level storage abstractions. |
| compaction/ | Compaction logic. |
| dedup/ | Deduplication. |
| erase.rs + erase/ | Erase/redaction operations. |
| file_lock.rs | File locking (private `mod`). |
| gc/ | Garbage collection. |
| index/ | Storage indexes. |
| layers/ | Layered storage tiers. |
| ledger_view.rs | Ledger view over storage. |
| manifest/ | Manifest format and management. |
| memtable/ | In-memory write buffer. |
| mmap_col.rs | Memory-mapped column access. |
| mvcc/ | Multi-version concurrency control. |
| olap/ | OLAP/analytical access paths. |
| plain_column/ | Plain (uncompressed) column store. |
| plain_graph/ | Plain graph store. |
| pressure.rs | Memory/resource pressure handling. |
| recurrence.rs + recurrence_tests.rs | Recurrence storage support (+ tests). |
| redaction.rs | Redaction. |
| residency.rs | Residency/placement of data. |
| resource/ | Resource accounting. |
| retention.rs + retention/ | Retention policy. |
| security/ | Storage-layer security. |
| sst/ | Sorted string tables (SST). |
| storage_names.rs | Storage naming conventions. |
| stream/ | Streaming access. |
| stride_fsv.rs | Stride full-system-verification scaffolding. |
| supply_chain.rs + supply_chain/ | Supply-chain integrity. |
| timetravel/ | Time-travel reads. |
| txn/ | Transactions. |
| vault.rs + vault/ | Vault (top-level store) management. |
| wal/ | Write-ahead log. |

### calyx-forge (`crates/calyx-forge/src/`)
Doc: "Forge math runtime skeleton for CPU, CUDA, and quantized kernels."

| Module | Description |
|---|---|
| autotune/ | Kernel auto-tuning. |
| backend.rs | Backend abstraction (private `mod`). |
| compression_report/ | Compression reporting. |
| cpu/ | CPU kernels. |
| cuda/ | CUDA kernels. |
| error.rs | Forge error types (private `mod`). |
| mxfp4 | MXFP4 quantization format (declared in lib.rs). |
| mxfp8 | MXFP8 quantization format (declared in lib.rs). |
| quant/ | Quantization support. |
| vram/ | VRAM accounting/budget. |

### calyx-registry (`crates/calyx-registry/src/`)
Doc: "Registry runtimes for frozen Calyx lenses."

| Module | Description |
|---|---|
| backfill.rs | Backfill of registry data. |
| commission.rs + commission/ | Lens commissioning. |
| compression/ | Registry compression. |
| drift.rs | Drift detection. |
| explain.rs | Explainability output. |
| frozen.rs + frozen/ | Frozen-lens runtime. |
| ingest_microbatch.rs + ingest_microbatch/ | Micro-batch ingest. |
| lens.rs + lens/ | Lens definitions. |
| panel_ops.rs | Panel operations. |
| panels/ | Panel structures. |
| persistence.rs | Registry persistence. |
| placement.rs | Lens placement. |
| profile.rs + profile/ | Registry profiles. |
| runtime/ | Registry runtime. |
| spec.rs | Registry spec types. |
| swap.rs + swap/ | Hot-swap of lenses. |
| temporal/ | Temporal registry support. |

### calyx-sextant (`crates/calyx-sextant/src/`)
Doc: "Sextant search and navigation for Calyx retrieval."

| Module | Description |
|---|---|
| error.rs | Sextant error types. |
| fusion/ | Result fusion. |
| guarded.rs | Guarded (policy-checked) search. |
| hit.rs | Search-hit types. |
| index/ | Search indexes (e.g. DiskANN/HNSW). |
| navigation/ | Navigation over results. |
| planner.rs + planner_explain.rs | Query planner (+ explain). |
| query.rs + query/ | Query types and parsing. |
| query_admission.rs | Query admission control. |
| reranker.rs | Re-ranking. |
| search.rs | Core search entry. |
| search_support.rs | Search support helpers (private `mod`). |
| slot_index_map.rs | Slot-to-index mapping. |
| temporal/ | Temporal search support. |
| util.rs | Utilities (private `mod`). |

### calyx-loom (`crates/calyx-loom/src/`)
Doc: "Loom DDA cross-term and agreement-graph engine."

| Module | Description |
|---|---|
| abundance.rs | Abundance signal computation. |
| agreement_graph.rs | Agreement-graph construction. |
| blind_spot.rs | Blind-spot detection. |
| cross_term.rs | Cross-term DDA computation. |
| error.rs | Loom error types. |
| lru_cache.rs | LRU cache. |
| materialization.rs | Materialization of derived values. |
| reactive/ | Reactive update propagation. |
| recurrence/ | Recurrence support. |

### calyx-assay (`crates/calyx-assay/src/`)
Doc: "Assay signal-bit measurement, panel sufficiency, and persistence contracts."

| Module | Description |
|---|---|
| attribution.rs | Attribution analysis. |
| bayesian.rs | Bayesian estimators. |
| bootstrap.rs | Bootstrap resampling. |
| contract.rs | Persistence/measurement contracts. |
| estimate.rs | Estimation primitives. |
| formula_catalog.rs + formulas.rs | Formula catalog and definitions. |
| gate.rs | Sufficiency gate. |
| ksg.rs | KSG mutual-information estimator. |
| logistic.rs | Logistic models. |
| loom_adapter.rs | Adapter to calyx-loom. |
| mmd.rs | Maximum mean discrepancy. |
| n_eff.rs | Effective sample size. |
| nmi.rs | Normalized mutual information. |
| periodicity.rs | Periodicity measurement. |
| projection.rs | Projection operations. |
| recurrence_anchor.rs + recurrence_hazard.rs | Recurrence anchoring and hazard. |
| samples.rs | Sample handling (private `mod`). |
| special_fn.rs | Special functions (private `mod`). |
| store.rs | Measurement store. |
| stratified.rs | Stratified sampling. |
| sufficiency.rs | Panel sufficiency. |
| total_correlation.rs | Total correlation. |
| transfer_entropy.rs | Transfer entropy. |

### calyx-paths (`crates/calyx-paths/src/`)
Doc: "Path and graph traversal over Calyx association networks."

| Module | Description |
|---|---|
| attenuation.rs | Path-weight attenuation. |
| error.rs | Path error types (private `mod`). |
| graph.rs | Graph representation. |
| traversal.rs | Traversal algorithms. |

### calyx-mincut (`crates/calyx-mincut/src/`)
Doc: "Directed graph primitives for Calyx grounding kernels."

| Module | Description |
|---|---|
| betweenness.rs | Betweenness centrality. |
| error.rs | Error types (private `mod`). |
| graph_builder.rs | Graph construction. |
| lp_scaffold.rs | LP (linear-program) scaffold. |
| scc.rs | Strongly-connected components. |
| spectral.rs | Spectral methods. |
| spectral_linalg.rs | Spectral linear algebra (private `mod`). |

### calyx-lodestar (`crates/calyx-lodestar/src/`)
Doc: "Lodestar grounding-kernel discovery and maintenance."

| Module | Description |
|---|---|
| aster_bridge.rs | Bridge to calyx-aster. |
| dfvs.rs | Directed feedback vertex set. |
| error.rs | Error types (private `mod`). |
| grounding_gaps.rs | Grounding-gap detection. |
| hierarchical.rs | Hierarchical kernels. |
| incremental.rs | Incremental kernel maintenance. |
| kernel.rs / kernel_answer.rs / kernel_graph.rs / kernel_health.rs / kernel_index.rs | Kernel core, answer, graph, health, index. |
| label_propagation.rs | Label propagation. |
| loom_assoc.rs | Loom association integration. |
| multi_scope.rs | Multi-scope kernels. |
| provenance.rs | Kernel provenance. |
| recall_test.rs | Recall testing. |
| scope.rs / scope_cache.rs / scope_report.rs | Scope core, cache, reporting. |
| summarize.rs | Kernel summarization. |
| temporal_kernel.rs | Temporal kernels. |

### calyx-ledger (`crates/calyx-ledger/src/`)
Doc: "Append-only Ledger provenance primitives."

| Module | Description |
|---|---|
| append.rs + append/ | Append path. |
| audit.rs + audit/ | Audit support. |
| checkpoint.rs | Checkpointing. |
| codec.rs | Entry codec. |
| entry.rs | Ledger entry types. |
| group_commit.rs | Group-commit batching. |
| kind.rs | Entry-kind taxonomy. |
| merkle.rs | Merkle chaining. |
| redaction.rs | Redaction. |
| reproduce.rs + reproduce/ | Reproduction/replay. |
| tombstone.rs + tombstone/ | Tombstones. |
| verify.rs | Chain verification. |

### calyx-ward (`crates/calyx-ward/src/`)
Doc: "Ward guard profile types for per-slot cosine policy enforcement."

| Module | Description |
|---|---|
| calibrate.rs | Threshold calibration. |
| drift.rs | Drift detection. |
| error.rs | Error types. |
| generate.rs | Profile generation. |
| guard.rs | Guard enforcement. |
| identity.rs | Identity profiles. |
| ledger.rs | Ward ledger integration. |
| novelty.rs | Novelty detection. |
| polis.rs | Polis (collective) policy. |
| profile.rs | Guard profile types. |
| query.rs | Guarded query. |
| required.rs | Required-policy checks. |
| speaker_lens.rs / style_lens.rs | Speaker and style lenses. |
| verdict.rs | Guard verdicts. |

### calyx-anneal (`crates/calyx-anneal/src/`)
Doc: "Anneal self-optimization contracts for reversible tuning loops."
All modules are private (`mod`); the public API is re-exported from `lib.rs`.

| Module | Description |
|---|---|
| budget.rs | Tuning budget accounting. |
| heal/ | Self-heal logic. |
| integration_fsv.rs | Integration full-system-verification. |
| j/ | Objective ("J") evaluation. |
| learn/ | Learning/bandit logic. |
| ledger_anneal.rs | Ledger integration for anneal runs. |
| propose/ | Proposal generation. |
| recurrence_schedule.rs | Recurrence scheduling. |
| rollback.rs + rollback_codec.rs | Rollback and its codec. |
| shadow.rs | Shadow evaluation. |
| tripwire.rs | Tripwire/guardrails. |
| tune/ | Tuning loops. |

### calyx-oracle (`crates/calyx-oracle/src/`)
Doc: "Oracle consequence prediction and completion primitives."
All modules are private (`mod`); public API re-exported from `lib.rs`. Each `*.rs`
has a sibling `*_tests.rs` test module (summarized, not listed individually).

| Module | Description |
|---|---|
| butterfly.rs | Butterfly (cascade) prediction. |
| complete.rs | Completion primitives. |
| energy.rs | Energy-based scoring. |
| error.rs | Error types. |
| honesty_gate.rs | Honesty gate. |
| prd22.rs | PRD-22 contract logic. |
| predict.rs | Core prediction. |
| reverse_query.rs (+ reverse_query_context.rs) | Reverse-query prediction. |
| self_consistency.rs | Self-consistency checks. |
| super_intel.rs / super_intel_full.rs / super_intel_types.rs | Super-intelligence aggregation. |
| time_prediction.rs | Time-of-event prediction. |
| types.rs | Shared oracle types. |

### calyx-cli (`crates/calyx-cli/src/`)
Doc: binary crate (no `lib.rs`); `main.rs` declares all modules. There is no `lib.rs`.
The CLI surface is large; `*_readback.rs`, `*_commands.rs`, and `*_validation` modules
each implement one command family (summarized below). `*_tests.rs` and `_validation/`
subdirectories are summarized at the module level.

| Module | Description |
|---|---|
| main.rs | Binary root; declares modules; calls `entry::main`. |
| entry.rs | Top-level routing (verify-restore, healthcheck-daemon, then dispatch). |
| dispatch.rs | Argument pattern-matching dispatcher to all command handlers. |
| error.rs / output.rs / cli_support.rs / fsv.rs | Error taxonomy, output formatting, shared parse/readback helpers, FSV. |
| anneal_*.rs (ab_log, autotune_report, bandit_readback, commands, deficit_map, frozen_guard_readback, goodhart_check, growth_curve, head_readback, intelligence_report, ledger_readback, lens_proposal_log, mistakes_readback, propose_lens_fixture, propose_lens_run, propose_preview, regression_readback, replay_readback, soak, soak_report, status) + anneal_soak/ | Anneal command/readback family (proposals, soak runs, readbacks, reports). |
| oracle_readback.rs + oracle_readback/ | Oracle readbacks. |
| sextant_commands.rs, sextant_diskann_validation(.rs/dir), sextant_recall_validation(.rs/dir) | Sextant commands and validation. |
| lens_commands.rs / panel_commands.rs | Registry lens and panel commands. |
| lodestar_commands.rs, lodestar_kernel_validation(.rs/dir), kernel_health_readback.rs | Lodestar commands, kernel validation, kernel-health readback. |
| intelligence_commands.rs | Intelligence command family. |
| media_commands.rs, media_emotion_validation(.rs/dir), media_image_validation(.rs/dir) | Media commands and validation. |
| ledger_store.rs / provenance.rs / merkle.rs / verify.rs / verify_restore.rs | Ledger store, provenance, Merkle, verify, byte-level verify-restore. |
| healthcheck.rs / healthcheck_daemon.rs / healthcheck_tests.rs | Deploy and daemon-readiness health checks (+ tests). |
| cf_read.rs / dedup_readback.rs / dedup_audit_readback.rs / manifest_readback.rs / vault_tree.rs / timetravel_readback.rs | Aster storage readbacks (column-family, dedup, manifest, vault tree, time-travel). |
| temporal_readback.rs, temporal_log_recurrence_readback/, recurrence_readback.rs | Temporal and recurrence readbacks. |
| time_prediction_readback.rs / trigger_readback.rs / tripwire_readback.rs / budget_readback.rs / ph42_readback.rs / ward_tau_readback.rs | Assorted readback families. |
| resource_status.rs / resource_drill.rs | Resource status and drill-down. |
| scan.rs / crash.rs / summarize_command.rs / navigate/ / ops.rs + ops/ / migrate/ | Scan, crash diagnostics, summarize, navigation, ops, migration commands. |
| leapable/ | Leapable shadow-harness / shadow-removal command family. |
| usage.rs / main_tests.rs | Usage text and main tests. |

### calyx-mcp (`crates/calyx-mcp/src/`)
Doc: "MCP interface for agent-facing Calyx operations."

| Module | Description |
|---|---|
| main.rs | stdio entrypoint: decode JSON-RPC, dispatch via `McpServer`, write responses. |
| jsonrpc.rs | Inbound JSON-RPC request decoding. |
| protocol.rs | Response framing and MCP descriptors. |
| schema.rs | Tool input-schema construction. |
| server.rs | Tool registry and dispatch (`McpServer`). |

### calyxd (`crates/calyxd/src/`)
Doc: "`calyxd` library surface" — daemon binary plus public library consumed by calyx-cli.

| Module | Description |
|---|---|
| main.rs | Daemon binary: arg parse, CUDA/VRAM preflight, chain-verify, serve `/metrics`. |
| config.rs | `CalyxConfig` runtime configuration (authoritative). |
| cuda_probe.rs | T02 CUDA device startup probe. |
| vram.rs | T03 VRAM budget probe (NVML). |
| error.rs | `CALYX_DAEMON_*` error taxonomy. |
| health.rs | T04 daemon-readiness probe. |
| metrics.rs + metrics/ | T03 Prometheus metric surface (`/metrics`). |
| server.rs | Metrics HTTP server. |
| mcp_server.rs | T05 loopback MCP-over-socket dispatch transport. |
| verify.rs | PH67 verify-restore byte-level verification. |
| verify_loop.rs | Binary-only periodic chain-verify driver (declared in `main.rs`, not `lib.rs`). |

### calyx-testkit (`crates/calyx-testkit/src/`)
Doc: "Reusable deterministic test scaffolding for Calyx crates."

| Module | Description |
|---|---|
| lib.rs | Single-file crate of deterministic test scaffolding helpers. |

---

## 3. Inter-crate dependency graph

Parsed from each `crates/<x>/Cargo.toml` `[dependencies]` `calyx-*` path entries
(deduplicated; an entry appearing under both `[dependencies]` and a feature/dev table
is listed once). Arrow = "depends on".

```
calyx-core      -> (none)                          [leaf]
calyx-testkit   -> calyx-core
calyx-paths     -> calyx-core
calyx-ledger    -> calyx-core
calyx-forge     -> calyx-core
calyx-mcp       -> calyx-core
calyx-mincut    -> calyx-core, calyx-paths
calyx-aster     -> calyx-core, calyx-forge, calyx-ledger, calyx-paths
calyx-ward      -> calyx-core, calyx-aster, calyx-forge, calyx-ledger, calyx-assay
calyx-loom      -> calyx-core, calyx-aster, calyx-ledger, calyx-ward, calyx-forge
calyx-sextant   -> calyx-core, calyx-aster, calyx-paths, calyx-ward, calyx-loom, calyx-oracle
calyx-lodestar  -> calyx-core, calyx-aster, calyx-ledger, calyx-loom, calyx-mincut,
                   calyx-paths, calyx-sextant, calyx-forge, calyx-ward
calyx-registry  -> calyx-core, calyx-assay, calyx-aster, calyx-forge, calyx-ledger,
                   calyx-loom, calyx-sextant
calyx-anneal    -> calyx-core, calyx-aster, calyx-forge, calyx-ledger, calyx-registry
calyx-assay     -> calyx-core, calyx-aster, calyx-loom, calyx-anneal, calyx-ledger,
                   calyx-lodestar, calyx-mincut, calyx-oracle, calyx-paths,
                   calyx-sextant, calyx-ward
calyx-oracle    -> calyx-core, calyx-assay, calyx-anneal, calyx-aster, calyx-forge,
                   calyx-ledger, calyx-lodestar, calyx-paths, calyx-ward, calyx-loom,
                   calyx-testkit
calyxd          -> calyx-core, calyx-aster, calyx-forge, calyx-mcp, calyx-ledger
calyx-cli       -> calyx-core, calyx-assay, calyx-anneal, calyx-aster, calyx-forge,
                   calyx-ledger, calyx-lodestar, calyx-loom, calyx-oracle, calyx-paths,
                   calyx-registry, calyx-sextant, calyx-ward
```

Layering:

- Leaves (no first-party deps): `calyx-core`.
- Near-leaves (core only, plus testkit/paths/ledger/forge/mcp): `calyx-testkit`,
  `calyx-paths`, `calyx-ledger`, `calyx-forge`, `calyx-mcp`; `calyx-mincut` adds `paths`.
- Mid layer: `calyx-aster`, `calyx-ward`, `calyx-loom`, `calyx-sextant`,
  `calyx-lodestar`, `calyx-registry`, `calyx-anneal`, `calyx-assay`, `calyx-oracle`.
  Note: `calyx-assay`/`calyx-oracle`/`calyx-anneal` form a cluster that aggregates most
  mid-layer crates; `calyx-sextant` and `calyx-oracle` are mutually referenced through
  the assay/oracle cluster (sextant -> oracle; assay/oracle -> sextant).
- Top (binary crates): `calyx-cli` (`calyx`), `calyxd` (`calyxd`), `calyx-mcp`
  (`calyx-mcp`). `calyx-cli` depends on the widest fan-in (13 first-party crates);
  `calyxd` is a thinner daemon over aster/forge/ledger/mcp.

---

## 4. Entry-point traces

### `calyx` — `crates/calyx-cli/src/main.rs`
```
main() -> entry::main()
  entry::main():
    1. verify_restore::try_run(args)      -> PH67 byte-level verify-restore (early exit)
    2. healthcheck_daemon::try_run(args)  -> PH65 T04 daemon-readiness (early exit)
    3. dispatch::run(args)                -> generic command dispatch
         dispatch::run() pattern-matches argv and routes to one handler module, e.g.:
           readback --hex / --vault-tree / --show-manifest -> cli_support, vault_tree, leapable
           vault-manifest / temporal_search readbacks       -> manifest_readback, temporal_readback
           anneal_*                                          -> anneal_commands + anneal_*_readback
           sextant / lens / panel / lodestar / oracle / media-> *_commands, *_readback, *_validation
           verify / provenance / merkle / ledger_store       -> ledger + provenance handlers
```

### `calyxd` — `crates/calyxd/src/main.rs`
```
main() -> parse_args(argv)
  if --validate-config -> validate_config()        -> calyxd::config::CalyxConfig::from_file
  if --config <path>   -> run_server(path):
        calyxd::config::CalyxConfig::from_file      (load runtime config)
        calyxd::cuda_probe::probe_cuda_device       (T02 fatal CUDA preflight)
        calyxd::vram::{NvmlVramUsage, VramBudget}   (T03 VRAM budget audit)
        -> run(config)
  else                 -> run(config):
        VerifyTarget::validate (per target)
        calyxd::metrics::ChainVerifyMetrics::new + verify_loop::run_cycle  (initial verify)
        calyxd::metrics::CalyxMetrics::new + collect_default_zfs_integrity (compose surface)
        if --once -> encode_text + exit
        calyxd::server::MetricsServer::bind         (serve /metrics on loopback)
        verify_loop::spawn_loop + spawn_zfs_metrics_loop (background verify + zfs metrics)
```
(The T05 MCP-over-socket transport lives in `calyxd::mcp_server`; `verify_loop` is the
binary-only periodic chain-verify driver declared in `main.rs`.)

### `calyx-mcp` — `crates/calyx-mcp/src/main.rs`
```
main():
  calyx_mcp::server::McpServer::new()              (tool registry; scaffold registers none yet)
  loop over stdin lines (newline-delimited JSON-RPC):
     calyx_mcp::jsonrpc::decode_jsonrpc_request    (decode request; malformed -> stderr, continue)
     server.dispatch(request)                      (route through tool registry)
     notifications (no id) -> no reply
     serde_json::to_string(response) -> stdout (flush per line)
  EOF on stdin -> clean shutdown (ExitCode::SUCCESS)
```
(Protocol framing is in `protocol.rs`; tool input schemas in `schema.rs`.)

---

## 5. Build / package configuration

### Root `Cargo.toml`
- `[workspace]` `resolver = "2"`, `members = ["crates/*"]` (the 18 crates above).
- `[workspace.package]`: `version = "0.1.0"`, `edition = "2024"`,
  `rust-version = "1.95"`, `publish = false`,
  `repository = "https://github.com/ChrisRoyse/Calyx"`.
- `[workspace.dependencies]` (shared/pinned third-party deps): aes-gcm 0.10, blake3 1,
  bincode (`bincode_reloaded` 3.1.6, serde), ciborium 0.2, crc32fast 1, criterion 0.5,
  cudarc 0.19.7 (default-features off), ed25519-dalek 2, hkdf 0.12, memmap2 0.9,
  nix 0.30 (fs), prometheus 0.14 (default-features off), proptest 1, rand 0.8,
  rand_chacha 0.3, rayon 1, nvml-wrapper 0.10, rusqlite 0.40.1 (bundled),
  serde 1 (derive), serde_json 1, sha2 0.10, thiserror 2, toml 0.8, tracing 0.1,
  ulid 1 (serde), uuid 1 (serde), wide 1, zeroize 1.
- Build profiles (tuned for the workspace's high-churn build cadence):
  - `[profile.dev]` `debug = "line-tables-only"` (function names + file:line in
    backtraces; smaller executables and faster link than full DWARF).
  - `[profile.dev.package."*"]` `debug = false` (dependencies get no debuginfo).
  - No `[profile.release]`/`[profile.bench]` overrides are present in the root
    `Cargo.toml` (use cargo defaults). Linker (mold) and incremental policy are
    machine-local and intentionally not in this file (see
    `docs/implementation/02_BUILD_PERFORMANCE.md`).

### `rust-toolchain.toml`
- `channel = "1.95.0"`, `profile = "minimal"`, `components = ["clippy", "rustfmt"]`.

### `[[bin]]` targets
| Binary | Crate | Path |
|---|---|---|
| calyx | calyx-cli | src/main.rs |
| calyxd | calyxd | src/main.rs |
| calyx-mcp | calyx-mcp | src/main.rs |

### `[[bench]]` targets (Criterion, `harness = false`)
| Bench | Crate | Source |
|---|---|---|
| bench_arena_reset | calyx-aster | benches/bench_arena_reset.rs |
| bench_admission_overhead | calyx-forge | benches/bench_admission_overhead.rs |

### `fuzz/` — cargo-fuzz package (`calyx-fuzz`)
- Standalone package (`[workspace]` empty so it's its own workspace), `publish = false`,
  `edition = "2024"`, `[package.metadata] cargo-fuzz = true`.
- Dependencies: calyx-aster, calyx-core, calyx-mcp, calyx-sextant (path),
  libfuzzer-sys 0.4, serde_json 1.
- Fuzz `[[bin]]` targets (`fuzz/fuzz_targets/`, each `test=false doc=false bench=false`):
  - aster_sst_decode — Aster SST decode.
  - aster_wal_replay — Aster WAL replay.
  - aster_manifest_decode — Aster manifest decode.
  - query_parse — Sextant query parsing.
  - lens_output_decode — Registry lens-output decode.
  - mcp_jsonrpc_decode — MCP JSON-RPC decode.
- Includes `corpus/` (seed corpora) and `README.md`.
