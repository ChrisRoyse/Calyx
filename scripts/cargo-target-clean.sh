#!/usr/bin/env bash
# Clean Cargo targets and print bounded absence/audit readbacks.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
CALYX_GATE_STAGE_OWNER="cargo-target-clean"
# shellcheck source=gate-runtime-state.sh
source "$script_dir/gate-runtime-state.sh"

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/cargo-target-clean.sh [--repo-root <path>] [--target-dir <path>] [--expected-target-identity <dev:ino>] [--gate-lock-fd <fd>] [--also-workspace-target] [--also-repo-targets] [--also-calyx-home-target]

Safely removes a Cargo target directory only when the resolved path is:
  - the workspace target directory,
  - a child of the workspace target directory, or
  - a directory named target below the repository root, or
  - the exact `$CALYX_HOME/target` directory, or
  - an immediate Calyx-home directory named target-*, except the configured
    target-scratch root, or
  - a child of `CALYX_TARGET_SCRATCH_ROOT` (canonical default:
    `$CALYX_HOME/target-scratch`).

--also-repo-targets recursively removes every directory named target below the
repository root. Cargo gates must use it because nested Cargo invocations can
ignore the parent CARGO_TARGET_DIR and create crate-local target directories.

--also-calyx-home-target removes both the exact Calyx-home target and immediate
legacy target-* Cargo roots. The configured target-scratch container is never
removed and must be empty after cleanup. Every invocation audits these legacy
roots and fails closed if one remains. A nonempty legacy root must be owned by
the current user and contain Cargo's canonical `CACHEDIR.TAG` signature;
unmarked or malformed lookalikes are preserved and reported.

Successful removals print bounded target-absence lines. Every invocation that
reaches the cleanup audit also prints bounded repository, legacy-root, and
target-scratch readbacks; refusals retain the exact recovery path and return nonzero.
USAGE
}

repo_root_arg=""
target_dir_arg=""
also_workspace_target=0
also_repo_targets=0
also_calyx_home_target=0
expected_target_identity=""
gate_lock_fd_arg=""
readonly ENUMERATION_TIMEOUT_SECONDS=300
readonly SORT_TIMEOUT_SECONDS=60
readonly MAX_ENUMERATION_ENTRIES=16384
readonly MAX_ENUMERATION_BYTES=67108864

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      repo_root_arg="$2"
      shift 2
      ;;
    --target-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      target_dir_arg="$2"
      shift 2
      ;;
    --also-workspace-target)
      also_workspace_target=1
      shift
      ;;
    --also-repo-targets)
      also_repo_targets=1
      shift
      ;;
    --also-calyx-home-target)
      also_calyx_home_target=1
      shift
      ;;
    --expected-target-identity)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      expected_target_identity="$2"
      shift 2
      ;;
    --gate-lock-fd)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      gate_lock_fd_arg="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "cargo-target-clean: ERROR unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if ! command -v realpath >/dev/null 2>&1; then
  echo "cargo-target-clean: ERROR realpath not found on PATH" >&2
  exit 127
fi

if [[ -n "$repo_root_arg" ]]; then
  [[ -d "$repo_root_arg" ]] || {
    echo "cargo-target-clean: ERROR repo root does not exist: $repo_root_arg" >&2
    exit 2
  }
  repo_root="$(cd "$repo_root_arg" && pwd -P)"
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fi

