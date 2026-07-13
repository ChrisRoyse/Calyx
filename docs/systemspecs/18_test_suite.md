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
integration tests (in the crate's `tests/` directory). There are also **19**
`#[tokio::test]` functions, all in `calyx-web-api`.

| Crate | `#[test]` in `src/` (unit) | `#[test]` in `tests/` (integration) | Total |
|-------|---------------------------:|------------------------------------:|------:|
| calyx-anneal | 2 | 319 | 321 |
| calyx-assay | 151 | 110 | 261 |
| calyx-aster | 660 | 59 | 719 |
| calyx-buildinfo | 13 | 0 | 13 |
| calyx-cli | 917 | 87 | 1004 |
| calyx-core | 82 | 3 | 85 |
| calyxd | 130 | 34 | 164 |
| calyx-forge | 231 | 75 | 306 |
| calyx-fsv | 8 | 0 | 8 |
| calyx-hazard-soak | 12 | 0 | 12 |
| calyx-leapable | 18 | 14 | 32 |
| calyx-ledger | 59 | 49 | 108 |
| calyx-lodestar | 6 | 206 | 212 |
| calyx-loom | 35 | 32 | 67 |
| calyx-mcp | 98 | 17 | 115 |
| calyx-mincut | 10 | 15 | 25 |
| calyx-oracle | 143 | 5 | 148 |
| calyx-paths | 1 | 11 | 12 |
| calyx-poly | 116 | 136 | 252 |
| calyx-registry | 224 | 32 | 256 |
| calyx-search | 72 | 2 | 74 |
| calyx-sextant | 129 | 208 | 337 |
| calyx-testkit | 10 | 1 | 11 |
| calyx-ward | 41 | 179 | 220 |
| calyx-web-api | 14 | 3 | 17 |
| **TOTAL** | **3182** | **1597** | **4779** |

- **Total `#[test]` functions: 4779.**
- **Total `#[tokio::test]` functions: 19.**
- 22 of the 25 crates have a `tests/` integration directory
  (`crates/*/tests/`).

Note: a `#[test]` count overcounts the runnable test total slightly because
some tests are wrapped in `proptest! { #[test] fn ... }` blocks and some are
generated. These are attribute occurrences, not post-expansion test cases.

`tests/` directories present (22 crates): calyx-anneal, calyx-assay,
calyx-aster, calyx-cli, calyx-core, calyxd, calyx-forge, calyx-leapable,
calyx-ledger, calyx-lodestar, calyx-loom, calyx-mcp, calyx-mincut,
calyx-oracle, calyx-paths, calyx-poly, calyx-registry, calyx-search,
calyx-sextant, calyx-testkit, calyx-ward, calyx-web-api.

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
and then read authoritative Calyx/Aster rows back from the database. Diagnostic
files such as JSON logs, hex/`xxd` dumps, reports, and shell transcripts may be
emitted under an FSV "root" directory, but they are not the source of truth and
cannot admit an association, gate, roster, phase exit, or provenance claim.
The README repeatedly states that components are "FSV-signed-off on aiwonder"
and lists concrete FSV root paths such as `/home/croyse/calyx/data/fsv-stage5-...`
(`README.md`).

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

- **FSV evidence roots are absolute-only.** All readers resolve `CALYX_FSV_ROOT`
  through the `calyx-fsv` crate: unset means the test owns a temp root; a set
  value must be an absolute path or the run fails closed with
  `CALYX_FSV_ROOT_NOT_ABSOLUTE`/`CALYX_FSV_ROOT_EMPTY` (value, cwd, and
  remediation in the message). Rationale: cargo runs every test from the crate
  root, not the workspace root, so relative roots scatter evidence under
  `crates/<name>/...` while readback looks at the workspace (`#1014`).
  `scripts/check.sh` additionally refuses to run with a suite-wide
  `CALYX_FSV_ROOT` set, because independent FSV tests would collide in one
  shared evidence root.

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
bash scripts/cargo-fmt-workspace.sh --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace      # primary parallel test run
cargo test --workspace --doc       # doctests (nextest does not run these)
```

- `scripts/check.sh` exports `CARGO_INCREMENTAL=0`, fails loudly if
  `cargo-nextest` is missing, and points to the focused provisioning command:
  `bash scripts/install-cargo-nextest.sh` for Bash/WSL or
  `pwsh -File scripts/install-cargo-nextest.ps1` for native Windows
  PowerShell. It uses `scripts/cargo-fmt-workspace.sh` for formatting so the
  workspace gate checks one package at a time instead of expanding the whole
  workspace into one long rustfmt command. On native Windows, use
  `pwsh -File scripts/cargo-fmt-workspace.ps1` for the same package-by-package
  check, or add `-Write` to apply formatting. It then additionally runs
  `scripts/orphan_rs.sh`, `scripts/linecount.sh`,
  `scripts/verify_dataset.sh --self-test`, and
  `scripts/check_manifest_coverage.sh --self-test`. Before Cargo, it runs the
  synthetic `check-tmpdir-regression.sh` and
  `gate-wrapper-tmpdir-regression.sh` batteries against the real gate scripts.
- Each top-level `check.sh` / public gate and each standalone fuzz gate owns one
  unique `TMPDIR` under the Calyx-home `tmp/` directory; nested fuzz inherits
  the validated outer state. Every stage and nested test uses that directory. The gate validates that
  the resolved run root is a current-user-owned direct `check.*` child of a
  group/world-nonwritable stable parent. Creation publishes the opened root and
  entry device/inode identities through a private unlinked FD before the parent
  exposes ownership. Cleanup passes those identities to the FD-anchored remover,
  which uses `openat2` beneath the authorization root, `renameat2(RENAME_NOREPLACE)`
  quarantine, and descriptor-relative traversal without crossing symlinks,
  magic links, or mount points. Cargo targets use the same create/publication and
  removal contracts. Cargo-bearing gates hold a current-user regular OFD
  `tmp/.cargo-gate.lock` with no group/world write bits (created mode 0600) across target publication, every Cargo stage, fallback
  target sweeps, and final deletion; concurrent invocations keep distinct run
  TMPDIRs but serialize their Cargo lifecycles. Each stage runs in a unique
  systemd user scope beneath one per-run slice. Nested wrappers must prove their
  owner PID start identity and physical membership beneath that same slice, so
  `setsid` and double-fork descendants cannot escape HUP/INT/TERM or normal-exit
  draining. Cleanup and exact slice-release failures return containment status
  125. Destructive target enumeration uses private verified NUL snapshots;
  cleanup prediagnostics sample at most one top-level entry, and producer errors
  fail closed. The shared-`/tmp` baseline is bound by device/inode, byte size,
  and SHA-256; that guard remains recovery, not primary lifecycle.
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
| `scripts/check.sh` | The manual aiwonder per-merge gate: binds every stage to one run-owned Calyx `TMPDIR`, then runs rustfmt check, `cargo check --workspace --all-targets`, `cargo clippy ... -D warnings`, `cargo nextest run --workspace`, `cargo test --workspace --doc`, `scripts/orphan_rs.sh`, `scripts/linecount.sh`, and dataset/manifest self-tests; exact temp and Cargo cleanup is attempted on every exit, with rc 125 and recovery state retained on cleanup failure. |
| `scripts/fuzz-gate.sh` | Formats, checks, and lints the fuzz workspace under inherited validated gate state or standalone run-owned temp/Cargo roots, with identity-bound final cleanup. |
| `scripts/public-mirror-gate.sh` | Runs the public checkout gate from dev tooling under run-owned temp/Cargo roots and removes only the exact known public-scratch leaf, including a top-level symlink without following its referent. |
| `scripts/check-tmpdir.sh` | Creates temp/Cargo scratch children through a pinned private-staging helper and publishes opened root/entry identities; ordinary failures clean the exact identity, while pathname changes return 125 with recovery state retained. Cleanup accepts only the expected direct child identity and emits bounded before/absence readback. |
| `scripts/gate-runtime-state.sh` / `gate_ofd_lock_query.py` | Source-only path, atomic identity-capture, helper-pinning, and non-mutating exact-OFD lock proof contracts. |
| `scripts/gate-stage-runner.sh` / `gate-cgroup-runtime.sh` / `gate-cgroup-scope.sh` | One unique scope per stage beneath an identity-checked per-run slice, PID-bound pre-publication capture, inherited-wrapper membership validation, bounded whole-cgroup HUP/INT/TERM or false-success drain, and exact slice release. |
| `scripts/gate_safe_rmtree.py` | Linux FD-anchored removal primitive: validates lock/root/entry identities and policy, quarantines with no-replace rename, recursively removes through descriptors, refuses symlink/magic-link/mount traversal, validates Cargo's canonical cache marker, bounds deferred signals, and reports rollback or exact recovery quarantine on partial failure. |
| `scripts/tests/check-tmpdir-regression.sh` / `gate-wrapper-tmpdir-regression.sh` and their scenario modules | Mutation-sensitive batteries for shared run roots, exact failure statuses, create-identity fail/swap/HUP/INT/TERM, cgroup escape/drain and metadata failure, path/mode/UID/mount/identity substitution, OFD ownership, baseline tampering, FD-removal rollback/recovery, producer-failure enumeration, serialized concurrency, nested wrappers, public-origin/FSV boundaries, top-level public-scratch symlink containment, explicit/generated target setup, and green-workload final-clean failure propagation across check/fuzz/public. |
| `scripts/cargo-fmt-workspace.sh` / `.ps1` | Workspace rustfmt gate that enumerates `cargo metadata` workspace packages and runs `cargo fmt -p <package>` one package at a time. This is the native Windows-safe format check for long worktree paths; it fails on the first unformatted package and prints the exact package command. |
| `scripts/install-cargo-nextest.sh` / `.ps1` | Idempotent local provisioning for the required `cargo-nextest` subcommand on Bash/WSL and native Windows PowerShell. |
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

The crate's own tests (11 `#[test]`, including `proptest!` blocks) verify the
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
