# shellcheck shell=bash
# repro-build-env.sh — canonical reproducible-build environment for Calyx.
#
# SOURCE this file (do not execute): `source scripts/supply-chain/repro-build-env.sh`
#
# PRD 30 §2 (Dependency / supply chain: "reproducible builds"). Issue #596.
#
# Rationale (web-researched best practice, reproducible-builds.org/docs/rust):
#   1. Pin the toolchain  — already done via rust-toolchain.toml (channel 1.95.0).
#   2. Lock dependencies  — builds use `--locked` so Cargo.lock is authoritative.
#   3. Kill non-determinism sources we control:
#        * CARGO_INCREMENTAL=0  — incremental compilation is not deterministic.
#        * SOURCE_DATE_EPOCH    — any compile-time timestamp reads this, not the
#                                 wall clock (commit time = stable input).
#        * --remap-path-prefix  — strip machine-specific absolute paths
#                                 ($WS_ROOT, $CARGO_HOME) from the emitted
#                                 artifacts so two checkouts in different
#                                 directories still produce identical bytes.
#        * LC_ALL=C / TZ=UTC    — locale- and timezone-independent output.
#
# These live HERE (applied at build time) rather than in .cargo/config.toml on
# purpose: that committed file must stay free of absolute, machine-specific
# paths (see its header), and --remap-path-prefix needs them.

# Resolve the workspace root from git when available, else the current dir.
if _ws_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
  :
else
  _ws_root="$(pwd)"
fi
_cargo_home="${CARGO_HOME:-$HOME/.cargo}"

# Commit time is a stable, content-derived timestamp. Fall back to 0 (epoch) when
# not in a git tree — never the wall clock (that would break reproducibility).
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  if _ct="$(git -C "$_ws_root" log -1 --format=%ct 2>/dev/null)" && [ -n "$_ct" ]; then
    SOURCE_DATE_EPOCH="$_ct"
  else
    SOURCE_DATE_EPOCH=0
  fi
fi
export SOURCE_DATE_EPOCH

export CARGO_INCREMENTAL=0
export LC_ALL=C
export TZ=UTC

_remap="--remap-path-prefix=${_ws_root}=/calyx --remap-path-prefix=${_cargo_home}=/cargo"
export RUSTFLAGS="${RUSTFLAGS:-} ${_remap}"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} ${_remap}"

echo "[repro-build-env] SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
echo "[repro-build-env] CARGO_INCREMENTAL=${CARGO_INCREMENTAL} LC_ALL=${LC_ALL} TZ=${TZ}"
echo "[repro-build-env] RUSTFLAGS=${RUSTFLAGS}"

unset _ws_root _cargo_home _remap _ct
