#!/usr/bin/env bash
set -euo pipefail

# deploy-calyx-runner.sh (#1108): build the requested Calyx binaries from the
# repo checkout at origin/main and install them into the runner directory
# with a verified identity trail.
#
# Root cause this closes: /home/croyse/calyx/target/release/ is a hand-managed
# deploy directory, not a cargo target. Binaries were copied there ad hoc, so
# rebuilds in ~/calyx/repo never reached the runner path and probes silently
# ran weeks-old code. Every step here either verifies or fails loudly:
#
#   1. repo must be clean and exactly at origin/main (fetched fresh)
#   2. no live ingestion may be running
#   3. the built binary must self-report the expected git SHA (build-info)
#   4. install is a same-filesystem staged rename (atomic; a running process
#      keeps its old inode, so nothing observes a half-written binary)
#   5. the deployed binary is re-executed and must self-report the same SHA
#   6. a <binary>.deploy.json manifest records the deploy for later audit
#
# There are no fallback paths. Any gate failure exits non-zero with a
# CALYX_DEPLOY_* code naming exactly what failed and how to fix it.
#
# usage: deploy-calyx-runner.sh [--repo DIR] [--dest DIR] [--features LIST]
#                               [--binary calyx|calyxd|calyx-mcp]...
#                               [--allow-in-use]
#
#   --binary may repeat; default deploys only `calyx` (the runner CLI).
#   --allow-in-use permits deploying over a binary some process is currently
#     executing. This is rename-safe (the process keeps the old inode) but
#     the running process will NOT pick up the new code until restarted.

repo="/home/croyse/calyx/repo"
dest="/home/croyse/calyx/target/release"
features=""
allow_in_use=0
binaries=()

die() {
  local code="$1"
  shift
  echo "ERROR: ${code}: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ $# -ge 2 ]] || die CALYX_DEPLOY_USAGE "--repo requires a value"
      repo="$2"
      shift 2
      ;;
    --dest)
      [[ $# -ge 2 ]] || die CALYX_DEPLOY_USAGE "--dest requires a value"
      dest="$2"
      shift 2
      ;;
    --features)
      [[ $# -ge 2 ]] || die CALYX_DEPLOY_USAGE "--features requires a value"
      features="$2"
      shift 2
      ;;
    --binary)
      [[ $# -ge 2 ]] || die CALYX_DEPLOY_USAGE "--binary requires a value"
      case "$2" in
        calyx|calyxd|calyx-mcp) binaries+=("$2") ;;
        *) die CALYX_DEPLOY_USAGE "--binary must be calyx, calyxd, or calyx-mcp; got $2" ;;
      esac
      shift 2
      ;;
    --allow-in-use)
      allow_in_use=1
      shift
      ;;
    *)
      die CALYX_DEPLOY_USAGE "unknown argument $1 (see script header for usage)"
      ;;
  esac
done
[[ ${#binaries[@]} -gt 0 ]] || binaries=(calyx)

for tool in git cargo jq; do
  command -v "$tool" >/dev/null 2>&1 \
    || die CALYX_DEPLOY_TOOL_MISSING "required tool not on PATH: $tool"
done

[[ -d "$repo/.git" || -f "$repo/.git" ]] \
  || die CALYX_DEPLOY_REPO_MISSING "$repo is not a git checkout"
[[ -d "$dest" ]] \
  || die CALYX_DEPLOY_DEST_MISSING "deploy directory $dest does not exist"

# Gate 1: repo must be exactly at a freshly fetched origin/main, clean.
git -C "$repo" fetch origin main \
  || die CALYX_DEPLOY_FETCH_FAILED "git fetch origin main failed in $repo; fix connectivity and rerun"
head_sha="$(git -C "$repo" rev-parse --verify HEAD)"
main_sha="$(git -C "$repo" rev-parse --verify origin/main)"
if [[ "$head_sha" != "$main_sha" ]]; then
  die CALYX_DEPLOY_REPO_STALE \
    "HEAD $head_sha != origin/main $main_sha in $repo; run 'git -C $repo checkout main && git -C $repo pull --ff-only origin main' and rerun"
fi
if [[ -n "$(git -C "$repo" --no-optional-locks status --porcelain --untracked-files=no)" ]]; then
  git -C "$repo" --no-optional-locks status --short --untracked-files=no >&2
  die CALYX_DEPLOY_REPO_DIRTY \
    "$repo has modified tracked files; commit, stash, or restore them and rerun"
fi

# Gate 2: never deploy while an ingestion is live (#1108 deploy constraint).
ingest_patterns=(
  'calyx ingest'
  '__ingest-lens-worker'
  'fsv-longrun'
  'longrun-supervise'
)
for pattern in "${ingest_patterns[@]}"; do
  if pids="$(pgrep -af -- "$pattern" 2>/dev/null)" && [[ -n "$pids" ]]; then
    echo "$pids" >&2
    die CALYX_DEPLOY_INGEST_ACTIVE \
      "live ingestion process matches '$pattern'; wait for it to finish and rerun"
  fi
done

# Gate 3: refuse to replace a binary a process is executing unless the
# operator opted into the (rename-safe) in-use swap.
if [[ "$allow_in_use" -ne 1 ]]; then
  for name in "${binaries[@]}"; do
    target="$dest/$name"
    [[ -e "$target" ]] || continue
    if command -v fuser >/dev/null 2>&1 && fuser -s "$target" 2>/dev/null; then
      fuser -v "$target" >&2 || true
      die CALYX_DEPLOY_BINARY_IN_USE \
        "$target is being executed; stop the process or pass --allow-in-use (rename-safe, but the running process keeps old code until restarted)"
    fi
  done
fi

build_info_json() {
  local name="$1" path="$2"
  case "$name" in
    calyx) "$path" build-info ;;
    calyxd|calyx-mcp) "$path" --build-info ;;
  esac
}

verify_identity() {
  local name="$1" path="$2" context="$3"
  local report sha dirty
  report="$(build_info_json "$name" "$path")" \
    || die CALYX_DEPLOY_IDENTITY_UNREADABLE \
      "$context: $path does not answer build-info; it predates #1108 or is not a Calyx binary"
  sha="$(jq -er '.git_sha' <<<"$report")" \
    || die CALYX_DEPLOY_IDENTITY_UNREADABLE "$context: $path build-info JSON lacks git_sha: $report"
  dirty="$(jq -er '.git_dirty' <<<"$report")" \
    || die CALYX_DEPLOY_IDENTITY_UNREADABLE "$context: $path build-info JSON lacks git_dirty: $report"
  [[ "$sha" == "$head_sha" ]] \
    || die CALYX_DEPLOY_IDENTITY_MISMATCH \
      "$context: $path reports git_sha=$sha, expected $head_sha; the build did not come from this checkout state"
  [[ "$dirty" == "false" ]] \
    || die CALYX_DEPLOY_IDENTITY_MISMATCH "$context: $path reports git_dirty=true; rebuild from a clean checkout"
  echo "[deploy] $context: $name identity verified git_sha=$sha"
}

crate_for() {
  case "$1" in
    calyx) echo "calyx-cli" ;;
    calyxd) echo "calyxd" ;;
    calyx-mcp) echo "calyx-mcp" ;;
  esac
}

