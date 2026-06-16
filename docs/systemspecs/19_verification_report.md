# 19. Verification Report

**Source files covered:** whole-workspace metrics derived by enumeration over `crates/`, `fuzz/`, `Cargo.toml`, `rust-toolchain.toml`, and `scripts/`. Counts were produced by `find`/`grep` over the tree on branch `issue569-ph71-remove-shadow` at commit `ac97a37` (2026-06-15). This document records measurements, not test execution results — see §1 note.

---

## 1. Test execution status

| Item | Value | Source |
|---|---|---|
| Test execution performed here | **No** — builds/tests run on remote host "aiwonder" (`/home/croyse/calyx`), not the local authoring checkout | `README.md`, `docs/implementation/01_AIWONDER_ENVIRONMENT.md` |
| Total test functions (declared) | **2618** `#[test]` (1515 unit in `src/`, 1103 integration in `tests/`) | grep over `crates/` |
| `#[tokio::test]` | 0 (suite is synchronous) | grep over `crates/` |
| Tests gated `#[ignore]` | 196 (176 reference aiwonder/FSV environments) | grep over `crates/` |
| Pass/fail | Not determined from source (not executed locally) | — |

> To run the gate, see [18_test_suite.md](18_test_suite.md) §"How to run". The standard command is `cargo nextest run` (plus `cargo test --doc`), per `scripts/check.sh`.

## 2. Lint / gate configuration

| Gate step | Tool | Source |
|---|---|---|
| Format | `cargo fmt --check` (rustfmt component pinned) | `rust-toolchain.toml`, `scripts/check.sh` |
| Type/check | `cargo check` | `scripts/check.sh` |
| Lint | `cargo clippy` (clippy component pinned) | `rust-toolchain.toml`, `scripts/check.sh` |
| Test | `cargo nextest run` | `scripts/check.sh` |
| Doctests | `cargo test --doc` | `scripts/check.sh` |
| Pre-commit | `.pre-commit-config.yaml`, `.githooks/`, `.gitleaksignore` (secret scanning) | repo root |

Status of lint runs: Not determined from source (not executed here).

## 3. Codebase metrics

| Metric | Value | How measured |
|---|---|---|
| Workspace crates | 18 | `crates/*/` |
| Binaries | 3 (`calyx`, `calyxd`, `calyx-mcp`) | `[[bin]]` in Cargo.tomls |
| Rust source files (`.rs`) | 1119 | `find crates -name '*.rs'` |
| Rust LOC (incl. tests/comments) | ~273,869 | `cat` over all `.rs` |
| Integration test files (`tests/*.rs`) | 292 across 17 crates | per-crate `tests/` listing |
| FSV harness files (`*fsv*.rs`) | 156 | `find` |
| `proptest!` invocations | 162 | grep |
| cargo-fuzz targets | 6 | `fuzz/fuzz_targets/` |
| Criterion benches | 2 (`bench_arena_reset`, `bench_admission_overhead`) | Cargo.toml `[[bench]]` |
| Declared test functions | 2618 | grep `#[test]` |
| `CALYX_*` identifiers in core | 58 distinct (≈38 are the closed PRD-18 error-code catalog; remainder are module-local codes/constants) | grep over `crates/calyx-core/src/` — see [04_core_foundation.md](04_core_foundation.md) |
| Column families (static) | ~30 (plus per-slot vector columns) | [05_aster_storage.md](05_aster_storage.md) |
| MCP production tools registered | 0 | [16_mcp_and_daemon.md](16_mcp_and_daemon.md) |

### 3.1 LOC and test-file distribution by crate

| Crate | LOC (approx) | `.rs` files | `tests/` files |
|---|---|---|---|
| calyx-aster | 52,171 | 214 | 30 |
| calyx-anneal | 39,770 | 151 | 65 |
| calyx-cli | 39,264 | 186 | 31 |
| calyx-sextant | 24,449 | 104 | 38 |
| calyx-registry | 19,342 | 72 | 15 |
| calyx-forge | 17,338 | 71 | 14 |
| calyx-lodestar | 15,387 | 62 | 29 |
| calyx-ward | 14,603 | 51 | 23 |
| calyx-oracle | 11,648 | 41 | 5 |
| calyx-assay | 11,260 | 45 | 17 |
| calyx-ledger | 7,885 | 25 | 7 |
| calyx-loom | 5,945 | 27 | 8 |
| calyxd | 5,686 | 20 | 4 |
| calyx-core | 5,349 | 25 | 1 |
| calyx-registry-adj./mincut | 1,695 | 10 | 2 |
| calyx-mcp | 1,025 | 8 | 2 |
| calyx-paths | 730 | 6 | 1 |
| calyx-testkit | 217 | 1 | 0 |

