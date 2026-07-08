#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/linecount.sh [--repo-root <path>]

Checks every Rust source file under <repo>/crates against the 500-line limit.
USAGE
}

repo_root_arg=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      if [[ $# -lt 2 ]]; then
        echo "linecount: ERROR --repo-root requires a path" >&2
        usage
        exit 2
      fi
      repo_root_arg="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "linecount: ERROR unknown argument '$1'" >&2
      usage
      exit 2
      ;;
  esac
  shift
done

if [[ -n "$repo_root_arg" ]]; then
  if [[ ! -d "$repo_root_arg" ]]; then
    echo "linecount: ERROR repo root does not exist or is not a directory: $repo_root_arg" >&2
    exit 2
  fi
  repo_root="$(cd "$repo_root_arg" && pwd)"
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
cd "$repo_root"

find crates -type f -name '*.rs' -exec wc -l {} + |
  awk -v max=500 '
    $1 > max && $2 != "total" {
      print "❌", $0
      violations = 1
    }
    END {
      if (violations) {
        exit 1
      }
      print "✅ all .rs ≤ 500 lines"
    }
  '
