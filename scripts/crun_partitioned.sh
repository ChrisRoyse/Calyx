#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
cargo test -p calyx-sextant --lib partitioned -- --nocapture 2>&1 | tail -35
