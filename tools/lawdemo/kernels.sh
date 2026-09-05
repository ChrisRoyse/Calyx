#!/usr/bin/env bash
# Rebuild the three judge sub-vault kernels (Boyle 8055, Keough 8108, Kilbane 8109)
# for the Cuyahoga law demo, then restore the production resident. Fully
# parameterized: no hardcoded binaries/vaults. Every path is an env-overridable
# default that points at the current (issue #1955) production layout.
#
# Resident dance uses the GPU reservation broker (`calyx gpu run`) one resident
# at a time on the single GPU, matching the issue #1955 endgame recipe.
set -Eeuo pipefail
umask 002

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)

CALYX_HOME=${CALYX_HOME:-/home/${USER}/calyx}
FSV_HOME=${LAWDEMO_FSV_HOME:-$CALYX_HOME/fsv}
B=${LAWDEMO_BINARY:-$CALYX_HOME/repo/target/release/calyx}
ENV_FILE=${LAWDEMO_ENV_FILE:-$FSV_HOME/issue1837-weave-weight-bfe31521-20260716T210747Z/source/env.sh}
ORT_DYLIB_PATH=${ORT_DYLIB_PATH:-$CALYX_HOME/vendor/onnxruntime-v1.26.0/build/Linux/Release/libonnxruntime.so}
RESIDENT_ADDR=${LAWDEMO_RESIDENT_ADDR:-127.0.0.1:18401}
OWNER=${LAWDEMO_GPU_OWNER:-issue1955}
Q=${LAWDEMO_ADMISSION_QUERIES:-$SCRIPT_DIR/ohio_admission_queries.v1.jsonl}

# Output/work root for readback artifacts.
R=${LAWDEMO_KERNELS_OUT:-$FSV_HOME/issue1955-regen/judge-rebuild}

# Production vault to restore when done (vault-mode, 9 content lenses).
PROD_V=${LAWDEMO_VAULT:-$CALYX_HOME/vaults/01KXZZH7EA4B4BAMPC434X84Q4}
PROD_READY=${LAWDEMO_PROD_READY:-$FSV_HOME/issue1955-endgame/prod-resident/ready.json}
PROD_PROGRESS=${LAWDEMO_PROD_PROGRESS:-$FSV_HOME/issue1955-endgame/prod-resident/progress.jsonl}
PROD_SERVE_LOG=${LAWDEMO_PROD_SERVE_LOG:-$FSV_HOME/issue1955-endgame/prod-resident/serve.log}

# Judge sub-vault ids (person_id -> vault id). Override via LAWDEMO_JUDGE_<pid>_VAULT.
J8055_VAULT=${LAWDEMO_JUDGE_8055_VAULT:-01KY068KMQ9PJQVPVKY17KK8VR}
J8108_VAULT=${LAWDEMO_JUDGE_8108_VAULT:-01KY14EAZBMFDWCVAH4XJBYG49}
J8109_VAULT=${LAWDEMO_JUDGE_8109_VAULT:-01KY14RT272WGFRY00BA4K8F3B}

RESIDENT_MIB=${LAWDEMO_RESIDENT_MIB:-8000}
MAX_RESIDENT_VRAM_MIB=${LAWDEMO_MAX_RESIDENT_VRAM_MIB:-22528}
MAX_LOAD_SECS=${LAWDEMO_MAX_LOAD_SECS:-120}
MAX_RUNTIME_BATCH=${LAWDEMO_MAX_RUNTIME_BATCH:-4}
EXPECTED_CONTENT_LENSES=${LAWDEMO_CONTENT_LENSES:-9}

export CALYX_HOME ORT_DYLIB_PATH CUDA_VISIBLE_DEVICES=${CUDA_VISIBLE_DEVICES:-0}
[[ -f "$ENV_FILE" ]] && source "$ENV_FILE"
mkdir -p "$R"

vault_dir() { printf "%s/vaults/%s" "$CALYX_HOME" "$1"; }

