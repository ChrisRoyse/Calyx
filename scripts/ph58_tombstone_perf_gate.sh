#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
root="${CALYX_FSV_ROOT:-$repo_root/target/fsv/issue1048-ph58-tombstone-perf/$stamp}"
stdout_log="$root/runner.stdout.log"
stderr_log="$root/runner.stderr.log"
build_stdout_log="$root/build.stdout.log"
build_stderr_log="$root/build.stderr.log"
exit_marker="$root/runner.exit"
context_file="$root/runner-context.txt"
competing_file="$root/preflight-competing-builds.txt"

mkdir -p "$root"

if ! command -v cargo >/dev/null 2>&1 && [ -r "$HOME/.cargo/env" ]; then
  # Non-login shells launched by automation do not always inherit rustup's PATH.
  . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: CALYX_PH58_CARGO_NOT_FOUND cargo is not on PATH after loading ~/.cargo/env" >&2
  echo 127 >"$exit_marker"
  exit 127
fi

{
  printf 'PH58_TOMBSTONE_PERF_FSV_ROOT=%s\n' "$root"
  printf 'UTC=%s\n' "$stamp"
  printf 'REPO_ROOT=%s\n' "$repo_root"
  printf 'HEAD=%s\n' "$(git -C "$repo_root" rev-parse --verify HEAD)"
  printf 'STATUS_BEGIN\n'
  git -C "$repo_root" status --short --branch
  printf 'STATUS_END\n'
  printf 'UNAME=%s\n' "$(uname -a)"
  printf 'NPROC=%s\n' "$(nproc)"
  printf 'LOADAVG=%s\n' "$(cat /proc/loadavg)"
} >"$context_file"

if pgrep -af 'cargo nextest|cargo test|rustc' >"$competing_file"; then
  echo "ERROR: CALYX_PH58_COMPETING_BUILD_PROCESS active cargo/rustc process found before serial perf gate" >&2
  cat "$competing_file" >&2
  echo 2 >"$exit_marker"
  exit 2
fi

export CALYX_FSV_ROOT="$root"
export CALYX_PH58_TOMBSTONE_PERF_GATE=1
export CALYX_PH58_MAX_HOST_LOAD_PER_CPU="${CALYX_PH58_MAX_HOST_LOAD_PER_CPU:-1.0}"
export CALYX_PH58_MAX_P99_RATIO="${CALYX_PH58_MAX_P99_RATIO:-2.0}"

set +e
cargo test -p calyx-aster --test ph58_tombstone_perf_gate --no-run \
  > >(tee "$build_stdout_log") 2> >(tee "$build_stderr_log" >&2)
status=$?
set -e
if [ "$status" -ne 0 ]; then
  echo "$status" >"$exit_marker"
  exit "$status"
fi

set +e
cargo test -p calyx-aster --test ph58_tombstone_perf_gate \
  ph58_tombstone_serving_p99_serial_gate \
  -- --ignored --exact --nocapture --test-threads=1 \
  > >(tee "$stdout_log") 2> >(tee "$stderr_log" >&2)
status=$?
set -e

echo "$status" >"$exit_marker"
if [ "$status" -ne 0 ]; then
  exit "$status"
fi

artifact="$root/ph58_tombstone_perf_gate.json"
if [ ! -s "$artifact" ]; then
  echo "ERROR: CALYX_PH58_ARTIFACT_MISSING $artifact" >&2
  echo 3 >"$exit_marker"
  exit 3
fi

python3 - "$artifact" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text())
missing = []
for dotted in [
    "actual.gate_passed",
    "actual.baseline.p99_ns",
    "actual.after.p99_ns",
    "actual.before_readback.visible",
    "actual.after_readback.visible",
    "host_context.load_before.cpu_count",
    "host_context.load_before.load1_per_cpu",
    "host_context.df_vault",
    "host_context.git_head",
]:
    node = data
    for part in dotted.split("."):
        if not isinstance(node, dict) or part not in node:
            missing.append(dotted)
            break
        node = node[part]
if missing:
    raise SystemExit(f"CALYX_PH58_ARTIFACT_INCOMPLETE missing={missing}")
if data["actual"]["gate_passed"] is not True:
    raise SystemExit("CALYX_PH58_ARTIFACT_GATE_FAILED")
if data["actual"]["before_readback"]["missing"] != 0 or data["actual"]["after_readback"]["missing"] != 0:
    raise SystemExit("CALYX_PH58_ARTIFACT_READBACK_MISSING")
print(f"PH58_TOMBSTONE_PERF_ARTIFACT={path}")
print(f"PH58_TOMBSTONE_PERF_P99_BASELINE_NS={data['actual']['baseline']['p99_ns']}")
print(f"PH58_TOMBSTONE_PERF_P99_AFTER_NS={data['actual']['after']['p99_ns']}")
print(f"PH58_TOMBSTONE_PERF_P99_RATIO={data['actual']['p99_ratio']}")
PY
