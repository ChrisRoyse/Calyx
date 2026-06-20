# 01 — System Overview

**Source files covered (high-level entry points & manifests):**
- `README.md`, `Cargo.toml`, `rust-toolchain.toml`, `LICENSE`
- `crates/calyx-core/src/lib.rs` (types, error catalog, traits)
- `crates/calyx-cli/src/main.rs`, `crates/calyxd/src/main.rs` + `src/lib.rs`, `crates/calyx-mcp/src/main.rs` + `src/lib.rs`
- `crates/calyx-hazard-soak/src/main.rs`
- All 19 crate `src/lib.rs` module headers
- Detailed per-subsystem behavior is documented in docs 04–20; this doc is the map.

> This document describes **what the code is**, traced to source. Where the code is a
> stub or diverges from the design plans in `docs/dbprdplans/`, that is noted here and
> in the per-subsystem docs. Calyx is **pre-1.0** software; the README itself flags the
> on-disk format and interfaces as unstable.

---

## 1. What the system is

Calyx is an **embedded, association-native database engine** written in Rust
(workspace, `edition = 2024`, pinned toolchain `1.95.0`, license BSL 1.1). Its native
record is not a row or a single vector but a **constellation**: one input measured
through many independent, frozen embedders ("lenses"), each producing its own typed
slot-vector that is kept separate (never flattened). On top of that record the engine
bakes in: multi-signal search and fusion, derivation of the associations *between*
slots, information-theoretic measurement of which lenses actually add signal, discovery
of the minimal "grounding kernel" that explains a corpus, a fail-closed guard against
out-of-distribution answers, a hash-chained provenance ledger, reversible
self-optimization, and grounded consequence prediction.

It is **not a service mesh.** It is a stack of focused Rust crates compiled together,
with three thin entry points on top: a CLI (`calyx`), a daemon (`calyxd`), and an MCP
server (`calyx-mcp`). It builds **CPU-only by default**; a CUDA GPU backend is an opt-in
Cargo feature (`--features cuda`).

### 1.1 The four verbs (organizing model)

The README and `docs/dbprdplans/DOCTRINE.md` describe the engine as a "calculus of
association" with four verbs, each mapped to subsystems implemented in the workspace:

| Verb | Meaning | Subsystem / crate |
|---|---|---|
| **Measure** | Assemble a constellation by viewing one input through a panel of lenses | `calyx-registry`, `calyx-aster` |
| **Count** | Derive the associations between slots (agreement, delta, interaction) | `calyx-loom` |
| **Differentiate** | Quantify the unique information (bits) each lens adds about real outcomes | `calyx-assay` |
| **Compose** | Find the kernel, guard generation, answer with provenance | `calyx-lodestar`, `calyx-ward`, `calyx-ledger` |

### 1.2 Three trust principles (enforced in code)

- **Grounding is mandatory** — claims are measured against anchored outcomes; ungrounded
  results are tagged *provisional* (see `CxFlags`/`AnchorKind` in [05_core.md](05_core.md),
  and the Oracle honesty gate in [16_oracle_prediction.md](16_oracle_prediction.md)).
- **No-flatten** — slots stay typed and separate end-to-end (`SlotVector` is never
  concatenated into one opaque blob; Ward scores each slot independently).
- **Fail closed** — unknown lens, shape mismatch, uncalibrated guard, or missing data
  returns a structured `CalyxError`, never a silent wrong answer. The closed error
  catalog (`CalyxErrorCode`, 38 codes) is the spine of this principle.

---

## 2. Architecture

### 2.1 Layered crate stack

```
Entry points :  calyx (CLI)   ·   calyxd (daemon + /metrics)   ·   calyx-mcp (agent MCP)
                       │                    │                            │
Intelligence :   Oracle  ·  Anneal  ·  Lodestar
                       │
Engine       :   Sextant · Loom · Assay · Ward · Registry
                       │
Foundation   :   Aster (LSM storage) · Forge (CPU/GPU math) · Ledger (provenance)
                       │
Core         :   calyx-core (ids, error catalog, data model, engine traits)
                 calyx-paths / calyx-mincut (graph primitives)
```

`Surfaces → Intelligence → Engine → Foundation` is the README's flow. The actual
internal dependency graph (103 edges across 19 crates) is in
[02_source_code_map.md](02_source_code_map.md); `calyx-core` is the zero-internal-dep
foundation depended on by all 18 other crates.

