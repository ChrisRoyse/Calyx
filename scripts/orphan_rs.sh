#!/usr/bin/env bash
set -euo pipefail

# Fails when a .rs file in the repo (tracked OR untracked-but-not-ignored) is
# never read by the compiler during a full-workspace check. Such a file looks
# like finished work but is not built, tested, linted, or shipped — authored
# code that silently does nothing (e.g. a module file whose `mod` declaration
# was never added). Compiler dep-info (*.d files) is the source of truth, so
# wiring via `mod`, `#[path = ...]`, and `include!` are all recognized without
# heuristics.
#
# Feature-gated module trees that the default feature set cannot compile must
# be listed in scripts/orphan_rs_allow.txt with a justification comment.
#
# Known limitation: a stale dep-info file from a unit that no longer exists
# can hide a file that was wired once and later unwired. The gate is exact for
# never-wired files; run against a clean target dir for a from-scratch proof.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"

cargo check --workspace --all-targets --quiet
# calyx-lodestar's FSV test suites are `#![cfg(feature = "fsv")]`; the feature
# is empty (no extra deps) so it compiles everywhere — check it too so the
# suites' support modules are verified as wired instead of allowlisted.
cargo check -p calyx-lodestar --features fsv --all-targets --quiet

debug_dir="$target_dir/debug"
if [[ ! -d "$debug_dir" ]]; then
    echo "CALYX_ORPHAN_RS_NO_DEPINFO: $debug_dir missing after cargo check;" \
        "CARGO_TARGET_DIR does not match the directory cargo used" >&2
    exit 1
fi

# Windows shells: dep-info holds C:/-style paths, bash holds /c/-style, and
# the filesystem is case-insensitive — normalize both sides identically.
case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    repo_abs="$(cygpath -m "$repo_root")"
    fold_case() { tr '[:upper:]' '[:lower:]'; }
    ;;
*)
    repo_abs="$repo_root"
    fold_case() { cat; }
    ;;
esac

compiled_list="$(mktemp)"
repo_list="$(mktemp)"
trap 'rm -f "$compiled_list" "$repo_list"' EXIT

# Prerequisite lists of every dep-info file → one repo-relative path per
# line. Cargo writes workspace files relative to the workspace root and
# external files (registry deps) absolute; strip the repo prefix from
# absolute entries and keep relative ones as-is. Splitting on ': '
# (colon-space) is safe with Windows drive colons.
repo_prefix="$(printf '%s' "$repo_abs" | fold_case)"
find "$debug_dir" -name '*.d' -type f -print0 |
    xargs -0 sed -n 's/^[^ ]*: //p' |
    tr ' ' '\n' |
    tr '\\' '/' |
    grep '\.rs$' |
    fold_case |
    sed "s|^$repo_prefix/||" |
    sort -u >"$compiled_list"

if [[ ! -s "$compiled_list" ]]; then
    echo "CALYX_ORPHAN_RS_NO_DEPINFO: no dep-info prerequisites found under $debug_dir" >&2
    exit 1
fi

allow_file="$repo_root/scripts/orphan_rs_allow.txt"
is_allowed() {
    local rel="$1"
    [[ -f "$allow_file" ]] || return 1
    while IFS= read -r prefix; do
        [[ -z "$prefix" || "$prefix" == \#* ]] && continue
        [[ "$rel" == "$prefix"* ]] && return 0
    done <"$allow_file"
    return 1
}

git ls-files --cached --others --exclude-standard -- 'crates/' |
    grep -E '^crates/[^/]+/(src|tests)/.*\.rs$' >"$repo_list"

orphans=0
while IFS= read -r rel; do
    folded="$(printf '%s' "$rel" | fold_case)"
    if ! grep -Fxq "$folded" "$compiled_list"; then
        if is_allowed "$rel"; then
            continue
        fi
        echo "CALYX_ORPHAN_RS: $rel exists but is never compiled (not reachable from any module tree)" >&2
        orphans=$((orphans + 1))
    fi
done <"$repo_list"

if [[ "$orphans" -gt 0 ]]; then
    echo "CALYX_ORPHAN_RS: $orphans orphaned .rs file(s); wire them with 'mod'/'#[path]' or delete them" >&2
    exit 1
fi
echo "✅ no orphaned .rs files"
