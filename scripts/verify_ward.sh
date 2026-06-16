#!/usr/bin/env bash
set -euo pipefail
source /home/croyse/calyx_env.sh
echo "=== fmt ==="
cargo fmt -p calyx-ward -- --check
echo "=== clippy ==="
cargo clippy -p calyx-ward --tests -- -D warnings 2>&1 | tail -5
echo "=== test (non-ignored) ==="
cargo test -p calyx-ward 2>&1 | grep -E "test result|error\[|warning: unused" | tail -40
echo "=== DONE verify_ward ==="