### 2.2 Process / surface map

| Surface | Binary | Crate | Transport / port | Purpose |
|---|---|---|---|---|
| CLI | `calyx` | `calyx-cli` | process invocation; JSON error envelope on stderr | Drive every engine operation from the shell; ~20 structured subcommands + legacy/domain command families |
| Daemon | `calyxd` | `calyxd` | HTTP `GET /metrics` (Prometheus 0.0.4) on loopback, default `127.0.0.1:7700` | Long-running service: startup CUDA/VRAM probes, health probe, Prometheus metrics, `verify-restore` tool. A loopback MCP-over-socket transport (length-framed) exists in the library but is **not wired into the live daemon** (serves only `/metrics`) |
| MCP server | `calyx-mcp` | `calyx-mcp` | **stdio only**, line-delimited JSON-RPC 2.0, MCP protocol `2024-11-05` | Agent-facing tool surface: 30 `calyx.*` tools |
| Soak/hazard harness | `calyx-hazard-soak` | `calyx-hazard-soak` | CLI binary | 25 fault-injection probes (H1–H25) + a long integrated soak; emits JSON + Prometheus evidence |

### 2.3 Technology stack

| Layer | Technology | Version / constraint (from `Cargo.toml`) |
|---|---|---|
| Language / toolchain | Rust, edition 2024 | toolchain `1.95.0` (pinned in `rust-toolchain.toml`), `rust-version = 1.95` |
| Hashing | `blake3` | `1` — content addressing, ledger chain, Bloom filters |
| Crypto (signing) | `ed25519-dalek` | `2` — ledger checkpoint signatures |
| Crypto (at-rest) | `aes-gcm`, `hkdf`, `sha2`, `zeroize` | `0.10` / `0.12` / `0.10` / `1` |
| Serialization | `serde`, `serde_json`, `bincode_reloaded` (3.1.6), `ciborium` | `1` / `1` / `3.1.6` / `0.2` |
| Storage | `memmap2` `0.9`, `crc32fast` `1`, `zstd` `0.13`, `filetime` `0.2`, `nix` `0.30` (fs) | mmap tables, CRC, posting-list compression |
| CPU math | `wide` `1` (SIMD), `rayon` `1` (parallelism) | Forge CPU backend |
| GPU math (opt-in) | `cudarc` `0.19.7`, `nvml-wrapper` `0.10` | CUDA 13.2, `sm_120`; feature `cuda`, default off |
| SQLite (import-only) | `rusqlite` `0.40.1` (bundled) | only the `calyx migrate` CLI reads external SQLite |
| IDs / time | `ulid` `1`, `uuid` `1` (dep present; core uses ULID), Unix-ms `Ts` | |
| Randomness | `rand` `0.8`, `rand_chacha` `0.3` | deterministic seeded RNG |
| Observability | `tracing` `0.1`, `prometheus` `0.14` | |
| Errors | `thiserror` `2` | |
| Testing | `proptest` `1`, `criterion` `0.5`, `cargo-fuzz` (6 targets) | see [21_test_suite.md](21_test_suite.md) |

### 2.4 Build profiles

The workspace `Cargo.toml` tunes debug info for a high-churn multi-agent build cadence:
`[profile.dev] debug = "line-tables-only"` (first-party crates keep function names +
file:line for backtraces) and `[profile.dev.package."*"] debug = false` (dependencies get
no debuginfo). This was a deliberate response to dev/test executables reaching ~280 MB
and the shared `target/` dir ballooning to ~190 GB. See
[03_configuration.md](03_configuration.md) and `docs/implementation/02_BUILD_PERFORMANCE.md`.

---

## 3. Public surfaces (APIs / tools / commands)

### 3.1 MCP tools — 30 total (`calyx.*`), grouped by domain

Full parameter/return/error tables in [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md).
The tool set is pinned by `crates/calyx-mcp/tests/stdio.rs::EXPECTED_TOOLS`.

| Domain | Tools |
|---|---|
| Vault & panel (6) | `create_vault`, `add_lens`, `retire_lens`, `park_lens`, `list_panel`, `profile_lens` |
| Ingest & measure (3) | `ingest`, `anchor`, `measure` |
| Search & navigate (10) | `search`, `kernel_answer`, `neighbors`, `agree`, `disagree`, `define`, `guard_generate`, `traverse`, `skills`, `search_skill` |
| Intelligence (6) | `abundance`, `bits`, `kernel`, `guard.calibrate`, `guard.check`, `propose_lens` |
| Provenance & ops (5) | `provenance`, `answer_trace`, `verify_chain`, `reproduce`, `anneal.status` |

