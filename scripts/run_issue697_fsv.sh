#!/usr/bin/env bash
# PH70/#697 runtime injection-guard FSV on aiwonder (real ONNX lens, real corpus).
set -euo pipefail
source /home/croyse/calyx_env.sh
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
export CALYX_INJECTION_GUARD_FSV_DIR="/home/croyse/calyx/data/fsv-issue697-injection-onnx-${STAMP}"
# rc.12 ort CUDA binaries lack sm_120 (Blackwell) kernels -> explicit CPU EP for
# the capability proof. GPU acceleration tracked separately (sm_120 ort build).
export CALYX_INJECTION_GUARD_PROVIDER="${CALYX_INJECTION_GUARD_PROVIDER:-cpu}"
echo "PROVIDER=${CALYX_INJECTION_GUARD_PROVIDER}"
echo "FSV_DIR=${CALYX_INJECTION_GUARD_FSV_DIR}"
cargo test -p calyx-ward --test injection_guard_runtime_fsv -- --ignored --nocapture
echo "=== READBACK gates.json ==="
cat "${CALYX_INJECTION_GUARD_FSV_DIR}/gates.json"
echo "FSV_DIR=${CALYX_INJECTION_GUARD_FSV_DIR}"
