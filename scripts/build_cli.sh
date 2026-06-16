#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
cargo build -q -p calyx-cli 2>&1 | grep -E "error|warning: unused" | head -30
echo "=== build_cli exit: done ==="
ls -la target/debug/calyx