### 3.2 CLI commands

`calyx-cli` uses **hand-rolled arg parsing** (no clap), dispatched in layers. ~20
structured subcommands (`create-vault`, `add-lens`, `retire-lens`, `park-lens`,
`list-panel`, `profile-lens`, `ingest`, `anchor`, `measure`, `search`, `kernel-answer`,
`bits`, `kernel`, `guard`, `abundance`, `propose-lens`, `provenance`, `verify-chain`,
`reproduce`, `anneal-status`) plus legacy commands (`readback`, `merkle-root`, FSV/ops
families) and domain command groups (`navigate`, `sextant`, `media`, `lodestar`, `lens`,
`panel`, `intelligence`, `summarize`, `migrate`, `anneal`, `leapable`). Every failure
emits a JSON `{code,message,remediation}` envelope on stderr with exit code 2. Full
tables in [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md).

### 3.3 Daemon endpoint

`calyxd` serves exactly one HTTP route, `GET /metrics` (Prometheus text format 0.0.4) on
loopback. Metric families and startup probes are documented in
[20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md).

---

## 4. Entry points & invocation

| Binary | Crate | How invoked |
|---|---|---|
| `calyx` | `calyx-cli` | `cargo run -p calyx-cli -- <command>` or the built `calyx` binary |
| `calyxd` | `calyxd` | `calyxd --config <path.toml>` (validate-only without a live serve); `calyxd verify-restore ...` (PH67) |
| `calyx-mcp` | `calyx-mcp` | launched by an MCP client over stdio |
| `calyx-hazard-soak` | `calyx-hazard-soak` | `calyx-hazard-soak --hazards <range>` / `--all-hazards` / soak mode |

Build/test: `cargo build --release --workspace` (CPU), `cargo test --workspace`,
optional `cargo build --release --workspace --features cuda`.

---

## 5. Runtime directory layout & data location

Data lives under a **vault directory** (root resolved via `vault_path` /
`$CALYX_HOME`). The full on-disk tree and per-artifact format table are in
[04_storage_and_schema.md](04_storage_and_schema.md). Summary of what gets created:

| Path (under vault) | Contents | Owner subsystem |
|---|---|---|
| WAL segments | write-ahead log records (`CXW1`) | Aster |
| SSTables | sorted immutable tables (`CXS1` v2) + Bloom filters | Aster |
| `manifest-{seq:020}.json` | versioned manifest (`ManifestVersion{1,0}`), immutable refs verified by BLAKE3 | Aster |
| Arrow column chunks (`CXA1` v1) | column-major f32 slot data | Aster |
| `idx/kernel/<id>/index.json` + `kernel.json` | grounding kernel index/artifact (`FORMAT_VERSION = 1`) | Lodestar |
| `idx/*` | ANN/DiskANN/SPANN indexes (`CLXDA001` graph pages) | Sextant |
| `.anneal/tripwire.toml`, `.anneal/budget.toml` | persisted self-optimization tripwires & budgets | Anneal |

### 5.1 Storage tier classification

From [04_storage_and_schema.md](04_storage_and_schema.md):

| Tier | Examples | Rule |
|---|---|---|
| **Sacred** (never auto-deleted) | WAL, Ledger CF, Base/slot CFs, manifest + immutable refs, Anchors | source of truth |
| **Regenerable** | Kernel/Guard CFs + `idx/kernel/*.json`, B-tree/inverted indexes, ANN `idx/*`, Assay cache, XTerm CFs, compacted SSTs, raw slot sidecars | rebuildable from sacred data |
| **Ephemeral** | memtables, LRU/TTL caches, arenas, reader-lease registry, VRAM blocks | in-memory, lost on restart |

---

## 6. Error hierarchy

The closed error catalog lives in `calyx-core` ([05_core.md](05_core.md)). The base
wire type is **`CalyxError`** (`{ code: &'static str, message: String, remediation:
&'static str }`, `Serialize`-only). The canonical catalog is the `CalyxErrorCode` enum
plus the `CALYX_ERROR_CODES` slice — **38 codes** in PRD-18 order, pinned by the test
`catalog_matches_prd_18_exactly`. `type Result<T> = Result<T, CalyxError>`.

