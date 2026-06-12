#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

source "$HOME/.cargo/env"
cd "$repo_root"

if [[ -f "$repo_root/env.sh" ]]; then
  source "$repo_root/env.sh"
fi

# The gate is a one-shot build (no edit-rebuild loop), so incremental
# compilation only adds overhead and disk churn (its cache grew to ~61 GB on the
# shared build host). Disable it for the gate; interactive dev keeps its own
# default. Standard CI hygiene, and portable (also applies if Actions re-enabled).
export CARGO_INCREMENTAL=0

cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash "$repo_root/scripts/orphan_rs.sh"
bash "$repo_root/scripts/linecount.sh"
