#!/usr/bin/env bash
set -euo pipefail

CALYX_HOME="${CALYX_HOME:-/home/croyse/calyx}"
CALYX_BIN="${CALYX_BIN:-$CALYX_HOME/target/release/calyx}"
CALYX_HEALTH_LOG_PATH="${CALYX_HEALTH_LOG_PATH:-/zfs/hot/logs/calyx-health/latest.json}"
CALYX_SECRET_ENV="${CALYX_SECRET_ENV:-/run/leapable/secrets/calyx.env}"
CALYX_HEALTH_WAIT_SECS="${CALYX_HEALTH_WAIT_SECS:-30}"
CALYX_HEALTH_VAULT="${CALYX_HEALTH_VAULT:-$CALYX_HOME/data/vault}"
CALYX_HEALTH_METRICS_URL="${CALYX_HEALTH_METRICS_URL:-http://127.0.0.1:7700/metrics}"

args=(
  healthcheck
  --wait "$CALYX_HEALTH_WAIT_SECS"
  --out "$CALYX_HEALTH_LOG_PATH"
  --secret-env "$CALYX_SECRET_ENV"
  --calyx-home "$CALYX_HOME"
  --require-env HF_HUB_TOKEN
  --require-env HF_TOKEN
)

if [[ -n "${CALYX_HEALTH_VAULT:-}" ]]; then
  args+=(--vault "$CALYX_HEALTH_VAULT")
fi

if [[ -n "${CALYX_HEALTH_METRICS_URL:-}" ]]; then
  args+=(--metrics-url "$CALYX_HEALTH_METRICS_URL")
fi

exec "$CALYX_BIN" "${args[@]}"