# Build. `calyx` goes through the verified-build harness (env.sh sourcing,
# metadata readback, ELF RUNPATH checks); calyxd/calyx-mcp build directly
# from the same checkout and are verified through build-info below.
target_dir="${CARGO_TARGET_DIR:-$repo/target}"
declare -A built_path
for name in "${binaries[@]}"; do
  crate="$(crate_for "$name")"
  echo "[deploy] building $name (crate $crate) at $head_sha"
  if [[ "$name" == "calyx" ]]; then
    build_args=(--profile release --expect-head "$head_sha" --require-clean)
    [[ -n "$features" ]] && build_args+=(--features "$features")
    build_output="$(bash "$repo/scripts/build-verified-calyx.sh" "${build_args[@]}")" \
      || die CALYX_DEPLOY_BUILD_FAILED "verified build failed for $name"
    printf '%s\n' "$build_output" | sed 's/^/[build] /'
    built="$(printf '%s\n' "$build_output" | sed -n 's/^CALYX_VERIFIED_BINARY=//p' | tail -1)"
    [[ -n "$built" && -x "$built" ]] \
      || die CALYX_DEPLOY_BUILD_FAILED "build-verified-calyx.sh printed no CALYX_VERIFIED_BINARY"
  else
    feature_args=()
    [[ -n "$features" ]] && feature_args=(--features "$features")
    (
      cd "$repo"
      # shellcheck disable=SC1091
      [[ -f env.sh ]] && source env.sh
      CARGO_TARGET_DIR="$target_dir" cargo build --release \
        --manifest-path "$repo/Cargo.toml" -p "$crate" --bin "$name" "${feature_args[@]}"
    ) || die CALYX_DEPLOY_BUILD_FAILED "cargo build failed for $name"
    built="$target_dir/release/$name"
    [[ -x "$built" ]] \
      || die CALYX_DEPLOY_BUILD_FAILED "expected executable missing after build: $built"
  fi
  verify_identity "$name" "$built" "built"
  built_path["$name"]="$built"
done

# Install: stage in the destination directory (same filesystem), then rename.
deployed_at="$(date -u +%s)"
for name in "${binaries[@]}"; do
  built="${built_path[$name]}"
  target="$dest/$name"
  staged="$dest/.$name.deploy-staged.$$"
  cp -f "$built" "$staged"
  chmod 0755 "$staged"
  verify_identity "$name" "$staged" "staged"
  mv -f "$staged" "$target"
  verify_identity "$name" "$target" "deployed"
  jq -n \
    --arg binary "$name" \
    --arg git_sha "$head_sha" \
    --arg source_repo "$repo" \
    --arg target "$target" \
    --argjson deployed_at_unix_secs "$deployed_at" \
    '{binary: $binary, git_sha: $git_sha, source_repo: $source_repo,
      target: $target, deployed_at_unix_secs: $deployed_at_unix_secs}' \
    > "$dest/$name.deploy.json"
  jq -e '.git_sha == "'"$head_sha"'"' "$dest/$name.deploy.json" >/dev/null \
    || die CALYX_DEPLOY_MANIFEST_UNREADABLE "readback of $dest/$name.deploy.json failed"
  stat -c "[deploy] installed %n size=%s mtime=%y" "$target"
  echo "[deploy] manifest written: $dest/$name.deploy.json"
done

for name in "${binaries[@]}"; do
  if [[ "$name" != "calyx" ]] && pgrep -x "$name" >/dev/null 2>&1; then
    echo "[deploy] NOTE: a running $name process still executes the pre-deploy inode;" \
         "restart its service (e.g. sudo systemctl restart ${name}.service) to adopt $head_sha" >&2
  fi
done

echo "[deploy] OK: ${binaries[*]} at $head_sha -> $dest"
