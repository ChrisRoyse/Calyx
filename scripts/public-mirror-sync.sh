#!/usr/bin/env bash
# Deterministically sync public-bound Calyx-Dev paths into a ChrisRoyse/Calyx checkout.
#
# This script is intentionally private-repo tooling. It copies only the public
# allowlist, removes public-forbidden tracked paths, and runs the hard public
# tree gate before the operator commits and pushes the public mirror.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/public-mirror-sync.sh --public-dir <path-to-Calyx-public-checkout>

After this succeeds:
  cd <path-to-Calyx-public-checkout>
  git diff --check
  cargo fmt --all -- --check
  cargo test ... / cargo check ...
  git commit -m "Sync public mirror"
  git push origin main
USAGE
}

public_dir=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --public-dir)
      if [[ $# -lt 2 ]]; then
        usage
        exit 2
      fi
      public_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "public-mirror-sync: ERROR: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$public_dir" ]]; then
  usage
  exit 2
fi

dev_root="$(git rev-parse --show-toplevel)"
public_root="$(git -C "$public_dir" rev-parse --show-toplevel)"

if [[ "$dev_root" == "$public_root" ]]; then
  echo "public-mirror-sync: ERROR: public checkout must be separate from Calyx-Dev." >&2
  exit 2
fi

origin_url="$(git -C "$public_root" remote get-url origin 2>/dev/null || true)"
if [[ "$origin_url" != *"ChrisRoyse/Calyx"* ]]; then
  echo "public-mirror-sync: ERROR: public checkout origin is not ChrisRoyse/Calyx: $origin_url" >&2
  exit 2
fi

if [[ -n "$(git -C "$public_root" status --porcelain)" ]]; then
  echo "public-mirror-sync: ERROR: public checkout is dirty; commit/stash/reset before sync." >&2
  exit 2
fi

allow_paths=(
  .cargo .config assets crates fuzz tools
  .gitattributes .gitignore .gitleaksignore
  Cargo.lock Cargo.toml LICENSE README.md rust-toolchain.toml
)

for path in "${allow_paths[@]}"; do
  rm -rf "$public_root/$path"
  if [[ -e "$dev_root/$path" ]]; then
    parent="$(dirname "$public_root/$path")"
    mkdir -p "$parent"
    cp -a "$dev_root/$path" "$parent/"
  fi
done

# Stage the full allowlist sync (issue #1195). The copy loop above only touches
# the working tree; without an explicit `git add` the copied/modified files stay
# unstaged, so an immediate `git commit` produced a deletion-only commit while
# the real source updates were left dirty, even though this script claimed the
# sync was staged. `git add -A` on each allowlisted pathspec stages additions,
# modifications, AND deletions (a path removed from dev is staged as a deletion
# in the mirror). We stage only pathspecs that now exist (were copied) or are
# still tracked (so their removal stages); a never-present optional allowlist
# path is skipped instead of hard-failing `git add` on an unmatched pathspec.
stage_paths=()
for path in "${allow_paths[@]}"; do
  if [[ -e "$public_root/$path" ]] \
    || git -C "$public_root" ls-files --error-unmatch -- "$path" >/dev/null 2>&1; then
    stage_paths+=("$path")
  fi
done
if [[ ${#stage_paths[@]} -gt 0 ]]; then
  git -C "$public_root" add -A -- "${stage_paths[@]}"
fi

git -C "$public_root" rm -r --ignore-unmatch -- \
  docs docs2 scripts infra datasets env.sh .env ".env.*" \
  .pre-commit-config.yaml .githooks CONTRIBUTING.md >/dev/null

bash "$dev_root/scripts/public-leak-scan.sh" --public-tree "$public_root"

# Fail closed if the allowlist sync left changes unstaged: the whole point of
# this script is that the operator can commit immediately after it returns. Any
# unstaged/untracked path under the working tree here means the sync did not
# fully stage what it copied, so we refuse rather than repeat the #1195 bug of
# claiming a staged sync that isn't.
unstaged="$(git -C "$public_root" status --porcelain | grep -E '^( .|\?\?)' || true)"
if [[ -n "$unstaged" ]]; then
  echo "public-mirror-sync: ERROR: allowlist sync left unstaged/untracked paths:" >&2
  echo "$unstaged" | sed 's/^/  /' >&2
  echo "public-mirror-sync: this is the #1195 failure mode; not claiming a staged sync." >&2
  exit 1
fi

echo "public-mirror-sync: staged public allowlist sync in $public_root" >&2
echo "public-mirror-sync: review, run gates, commit, push, and read back origin/main." >&2
