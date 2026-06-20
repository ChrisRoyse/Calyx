# Calyx Test Suite — Inventory, Categories, How to Run

**Source files covered:**

- `Cargo.toml` (workspace deps: `proptest`, `criterion`)
- `rust-toolchain.toml`
- `CONTRIBUTING.md`
- `.githooks/pre-push`
- `.pre-commit-config.yaml`
- `.github/workflows/ci.yml`
- `scripts/check.sh`, `scripts/orphan_rs.sh`, `scripts/linecount.sh`, `scripts/verify_dataset.sh`, `scripts/check_manifest_coverage.sh`, `scripts/secret-scan.sh`
- `fuzz/Cargo.toml`, `fuzz/README.md`, `fuzz/fuzz_targets/*.rs`, `fuzz/corpus/*`
- `crates/calyx-testkit/src/lib.rs`, `crates/calyx-testkit/Cargo.toml`
- `crates/calyx-hazard-soak/src/{lib.rs,soak.rs,cli.rs,main.rs}`, `crates/calyx-hazard-soak/src/hazards/*`, `crates/calyx-hazard-soak/src/soak/ops.rs`
- `crates/*/tests/*.rs` (365 integration test files), `crates/*/src/**` (inline `#[cfg(test)]` modules)
- `docs/implementation/FSV_NOTES.md`

This document is a mechanical inventory of testing infrastructure as it exists in the
repo. Counts are derived by `ripgrep` over `#[test]` / `#[tokio::test]` attributes and
`proptest!` macro blocks. See [18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md)
for the soak/hazard harness and testkit internals, and
[22_verification_report.md](22_verification_report.md) for health-snapshot numbers.

---

## 1. Test inventory

### 1.1 Counting method

Counts are attribute-based (not function-name based) to avoid double-counting helper
`fn`s inside `proptest!` blocks and FSV support modules:

| Bucket | How counted |
|---|---|
| Unit (inline) | `#[test]` / `#[tokio::test]` occurrences under each crate's `src/` (inside `#[cfg(test)]` modules) |
| Integration | `#[test]` / `#[tokio::test]` occurrences under each crate's `tests/` dir |
| Property | `proptest!` macro blocks (each block holds one or more `fn` property tests run over N cases) |
| Files | `*.rs` files under `crates/*/tests/` |

Property tests written with the `proptest!` block macro do **not** carry a `#[test]`
attribute on the inner functions, so they are counted separately as macro blocks.

### 1.2 Per-crate table

| Crate | Integration test files | Integration `#[test]`/`#[tokio::test]` | Inline (unit) `#[test]`/`#[tokio::test]` | `proptest!` blocks (in `tests/`) |
|---|---:|---:|---:|---:|
| calyx-anneal | 74 | 278 | 1 | 28 |
| calyx-assay | 19 | 65 | 17 | 4 |
| calyx-aster | 38 | 53 | 543 | 0 |
| calyx-cli | 46 | 64 | 217 | 0 |
| calyx-core | 1 | 3 | 80 | 1 |
| calyx-forge | 14 | 68 | 206 | 4 |
| calyx-hazard-soak | 0 | 0 | 4 | 0 |
| calyx-ledger | 7 | 46 | 47 | 1 |
| calyx-lodestar | 39 | 118 | 0 | 7 |
| calyx-loom | 8 | 28 | 30 | 1 |
| calyx-mcp | 2 | 17 | 66 | 0 |
| calyx-mincut | 2 | 15 | 0 | 2 |
| calyx-oracle | 11 | 5 | 125 | 0 |
| calyx-paths | 1 | 7 | 0 | 1 |
| calyx-registry | 18 | 21 | 120 | 0 |
| calyx-sextant | 45 | 174 | 88 | 7 |
| calyx-testkit | 0 | 0 | 5 | 0 |
| calyx-ward | 35 | 171 | 33 | 8 |
| calyxd | 5 | 31 | 84 | 0 |
| **Total** | **365** | **1164** | **1666** | **64** |

**Workspace totals**

| Metric | Value |
|---|---|
| `#[test]` + `#[tokio::test]` attributes (src + tests) | **2830** |
| Integration test files (`crates/*/tests/*.rs`) | **365** |
| `proptest!` macro blocks (workspace, src + tests) | ~438 occurrences of `proptest`/`proptest!` across 188 files; 64 `proptest!` blocks in `tests/` |
| FSV-named test files (`*fsv*.rs` under `tests/`) | **150** |
| Fuzz targets (`fuzz/fuzz_targets/`) | **6** |

