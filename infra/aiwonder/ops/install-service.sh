#!/usr/bin/env bash
# install-service.sh — install + start calyxd.service (PH66 T01, issue #536).
#
# This script REQUIRES sudo (passwordless-sudo constraint: 01 §3). Run it as the
# operator on aiwonder — never via cargo test or agent automation. It is
# idempotent: re-running copies the current unit, stops any running instance,
# and restarts cleanly (it never leaves two calyxd instances on port 7700).
set -euo pipefail

UNIT_SRC="$(dirname "$0")/../systemd/calyxd.service"
UNIT_DST="/etc/systemd/system/calyxd.service"

# Pre-flight: ensure calyxd binary exists
CALYXD_BIN="/home/croyse/calyx/target/release/calyxd"
[[ -x "$CALYXD_BIN" ]] || {
  echo "ERROR: calyxd not built at $CALYXD_BIN"
  exit 1
}

# Pre-flight: no existing process on port 7700
if ss -tlnp | grep -q ':7700'; then
  echo "WARNING: port 7700 already in use; stopping existing calyxd"
  sudo systemctl stop calyxd || true
fi

sudo cp "$UNIT_SRC" "$UNIT_DST"
sudo chmod 644 "$UNIT_DST"
sudo systemctl daemon-reload
sudo systemctl enable calyxd
sudo systemctl start calyxd
sleep 5
sudo systemctl is-active calyxd
echo "calyxd.service installed and active"
