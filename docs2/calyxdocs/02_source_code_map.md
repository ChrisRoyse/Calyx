# 02 — Source Code Map

This document is the **map** of the Calyx Rust workspace at `C:\code\Calyx-Dev`: the
top-level directory layout, the complete per-crate source-file tree with a one-line
description for every `.rs` file under `crates/*/src/`, the inter-crate dependency
graph, entry-point traces, and the build/package configuration.

Per-crate internals (algorithms, struct fields, error taxonomies) are documented in
docs 05–20; this document cross-references them rather than re-describing logic. See
[01_system_overview.md](01_system_overview.md) for the architecture narrative and
[00_INDEX.md](00_INDEX.md) for the full index.

**Source files covered:** every `.rs` file under `crates/*/src/` (877 files across 19
crates), plus `Cargo.toml` (workspace + per-crate), `rust-toolchain.toml`,
`fuzz/Cargo.toml` + `fuzz/fuzz_targets/*.rs`, and `infra/aiwonder/**`.

> **Method note.** The per-file descriptions in §3 were generated mechanically by
> extracting the first module-doc comment (`//!`) from each file; where absent, the
> first top-of-file item-doc (`///`, marked `(item-doc)`) was used; where neither
> exists the line is derived from the file/module name and marked `(no doc comment)`.
> Of the 877 files, 293 carry a module `//!` doc, 14 a top item `///` doc, and 570
> have no top-of-file doc comment. The map reports WHAT IS present, not intent.

> **Count note.** The dispatch brief cites ~1174 `.rs` files; that count includes
> `tests/`, `benches/`, `examples/`, `build.rs`, and `fuzz/` files outside `src/`.
> This map enumerates the **877** files under `crates/*/src/` (the in-crate source
> tree). The fuzz targets are listed separately in §6.3.

---

## 1. Workspace layout

The repository root (`C:\code\Calyx-Dev`) is a Cargo workspace (`resolver = "2"`,
`members = ["crates/*"]`, edition 2024, rust-version 1.95). Top-level entries:

| Path | Purpose (as observed in the tree) |
|------|------------------------------------|
| `crates/` | The 19 workspace member crates (all source code). See §3. |
| `docs/` | Original design/planning material: `dbprdplans/` (PRD plans), `implementation/` (build/impl notes incl. `02_BUILD_PERFORMANCE.md`), `systemspecs/`. |
| `docs2/` | Loose prose/PDF design essays and operator guides (`releaseguide.md`, `infisical-secrets-guide.md`, prompt docs, `*.pdf`). Not part of the numbered reference. |
| `scripts/` | Shell scripts: dataset acquisition (`acquire_*.sh`), build setup (`aiwonder-build-setup.sh`), CI/gate helpers (`check.sh`, `check_manifest_coverage.sh`, `fsv_*.sh`), `dataset_acquire_lib.sh`. |
| `tools/` | Auxiliary tooling; currently `tools/lensforge/` (lens-conversion tooling). |
| `infra/` | Deployment/ops for the `aiwonder` host: systemd units, Prometheus/Grafana/Alertmanager config, restic backup units, ZFS provisioning/scrub scripts, secrets-loader map. See §6.4. |
| `fuzz/` | `cargo-fuzz` package (`calyx-fuzz`, separate `[workspace]`) with 6 libFuzzer targets. See §6.3. |
| `datasets/` | Dataset manifest (`MANIFEST.md`); actual data acquired via `scripts/acquire_*.sh`. |
| `assets/` | PNG marketing/diagram images (`hero.png`, `ledger.png`, `oracle.png`, `ward-guard.png`, etc.). |
| `.github/` | `workflows/` — CI definitions. |
| `.githooks/` | `pre-push` git hook. |
| `.cargo/` | `config.toml` — cargo config (kept machine-agnostic per the workspace `Cargo.toml` comment; linker/incremental are machine-local). |
| `.config/` | `nextest.toml` — cargo-nextest test-runner config. |
| `target/` | Build output (shared workspace target dir). |
| `Cargo.toml`, `Cargo.lock` | Workspace manifest + lockfile. See §6.1. |
| `rust-toolchain.toml` | Pins toolchain `1.95.0`, `profile = minimal`, components `clippy` + `rustfmt`. |
| `env.sh` | Environment bootstrap script. |
| `README.md`, `CONTRIBUTING.md`, `LICENSE` | Standard repo docs. |

### 1.1 Binaries produced

Four executables are built from the workspace. Each is declared with an explicit
`[[bin]]` name and a `src/main.rs` in its crate:

| Binary name | Producing crate | `main.rs` | Role |
|-------------|-----------------|-----------|------|
| `calyx` | `calyx-cli` | `crates/calyx-cli/src/main.rs` | The command-line tool: ingest, search, readbacks, ops, migrate, verify-restore. See [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md). |
| `calyxd` | `calyxd` | `crates/calyxd/src/main.rs` | The daemon: loopback `/metrics` Prometheus endpoint driven by a periodic Ledger chain-verify loop. See [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md). |
| `calyx-mcp` | `calyx-mcp` | `crates/calyx-mcp/src/main.rs` | The MCP stdio server: newline-delimited JSON-RPC over stdin/stdout. See [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md). |
| `calyx-hazard-soak` | `calyx-hazard-soak` | `crates/calyx-hazard-soak/src/main.rs` | The 25-hazard soak harness driver. See [18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md). |

---

## 2. Crate inventory (file counts)

| Crate | `.rs` files under `src/` | Doc topic |
|-------|--------------------------:|-----------|
| `calyx-core` | 24 | [05_core.md](05_core.md) |
| `calyx-paths` | 5 | [17_graph_mincut_paths.md](17_graph_mincut_paths.md) |
| `calyx-mincut` | 8 | [17_graph_mincut_paths.md](17_graph_mincut_paths.md) |
| `calyx-forge` | 55 | [07_forge_math_runtime.md](07_forge_math_runtime.md) |
| `calyx-ledger` | 18 | [14_ledger_provenance.md](14_ledger_provenance.md) |
| `calyx-aster` | 192 | [06_aster_storage_engine.md](06_aster_storage_engine.md) |
| `calyx-loom` | 19 | [10_loom_associations.md](10_loom_associations.md) |
| `calyx-ward` | 16 | [13_ward_guard.md](13_ward_guard.md) |
| `calyx-assay` | 26 | [11_assay_signal_bits.md](11_assay_signal_bits.md) |
| `calyx-sextant` | 67 | [09_sextant_search.md](09_sextant_search.md) |
| `calyx-registry` | 54 | [08_registry_lenses.md](08_registry_lenses.md) |
| `calyx-lodestar` | 22 | [12_lodestar_kernel.md](12_lodestar_kernel.md) |
| `calyx-oracle` | 30 | [16_oracle_prediction.md](16_oracle_prediction.md) |
| `calyx-anneal` | 82 | [15_anneal_optimization.md](15_anneal_optimization.md) |
| `calyx-testkit` | 1 | [18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md) |
| `calyx-hazard-soak` | 20 | [18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md) |
| `calyx-mcp` | 39 | [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md) |
| `calyx-cli` | 183 | [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md) |
| `calyxd` | 16 | [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md) |
| **Total** | **877** | |

---

## 3. Per-crate source-file tree

Paths below are relative to each crate's `src/` directory. Crates are ordered to
roughly follow the dependency layering (foundations first, entry points last).

<!-- BEGIN GENERATED TREE -->

#### calyx-anneal