Notes on per-crate anomalies (documenting WHAT IS):

- **calyx-aster** has the bulk of unit tests inline in `src/` (543), with only 53
  integration `#[test]`s — its `cf/tests.rs`, `wal/tests.rs`, `index/btree_tests.rs`
  modules carry the load.
- **calyx-oracle** shows only 5 integration `#[test]`s across 11 files because its
  `tests/*_fsv.rs` files (`predict_fsv.rs`, `ph50_exit_fsv.rs`,
  `rolled_recurrence_fsv.rs`, `super_intel_fsv.rs`) are FSV-style: large single tests
  plus many helper `fn`s and a `tests/support/` module. Its unit coverage (125) lives
  inline in `src/`.
- **calyx-anneal** is integration-heavy (74 files, 278 tests, 28 property blocks) and
  carries almost no inline unit tests (1).
- **calyx-lodestar**, **calyx-mincut**, **calyx-paths** carry **zero** inline `src/`
  unit tests — all coverage is in `tests/`.

---

## 2. Test categories

| Category | Mechanism | Crates that use it |
|---|---|---|
| Unit | inline `#[cfg(test)] mod tests` in `src/` | all except calyx-lodestar, calyx-mincut, calyx-paths (which test only via `tests/`) |
| Integration | `crates/*/tests/*.rs` | all 17 library crates + calyxd (365 files) |
| Property-based | `proptest!` blocks + `proptest` strategies (workspace dep `proptest = "1"`) | calyx-anneal, calyx-assay, calyx-core, calyx-forge, calyx-ledger, calyx-lodestar, calyx-loom, calyx-mincut, calyx-paths, calyx-sextant, calyx-ward (188 files reference `proptest`) |
| FSV (Field/Final System Verification) | `*_fsv.rs` test files that write source-of-truth byte artifacts; some `#[test]` (run in CI, artifacts to temp), some `#[ignore]`d (aiwonder-only) | calyx-aster, calyx-ledger, calyx-forge, calyx-oracle, calyx-registry, calyx-sextant, calyx-ward, calyx-loom, calyx-assay, calyx-lodestar, calyx-anneal, calyxd (150 FSV files) |
| Fuzz | `cargo-fuzz` / `libfuzzer-sys` targets in `fuzz/` | calyx-aster, calyx-core, calyx-mcp, calyx-sextant (fuzz crate deps) |
| Soak / Hazard | `calyx-hazard-soak` binary crate (`soak`/`hazards` modules) | calyx-hazard-soak (drives many crates) |
| Golden-vector | tests comparing against checked-in expected vectors / `tests/golden/` | calyx-forge (`tests/golden/generate_golden.py`, `turboquant_tests.rs`, `cuda_parity.rs`, `cpu_kernels.rs`), calyx-ledger (`merkle_tests.rs`), calyx-aster (`btree_index_fsv.rs`), calyx-registry (`stage3_atomic_fsv.rs`) — 17 files reference "golden" |
| Doctests | `cargo test --doc` examples in `///` doc comments | workspace-wide (CI runs a dedicated doctest step) |
| Bench (not a test gate) | `criterion` benches in `benches/` | calyx-aster, calyx-forge, calyx-hazard-soak |
| Mutation (agent-invoked, not CI) | `cargo mutants` | per CONTRIBUTING.md, run manually on aiwonder |

### 2.1 FSV (Field / Final System Verification)

Per `docs/implementation/FSV_NOTES.md`: **FSV tools print source-of-truth bytes for a
human or agent to inspect; they do not emit pass/fail verdicts.** "A passing test is a
claim; the bytes are the verdict." FSV tests persist artifacts (SST/WAL/MANIFEST bytes,
vault trees, JSON evidence, `BLAKE3SUMS.txt`) which are then read back with the
`calyx readback` / `calyx verify-chain` CLI surfaces (see
[20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md)) and attached to a
GitHub issue.

There are two FSV sub-styles in the code:

1. **Runnable FSV** (plain `#[test]`, runs in CI). Example:
   `crates/calyx-oracle/tests/predict_fsv.rs::ph49_fsv_swe_bench_deficit_refuses_and_caps_predictions`
   is a plain `#[test]`. It writes artifacts under a root chosen by `CALYX_FSV_ROOT`,
   defaulting to `std::env::temp_dir()` when the env var is unset — so it is
   runner-safe.
