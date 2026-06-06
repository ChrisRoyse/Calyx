# T-009 — Line-count gate + `check.sh` wrapper

**Phase:** PH01 · **Dep:** T-008 · **Sudo:** no

## Objective
Wire the **≤500-line** gate (DOCTRINE §8, hard) and a single pre-merge wrapper
that runs check + clippy + test + gate, so the rule is enforced from the first
real file.

## Preconditions
- T-008 (workspace).

## Steps
1. `repo/scripts/linecount.sh` (the exact gate from DOCTRINE §8):
   ```bash
   #!/usr/bin/env bash
   set -euo pipefail
   find crates -name '*.rs' -exec wc -l {} + \
    | awk -v max=500 '$1>max && !/total/{print "❌",$0; v=1} END{if(!v)print "✅ all .rs ≤ 500 lines"}'
   ```
2. `repo/scripts/check.sh` — the pre-merge wrapper:
   ```bash
   #!/usr/bin/env bash
   set -euo pipefail
   source "$HOME/.cargo/env"
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   bash "$(dirname "$0")/linecount.sh"
   ```
   `chmod +x repo/scripts/*.sh`.
3. Run it; confirm ✅.
4. Document in the repo README + the working-agreement that `check.sh` is the
   per-merge gate (no CI — FSV is CI). Over-limit file → open a `type:task`
   issue + modularize before the gate passes.

## Deliverables
- `linecount.sh`, `check.sh`; documented as the per-merge gate.

## FSV gate
`bash scripts/check.sh` runs on aiwonder and prints ✅ (fmt + clippy + test +
≤500); deliberately create a 501-line `.rs` → the gate prints ❌ and names it
(then delete it). Proves the gate actually fails when violated.

## Done
The gate is wired, demonstrated to fail-when-wrong, and adopted as the per-merge
check.

## Refs
DOCTRINE §8, PRD `19 §3b`, `../02_WORKING_AGREEMENT.md §4/§5`.
