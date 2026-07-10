#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/../.." && pwd -P)"
cd "$repo_root"

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi
if [[ -f "$repo_root/env.sh" ]]; then
  # shellcheck source=/dev/null
  source "$repo_root/env.sh"
fi

scratch_root="${CALYX_TARGET_SCRATCH_ROOT:-$(dirname "$repo_root")/target-scratch}"
export CALYX_TARGET_SCRATCH_ROOT="$scratch_root"
mkdir -p "$scratch_root"
target_dir="$(mktemp -d "$scratch_root/issue1400-fuzz-gate-fsv.XXXXXX")"
cleanup() {
  local status=$?
  local cleanup_status=0
  bash "$repo_root/scripts/cargo-target-clean.sh" \
    --repo-root "$repo_root" \
    --target-dir "$target_dir" \
    --also-workspace-target \
    --also-repo-targets \
    --also-calyx-home-target || cleanup_status=$?
  if [[ "$status" -eq 0 && "$cleanup_status" -ne 0 ]]; then
    exit "$cleanup_status"
  fi
  exit "$status"
}
trap cleanup EXIT

export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$target_dir"
echo "fuzz-gate-fsv: repo=$repo_root"
echo "fuzz-gate-fsv: CARGO_TARGET_DIR=$CARGO_TARGET_DIR"

corpus_minimum() {
  local target="$1" minimum="$2"
  local dir="fuzz/corpus/$target"
  if [[ ! -d "$dir" ]]; then
    echo "CALYX_FUZZ_CORPUS_MISSING: $dir" >&2
    exit 1
  fi
  local count bytes
  count="$(find "$dir" -type f | wc -l | tr -d ' ')"
  bytes="$(find "$dir" -type f -printf '%s\n' | awk '{s += $1} END {print s + 0}')"
  echo "fuzz-gate-fsv: corpus=$target files=$count bytes=$bytes minimum=$minimum"
  if [[ "$count" -lt "$minimum" ]]; then
    echo "CALYX_FUZZ_CORPUS_TOO_SMALL: $target has $count files, expected at least $minimum" >&2
    exit 1
  fi
}

corpus_minimum aster_manifest_decode 3
corpus_minimum aster_sst_decode 10
corpus_minimum aster_wal_replay 10
corpus_minimum lens_output_decode 7
corpus_minimum mcp_jsonrpc_decode 5
corpus_minimum query_parse 4

python3 - <<'PY'
import json
import pathlib
import sys

manifest_dir = pathlib.Path("fuzz/corpus/aster_manifest_decode")
valid = []
for path in sorted(manifest_dir.iterdir()):
    if not path.is_file():
        continue
    try:
        data = json.loads(path.read_bytes())
    except Exception:
        continue
    version = data.get("version") or {}
    if (
        version.get("major") == 1
        and "manifest_seq" in data
        and "durable_seq" in data
        and "panel_ref" in data
    ):
        valid.append(path.name)

print("fuzz-gate-fsv: valid_current_manifest_seeds=" + ",".join(valid))
if len(valid) < 2:
    print("CALYX_FUZZ_MANIFEST_REAL_SEEDS_MISSING", file=sys.stderr)
    sys.exit(1)
PY

bash "$repo_root/scripts/fuzz-gate.sh" --repo-root "$repo_root"
bash "$repo_root/scripts/linecount.sh" --repo-root "$repo_root"
bash "$repo_root/scripts/orphan_rs.sh" --repo-root "$repo_root"