2. **aiwonder-only FSV** (`#[ignore]`d). These are gated off CI and run explicitly with
   `--ignored` on the GPU/evidence host, e.g.
   `#[ignore = "aiwonder FSV writes source-of-truth artifacts"]`,
   `#[ignore = "server-only FSV trigger writes ... artifacts"]`,
   `#[ignore = "manual aiwonder FSV fixture; set CALYX_WARD_IDENTITY_FSV_DIR"]`.

There are **174** `#[ignore]` attributes in `tests/` and **33** in `src/`.

### 2.2 FSV gating mechanism (exact)

| Knob | Type | Effect |
|---|---|---|
| `#[ignore]` attribute | compile-time test attribute | Excludes the test from default `cargo test` / `cargo nextest run`. CI never runs ignored tests. Run explicitly with `cargo test -- --ignored` (or per-test by name). This is the **primary** skip gate for aiwonder-only suites. |
| `CALYX_FSV_ROOT` | env var (read via `std::env::var`/`var_os`, 129 occurrences) | **Destination** of FSV evidence artifacts, not a skip gate. When unset, FSV tests fall back to `std::env::temp_dir().join(...)`. Set it to a durable path (e.g. an aiwonder evidence vault) so byte-readback artifacts persist. |
| `--features cuda` | cargo feature | Enables GPU code paths and CUDA-only tests (`cfg(feature = "cuda")`). Defined `cuda = ["dep:cudarc"]` in calyx-forge; propagated via `calyx-forge/cuda` by calyx-hazard-soak, calyx-loom, calyx-registry, calyx-ward, calyxd. CUDA tests are also typically `#[ignore]`d (`"requires a CUDA GPU (run on aiwonder with --features cuda --ignored)"`). |
| Scenario env vars | env vars read inside specific FSV tests | Many FSV/fault-injection tests key off named env vars to select a failure scenario or fixture, e.g. `CALYX_LEDGER_CHAIN_BROKEN`, `CALYX_ASTER_CORRUPT_SHARD`, `CALYX_ASSAY_INSUFFICIENT_SAMPLES`, `CALYX_SEXTANT_QUERY_SHAPE`, `CALYX_ORACLE_INSUFFICIENT`, `CALYX_WARD_VOXCELEB_BAD_WAV`, `CALYX_IO_ERROR`, `CALYX_DECRYPTION_FAILED`, `CALYX_HOME`, etc. (50+ distinct `CALYX_*` vars). These select error/fault branches rather than gate test execution. |

### 2.3 Fuzz targets

`fuzz/` is a standalone `cargo-fuzz` crate (`name = "calyx-fuzz"`, `libfuzzer-sys`,
`[workspace]` of its own). Six targets map to PRD `28 §6c` untrusted-input boundaries:

| Target | Boundary exercised |
|---|---|
| `aster_sst_decode` | Aster SST/shard reader via `SstReader::open` |
| `aster_wal_replay` | WAL segment replay via `wal::replay_dir` |
| `aster_manifest_decode` | durable vault manifest load via `ManifestStore` |
| `query_parse` | Sextant query JSON decode, validation, planning bounds |
| `lens_output_decode` | `SlotVector` JSON/raw f32 schema validation (dense, sparse, multi-vector) |
| `mcp_jsonrpc_decode` | MCP JSON-RPC request/batch wire decode |

Each has a seed corpus dir under `fuzz/corpus/<target>/`. Fuzz crate deps:
calyx-aster, calyx-core, calyx-mcp, calyx-sextant.

### 2.4 Soak / hazard

`calyx-hazard-soak` is a binary crate (`main.rs` + `cli.rs`) with `soak` and `hazards`
library modules. `hazards/` is grouped into `numerical`, `operational`
(`operational_h13_14.rs`, `_h15_16`, `_h17_19`, `_h20_21`), `resource`
(`resource_hazards_6_8.rs`), `security`, and `heap_soak.rs`, each with `*_support.rs`
helpers. `soak/ops.rs` holds soak operations. It has a `cuda` feature
(`calyx-forge/cuda`). Full detail in
[18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md).

---

## 3. How to run tests

### 3.1 Toolchain

