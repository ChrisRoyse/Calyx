#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

source "$HOME/.cargo/env"
cd "$repo_root"

if [[ -f "$repo_root/env.sh" ]]; then
  source "$repo_root/env.sh"
fi

cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash "$repo_root/scripts/orphan_rs.sh"
bash "$repo_root/scripts/linecount.sh"
