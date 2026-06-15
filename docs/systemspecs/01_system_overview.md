# 01. System Overview

**Source files covered:** `Cargo.toml` (workspace), `README.md`, `crates/*/Cargo.toml`, `crates/*/src/lib.rs`, `crates/calyx-cli/src/main.rs`, `crates/calyxd/src/main.rs`, `crates/calyx-mcp/src/main.rs`, `docs/implementation/03_PHASE_MAP.md`, `docs/dbprdplans/*`. Subsystem-level claims are traced in the per-crate deep-dive documents 04–16; this document summarizes and cross-references them.

> Documentation discipline: every statement here is derived from the workspace source as of branch `issue569-ph71-remove-shadow` (2026-06-15). Where the README/PRDs state design targets, this document marks them as *targets*, not measured guarantees. "Not determined from source" is used where the code does not settle a question.

---

## 1. What Calyx Is

Calyx is an **association-native database** implemented as a Rust workspace (`edition = 2024`, `rust-version = 1.95`). Rather than storing rows and running queries against them, Calyx stores *constellations* (multi-slot objects), measures each slot through frozen, content-addressed **lenses** (embedders/feature extractors), indexes the resulting vectors per slot, and answers queries by **fusing** per-slot retrieval, then **guarding** the result against out-of-distribution and ungrounded responses. Provenance for every measurement, kernel, guard verdict, and answer is written to a hash-chained **ledger**.

The system is delivered as a Rust library workspace plus three binaries:

| Binary | Crate | Role |
|---|---|---|
| `calyx` | `calyx-cli` | Operator/readback/FSV/migration CLI (no server) |
| `calyxd` | `calyxd` | Loopback daemon: Prometheus `/metrics`, ledger chain-verify loop, VRAM preflight, healthcheck, restore verify |
| `calyx-mcp` | `calyx-mcp` | JSON-RPC 2.0 MCP (Model Context Protocol) server over stdio |

All build/test/FSV work is performed on a remote host ("aiwonder", `/home/croyse/calyx`); the local checkout is for authoring (per `README.md`). See [18_test_suite.md](18_test_suite.md).

### 1.1 Stage/Phase model

The codebase is built in numbered **stages** (S0–S20+), each composed of **phases** (PH00…PH74). Each crate maps to one or more stages. The current build state per `README.md` is Stage 9 (PH40–PH42) closeout, with Stages 0–8 FSV-signed-off. Component provenance below notes the introducing stage.

---

## 2. Architecture

### 2.1 Process / surface map

Calyx is not a multi-process service mesh; it is an embedded engine with three thin entry binaries over a shared crate stack.

| Surface | Process/Transport | Binding | Technology | Purpose |
|---|---|---|---|---|
| Embedded engine | in-process library | — | Rust crates `calyx-*` | All storage, math, search, guard, ledger logic |
| `calyxd` metrics | HTTP `/metrics` | loopback only (enforced ×3) | Prometheus text exposition | Operational metrics (chain-verify, ingest/search, guard, hazard, ZFS, VRAM) |
| `calyxd` MCP socket | length-prefixed framed socket | loopback only | `CalyxMcpServer` reusing `calyx-mcp` dispatch | MCP over socket (present; `main` does not yet wire it) |
| `calyx-mcp` | JSON-RPC 2.0 over stdio | stdio | `initialize`/`tools/list`/`tools/call` | MCP tool host (zero production tools registered yet) |
| `calyx` CLI | process invocation | — | manual arg dispatch (no clap) | Readback, FSV, migration, drills |

See [16_mcp_and_daemon.md](16_mcp_and_daemon.md) for transport, ports, and lifecycle detail.

### 2.2 Technology stack