`rust-toolchain.toml` pins channel **1.95.0**, profile `minimal`, components
`clippy` + `rustfmt`. Workspace is edition 2024, `rust-version = "1.95"`.

### 3.2 Commands (exact)

| Goal | Command |
|---|---|
| Full workspace test (built-in harness) | `cargo test --workspace` |
| Full workspace test (parallel, CI uses this) | `cargo nextest run --workspace` |
| Doctests (nextest does NOT run doctests) | `cargo test --workspace --doc` |
| Single crate | `cargo test -p calyx-aster` |
| Run `#[ignore]`d (aiwonder/FSV/CUDA) tests | `cargo test -- --ignored` (or `cargo nextest run --run-ignored all`) |
| CUDA-feature tests | `cargo test -p calyx-forge --features cuda -- --ignored` |
| Persist FSV evidence to a durable path | `CALYX_FSV_ROOT=/home/croyse/calyx/data/<run> cargo test -p calyx-oracle predict_fsv` |
| Format check (pre-push gate) | `cargo fmt --all -- --check` |
| Clippy (deny warnings) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Full local gate (mirrors CI) | `bash scripts/check.sh` |

### 3.3 The gate script (`scripts/check.sh`)

`scripts/check.sh` is the per-merge gate (same bar as CI). It sets
`CARGO_INCREMENTAL=0` and runs, in order:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo nextest run --workspace` (errors loudly if `cargo-nextest` missing)
5. `cargo test --workspace --doc`
6. `bash scripts/orphan_rs.sh` (orphaned `.rs` gate)
7. `bash scripts/linecount.sh` (line-count gate)
8. `bash scripts/verify_dataset.sh --self-test` (PH69 T01 MANIFEST tooling)
9. `bash scripts/check_manifest_coverage.sh --self-test` (PH69 T08 BUILD_DONE coverage)

### 3.4 Fuzz / soak / mutation

```bash
# Fuzz (aiwonder, from repo root) — see fuzz/README.md
cargo fuzz list
cargo fuzz run aster_sst_decode fuzz/corpus/aster_sst_decode -- -runs=1000
cargo fuzz run query_parse fuzz/corpus/query_parse -- -runs=1000

# Mutation testing (agent-invoked on aiwonder, not hosted CI) — see CONTRIBUTING.md
cargo mutants --in-diff origin/main...HEAD --check
cargo mutants --package calyx-core --package calyx-aster

# Soak / hazard — run the calyx-hazard-soak binary (see 18_hazard_soak_and_testkit.md)
```

Per CONTRIBUTING.md a fuzzer crash artifact is "not handled" until a GitHub issue and a
regression test exist; every survived mutant is a test-gap issue.

---

## 4. Fixtures & test utilities

### 4.1 `calyx-testkit`

`crates/calyx-testkit/src/lib.rs` (single file, 217 lines) is the shared deterministic
scaffolding. Depends on calyx-core + `proptest` + `rand`. Public API:

| Item | Type | Purpose |
|---|---|---|
| `DEFAULT_TEST_SEED` | `const u64 = 0xCA1A_CAFE_D15C_1A11` | default RNG seed |
| `DEFAULT_TEST_TS` | `const Ts = 1_785_500_000` | default fixed timestamp |
| `seeded_rng(seed: u64) -> StdRng` | fn | `StdRng::seed_from_u64(seed)` — repeatable RNG |
| `fixed_clock() -> FixedClock` | fn | `FixedClock::new(DEFAULT_TEST_TS)` — injectable deterministic clock |
| `slot_id_strategy() -> BoxedStrategy<SlotId>` | proptest strategy | stable slot ids |
| `cx_id_strategy() -> BoxedStrategy<CxId>` | proptest strategy | 16-byte constellation ids |
| `modality_strategy() -> BoxedStrategy<Modality>` | proptest strategy | one of Text/Code/Image/Audio/Video/Protein/Dna/Molecule/Structured/Mixed |
| `anchor_kind_strategy() -> BoxedStrategy<AnchorKind>` | proptest strategy | anchor kinds |
| `absent_reason_strategy() -> BoxedStrategy<AbsentReason>` | proptest strategy | absent reasons |
| `slot_vector_strategy() -> BoxedStrategy<SlotVector>` | proptest strategy | slot vectors |
| `small_constellation_strategy() -> BoxedStrategy<Constellation>` | proptest strategy | small constellations |

Direct dependents (in `Cargo.toml`): **calyx-oracle**. (Other crates inline their own
proptest strategies or use `calyx-testkit::fixed_clock` via dev-deps; the only crate
declaring it as a dep besides itself is calyx-oracle.) See
[18_hazard_soak_and_testkit.md](18_hazard_soak_and_testkit.md) for deeper detail.

The CONTRIBUTING.md "FIRST" doctrine codifies determinism: seed RNGs with
`StdRng::seed_from_u64`, inject `Clock`, never read wall time in logic; `sleep()` as
synchronization and lingering `#[ignore]` are forbidden in committed tests.

