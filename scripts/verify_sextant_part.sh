#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
echo "=== fmt ==="
cargo fmt -p calyx-sextant -- --check && echo FMT_OK || { cargo fmt -p calyx-sextant; echo "auto-formatted"; }
echo "=== clippy sextant ==="
cargo clippy -p calyx-sextant --lib -- -D warnings 2>&1 | tail -6
echo "=== DONE ==="