| Layer | Technology | Version constraint (workspace) |
|---|---|---|
| Language / edition | Rust | toolchain `1.95.0` (`rust-toolchain.toml`), edition 2024 |
| Hashing | BLAKE3, SHA-2, CRC32 | `blake3 = 1`, `sha2 = 0.10`, `crc32fast = 1` |
| Signatures | ed25519 | `ed25519-dalek = 2` |
| Symmetric crypto / KDF | AES-GCM, HKDF | `aes-gcm = 0.10`, `hkdf = 0.12` |
| Serialization | serde, serde_json, bincode (reloaded), CBOR, TOML | `serde = 1`, `bincode_reloaded = 3.1.6`, `ciborium = 0.2`, `toml = 0.8` |
| Memory-mapped IO | memmap2 | `0.9` |
| Embedded SQL (migration source) | rusqlite (bundled SQLite) | `0.40.1` |
| SIMD | `wide` (f32x8/f32x16) | `1` |
| GPU | CUDA sm_120 via `cudarc`, NVML via `nvml-wrapper` | `cudarc 0.19.7`, `nvml-wrapper 0.10` |
| Parallelism | rayon | `1` |
| IDs | ULID, UUID | `ulid = 1`, `uuid = 1` |
| Metrics | prometheus | `0.14` |
| Property testing | proptest | `1` |
| Errors | thiserror | `2` |
| Zeroization | zeroize | `1` |

(Full list in root `Cargo.toml` `[workspace.dependencies]`; see [03_configuration.md](03_configuration.md).)

### 2.3 Layering / dependency graph

`calyx-core` is the single leaf foundation (zero intra-workspace deps). Crates layer upward to the binaries. Edges below are parsed from each `crates/*/Cargo.toml`.

```
calyx-core            -> (none) — foundation
calyx-paths           -> core
calyx-mincut          -> core, paths
calyx-forge           -> core
calyx-ledger          -> core
calyx-mcp             -> core
calyx-testkit         -> core
calyx-aster           -> core, forge, ledger, paths
calyx-ward            -> core, forge, aster, ledger, assay
calyx-loom            -> core, forge, aster, ledger, ward
calyx-sextant         -> core, paths, aster, loom, ward, oracle
calyx-registry        -> core, forge, aster, ledger, loom, sextant, assay
calyx-anneal          -> core, forge, aster, ledger, registry
calyx-lodestar        -> core, forge, paths, mincut, aster, ledger, loom, sextant, ward
calyx-oracle          -> core, forge, paths, aster, ledger, loom, lodestar, ward, assay, anneal, testkit
calyx-assay           -> core, paths, mincut, aster, ledger, loom, lodestar, sextant, ward, oracle, anneal
calyxd                -> core, forge, aster, ledger, mcp
calyx-cli             -> (almost all: core, forge, aster, ledger, paths, loom, sextant, ward, lodestar, oracle, anneal, registry, assay)
```

> Note: the `assay`/`oracle`/`sextant` cluster has mutual high-level edges (e.g. `sextant -> oracle`, `oracle -> assay`, `assay -> sextant`) reflecting cross-subsystem integration tests and wiring; the build is acyclic at the Cargo level. Full edge listing and per-crate module trees: [02_source_code_map.md](02_source_code_map.md).

---

## 3. Subsystems

Each subsystem has a dedicated deep-dive document. One-paragraph summaries follow; the introducing stage is noted.

### 3.1 Core foundation — `calyx-core` (S0) → [04_core_foundation.md](04_core_foundation.md)
The dependency-free foundation. Defines stable identifier types (ULID-backed `VaultId`; 16-byte truncated-BLAKE3 content-addressed `LensId`/`CxId`; `u16` `SlotId`; `SlotKey`) and a length-delimited `content_address` hasher; the closed PRD-18 `CALYX_*` error catalog (`CalyxError`/`CalyxErrorCode`, with stable codes, meanings, remediations); shared enums (`Modality`, `SlotShape`, `QuantPolicy`, `AnchorKind`, `SlotState`, `AbsentReason`); the Constellation/Slot/Panel/Anchor/Signal data model with schema validation; four engine traits (`Lens`, `Index`, `VaultStore`, `Estimator`); an injected `Clock`; dense-cosine helpers; and bounded allocation (`Arena`, `SlabPool`) / cache (`LruTtlCache`) primitives enforcing the bounded-resource axiom.

### 3.2 Aster storage engine — `calyx-aster` (S1) → [05_aster_storage.md](05_aster_storage.md)
Embedded LSM storage. Writes are framed into a CRC32-checked WAL (magic `CXW1`, 20-byte headers, 64 MiB segments) and durably committed via a group-commit batcher coalescing appends into one fsync within a 2 ms window. Each commit takes one vault-wide MVCC sequence and lands atomically across column families into a versioned in-memory row table with snapshot-isolated, tombstone-aware visibility and reader leases. Bounded memtables flush to immutable mmap'd SSTables (magic `CXS1`, v2, bloom-filtered, CRC-verified). An atomic JSON manifest (`CURRENT`/`MANIFEST` swap) plus manifest-first WAL replay drives crash recovery. ~30 static column families (plus per-slot vector columns) carry big-endian keys; content-addressed constellation ingest is idempotent. Snapshot-safe compaction with debt-scored scheduling, hot/cold ZFS tiering, and btree + BM25 secondary indexes. This is Calyx's "database schema" layer.

