#!/usr/bin/env bash
set -euo pipefail

# Never commit a credential value. Use Infisical for secrets; env-var names in code only.

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)"
mode="worktree"
git_log_opts=""

usage() {
  cat >&2 <<'USAGE'
usage: scripts/secret-scan.sh [--staged|--cached|--git-history|--git-log-opts OPTS] [gitleaks flags...]

Modes:
  default       scan the current worktree contents
  --staged     scan the Git index exactly as it would be committed
  --cached     alias for --staged
  --git-history scan the current repository history reachable from HEAD
  --git-log-opts scan repository history using explicit git log options
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staged|--cached)
      mode="staged"
      shift
      ;;
    --git-history)
      mode="git-history"
      shift
      ;;
    --git-log-opts)
      if [[ $# -lt 2 || -z "$2" ]]; then
        echo "secret-scan: --git-log-opts requires a non-empty git log option string" >&2
        exit 2
      fi
      mode="git-log-opts"
      git_log_opts="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

if command -v gitleaks >/dev/null 2>&1; then
  GITLEAKS_BIN="gitleaks"
elif [ -x "${HOME}/go/bin/gitleaks" ]; then
  GITLEAKS_BIN="${HOME}/go/bin/gitleaks"
elif [ -x "${HOME}/go/bin/gitleaks.exe" ]; then
  GITLEAKS_BIN="${HOME}/go/bin/gitleaks.exe"
elif [ -x "${HOME}/.local/bin/gitleaks" ]; then
  GITLEAKS_BIN="${HOME}/.local/bin/gitleaks"
elif [ -n "${LOCALAPPDATA:-}" ] && [ -x "${LOCALAPPDATA}/Microsoft/WinGet/Links/gitleaks.exe" ]; then
  GITLEAKS_BIN="${LOCALAPPDATA}/Microsoft/WinGet/Links/gitleaks.exe"
else
  echo "gitleaks not found; install it before committing" >&2
  exit 127
fi

case "$mode" in
  worktree)
    tmp="$(mktemp -d -t calyx-secret-scan-worktree.XXXXXX)"
    cleanup() {
      rm -rf "$tmp"
    }
    trap cleanup EXIT
    git -C "$repo_root" ls-files -z --cached --others --exclude-standard |
      while IFS= read -r -d '' rel; do
        src="$repo_root/$rel"
        [ -f "$src" ] || continue
        mkdir -p "$tmp/$(dirname "$rel")"
        cp -p "$src" "$tmp/$rel"
      done
    exec "${GITLEAKS_BIN}" dir --redact "$tmp" "$@"
    ;;
  git-history)
    echo "secret-scan: scanning git history reachable from HEAD" >&2
    exec "${GITLEAKS_BIN}" git --redact --log-opts HEAD "$repo_root" "$@"
    ;;
  git-log-opts)
    echo "secret-scan: scanning git history with log opts: $git_log_opts" >&2
    exec "${GITLEAKS_BIN}" git --redact --log-opts "$git_log_opts" "$repo_root" "$@"
    ;;
  staged)
    tmp="$(mktemp -d -t calyx-secret-scan-index.XXXXXX)"
    cleanup() {
      rm -rf "$tmp"
    }
    trap cleanup EXIT
    git -C "$repo_root" checkout-index --all --force --prefix="$tmp/"
    exec "${GITLEAKS_BIN}" dir --redact "$tmp" "$@"
    ;;
  *)
    echo "unknown secret scan mode: $mode" >&2
    exit 2
    ;;
esac
