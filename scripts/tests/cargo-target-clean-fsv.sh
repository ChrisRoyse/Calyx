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
echo "SCENARIO 3 - Calyx-home FSV target is cleaned when requested"
echo "=================================================================="
calyx_home_target="$work/target"
mkdir -p "$scratch_target/debug" "$repo/target/debug" "$calyx_home_target/fsv"
printf 'scratch\n' > "$scratch_target/debug/libdemo.rlib"
printf 'workspace\n' > "$repo/target/debug/libdemo.rlib"
printf 'fsv\n' > "$calyx_home_target/fsv/evidence.bin"
out3="$(bash "$clean_src" --repo-root "$repo" --target-dir "$scratch_target" --also-workspace-target --also-calyx-home-target 2>&1)"
echo "$out3" | sed 's/^/   /'
[[ ! -e "$scratch_target" && ! -e "$repo/target" && ! -e "$calyx_home_target" ]] \
  && pass "scratch, workspace, and Calyx-home targets absent after cleanup" \
  || bad "one or more requested target directories still exist"
echo "$out3" | grep -Fq "target_absent path=$calyx_home_target" \
  && pass "Calyx-home absent readback printed" \
  || bad "missing Calyx-home target_absent readback"

echo
echo "=================================================================="
echo "SCENARIO 4 - crate-local targets are swept recursively"
echo "=================================================================="
crate_target="$repo/crates/demo/target"
tool_target="$repo/tools/demo/target"
mkdir -p "$scratch_target/debug" "$crate_target/debug" "$tool_target/release"
printf 'scratch\n' > "$scratch_target/debug/libdemo.rlib"
printf 'crate\n' > "$crate_target/debug/libdemo.rlib"
printf 'tool\n' > "$tool_target/release/demo"
out4="$(bash "$clean_src" --repo-root "$repo" --target-dir "$scratch_target" --also-repo-targets 2>&1)"
echo "$out4" | sed 's/^/   /'
[[ ! -e "$scratch_target" && ! -e "$crate_target" && ! -e "$tool_target" ]] \
  && pass "scratch and crate-local targets absent after recursive cleanup" \
  || bad "one or more recursive repository targets still exist"
remaining_repo_target="$(find "$repo" -mindepth 1 -type d -name target -prune -print -quit)"
[[ -z "$remaining_repo_target" ]] \
  && pass "recursive repository target scan is empty" \
  || bad "recursive repository target scan found $remaining_repo_target"
echo "$out4" | grep -Fq "repo_targets_absent root=$repo cleaned=2" \
  && pass "bounded recursive cleanup readback printed" \
  || bad "missing recursive cleanup readback"

echo
echo "=================================================================="
echo "SCENARIO 5 - Calyx-home allowlist is exact, not recursive"
echo "=================================================================="
calyx_child="$work/target/keep"
mkdir -p "$calyx_child"
set +e
out5="$(bash "$clean_src" --repo-root "$repo" --target-dir "$calyx_child" 2>&1)"
rc5=$?
set -e
echo "-- cleanup exit code: $rc5 --"
echo "$out5" | sed 's/^/   /'
[[ "$rc5" -ne 0 && -d "$calyx_child" ]] \
  && pass "Calyx-home child rejected without removing it" \
  || bad "Calyx-home allowlist accepted a child path"

echo
echo "=================================================================="
echo "SCENARIO 6 - unsafe target outside workspace/scratch fails closed"
echo "=================================================================="
unsafe="$work/outside-target"
mkdir -p "$unsafe"
set +e
out6="$(bash "$clean_src" --repo-root "$repo" --target-dir "$unsafe" 2>&1)"
rc6=$?
set -e
echo "-- cleanup exit code: $rc6 --"
echo "$out6" | sed 's/^/   /'
[[ "$rc6" -ne 0 ]] \
  && pass "unsafe path rejected" \
  || bad "unsafe path should have failed"
[[ -d "$unsafe" ]] \
  && pass "unsafe directory was not removed" \
  || bad "unsafe directory was removed"
echo "$out6" | grep -Fq "refusing unsafe target path" \
  && pass "failure reason reports unsafe target" \
  || bad "unsafe failure reason missing"

echo
if [[ "$fail" -eq 0 ]]; then
  echo "ALL SCENARIOS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit "$fail"
