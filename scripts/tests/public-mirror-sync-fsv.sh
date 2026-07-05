#!/usr/bin/env bash
# Full-state-verification regression test for scripts/public-mirror-sync.sh
# (issue #1195: the sync claimed a staged allowlist sync but left copied files
# unstaged, so an immediate commit produced a deletion-only commit).
#
# This is a REAL git integration test - no mocks. It builds a synthetic dev repo
# and a synthetic public checkout (with a ChrisRoyse/Calyx origin so the guard
# passes), runs the real sync, and then inspects the SOURCE OF TRUTH: the public
# repo's git index (`git diff --cached`) and working-tree status.
#
# Exit 0 = all scenarios pass, 1 = a regression was observed.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_scripts="$(cd "$script_dir/.." && pwd)"
sync_src="$repo_scripts/public-mirror-sync.sh"
leak_src="$repo_scripts/public-leak-scan.sh"

for f in "$sync_src" "$leak_src"; do
  [[ -f "$f" ]] || { echo "FSV ERROR: missing $f" >&2; exit 2; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail=0

pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1" >&2; fail=1; }

# Builds a synthetic dev repo with the real sync/leak scripts vendored under
# scripts/, plus the caller-provided allowlist body written into crates/.
make_dev() {
  local dev="$1" crate_body="$2"
  mkdir -p "$dev/scripts" "$dev/crates/demo/src"
  cp "$sync_src" "$dev/scripts/public-mirror-sync.sh"
  cp "$leak_src" "$dev/scripts/public-leak-scan.sh"
  printf '%s\n' "$crate_body" > "$dev/crates/demo/src/lib.rs"
  printf '[workspace]\nmembers=["crates/demo"]\n' > "$dev/Cargo.toml"
  printf 'target/\n' > "$dev/.gitignore"
  git -C "$dev" init -q
  git -C "$dev" config user.email fsv@example.com
  git -C "$dev" config user.name FSV
  git -C "$dev" add -A
  git -C "$dev" commit -qm "dev base"
}

# Builds a synthetic public checkout with a ChrisRoyse/Calyx origin, a stale
# copy of the allowlist file, a forbidden tracked tree (docs/, scripts/), and a
# tracked README the dev side does not have (to prove deletion staging).
make_public() {
  local pub="$1" stale_body="$2"
  mkdir -p "$pub/crates/demo/src" "$pub/docs" "$pub/scripts"
  printf '%s\n' "$stale_body" > "$pub/crates/demo/src/lib.rs"
  printf '[workspace]\nmembers=["crates/demo"]\n' > "$pub/Cargo.toml"
  printf 'target/\n' > "$pub/.gitignore"
  printf 'internal build host notes\n' > "$pub/docs/internal.md"
  printf 'echo secret\n' > "$pub/scripts/secret.sh"
  printf 'stale public readme\n' > "$pub/README.md"
  git -C "$pub" init -q
  git -C "$pub" config user.email fsv@example.com
  git -C "$pub" config user.name FSV
  git -C "$pub" remote add origin https://github.com/ChrisRoyse/Calyx.git
  git -C "$pub" add -A
  git -C "$pub" commit -qm "public base"
}

echo "=================================================================="
echo "SCENARIO 1 - happy path: changed allowlist file must be staged,"
echo "forbidden tracked paths must be staged as deletions, clean sync."
echo "=================================================================="
dev1="$work/dev1"; pub1="$work/pub1"
make_dev  "$dev1" "pub fn demo() -> u32 { 42 }"
make_public "$pub1" "pub fn demo() -> u32 { 1 }"

echo "-- SOURCE OF TRUTH BEFORE sync (public index vs worktree) --"
git -C "$pub1" status --porcelain || true
echo "   (clean working tree expected above)"

( cd "$dev1" && bash scripts/public-mirror-sync.sh --public-dir "$pub1" ) \
  && sync_rc=0 || sync_rc=$?
echo "-- sync exit code: $sync_rc --"
[[ $sync_rc -eq 0 ]] && pass "sync exited 0 on clean allowlist" \
  || bad "sync should exit 0 on clean allowlist (got $sync_rc)"

echo "-- SOURCE OF TRUTH AFTER sync - staged (git diff --cached) --"
git -C "$pub1" diff --cached --name-status

staged="$(git -C "$pub1" diff --cached --name-only)"
echo "$staged" | grep -qx "crates/demo/src/lib.rs" \
  && pass "changed allowlist file is STAGED" \
  || bad "changed allowlist file was NOT staged (the #1195 bug)"

# Staged content must equal dev's new content - prove the real update landed.
staged_body="$(git -C "$pub1" show :crates/demo/src/lib.rs)"
[[ "$staged_body" == "pub fn demo() -> u32 { 42 }" ]] \
  && pass "staged content matches dev source of truth" \
  || bad "staged content wrong: [$staged_body]"

git -C "$pub1" diff --cached --name-only --diff-filter=D | grep -qx "docs/internal.md" \
  && pass "forbidden docs/internal.md staged as DELETION" \
  || bad "forbidden docs/internal.md deletion NOT staged"
git -C "$pub1" diff --cached --name-only --diff-filter=D | grep -qx "scripts/secret.sh" \
  && pass "forbidden scripts/secret.sh staged as DELETION" \
  || bad "forbidden scripts/secret.sh deletion NOT staged"

# README existed in public but not dev -> allowlist deletion must be staged.
git -C "$pub1" diff --cached --name-only --diff-filter=D | grep -qx "README.md" \
  && pass "allowlisted-but-removed README.md staged as DELETION" \
  || bad "README.md deletion NOT staged"

# The decisive invariant: NOTHING unstaged/untracked remains. An immediate
# commit must therefore capture the full sync, not a deletion-only commit.
leftover="$(git -C "$pub1" status --porcelain | grep -E '^( .|\?\?)' || true)"
[[ -z "$leftover" ]] \
  && pass "no unstaged/untracked leftovers - commit-ready" \
  || bad "unstaged/untracked leftovers remain:"$'\n'"$leftover"

# Forbidden paths must be gone from the tracked set (index).
git -C "$pub1" ls-files | grep -qE '^(docs/|scripts/)' \
  && bad "forbidden paths still tracked in index" \
  || pass "no forbidden paths tracked in index (leak gate satisfied)"

# Simulate the operator's immediate commit and prove it is NOT deletion-only.
git -C "$pub1" commit -qm "Sync public mirror"
if git -C "$pub1" show --stat HEAD | grep -q "crates/demo/src/lib.rs"; then
  pass "operator commit includes the allowlist update (not deletion-only)"
else
  bad "operator commit was deletion-only - #1195 regression"
fi

echo
echo "=================================================================="
echo "SCENARIO 2 - edge: internal identifier in allowlist must FAIL CLOSED"
echo "(leak scan rejects; sync must not claim a staged sync)."
echo "=================================================================="
dev2="$work/dev2"; pub2="$work/pub2"
make_dev  "$dev2" "pub fn demo() { let host = \"aiwonder\"; }"  # forbidden token
make_public "$pub2" "pub fn demo() {}"

set +e
out2="$( cd "$dev2" && bash scripts/public-mirror-sync.sh --public-dir "$pub2" 2>&1 )"
rc2=$?
set -e
echo "-- sync exit code: $rc2 --"
echo "$out2" | sed 's/^/   /'
[[ $rc2 -ne 0 ]] \
  && pass "sync failed closed on forbidden identifier" \
  || bad "sync should have failed on forbidden identifier"
echo "$out2" | grep -q "staged public allowlist sync" \
  && bad "sync wrongly claimed a staged sync while failing" \
  || pass "sync did NOT claim a staged sync on failure"

echo
echo "=================================================================="
echo "SCENARIO 3 - edge: no allowlist change at all -> still clean & staged,"
echo "no false unstaged leftovers, sync exits 0."
echo "=================================================================="
dev3="$work/dev3"; pub3="$work/pub3"
make_dev  "$dev3" "pub fn demo() -> u32 { 7 }"
make_public "$pub3" "pub fn demo() -> u32 { 7 }"   # identical allowlist body
set +e
out3="$( cd "$dev3" && bash scripts/public-mirror-sync.sh --public-dir "$pub3" 2>&1 )"
rc3=$?
set -e
echo "-- sync exit code: $rc3 --"
[[ $rc3 -eq 0 ]] \
  && pass "sync exited 0 with identical allowlist" \
  || { echo "$out3" | sed 's/^/   /'; bad "sync should exit 0 (got $rc3)"; }
leftover3="$(git -C "$pub3" status --porcelain | grep -E '^( .|\?\?)' || true)"
[[ -z "$leftover3" ]] \
  && pass "no unstaged/untracked leftovers on no-op allowlist" \
  || bad "leftovers on no-op sync:"$'\n'"$leftover3"
# Forbidden removals are still staged even when allowlist is unchanged.
git -C "$pub3" diff --cached --name-only --diff-filter=D | grep -qx "docs/internal.md" \
  && pass "forbidden removal staged even on no-op allowlist" \
  || bad "forbidden removal NOT staged on no-op allowlist"

echo
echo "=================================================================="
echo "SCENARIO 4 - leak gate: after allowlist staging, forbidden tracked"
echo "public paths still make public-leak-scan fail closed."
echo "=================================================================="
dev4="$work/dev4"; pub4="$work/pub4"
make_dev  "$dev4" "pub fn demo() -> u32 { 9 }"
make_public "$pub4" "pub fn demo() -> u32 { 3 }"
cp "$dev4/crates/demo/src/lib.rs" "$pub4/crates/demo/src/lib.rs"
git -C "$pub4" add -A -- crates/demo/src/lib.rs
set +e
out4="$(bash "$leak_src" --public-tree "$pub4" 2>&1)"
rc4=$?
set -e
echo "-- leak-scan exit code: $rc4 --"
echo "$out4" | sed 's/^/   /'
[[ $rc4 -ne 0 ]] \
  && pass "public-leak-scan failed closed with forbidden tracked path present" \
  || bad "public-leak-scan should fail when forbidden tracked path remains"
echo "$out4" | grep -q "forbidden tracked path" \
  && pass "leak-scan reported forbidden tracked path reason" \
  || bad "leak-scan did not report forbidden tracked path reason"

echo
if [[ $fail -eq 0 ]]; then
  echo "ALL SCENARIOS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit $fail