### 3.3 Forge math runtime — `calyx-forge` (S2) → [06_forge_math_runtime.md](06_forge_math_runtime.md)
The numeric runtime. A single `Backend` trait (gemm, cosine, dot, l2, normalize, topk) implemented twice — CPU SIMD over `wide` f32x8/f32x16 and a CUDA `sm_120` backend — engineered for bit-near parity (deterministic nvcc flags, matched reduction order, 1e-3 rel / 1e-6 abs golden tolerances). f32 GEMM uses cuBLAS; distance/topk use embedded PTX. Quantization: TurboQuant (7-bit/base-5) with Hadamard+Rademacher rotation and QJL dot residuals; 1-bit binary codec with Hamming prefilter; MXFP4 (32-value/16-byte blocks, E8M0 scale) / MXFP8 (E4M3) gated by an Assay cosine≥0.99 preservation check. Adds grouped/ragged GEMM and a persisted JSON autotune cache with epsilon-greedy/Thompson exploration and a >2%-margin A/B promotion gate. Fail-closed via `ForgeError`.

### 3.4 Lens registry — `calyx-registry` (S3) → [07_registry_lenses.md](07_registry_lenses.md)
The embedder/lens registry. Defines the `Lens` trait and seven deterministic runtimes (algorithmic, TEI HTTP, Candle local BERT, ONNX, static-lookup mmap, external-command subprocess, multimodal adapter). Every lens is frozen behind a content-addressed `FrozenLensContract`/`LensId` (length-delimited BLAKE3 over name, weight hash, corpus hash, shape fingerprint). The `Registry` fails closed, validating shape, modality, and norm policy per measurement. Provides hot-swap, priority backfill, CPU/GPU placement budgeting, ingest microbatch admission with circuit breakers, capability cards + signal/correlation admit-park-retire gate, eight default domain panels, three algorithmic temporal lenses, matryoshka compression, and JSON vault persistence.

### 3.5 Sextant search & navigation — `calyx-sextant` (S4) → [08_sextant_search.md](08_sextant_search.md)
Search and navigation. A `SextantIndex` trait with five per-slot implementations: in-RAM HNSW (M=32, deterministic blake3-seeded levels), server-only DiskANN/Vamana on-disk graph (4 KiB blocks, two-pass build, alpha-pruned, raw-vector rescore), BM25 inverted index (k1=1.2, b=0.75), MaxSim multi-vector, and dual directional. Per-slot results fuse via SingleLens, RRF (`weight/(rank+60)`), WeightedRRF (14 named slot-weight profiles), or two-stage Pipeline. The `SearchEngine` runs admission control, freshness enforcement, fusion, predicate filters, an optional HTTP reranker, and an in-region guard, returning provenanced `Hit`s. Navigation adds k-NN, lens consensus, graph traversal, and deterministic HDBSCAN* skill discovery. Temporal/causal scoring is additive and never dominant (the AP-60 rule).

### 3.6 Loom DDA & Assay signal bits — `calyx-loom`, `calyx-assay` (S5) → [09_loom_assay_dda.md](09_loom_assay_dda.md)
**Loom** implements Dimensional Derivative Amplification: derives four cross-term kinds (agreement/cosine, delta, interaction/Hadamard, concat) from slot-vector pairs, materialized eagerly or lazily via an LRU policy (interaction stored only when pair-gain ≥ 0.05 bits), weaves agreement scalars into an in-memory agreement graph, persists xterms to Aster, and provides honest abundance reports, blind-spot alerts, a bounded reactive trigger engine, and recurrence/lead-lag series math. **Assay** measures signal bits: KSG, histogram-NMI, and logistic-probe mutual-information estimators with deterministic bootstrap CIs; enforces the lens differentiation contract (≥0.05 bits, ≤0.6 correlation); computes effective rank (`n_eff`), panel sufficiency with deficit routing, per-sensor attribution, cache provenance, plus advanced estimators (Bayesian, MMD, transfer entropy, total correlation, Lomb-Scargle, hazard/CUSUM).