calyx_gate_resolve_home calyx_home || exit $?
case "$repo_root" in
  "$calyx_home"/*) ;;
  *)
    echo "cargo-target-clean: ERROR repo root is outside CALYX_HOME: home=$calyx_home repo=$repo_root" >&2
    exit 2
    ;;
esac
calyx_gate_lexical_path workspace_target "workspace target" "$repo_root/target" || exit $?
calyx_gate_lexical_path calyx_home_target "Calyx-home target" "$calyx_home/target" || exit $?
calyx_gate_resolve_target_root scratch_root "$calyx_home" || exit $?
current_uid="$(id -u)"
if [[ -n "$expected_target_identity" \
  && ! "$expected_target_identity" =~ ^[0-9]+:[0-9]+$ ]]; then
  echo "cargo-target-clean: ERROR invalid expected target identity: $expected_target_identity" >&2
  exit 2
fi

gate_lock_root="$calyx_home/tmp"
gate_lock_path="$gate_lock_root/.cargo-gate.lock"
if [[ -n "${CALYX_GATE_LOCK_PATH:-}" \
  && "$CALYX_GATE_LOCK_PATH" != "$gate_lock_path" ]]; then
  echo "cargo-target-clean: ERROR gate lock path differs from canonical path: expected=$gate_lock_path actual=$CALYX_GATE_LOCK_PATH" >&2
  exit 2
fi
if [[ ! -e "$gate_lock_root" ]]; then
  (umask 077; mkdir -- "$gate_lock_root") || [[ -d "$gate_lock_root" ]] || {
    echo "cargo-target-clean: ERROR cannot create gate lock root: $gate_lock_root" >&2
    exit 2
  }
fi
if [[ -L "$gate_lock_root" || ! -d "$gate_lock_root" ]]; then
  echo "cargo-target-clean: ERROR gate lock root is not a stable directory: $gate_lock_root" >&2
  exit 2
fi
gate_lock_root_uid="$(stat -c '%u' -- "$gate_lock_root")"
gate_lock_root_mode="$(stat -c '%a' -- "$gate_lock_root")"
gate_lock_root_mode_value=$((8#$gate_lock_root_mode))
if [[ "$gate_lock_root_uid" != "$current_uid" || $((gate_lock_root_mode_value & 0022)) -ne 0 ]]; then
  echo "cargo-target-clean: ERROR gate lock root ownership/mode is unsafe: path=$gate_lock_root uid=$gate_lock_root_uid mode=$gate_lock_root_mode" >&2
  exit 2
fi
if [[ -e "$gate_lock_path" || -L "$gate_lock_path" ]]; then
  [[ -f "$gate_lock_path" && ! -L "$gate_lock_path" ]] || {
    echo "cargo-target-clean: ERROR gate lock is not a regular file: $gate_lock_path" >&2
    exit 2
  }
else
  old_umask="$(umask)"
  umask 077
  : >>"$gate_lock_path"
  umask "$old_umask"
fi
gate_lock_uid="$(stat -c '%u' -- "$gate_lock_path")"
gate_lock_mode="$(stat -c '%a' -- "$gate_lock_path")"
gate_lock_mode_value=$((8#$gate_lock_mode))
if [[ "$gate_lock_uid" != "$current_uid" || $((gate_lock_mode_value & 0022)) -ne 0 ]]; then
  echo "cargo-target-clean: ERROR gate lock ownership/mode is unsafe: path=$gate_lock_path uid=$gate_lock_uid mode=$gate_lock_mode" >&2
  exit 2
fi
calyx_gate_require_ofd_flock || exit $?
if [[ -n "$gate_lock_fd_arg" ]]; then
  [[ "$gate_lock_fd_arg" =~ ^[0-9]+$ && -e "/proc/$BASHPID/fd/$gate_lock_fd_arg" ]] || {
    echo "cargo-target-clean: ERROR inherited gate lock fd is unavailable: $gate_lock_fd_arg" >&2
    exit 2
  }
  inherited_lock_path="$(readlink -f -- "/proc/$BASHPID/fd/$gate_lock_fd_arg")"
  [[ "$inherited_lock_path" == "$gate_lock_path" ]] || {
    echo "cargo-target-clean: ERROR inherited gate lock path mismatch: expected=$gate_lock_path actual=$inherited_lock_path" >&2
    exit 2
  }
  calyx_gate_validate_open_lock "$gate_lock_path" "$gate_lock_fd_arg" || {
    echo "cargo-target-clean: ERROR inherited gate lock identity changed: $gate_lock_path" >&2
    exit 2
  }
  calyx_gate_prove_lock_held "$gate_lock_path" "$gate_lock_fd_arg" || exit 2
  gate_lock_fd="$gate_lock_fd_arg"
else
  exec {gate_lock_fd}>>"$gate_lock_path"
  calyx_gate_validate_lock_path "$gate_lock_path" || exit $?
  calyx_gate_validate_open_lock "$gate_lock_path" "$gate_lock_fd" || exit $?
  flock --fcntl --exclusive "$gate_lock_fd"
fi
echo "cargo-target-clean: gate_lock_acquired path=$gate_lock_path"
export CALYX_GATE_LOCK_PATH="$gate_lock_path"
export CALYX_GATE_LOCK_FD="$gate_lock_fd"

snapshot_find() {
  local outvar="$1" label="$2" mode="$3" status=0 bytes=0 entry had_errexit=0
  local -a producer_status=()
  local -n result="$outvar"
  shift 3
  [[ "$mode" == full || "$mode" == first ]] || return 64
  result=()
  [[ "$-" != *e* ]] || had_errexit=1
  shopt -s lastpipe
  set +e
  if [[ "$mode" == first ]]; then
    timeout --signal=TERM "${ENUMERATION_TIMEOUT_SECONDS}s" find "$@" -print0 -quit | \
      mapfile -d '' -t -n 2 "$outvar"
  else
    timeout --signal=TERM "${ENUMERATION_TIMEOUT_SECONDS}s" find "$@" -print0 | (
      LC_ALL=C
      count=0
      bytes=0
      while IFS= read -r -d '' entry; do
        count=$((count + 1))
        bytes=$((bytes + ${#entry} + 1))
        if ((count > MAX_ENUMERATION_ENTRIES || bytes > MAX_ENUMERATION_BYTES)); then
          echo "cargo-target-clean: ERROR enumeration limit exceeded: label=$label entries=$count bytes=$bytes" >&2
          exit 75
        fi
        printf '%s\0' "$entry"
      done
    ) | timeout --signal=TERM "${SORT_TIMEOUT_SECONDS}s" env LC_ALL=C sort -z -- | \
      mapfile -d '' -t -n 16385 "$outvar"
  fi
  producer_status=("${PIPESTATUS[@]}")
  [[ "$had_errexit" -eq 0 ]] || set -e
  for status in "${producer_status[@]}"; do
    if [[ "$status" -ne 0 ]]; then
      echo "cargo-target-clean: ERROR enumeration failed: label=$label status=$status" >&2
      return "$status"
    fi
  done
  if [[ "$mode" == full && "${#result[@]}" -gt MAX_ENUMERATION_ENTRIES ]]; then
    echo "cargo-target-clean: ERROR enumeration entry limit exceeded: label=$label limit=$MAX_ENUMERATION_ENTRIES" >&2
    return 125
  fi
  for entry in "${result[@]}"; do
    bytes=$((bytes + ${#entry}))
    if (( bytes > MAX_ENUMERATION_BYTES )); then
      echo "cargo-target-clean: ERROR enumeration byte limit exceeded: label=$label limit=$MAX_ENUMERATION_BYTES" >&2
      return 125
    fi
  done
  if [[ "$mode" == first && "${#result[@]}" -gt 1 ]]; then
    result=("${result[0]}")
  fi
}

for enum_command in find sort timeout; do
  if ! command -v "$enum_command" >/dev/null 2>&1; then
    echo "cargo-target-clean: ERROR $enum_command not found on PATH" >&2
    exit 127
  fi
done

if [[ -z "$target_dir_arg" ]]; then
  target_dir_arg="${CARGO_TARGET_DIR:-$workspace_target}"
fi

target_dirs=("$target_dir_arg")
if [[ "$also_workspace_target" -eq 1 ]]; then
  target_dirs+=("$workspace_target")
fi
if [[ "$also_calyx_home_target" -eq 1 ]]; then
  target_dirs+=("$calyx_home_target")
fi

is_calyx_home_legacy_target() {
  local target="$1"
  [[ "$(dirname "$target")" == "$calyx_home" \
    && "${target##*/}" == target-* \
    && "$target" != "$scratch_root" ]]
}

