#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