### 3.7 Graph primitives & Lodestar kernel — `calyx-paths`, `calyx-mincut`, `calyx-lodestar` (S6) → [10_graph_kernel.md](10_graph_kernel.md)
**paths** defines the CSR-style directed `AssocGraph`, bidirectional-BFS `reach`, weight-product `reach_scored`, and the single `0.9^hop` attenuation constant. **mincut** adds graph construction, recursive Tarjan SCC + condensation, weighted Brandes betweenness, an LP scaffold (no solver), and a Lanczos+cyclic-Jacobi spectral module. **lodestar** discovers grounding kernels: degree/betweenness/groundedness scoring (0.40/0.40/0.20), heuristic LP rounding, DFVS approximation (exact ≤20 nodes, tournament-2-approx, greedy+local-search), groundedness gaps with anchored/provisional tagging, HNSW kernel index/answer, deterministic recall@k gating (0.95), scope materialization/LRU cache, temporal kernels, label propagation, hierarchical/incremental kernels, Loom/Aster bridges, and ledger-provenanced summarization.

### 3.8 Ledger provenance — `calyx-ledger` (S7) → [11_ledger_provenance.md](11_ledger_provenance.md)
Append-only, hash-chained provenance column family. Each `LedgerEntry` (seq, prev_hash, kind, subject, payload, actor, ts, entry_hash) carries a BLAKE3 `entry_hash` over length-framed fields, chained via `prev_hash` for tamper-evidence. Ten `EntryKind`s (Ingest, Measure, Assay, Kernel, Guard, Answer, Anneal, Migrate, Admin, Erase) have stable wire codes. `LedgerAppender` is the single write path, advancing only through the durable group-commit staged path; direct commits fail closed. A redaction policy strips secrets from payloads. Periodic Merkle checkpoints (domain-separated BLAKE3 tree, optional ed25519 signatures) are stored as Admin rows. `verify_chain` classifies ranges Intact/Broken/Corrupt; quarantined seqs are rejected from audit/provenance/answer-trace queries. Reproduce re-measures frozen-lens slots and re-runs fusion within tolerance.

### 3.9 Ward guard — `calyx-ward` (S8) → [12_ward_guard.md](12_ward_guard.md)
The fail-closed "TCT guard". Each required slot is scored independently against trusted reference vectors — no averaged/flattened aggregate gate. A `GuardProfile` carries per-slot tau, required slots, and an AllRequired or KofN{k} policy; `guard()` emits a structured `GuardVerdict` with full per-slot decomposition, wrapping non-pass as `CALYX_GUARD_OOD`. Per-slot tau is set by conformal-quantile calibration targeting a FAR ceiling (Identity 0.01 / Stylistic 0.05 / Content 0.03) with a binomial confidence bound; high-stakes calls refuse uncalibrated profiles. Failed verdicts route to novelty (NewRegion/Quarantine/RejectClosed) and recurrence/surprise classification. A `DriftMonitor` tracks rolling rejection rates. Identity profiles guard speaker (WavLM-512) and style (RoBERTa-768) lenses. Calibration and verdicts are appended to the Ledger.

### 3.10 Anneal self-improvement — `calyx-anneal` (S10) → [13_anneal_selfheal.md](13_anneal_selfheal.md)
Reversible self-optimization. Every mutation runs through a substrate that gates candidates on five hysteretic tripwires (recall@k, guard FAR/FRR, search-p99, ingest-p95) and held-out shadow replay, recording a promote/revert verdict to an MVCC rollback store and a hash-only Anneal ledger under a background CPU/VRAM budget. It computes the **J intelligence objective** (8 grounded positive terms minus redundancy, ungrounded, and Goodhart penalties) with generated-signal exclusion, an intelligence gradient queue, growth curve, and reports. Self-heal detects faults and drives a degrade health state machine (rebuild, tau recalibration, lens park/unpark, base-shard restore). Learning closes mistakes via a surprise-prioritized replay buffer, online heads with Fisher-regularized SGD and regression rollback, and sleep passes. Autotune uses bandits, A/B trials, a soak harness, and per-scope tuners; propose synthesizes lenses/operators from deficits.

