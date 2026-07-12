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
repo_root="$(cd "$repo_scripts/.." && pwd)"
sync_src="$repo_scripts/public-mirror-sync.sh"
leak_src="$repo_scripts/public-leak-scan.sh"
secret_src="$repo_scripts/secret-scan.sh"
precommit_cfg="$repo_root/.pre-commit-config.yaml"
githook_precommit="$repo_root/.githooks/pre-commit"
real_git="$(command -v git)"

for f in "$sync_src" "$leak_src" "$secret_src" "$precommit_cfg" "$githook_precommit"; do
  [[ -f "$f" ]] || { echo "FSV ERROR: missing $f" >&2; exit 2; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail=0

pass() { echo "  PASS: $1"; }
bad()  { echo "  FAIL: $1" >&2; fail=1; }

fake_bin="$work/bin"
mkdir -p "$fake_bin"
cat >"$fake_bin/gitleaks" <<'FAKE_GITLEAKS'
#!/usr/bin/env bash
set -euo pipefail
mode="${1:-}"
shift || true
case "$mode" in
  dir)
    leak=0
    for arg in "$@"; do
      [[ -d "$arg" ]] || continue
      if grep -R "SECRET_LEAK_TOKEN" "$arg" >/dev/null 2>&1; then
        leak=1
      fi
    done
    if [[ "$leak" -ne 0 ]]; then
      echo "fake gitleaks: leak found" >&2
      exit 1
    fi
    ;;
  git)
    ;;
  *)
    echo "fake gitleaks: unsupported mode $mode" >&2
    exit 2
    ;;
esac
FAKE_GITLEAKS
chmod +x "$fake_bin/gitleaks"
export PATH="$fake_bin:$PATH"

# Builds a synthetic dev repo with the real sync/leak scripts vendored under
# scripts/, plus the caller-provided allowlist body written into crates/.
make_dev() {
  local dev="$1" crate_body="$2"
  mkdir -p "$dev/scripts" "$dev/crates/demo/src" "$dev/docs/readme"
  cp "$sync_src" "$dev/scripts/public-mirror-sync.sh"
  cp "$leak_src" "$dev/scripts/public-leak-scan.sh"
  cp "$secret_src" "$dev/scripts/secret-scan.sh"
  printf '%s\n' "$crate_body" > "$dev/crates/demo/src/lib.rs"
  printf 'public README image contract\n' > "$dev/docs/readme/README.md"
  printf '[workspace]\nmembers=["crates/demo"]\n' > "$dev/Cargo.toml"
  printf 'target/\n' > "$dev/.gitignore"
  git -C "$dev" init -q
  git -C "$dev" config user.email fsv@example.com
  git -C "$dev" config user.name FSV
  git -C "$dev" add -A
  git -C "$dev" commit -qm "dev base"
}

