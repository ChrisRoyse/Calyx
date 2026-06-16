# 18. Test Suite & Verification Infrastructure

This document describes the test and verification infrastructure of the Calyx
Rust workspace as it exists in source. All counts were measured directly with
`grep`/`find` over `crates/` and the repo root on the working tree.

Source files covered (representative):

- `Cargo.toml` (workspace, build profiles)
- `.config/nextest.toml` (parallel test runner config)
- `scripts/check.sh` (the manual per-merge gate run on the aiwonder build host)
- `scripts/fsv_ph36.sh`, `scripts/aiwonder-build-setup.sh`, `scripts/orphan_rs.sh`, `scripts/linecount.sh`, `scripts/verify_dataset.sh`, `scripts/check_manifest_coverage.sh`
- `crates/calyx-testkit/src/lib.rs`, `crates/calyx-testkit/Cargo.toml`
- `crates/calyx-cli/src/fsv.rs`
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/*.rs`
- `crates/calyx-aster/benches/bench_arena_reset.rs`, `crates/calyx-forge/benches/bench_admission_overhead.rs`
- Per-crate `tests/` directories under `crates/*/tests/`
- Sample FSV test: `crates/calyx-anneal/tests/rollback_fsv.rs`

---

## 1. Test inventory (per-crate counts)

Counts of the `#[test]` attribute, split between unit tests (in `src/`) and
integration tests (in the crate's `tests/` directory). There are **0**
`#[tokio::test]` functions anywhere in `crates/` — the workspace's tests are
synchronous.

| Crate | `#[test]` in `src/` (unit) | `#[test]` in `tests/` (integration) | Total |
|-------|---------------------------:|------------------------------------:|------:|
| calyx-anneal   | 1   | 268 | 269 |
| calyx-assay    | 17  | 65  | 82  |
| calyx-aster    | 507 | 47  | 554 |
| calyx-cli      | 153 | 62  | 215 |
| calyx-core     | 80  | 3   | 83  |
| calyxd         | 82  | 27  | 109 |
| calyx-forge    | 206 | 68  | 274 |
| calyx-ledger   | 47  | 46  | 93  |
| calyx-lodestar | 0   | 118 | 118 |
| calyx-loom     | 30  | 28  | 58  |
| calyx-mcp      | 21  | 13  | 34  |
| calyx-mincut   | 0   | 15  | 15  |
| calyx-oracle   | 125 | 5   | 130 |
| calyx-paths    | 0   | 7   | 7   |
| calyx-registry | 120 | 21  | 141 |
| calyx-sextant  | 88  | 139 | 227 |
| calyx-testkit  | 5   | 0   | 5   |
| calyx-ward     | 33  | 171 | 204 |
| **TOTAL**      | **1515** | **1103** | **2618** |

- **Total `#[test]` functions: 2618.**
- **Total `#[tokio::test]` functions: 0.**
- 17 of the 18 crates have a `tests/` integration directory
  (`crates/*/tests/`); only `calyx-testkit` does not.

Note: a `#[test]` count overcounts the runnable test total slightly because
some tests are wrapped in `proptest! { #[test] fn ... }` blocks and some are
generated. These are attribute occurrences, not post-expansion test cases.

`tests/` directories present (17 crates): calyx-anneal, calyx-assay,
calyx-aster, calyx-cli, calyx-core, calyxd, calyx-forge, calyx-ledger,
calyx-lodestar, calyx-loom, calyx-mcp, calyx-mincut, calyx-oracle, calyx-paths,
calyx-registry, calyx-sextant, calyx-ward.

The integration test files are numerous (e.g. calyx-anneal has ~65 files,
calyx-sextant ~45, calyx-ward ~30). Files follow consistent naming conventions:
`*_fsv.rs` (full-system-verification), `*_readback.rs` / `*_readback_fsv.rs`
(byte-level readback evidence, concentrated in calyx-cli), `ph<NN>_*` (phase
suites), and `issue<NNN>_*` (per-issue regression suites).

---

## 2. Test categories present

| Category | Where | Evidence / count |
|----------|-------|------------------|
| Unit tests | `#[cfg(test)] mod tests` inside `src/` | 1515 `#[test]` in `src/`; e.g. `crates/calyx-testkit/src/lib.rs`, `crates/calyx-cli/src/fsv.rs` |
| Integration tests | crate `tests/` dirs | 1103 `#[test]` across 17 crates' `tests/` dirs |
| Property tests (proptest) | `proptest!` macro | `proptest` is a workspace dependency (`Cargo.toml`); 162 files use the `proptest!` macro; 162 macro invocations across `crates/` |
| Fuzz targets | `fuzz/` at repo root | `fuzz/Cargo.toml` (cargo-fuzz, `libfuzzer-sys = "0.4"`) with 6 targets in `fuzz/fuzz_targets/`: `aster_sst_decode`, `aster_wal_replay`, `aster_manifest_decode`, `query_parse`, `lens_output_decode`, `mcp_jsonrpc_decode`; a `corpus/` dir is present |
| Criterion benchmarks | `benches/` | Two crates: `crates/calyx-aster/benches/bench_arena_reset.rs` and `crates/calyx-forge/benches/bench_admission_overhead.rs`. Both declared `[[bench]]` with `harness = false`; `criterion = "0.5"` is the workspace dep |
| FSV (full-system-verification) | `*_fsv.rs` integration tests + `scripts/fsv_*.sh` | 130 `tests/*_fsv.rs` files; see section 3 |

Doctests are also run as a separate category (`cargo test --workspace --doc`),
since nextest does not execute them — see `scripts/check.sh`.

---

## 3. FSV (full-system-verification) discipline

"FSV" is a project-specific verification discipline, not a third-party
framework. It denotes end-to-end tests that exercise real durable storage paths
and emit **source-of-truth evidence artifacts** (JSON logs, hex/`xxd` dumps)
under an FSV "root" directory, which are then "signed off" on the remote build
host (aiwonder). The README repeatedly states that components are
"FSV-signed-off on aiwonder" and lists concrete FSV root paths such as
`/home/croyse/calyx/data/fsv-stage5-...` (`README.md`).

How FSV is realized in code:

- **`#[ignore]`-gated tests.** FSV tests are marked `#[ignore]` so they do not
  run in the ordinary gate; they are invoked explicitly with `-- --ignored`.
  There are 196 `#[ignore]` attributes across 171 files; 176 mention "aiwonder".
  Common ignore reasons include
  `"aiwonder FSV writes source-of-truth artifacts"`,
  `"requires aiwonder HF cache/network ..."`, and
  `"requires a CUDA GPU (run on aiwonder with --features cuda --ignored)"`.
  The manual gate keeps aiwonder-only FSV suites `#[ignore]`d so ordinary
  claim-check tests stay separate from the byte-readback FSV gate.

- **Environment-rooted artifact emission.** An FSV test reads an env var
  pointing at the FSV root, creates a durable vault there, performs the real
  operation, and writes evidence. Example
  `crates/calyx-anneal/tests/rollback_fsv.rs`:
  `#[ignore = "requires CALYX_ISSUE396_FSV_ROOT on aiwonder"]`, reads
  `CALYX_ISSUE396_FSV_ROOT`, opens `AsterVault::new_durable(...)`, and uses
  `serde_json::json` to emit artifacts.

- **CLI FSV harness.** `crates/calyx-cli/src/fsv.rs` provides on-disk drills
  (`arrow_demo`, `cf_demo`, `mvcc_demo`, `wal_drill`, `wal_replay`,
  `corrupt_shard`, `readback_level`) that print tab-delimited readback lines
  (e.g. `WAL_DRILL\t...`, `CORRUPT_SHARD\t...`) for verification.

- **FSV driver scripts.** `scripts/fsv_ph36.sh` sets `CALYX_FSV_ROOT`, runs the
  ignored FSV integration test with `cargo test -p calyx-cli --test
  ph36_fsv_integration ... -- --ignored --nocapture`, tees the log, produces an
  `xxd` dump, and `grep`s for expected tamper-detection markers (e.g.
  `PH36 FSV PASS: tamper detected at seq=11`).

- **Validation scripts** (also under `scripts/`): `validate_anneal_j.sh`,
  `validate_lodestar_kernel.sh`, `validate_media_emotion.sh`,
  `validate_media_image.sh`, `validate_sextant_recall.sh`,
  `ph70_evidence_bundle.sh`.

---

## 4. How to run tests

The repo states that all build/test/verification work happens on the remote
host "aiwonder" (`README.md`: "All build, test, and verification work happens on
aiwonder ..."). The per-merge gate is `scripts/check.sh`, run on aiwonder before
every merge (README section "Run the gate on aiwonder before every merge").

Standard commands (from `scripts/check.sh`):

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace      # primary parallel test run
cargo test --workspace --doc       # doctests (nextest does not run these)
```

- `scripts/check.sh` exports `CARGO_INCREMENTAL=0`, fails loudly if
  `cargo-nextest` is missing, then additionally runs `scripts/orphan_rs.sh`,
  `scripts/linecount.sh`, `scripts/verify_dataset.sh --self-test`, and
  `scripts/check_manifest_coverage.sh --self-test`.
- FSV / ignored tests run explicitly, e.g.
  `cargo test -p <crate> --test <name> -- --ignored --nocapture`, usually with
  an FSV-root env var set (see `scripts/fsv_ph36.sh`). CUDA-gated tests require
  `--features cuda`.

Nextest configuration (`.config/nextest.toml`, `[profile.default]`):
`test-threads = "num-cpus"`, `fail-fast = false`, `retries = 0` (flakiness is
treated as a bug, never masked by re-runs), and
`slow-timeout = { period = "120s", terminate-after = 10 }` (flags slow tests at
120s, terminates only after ~20 min so long FSV/dataset tests are not falsely
killed).

Build profiles (`Cargo.toml`): `[profile.dev] debug = "line-tables-only"` and
dependencies stripped of debuginfo, to keep test/dev executables small on the
shared build host. Machine-local accelerations (mold linker, incremental
policy, target GC) are provisioned by `scripts/aiwonder-build-setup.sh` and kept
out of the committed `.cargo/config.toml` so Windows/local authoring checkouts
are not broken.

---

## 5. Manual gate scripts

| Path | Purpose |
|------|---------|
| `scripts/check.sh` | The manual aiwonder per-merge gate: rustfmt check, `cargo check --workspace --all-targets`, `cargo clippy ... -D warnings`, `cargo nextest run --workspace`, `cargo test --workspace --doc`, `scripts/orphan_rs.sh`, `scripts/linecount.sh`, and dataset/manifest self-tests. |
| `scripts/orphan_rs.sh` (+ `orphan_rs_allow.txt`) | Gate: detects `.rs` files not wired into the build. |
| `scripts/linecount.sh` | Gate: line-count limits. |
| `scripts/verify_dataset.sh` / `scripts/check_manifest_coverage.sh` | Dataset MANIFEST tooling self-tests (hermetic synthetic batteries). |
| `scripts/fsv_ph36.sh`, `scripts/validate_*.sh`, `scripts/ph70_evidence_bundle.sh` | FSV / validation drivers (run manually on aiwonder). |
| `scripts/acquire_*.sh`, `scripts/dataset_acquire_lib.sh` | Dataset acquisition (not test runners). |

Calyx has no hosted CI, no GitHub Actions gate, and no PR/push pipeline. Manual
Full State Verification on aiwonder is the release gate.

---

## 6. Fixtures / test utilities (`calyx-testkit`)

`crates/calyx-testkit/src/lib.rs` is the workspace's "reusable deterministic
test scaffolding." It depends on `calyx-core`, `proptest`, `rand`, `serde`,
`serde_json` (`crates/calyx-testkit/Cargo.toml`).

| Item | Kind | Provides |
|------|------|----------|
| `DEFAULT_TEST_SEED` (`0xCA1A_CAFE_D15C_1A11`) | const | deterministic RNG seed |
| `DEFAULT_TEST_TS` (`1_785_500_000`) | const | fixed test timestamp |
| `seeded_rng(seed) -> StdRng` | fn | deterministic RNG builder |
| `fixed_clock() -> FixedClock` | fn | standard fixed test clock |
| `slot_id_strategy()` | proptest strategy | stable `SlotId` |
| `cx_id_strategy()` | proptest strategy | stable `CxId` (16-byte) |
| `modality_strategy()` | proptest strategy | all `Modality` variants |
| `anchor_kind_strategy()` | proptest strategy | `AnchorKind` incl. labels |
| `absent_reason_strategy()` | proptest strategy | `AbsentReason` variants |
| `slot_vector_strategy()` | proptest strategy | small dense/absent `SlotVector` |
| `small_constellation_strategy()` | proptest strategy | small deterministic `Constellation` |

The crate's own tests (5 `#[test]`, including `proptest!` blocks) verify the
helpers are stable (RNG replay, fixed-clock equality, serde round-trips).

Note: only `calyx-oracle` declares a dependency on `calyx-testkit` in its
`Cargo.toml` (besides testkit itself). Most crates build local test fixtures
inline (e.g. the env-rooted durable-vault setup pattern in FSV tests) rather
than via testkit. Per-crate `tests/` directories also contain shared `support/`
and helper sub-modules (e.g. `crates/calyx-anneal/tests/support/`,
`crates/calyx-cli/tests/support/`, `crates/calyx-oracle/tests/support/`,
`crates/calyx-sextant/tests/issue640_support/`).

---

## 7. Coverage metrics

No coverage tooling is configured. A search for `tarpaulin`, `llvm-cov`, and
`grcov` across `*.toml`, `*.sh`, `*.yml`/`*.yaml`, and `*.md` returned no
matches. Coverage instrumentation: **Not determined from source (none found).**

The closest analogue to a coverage gate is the FSV "evidence" discipline plus
the bespoke gates in `scripts/check.sh` (orphan-`.rs` detection, manifest
coverage self-test), which assert that code/datasets are wired in — not
line/branch coverage in the conventional sense.