### 3.11 Oracle intelligence — `calyx-oracle` (S11) → [14_oracle_intelligence.md](14_oracle_intelligence.md)
Consequence prediction / super-intelligence over recurrence-grounded constellations. `oracle_predict` mines domain/action-matching recurrence series, computes a count-based posterior with confidence `support*separation*sample_support`, floored by self-consistency and DPI ceilings, then writes ledger provenance. `butterfly` builds hop-attenuated (0.7) consequence trees to depth 4; `reverse_query` walks causes backward (depth 3) for epistemic symmetry. `complete` fills free slots via softmax energy descent toward Ward trusted regions. An Assay-backed honesty gate refuses prediction when `panel_bits < anchor_entropy_bits`. A six-tier predicate (OracleClean, PanelSufficient, KernelExists, Calibrated, GoodhartDefended, MistakeClosed) gates super-intelligence. `time_prediction` forecasts next-occurrence timestamps from median cadence.

### 3.12 CLI — `calyx-cli` (S15+) → [15_cli_reference.md](15_cli_reference.md)
The `calyx` binary, invoked `calyx <command> [subcommand] [flags]`. No clap; `dispatch.rs` manually pattern-matches the raw arg vector with order-significant positional guards, while module groups have their own `--key value` parsers. Major groups: `readback` (read-only/FSV inspection, oracle/PH42/temporal topics), `migrate` (SQLite→vault), `leapable` (shadow-cutover dual-write/read-flip/remove-shadow), `anneal`, `navigate`, `sextant`/`media`/`lodestar` validation harnesses, `lens`/`panel`, `summarize`, ledger/merkle/provenance verifiers, `healthcheck`, and storage/FSV/crash/resource drills. Most commands emit JSON to stdout; FSV commands write Source-of-Truth files and BLAKE3-digest them.

### 3.13 MCP server & daemon — `calyx-mcp`, `calyxd` (S16/S19) → [16_mcp_and_daemon.md](16_mcp_and_daemon.md)
**calyx-mcp** is a JSON-RPC 2.0 MCP server (library + stdio binary) implementing `initialize`/`tools/list`/`tools/call` with per-tool panic isolation and `CalyxError`→`-32000` mapping; it ships with **zero production tools registered** (deferred), exposing only the registry/dispatch machinery. **calyxd** is the daemon: a loopback-only HTTP `/metrics` Prometheus exporter, a periodic ledger chain-verify loop, a fatal CUDA/NVML VRAM startup preflight (no CPU fallback), a fail-closed `CalyxConfig` TOML loader, a daemon-readiness healthcheck, and a byte-level `verify_restore` vault read-back. It also provides a loopback length-prefixed MCP-over-socket transport reusing `calyx-mcp` dispatch (present but not yet wired in `main`). Loopback-only binding is enforced in three places.

---

## 4. Public surface summary

| Domain | Surface | Where | Notes |
|---|---|---|---|
| Operator CLI | `calyx <cmd>` command groups | `calyx-cli` | readback, migrate, leapable, anneal, navigate, sextant/media/lodestar, lens/panel, summarize, ledger/merkle/provenance, healthcheck, FSV/crash/resource drills — see [15_cli_reference.md](15_cli_reference.md) |
| Daemon metrics | HTTP `GET /metrics` (loopback) | `calyxd` | Prometheus families: chain-verify, ingest/search, guard, hazard, ZFS, VRAM |
| MCP | JSON-RPC `initialize`/`tools/list`/`tools/call` | `calyx-mcp` | 0 production tools registered as of this branch |
| Library APIs | `Lens`, `Index`/`SextantIndex`, `VaultStore`, `Estimator`, `Backend` traits + per-crate engines | `calyx-core` + subsystem crates | consumed in-process by binaries |

---

## 5. Data storage & tiers

Calyx persists to a **vault** directory (path from config `vault_path`, with `CALYX_HOME` interpolation — see [03_configuration.md](03_configuration.md)). Storage is managed by Aster ([05_aster_storage.md](05_aster_storage.md)).

| Tier | Artifacts | Regenerability |
|---|---|---|
| Sacred (durable, source of truth) | WAL segments (`CXW1`), SSTables (`CXS1`), `CURRENT`/`MANIFEST`, ledger CF (hash-chained), Merkle checkpoints | Must survive crash; recovery replays WAL after manifest load |
| Derived / regenerable | Secondary indexes (btree, BM25), HNSW/DiskANN vector indexes, Loom cross-terms, kernel/scope caches, autotune cache JSON | Rebuildable from base column families |
| Ephemeral | Memtables, reader-lease snapshots, in-RAM agreement graph, LRU caches | In-memory only, lost on restart |

