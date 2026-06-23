#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/fsv_pin_binary.sh --bin <path> --root <fsv-root> [--name <file-name>] [--repo <repo>]

Copies an executable into <fsv-root>/bin, chmods the pinned copy read/execute,
writes <name>.pin.json with byte/hash/source metadata, and prints a bounded
summary JSON. By default <fsv-root> must live under /home/croyse/calyx/fsv.
Set CALYX_ALLOW_NON_FSV_PIN_ROOT=1 only for unit/smoke tests.
USAGE
}

bin_path=""
root=""
name=""
repo=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --bin)
      bin_path="${2:-}"
      shift 2
      ;;
    --root)
      root="${2:-}"
      shift 2
      ;;
    --name)
      name="${2:-}"
      shift 2
      ;;
    --repo)
      repo="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [ -z "$bin_path" ] || [ -z "$root" ]; then
  usage
  exit 2
fi
if [ ! -f "$bin_path" ] || [ ! -x "$bin_path" ]; then
  echo "binary is missing or not executable: $bin_path" >&2
  exit 2
fi

root="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$root")"
case "$root" in
  /home/croyse/calyx/fsv/*) ;;
  *)
    if [ "${CALYX_ALLOW_NON_FSV_PIN_ROOT:-0}" != "1" ]; then
      echo "refusing non-FSV root: $root" >&2
      exit 2
    fi
    ;;
esac

if [ -z "$name" ]; then
  name="$(basename "$bin_path")"
fi
case "$name" in
  */*|"")
    echo "invalid pinned binary name: $name" >&2
    exit 2
    ;;
esac

pin_dir="$root/bin"
dest="$pin_dir/$name"
manifest="$pin_dir/$name.pin.json"
mkdir -p "$pin_dir"
tmp="$dest.tmp.$$"
cp "$bin_path" "$tmp"
chmod 0555 "$tmp"
mv -f "$tmp" "$dest"

PIN_SOURCE="$bin_path" PIN_DEST="$dest" PIN_MANIFEST="$manifest" PIN_REPO="$repo" python3 - <<'PY'
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

source = pathlib.Path(os.environ["PIN_SOURCE"])
dest = pathlib.Path(os.environ["PIN_DEST"])
manifest = pathlib.Path(os.environ["PIN_MANIFEST"])
repo = os.environ.get("PIN_REPO", "")

def file_meta(path: pathlib.Path):
    data = path.read_bytes()
    stat = path.stat()
    return {
        "path": str(path),
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "mtime_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(stat.st_mtime)),
        "device": stat.st_dev,
        "inode": stat.st_ino,
    }

def run(args):
    try:
        return subprocess.check_output(args, text=True, stderr=subprocess.DEVNULL).strip()
    except Exception:
        return None

source_meta = file_meta(source)
dest_meta = file_meta(dest)
if source_meta["sha256"] != dest_meta["sha256"]:
    print(
        json.dumps(
            {
                "error": "CALYX_FSV_PIN_BINARY_HASH_MISMATCH",
                "source": source_meta,
                "dest": dest_meta,
            },
            indent=2,
            sort_keys=True,
        ),
        file=sys.stderr,
    )
    sys.exit(3)

repo_meta = None
if repo:
    head = run(["git", "-C", repo, "rev-parse", "HEAD"])
    status = run(["git", "-C", repo, "status", "--short", "--branch"])
    repo_meta = {
        "path": repo,
        "head": head,
        "status": status,
        "clean": status is not None and "\n" not in status and status.startswith("## "),
    }

payload = {
    "format": "calyx-fsv-pinned-binary-v1",
    "pinned_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "source": source_meta,
    "pinned": dest_meta,
    "repo": repo_meta,
}
manifest.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
manifest_meta = file_meta(manifest)
print(
    json.dumps(
        {
            "pinned_binary": dest_meta["path"],
            "binary_bytes": dest_meta["bytes"],
            "binary_sha256": dest_meta["sha256"],
            "manifest": manifest_meta["path"],
            "manifest_bytes": manifest_meta["bytes"],
            "manifest_sha256": manifest_meta["sha256"],
        },
        indent=2,
        sort_keys=True,
    )
)
PY