is_cargo_target_root() {
  local target="$1" first="" marker_line=""
  local -a cargo_snapshot=()
  if [[ -f "$target/CACHEDIR.TAG" && ! -L "$target/CACHEDIR.TAG" ]]; then
    IFS= read -r marker_line <"$target/CACHEDIR.TAG" || true
    [[ "$marker_line" == 'Signature: 8a477f597d28d172789f06886806bc55' ]] && return 0
  fi
  snapshot_find cargo_snapshot cargo-root-classification first "$target" \
    -mindepth 1 -maxdepth 1 || return $?
  first="${cargo_snapshot[0]:-}"
  [[ -z "$first" ]]
}

is_safe_target() {
  local target="$1"
  case "$target" in
    "$workspace_target"|"$workspace_target"/*) return 0 ;;
    "$calyx_home_target") return 0 ;;
    "$scratch_root"/*) return 0 ;;
  esac
  if is_calyx_home_legacy_target "$target"; then
    return 0
  fi
  [[ "$target" == "$repo_root/"* && "${target##*/}" == "target" ]]
}

clean_one() {
  local raw="$1" expected_identity="${2:-}"
  local target physical actual_identity owner_uid mode mode_value parent parent_identity cargo_policy=0
  local cargo_root_status=0
  calyx_gate_lexical_path target "Cargo target" "$raw" || return $?
  physical="$(realpath -m -- "$target")" || return 125
  if [[ "$physical" != "$target" ]]; then
    echo "cargo-target-clean: ERROR refusing target with symlink component: lexical=$target physical=$physical" >&2
    return 2
  fi

  if ! is_safe_target "$target"; then
    echo "cargo-target-clean: ERROR refusing unsafe target path: $target" >&2
    echo "cargo-target-clean: workspace_target=$workspace_target" >&2
    echo "cargo-target-clean: calyx_home_target=$calyx_home_target" >&2
    echo "cargo-target-clean: scratch_root=$scratch_root" >&2
    return 2
  fi

  if [[ ! -e "$target" ]]; then
    if [[ -n "$expected_identity" ]]; then
      echo "cargo-target-clean: ERROR expected target is absent: path=$target identity=$expected_identity" >&2
      return 1
    fi
    echo "cargo-target-clean: target_absent path=$target"
    return 0
  fi

  if [[ ! -d "$target" ]]; then
    echo "cargo-target-clean: ERROR target path exists but is not a directory: $target" >&2
    return 2
  fi

  owner_uid="$(stat -c '%u' -- "$target")"
  mode="$(stat -c '%a' -- "$target")"
  mode_value=$((8#$mode))
  [[ "$owner_uid" == "$current_uid" && $((mode_value & 0022)) -eq 0 ]] || {
    echo "cargo-target-clean: ERROR target ownership/mode is unsafe: path=$target uid=$owner_uid mode=$mode" >&2
    return 1
  }

  actual_identity="$(stat -c '%d:%i' -- "$target")" || return 1

  if [[ -n "$expected_identity" ]]; then
    [[ "$actual_identity" == "$expected_identity" ]] || {
      echo "cargo-target-clean: ERROR target identity changed: path=$target expected=$expected_identity actual=$actual_identity" >&2
      return 1
    }
  fi

  if is_calyx_home_legacy_target "$target"; then
    cargo_policy=1
    owner_uid="$(stat -c '%u' -- "$target")"
    if [[ "$owner_uid" != "$current_uid" ]]; then
      echo "cargo-target-clean: ERROR legacy target is not owned by current uid $current_uid: $target owner_uid=$owner_uid" >&2
      return 2
    fi
    cargo_root_status=0
    is_cargo_target_root "$target" || cargo_root_status=$?
    if [[ "$cargo_root_status" -ne 0 ]]; then
      [[ "$cargo_root_status" -eq 1 ]] || return "$cargo_root_status"
      echo "cargo-target-clean: ERROR nonempty legacy target lacks Cargo root marker: $target" >&2
      return 2
    fi
  fi

  calyx_gate_assert_no_mounts_at_or_below "$target" "Cargo target" || return $?
  parent="$(dirname -- "$target")"
  parent_identity="$(stat -c '%d:%i' -- "$parent")" || return 125
  calyx_gate_safe_remove_entry "$parent" "$parent_identity" "$(basename -- "$target")" \
    "$actual_identity" directory cargo-target "$current_uid" "$current_uid" 0022 0022 "$cargo_policy" || return $?
  [[ ! -e "$target" && ! -L "$target" ]] || return 1

  echo "cargo-target-clean: target_absent path=$target"
}

seen=""
rc=0
first_target=1
for target in "${target_dirs[@]}"; do
  calyx_gate_lexical_path resolved "Cargo target" "$target" || { rc=$?; continue; }
  case "
$seen
" in
    *"
$resolved
"*) continue ;;
  esac
  seen="${seen}${resolved}"$'\n'
  expected=""
  if [[ "$first_target" -eq 1 ]]; then
    expected="$expected_target_identity"
    first_target=0
  fi
  clean_one "$resolved" "$expected" || rc=$?
done

if [[ "$also_repo_targets" -eq 1 ]]; then
  calyx_gate_assert_no_mounts_at_or_below "$repo_root" "repository" || exit $?
  repo_target_count=0
  repo_targets_snapshot=()
  snapshot_find repo_targets_snapshot repo-targets full "$repo_root" \
    -mindepth 1 -type d -name target -prune || exit $?
  for target in "${repo_targets_snapshot[@]}"; do
    clean_one "$target" || rc=$?
    repo_target_count=$((repo_target_count + 1))
  done

  remaining_repo_snapshot=()
  snapshot_find remaining_repo_snapshot remaining-repo-targets first "$repo_root" \
    -mindepth 1 -type d -name target -prune || exit $?
  remaining_repo_target="${remaining_repo_snapshot[0]:-}"
  if [[ -n "$remaining_repo_target" ]]; then
    echo "cargo-target-clean: ERROR repo target remains: $remaining_repo_target" >&2
    rc=1
  else
    echo "cargo-target-clean: repo_targets_absent root=$repo_root cleaned=$repo_target_count"
  fi
fi

legacy_target_count=0
legacy_targets_snapshot=()
snapshot_find legacy_targets_snapshot legacy-targets full "$calyx_home" \
  -mindepth 1 -maxdepth 1 -name 'target-*' || exit $?
for candidate in "${legacy_targets_snapshot[@]}"; do
  if [[ "$candidate" == "$scratch_root" ]]; then
    continue
  fi
  if [[ "$also_calyx_home_target" -eq 1 ]]; then
    clean_one "$candidate" || rc=$?
    legacy_target_count=$((legacy_target_count + 1))
  fi
done

remaining_calyx_target=""
remaining_calyx_target_resolved=""
remaining_legacy_snapshot=()
snapshot_find remaining_legacy_snapshot remaining-legacy-targets full "$calyx_home" \
  -mindepth 1 -maxdepth 1 -name 'target-*' || exit $?
for candidate in "${remaining_legacy_snapshot[@]}"; do
  if [[ "$candidate" == "$scratch_root" ]]; then
    continue
  fi
  remaining_calyx_target="$candidate"
  remaining_calyx_target_resolved="$(realpath -m -- "$candidate")"
  break
done

if [[ -n "$remaining_calyx_target" ]]; then
  echo "cargo-target-clean: ERROR Calyx-home target-* path remains: $remaining_calyx_target resolved=$remaining_calyx_target_resolved" >&2
  echo "cargo-target-clean: rerun with --also-calyx-home-target after verifying the path is not in use" >&2
  rc=1
else
  echo "cargo-target-clean: calyx_home_target_prefixes_absent root=$calyx_home cleaned=$legacy_target_count"
fi

if [[ -e "$scratch_root" && ! -d "$scratch_root" ]]; then
  echo "cargo-target-clean: ERROR scratch root exists but is not a directory: $scratch_root" >&2
  rc=1
elif [[ -d "$scratch_root" ]]; then
  scratch_snapshot=()
  snapshot_find scratch_snapshot scratch-final first "$scratch_root" \
    -mindepth 1 -maxdepth 1 || exit $?
  scratch_entry="${scratch_snapshot[0]:-}"
  if [[ -n "$scratch_entry" ]]; then
    scratch_blocks="$(stat -c '%b' -- "$scratch_root" 2>/dev/null || printf unknown)"
    printf 'cargo-target-clean: ERROR scratch_root_not_empty path=%s dir_blocks=%s first_entry=%q\n' \
      "$scratch_root" "$scratch_blocks" "$scratch_entry" >&2
    rc=1
  else
    echo "cargo-target-clean: scratch_root_empty path=$scratch_root"
  fi
else
  echo "cargo-target-clean: scratch_root_absent path=$scratch_root"
fi

exit "$rc"
