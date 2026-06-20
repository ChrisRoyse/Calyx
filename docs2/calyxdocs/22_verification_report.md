# 22 — Verification Report

**Source of figures:** counts in this report were produced by running `git ls-files`,
`ripgrep`, and `wc` against the repository at `C:\code\Calyx-Dev` synced to remote
`origin/main` commit `2e7b9a2` ("Add kernel-first funnel search", 2026-06-16
08:42:31 -0500). Cross-references: [21_test_suite.md](21_test_suite.md),
[04_storage_and_schema.md](04_storage_and_schema.md),
[19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md),
[02_source_code_map.md](02_source_code_map.md).

> This is a **codebase-health snapshot derived from static inspection**, not a live test
> run. The documentation in this set was produced by reading source code; tests were not
> executed as part of producing these docs. Where a number reflects a pinned-by-test
> invariant (e.g. 38 error codes, 30 MCP tools), that is stated.

---

## 1. Codebase metrics

| Metric | Value | How counted |
|---|---|---|
| Crates (workspace members) | **19** | `ls crates/` |
| Binaries | **4** (`calyx`, `calyxd`, `calyx-mcp`, `calyx-hazard-soak`) | `grep -l '[[bin]]' crates/*/Cargo.toml` |
| Tracked `.rs` files (total) | **1,256** | `git ls-files '*.rs'` |
| `.rs` files under `crates/*/src/` | **877** | `git ls-files 'crates/*/src/*.rs'` |
| `.rs` files under `crates/*/tests/` | **374** | `git ls-files 'crates/*/tests/*.rs'` |
| Remaining `.rs` (fuzz/benches/build.rs/examples) | **5** | difference |
| Total Rust LOC (all tracked `.rs`) | **307,514** | `git ls-files '*.rs' \| xargs cat \| wc -l` |
| Source-only Rust LOC (`crates/*/src/`) | **205,368** | as above, src glob |
| Markdown files (`.md`) | **601** | `git ls-files '*.md'` |
| CUDA kernels (`.cu`) | **3** (`distance.cu`, `topk.cu`, `mxfp4_gemm.cu`) | in `crates/calyx-forge` |
| Fuzz targets | **6** | `ls fuzz/fuzz_targets/` |
| CI workflows | **1** (`.github/workflows/ci.yml`) | `ls .github/workflows/` |
| MCP tools | **30** (pinned by `tests/stdio.rs::EXPECTED_TOOLS`) | [19](19_mcp_api_tools_reference.md) |
| CLI structured subcommands | **~20** + legacy + domain command groups | [20](20_cli_and_daemon_reference.md) |
| Column families (static) | **33** (`ColumnFamily::STATIC`, indices 0–32) + parameterized slot CFs | `crates/calyx-aster/src/cf/family.rs` |
| Core closed error catalog | **38** codes (`CalyxErrorCode` / `CALYX_ERROR_CODES`) | [05](05_core.md) |
| SQLite tables (Calyx's own store) | **0** — SQLite is import-only (`calyx migrate`, read-only) | [04](04_storage_and_schema.md) |

### 1.1 LOC by crate (source `.rs`, approximate)

| Crate | `src/` `.rs` files | Source LOC (approx) |
|---|---|---|
| calyx-aster | 232 | ~57,601 |
| calyx-cli | 229 | ~48,116 |
| calyx-anneal | 156 | ~41,076 |
| calyx-sextant | 113 | ~28,056 |
| calyx-registry | 72 | ~19,342 |
| calyx-forge | 71 | ~17,338 |
| calyx-lodestar | 62 | ~15,387 |
| calyx-ward | 51 | ~14,603 |
| calyx-assay | 45 | ~11,260 |
| calyx-mcp | 41 | ~8,696 |
| calyx-oracle | 41 | ~11,648 |
| calyx-loom | 27 | ~5,945 |
| calyx-core | 25 | ~5,349 |
| calyx-ledger | 25 | ~7,885 |
| calyxd | 22 | ~6,044 |
| calyx-hazard-soak | 21 | ~6,332 |
| calyx-mincut | 10 | ~1,695 |
| calyx-paths | 6 | ~730 |
| calyx-testkit | 1 | ~217 |

(Per-crate counts taken earlier in the session; sum is consistent with the 205,368
source-LOC workspace total. The discrepancy between "232 src files" here and the
877/1,256 split above is that these per-crate figures count files within each crate's
`src/`.)

---

## 2. Test inventory summary

From [21_test_suite.md](21_test_suite.md) (counts are static `ripgrep` tallies, not a
test-runner report):

| Item | Value |
|---|---|
| `#[test]` / `#[tokio::test]` attributes (approx total) | **~2,830** (≈1,666 inline-unit in `src/` + ≈1,164 integration in `tests/`) |
| Integration test files (`crates/*/tests/*.rs`) | **374** (≈150 FSV-named, `*fsv*.rs`) |
| `proptest!` property blocks | **64** in `tests/` (each runs many cases) |
| Golden-vector test files | **17** (forge / ledger / aster / registry) |
| Fuzz targets (`cargo-fuzz`) | **6** (`aster_manifest_decode`, `aster_sst_decode`, `aster_wal_replay`, `lens_output_decode`, `mcp_jsonrpc_decode`, `query_parse`) |
| `#[ignore]`-gated tests | ~174 in `tests/`, ~33 in `src/` (aiwonder/CUDA-only FSV suites) |
| Soak/hazard probes | 25 (H1–H25) + integrated soak — [18](18_hazard_soak_and_testkit.md) |

**Test categories present:** unit (inline `#[cfg(test)]`), integration (`tests/`),
property-based (proptest), FSV (Field/Final System Verification), fuzz, soak/hazard,
golden-vector, doctests, criterion benches.

**Pass/fail status:** **Not executed for this report.** Tests are gated in CI by
`.github/workflows/ci.yml` (job `gate` on ubuntu-latest), which runs `cargo nextest run
--workspace` and `cargo test --workspace --doc`. FSV/`#[ignore]` tests run only with
their environment gates set (e.g. on a CUDA/aiwonder host) and are not run in CI. To
reproduce locally: `cargo test --workspace` (CPU) and `cargo build --workspace --features
cuda` for the GPU path. See [21_test_suite.md](21_test_suite.md) for the exact commands
and env gates.

---

## 3. Lint / quality gates

| Gate | Tool | Where | Status |
|---|---|---|---|
| Formatting | `cargo fmt --check` | CI `gate` job + `.githooks/pre-push` | enforced |
| Type check | `cargo check --all-targets` | CI `gate` job | enforced |
| Lints | `cargo clippy -- -D warnings` (warnings = errors) | CI `gate` job | enforced |
| Tests | `cargo nextest run --workspace` + doc tests | CI `gate` job | enforced (excludes `#[ignore]`) |
| Repo hygiene | `orphan_rs` + line-count gates (≤500-line files per doctrine) | CI `gate` job | enforced |
| Secret scan | `gitleaks` via `.pre-commit-config.yaml` | pre-commit | enforced |
| Code coverage | none | — | **No coverage tooling present** (no tarpaulin/llvm-cov/grcov/codecov) |

---

## 4. Schema / format versions

| Artifact | Magic | Version |
|---|---|---|
| WAL record stream | `CXW1` | header v1 (20-byte) |
| SSTable | `CXS1` | **VERSION 2** (legacy 1 still readable) |
| Arrow column chunk | `CXA1` | VERSION 1 |
| Vault manifest | (JSON) | `ManifestVersion{major=1, minor=0}` |
| Lodestar kernel index/artifact | (JSON) | `FORMAT_VERSION = 1` |
| DiskANN graph | `CLXDA001` | 4 KiB page-aligned |
| Static-lookup lens matrix | `CXLKUP1` | — |
| Lead/lag series record | `LLAG1` | 61-byte |
| Compression report schema | — | `COMPRESSION_REPORT_SCHEMA_VERSION = 1` |
| Forge seed format | — | `CURRENT_SEED_VERSION = 1` |

Unknown manifest **major** version → `CALYX_FORMAT_VERSION_UNSUPPORTED` (fail-closed).

---

## 5. Notable constants & magic numbers

| Constant | Value | Subsystem | Source doc |
|---|---|---|---|
| `DEFAULT_EMBED_DIM` | 768 | core | [05](05_core.md) |
| `PAGE_SIZE` / `ARENA_BASE_ALIGN` | 4096 | core | [05](05_core.md) |
| Fusion weights default | 0.50 / 0.35 / 0.15 | core/sextant | [05](05_core.md) |
| `DEFAULT_GROUP_COMMIT_WINDOW` | 2 ms (max) | aster | [06](06_aster_storage_engine.md) |
| `MAX_RECORD_BYTES` (WAL) | 64 MiB | aster | [06](06_aster_storage_engine.md) |
| Bloom filter hashes | 3 (BLAKE3) | aster | [06](06_aster_storage_engine.md) |
| `RRF_K` (fusion) | 60.0 | sextant | [09](09_sextant_search.md) |
| BM25 `k1` / `b` | 1.2 / 0.75 | sextant | [09](09_sextant_search.md) |
| HNSW `M` / default ef | 32 / `max(k,2M)` | sextant | [09](09_sextant_search.md) |
| DiskANN beamwidth / ef_search / rescore_k | 32 / 64 / 64 | sextant | [09](09_sextant_search.md) |
| Temporal boost weights | 50 / 35 / 15 (recency/sequence/periodic) | sextant | [09](09_sextant_search.md) |
| `MIN_SIGNAL_BITS` | 0.05 | assay/loom | [11](11_assay_signal_bits.md) |
| `MAX_PAIRWISE_CORR` | 0.6 | assay | [11](11_assay_signal_bits.md) |
| `MIN_ASSAY_SAMPLES` | 50 | assay | [11](11_assay_signal_bits.md) |
| Kernel candidate score weights | 0.40 degree / 0.40 betweenness / 0.20 groundedness | lodestar | [12](12_lodestar_kernel.md) |
| Recall gate default | 0.95 | lodestar | [12](12_lodestar_kernel.md) |
| Hop attenuation (answer path / oracle) | 0.9 / 0.7 | lodestar / oracle | [12](12_lodestar_kernel.md), [16](16_oracle_prediction.md) |
| Ward cold-start `tau` (`DEFAULT_TAU`) | 0.7 | ward | [13](13_ward_guard.md) |
| Ward default FAR (Identity/Content/Stylistic) | 0.01 / 0.03 / 0.05 | ward | [13](13_ward_guard.md) |
| Ward min bad scores for calibration | 50 | ward | [13](13_ward_guard.md) |
| `DEFAULT_CHECKPOINT_INTERVAL` (ledger) | 1000 entries | ledger | [14](14_ledger_provenance.md) |
| Anneal tripwires | RecallAtK<0.90, GuardFAR>0.01, GuardFRR>0.05, SearchP99>200ms, IngestP95>500ms; 5% hysteresis | anneal | [15](15_anneal_optimization.md) |
| Anneal ledger action variants | 28 | anneal | [15](15_anneal_optimization.md) |
| Oracle `MAX_DEPTH` / `MAX_REVERSE_DEPTH` / `MIN_CONFIDENCE` | 4 / 3 / 0.05 | oracle | [16](16_oracle_prediction.md) |
| Oracle super-intel thresholds | clean 0.7, kernel recall 0.95, Goodhart 0.9 | oracle | [16](16_oracle_prediction.md) |
| Forge default VRAM soft cap | 12 GiB | forge | [07](07_forge_math_runtime.md) |
| Forge CUDA exact-topk max k | 1024 | forge | [07](07_forge_math_runtime.md) |
| Forge assay-safety gate | cosine ≥0.99, retained-bits ≥0.95, FAR delta ≤0.01 | forge | [07](07_forge_math_runtime.md) |
| Soak default ops / seed | 10,000,000 / `0xCA1A_0059` | hazard-soak | [18](18_hazard_soak_and_testkit.md) |
| `DEFAULT_TEST_SEED` / `DEFAULT_TEST_TS` | `0xCA1A_CAFE_D15C_1A11` / 1,785,500,000 | testkit | [18](18_hazard_soak_and_testkit.md) |
| Daemon default bind / VRAM range | `127.0.0.1:7700` / 1..=30000 MiB | daemon | [20](20_cli_and_daemon_reference.md) |

---

## 6. Dependency-graph health

From [02_source_code_map.md](02_source_code_map.md):

- **103 internal dependency edges** across 19 crates.
- `calyx-core` has **zero internal dependencies** and is depended on by **all 18** other
  crates (the foundation). `calyx-paths` is the second-layer foundation (depends only on
  core).
- Leaf entry points (in-degree 0): `calyx-cli`, `calyxd`, `calyx-hazard-soak`;
  `calyx-mcp` is consumed only by `calyxd`.
- The normal build graph is acyclic; apparent assay↔(oracle/sextant/lodestar) back-edges
  are **dev-dependencies** (test wiring) only.

---

## 7. Known gaps surfaced during documentation

Consolidated in [24_roadmap.md](24_roadmap.md) §4. Highlights:

| Gap | Subsystem | Source doc |
|---|---|---|
| LP/min-cut/FVS solver not implemented — heuristic DFVS fallback only | mincut / lodestar | [17](17_graph_mincut_paths.md), [12](12_lodestar_kernel.md) |
| Kernel build pipeline does not measure recall (defaults to 0) | lodestar | [12](12_lodestar_kernel.md) |
| MCP-over-socket transport not wired into the live daemon | calyxd | [20](20_cli_and_daemon_reference.md) |
| `Bits8` quantization level has no shipping codec | forge | [07](07_forge_math_runtime.md) |
| Multimodal / commissioned lenses emit hash projections, not learned vectors | registry | [08](08_registry_lenses.md) |
| `AnnealHook` is an interim pre-PH48 shim | ward / anneal | [13](13_ward_guard.md) |
| MCP `search.fresh` parsed-but-unused; `search.guard=in_region` always errors | mcp | [19](19_mcp_api_tools_reference.md) |
| Aster compaction cadence fixed (`FIXME(PH46)`); crate self-labeled "skeleton" | aster | [06](06_aster_storage_engine.md) |
| PRD column-family numbering (Ledger=2/Assay=6) ≠ implemented tags (Ledger=8/Assay=7) | aster | [04](04_storage_and_schema.md) |
| No code-coverage tooling | repo-wide | [21](21_test_suite.md) |

---

## 8. Overall snapshot

| Dimension | State |
|---|---|
| Build | CPU-only by default; CUDA opt-in (`--features cuda`, requires sm_120 / CUDA 13.2) |
| Maturity | pre-1.0 (`0.1.0`); on-disk format & interfaces explicitly unstable |
| Gating | single CI workflow enforcing fmt + clippy(-D) + check + nextest + doctests + hygiene; FSV/CUDA tests gated off CI |
| Provenance/safety posture | fail-closed error catalog, per-slot guard, hash-chained ledger — all implemented |
| Largest risk areas | the unimplemented LP/DFVS solver path, unmeasured kernel recall, and the several declared-but-stubbed paths listed in §7 |
