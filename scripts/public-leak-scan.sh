#!/usr/bin/env bash
# Calyx public-source leak gate.
#
# WHY: The public repo (ChrisRoyse/Calyx) is mirrored from this repo's
# ALLOWLISTED paths only (the same set the sync copies). Internal infra
# identifiers — the build-host codename, developer home-directory paths, the
# internal domain — must never reach those paths, or they leak to the public
# mirror. This check makes that class of leak impossible to commit or push.
#
# Dev-only trees (infra/, scripts/, docs/, docs2/, datasets/, .githooks/) are
# NOT scanned: they legitimately reference the real host and never go public.
#
# Modes:
#   scripts/public-leak-scan.sh                         # scan dev working tree public-bound paths
#   scripts/public-leak-scan.sh --cached                # scan staged dev public-bound paths
#   scripts/public-leak-scan.sh --public-tree <path>    # scan public checkout and reject forbidden tracked paths
#
# Exit: 0 = clean, 1 = forbidden identifier found, 2 = environment error.
set -euo pipefail

if ! command -v git >/dev/null 2>&1; then
  echo "public-leak-scan: ERROR — git not found on PATH." >&2
  exit 2
fi

mode="dev"
cached=""
repo_root=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cached)
      cached="--cached"
      shift
      ;;
    --public-tree)
      mode="public"
      if [[ $# -lt 2 ]]; then
        echo "public-leak-scan: ERROR — --public-tree requires a checkout path." >&2
        exit 2
      fi
      repo_root="$2"
      shift 2
      ;;
    *)
      echo "public-leak-scan: ERROR — unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -n "$cached" && "$mode" == "public" ]]; then
  echo "public-leak-scan: ERROR — --cached cannot be combined with --public-tree." >&2
  exit 2
fi

if [[ -z "$repo_root" ]]; then
  repo_root="$(git rev-parse --show-toplevel)"
fi

cd "$repo_root"

# Paths mirrored to the public repo. KEEP IN SYNC with the sync allowlist.
ALLOW_PATHS=(
  .cargo .config assets crates fuzz tools
  .gitattributes .gitignore .gitleaksignore
  Cargo.lock Cargo.toml LICENSE README.md rust-toolchain.toml
)

# Internal identifiers that must never appear in public-bound source.
# Case-insensitive. EXTEND this list as new internal names appear.
#   aiwonder  -> build-host codename        -> use a neutral placeholder (gpuhost)
#   croyse    -> developer username / homedir -> /var/lib/calyx/...
#   mst.com   -> internal domain
#   Calyx-Dev -> private development repo name
FORBIDDEN='aiwonder|croyse|mst\.com|Calyx-Dev'

# Paths that must never be tracked in the public repository at all. This is a
# separate gate from content scanning: these trees are allowed in Calyx-Dev but
# forbidden in ChrisRoyse/Calyx even when their contents look harmless.
PUBLIC_FORBIDDEN_PATH_RE='^(docs/|docs2/|scripts/|infra/|datasets/|env\.sh$|\.env|\.pre-commit-config\.yaml$|\.githooks/|CONTRIBUTING\.md$)'

if [[ "$mode" == "public" ]]; then
  forbidden_paths="$(git ls-files | grep -E "$PUBLIC_FORBIDDEN_PATH_RE" || true)"
  if [[ -n "$forbidden_paths" ]]; then
    echo "" >&2
    echo "public-leak-scan: REJECTED — forbidden tracked path(s) in public checkout:" >&2
    echo "$forbidden_paths" | sed 's/^/  /' >&2
    echo "" >&2
    echo "The public repo must contain only the public allowlist; remove these paths before push." >&2
    exit 1
  fi
fi

# Only scan allowlisted paths that actually exist in this checkout.
paths=()
for p in "${ALLOW_PATHS[@]}"; do
  [[ -e "$p" ]] && paths+=("$p")
done
if [[ ${#paths[@]} -eq 0 ]]; then
  echo "public-leak-scan: no allowlisted paths present; nothing to scan." >&2
  exit 0
fi

# git grep exits 1 when there are no matches; that is the success case here.
hits="$(git grep ${cached} -nIE -i -e "$FORBIDDEN" -- "${paths[@]}" 2>/dev/null || true)"

if [[ -n "$hits" ]]; then
  echo "" >&2
  echo "public-leak-scan: REJECTED — internal identifier(s) in public-bound source:" >&2
  echo "$hits" | sed 's/^/  /' >&2
  echo "" >&2
  echo "These paths are mirrored to the public repo (ChrisRoyse/Calyx)." >&2
  echo "Genericize before committing, e.g.:" >&2
  echo "    aiwonder         -> gpuhost" >&2
  echo "    /home/croyse/... -> /var/lib/calyx/..." >&2
  echo "    Calyx-Dev        -> Calyx" >&2
  echo "" >&2
  echo "Dev-only trees (infra/, scripts/, docs/, datasets/) may keep the real" >&2
  echo "names — they are not scanned and are not published." >&2
  exit 1
fi

if [[ "$mode" == "public" ]]; then
  echo "public-leak-scan: clean — no forbidden public tracked paths or internal identifiers in public-bound source." >&2
else
  echo "public-leak-scan: clean — no internal identifiers in public-bound source." >&2
fi
exit 0