Individual subsystems define **module-local `&'static str` codes** that are deliberately
NOT in the closed 38-code catalog (they namespace per crate). Roots observed across the
workspace:

| Subsystem | Error-code roots (prefix family) |
|---|---|
| Core (catalog) | the 38 `CALYX_*` codes; plus module-local `CALYX_RECORD_SCHEMA_VIOLATION`, `CALYX_ALLOC_CAP_EXCEEDED`, `CALYX_AUTHN_REQUIRED`, `CALYX_CONSENT_VIOLATION`, `CALYX_PROVISIONAL_VAULT`, `CALYX_TEMPORAL_*` |
| Aster | `CALYX_ASTER_TORN_WAL`, `CALYX_ASTER_CORRUPT_SHARD`, `CALYX_ASTER_BASE_CORRUPT`, `CALYX_FORMAT_VERSION_UNSUPPORTED` |
| Forge | `CALYX_FORGE_*`, `CALYX_QUANT_INTELLIGENCE_LOSS`, `CALYX_VRAM_BUDGET_EXCEEDED` |
| Registry | `CALYX_LENS_{FROZEN_VIOLATION,DIM_MISMATCH,NUMERICAL_INVARIANT,UNREACHABLE}`, `CALYX_REGISTRY_DUPLICATE`, `CALYX_PANEL_LENS_MISSING`, `CALYX_{RAM,VRAM}_BUDGET_EXCEEDED`, `CALYX_STALE_DERIVED` |
| Loom | `CALYX_LOOM_*`, `CALYX_REACTIVE_*` |
| Assay | `CALYX_ASSAY_{LOW_SIGNAL,REDUNDANT,INSUFFICIENT_SAMPLES}`, `CALYX_TC/TE_*`, `CALYX_BAYES_INVALID_INTERVAL` |
| Lodestar | `CALYX_KERNEL_*`, `CALYX_RECALL_*`, `CALYX_DFVS_*`, `CALYX_PROP_*`, `CALYX_SCOPE_*`, `CALYX_LODESTAR_*` |
| Ward | `CALYX_GUARD_*`, `CALYX_WARD_*` |
| Ledger | `CALYX_LEDGER_*`, `CALYX_REPRODUCE_*` |
| Anneal | `CALYX_ANNEAL_{UNKNOWN_CHANGE_ID,CHANGE_COMMITTED,INVALID_ROLLBACK_STATE}` |
| Oracle | `CALYX_ORACLE_*` |
| Mincut / Paths | `CALYX_SCC_*`, `CALYX_BETWEENNESS_*`, `CALYX_MINCUT_*`, `CALYX_GRAPH_*`, `CALYX_PATHS_*` |
| MCP | JSON-RPC codes + `CALYX_MCP_JSONRPC_INVALID`, `CALYX_MCP_TOOL_DUPLICATE` |
| CLI | `CALYX_CLI_USAGE_ERROR`, `CALYX_CLI_IO_ERROR` |
| Daemon | `CALYX_DAEMON_{BIND_FAILED,CONFIG_INVALID,HEALTH_FAIL,FRAME_INVALID,CONN_PANIC}` |

---

## 7. Subsystem summaries

Each one-paragraph summary points to its deep-dive doc. All behavioral claims are traced
there.

### 7.1 calyx-core — types, errors, traits ([05_core.md](05_core.md))
Dependency-free foundation. Provides content-addressed IDs (`VaultId(Ulid)`,
`LensId([u8;16])`, `CxId([u8;16])`, `SlotId(u16)`) where `LensId`/`CxId` are the first
16 bytes of BLAKE3 over length-delimited ordered parts; the closed `CalyxErrorCode`
catalog (38 codes); the data model (`Constellation`, `Slot`, `Panel`, `SlotVector` =
Dense/Sparse/Multi/Absent, `Anchor`, `Signal`, etc.); and the object-safe engine traits
`Lens`, `Index`, `VaultStore`, `Estimator`, plus `Input` and `Clock`. Serialization is
serde/JSON in this crate.

