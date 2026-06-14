#!/usr/bin/env bash
# verify-reproducible-build.sh — prove a Calyx package builds bit-for-bit
# identically across two independent builds. PRD 30 §2. Issue #596.
#
# Method (reproducible-builds.org/docs/rust): build the package twice into the
# SAME target path (build -> snapshot manifest -> clean -> rebuild), then compare
# a sha256 manifest of every emitted artifact (excluding inherently-volatile
# fingerprint/incremental/depfile state).
#
# The target path must be byte-identical across both builds: build scripts embed
# their OUT_DIR (which lives *under* the target dir) into captured `output` files
# and sometimes into the compiled artifact, and that path is NOT covered by
# --remap-path-prefix. Building into two *different* dirs would inject a spurious
# path difference and falsely report non-reproducibility — so we reuse one path.
#
#   Trigger (X): two `cargo build --locked --release` of identical source.
#   Outcome (Y): byte-identical artifacts  =>  identical manifest sha256.
#
# SoT: the files under <target>/release/ on disk (read back independently with
#      sha256sum), NOT cargo's return code.
#
# Exit codes / fail-closed taxonomy:
#   0                               reproducible (manifests match)
#   CALYX_REPRO_BUILD_MISMATCH (10) artifacts differ between builds
#   CALYX_REPRO_BUILD_FAILED   (11) a cargo build itself failed
#   CALYX_REPRO_NO_ARTIFACTS   (12) no artifacts found to compare
#
# Usage: verify-reproducible-build.sh [package]   (default: calyx-core)
set -euo pipefail

PKG="${1:-calyx-core}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WS_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$WS_ROOT"

# shellcheck source=/dev/null
source "$SCRIPT_DIR/repro-build-env.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
TARGET="$WORK/target"   # one stable path, reused for both builds

manifest() {
  # Emits "<sha256>  <path-relative-to-release>" lines, sorted. Excludes the
  # inherently-volatile fingerprint/incremental/depfile state per
  # reproducible-builds.org guidance.
  local rel="$TARGET/release"
  [ -d "$rel" ] || return 0
  ( cd "$rel"
    find . \
      \( -path './.fingerprint' -o -path './incremental' \) -prune -o \
      -type f ! -name '*.d' ! -name '.cargo-lock' ! -name '.rustc_info.json' -print \
    | LC_ALL=C sort \
    | while IFS= read -r f; do
        sha256sum "$f"
      done )
}

build() {
  # $1 = label for the log file
  echo ">>> build ($1) into $TARGET"
  if ! CARGO_TARGET_DIR="$TARGET" cargo build --locked --release -p "$PKG" >"$WORK/$1.log" 2>&1; then
    echo "CALYX_REPRO_BUILD_FAILED: cargo build -p $PKG failed; tail:" >&2
    tail -30 "$WORK/$1.log" >&2
    exit 11
  fi
}

M1="$WORK/manifest1.txt"
M2="$WORK/manifest2.txt"

build build1
manifest >"$M1"
rm -rf "$TARGET"      # clean so build2 recreates every artifact at the same path
build build2
manifest >"$M2"

N1="$(wc -l <"$M1")"
N2="$(wc -l <"$M2")"
if [ "$N1" -eq 0 ] || [ "$N2" -eq 0 ]; then
  echo "CALYX_REPRO_NO_ARTIFACTS: nothing under release/ to compare (n1=$N1 n2=$N2)" >&2
  exit 12
fi

H1="$(sha256sum <"$M1" | cut -d' ' -f1)"
H2="$(sha256sum <"$M2" | cut -d' ' -f1)"

echo "----------------------------------------------------------------"
echo "package          : $PKG"
echo "artifacts hashed : $N1 (build1) / $N2 (build2)"
echo "manifest sha256  : build1=$H1"
echo "                   build2=$H2"
echo "SOURCE_DATE_EPOCH: ${SOURCE_DATE_EPOCH}"
echo "----------------------------------------------------------------"

if [ "$H1" = "$H2" ] && [ "$N1" = "$N2" ]; then
  echo "REPRODUCIBLE: build1 == build2 (manifest sha256 identical)"
  exit 0
fi

echo "CALYX_REPRO_BUILD_MISMATCH: artifacts differ between builds" >&2
echo "--- diff (build1 vs build2) ---" >&2
diff <(cut -c66- "$M1") <(cut -c66- "$M2") >&2 || true
diff "$M1" "$M2" | head -40 >&2 || true
exit 10