- `budget.rs` — Creates an unreserved cooperative tick handle for tests and shadow replay. (item-doc)
- `heal/degrade.rs` — degrade module (no doc comment)
- `heal/mod.rs` — heal mod module (no doc comment)
- `heal/rebuild.rs` — rebuild module (no doc comment)
- `heal/rebuild/artifact.rs` — artifact module (no doc comment)
- `heal/rebuild/builders.rs` — builders module (no doc comment)
- `heal/rebuild/scheduler.rs` — scheduler module (no doc comment)
- `heal/rebuild/source.rs` — source module (no doc comment)
- `heal/recalibrate.rs` — recalibrate module (no doc comment)
- `heal/recalibrate/lens.rs` — lens module (no doc comment)
- `heal/recalibrate/store.rs` — store module (no doc comment)
- `heal/recalibrate/tau.rs` — tau module (no doc comment)
- `heal/recalibrate/types.rs` — types module (no doc comment)
- `heal/restore.rs` — restore module (no doc comment)
- `heal/restore/alert.rs` — alert module (no doc comment)
- `heal/restore/barrier.rs` — barrier module (no doc comment)
- `heal/restore/checksum.rs` — checksum module (no doc comment)
- `heal/triggers.rs` — triggers module (no doc comment)
- `heal/triggers/support.rs` — support module (no doc comment)
- `integration_fsv.rs` — integration fsv module (no doc comment)
- `j/goodhart.rs` — goodhart module (no doc comment)
- `j/gradient.rs` — gradient module (no doc comment)
- `j/growth_curve.rs` — growth curve module (no doc comment)
- `j/intelligence_report.rs` — intelligence report module (no doc comment)
- `j/j_composite.rs` — j composite module (no doc comment)
- `j/mod.rs` — j mod module (no doc comment)
- `janitor.rs` — Anneal-managed operational janitor for bounded hotpool buildup.
- `janitor/fs_ops.rs` — fs ops module (no doc comment)
- `janitor/types.rs` — types module (no doc comment)
- `learn/frozen_guard.rs` — frozen guard module (no doc comment)
- `learn/mistake_log.rs` — mistake log module (no doc comment)
- `learn/mod.rs` — learn mod module (no doc comment)
- `learn/online_head.rs` — online head module (no doc comment)
- `learn/online_head/codec.rs` — codec module (no doc comment)
- `learn/online_head/regression.rs` — regression module (no doc comment)
- `learn/online_head/sleep_pass.rs` — sleep pass module (no doc comment)
- `learn/online_head/storage.rs` — storage module (no doc comment)
- `learn/online_head/update.rs` — update module (no doc comment)
- `learn/outcome.rs` — outcome module (no doc comment)
- `learn/outcome/queue.rs` — queue module (no doc comment)
- `learn/regression_assert.rs` — regression assert module (no doc comment)
- `learn/replay_buffer.rs` — replay buffer module (no doc comment)
- `ledger_anneal.rs` — ledger anneal module (no doc comment)
- `lib.rs` — Anneal self-optimization contracts for reversible tuning loops.
- `propose/admission_record.rs` — admission record module (no doc comment)
- `propose/candidate_synth.rs` — candidate synth module (no doc comment)
- `propose/candidate_synth/targets.rs` — targets module (no doc comment)
- `propose/deficit_localize.rs` — deficit localize module (no doc comment)
- `propose/differentiation_gate.rs` — differentiation gate module (no doc comment)
- `propose/mod.rs` — Lens proposal primitives for Anneal.
- `propose/operator_synth.rs` — operator synth module (no doc comment)
- `propose/operator_synth/codec.rs` — codec module (no doc comment)
- `propose/operator_synth/gate.rs` — gate module (no doc comment)
- `propose/operator_synth/storage.rs` — storage module (no doc comment)
- `propose/propose_lens.rs` — propose lens module (no doc comment)
- `propose/registry_hot_add.rs` — registry hot add module (no doc comment)
- `recurrence_schedule.rs` — recurrence schedule module (no doc comment)
- `rollback.rs` — rollback module (no doc comment)
- `rollback_codec.rs` — rollback codec module (no doc comment)
- `shadow.rs` — shadow module (no doc comment)
- `tripwire.rs` — tripwire module (no doc comment)
- `tune/ab_runner.rs` — ab runner module (no doc comment)
- `tune/ab_runner/errors.rs` — errors module (no doc comment)
- `tune/ab_runner/types.rs` — types module (no doc comment)
- `tune/ab_runner/writer.rs` — writer module (no doc comment)
- `tune/bandit.rs` — bandit module (no doc comment)
- `tune/mod.rs` — tune mod module (no doc comment)
- `tune/scope_forge.rs` — scope forge module (no doc comment)
- `tune/scope_forge/types.rs` — types module (no doc comment)
- `tune/scope_forge/writer.rs` — writer module (no doc comment)
- `tune/scope_index.rs` — scope index module (no doc comment)
- `tune/scope_index/types.rs` — types module (no doc comment)
- `tune/scope_index/writer.rs` — writer module (no doc comment)
- `tune/scope_loom.rs` — scope loom module (no doc comment)
- `tune/scope_loom/types.rs` — types module (no doc comment)
- `tune/scope_loom/writer.rs` — writer module (no doc comment)
- `tune/scope_storage.rs` — scope storage module (no doc comment)
- `tune/scope_storage/types.rs` — types module (no doc comment)
- `tune/scope_storage/writer.rs` — writer module (no doc comment)
- `tune/soak_harness.rs` — soak harness module (no doc comment)
- `tune/soak_harness/storage.rs` — storage module (no doc comment)
- `tune/soak_harness/types.rs` — types module (no doc comment)

#### calyx-assay

- `attribution.rs` — Per-sensor signal attribution and bits reports.
- `bayesian.rs` — Conjugate Bayesian posteriors for recurrence rates and oracle consistency.
- `bootstrap.rs` — Deterministic bootstrap confidence intervals.
- `contract.rs` — Lens differentiation contract enforcement.
- `estimate.rs` — Shared Assay estimate types.
- `formula_catalog.rs` — PRD-22 formula coverage catalog.
- `formulas.rs` — PRD-22 Assay formula-name wrappers.
- `gate.rs` — AssayGate facade for lens signal and pair gain.
- `ksg.rs` — KSG-style k-nearest-neighbor mutual information estimators.
- `lib.rs` — Assay signal-bit measurement, panel sufficiency, and persistence contracts.
- `logistic.rs` — Binary outcome logistic-probe MI estimator.
- `loom_adapter.rs` — Aster-backed Assay adapter for Loom materialization planning.
- `mmd.rs` — Maximum mean discrepancy (MMD) drift tests (PRD 26 §7, PH70).
- `n_eff.rs` — Effective-rank reporting for panel redundancy.
- `nmi.rs` — Partitioned histogram normalized mutual information.
- `periodicity.rs` — Periodicity detection on irregularly sampled series (PRD 26 §4, PH52).
- `projection.rs` — Deterministic random projection pre-step for high-dimensional Assay inputs.
- `recurrence_anchor.rs` — PH42 recurrence anchors and oracle self-consistency for Assay.
- `recurrence_hazard.rs` — Inter-event-time hazard ("overdue" anomaly) and CUSUM rate change-point.
- `samples.rs` — samples module (no doc comment)
- `special_fn.rs` — Deterministic special functions shared by the statistical modules.
- `store.rs` — In-memory Assay result CF/cache with provenance.
- `stratified.rs` — Stratified signal-bit accounting for rare sole-carrier anchors.
- `sufficiency.rs` — Panel sufficiency and deficit routing.
- `total_correlation.rs` — Total correlation and interaction information for Assay panels.
- `transfer_entropy.rs` — Transfer entropy over recurrence streams (PRD 26 §4, PH52).

#### calyx-aster