(LOC and file counts are the per-crate measurements from §3; `mincut` row label is approximate.)

## 4. Schema / format versions

| Artifact | Magic / version | Source |
|---|---|---|
| WAL segment | magic `CXW1`, 20-byte header, 64 MiB segments | [05_aster_storage.md](05_aster_storage.md) |
| SSTable | magic `CXS1`, version 2, bloom-filtered, CRC32 | [05_aster_storage.md](05_aster_storage.md) |
| Manifest | atomic `CURRENT`/`MANIFEST` JSON swap | [05_aster_storage.md](05_aster_storage.md) |
| Ledger entry hash / Merkle | length-framed BLAKE3, domain-separated tree, optional ed25519 | [11_ledger_provenance.md](11_ledger_provenance.md) |
| LensId / contract | length-delimited truncated BLAKE3 (16 bytes) | [04_core_foundation.md](04_core_foundation.md), [07_registry_lenses.md](07_registry_lenses.md) |
| Rust toolchain | `1.95.0` (profile minimal; clippy, rustfmt) | `rust-toolchain.toml` |
| Workspace version | `0.1.0`, edition 2024, rust-version 1.95 | root `Cargo.toml` |

## 5. Notable constants / magic numbers

| Constant | Value | Subsystem | Doc |
|---|---|---|---|
| Group-commit fsync window | 2 ms | Aster | 05 |
| WAL segment size | 64 MiB | Aster | 05 |
| HNSW M | 32 | Sextant | 08 |
| DiskANN block size | 4 KiB page-aligned | Sextant | 08 |
| BM25 k1 / b | 1.2 / 0.75 | Sextant | 08 |
| RRF rank constant | 60 (`weight/(rank+60)`) | Sextant | 08 |
| WeightedRRF slot profiles | 14 named | Sextant | 08 |
| Graph hop attenuation | `0.9^hop` | paths | 10 |
| Consequence-tree attenuation | `0.7` (depth 4) | Oracle | 14 |
| Reverse-query depth | 3 | Oracle | 14 |
| Kernel score weights | degree/betweenness/groundedness 0.40/0.40/0.20 | Lodestar | 10 |
| DFVS exact threshold | ≤20 nodes | Lodestar | 10 |
| Recall@k gate | 0.95 | Lodestar | 10 |
| Cross-term interaction gate | pair-gain ≥ 0.05 bits | Loom | 09 |
| Differentiation contract | ≥0.05 bits, ≤0.6 correlation | Assay | 09 |
| MXFP4 block | 32 values / 16 bytes, E8M0 scale | Forge | 06 |
| Quant preservation gate | cosine ≥ 0.99 | Forge | 06 |
| Autotune A/B promotion margin | > 2% | Forge | 06 |
| GEMM golden tolerance | 1e-3 rel / 1e-6 abs | Forge | 06 |
| Guard FAR ceilings | Identity 0.01 / Stylistic 0.05 / Content 0.03 | Ward | 12 |
| Anneal tripwires | recall@k, guard FAR/FRR, search-p99, ingest-p95 | Anneal | 13 |
| J-objective | 8 positive terms − redundancy/ungrounded/Goodhart penalties | Anneal | 13 |
| VRAM budget range | `1..=30000` MiB | calyxd config | 03 |

## 6. Coverage tooling

No coverage tooling (tarpaulin / llvm-cov / grcov) is configured in the workspace. Coverage metrics: Not determined from source.

## 7. What is NOT covered / known gaps (from source)

- **Local test results unavailable**: this repo is an authoring checkout; pass/fail and lint status must be obtained from the aiwonder build host.
- **MCP tools**: `calyx-mcp` registers 0 production tools on this branch; the JSON-RPC machinery exists but exposes no Calyx operations yet ([16_mcp_and_daemon.md](16_mcp_and_daemon.md)).
- **calyxd MCP socket**: the loopback MCP-over-socket transport exists but `main` does not wire it ([16_mcp_and_daemon.md](16_mcp_and_daemon.md)).
- **Sextant/HNSW persistence**: search engine and vector indexes are in-memory; not persisted as engine state ([08_sextant_search.md](08_sextant_search.md)).
- **LP solver**: `calyx-mincut` LP scaffold is a relaxation skeleton with no actual solver ([10_graph_kernel.md](10_graph_kernel.md)).
- **Build state**: per `README.md`, Stages 0–8 are FSV-signed-off; Stage 9 (PH40–PH42) is in closeout; later stages/phases (PH43+) exist in `docs/implementation/` and partially in code (e.g. `calyx-anneal`, `calyx-oracle`, leapable shadow-removal on the current branch) but are not all signed off. Verify per-phase status against GitHub issue #23 (the stated source of truth).
