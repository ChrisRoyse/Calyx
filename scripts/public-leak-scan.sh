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
#   scripts/public-leak-scan.sh            # scan working tree   (pre-push / CI)
#   scripts/public-leak-scan.sh --cached   # scan staged content (pre-commit)
#
# Exit: 0 = clean, 1 = forbidden identifier found, 2 = environment error.
set -euo pipefail

if ! command -v git >/dev/null 2>&1; then
  echo "public-leak-scan: ERROR — git not found on PATH." >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
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

# Only scan allowlisted paths that actually exist in this checkout.
paths=()
for p in "${ALLOW_PATHS[@]}"; do
  [[ -e "$p" ]] && paths+=("$p")
done
if [[ ${#paths[@]} -eq 0 ]]; then
  echo "public-leak-scan: no allowlisted paths present; nothing to scan." >&2
  exit 0
fi

cached=""
if [[ "${1:-}" == "--cached" ]]; then
  cached="--cached"
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

echo "public-leak-scan: clean — no internal identifiers in public-bound source." >&2
exit 0
