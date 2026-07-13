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

unset CALYX_GATE_TMPDIR CALYX_GATE_TMP_ROOT CALYX_GATE_TMP_ROOT_IDENTITY
unset CALYX_GATE_TMP_IDENTITY CALYX_GATE_LOCK_PATH CALYX_GATE_LOCK_FD
unset CALYX_GATE_TARGET_ROOT CALYX_GATE_TARGET_ROOT_IDENTITY
unset CALYX_GATE_TARGET_DIR CALYX_GATE_TARGET_IDENTITY CARGO_TARGET_DIR TMPDIR
echo "fuzz-gate-fsv: repo=$repo_root"
echo "fuzz-gate-fsv: standalone gate owns TMPDIR and CARGO_TARGET_DIR"

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
