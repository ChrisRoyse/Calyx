#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

source "$HOME/.cargo/env"
cd "$repo_root"

tmp_guard_baseline="$(mktemp -t calyx-check-tmp-baseline.XXXXXX)"
cleanup_tmp_guard() {
  local status=$?
  bash "$repo_root/scripts/tmp_scratch_guard.sh" post "$tmp_guard_baseline" || true
  rm -f "$tmp_guard_baseline"
  exit "$status"
}
trap cleanup_tmp_guard EXIT

if [[ -f "$repo_root/env.sh" ]]; then
  source "$repo_root/env.sh"
fi

# The gate is a one-shot build (no edit-rebuild loop), so incremental
# compilation only adds overhead and disk churn (its cache grew to ~61 GB on the
# shared build host). Disable it for the manual aiwonder gate; interactive dev
# keeps its own default. This project has no Actions/hosted CI gate.
export CARGO_INCREMENTAL=0

bash "$repo_root/scripts/tmp_scratch_guard.sh" pre "$tmp_guard_baseline"

cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

# Test execution with nextest: it runs every test across every binary in a
# single parallel pool sized to all logical CPUs, whereas `cargo test` runs each
# test binary sequentially and leaves most cores idle. With 1500+ tests across
# 250+ binaries that is the difference between saturating the box and waiting on
# one core. Fail-loud: if cargo-nextest is missing the gate errors (run
# scripts/aiwonder-build-setup.sh to provision it) rather than silently skipping.
if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "ERROR: cargo-nextest not installed. Run scripts/aiwonder-build-setup.sh" >&2
  exit 1
fi
cargo nextest run --workspace
# nextest does not run doctests; run them with the built-in harness so doc
# examples stay covered.
cargo test --workspace --doc

bash "$repo_root/scripts/orphan_rs.sh"
bash "$repo_root/scripts/linecount.sh"
# Dataset MANIFEST tooling (PH69 T01): synthetic known-I/O + edge battery in a
# temp root - fast, hermetic, and keeps the digest algorithm pinned.
bash "$repo_root/scripts/verify_dataset.sh" --self-test
# DATA BUILD_DONE coverage gate (PH69 T08): hermetic synthetic-MANIFEST
# battery pinning the 12 required (modality x outcome) cells.
bash "$repo_root/scripts/check_manifest_coverage.sh" --self-test
