#!/usr/bin/env bash
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

version="${CALYX_NEXTEST_VERSION:-0.9}"
if [[ -z "$version" || "$version" == *"/"* || "$version" == *"\\"* || "$version" == *".."* ]]; then
  echo "ERROR: invalid CALYX_NEXTEST_VERSION: '$version'" >&2
  exit 1
fi

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "ERROR: required tool not found on PATH: $1" >&2
    exit 1
  }
}

require cargo

verify_nextest() {
  local output
  output="$(cargo nextest --version 2>&1)" || {
    echo "ERROR: cargo-nextest is on PATH but cargo nextest --version failed:" >&2
    echo "$output" >&2
    exit 1
  }
  case "$output" in
    cargo-nextest\ *) ;;
    *)
      echo "ERROR: unexpected cargo-nextest version output: $output" >&2
      exit 1
      ;;
  esac
  echo "[nextest] $(command -v cargo-nextest)"
  echo "[nextest] $output"
}

if command -v cargo-nextest >/dev/null 2>&1; then
  verify_nextest
  exit 0
fi

require curl
require tar
require uname

os="$(uname -s)"
arch="$(uname -m)"
case "$os:$arch" in
  Linux:x86_64) asset="linux" ;;
  Linux:aarch64|Linux:arm64) asset="linux-arm" ;;
  Darwin:*) asset="mac" ;;
  MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64) asset="windows-tar" ;;
  MINGW*:aarch64|MSYS*:aarch64|CYGWIN*:aarch64|MINGW*:arm64|MSYS*:arm64|CYGWIN*:arm64) asset="windows-arm-tar" ;;
  MINGW*:i686|MSYS*:i686|CYGWIN*:i686) asset="windows-x86-tar" ;;
  *)
    echo "ERROR: unsupported cargo-nextest install platform: uname -s='$os' uname -m='$arch'" >&2
    echo "ERROR: install cargo-nextest manually from https://nexte.st/docs/installation/pre-built-binaries/ and rerun cargo nextest --version" >&2
    exit 1
    ;;
esac

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
cargo_bin_dir="$cargo_home/bin"
mkdir -p "$cargo_bin_dir"

tmp_dir="$(mktemp -d -t calyx-nextest.XXXXXX)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

url="https://get.nexte.st/$version/$asset"
archive="$tmp_dir/cargo-nextest.tar.gz"
echo "[nextest] downloading $url"
curl -fsSL "$url" -o "$archive"
tar -xzf "$archive" -C "$cargo_bin_dir"
chmod +x "$cargo_bin_dir/cargo-nextest" "$cargo_bin_dir/cargo-nextest.exe" 2>/dev/null || true

if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "ERROR: installed cargo-nextest into '$cargo_bin_dir', but it is not on PATH" >&2
  echo "ERROR: add '$cargo_bin_dir' to PATH and rerun cargo nextest --version" >&2
  exit 1
fi

verify_nextest
