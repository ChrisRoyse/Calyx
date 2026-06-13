#!/usr/bin/env bash
set -euo pipefail

# Operator/sudo step for PH66/#603. It materializes the Calyx secret env map
# entry and installs the health wrapper used by leapable-aiwonder-healthcheck.
# It never prints secret values.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
map_fragment="$repo_root/infra/aiwonder/secrets-loader/calyx.env.map.json"
wrapper_src="$repo_root/infra/aiwonder/bin/calyx-aiwonder-healthcheck.sh"
live_map="${LEAPABLE_SECRETS_MAP:-/etc/leapable/secrets-map.json}"
source_map="${LEAPABLE_SOURCE_SECRETS_MAP:-/home/croyse/leapablememory/infra/aiwonder/secrets-loader/secrets-map.json}"
health_script="${LEAPABLE_HEALTHCHECK_SCRIPT:-/usr/local/sbin/leapable-aiwonder-healthcheck.sh}"
wrapper_dst="${CALYX_HEALTHCHECK_WRAPPER:-/usr/local/sbin/calyx-aiwonder-healthcheck.sh}"
backup_suffix="${CALYX_WIRING_BACKUP_SUFFIX:-$(date -u +%Y%m%dT%H%M%SZ)}"

require_file() {
  [[ -f "$1" ]] || {
    echo "missing required file: $1" >&2
    exit 1
  }
}

require_file "$map_fragment"
require_file "$wrapper_src"
require_file "$health_script"

install_wrapper() {
  sudo install -o root -g root -m 0755 "$wrapper_src" "$wrapper_dst"
}

merge_map() {
  local dst="$1"
  if sudo test -f "$dst"; then
    sudo cp -a "$dst" "$dst.calyx603.$backup_suffix.bak"
    tmp="$(mktemp)"
    sudo jq -S -s '.[0] * .[1]' "$dst" "$map_fragment" > "$tmp"
    sudo install -o root -g root -m 0644 "$tmp" "$dst"
    rm -f "$tmp"
  fi
}

patch_health_script() {
  if sudo grep -q 'BEGIN CALYX HEALTHCHECK' "$health_script"; then
    return
  fi
  sudo cp -a "$health_script" "$health_script.calyx603.$backup_suffix.bak"
  tmp="$(mktemp)"
  sudo awk -v wrapper="$wrapper_dst" '
    /^FINISHED_AT=/ && !inserted {
      print "# BEGIN CALYX HEALTHCHECK"
      print "run_check \"calyx_secret_env_rendered\" bash -lc '\''test -f /run/leapable/secrets/calyx.env && test \"$(stat -c %a /run/leapable/secrets/calyx.env)\" = \"400\" && grep -Eq \"^HF_HUB_TOKEN=\" /run/leapable/secrets/calyx.env && grep -Eq \"^HF_TOKEN=\" /run/leapable/secrets/calyx.env'\''"
      print "run_check \"calyx_healthcheck_latest_json\" \"" wrapper "\""
      print "run_check \"calyx_health_latest_status\" bash -lc '\''jq -e \".status == \\\"pass\\\"\" /zfs/hot/logs/calyx-health/latest.json >/dev/null'\''"
      print "# END CALYX HEALTHCHECK"
      inserted=1
    }
    { print }
  ' "$health_script" > "$tmp"
  sudo install -o root -g root -m 0755 "$tmp" "$health_script"
  rm -f "$tmp"
}

install_wrapper
merge_map "$live_map"
merge_map "$source_map"
patch_health_script

echo "calyx deploy wiring installed: wrapper=$wrapper_dst map_fragment=calyx.env"
