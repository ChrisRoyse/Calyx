#!/usr/bin/env bash
# emit-sbom.sh — generate the Calyx CycloneDX 1.6 SBOM artifact and verify it.
# PRD 30 §2. Issue #596.
#
# Wraps emit-sbom.py with:
#   * a deterministic SOURCE_DATE_EPOCH (git commit time) so the SBOM is itself
#     reproducible;
#   * an INDEPENDENT cross-check (the FSV truth gate): re-read the written SBOM
#     and assert its component count equals the number of [[package]] entries in
#     Cargo.lock. The emitter must lose nothing.
#
# SoT: the SBOM JSON file on disk + Cargo.lock on disk (both read back here).
#
# Exit codes / fail-closed taxonomy:
#   0                              SBOM written and count cross-check passed
#   CALYX_SBOM_COUNT_MISMATCH (4)  SBOM component count != Cargo.lock packages
#   (2/3 propagate from emit-sbom.py: PARSE_ERROR / EMPTY)
#
# Usage: emit-sbom.sh [output.json]   (default: dist/sbom/calyx.cdx.json)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WS_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
LOCK="$WS_ROOT/Cargo.lock"
OUT="${1:-$WS_ROOT/dist/sbom/calyx.cdx.json}"
PY="${PYTHON:-python3}"

[ -f "$LOCK" ] || { echo "CALYX_SBOM_PARSE_ERROR: $LOCK not found" >&2; exit 2; }

# Deterministic timestamp from commit time (fall back to epoch 0, never wall clock).
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH="$(git -C "$WS_ROOT" log -1 --format=%ct 2>/dev/null || echo 0)"
fi
export SOURCE_DATE_EPOCH

# Root component version = workspace.package.version from the root Cargo.toml.
APP_VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$WS_ROOT/Cargo.toml" | head -1)"
APP_VERSION="${APP_VERSION:-0.1.0}"

"$PY" "$SCRIPT_DIR/emit-sbom.py" "$LOCK" \
  --output "$OUT" --app-name calyx --app-version "$APP_VERSION"

# --- Independent FSV cross-check ------------------------------------------------
# Source of truth #1: number of [[package]] headers in Cargo.lock.
LOCK_PKGS="$(grep -c '^\[\[package\]\]' "$LOCK")"
# Source of truth #2: components actually present in the written SBOM (re-read it).
SBOM_COMPS="$("$PY" - "$OUT" <<'PYEOF'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    print(len(json.load(fh)["components"]))
PYEOF
)"

echo "----------------------------------------------------------------"
echo "Cargo.lock [[package]] entries : $LOCK_PKGS"
echo "SBOM components written        : $SBOM_COMPS"
echo "SBOM path                      : $OUT"
echo "SOURCE_DATE_EPOCH              : $SOURCE_DATE_EPOCH"
echo "----------------------------------------------------------------"

if [ "$LOCK_PKGS" != "$SBOM_COMPS" ]; then
  echo "CALYX_SBOM_COUNT_MISMATCH: Cargo.lock=$LOCK_PKGS SBOM=$SBOM_COMPS" >&2
  exit 4
fi
echo "OK: every Cargo.lock package is represented in the SBOM ($SBOM_COMPS)."