### 7.2 calyx-aster — LSM storage engine ([06_aster_storage_engine.md](06_aster_storage_engine.md))
The "schema" layer and largest crate. A write-ahead log (`CXW1`, 20-byte header, CRC32,
group-commit with a 2 ms window) feeds bounded memtables that freeze and flush to
SSTables (`CXS1` v2, sorted index, trailing BLAKE3 Bloom). MVCC snapshots via
`VersionedCfStore`/`Snapshot`/`ReaderLease`; crash recovery via a JSON manifest
(`ManifestVersion{1,0}`, atomic rename, BLAKE3-verified immutable refs); 33 column
families routed by `CfRouter`; compaction + hot/cold tiering. Unknown manifest major →
`CALYX_FORMAT_VERSION_UNSUPPORTED`. The crate header still calls itself a "skeleton" and
compaction cadence is fixed (`FIXME(PH46)`).

### 7.3 calyx-forge — math runtime ([07_forge_math_runtime.md](07_forge_math_runtime.md))
One numeric backend implemented twice: `CpuBackend` (SIMD via `wide`) and `CudaBackend`
(`cudarc`), both implementing the `Backend` trait (7 ops: gemm, cosine, dot, l2,
normalize, topk, device_info). No runtime auto-dispatch — callers pick a `dyn Backend`;
an `AutotuneCache` records but does not auto-apply winning configs. 7 quantization levels
(`QuantLevel`): F32, Bits8 (no codec ships), Bits8Fp (MXFP8), Bits4Fp (MXFP4), Bits3p5,
Bits2p5 (TurboQuant + QJL residual), Bits1 (sign bits). The `cuda` feature (off by
default) compiles 3 `.cu` kernels to PTX+cubin with `--fmad=false` for bit-near CPU/GPU
parity; **fail-closed, no silent CPU fallback**. Default VRAM soft cap 12 GiB.

### 7.4 calyx-registry — lens registry ([08_registry_lenses.md](08_registry_lenses.md))
Holds frozen, content-addressed embedders implementing the `Lens` trait. Runtimes:
`AlgorithmicLens`, `TeiHttpLens` (HF TEI HTTP), `CandleLens` (BERT via candle),
`OnnxLens` (fastembed/ort), `StaticLookupLens` (mmap model2vec, `CXLKUP1`),
`MultimodalAdapterLens` (hash-derived projection), `ExternalCmdLens`, temporal lenses
(`E2RecencyLens`, `E3PeriodicLens`, `E4PositionalLens`), and `CommissionedLens`.
`LensId = content_address([name, weights_sha256, corpus_hash, output_shape_fingerprint])`.
Registration always fails closed (the frozen contract is mandatory). Hot-swap via
`SwapController` + durable `BackfillScheduler`. Multimodal/commissioned lenses currently
emit deterministic hash projections, not learned embeddings.

### 7.5 calyx-sextant — search & navigation ([09_sextant_search.md](09_sextant_search.md))
In-RAM HNSW (M=32, deterministic BLAKE3 levels), on-disk DiskANN/Vamana (`CLXDA001`,
4 KiB pages, RobustPrune, beam search), DualDiskANN, SPANN (RAM centroid HNSW + zstd-3
on-disk posting lists, k-means++), and a kernel-first 3-hop funnel search. BM25 inverted
index (k1=1.2, b=0.75, Lucene-style idf). Multi-vector MaxSim (late interaction). Fusion
is Reciprocal Rank Fusion with `RRF_K = 60.0`; a deterministic keyword classifier picks
the strategy and a planner enforces caps (k≤100, ef≤512, slots≤16, 20M cost cap).
Temporal post-retrieval boosts (50/35/15) and causal gating.

### 7.6 calyx-loom — associations ([10_loom_associations.md](10_loom_associations.md))
Computes cross-terms between slot vectors: **agreement** = cosine (scalar), **delta** =
elementwise `aᵢ−bᵢ`, **interaction** = elementwise Hadamard `aᵢ·bᵢ`, **concat** = `[a‖b]`.
`LoomStore::agreement_graph()` returns an undirected weighted edge list
(`Vec<AgreementEdge>`) recomputed from an in-memory cross-term map; a 0.05-bit
`PairGainGate` controls materialization. Also lead/lag series analysis (`LLAG1` wire
format). Persists to Aster `XTerm`/`Reactive` CFs.

### 7.7 calyx-assay — signal bits ([11_assay_signal_bits.md](11_assay_signal_bits.md))
Measures mutual information (bits) each lens adds about an anchor using a **KSG estimator
(k-NN, Chebyshev distance, brute-force O(n²))**, nats→bits, with a deterministic ChaCha8
bootstrap CI. Enforces a redundancy contract (`MIN_SIGNAL_BITS = 0.05`,
`MAX_PAIRWISE_CORR = 0.6`, min 50 samples). Panel sufficiency: `panel_bits ≥
anchor_entropy_bits` (no slack). Persists to Aster `Assay` CF.

