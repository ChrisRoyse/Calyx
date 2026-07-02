# Issue #1108 — Runner binary identity and verified deploy

## Problem

During the #1096 FSV (2026-07-01) the deployed runner binary
`/home/croyse/calyx/target/release/calyx` turned out to be built 2026-06-27:
no `weave-loom` command, none of the #1058/#1060/#1096/#1101 readback fixes.
Anyone driving probes through the runner path silently got pre-#1058
behavior for four days.

## Root cause

`/home/croyse/calyx/target/release/` is a hand-managed deploy directory (the
`calyxd.service` unit and the healthcheck wrapper execute from it), while
builds land in `~/calyx/repo/target/release/`. Nothing copied fresh builds
across, and the binaries carried no self-identity, so the only staleness
signal was file mtime — which nothing checked.

## Fix

Two halves: binaries that know what they are, and a deploy path that refuses
to install anything unverified.

### Embedded build identity (`crates/calyx-buildinfo`)

Each deployed binary crate (`calyx-cli`, `calyxd`, `calyx-mcp`) gains a
`build.rs` calling `calyx_buildinfo::emit()`, which embeds:

- `git_sha` — full 40-hex commit SHA (`git rev-parse HEAD`)
- `git_dirty` — tracked-files dirty flag at build-script time
- `git_commit_unix_secs` — committer timestamp (deterministic per SHA; the
  build wall clock is deliberately not embedded so rebuilding identical
  sources does not churn)

There is no fallback: building outside a usable git checkout fails with
`CALYX_BUILD_INFO_GIT_UNAVAILABLE`. A deployed Calyx binary can no longer
exist without a verifiable identity.

Surfaces:

- `calyx build-info` — JSON report (also `--help` usage entry)
- `calyxd --build-info`, `calyx-mcp --build-info` — same JSON schema
- `calyx healthcheck` (`latest.json`) now records a `binary` block, so every
  healthcheck run and every `calyxd.service` start logs which SHA answered

### Verified deploy (`infra/aiwonder/bin/deploy-calyx-runner.sh`)

Gates, in order, each failing loudly with a `CALYX_DEPLOY_*` code:

1. `CALYX_DEPLOY_REPO_STALE` / `CALYX_DEPLOY_REPO_DIRTY` — repo must be
   clean and exactly at a freshly fetched `origin/main`
2. `CALYX_DEPLOY_INGEST_ACTIVE` — refuses while `calyx ingest`,
   `__ingest-lens-worker`, or longrun FSV supervisors are running
3. `CALYX_DEPLOY_BINARY_IN_USE` — refuses to replace a binary a process is
   executing unless `--allow-in-use` (the swap is a same-filesystem staged
   rename, so it is inode-safe either way; the flag exists so replacing a
   live service binary is an explicit decision)
4. `CALYX_DEPLOY_IDENTITY_MISMATCH` — the binary must self-report the
   expected `git_sha` three times: after build, after staging, and after the
   final rename (`calyx` additionally goes through
   `scripts/build-verified-calyx.sh` for env/ELF RUNPATH verification)
5. `<binary>.deploy.json` manifest written next to the binary and read back

A running `calyxd` keeps its pre-deploy inode after a swap; the script
prints an explicit NOTE that `systemctl restart calyxd` is required to adopt
the new SHA.

## Staleness detection going forward

- `jq .binary.git_sha /zfs/hot/logs/calyx-health/latest.json` vs
  `git -C ~/calyx/repo rev-parse origin/main` answers "is the runner stale"
  in one line.
- `~/calyx/target/release/<binary>.deploy.json` records what was deployed,
  from where, and when.
- Any binary that cannot answer `build-info` predates #1108 and is stale by
  definition.

## Tests

- `crates/calyx-buildinfo/src/tests.rs` — identity computation against the
  real checkout; validation rejects malformed SHA/dirty/timestamp values;
  non-checkout directories error with `CALYX_BUILD_INFO_GIT_UNAVAILABLE`.
- `crates/calyx-cli/tests/issue1108_build_info_fsv.rs` — runs the real
  built binary: JSON matches the checkout HEAD, extra arguments are
  rejected, `--help` prints usage.
- `crates/calyx-cli/src/healthcheck_tests.rs` — `latest.json` now carries
  the `binary` identity block matching the checkout.
