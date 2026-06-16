#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
echo "=== fmt ==="
cargo fmt -p calyx-aster -p calyx-cli -- --check && echo FMT_OK || { echo FMT_FAIL; cargo fmt -p calyx-aster -p calyx-cli; echo "auto-formatted"; }
echo "=== clippy aster ==="
cargo clippy -p calyx-aster --lib -- -D warnings 2>&1 | tail -4
echo "=== clippy cli ==="
cargo clippy -p calyx-cli --bin calyx -- -D warnings 2>&1 | tail -4
echo "=== test aster htap ==="
cargo test -p calyx-aster --lib htap 2>&1 | grep -E "test result|error" | tail -5
echo "=== DONE verify_587 ==="