### 7.8 calyx-lodestar — grounding kernel ([12_lodestar_kernel.md](12_lodestar_kernel.md))
Discovers the minimal record set (the ~1% MFVS) explaining a corpus via a staged
approximate directed-MFVS pipeline: Tarjan SCC → Brandes betweenness → greedy top-fraction
candidate selection (score = `0.40·degree + 0.40·betweenness + 0.20·groundedness`) →
LP-round (heuristic fallback) → `dfvs_approx`. The kernel doubles as an HNSW index and an
answer path (BFS reach with `hop_score = edge_weight·0.9^hop`). Recall gate default 0.95;
kernel artifact `FORMAT_VERSION = 1`. The LP relaxation is not wired to a solver and the
build pipeline does not measure recall (defaults to 0).

### 7.9 calyx-ward — fail-closed guard ([13_ward_guard.md](13_ward_guard.md))
Scores each required slot independently with `dense_cosine` against a per-slot threshold
`tau` (INVARIANT A3 — no flattened/averaged gate). Combination rule: `AllRequired` (AND
of all slots) or `KofN{k}`. Conformal calibration sets `tau` to the smallest candidate
cosine where empirical bad-FAR ≤ target_far AND a binomial CDF bound holds (estimator
`conformal_quantile_v1`, needs ≥50 bad scores). Default per-`SlotKind` FAR: Identity 0.01,
Content 0.03, Stylistic 0.05; cold-start `tau = 0.7`. Verdicts → accept / new-region(learn)
/ quarantine / refuse(OOD). Persists verdicts to the Ledger.

### 7.10 calyx-ledger — provenance ([14_ledger_provenance.md](14_ledger_provenance.md))
Append-only, hash-chained log. Entry hash = BLAKE3 over length-framed
`(seq, prev_hash, kind, subject, payload, actor, ts)`; each entry's `prev_hash` seals the
previous (genesis = zeros). 10 entry kinds (wire codes 0–9). Every
`DEFAULT_CHECKPOINT_INTERVAL = 1000` entries an Admin row carries a Merkle
`CheckpointPayload`, optionally ed25519-signed (message `b"calyx-ledger-root-v1" ‖ …`).
Verification (`VerifyResult` = Intact/Broken/Corrupt) re-walks and re-hashes the chain.
Persists to Aster `Ledger` CF. No key management in-crate.

### 7.11 calyx-anneal — self-optimization ([15_anneal_optimization.md](15_anneal_optimization.md))
Reversible tuning of index params (hnsw_ef=64, hnsw_m=16, diskann_beamwidth=32, …),
Forge tiling/dtype, Loom eager pairs, and storage knobs, plus online heads (≤1024 params)
and Ward τ. Loop: prepare(reserve rollback) → budget → shadow-test on held-out replay →
gate (tripwire + per-metric non-regression) → promote(pointer swap + Ledger) or rollback.
Tripwires (`.anneal/tripwire.toml`): RecallAtK<0.90, GuardFAR>0.01, GuardFRR>0.05,
SearchP99>200ms, IngestP95>500ms, 5% hysteresis. Rollback state machine: Prepared →
Promoted → Reverted/Committed. Audited via 28 `AnnealLedgerAction` variants (tag
`anneal_event_v1`); persisted to Aster `AnnealRollback` CF.

### 7.12 calyx-oracle — consequence prediction ([16_oracle_prediction.md](16_oracle_prediction.md))
Builds a recursive "butterfly" `ConsequenceTree` (DFS, `MAX_DEPTH = 4`, `HOP_ATTENUATION
= 0.7`, prune below `MIN_CONFIDENCE_THRESHOLD = 0.05`, cycle-guarded), a reverse
outcome→cause walk (`MAX_REVERSE_DEPTH = 3`, `grounded_confidence(n)=n/(n+1)`), and an
**honesty gate**: `sufficient = panel_bits ≥ anchor_entropy_bits`; otherwise returns
`OracleError::Insufficient` carrying a `SufficiencyBound` with per-sensor deficits. Ledger
tags `oracle_predict_v1`, etc. The intelligence-objective `J`/growth-curve APIs from the
plans are not implemented here.

