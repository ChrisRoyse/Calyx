#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
echo "=== fmt ==="
cargo fmt -p calyx-sextant -p calyx-cli -- --check && echo FMT_OK || { cargo fmt -p calyx-sextant -p calyx-cli; echo auto-formatted; }
echo "=== clippy sextant ==="
cargo clippy -p calyx-sextant --lib -- -D warnings 2>&1 | tail -3
echo "=== clippy cli ==="
cargo clippy -p calyx-cli --bin calyx -- -D warnings 2>&1 | tail -3
echo "=== DONE ==="