### 4.2 Shared per-crate helpers

Several crates ship `tests/*_support.rs` / `tests/support/` modules (not test targets
themselves, pulled in as `mod`s), e.g. `calyx-forge/tests/cuda_parity_support.rs`,
`calyx-oracle/tests/support/`, and the hazard-soak `*_support.rs` modules. Golden
vectors for forge are generated by `crates/calyx-forge/tests/golden/generate_golden.py`.

---

## 5. Coverage / CI

### 5.1 CI workflow

There is exactly **one** GitHub Actions workflow: `.github/workflows/ci.yml`
(job `gate`, `runs-on: ubuntu-latest`, `timeout-minutes: 60`). Triggers: `push` to
`main` and all `pull_request`. Steps:

| Step | Command |
|---|---|
| checkout | `actions/checkout@v4` |
| cache | `Swatinem/rust-cache@v2` |
| install nextest | `taiki-e/install-action@nextest` |
| rustfmt | `cargo fmt --all -- --check` |
| check (all targets) | `cargo check --workspace --all-targets` |
| clippy (deny warnings) | `cargo clippy --workspace --all-targets -- -D warnings` |
| test (nextest, parallel) | `cargo nextest run --workspace` |
| doctests | `cargo test --workspace --doc` |
| orphaned `.rs` gate | `bash scripts/orphan_rs.sh` |
| line-count gate | `bash scripts/linecount.sh` |

The workflow header states aiwonder-only FSV suites are `#[ignore]`d and the `cuda`
feature is off by default, "so the workspace is runner-safe." CI does **not** run
ignored tests, CUDA tests, fuzz, soak, or mutation testing.

### 5.2 Git hooks / pre-commit

| Gate | File | What it enforces |
|---|---|---|
| pre-push | `.githooks/pre-push` | Rejects any push whose tree is not `cargo fmt --all -- --check` clean. Fail-loud if cargo is missing. Install once: `git config core.hooksPath .githooks` (done by `scripts/aiwonder-build-setup.sh`). |
| pre-commit | `.pre-commit-config.yaml` | Single local hook `calyx-secret-scan` → `scripts/secret-scan.sh` (`pass_filenames: false`). |

### 5.3 Coverage tooling

**Not determined from source / none found.** No `cargo-tarpaulin`, `llvm-cov`, `grcov`,
or `codecov` configuration exists in `.github/`, `scripts/`, `CONTRIBUTING.md`, or
`Cargo.toml`. Test-usefulness is instead asserted via **mutation testing**
(`cargo mutants`, agent-invoked on aiwonder per CONTRIBUTING.md) and **FSV byte
readback**, not line/branch coverage percentages. Per CONTRIBUTING.md: "Tests are the
fast claim. FSV is the verdict."

---

## 6. Gaps / not covered

- **No line/branch coverage metric** is produced anywhere in the repo (see 5.3). Any
  coverage figure must come from running `cargo mutants` manually; there is no checked-in
  report.
- **Property-test case counts are not enumerable statically** — `proptest!` blocks run
  N cases each (default 256 unless overridden); this doc counts blocks, not realized
  cases.
- **Mutation, fuzz, soak, and CUDA results are off-CI** and live on the aiwonder host /
  GitHub issues, not in this repo; their current pass state is not determinable from
  source.
- **calyx-testkit declared-dependent count is 1** (calyx-oracle) in `Cargo.toml` graph;
  other crates' use of testkit (if any) is via dev-dependencies not surfaced by a plain
  manifest grep — exact dev-dep fan-out not fully determined here.
- The `2830` attribute total counts `#[test]`/`#[tokio::test]` only; it excludes the 64
  `proptest!` blocks in `tests/` and inline `proptest!` blocks in `src/`, so realized
  test executions exceed 2830.