> The Sextant search engine and HNSW indexes are **in-memory and not persisted** as engine state (vault→engine hydration is spec-driven); see the per-crate docs and the project memory note on this behavior.

---

## 6. Error model

`calyx-core` defines the closed `CALYX_*` error catalog (`CalyxError`/`CalyxErrorCode`) — a stable, numbered set of ~38 PRD-18 codes plus module-local codes for temporal, security, consent, cold-start, allocation, and cache concerns. Per-subsystem typed errors (`ForgeError`, Aster errors, `GuardVerdict`/`CALYX_GUARD_OOD`/`CALYX_GUARD_PROVISIONAL`, ledger verify states) compose with or map onto these. The pervasive discipline is **fail-closed**: invalid input, missing contracts, uncalibrated guards, VRAM preflight failure, and unknown config keys all abort with a typed error rather than degrading silently. Full catalog: [04_core_foundation.md](04_core_foundation.md).

---

## 7. Cross-cutting invariants (axioms)

Derived from enforcement code across crates (not aspirational comments):

| Invariant | Enforced by | Doc |
|---|---|---|
| Lenses are frozen & content-addressed | `FrozenLensContract`/`LensId` validation in Registry | 07 |
| No-average / no-flatten guard | per-slot scoring in Ward `guard()` | 12 |
| Bounded resources (A26) | `Arena`/`SlabPool`/`LruTtlCache` in core; VRAM budgeter | 04, 13 |
| Fail-closed everywhere | typed errors; `deny_unknown_fields` config; VRAM preflight | 03, 05, 12, 16 |
| Provenance for every primitive | ledger appends from measure/kernel/guard/answer/anneal | 11 |
| Temporal/causal never dominant (AP-60) | additive post-retrieval scoring in Sextant | 08 |
| CPU/GPU bit-near parity | golden-tolerance tests in Forge | 06 |

---

## 8. Document index

| Doc | Subject |
|---|---|
| [01_system_overview.md](01_system_overview.md) | This document |
| [02_source_code_map.md](02_source_code_map.md) | File tree, module trees, dependency graph, entry traces, build config |
| [03_configuration.md](03_configuration.md) | TOML config, env vars, toolchain, validation/precedence |
| [04_core_foundation.md](04_core_foundation.md) | IDs, error catalog, enums, data model, engine traits, Clock |
| [05_aster_storage.md](05_aster_storage.md) | WAL/group-commit, SSTable, column families, MVCC, recovery, compaction, indexes |
| [06_forge_math_runtime.md](06_forge_math_runtime.md) | CPU/CUDA backends, quantization, GEMM, autotune |
| [07_registry_lenses.md](07_registry_lenses.md) | Lens trait/runtimes, LensId/contract, panels, hot-swap/backfill |
| [08_sextant_search.md](08_sextant_search.md) | HNSW/DiskANN/BM25 indexes, fusion, planner, navigation |
| [09_loom_assay_dda.md](09_loom_assay_dda.md) | DDA cross-terms, agreement graph, MI/NMI estimators, sufficiency |
| [10_graph_kernel.md](10_graph_kernel.md) | Graph primitives, SCC/betweenness, kernel discovery, scopes |
| [11_ledger_provenance.md](11_ledger_provenance.md) | Hash-chain, EntryKinds, Merkle checkpoints, verify/quarantine |
| [12_ward_guard.md](12_ward_guard.md) | Guard profile/verdict, tau calibration, novelty/drift, identity |
| [13_anneal_selfheal.md](13_anneal_selfheal.md) | Tripwires/shadow rollback, J-objective, self-heal, autotune |
| [14_oracle_intelligence.md](14_oracle_intelligence.md) | Consequence prediction, reverse query, completion, honesty gate |
| [15_cli_reference.md](15_cli_reference.md) | `calyx` commands and flags |
| [16_mcp_and_daemon.md](16_mcp_and_daemon.md) | MCP server + calyxd daemon |
| [18_test_suite.md](18_test_suite.md) | Test inventory, FSV discipline, how to run |
| [19_verification_report.md](19_verification_report.md) | Codebase metrics snapshot |