wait_reservations_clear() {
  for _ in $(seq 1 120); do
    if "$B" gpu reservations 2>/dev/null | jq -e ".reserved_mib == 0" >/dev/null; then
      return 0
    fi
    sleep 1
  done
  echo "GPU reservations did not clear" >&2; return 1
}

wait_ready() {
  local file=$1 vault_id=$2
  for _ in $(seq 1 240); do
    if test -s "$file" && jq -e --arg n "$EXPECTED_CONTENT_LENSES" --arg id "$vault_id" \
      '.ready == true and .content_lens_count == ($n|tonumber) and .gpu_content_lens_count == ($n|tonumber) and .cpu_content_lens_count == 0 and (.template_source | contains($id))' \
      "$file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "resident not ready: $file" >&2; return 1
}

serve_vault() {
  # serve_vault <vault_dir> <ready_out> <progress_out> <serve_log>
  local vdir=$1 ready=$2 progress=$3 log=$4
  rm -f "$ready"
  setsid nohup "$B" gpu run --mib "$RESIDENT_MIB" --job legal-resident --owner "$OWNER" -- \
    "$B" panel resident serve --vault "$vdir" --home "$CALYX_HOME" --bind "$RESIDENT_ADDR" \
    --ready-out "$ready" --progress-out "$progress" \
    --max-resident-vram-mib "$MAX_RESIDENT_VRAM_MIB" --max-load-secs "$MAX_LOAD_SECS" \
    --max-runtime-batch "$MAX_RUNTIME_BATCH" \
    > "$log" 2>&1 < /dev/null &
}

stop_resident() {
  "$B" panel resident stop --addr "$RESIDENT_ADDR" >/dev/null 2>&1 || true
  wait_reservations_clear
}

restore_production() {
  stop_resident
  mkdir -p "$(dirname "$PROD_READY")"
  serve_vault "$PROD_V" "$PROD_READY" "$PROD_PROGRESS" "$PROD_SERVE_LOG"
  wait_ready "$PROD_READY" "$(basename "$PROD_V")"
  "$B" panel resident ready --addr "$RESIDENT_ADDR" --out "$R/production-resident-restored.json"
  date -u +%FT%TZ > "$R/production-resident-restored.complete"
}

on_exit() {
  local rc=$?
  trap - EXIT
  restore_production || rc=99
  exit "$rc"
}
trap on_exit EXIT

stop_resident

for spec in "8055:$J8055_VAULT" "8108:$J8108_VAULT" "8109:$J8109_VAULT"; do
  P=${spec%%:*}; ID=${spec##*:}
  V=$(vault_dir "$ID")
  READY="$R/judge-$P-resident-ready.json"
  PROGRESS="$R/judge-$P-resident-progress.jsonl"
  SERVE_LOG="$R/judge-$P-serve.log"
  [[ -d "$V" ]] || { echo "judge $P vault missing: $V" >&2; exit 1; }
  serve_vault "$V" "$READY" "$PROGRESS" "$SERVE_LOG"
  wait_ready "$READY" "$ID"
  "$B" kernel-build "$V" --admission-queries "$Q" --resident-addr "$RESIDENT_ADDR" \
    > "$R/judge-$P-kernel-build.json" 2> "$R/judge-$P-kernel-build.stderr"
  K=$(jq -r ".kernel.kernel_id" "$R/judge-$P-kernel-build.json")
  S=$(jq -r ".artifacts.build_ledger_seq" "$R/judge-$P-kernel-build.json")
  "$B" readback kernel-health --root "$V" --kernel-id "$K" > "$R/judge-$P-kernel-health.json"
  "$B" readback --ledger "$V" --seq "$S" > "$R/judge-$P-kernel-ledger-readback.json"
  "$B" verify-chain --vault "$V" > "$R/judge-$P-verify-after-kernel.json"
  sha256sum "$R/judge-$P-kernel-health.json" > "$R/judge-$P-kernel-health.sha256"
  echo "$K" > "$R/judge-$P-kernel-id.txt"
  stop_resident
done

date -u +%FT%TZ > "$R/kernels-all.complete"