### 7.13 calyx-mincut + calyx-paths — graph primitives ([17_graph_mincut_paths.md](17_graph_mincut_paths.md))
`calyx-mincut`: Tarjan SCC + condensation, Brandes betweenness, eigenvector centrality,
Laplacian eigenmaps / GFT, spectral gap. Despite the name, there is **no min-cut/max-flow
or feedback-vertex-set solver** — only an LP scaffold that formulates (no cycle
constraints, no solver), so Lodestar falls back to heuristic DFVS. `calyx-paths`:
`AssocGraph` (CSR adjacency), `reach` (bidirectional BFS), `reach_scored` (weighted
best-first, `score·0.9^hops`).

### 7.14 calyx-hazard-soak + calyx-testkit ([18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md))
`calyx-hazard-soak`: a fault-injection binary (PH59 suite) with 25 hazard probes (write
amplification, memtable stall, tombstone buildup, fsync spikes, VRAM-OOM, clock skew,
secret leakage, DR drill, …) plus a long integrated soak (`DEFAULT_SOAK_OPS =
10_000_000`, seed `0xCA1A_0059`) asserting bounded RSS/VRAM growth. `calyx-testkit`:
deterministic scaffolding — `DEFAULT_TEST_SEED = 0xCA1A_CAFE_D15C_1A11`, `DEFAULT_TEST_TS
= 1_785_500_000`, `seeded_rng`, `fixed_clock`, proptest strategies.

### 7.15 Entry points — calyx-cli, calyxd, calyx-mcp ([19](19_mcp_api_tools_reference.md), [20](20_cli_and_daemon_reference.md))
Thin surfaces over the engine. The CLI hand-parses args and returns JSON error envelopes;
`calyxd` runs startup probes (T02 CUDA preflight, T03 NVML VRAM budget ≤30000 MiB, T04
health, T05 loopback MCP transport) and serves `/metrics`; `calyx-mcp` exposes 30 tools
over stdio JSON-RPC with no auth/consent gate (vault resolution via `CALYX_HOME`).

---

## 8. Versions & invariants snapshot

| Item | Value |
|---|---|
| Workspace version | `0.1.0` (all crates) |
| Rust edition / toolchain | 2024 / `1.95.0` (pinned) |
| License | BSL 1.1 (auto-converts to Apache-2.0 four years post-release) |
| WAL format | `CXW1`, 20-byte header, CRC32, group-commit window 2 ms |
| SSTable format | `CXS1`, VERSION 2 (legacy 1) |
| Arrow column format | `CXA1`, VERSION 1 |
| Manifest format | JSON, `ManifestVersion{major=1, minor=0}` |
| Lodestar kernel artifact | `FORMAT_VERSION = 1` |
| DiskANN graph | `CLXDA001`, 4 KiB pages |
| Column families | 33 static (+ parameterized slot CFs) |
| Core error catalog | 38 codes (`CalyxErrorCode`, pinned by test) |
| MCP tools | 30 (pinned by test) |
| Default daemon bind | `127.0.0.1:7700` (loopback only) |
| Default VRAM soft cap | 12 GiB (Forge); daemon `vram_budget_mib` 1..=30000 |

---

## 9. Gaps / not covered (system-level)

- **Skeleton/stub markers in shipped code**: Aster `lib.rs` self-describes as a
  "skeleton"; compaction cadence is a fixed `FIXME(PH46)`.
- **GPU is opt-in and fail-loud**: default builds are CPU-only; in server mode CUDA
  absence fails loudly rather than falling back.
- **Unwired/partial paths**: the MCP-over-socket transport in `calyxd` is library-complete
  but not served by the live daemon; the LP solver for kernel DFVS is not wired
  (heuristic fallback only); `Bits8` quantization has no codec; multimodal/commissioned
  lenses emit hash projections, not learned vectors; `AnnealHook` is an interim pre-PH48
  shim; MCP `search.fresh` is parsed but unused and `search.guard=in_region` always errors.
- **No code-coverage tooling** exists in the repo (see [21_test_suite.md](21_test_suite.md)).
- **Design-vs-code divergences** (e.g. column-family numbering in the PRD vs implemented
  tags, Loom interaction being Hadamard-only) are captured in the relevant per-subsystem
  docs and in [24_roadmap.md](24_roadmap.md).
- Detailed remaining work and open issues: see [24_roadmap.md](24_roadmap.md). Design
  intent vs implementation: see [23_planning_docs_summary.md](23_planning_docs_summary.md).