# Builds a synthetic public checkout with a ChrisRoyse/Calyx origin, a stale
# copy of the allowlist file, a forbidden tracked tree (docs/ except the
# README contract, scripts/, hosted workflows), and a tracked README the dev
# side does not have (to prove deletion staging).
make_public() {
  local pub="$1" stale_body="$2"
  mkdir -p "$pub/crates/demo/src" "$pub/docs/readme" "$pub/scripts" \
    "$pub/.github/workflows"
  printf '%s\n' "$stale_body" > "$pub/crates/demo/src/lib.rs"
  printf '[workspace]\nmembers=["crates/demo"]\n' > "$pub/Cargo.toml"
  printf 'target/\n' > "$pub/.gitignore"
  printf 'internal build host notes\n' > "$pub/docs/internal.md"
  printf 'stale public README contract\n' > "$pub/docs/readme/README.md"
  printf 'echo secret\n' > "$pub/scripts/secret.sh"
  printf 'name: stale-hosted-workflow\n' > "$pub/.github/workflows/stale.yml"
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
git -C "$pub1" diff --cached --name-only --diff-filter=D | grep -qx ".github/workflows/stale.yml" \
  && pass "forbidden hosted workflow staged as DELETION" \
  || bad "hosted workflow deletion NOT staged"
git -C "$pub1" ls-files | grep -qx "docs/readme/README.md" \
  && pass "allowed docs/readme/README.md remains tracked" \
  || bad "allowed docs/readme/README.md is not tracked"
staged_contract="$(git -C "$pub1" show :docs/readme/README.md)"
[[ "$staged_contract" == "public README image contract" ]] \
  && pass "README contract content matches dev source of truth" \
  || bad "README contract content wrong: [$staged_contract]"

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
git -C "$pub1" ls-files |
  grep -E '^(docs/|scripts/|\.github/workflows/)' |
  grep -vx 'docs/readme/README.md' >/dev/null \
  && bad "forbidden paths still tracked in index" \
  || pass "no forbidden paths tracked in index except README contract"

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
git -C "$pub3" diff --cached --name-only --diff-filter=D | grep -qx ".github/workflows/stale.yml" \
  && pass "hosted workflow removal staged on no-op allowlist" \
  || bad "hosted workflow removal NOT staged on no-op allowlist"

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
echo "$out4" | grep -q "\.github/workflows/stale.yml" \
  && pass "leak-scan reported hosted workflow path" \
  || bad "leak-scan did not report hosted workflow path"

echo
echo "=================================================================="
echo "SCENARIO 5 - sync copies only tracked allowlist bytes; untracked"
echo "dev scratch/secret files under crates/ must not reach the mirror."
echo "=================================================================="
dev5="$work/dev5"; pub5="$work/pub5"
make_dev  "$dev5" "pub fn demo() -> u32 { 11 }"
make_public "$pub5" "pub fn demo() -> u32 { 4 }"
printf 'const SECRET: &str = \"SECRET_LEAK_TOKEN\";\n' > "$dev5/crates/demo/src/private.rs"
set +e
out5="$( cd "$dev5" && bash scripts/public-mirror-sync.sh --public-dir "$pub5" 2>&1 )"
rc5=$?
set -e
echo "-- sync exit code: $rc5 --"
echo "$out5" | sed 's/^/   /'
[[ $rc5 -eq 0 ]] \
  && pass "sync ignores untracked dev allowlist files" \
  || bad "sync should not copy or scan untracked dev scratch files"
git -C "$pub5" diff --cached --name-only | grep -qx "crates/demo/src/private.rs" \
  && bad "untracked dev file was staged into public mirror" \
  || pass "untracked dev file not staged into public mirror"
[[ ! -e "$pub5/crates/demo/src/private.rs" ]] \
  && pass "untracked dev file absent from public worktree" \
  || bad "untracked dev file copied into public worktree"

echo
echo "=================================================================="
echo "SCENARIO 6 - staged public mirror secrets must fail closed before"
echo "sync claims success."
echo "=================================================================="
dev6="$work/dev6"; pub6="$work/pub6"
make_dev  "$dev6" 'pub const TOKEN: &str = "SECRET_LEAK_TOKEN";'
make_public "$pub6" "pub fn demo() {}"
set +e
out6="$( cd "$dev6" && bash scripts/public-mirror-sync.sh --public-dir "$pub6" 2>&1 )"
rc6=$?
set -e
echo "-- sync exit code: $rc6 --"
echo "$out6" | sed 's/^/   /'
[[ $rc6 -ne 0 ]] \
  && pass "sync failed closed on staged public secret" \
  || bad "sync should fail when staged public mirror contains a secret"
echo "$out6" | grep -q "staged public allowlist sync" \
  && bad "sync wrongly claimed staged success after secret failure" \
  || pass "sync did NOT claim staged success after secret failure"

echo
echo "=================================================================="
echo "SCENARIO 7 - public-leak-scan fails closed on git grep errors."
echo "=================================================================="
dev7="$work/dev7"
make_dev "$dev7" "pub fn demo() {}"
fake_git_grep="$work/fake-git-grep"
mkdir -p "$fake_git_grep"
cat >"$fake_git_grep/git" <<FAKE_GIT_GREP
#!/usr/bin/env bash
if [[ "\${1:-}" == "grep" ]]; then
  echo "fatal: simulated git grep failure" >&2
  exit 128
fi
exec "$real_git" "\$@"
FAKE_GIT_GREP
chmod +x "$fake_git_grep/git"
set +e
out7="$( cd "$dev7" && PATH="$fake_git_grep:$PATH" bash "$leak_src" 2>&1 )"
rc7=$?
set -e
echo "-- leak-scan exit code: $rc7 --"
echo "$out7" | sed 's/^/   /'
[[ $rc7 -eq 2 ]] \
  && pass "git grep plumbing error exits as environment failure" \
  || bad "git grep plumbing error should exit 2"
echo "$out7" | grep -q "git grep failed" \
  && pass "git grep failure reason reported" \
  || bad "git grep failure reason missing"

echo
echo "=================================================================="
echo "SCENARIO 8 - public-leak-scan fails closed on public git ls-files"
echo "errors instead of treating them as no forbidden paths."
echo "=================================================================="
dev8="$work/dev8"; pub8="$work/pub8"
make_dev "$dev8" "pub fn demo() {}"
make_public "$pub8" "pub fn demo() {}"
fake_git_ls="$work/fake-git-ls"
mkdir -p "$fake_git_ls"
cat >"$fake_git_ls/git" <<FAKE_GIT_LS
#!/usr/bin/env bash
if [[ "\${1:-}" == "ls-files" ]]; then
  echo "fatal: simulated git ls-files failure" >&2
  exit 128
fi
exec "$real_git" "\$@"
FAKE_GIT_LS
chmod +x "$fake_git_ls/git"
set +e
out8="$( PATH="$fake_git_ls:$PATH" bash "$leak_src" --public-tree "$pub8" 2>&1 )"
rc8=$?
set -e
echo "-- leak-scan exit code: $rc8 --"
echo "$out8" | sed 's/^/   /'
[[ $rc8 -eq 2 ]] \
  && pass "git ls-files plumbing error exits as environment failure" \
  || bad "git ls-files plumbing error should exit 2"
echo "$out8" | grep -q "git ls-files failed" \
  && pass "git ls-files failure reason reported" \
  || bad "git ls-files failure reason missing"

echo
echo "=================================================================="
echo "SCENARIO 9 - secret-scan temp copies are removed after gitleaks."
echo "=================================================================="
scan_repo="$work/scanrepo"
scan_tmp="$work/scan-tmp"
mkdir -p "$scan_repo" "$scan_tmp"
printf 'clean\n' > "$scan_repo/file.txt"
git -C "$scan_repo" init -q
git -C "$scan_repo" config user.email fsv@example.com
git -C "$scan_repo" config user.name FSV
git -C "$scan_repo" add -A
git -C "$scan_repo" commit -qm "scan base"
( cd "$scan_repo" && TMPDIR="$scan_tmp" bash "$secret_src" )
( cd "$scan_repo" && TMPDIR="$scan_tmp" bash "$secret_src" --staged )
leftover_secret_tmp="$(find "$scan_tmp" -maxdepth 1 -type d -name 'calyx-secret-scan-*' -print -quit)"
[[ -z "$leftover_secret_tmp" ]] \
  && pass "secret-scan worktree/staged temp dirs cleaned" \
  || bad "secret-scan temp dir remained: $leftover_secret_tmp"

echo
echo "=================================================================="
echo "SCENARIO 10 - pre-commit config mirrors the active staged hooks."
echo "=================================================================="
grep -Fq "entry: scripts/secret-scan.sh" "$precommit_cfg" \
  && grep -Fq -- "--staged" "$precommit_cfg" \
  && pass "pre-commit config secret scan is staged like .githooks/pre-commit" \
  || bad "pre-commit config secret scan is not staged"
grep -Fq "entry: scripts/public-leak-scan.sh" "$precommit_cfg" \
  && grep -Fq -- "--cached" "$precommit_cfg" \
  && pass "pre-commit config public leak scan is cached like .githooks/pre-commit" \
  || bad "pre-commit config public leak scan is not cached"
grep -Fq "scripts/secret-scan.sh\" --staged" "$githook_precommit" \
  && grep -Fq "scripts/public-leak-scan.sh\" --cached" "$githook_precommit" \
  && pass ".githooks/pre-commit keeps staged secret/cached public scans" \
  || bad ".githooks/pre-commit staged/cached hook parity missing"

echo
echo "=================================================================="
echo "SCENARIO 11 - tracked dev file beneath an ignored directory must"
echo "still be staged; ignored status must not hide it from the commit."
echo "=================================================================="
dev11="$work/dev11"; pub11="$work/pub11"
make_dev  "$dev11" "pub fn demo() -> u32 { 13 }"
make_public "$pub11" "pub fn demo() -> u32 { 5 }"
mkdir -p "$dev11/crates/demo/src/storage"
printf 'storage/\n' >> "$dev11/.gitignore"
printf 'pub fn persisted_source() -> u32 { 21 }\n' \
  > "$dev11/crates/demo/src/storage/new.rs"
git -C "$dev11" add .gitignore
git -C "$dev11" add -f crates/demo/src/storage/new.rs
git -C "$dev11" commit -qm "add tracked source beneath ignored directory"

set +e
out11="$( cd "$dev11" && bash scripts/public-mirror-sync.sh --public-dir "$pub11" 2>&1 )"
rc11=$?
set -e
echo "-- sync exit code: $rc11 --"
echo "$out11" | sed 's/^/   /'
[[ $rc11 -eq 0 ]] \
  && pass "sync exits 0 when tracked source collides with an ignore rule" \
  || bad "sync should stage tracked ignored source (got $rc11)"
git -C "$pub11" diff --cached --name-only | \
  grep -qx "crates/demo/src/storage/new.rs" \
  && pass "tracked ignored source is STAGED" \
  || bad "tracked ignored source was omitted from the public index"
staged11="$(git -C "$pub11" show :crates/demo/src/storage/new.rs 2>/dev/null || true)"
[[ "$staged11" == "pub fn persisted_source() -> u32 { 21 }" ]] \
  && pass "staged ignored-source bytes match dev" \
  || bad "staged ignored-source bytes do not match dev: [$staged11]"
ignored11="$(
  git -C "$pub11" ls-files --others --ignored --exclude-standard -- \
    crates/demo/src/storage
)"
[[ -z "$ignored11" ]] \
  && pass "no copied allowlist file remains hidden in ignored state" \
  || bad "ignored copied files remain outside the index:"$'\n'"$ignored11"

echo
if [[ $fail -eq 0 ]]; then
  echo "ALL SCENARIOS PASSED"
else
  echo "REGRESSION DETECTED" >&2
fi
exit $fail
