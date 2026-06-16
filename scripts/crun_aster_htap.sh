#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
cargo test -p calyx-aster --lib htap 2>&1 | tail -30