- `cf/family.rs` — Column-family identity and on-disk names.
- `cf/key.rs` — Big-endian key codecs for Aster column families.
- `cf/mod.rs` — Association-native Aster column families and key codecs.
- `cf/router.rs` — router module (no doc comment)
- `cf/router_tests.rs` — router tests module (no doc comment)
- `cf/tests.rs` — cf tests module (no doc comment)
- `collection/enhancement.rs` — enhancement module (no doc comment)
- `collection/enhancement_tests.rs` — enhancement tests module (no doc comment)
- `collection/mod.rs` — Collection descriptors for PH53 collections-as-any-model.
- `collection/policy.rs` — policy module (no doc comment)
- `collection/schema.rs` — schema module (no doc comment)
- `collection/tests.rs` — collection tests module (no doc comment)
- `compaction/mod.rs` — Snapshot-safe SST compaction and hot/cold tier placement.
- `compaction/scan.rs` — scan module (no doc comment)
- `compaction/tests.rs` — compaction tests module (no doc comment)
- `compaction/tiering.rs` — Hot/cold physical storage tier. (item-doc)
- `dedup/audit.rs` — audit module (no doc comment)
- `dedup/audit_tests.rs` — audit tests module (no doc comment)
- `dedup/compression_ratio.rs` — compression ratio module (no doc comment)
- `dedup/engine.rs` — Dedup decision engine for PH41 T02.
- `dedup/engine_tests.rs` — engine tests module (no doc comment)
- `dedup/ingest_at.rs` — ingest at module (no doc comment)
- `dedup/ingest_at_tests.rs` — ingest at tests module (no doc comment)
- `dedup/ingest_event.rs` — ingest event module (no doc comment)
- `dedup/ingest_input.rs` — ingest input module (no doc comment)
- `dedup/ingest_ledger.rs` — ingest ledger module (no doc comment)
- `dedup/mod.rs` — Vault-level deduplication policy contracts.
- `dedup/policy.rs` — policy module (no doc comment)
- `dedup/signature.rs` — signature module (no doc comment)
- `dedup/signature_tests.rs` — signature tests module (no doc comment)
- `erase.rs` — Lawful/user-requested erasure for Aster vault content (PH61 T01).
- `erase/ledger.rs` — ledger module (no doc comment)
- `erase/ledger_tests.rs` — ledger tests module (no doc comment)
- `erase/tests.rs` — erase tests module (no doc comment)
- `file_lock.rs` — file lock module (no doc comment)
- `gc/ann_gc.rs` — PH58 ANN tombstone GC with read-safe copy-on-write swaps.
- `gc/ann_gc/tests.rs` — ann gc tests module (no doc comment)
- `gc/compaction_gc.rs` — PH58 compaction GC facade for tombstone-heavy SST sets.
- `gc/compaction_gc/tests.rs` — compaction gc tests module (no doc comment)
- `gc/mod.rs` — Garbage-collection and reclaimer scaffolding for Aster.
- `gc/orphan_reconciler.rs` — PH58 orphan slot/index reconciler.
- `gc/orphan_reconciler/tests.rs` — orphan reconciler tests module (no doc comment)
- `gc/panel_version_gc.rs` — PH58 panel/codebook version GC and retired-lens pruning.
- `gc/panel_version_gc/codebook.rs` — codebook module (no doc comment)
- `gc/panel_version_gc/tests.rs` — panel version gc tests module (no doc comment)
- `gc/snapshot_gc.rs` — Snapshot-pin watchdog for MVCC reader leases (PRD 24 §4).
- `gc/snapshot_gc/reclaimer.rs` — Module-local fail-closed code for background GC scheduler failures. (item-doc)
- `gc/snapshot_gc/tests.rs` — snapshot gc tests module (no doc comment)
- `gc/wal_recycler.rs` — PH58 WAL recycler with fsync anti-storm guards.
- `gc/wal_recycler/tests.rs` — wal recycler tests module (no doc comment)
- `index/btree.rs` — Btree secondary-index key encoding (PH54 T01, discriminant 0x10).
- `index/btree_tests.rs` — Synthetic deterministic FSV for the btree index key encoding (PH54 T01).
- `index/inverted.rs` — Inverted secondary-index key encoding and BM25-style term queries (PH54 T03).
- `index/inverted_maintenance.rs` — Stages all posting rows plus the updated stats row for one field value. (item-doc)
- `index/inverted_tests.rs` — inverted tests module (no doc comment)
- `index/maintenance.rs` — Atomic secondary-index maintenance staged into layer write batches (PH54 T04).
- `index/maintenance_tests.rs` — maintenance tests module (no doc comment)
- `index/mod.rs` — Secondary-index trait, runtime IndexSpec, and index-value types (PH54 T01).
- `index/rebuild.rs` — Secondary-index verification and self-heal rebuild (PH54 T05).
- `index/rebuild/data.rs` — data module (no doc comment)
- `index/rebuild/expected.rs` — expected module (no doc comment)
- `index/rebuild/scan.rs` — scan module (no doc comment)
- `index/rebuild/support.rs` — support module (no doc comment)
- `index/rebuild/types.rs` — types module (no doc comment)
- `index/terms.rs` — Shared term normalization for Aster inverted secondary indexes.
- `layers/blob.rs` — Blob layer: chunked payload + manifest.
- `layers/blob/tests.rs` — blob tests module (no doc comment)
- `layers/document.rs` — Document (collection, doc_id, path...) -> leaf key-encoding layer.
- `layers/document/codec.rs` — codec module (no doc comment)
- `layers/document/errors.rs` — errors module (no doc comment)
- `layers/document/schema.rs` — schema module (no doc comment)
- `layers/document/tests.rs` — document tests module (no doc comment)
- `layers/document/tree.rs` — tree module (no doc comment)
- `layers/kv.rs` — KV (ns, key) -> value layer with check-on-read TTL.
- `layers/kv/tests.rs` — kv tests module (no doc comment)
- `layers/mod.rs` — Key-encoding layers over Aster's ordered transactional core.
- `layers/relational.rs` — Relational (collection, pk) -> row key-encoding layer.
- `layers/relational/tests.rs` — relational tests module (no doc comment)
- `layers/retention_reclaimer.rs` — Physical retention reclaimer for time-series points and blob rows (issue #591).
- `layers/retention_reclaimer/tests.rs` — retention reclaimer tests module (no doc comment)
- `layers/timeseries.rs` — Time-series (series, ts) -> point layer with continuous rollups.
- `layers/timeseries/tests.rs` — timeseries tests module (no doc comment)
- `ledger_view.rs` — Read-only Ledger column-family view over an Aster vault directory.
- `lib.rs` — Aster storage engine skeleton for Calyx column families and WAL.
- `manifest/mod.rs` — Atomic manifest and recovery ordering for Aster vaults.
- `manifest/quarantine.rs` — quarantine module (no doc comment)
- `manifest/tests.rs` — manifest tests module (no doc comment)
- `memtable/bounded.rs` — Logical per-row overhead used by memtable admission accounting. (item-doc)
- `memtable/mod.rs` — Bounded ordered memtable for Aster writes.
- `mmap_col.rs` — Read-only mmap accessor for cold/columnar Aster bytes.
- `mvcc/lease.rs` — Sequence allocation, freshness, and reader lease handles.
- `mvcc/mod.rs` — Vault-wide MVCC sequence and snapshot scaffolding.
- `mvcc/read_barrier.rs` — read barrier module (no doc comment)
- `mvcc/store.rs` — In-memory MVCC row table used to define the cross-CF snapshot contract.
- `mvcc/store/gc.rs` — store gc module (no doc comment)
- `mvcc/tests.rs` — mvcc tests module (no doc comment)
- `mvcc/tests/allocator.rs` — allocator module (no doc comment)
- `mvcc/tests/freshness.rs` — freshness module (no doc comment)
- `mvcc/tests/isolation.rs` — isolation module (no doc comment)
- `mvcc/tests/read_barrier.rs` — read barrier module (no doc comment)
- `mvcc/tests/router_bridge.rs` — router bridge module (no doc comment)
- `mvcc/tests/snapshot_gc.rs` — snapshot gc module (no doc comment)
- `olap/mod.rs` — olap mod module (no doc comment)
- `olap/tests.rs` — olap tests module (no doc comment)
- `olap/types.rs` — types module (no doc comment)
- `plain_column/key.rs` — Key-encoding for the plain-collection wide-column layer.
- `plain_column/mod.rs` — Sparse wide-column root op for plain (0-lens) collections.
- `plain_column/tests.rs` — Synthetic 3-row corpus with deliberately sparse columns. (item-doc)
- `plain_column/types.rs` — One materialized wide-column cell read back from the store. (item-doc)
- `plain_graph/key.rs` — plain graph key module (no doc comment)
- `plain_graph/mod.rs` — Plain graph key-encoding layer for 0-lens collections.
- `plain_graph/tests.rs` — plain graph tests module (no doc comment)
- `plain_graph/types.rs` — types module (no doc comment)
- `pressure.rs` — Disk-pressure guard for fail-closed hotpool write admission.
- `recurrence.rs` — Recurrence-series rows stored in Aster's dedicated recurrence CF.
- `recurrence_tests.rs` — recurrence tests module (no doc comment)
- `redaction.rs` — PII input redaction modes for privacy-preserving ingest.
- `residency.rs` — Vault data residency — governance by construction (PRD 30 §4, axiom A33).
- `resource/collect.rs` — Collector assembling ResourceStatus from an open vault store + its directory.
- `resource/counters.rs` — Process-lifetime backpressure event counters for one vault store.
- `resource/heap.rs` — Heap RSS probe over /proc/self/status (fail-closed off Linux).
- `resource/leases.rs` — Active reader-lease registry for oldest-pinned-seq gap accounting.
- `resource/mod.rs` — Aggregate resource_status surface (PRD 18 §4, 24 §8; issue #592).
- `resource/status.rs` — Aggregate resource-health status (PRD 18 §4 resource_status, 24 §8).
- `resource/tests.rs` — resource tests module (no doc comment)
- `retention.rs` — Per-collection retention policy and TTL sweep support (PH61 T03).
- `retention/tests.rs` — retention tests module (no doc comment)
- `security/lens_store.rs` — Lens-store cross-vault guard (PH60 · T06).
- `security/mod.rs` — Security utilities for PH60 tenant isolation (outermost ZFS crypto-at-rest).
- `security/zfs.rs` — ZFS native-encryption probe + operator guidance (PH60 · T06).
- `sst/arrow.rs` — sst arrow module (no doc comment)
- `sst/bloom.rs` — Small deterministic Bloom filter for SST point-lookups.
- `sst/level.rs` — sst level module (no doc comment)
- `sst/mod.rs` — Immutable SSTable writer and mmap reader.
- `storage_names.rs` — Canonical on-disk file-name contract for Aster-owned directories.
- `stream/backpressure.rs` — Token-bucket backpressure guard for the streaming ingest pipeline (A26).
- `stream/mod.rs` — Streaming ingest pipeline: channel-backed ingester, on-the-fly TurboQuant.
- `stream/quantize_online.rs` — On-the-fly slot quantization for the streaming ingest pipeline.
- `stream/stream_tests.rs` — stream tests module (no doc comment)
- `stride_fsv.rs` — STRIDE defense FSV proofs for PH61 T06.
- `supply_chain.rs` — Supply-chain integrity checks for PH61.
- `supply_chain/tests.rs` — supply chain tests module (no doc comment)
- `timetravel/mod.rs` — Time-travel reads: as_of(t) over MVCC time-keyed snapshots.
- `timetravel/retention.rs` — Retention horizon guard for PH72 time-travel reads.
- `timetravel/tests.rs` — A clock the test advances between commits so each group-commit is stamped. (item-doc)
- `timetravel/time_index.rs` — The time_index column family: a wall-clock → MVCC-seqno map.
- `txn/cross_model.rs` — cross model module (no doc comment)
- `txn/mod.rs` — Cross-model transaction serialization for one Aster vault.
- `txn/tests.rs` — txn tests module (no doc comment)
- `txn/tests/support.rs` — support module (no doc comment)
- `txn/tests/support/edges.rs` — edges module (no doc comment)
- `txn/validation.rs` — validation module (no doc comment)
- `vault.rs` — Aster VaultStore implementation over the PH08 MVCC CF table.
- `vault/anchor_codec.rs` — anchor codec module (no doc comment)
- `vault/batch_ingest.rs` — batch ingest module (no doc comment)
- `vault/cf_codec.rs` — cf codec module (no doc comment)
- `vault/commit.rs` — commit module (no doc comment)
- `vault/compaction_bridge.rs` — compaction bridge module (no doc comment)
- `vault/compaction_tests.rs` — compaction tests module (no doc comment)
- `vault/context.rs` — VaultContext — the PH60 tenant-isolation aggregate (T07).
- `vault/cursor.rs` — cursor module (no doc comment)
- `vault/dedup_commit.rs` — dedup commit module (no doc comment)
- `vault/durable.rs` — durable module (no doc comment)
- `vault/durable/manifest_ops.rs` — manifest ops module (no doc comment)
- `vault/encode.rs` — encode module (no doc comment)
- `vault/gc_bridge.rs` — Vault-facing bridge for snapshot GC scheduler ticks.
- `vault/grant.rs` — Cross-vault grant model with default-deny semantics (PH60 · T03).
- `vault/key.rs` — Per-vault key derivation and authenticated encryption (PH60 · T01).
- `vault/keyspace.rs` — Per-vault keyspace isolation (PH60 · T02).
- `vault/layer_commit.rs` — layer commit module (no doc comment)
- `vault/ledger_append.rs` — ledger append module (no doc comment)
- `vault/ledger_atomicity_tests.rs` — ledger atomicity tests module (no doc comment)
- `vault/ledger_checkpoint_tests.rs` — ledger checkpoint tests module (no doc comment)
- `vault/ledger_hook.rs` — ledger hook module (no doc comment)
- `vault/ledger_integration_tests.rs` — ledger integration tests module (no doc comment)
- `vault/ledger_stub.rs` — ledger stub module (no doc comment)
- `vault/ledger_timestamp_tests.rs` — ledger timestamp tests module (no doc comment)
- `vault/quota.rs` — Per-vault resource quotas with backpressure (PH60 · T04).
- `vault/recovery_tests.rs` — recovery tests module (no doc comment)
- `vault/retention_horizon.rs` — retention horizon module (no doc comment)
- `vault/router_bridge.rs` — router bridge module (no doc comment)
- `vault/seq_readback.rs` — Returns the visible MVCC sequence for one CF/key at snapshot. (item-doc)
- `vault/slot_backfill.rs` — slot backfill module (no doc comment)
- `vault/slot_column.rs` — slot column module (no doc comment)
- `vault/store.rs` — vault store module (no doc comment)
- `vault/temporal_xterm.rs` — temporal xterm module (no doc comment)
- `vault/tests.rs` — vault tests module (no doc comment)
- `wal/batch.rs` — wal batch module (no doc comment)
- `wal/mod.rs` — Write-ahead log storage for Aster.
- `wal/record.rs` — WAL record framing.
- `wal/segment.rs` — WAL segment naming helpers.
- `wal/tests.rs` — wal tests module (no doc comment)

#### calyx-core

- `alloc/arena.rs` — Arena / bump allocator (PH56 · T01).
- `alloc/mod.rs` — Bounded allocation primitives (PH56 — Stage S13).
- `alloc/slab.rs` — Slab / fixed-size object pool (PH56 · T02).
- `cache/lru_ttl.rs` — Generic LRU + TTL, byte-capped cache (PH56 · T03).
- `cache/lru_ttl/tests.rs` — lru ttl tests module (no doc comment)
- `cache/mod.rs` — Bounded caches (PH56 — Stage S13).
- `cold_start.rs` — Cold-start trust-state guard for provisional vaults (PRD 30 section 5).
- `consent.rs` — Consent and purpose-tag checks for privacy-governed Calyx processing.
- `cosine.rs` — Shared dense cosine helpers.
- `enums.rs` — Closed shared enum vocabulary for Calyx engines.
- `error.rs` — Closed CALYX_* error catalog.
- `ids.rs` — Stable Calyx identifiers and content-addressing helpers.
- `lib.rs` — Core Calyx identifiers, model contracts, and shared types.
- `model/anchor.rs` — Grounded outcome anchors.
- `model/constellation.rs` — Atomic Calyx constellation record.
- `model/mod.rs` — Constellation data-model structs.
- `model/signal.rs` — Shared signal, reference, and flag structs.
- `model/slot.rs` — Panel and slot declarations.
- `model/validation.rs` — Record-schema validation helpers.
- `model/vector.rs` — Slot vector representations.
- `security.rs` — Canonical transport-security and authentication types (PRD 30 §2).
- `temporal.rs` — Shared temporal policy contracts for post-retrieval boosting.
- `time.rs` — Clock injection and monotonic stamp types.
- `traits.rs` — Engine trait boundaries shared by Calyx crates.

#### calyx-forge

- `autotune/explorer.rs` — explorer module (no doc comment)
- `autotune/explorer_tests.rs` — explorer tests module (no doc comment)
- `autotune/microbench.rs` — microbench module (no doc comment)
- `autotune/microbench_tests.rs` — microbench tests module (no doc comment)
- `autotune/mod.rs` — autotune mod module (no doc comment)
- `autotune/promotion.rs` — promotion module (no doc comment)
- `autotune/promotion_tests.rs` — promotion tests module (no doc comment)
- `autotune/tests.rs` — autotune tests module (no doc comment)
- `backend.rs` — Backend operations implemented by the Stage 2 Backend trait. (item-doc)
- `compression_report/build.rs` — build module (no doc comment)
- `compression_report/mod.rs` — compression report mod module (no doc comment)
- `compression_report/types.rs` — types module (no doc comment)
- `compression_report/validate.rs` — validate module (no doc comment)
- `cpu/distance.rs` — distance module (no doc comment)
- `cpu/gemm.rs` — gemm module (no doc comment)
- `cpu/guard.rs` — guard module (no doc comment)
- `cpu/mod.rs` — cpu mod module (no doc comment)
- `cpu/normalize.rs` — normalize module (no doc comment)
- `cpu/topk.rs` — topk module (no doc comment)
- `cuda/context.rs` — context module (no doc comment)
- `cuda/distance.rs` — distance module (no doc comment)
- `cuda/distance_tests.rs` — distance tests module (no doc comment)
- `cuda/gemm.rs` — gemm module (no doc comment)
- `cuda/gemm/mxfp4_path.rs` — mxfp4 path module (no doc comment)
- `cuda/gemm/mxfp8_path.rs` — mxfp8 path module (no doc comment)
- `cuda/grouped_gemm.rs` — grouped gemm module (no doc comment)
- `cuda/grouped_gemm_tests.rs` — grouped gemm tests module (no doc comment)
- `cuda/kernels.rs` — kernels module (no doc comment)
- `cuda/mod.rs` — cuda mod module (no doc comment)
- `cuda/mxfp4.rs` — mxfp4 module (no doc comment)
- `cuda/mxfp8.rs` — mxfp8 module (no doc comment)
- `cuda/ragged_gemm.rs` — ragged gemm module (no doc comment)
- `cuda/topk.rs` — topk module (no doc comment)
- `cuda/topk_tests.rs` — topk tests module (no doc comment)
- `error.rs` — forge error module (no doc comment)
- `lib.rs` — Forge math runtime skeleton for CPU, CUDA, and quantized kernels.
- `quant/binary.rs` — binary module (no doc comment)
- `quant/binary/tests.rs` — binary tests module (no doc comment)
- `quant/mod.rs` — quant mod module (no doc comment)
- `quant/mxfp4_codec.rs` — mxfp4 codec module (no doc comment)
- `quant/qjl.rs` — qjl module (no doc comment)
- `quant/rotation.rs` — rotation module (no doc comment)
- `quant/turboquant.rs` — turboquant module (no doc comment)
- `quant/turboquant/tests.rs` — turboquant tests module (no doc comment)
- `vram/admission.rs` — Admission control for large Forge VRAM dispatches.
- `vram/admission_tests.rs` — admission tests module (no doc comment)
- `vram/budget.rs` — The VRAM budgeter: soft-cap config, live free-VRAM admission, atomic reserve.
- `vram/budget_tests.rs` — budget tests module (no doc comment)
- `vram/lru_evict.rs` — LRU eviction registry for GPU-resident blocks (PH57 · T02).
- `vram/lru_evict_tests.rs` — FSV for the GPU-block LRU eviction registry (PH57 · T02).
- `vram/mod.rs` — VRAM budgeting + admission control for calyx-forge.
- `vram/oom_guard.rs` — Last-resort OOM guard for Forge CUDA allocation and dispatch.
- `vram/oom_guard_tests.rs` — oom guard tests module (no doc comment)
- `vram/yield_policy.rs` — Anneal yield policy for PH57 T05.
- `vram/yield_policy_tests.rs` — yield policy tests module (no doc comment)

#### calyx-hazard-soak

- `cli.rs` — cli module (no doc comment)
- `hazards/heap_soak.rs` — heap soak module (no doc comment)
- `hazards/mod.rs` — hazards mod module (no doc comment)
- `hazards/numerical.rs` — numerical module (no doc comment)
- `hazards/numerical_support.rs` — numerical support module (no doc comment)
- `hazards/operational.rs` — operational module (no doc comment)
- `hazards/operational_h13_14.rs` — operational h13 14 module (no doc comment)
- `hazards/operational_h15_16.rs` — operational h15 16 module (no doc comment)
- `hazards/operational_h17_19.rs` — operational h17 19 module (no doc comment)
- `hazards/operational_h20_21.rs` — operational h20 21 module (no doc comment)
- `hazards/operational_support.rs` — operational support module (no doc comment)
- `hazards/resource.rs` — resource module (no doc comment)
- `hazards/resource_hazards_6_8.rs` — resource hazards 6 8 module (no doc comment)
- `hazards/resource_support.rs` — resource support module (no doc comment)
- `hazards/security.rs` — security module (no doc comment)
- `hazards/security_support.rs` — security support module (no doc comment)
- `lib.rs` — hazard-soak lib module (no doc comment)
- `main.rs` — hazard-soak main module (no doc comment)
- `soak.rs` — soak module (no doc comment)
- `soak/ops.rs` — soak ops module (no doc comment)

#### calyx-ledger

- `append.rs` — Append-only ledger writer and row-store adapters.
- `append/tests.rs` — append tests module (no doc comment)
- `audit.rs` — Quarantine-aware Ledger provenance query surface.
- `audit/mentions.rs` — mentions module (no doc comment)
- `checkpoint.rs` — Periodic Merkle checkpoint rows for the append-only ledger.
- `codec.rs` — Deterministic binary codec for ledger entries.
- `entry.rs` — Canonical ledger entry structure and hash framing.
- `group_commit.rs` — Group-commit hook for adding Ledger rows to a storage write batch.
- `kind.rs` — Stable ledger entry kinds and wire codes.
- `lib.rs` — Append-only Ledger provenance primitives.
- `merkle.rs` — Merkle roots and signed export bundles for Ledger ranges.
- `redaction.rs` — Ledger payload redaction and secret guardrails.
- `reproduce.rs` — Reproduce-time lens lookup and deterministic slot re-measurement.
- `reproduce/fusion.rs` — Fusion replay and reproduce verdicts.
- `tombstone.rs` — Erasure tombstones for the append-only Ledger.
- `tombstone/tests.rs` — tombstone tests module (no doc comment)
- `tombstone/wire.rs` — wire module (no doc comment)
- `verify.rs` — Ledger hash-chain verification.

#### calyx-lodestar

- `aster_bridge.rs` — aster bridge module (no doc comment)
- `dfvs.rs` — dfvs module (no doc comment)
- `error.rs` — lodestar error module (no doc comment)
- `grounding_gaps.rs` — grounding gaps module (no doc comment)
- `hierarchical.rs` — hierarchical module (no doc comment)
- `incremental.rs` — incremental module (no doc comment)
- `kernel.rs` — kernel module (no doc comment)
- `kernel_answer.rs` — kernel answer module (no doc comment)
- `kernel_graph.rs` — kernel graph module (no doc comment)
- `kernel_health.rs` — Engine-level kernel_health(kernel_id) aggregate (PRD 08 §8).
- `kernel_index.rs` — kernel index module (no doc comment)
- `label_propagation.rs` — Grounded label propagation by harmonic extension over the association graph.
- `lib.rs` — Lodestar grounding-kernel discovery and maintenance.
- `loom_assoc.rs` — loom assoc module (no doc comment)
- `multi_scope.rs` — multi scope module (no doc comment)
- `provenance.rs` — Ledger-backed Lodestar provenance writers.
- `recall_test.rs` — recall test module (no doc comment)
- `scope.rs` — scope module (no doc comment)
- `scope_cache.rs` — scope cache module (no doc comment)
- `scope_report.rs` — scope report module (no doc comment)
- `summarize.rs` — Universal summarization via the multi-scope kernel (PH72 · T06).
- `temporal_kernel.rs` — temporal kernel module (no doc comment)

#### calyx-loom

- `abundance.rs` — Honest DDA abundance reporting.
- `agreement_graph.rs` — In-memory xterm CF and agreement graph readbacks.
- `blind_spot.rs` — Cross-lens anomaly detector.
- `cross_term.rs` — Cross-term value types and CPU/GPU-parity math kernels.
- `error.rs` — Loom-local fail-closed error helpers.
- `lib.rs` — Loom DDA cross-term and agreement-graph engine.
- `lru_cache.rs` — Small deterministic LRU cache for lazy cross-terms.
- `materialization.rs` — Cross-term materialization policy.
- `reactive/durable.rs` — Durable reactive rows stored in Aster's reactive CF.
- `reactive/engine.rs` — The reactive engine: registry + bounded fired-event queue + audit log.
- `reactive/mod.rs` — Reactive trigger/subscription engine (PH72 · T02).
- `reactive/signals.rs` — Real ReactiveSignals sources backing reactive conditions.
- `reactive/subscription.rs` — Public subscription API over the reactive trigger engine.
- `recurrence/cross_terms.rs` — cross terms module (no doc comment)
- `recurrence/mod.rs` — Bounded recurrence-series storage over Aster recurrence CF rows.
- `recurrence/periodic.rs` — periodic module (no doc comment)
- `recurrence/series_store.rs` — series store module (no doc comment)
- `recurrence/signature.rs` — Recurrence signature facade over the Aster ingest detector.
- `recurrence/tests.rs` — recurrence tests module (no doc comment)

#### calyx-mcp

- `jsonrpc.rs` — Fail-closed JSON-RPC wire decoding for MCP requests.
- `lib.rs` — MCP interface for agent-facing Calyx operations.
- `main.rs` — calyx-mcp stdio entrypoint.
- `protocol.rs` — JSON-RPC 2.0 response framing and MCP tool descriptors.
- `schema.rs` — JSON Schema constructors for MCP tool input declarations.
- `server.rs` — MCP server: tool registry plus dispatch for the three mandatory methods.
- `tools/ingest.rs` — Ingest, anchor, and measure MCP tools for PH63 T03.
- `tools/ingest/anchor.rs` — anchor module (no doc comment)
- `tools/ingest/report.rs` — report module (no doc comment)
- `tools/ingest/tests.rs` — ingest tests module (no doc comment)
- `tools/intelligence.rs` — Intelligence extraction MCP tools for PH63 T06.
- `tools/intelligence/core.rs` — core module (no doc comment)
- `tools/intelligence/guard.rs` — guard module (no doc comment)
- `tools/intelligence/metrics.rs` — metrics module (no doc comment)
- `tools/intelligence/model.rs` — model module (no doc comment)
- `tools/intelligence/propose.rs` — propose module (no doc comment)
- `tools/intelligence/tests.rs` — intelligence tests module (no doc comment)
- `tools/mod.rs` — Registered MCP tool groups.
- `tools/provenance.rs` — Provenance and ops MCP tools for PH63 T07.
- `tools/provenance/core.rs` — core module (no doc comment)
- `tools/provenance/ids.rs` — ids module (no doc comment)
- `tools/provenance/quarantine.rs` — quarantine module (no doc comment)
- `tools/provenance/status.rs` — status module (no doc comment)
- `tools/provenance/tests.rs` — provenance tests module (no doc comment)
- `tools/search.rs` — Search, kernel-answer, and neighbors MCP tools for PH63 T04.
- `tools/search/engine.rs` — engine module (no doc comment)
- `tools/search/extension_tests.rs` — extension tests module (no doc comment)
- `tools/search/extensions.rs` — extensions module (no doc comment)
- `tools/search/extensions/guard_generate.rs` — guard generate module (no doc comment)
- `tools/search/extensions/render.rs` — render module (no doc comment)
- `tools/search/extensions/runtime.rs` — runtime module (no doc comment)
- `tools/search/extensions/xterms.rs` — xterms module (no doc comment)
- `tools/search/output.rs` — output module (no doc comment)
- `tools/search/tests.rs` — search tests module (no doc comment)
- `tools/test_support.rs` — test support module (no doc comment)
- `tools/vault.rs` — Vault and panel MCP tools for PH63 T02.
- `tools/vault/lens.rs` — lens module (no doc comment)
- `tools/vault/store.rs` — store module (no doc comment)
- `tools/vault/tests.rs` — vault tests module (no doc comment)

#### calyx-mincut

- `betweenness.rs` — betweenness module (no doc comment)
- `error.rs` — mincut error module (no doc comment)
- `graph_builder.rs` — graph builder module (no doc comment)
- `lib.rs` — Directed graph primitives for Calyx grounding kernels.
- `lp_scaffold.rs` — lp scaffold module (no doc comment)
- `scc.rs` — scc module (no doc comment)
- `spectral.rs` — spectral module (no doc comment)
- `spectral_linalg.rs` — spectral linalg module (no doc comment)

#### calyx-oracle

- `butterfly.rs` — Hop-attenuated Oracle butterfly tree traversal.
- `butterfly/context.rs` — context module (no doc comment)
- `butterfly_tests.rs` — butterfly tests module (no doc comment)
- `complete.rs` — Unified Oracle completion primitive over partial constellations.
- `complete_test_support.rs` — complete test support module (no doc comment)
- `complete_tests.rs` — complete tests module (no doc comment)
- `energy.rs` — PH51 energy descent substrate for complete().
- `energy_tests.rs` — energy tests module (no doc comment)
- `error.rs` — Structured Oracle error catalog.
- `honesty_gate.rs` — Oracle honesty gate backed by Assay sufficiency rows.
- `honesty_gate_tests.rs` — honesty gate tests module (no doc comment)
- `lib.rs` — Oracle consequence prediction and completion primitives.
- `prd22.rs` — PRD-22 Oracle formula primitives.
- `predict.rs` — Vault-backed Oracle consequence prediction.
- `predict/context.rs` — context module (no doc comment)
- `predict_tests.rs` — predict tests module (no doc comment)
- `reverse_query.rs` — Reverse Oracle traversal for epistemic symmetry.
- `reverse_query_context.rs` — reverse query context module (no doc comment)
- `reverse_query_tests.rs` — reverse query tests module (no doc comment)
- `self_consistency.rs` — Oracle self-consistency measured from grounded recurrence streams.
- `self_consistency_tests.rs` — self consistency tests module (no doc comment)
- `super_intel.rs` — PH50 super-intelligence tier measurement.
- `super_intel_full.rs` — PH50 full six-tier super-intelligence predicate.
- `super_intel_full_tests.rs` — super intel full tests module (no doc comment)
- `super_intel_tests.rs` — super intel tests module (no doc comment)
- `super_intel_types.rs` — PH50 super-intelligence predicate and reverse-query data contracts.
- `time_prediction.rs` — time prediction module (no doc comment)
- `time_prediction_tests.rs` — time prediction tests module (no doc comment)
- `types.rs` — Public Oracle contract types for consequence prediction.
- `types_tests.rs` — types tests module (no doc comment)

#### calyx-paths

- `attenuation.rs` — attenuation module (no doc comment)
- `error.rs` — paths error module (no doc comment)
- `graph.rs` — graph module (no doc comment)
- `lib.rs` — Path and graph traversal over Calyx association networks.
- `traversal.rs` — traversal module (no doc comment)

#### calyx-registry

- `backfill.rs` — Durable lazy backfill scheduler state.
- `commission.rs` — commission module (no doc comment)
- `commission/manifest.rs` — manifest module (no doc comment)
- `commission/tests.rs` — commission tests module (no doc comment)
- `compression/codec.rs` — codec module (no doc comment)
- `compression/mod.rs` — compression mod module (no doc comment)
- `compression/recall.rs` — recall module (no doc comment)
- `drift.rs` — registry drift module (no doc comment)
- `explain.rs` — explain module (no doc comment)
- `frozen.rs` — Runtime dtype declared by a frozen lens contract. (item-doc)
- `frozen/tests.rs` — frozen tests module (no doc comment)
- `ingest_microbatch.rs` — ingest microbatch module (no doc comment)
- `ingest_microbatch/tests.rs` — ingest microbatch tests module (no doc comment)
- `lens.rs` — lens module (no doc comment)
- `lens/tests.rs` — lens tests module (no doc comment)
- `lib.rs` — Registry runtimes for frozen Calyx lenses.
- `panel_ops.rs` — panel ops module (no doc comment)
- `panels/defaults.rs` — defaults module (no doc comment)
- `panels/mod.rs` — panels mod module (no doc comment)
- `persistence.rs` — persistence module (no doc comment)
- `placement.rs` — placement module (no doc comment)
- `profile.rs` — profile module (no doc comment)
- `profile/assay.rs` — assay module (no doc comment)
- `profile/cost.rs` — cost module (no doc comment)
- `profile/gating.rs` — gating module (no doc comment)
- `profile/tests.rs` — profile tests module (no doc comment)
- `runtime/adapters/axis.rs` — axis module (no doc comment)
- `runtime/adapters/lens.rs` — lens module (no doc comment)
- `runtime/adapters/mod.rs` — PH74 multimodal adapter lenses.
- `runtime/adapters/pack.rs` — pack module (no doc comment)
- `runtime/adapters/tests.rs` — adapters tests module (no doc comment)
- `runtime/algorithmic.rs` — Deterministic, data-local feature encoders with no model weights. (item-doc)
- `runtime/candle.rs` — candle module (no doc comment)
- `runtime/candle/load.rs` — load module (no doc comment)
- `runtime/candle/options.rs` — options module (no doc comment)
- `runtime/candle/pooling.rs` — pooling module (no doc comment)
- `runtime/candle/tests.rs` — candle tests module (no doc comment)
- `runtime/common.rs` — common module (no doc comment)
- `runtime/external_cmd.rs` — external cmd module (no doc comment)
- `runtime/mod.rs` — Lens runtime implementations.
- `runtime/onnx.rs` — onnx module (no doc comment)
- `runtime/onnx/custom.rs` — custom module (no doc comment)
- `runtime/onnx/fastembed_runtime.rs` — fastembed runtime module (no doc comment)
- `runtime/onnx/tests.rs` — onnx tests module (no doc comment)
- `runtime/static_lookup.rs` — static lookup module (no doc comment)
- `runtime/static_lookup/tests.rs` — static lookup tests module (no doc comment)
- `runtime/tei_http.rs` — Resident TEI endpoint on aiwonder. (item-doc)
- `spec.rs` — spec module (no doc comment)
- `swap.rs` — Slot declaration supplied when a lens is hot-added to a panel. (item-doc)
- `swap/tests.rs` — swap tests module (no doc comment)
- `temporal/e2_recency.rs` — e2 recency module (no doc comment)
- `temporal/e3_periodic.rs` — e3 periodic module (no doc comment)
- `temporal/e4_positional.rs` — e4 positional module (no doc comment)
- `temporal/mod.rs` — temporal mod module (no doc comment)

#### calyx-sextant

- `error.rs` — Sextant-local fail-closed error helpers.
- `fusion/mod.rs` — Fusion strategies for Sextant search.
- `fusion/pipeline.rs` — Pipeline strategy helpers.
- `fusion/profiles.rs` — profiles module (no doc comment)
- `fusion/rrf.rs` — rrf module (no doc comment)
- `fusion/single.rs` — single module (no doc comment)
- `guarded.rs` — Ward-backed guarded search filtering.
- `hit.rs` — Provenanced search hit types.
- `index/bm25.rs` — BM25 scorer using Lucene-like defaults.
- `index/diskann/build.rs` — Vamana graph construction for the DiskANN on-disk format (PH68 T01/T02).
- `index/diskann/concat.rs` — DiskANN over materialized concat cross-term (xterm) vectors.
- `index/diskann/dual.rs` — Dual-DiskANN search for asymmetric server-scale slots.
- `index/diskann/graph.rs` — DiskANN on-disk graph format: header, page-aligned node blocks, writer.
- `index/diskann/mod.rs` — DiskANN on-disk graph index (PH68, server-only).
- `index/diskann/search/helpers.rs` — helpers module (no doc comment)
- `index/diskann/search/mod.rs` — DiskANN beam search and raw-f32 rescore (PH68 T02).
- `index/diskann/token.rs` — Token DiskANN + segmented MaxSim rerank for server-scale multi slots.
- `index/diskann/token_sidecar.rs` — Binary sidecars for token DiskANN MaxSim indexes.
- `index/dual.rs` — Dual directional index scaffold for asymmetric slots.
- `index/funnel.rs` — Kernel-first 3-hop funnel for server-scale vault search.
- `index/hnsw/graph.rs` — hnsw graph module (no doc comment)
- `index/hnsw/mod.rs` — Deterministic in-RAM dense HNSW-style index.
- `index/hnsw/scored.rs` — scored module (no doc comment)
- `index/inverted.rs` — In-memory inverted index with BM25 scoring.
- `index/mod.rs` — Per-slot index trait and implementations.
- `index/multi.rs` — Multi-vector token index with MaxSim late interaction.
- `index/quant_config.rs` — Per-slot quantization policy for Sextant indexes.
- `index/spann/centroids.rs` — SPANN centroid state persisted as centroids.spn.
- `index/spann/mod.rs` — SPANN sparse-slot index: centroid ANN in RAM, posting lists on disk.
- `index/spann/posting.rs` — SPANN posting-list blocks: varint deltas inside zstd-compressed files.
- `index/tokenizer.rs` — Deterministic lowercase whitespace/punctuation tokenizer.
- `lib.rs` — Sextant search and navigation for Calyx retrieval.
- `navigation/consensus.rs` — Cross-lens agreement / disagreement search (PRD 10 §4).
- `navigation/hdbscan.rs` — Deterministic HDBSCAN* condensed-tree clustering (skill discovery core).
- `navigation/lens_nav.rs` — Navigation primitives over per-slot indexes.
- `navigation/mod.rs` — Navigation modes over the constellation space (PRD 10 §4, §9).
- `navigation/skills.rs` — Hierarchical skill discovery and skill-scoped search (PRD 10 §4).
- `navigation/traverse.rs` — Asymmetric hop-attenuated traversal (PRD 10 §4, 18 §4).
- `planner.rs` — Deterministic intent classifier and bounded query planner.
- `planner_explain.rs` — Planner-enriched explain metadata.
- `query/ask.rs` — PH55 ASK execution: retrieval grounding plus PH33/PH49-compatible stubs.
- `query/ask/fsv_tests.rs` — ask fsv tests module (no doc comment)
- `query/ask/tests.rs` — ask tests module (no doc comment)
- `query/executor.rs` — One-pass PH55 cross-model query executor.
- `query/executor/fsv_tests.rs` — executor fsv tests module (no doc comment)
- `query/executor/support.rs` — support module (no doc comment)
- `query/executor/tests.rs` — executor tests module (no doc comment)
- `query/executor/tests/ask.rs` — ask module (no doc comment)
- `query/mod.rs` — Query surfaces for Stage 4 search and PH55 cross-model planning.
- `query/planner.rs` — Cross-model universal query planner for PH55.
- `query/planner/fsv_tests.rs` — planner fsv tests module (no doc comment)
- `query/planner/tests.rs` — planner tests module (no doc comment)
- `query/search.rs` — Stage 4 search query request types and freshness policy.
- `query_admission.rs` — Bounded query admission for Sextant read/search paths.
- `reranker.rs` — Request-scoped reranker hook for the :8089 cross-encoder surface.
- `search.rs` — Top-level search engine wiring SlotIndexMap to fusion.
- `search_support.rs` — Small pure helpers for search.rs.
- `slot_index_map.rs` — Concurrent-read-safe SlotId to index registry.
- `temporal/boost.rs` — boost module (no doc comment)
- `temporal/causal_gate.rs` — causal gate module (no doc comment)
- `temporal/mod.rs` — Temporal search policy types for AP-60 post-retrieval boosting.
- `temporal/recall_budget.rs` — Bounded windowed recall for AP-60 temporal primary retrieval (issue #633).
- `temporal/recurrence_boost.rs` — recurrence boost module (no doc comment)
- `temporal/search.rs` — temporal search module (no doc comment)
- `temporal/tests.rs` — temporal tests module (no doc comment)
- `temporal/window.rs` — window module (no doc comment)
- `util.rs` — sextant util module (no doc comment)

#### calyx-testkit

- `lib.rs` — Reusable deterministic test scaffolding for Calyx crates.

#### calyx-ward

- `calibrate.rs` — Per-slot conformal tau calibration for Ward guard profiles.
- `drift.rs` — Rolling drift monitoring for calibrated Ward guards.
- `error.rs` — Ward error catalog with fail-closed Calyx codes.
- `generate.rs` — Identity-locked generation guard loop for PH39.
- `guard.rs` — Per-slot Ward guard math.
- `identity.rs` — Identity-locked Ward profile wrappers.
- `ledger.rs` — Ledger provenance writers for Ward calibration and guard verdicts.
- `lib.rs` — Ward guard profile types for per-slot cosine policy enforcement.
- `novelty.rs` — Novelty routing for failed Ward verdicts.
- `polis.rs` — Polis civic-panel guard validation over deterministic synthetic personas.
- `profile.rs` — Guard profile configuration shared by Ward guard calls.
- `query.rs` — Incoming-query Ward guard over trusted regions.
- `required.rs` — Assay-bit derived required-slot selection for Ward profiles.
- `speaker_lens.rs` — WavLM speaker lens adapter for PH39 identity slots.
- `style_lens.rs` — RoBERTa style lens adapter for PH39 identity slots.
- `verdict.rs` — Structured verdicts emitted by Ward guard calls.

#### calyxd

- `config.rs` — CalyxConfig — the single authoritative runtime configuration for calyxd.
- `cuda_probe.rs` — CUDA device preflight for calyxd (PH65 · T02).
- `error.rs` — Daemon error taxonomy mapping to stable CALYX_* codes (PH65).
- `health.rs` — PH65 · T04 — calyxd daemon-readiness healthcheck.
- `lib.rs` — calyxd library surface.
- `main.rs` — Calyx daemon: Ledger chain-verify metrics on a loopback /metrics endpoint.
- `mcp_server.rs` — Loopback-only MCP-over-socket transport for calyxd (PH65 · T05).
- `metrics.rs` — Prometheus registry for the Ledger chain-verify gauge family (PH66, issue #602).
- `metrics/calyx.rs` — CalyxMetrics: the full daemon /metrics surface (PH66 T03, issue #538).
- `metrics/hazards.rs` — The PH59 25-hazard register exposed as one gauge per hazard (PH66 T03).
- `metrics/zfs.rs` — ZFS integrity metric collector for calyxd (issue #729).
- `server.rs` — Loopback-only HTTP listener serving GET /metrics (PH65 bind rules).
- `startup.rs` — startup module (no doc comment)
- `verify.rs` — calyx verify-restore — byte-level read-back verification of a restored vault.
- `verify_loop.rs` — Periodic Ledger chain-verify cycle feeding the chain-verify gauge family.

> `calyx-cli` is the largest crate (183 src files) and is enumerated in full at §3
> above under the **calyx-cli** heading.

<!-- END GENERATED TREE -->

---

## 4. Crate dependency graph

Edges below are derived by grepping each crate's `Cargo.toml` for `calyx-*` workspace
(path) dependencies. There are **103 internal edges** across the 19 crates. Edges are
counted once per (crate, dependency) pair regardless of `[dependencies]` vs
`[dev-dependencies]`/`[build-dependencies]` section.

### 4.1 Edge list (`crate -> dependency`)

```
calyx-core      ->  (none)

calyx-paths     -> calyx-core

calyx-testkit   -> calyx-core

calyx-forge     -> calyx-core

calyx-ledger    -> calyx-core

calyx-mincut    -> calyx-core
calyx-mincut    -> calyx-paths

calyx-aster     -> calyx-core
calyx-aster     -> calyx-forge
calyx-aster     -> calyx-ledger
calyx-aster     -> calyx-paths

calyx-loom      -> calyx-aster
calyx-loom      -> calyx-core
calyx-loom      -> calyx-forge
calyx-loom      -> calyx-ledger
calyx-loom      -> calyx-ward

calyx-ward      -> calyx-assay
calyx-ward      -> calyx-aster
calyx-ward      -> calyx-core
calyx-ward      -> calyx-forge
calyx-ward      -> calyx-ledger

calyx-assay     -> calyx-anneal
calyx-assay     -> calyx-aster
calyx-assay     -> calyx-core
calyx-assay     -> calyx-ledger
calyx-assay     -> calyx-lodestar
calyx-assay     -> calyx-loom
calyx-assay     -> calyx-mincut
calyx-assay     -> calyx-oracle
calyx-assay     -> calyx-paths
calyx-assay     -> calyx-sextant
calyx-assay     -> calyx-ward

calyx-sextant   -> calyx-aster
calyx-sextant   -> calyx-core
calyx-sextant   -> calyx-loom
calyx-sextant   -> calyx-oracle
calyx-sextant   -> calyx-paths
calyx-sextant   -> calyx-ward

calyx-registry  -> calyx-assay
calyx-registry  -> calyx-aster
calyx-registry  -> calyx-core
calyx-registry  -> calyx-forge
calyx-registry  -> calyx-ledger
calyx-registry  -> calyx-loom
calyx-registry  -> calyx-sextant

calyx-lodestar  -> calyx-aster
calyx-lodestar  -> calyx-core
calyx-lodestar  -> calyx-forge
calyx-lodestar  -> calyx-ledger
calyx-lodestar  -> calyx-loom
calyx-lodestar  -> calyx-mincut
calyx-lodestar  -> calyx-paths
calyx-lodestar  -> calyx-sextant
calyx-lodestar  -> calyx-ward

calyx-oracle    -> calyx-anneal
calyx-oracle    -> calyx-assay
calyx-oracle    -> calyx-aster
calyx-oracle    -> calyx-core
calyx-oracle    -> calyx-forge
calyx-oracle    -> calyx-ledger
calyx-oracle    -> calyx-lodestar
calyx-oracle    -> calyx-loom
calyx-oracle    -> calyx-paths
calyx-oracle    -> calyx-testkit
calyx-oracle    -> calyx-ward

calyx-anneal    -> calyx-aster
calyx-anneal    -> calyx-core
calyx-anneal    -> calyx-forge
calyx-anneal    -> calyx-ledger
calyx-anneal    -> calyx-registry

calyx-hazard-soak -> calyx-anneal
calyx-hazard-soak -> calyx-aster
calyx-hazard-soak -> calyx-core
calyx-hazard-soak -> calyx-forge
calyx-hazard-soak -> calyx-ledger
calyx-hazard-soak -> calyx-registry
calyx-hazard-soak -> calyx-sextant

calyx-mcp       -> calyx-anneal
calyx-mcp       -> calyx-aster
calyx-mcp       -> calyx-core
calyx-mcp       -> calyx-ledger
calyx-mcp       -> calyx-loom
calyx-mcp       -> calyx-paths
calyx-mcp       -> calyx-registry
calyx-mcp       -> calyx-sextant
calyx-mcp       -> calyx-ward

calyxd          -> calyx-aster
calyxd          -> calyx-core
calyxd          -> calyx-forge
calyxd          -> calyx-ledger
calyxd          -> calyx-mcp

calyx-cli       -> calyx-anneal
calyx-cli       -> calyx-assay
calyx-cli       -> calyx-aster
calyx-cli       -> calyx-core
calyx-cli       -> calyx-forge
calyx-cli       -> calyx-ledger
calyx-cli       -> calyx-lodestar
calyx-cli       -> calyx-loom
calyx-cli       -> calyx-oracle
calyx-cli       -> calyx-paths
calyx-cli       -> calyx-registry
calyx-cli       -> calyx-sextant
calyx-cli       -> calyx-ward
```

### 4.2 Out-degree and in-degree

| Crate | Out-deg (depends on) | In-deg (depended on by) |
|-------|---------------------:|------------------------:|
| calyx-cli | 13 | 0 |
| calyx-assay | 11 | 4 |
| calyx-oracle | 11 | 3 |
| calyx-lodestar | 9 | 3 |
| calyx-mcp | 9 | 1 |
| calyx-hazard-soak | 7 | 0 |
| calyx-registry | 7 | 4 |
| calyx-sextant | 6 | 6 |
| calyx-anneal | 5 | 5 |
| calyx-loom | 5 | 7 |
| calyx-ward | 5 | 7 |
| calyxd | 5 | 0 |
| calyx-aster | 4 | 12 |
| calyx-mincut | 2 | 2 |
| calyx-forge | 1 | 11 |
| calyx-ledger | 1 | 13 |
| calyx-paths | 1 | 8 |
| calyx-testkit | 1 | 1 |
| **calyx-core** | **0** | **18** |

### 4.3 Foundation crates and leaf entry points

- **Foundation crates** (depended on by the most crates, lowest layer):
  - `calyx-core` — **0 internal deps**, depended on by **18** of the other 18 crates
    (every crate). It is the universal foundation: ids, error catalog, data model,
    shared traits/enums. See [05_core.md](05_core.md).
  - `calyx-paths` — only depends on `calyx-core`; depended on by 8 crates. With
    `calyx-core` it forms the second foundation layer (graph/path primitives). See
    [17_graph_mincut_paths.md](17_graph_mincut_paths.md).
  - `calyx-ledger` (in-deg 13) and `calyx-forge` (in-deg 11) are also near-foundational:
    each depends only on `calyx-core` and is pulled in by most engine crates.
- **Leaf entry points** (out-deg high, in-deg 0 — nothing depends on them):
  - `calyx-cli` (out-deg 13), `calyxd` (out-deg 5), `calyx-hazard-soak` (out-deg 7).
    `calyx-mcp` is a near-leaf: its only consumer is `calyxd` (in-deg 1), which embeds
    the MCP server over a loopback socket.
- **Cycle note.** No `calyx-*` build cycle exists in `[dependencies]` (Cargo would
  reject it). `calyx-assay -> calyx-oracle`/`calyx-sextant`/`calyx-lodestar` and the
  reverse `calyx-oracle -> calyx-assay`, `calyx-sextant -> calyx-oracle`,
  `calyx-lodestar -> calyx-sextant` appear in the edge list because some of these are
  **dev-dependencies** (test-only), not normal build dependencies. The normal-build
  layering remains acyclic; the apparent back-edges are test wiring.

---

## 5. Entry-point traces

High-level traces from each binary's `main` through its top modules to the crates it
pulls in. Detailed command/tool surfaces are in
[19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md) and
[20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md).

### 5.1 `calyx` — `crates/calyx-cli/src/main.rs`

```
main()  →  entry::main()           (crates/calyx-cli/src/entry.rs)
  ├─ verify_restore::try_run       — short-circuits `verify-restore` (PH67 T03)
  ├─ healthcheck_daemon::try_run   — short-circuits `healthcheck --config` (PH65 T04)
  ├─ cmd::try_run                  — structured subcommand table (crates/calyx-cli/src/cmd/)
  │      ├─ cmd/ingest             → calyx-aster, calyx-registry, calyx-loom, calyx-ledger
  │      ├─ cmd/search             → calyx-sextant, calyx-ward, calyx-oracle
  │      ├─ cmd/intelligence       → calyx-assay, calyx-anneal, calyx-lodestar, calyx-oracle
  │      ├─ cmd/provenance         → calyx-ledger
  │      └─ cmd/readback, vault, healthcheck
  └─ dispatch::run                 — legacy/flat dispatch for remaining readbacks
         (anneal_*, oracle_*, sextant_*, lodestar_*, media_*, temporal_*, ph42_*, …)
```

`main.rs` declares ~110 sibling modules (anneal readbacks, oracle readbacks, sextant
validation harnesses, leapable dual-write/shadow harnesses, migrate, ops, navigate,
merkle, etc.). Error handling routes through `error.rs` (`CliError::emit()` → stable
`ExitCode`). The crate links all 13 engine crates (see §4.1) — it is the broadest
consumer in the workspace.

### 5.2 `calyxd` — `crates/calyxd/src/main.rs`

```
#[tokio::main] main()             (crates/calyxd/src/main.rs)
  ├─ parse_args                    — --vault/--ledger/--bind/--interval-secs/--once/--config/--validate-config/--audit-vram
  ├─ startup::validate_config      (crates/calyxd/src/startup.rs)  → calyxd::config::CalyxConfig
  ├─ verify_loop::run_cycle        — synchronous first Ledger chain-verify before bind
  │      └─ uses calyx_aster::ledger_view  → calyx-aster, calyx-ledger
  ├─ calyxd::metrics::CalyxMetrics — Prometheus registry (chain-verify + 25-hazard + ZFS)
  ├─ calyxd::server::MetricsServer — loopback-only GET /metrics
  └─ verify_loop::spawn_loop       — periodic re-verify, fed by CancellationToken
```

Library modules consumed (single source of truth, also reused by `calyx-cli` and the
healthcheck): `config`, `cuda_probe` (→ uses Forge/CUDA preflight), `error`, `health`,
`mcp_server` (embeds `calyx-mcp`), `metrics` (+`metrics/calyx.rs`, `metrics/hazards.rs`,
`metrics/zfs.rs`), `server`, `verify`, `vram`. Crates pulled in: `calyx-core`,
`calyx-aster`, `calyx-forge`, `calyx-ledger`, `calyx-mcp`.

### 5.3 `calyx-mcp` — `crates/calyx-mcp/src/main.rs`

```
main()                            (crates/calyx-mcp/src/main.rs)
  ├─ McpServer::new                (crates/calyx-mcp/src/server.rs)
  ├─ tools::register_all           (crates/calyx-mcp/src/tools/mod.rs)
  │      ├─ tools/vault            → calyx-aster, calyx-registry  (PH63 T02)
  │      ├─ tools/ingest           → calyx-aster, calyx-loom      (PH63 T03)
  │      ├─ tools/search           → calyx-sextant, calyx-ward    (PH63 T04)
  │      ├─ tools/intelligence     → calyx-anneal, calyx-assay*   (PH63 T06)
  │      └─ tools/provenance       → calyx-ledger                 (PH63 T07)
  └─ stdin loop:
         decode_jsonrpc_request  (jsonrpc.rs)  →  server.dispatch  →  serde_json → stdout
```

stdout is reserved for JSON-RPC responses; all diagnostics go to stderr. Notifications
(no `id`) receive no reply. Crates pulled in: `calyx-core`, `calyx-aster`,
`calyx-ledger`, `calyx-loom`, `calyx-paths`, `calyx-registry`, `calyx-sextant`,
`calyx-ward`, `calyx-anneal`.

---

## 6. Build / package configuration

### 6.1 Workspace `Cargo.toml`

| Field | Value |
|-------|-------|
| `[workspace] resolver` | `"2"` |
| `members` | `["crates/*"]` (globs all 19 crates) |
| `[workspace.package] version` | `0.1.0` |
| `edition` | `2024` |
| `rust-version` | `1.95` |
| `repository` | `https://github.com/ChrisRoyse/Calyx` |
| `publish` | `false` |

`[workspace.dependencies]` pins the shared third-party stack (versions as written):
`aes-gcm 0.10`, `blake3 1`, `bincode` (= `bincode_reloaded 3.1.6`, serde feature),
`ciborium 0.2`, `crc32fast 1`, `criterion 0.5`, `cudarc 0.19.7` (no default features),
`ed25519-dalek 2`, `filetime 0.2`, `hkdf 0.12`, `memmap2 0.9`, `nix 0.30` (fs only),
`prometheus 0.14` (no defaults), `proptest 1`, `rand 0.8`, `rand_chacha 0.3`,
`rayon 1`, `nvml-wrapper 0.10`, `rusqlite 0.40.1` (bundled), `serde 1` (derive),
`serde_json 1`, `sha2 0.10`, `thiserror 2`, `tokio 1`, `tokio-util 0.7`, `toml 0.8`,
`tracing 0.1`, `tracing-subscriber 0.3`, `ulid 1` (serde), `uuid 1` (serde), `wide 1`,
`zeroize 1`, `zstd 0.13`.

**Build profiles** (tuned to shrink the shared `target/` dir for the multi-agent build
cadence; per the in-file comment, the cargo default `debug = 2` ballooned `target/` to
~190 GB):

| Profile | Setting | Effect |
|---------|---------|--------|
| `[profile.dev]` | `debug = "line-tables-only"` | First-party crates keep function names + file:line in backtraces; no full DWARF. |
| `[profile.dev.package."*"]` | `debug = false` | Dependencies get no debuginfo (largest `target/` size reduction). |

No custom `[profile.release]` is set in the workspace `Cargo.toml` (cargo defaults
apply). Linker (mold) and incremental policy are machine-local (see
`docs/implementation/02_BUILD_PERFORMANCE.md`, referenced in the manifest comment).

### 6.2 `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.95.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

### 6.3 Fuzz targets (`fuzz/`)

`fuzz/Cargo.toml` is a standalone `cargo-fuzz` package (`name = "calyx-fuzz"`,
`edition = 2024`, `publish = false`, its own `[workspace]` so it is excluded from the
main workspace; `[package.metadata] cargo-fuzz = true`). It depends on path crates
`calyx-aster`, `calyx-core`, `calyx-mcp`, `calyx-sextant`, plus `libfuzzer-sys 0.4` and
`serde_json 1`. Six libFuzzer targets (each `[[bin]]` with `test/doc/bench = false`):

| Target | Path | Exercises |
|--------|------|-----------|
| `aster_sst_decode` | `fuzz_targets/aster_sst_decode.rs` | Aster SST block decode. |
| `aster_wal_replay` | `fuzz_targets/aster_wal_replay.rs` | WAL record framing / replay. |
| `aster_manifest_decode` | `fuzz_targets/aster_manifest_decode.rs` | Manifest decode/recovery. |
| `query_parse` | `fuzz_targets/query_parse.rs` | Sextant query parsing. |
| `lens_output_decode` | `fuzz_targets/lens_output_decode.rs` | Lens runtime output decode. |
| `mcp_jsonrpc_decode` | `fuzz_targets/mcp_jsonrpc_decode.rs` | MCP JSON-RPC wire decode. |

### 6.4 `infra/` (operations for the `aiwonder` host)

All units are repo-owned and operator-installed (non-root, loopback-bound). Contents:

| Path | What it is |
|------|------------|
| `infra/aiwonder/systemd/calyxd.service` | systemd unit for `calyxd` (PH66 T01, #536). Runs `User=croyse`, loads `/run/leapable/secrets/calyx.env`, `ExecStart=…/calyxd --config …/calyx.toml`, `ExecStartPost=…/calyx healthcheck --wait 30`, `Restart=on-failure`, `LimitNOFILE=1048576`. After `network-online.target` + `leapable-secrets-load.service`. |
| `infra/aiwonder/backup/calyx-backup.service` | restic backup unit (PH67 T02). |
| `infra/aiwonder/backup/calyx-backup.timer` | Hourly restic backup timer (#542): `OnCalendar=hourly`, `Persistent=true`, `RandomizedDelaySec=300`. RPO = 1h. Requires the `.service`. |
| `infra/aiwonder/backup/restic-backup.sh`, `restic-restore.sh`, `zfs-snapshot.sh` | Backup/restore + ZFS snapshot scripts. |
| `infra/aiwonder/backup/dr-drill-runbook.md`, `README.md` | DR drill runbook (PH67 T04) + backup README. |
| `infra/aiwonder/calyx.toml` | The deployed runtime config consumed by `calyxd --config` (parsed by `calyxd::config::CalyxConfig`). See [03_configuration.md](03_configuration.md). |
| `infra/aiwonder/prometheus/calyx-scrape.yml` | Prometheus scrape config for the `calyxd` `/metrics` endpoint. |
| `infra/aiwonder/grafana/calyx-dashboard.json` | Grafana dashboard. |
| `infra/aiwonder/alertmanager/calyx-alerts.yml`, `calyx-alerts.test.yml` | Alertmanager rules + test. |
| `infra/aiwonder/bin/*.sh` | Healthcheck (`calyx-aiwonder-healthcheck.sh`), deploy-wiring installer, ZFS integrity verifier. |
| `infra/aiwonder/ops/*.sh` | Service install, ZFS provisioning/scrub, bitrot FSV, data relocation. |
| `infra/aiwonder/secrets-loader/calyx.env.map.json` | Secrets-loader env mapping (Infisical → `calyx.env`). |

There are **no `*.service`/`*.timer` files outside `infra/aiwonder/`**, and the systemd
units target the `aiwonder` host only.

---

## 7. Gaps / not covered

- This is a **map**, not a behavioral spec: it does not enumerate struct fields,
  function signatures, error variants, or algorithm steps. Those live in docs 04–20.
- 570 of 877 source files have no top-of-file doc comment; their one-liners are
  **derived from the file/module name** and explicitly marked `(no doc comment)`. They
  describe the file's likely role from naming only, not from reading the body.
- The `~1174 .rs` figure in the dispatch brief includes files outside `crates/*/src/`
  (`tests/`, `benches/`, `examples/`, `build.rs`, and the 6 `fuzz_targets/`). This map
  covers the **877** in-`src/` files plus the fuzz targets (§6.3). The full external
  test inventory is in [21_test_suite.md](21_test_suite.md).
- Stub/aspirational markers (`todo!()`, unimplemented paths) are noted in the
  per-subsystem docs, not here. One naming signal visible in the tree:
  `calyx-sextant/src/query/ask.rs` advertises "PH33/PH49-compatible **stubs**".
