#!/usr/bin/env bash
# FSV regression test for scripts/cargo-target-clean.sh.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_scripts="$(cd "$script_dir/.." && pwd -P)"
clean_src="$repo_scripts/cargo-target-clean.sh"
[[ -f "$clean_src" ]] || { echo "FSV ERROR: missing $clean_src" >&2; exit 2; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail=0

pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1" >&2; fail=1; }

repo="$work/repo"
scratch="$work/target-scratch"
mkdir -p "$repo" "$scratch"
printf '[workspace]\nmembers=[]\n' > "$repo/Cargo.toml"
export CALYX_TARGET_SCRATCH_ROOT="$scratch"

echo "=================================================================="
echo "SCENARIO 1 - workspace target is removed and read back absent"
echo "=================================================================="
mkdir -p "$repo/target/debug"
printf 'artifact\n' > "$repo/target/debug/libdemo.rlib"
out1="$(bash "$clean_src" --repo-root "$repo" --target-dir "$repo/target" 2>&1)"
echo "$out1" | sed 's/^/   /'
[[ ! -e "$repo/target" ]] \
  && pass "workspace target absent after cleanup" \
  || bad "workspace target still exists"
echo "$out1" | grep -Fq "target_absent path=$repo/target" \
  && pass "bounded absent readback printed" \
  || bad "missing target_absent readback"

echo
echo "=================================================================="
echo "SCENARIO 2 - scratch target and workspace target are both cleaned"
echo "=================================================================="
scratch_target="$scratch/run-a"
mkdir -p "$scratch_target/debug" "$repo/target/debug"
printf 'scratch\n' > "$scratch_target/debug/libdemo.rlib"
printf 'workspace\n' > "$repo/target/debug/libdemo.rlib"
out2="$(bash "$clean_src" --repo-root "$repo" --target-dir "$scratch_target" --also-workspace-target 2>&1)"
echo "$out2" | sed 's/^/   /'
[[ ! -e "$scratch_target" ]] \
  && pass "scratch target absent after cleanup" \
  || bad "scratch target still exists"
[[ ! -e "$repo/target" ]] \
  && pass "workspace target also absent after cleanup" \
  || bad "workspace target still exists after --also-workspace-target"

echo
echo "=================================================================="
echo "SCENARIO 3 - unsafe target outside workspace/scratch fails closed"
echo "=================================================================="
unsafe="$work/outside-target"
mkdir -p "$unsafe"
set +e
out3="$(bash "$clean_src" --repo-root "$repo" --target-dir "$unsafe" 2>&1)"
rc3=$?
set -e
echo "-- cleanup exit code: $rc3 --"
echo "$out3" | sed 's/^/   /'
[[ "$rc3" -ne 0 ]] \
  && pass "unsafe path rejected" \
  || bad "unsafe path should have failed"
[[ -d "$unsafe" ]] \
  && pass "unsafe directory was not removed" \
  || bad "unsafe directory was removed"
echo "$out3" | grep -Fq "refusing unsafe target path" \
  && pass "failure reason reports unsafe target" \
  || bad "unsafe failure reason missing"

echo
if [[ "$fail" -eq 0 ]]; then
  echo "ALL SCENARIOS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit "$fail"
