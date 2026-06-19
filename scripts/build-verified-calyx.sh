#!/usr/bin/env bash
# Build the calyx CLI from this exact worktree and verify the binary identity.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
sourced_env="not-found"
if [[ -f "$repo_root/env.sh" ]]; then
  case "$repo_root" in
    /home/croyse/calyx|/home/croyse/calyx/*)
      # shellcheck source=../env.sh
      source "$repo_root/env.sh"
      sourced_env="$repo_root/env.sh"
      ;;
    *)
      sourced_env="skipped-non-aiwonder:$repo_root/env.sh"
      ;;
  esac
fi

usage() {
  cat >&2 <<'USAGE'
usage: scripts/build-verified-calyx.sh [--profile debug|release] [--target-dir PATH]
                                       [--expect-head SHA_PREFIX]
                                       [--features CARGO_FEATURES]
                                       [--require-string TEXT]...
                                       [--require-clean]

Builds calyx-cli with an explicit target dir, reads Cargo metadata back, and
inspects the produced binary for required strings before printing
CALYX_VERIFIED_BINARY=<path>.
USAGE
}

profile="debug"
target_dir=""
expect_head=""
features=""
require_clean=0
required_strings=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      [[ $# -ge 2 ]] || { echo "ERROR: --profile requires a value" >&2; exit 2; }
      profile="$2"
      shift 2
      ;;
    --target-dir)
      [[ $# -ge 2 ]] || { echo "ERROR: --target-dir requires a value" >&2; exit 2; }
      target_dir="$2"
      shift 2
      ;;
    --expect-head)
      [[ $# -ge 2 ]] || { echo "ERROR: --expect-head requires a value" >&2; exit 2; }
      expect_head="$2"
      shift 2
      ;;
    --features)
      [[ $# -ge 2 ]] || { echo "ERROR: --features requires a value" >&2; exit 2; }
      features="$2"
      shift 2
      ;;
    --require-string)
      [[ $# -ge 2 ]] || { echo "ERROR: --require-string requires a value" >&2; exit 2; }
      required_strings+=("$2")
      shift 2
      ;;
    --require-clean)
      require_clean=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

case "$profile" in
  debug|dev) cargo_profile_args=(); profile_dir="debug" ;;
  release) cargo_profile_args=(--release); profile_dir="release" ;;
  *)
    echo "ERROR: --profile must be debug or release, got: $profile" >&2
    exit 2
    ;;
esac

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "ERROR: required tool not found on PATH: $1" >&2
    exit 1
  }
}

require_tool cargo
require_tool git
require_tool python3
require_tool strings

if [[ -z "$target_dir" ]]; then
  target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
fi
mkdir -p "$target_dir"
target_dir="$(cd "$target_dir" && pwd -P)"

head_sha="$(git -C "$repo_root" rev-parse --verify HEAD)"
if [[ -n "$expect_head" && "$head_sha" != "$expect_head"* ]]; then
  echo "ERROR: HEAD mismatch: expected prefix $expect_head, got $head_sha" >&2
  exit 1
fi

if [[ "$require_clean" -eq 1 && -n "$(git -C "$repo_root" status --porcelain)" ]]; then
  echo "ERROR: worktree is dirty; refusing verified build with --require-clean" >&2
  git -C "$repo_root" status --short >&2
  exit 1
fi

metadata="$(
  CARGO_TARGET_DIR="$target_dir" cargo metadata \
    --manifest-path "$repo_root/Cargo.toml" \
    --format-version=1 \
    --no-deps
)"

readarray -t metadata_paths < <(
  python3 -c 'import json,sys; m=json.loads(sys.stdin.read()); print(m["workspace_root"]); print(m["target_directory"])' \
    <<<"$metadata"
)
metadata_root="${metadata_paths[0]}"
metadata_target="${metadata_paths[1]}"

if [[ "$metadata_root" != "$repo_root" ]]; then
  echo "ERROR: Cargo metadata workspace_root mismatch: expected $repo_root, got $metadata_root" >&2
  exit 1
fi
if [[ "$metadata_target" != "$target_dir" ]]; then
  echo "ERROR: Cargo metadata target_directory mismatch: expected $target_dir, got $metadata_target" >&2
  exit 1
fi

echo "[build-verified] repo_root=$repo_root"
echo "[build-verified] env=$sourced_env"
echo "[build-verified] head=$head_sha"
echo "[build-verified] target_dir=$target_dir"
if [[ -n "$features" ]]; then
  echo "[build-verified] features=$features"
  cargo_feature_args=(--features "$features")
else
  cargo_feature_args=()
fi
CARGO_TARGET_DIR="$target_dir" cargo build \
  --manifest-path "$repo_root/Cargo.toml" \
  -p calyx-cli \
  --bin calyx \
  "${cargo_feature_args[@]}" \
  "${cargo_profile_args[@]}"

exe_suffix=""
case "$(uname -s 2>/dev/null || true)" in
  MINGW*|MSYS*|CYGWIN*) exe_suffix=".exe" ;;
esac
binary="$target_dir/$profile_dir/calyx$exe_suffix"

if [[ ! -x "$binary" ]]; then
  echo "ERROR: expected executable not found after build: $binary" >&2
  exit 1
fi

for needle in "${required_strings[@]}"; do
  count="$(strings "$binary" | grep -F -c -- "$needle" || true)"
  if [[ "$count" -lt 1 ]]; then
    echo "ERROR: binary identity check failed: '$needle' not found in $binary" >&2
    exit 1
  fi
  echo "[build-verified] require_string='$needle' count=$count"
done

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$binary"
fi
if stat --version >/dev/null 2>&1; then
  stat -c '[build-verified] binary size=%s mtime=%y path=%n' "$binary"
else
  stat "$binary"
fi
echo "CALYX_VERIFIED_BINARY=$binary"
