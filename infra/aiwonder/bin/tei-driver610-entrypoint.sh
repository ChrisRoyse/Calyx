#!/usr/bin/env bash
# TEI wrapper for aiwonder's driver-610/CUDA-13.3 host.
#
# The upstream phase0 wrapper only matches "CUDA Version" in nvidia-smi. Driver
# 610 prints "CUDA UMD Version", so that wrapper parses an empty version and
# prepends the CUDA 12.9 compat libcuda path. Keep this wrapper fail-closed: if
# the driver version cannot be read or libcuda still resolves from compat, exit.
set -euo pipefail

log() {
    printf '[tei-driver610-entrypoint] %s\n' "$*" >&2
}

die() {
    log "ERROR: $*"
    exit 72
}

require_tool() {
    command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"
}

version_to_int() {
    local version="$1" maj min patch
    IFS=. read -r maj min patch <<< "$version"
    : "${min:=0}"
    : "${patch:=0}"
    [[ "$maj" =~ ^[0-9]+$ ]] || return 1
    [[ "$min" =~ ^[0-9]+$ ]] || return 1
    [[ "$patch" =~ ^[0-9]+$ ]] || return 1
    printf '%d\n' "$((10#$maj * 10000 + 10#$min * 100 + 10#$patch))"
}

cuda_umd_version() {
    nvidia-smi 2>/dev/null \
        | sed -nE 's/.*CUDA (UMD )?Version:[[:space:]]*([0-9]+([.][0-9]+){0,2}).*/\2/p' \
        | head -n 1
}

resolved_path() {
    readlink -f "$1" 2>/dev/null || printf '%s\n' "$1"
}

path_without_compat() {
    local input="${1:-}" compat_dir="$2" compat_real="$3" part part_real out=""
    IFS=: read -r -a parts <<< "$input"
    for part in "${parts[@]}"; do
        [[ -n "$part" ]] || continue
        part_real="$(resolved_path "$part")"
        if [[ "$part" == "$compat_dir" || "$part_real" == "$compat_real" ]]; then
            continue
        fi
        out="${out:+$out:}$part"
    done
    printf '%s\n' "$out"
}

require_tool nvidia-smi
require_tool text-embeddings-router

compat_dir="/usr/local/cuda/compat"
compat_real="$(resolved_path "$compat_dir")"
cuda_version="$(cuda_umd_version)"
[[ -n "$cuda_version" ]] || die "could not parse CUDA UMD version from nvidia-smi"

cuda_int="$(version_to_int "$cuda_version")" || die "invalid CUDA UMD version: $cuda_version"
target_int="$(version_to_int "12.9.1")"

LD_LIBRARY_PATH="$(path_without_compat "${LD_LIBRARY_PATH:-}" "$compat_dir" "$compat_real")"
if [[ "$cuda_int" -lt "$target_int" && -d "$compat_dir" ]]; then
    LD_LIBRARY_PATH="$compat_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
export LD_LIBRARY_PATH

router="$(command -v text-embeddings-router)"
loader_probe="$(LD_LIBRARY_PATH="$LD_LIBRARY_PATH" ldd "$router" 2>/dev/null | grep 'libcuda.so.1' || true)"
[[ -n "$loader_probe" ]] || die "loader probe did not report libcuda.so.1"
if [[ "$loader_probe" == *"/usr/local/cuda"*"/compat/"* ]]; then
    die "libcuda still resolves from compat path: $loader_probe"
fi

log "cuda_umd_version=$cuda_version"
log "libcuda=$loader_probe"
log "LD_LIBRARY_PATH=$LD_LIBRARY_PATH"
exec text-embeddings-router "$@"
