#!/usr/bin/env bash
# PH69 T01 / issue #551 - canonical dataset MANIFEST verification + registration.
#
#   verify_dataset.sh <name>            verify one dataset against the MANIFEST
#   verify_dataset.sh ALL               verify every MANIFEST row + dir coverage
#   verify_dataset.sh register <name> \
#       --source S --revision R --license L --tests T [--rows-from g1,g2]
#                                       compute manifest.json + MANIFEST.md row
#                                       from the bytes on disk, then verify
#   verify_dataset.sh --self-test       synthetic known-I/O + edge-case battery
#
# Source of Truth: $CALYX_DATASET_ROOT/MANIFEST.md (catalog, one row per dataset)
# and $CALYX_DATASET_ROOT/<name>/manifest.json (per-file sha256/bytes/rows).
# The dataset-level sha256 is the digest of the sorted per-file hash lines
# "<sha256>  <relpath>\n" over all data files (everything except manifest.json,
# hidden entries, and *.tmp) - the standard signed-manifest / sorted-file-list
# pattern, so any added/removed/edited byte changes the dataset sha256.
#
# Fail-closed: any mismatch prints an exact CALYX_* code on stderr and exits 1.
# No fallbacks: a malformed catalog/manifest is an error, never skipped.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"

# Interpreter resolution (documented order, hard error at the end - no silent skip):
# 1. $CALYX_DATASET_PYTHON  2. dataset-root tools venv  3. python3 on PATH
# 4. python on PATH if it is a Python 3.
resolve_python() {
  if [[ -n "${CALYX_DATASET_PYTHON:-}" ]]; then
    echo "$CALYX_DATASET_PYTHON"
    return
  fi
  if [[ -x "$DATASET_ROOT/.dataset_tools_venv/bin/python3" ]]; then
    echo "$DATASET_ROOT/.dataset_tools_venv/bin/python3"
    return
  fi
  # Probe-execute each candidate: on Windows a `python3` App-Store stub exists
  # on PATH but is not a real interpreter, so `command -v` alone is not proof.
  local candidate
  for candidate in python3 python; do
    if "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info[0] == 3 else 1)' \
        >/dev/null 2>&1; then
      echo "$candidate"
      return
    fi
  done
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: no python3 found - install python3 or set CALYX_DATASET_PYTHON" >&2
  exit 1
}

run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import csv
import fnmatch
import hashlib
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])
MANIFEST_MD = ROOT / "MANIFEST.md"
HEADER = "| name | source | revision | sha256 | rows | bytes | license | tests |"
SEPARATOR = "|---|---|---|---|---|---|---|---|"
PREAMBLE = [
    "# Calyx dataset MANIFEST",
    "",
    "<!-- template: name=dataset dir under $CALYX_DATASET_ROOT | source=URL or",
    "     huggingface:<org>/<repo> <config> | revision=pinned commit/version |",
    "     sha256=digest of sorted '<file_sha256>  <relpath>' lines over all data",
    "     files | rows=sum of record counts of counted files (csv/tsv: records",
    "     minus header, jsonl: non-empty lines, parquet: num_rows) | bytes=sum",
    "     of data-file sizes | license=upstream license | tests=what it tests.",
    "     Rows are machine-written by scripts/verify_dataset.sh register - do",
    "     not hand-edit. -->",
]
ROW_KINDS = {".csv": "csv", ".tsv": "tsv", ".jsonl": "jsonl", ".parquet": "parquet"}
COLUMNS = ["name", "source", "revision", "sha256", "rows", "bytes", "license", "tests"]

# 2 GiB - 1: the largest value accepted on every platform (Windows C long is
# 32-bit, so sys.maxsize raises OverflowError there).
csv.field_size_limit(2**31 - 1)


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def data_files(ds_dir):
    """Sorted relative POSIX paths of all data files (deterministic walk)."""
    found = []
    for path in ds_dir.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(ds_dir).as_posix()
        parts = rel.split("/")
        if any(part.startswith(".") for part in parts):
            continue
        if rel == "manifest.json" or rel.endswith(".tmp"):
            continue
        found.append(rel)
    return sorted(found)


def count_rows(path, kind):
    # Row counts are derived from bytes that may be corrupt (truncated download,
    # bit rot). Reader exceptions are byte-integrity failures, so they map to
    # the closed catalog as CHECKSUM_MISMATCH - never a raw traceback.
    if kind in ("csv", "tsv"):
        delimiter = "," if kind == "csv" else "\t"
        try:
            with path.open("r", encoding="utf-8", newline="") as handle:
                records = sum(1 for _ in csv.reader(handle, delimiter=delimiter))
        except (UnicodeDecodeError, csv.Error, OSError) as err:
            fail("CALYX_DATASET_CHECKSUM_MISMATCH", f"{path}: unreadable {kind} data: {err}")
        if records == 0:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{path}: empty {kind} file (no header)")
        return records - 1
    if kind == "jsonl":
        try:
            with path.open("r", encoding="utf-8") as handle:
                return sum(1 for line in handle if line.strip())
        except (UnicodeDecodeError, OSError) as err:
            fail("CALYX_DATASET_CHECKSUM_MISMATCH", f"{path}: unreadable jsonl data: {err}")
    if kind == "parquet":
        try:
            import pyarrow.parquet as pq
        except ImportError:
            fail(
                "CALYX_DATASET_TOOLCHAIN_MISSING",
                f"{path}: pyarrow required for parquet row counts - "
                "run scripts/acquire_datasets.sh once or set CALYX_DATASET_PYTHON to a venv with pyarrow",
            )
        try:
            return pq.ParquetFile(path).metadata.num_rows
        except Exception as err:  # ArrowInvalid etc.: truncated/corrupt parquet bytes
            fail("CALYX_DATASET_CHECKSUM_MISMATCH", f"{path}: corrupt parquet: {err}")
    fail("CALYX_DATASET_MANIFEST_INVALID", f"{path}: unknown row_kind {kind!r}")


def dataset_digest(entries):
    """entries: [(relpath, file_sha256)] - digest of the sorted hash lines."""
    body = "".join(f"{sha}  {rel}\n" for rel, sha in sorted(entries))
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def scan_dataset(name, rows_from, require_match=False):
    """Recompute every per-file fact from the bytes on disk."""
    ds_dir = ROOT / name
    if not ds_dir.is_dir():
        fail("CALYX_DATASET_NOT_FOUND", f"dataset dir missing: {ds_dir}")
    rels = data_files(ds_dir)
    if not rels:
        fail("CALYX_DATASET_NOT_FOUND", f"dataset dir has no data files: {ds_dir}")
    # rows_from semantics: None = no filter (count every countable file);
    # a list (even empty) = count ONLY matches. The distinction matters when
    # --rows-from matched only non-countable files (e.g. a .zip): verify must
    # reconstruct "count nothing", not fall back to counting everything.
    if rows_from is not None:
        matched = {rel for rel in rels for glob in rows_from if fnmatch.fnmatch(rel, glob)}
        if require_match and not matched:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"--rows-from {rows_from} matched no files in {ds_dir}")
    files = []
    for rel in rels:
        path = ds_dir / rel
        kind = ROW_KINDS.get(path.suffix, "none")
        counted = kind != "none" and (rows_from is None or rel in matched)
        files.append(
            {
                "path": rel,
                "sha256": sha256_file(path),
                "bytes": path.stat().st_size,
                "row_kind": kind,
                "rows": count_rows(path, kind) if counted else None,
                "counted": counted,
            }
        )
    return files


def aggregates(files):
    return {
        "sha256": dataset_digest([(f["path"], f["sha256"]) for f in files]),
        "rows": sum(f["rows"] for f in files if f["counted"]),
        "bytes": sum(f["bytes"] for f in files),
    }


def read_catalog():
    """MANIFEST.md -> (preamble_lines, {name: row_dict}). Strict parse."""
    if not MANIFEST_MD.is_file():
        fail("CALYX_DATASET_MANIFEST_INVALID", f"catalog missing: {MANIFEST_MD}")
    lines = MANIFEST_MD.read_text(encoding="utf-8").splitlines()
    if HEADER not in lines:
        fail("CALYX_DATASET_MANIFEST_INVALID", f"{MANIFEST_MD}: canonical header row not found")
    at = lines.index(HEADER)
    preamble = lines[:at]
    if at + 1 >= len(lines) or lines[at + 1] != SEPARATOR:
        fail("CALYX_DATASET_MANIFEST_INVALID", f"{MANIFEST_MD}: separator row missing after header")
    rows = {}
    for line in lines[at + 2 :]:
        if not line.strip():
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if not line.startswith("|") or len(cells) != len(COLUMNS):
            fail("CALYX_DATASET_MANIFEST_INVALID", f"{MANIFEST_MD}: malformed row: {line!r}")
        row = dict(zip(COLUMNS, cells))
        if row["name"] in rows:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"{MANIFEST_MD}: duplicate row for {row['name']!r}")
        rows[row["name"]] = row
    return preamble, rows


def write_catalog(preamble, rows):
    lines = (preamble or PREAMBLE) + [HEADER, SEPARATOR]
    for name in sorted(rows):
        row = rows[name]
        lines.append("| " + " | ".join(str(row[col]) for col in COLUMNS) + " |")
    tmp = MANIFEST_MD.with_suffix(".md.tmp")
    tmp.write_text("\n".join(lines) + "\n", encoding="utf-8")
    os.replace(tmp, MANIFEST_MD)


def read_manifest_json(name):
    path = ROOT / name / "manifest.json"
    if not path.is_file():
        fail("CALYX_DATASET_MANIFEST_INVALID", f"manifest.json missing: {path}")
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        fail("CALYX_DATASET_MANIFEST_INVALID", f"{path}: invalid JSON: {err}")
    for key in ("schema_version", "name", "source", "revision", "license", "tests",
                "sha256", "rows", "bytes", "files"):
        if key not in manifest:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"{path}: missing key {key!r}")
    if manifest["name"] != name:
        fail("CALYX_DATASET_MANIFEST_INVALID", f"{path}: name {manifest['name']!r} != {name!r}")
    return manifest


def verify_one(name):
    _, rows = read_catalog()
    if name not in rows:
        fail("CALYX_DATASET_NOT_FOUND", f"no MANIFEST.md row for {name!r} in {MANIFEST_MD}")
    row = rows[name]
    if not (ROOT / name).is_dir():
        fail("CALYX_DATASET_NOT_FOUND", f"dataset dir missing: {ROOT / name}")
    manifest = read_manifest_json(name)
    # Reproduce the register-time counting decisions exactly: the recorded
    # counted paths ARE the filter. An empty list (register --rows-from
    # matched only non-countable files) must stay an empty filter - the old
    # `or None` collapse made verify count files register had excluded
    # (first hit: voxceleb1, #557).
    rows_from = [f["path"] for f in manifest["files"] if f["counted"] and f["row_kind"] != "none"]
    actual_files = scan_dataset(name, rows_from)
    expected_by_path = {f["path"]: f for f in manifest["files"]}
    actual_by_path = {f["path"]: f for f in actual_files}
    missing = sorted(set(expected_by_path) - set(actual_by_path))
    extra = sorted(set(actual_by_path) - set(expected_by_path))
    if missing or extra:
        fail(
            "CALYX_DATASET_CHECKSUM_MISMATCH",
            f"{name}: file set drift - missing on disk: {missing or 'none'}, "
            f"unrecorded on disk: {extra or 'none'}",
        )
    for path, expected in expected_by_path.items():
        actual = actual_by_path[path]
        for key in ("sha256", "bytes"):
            if actual[key] != expected[key]:
                fail(
                    "CALYX_DATASET_CHECKSUM_MISMATCH",
                    f"{name}/{path}: {key} {actual[key]} != recorded {expected[key]}",
                )
        if actual["rows"] != expected["rows"]:
            fail(
                "CALYX_DATASET_ROWCOUNT_MISMATCH",
                f"{name}/{path}: rows {actual['rows']} != recorded {expected['rows']}",
            )
    agg = aggregates(actual_files)
    for source, expected in (("manifest.json", manifest), ("MANIFEST.md", row)):
        for key, code in (
            ("sha256", "CALYX_DATASET_CHECKSUM_MISMATCH"),
            ("rows", "CALYX_DATASET_ROWCOUNT_MISMATCH"),
            ("bytes", "CALYX_DATASET_CHECKSUM_MISMATCH"),
        ):
            if str(agg[key]) != str(expected[key]):
                fail(code, f"{name}: {source} {key} {expected[key]} != recomputed {agg[key]}")
    for key in ("source", "revision", "license", "tests"):
        if row[key] != str(manifest[key]):
            fail(
                "CALYX_DATASET_MANIFEST_INVALID",
                f"{name}: MANIFEST.md {key} {row[key]!r} != manifest.json {manifest[key]!r}",
            )
    print(f"[OK] {name} files={len(actual_files)} rows={agg['rows']} bytes={agg['bytes']} sha256={agg['sha256']}")


def verify_all():
    _, rows = read_catalog()
    if not rows:
        fail("CALYX_DATASET_NOT_FOUND", f"{MANIFEST_MD}: catalog has no dataset rows")
    failures = []
    for name in sorted(rows):
        try:
            verify_one(name)
        except SystemExit:
            failures.append(name)
    on_disk = sorted(
        p.name for p in ROOT.iterdir() if p.is_dir() and not p.name.startswith(".")
    )
    unregistered = [name for name in on_disk if name not in rows]
    if unregistered:
        print(
            f"CALYX_DATASET_NOT_FOUND: dataset dirs without a MANIFEST.md row: {unregistered} - "
            "register via scripts/verify_dataset.sh register",
            file=sys.stderr,
        )
        failures.extend(unregistered)
    if failures:
        fail("CALYX_DATASET_CHECKSUM_MISMATCH", f"ALL: {len(failures)} dataset(s) failed: {sorted(set(failures))}")
    print(f"[OK] ALL ({len(rows)} datasets)")


def register(name, meta, rows_from):
    for key, value in meta.items():
        if "|" in value or "\n" in value:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"--{key} must not contain '|' or newline: {value!r}")
    files = scan_dataset(name, rows_from, require_match=True)
    agg = aggregates(files)
    manifest = {
        "schema_version": 1,
        "name": name,
        **meta,
        **agg,
        "files": files,
    }
    ds_manifest = ROOT / name / "manifest.json"
    tmp = ds_manifest.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(tmp, ds_manifest)
    if MANIFEST_MD.is_file():
        preamble, rows = read_catalog()
    else:
        preamble, rows = None, {}
    rows[name] = {"name": name, **meta, **{k: str(v) for k, v in agg.items()}}
    write_catalog(preamble, rows)
    print(f"[REGISTERED] {name} files={len(files)} rows={agg['rows']} bytes={agg['bytes']} sha256={agg['sha256']}")
    verify_one(name)


def main():
    args = sys.argv[1:]
    if args[0] == "verify":
        verify_all() if args[1] == "ALL" else verify_one(args[1])
        return
    if args[0] == "register":
        name = args[1]
        opts = {}
        rest = args[2:]
        if len(rest) % 2 != 0:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"register: dangling option in {rest}")
        for flag, value in zip(rest[::2], rest[1::2]):
            opts[flag] = value
        required = ["--source", "--revision", "--license", "--tests"]
        missing = [flag for flag in required if flag not in opts]
        if missing:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"register {name}: missing {missing}")
        unknown = [flag for flag in opts if flag not in required + ["--rows-from"]]
        if unknown:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"register {name}: unknown option(s) {unknown}")
        meta = {flag.lstrip("-"): opts[flag] for flag in required}
        rows_from = opts.get("--rows-from")
        register(name, meta, rows_from.split(",") if rows_from else None)
        return
    fail("CALYX_DATASET_MANIFEST_INVALID", f"unknown mode {args[0]!r}")


main()
PY
}

# --- self-test: synthetic known-I/O fixture + edge-case battery -------------
# Known input -> hand-computed expected output (the 2+2=4 discipline). The
# constants below were computed by hand from the literal fixture bytes; if the
# digest algorithm ever drifts, this fails loudly.
EXPECTED_FILE_SHA="5690d17928402728fe857961dddf327a04a2e45c42bfa1d2721a8859d3c096b4"
EXPECTED_DS_SHA="ed59b3298d0d2c5c56bfbb30c2438f87c54859c1ee785ea6020649421564e8d3"

self_test() {
  local tmp_root
  tmp_root="$(mktemp -d)"
  trap "rm -rf '$tmp_root'" EXIT
  export CALYX_DATASET_ROOT="$tmp_root"
  DATASET_ROOT="$tmp_root"
  local manifest="$tmp_root/MANIFEST.md"
  local pass=0

  step() { pass=$((pass + 1)); echo "[SELF-TEST $pass] $1"; }
  run_self() {
    bash "$SCRIPT_PATH" "$@"
  }
  show_sot() {
    echo "--- SoT $1 ---"
    if [[ -f "$manifest" ]]; then grep -E '^\| synthetic_fixture' "$manifest" || echo "(no row)"; else echo "(no MANIFEST.md)"; fi
  }
  expect_fail() {
    local code="$1"; shift
    local err_log="$tmp_root/err.log"
    if run_self "$@" >"$tmp_root/out.log" 2>"$err_log"; then
      echo "SELF-TEST FAILED: expected $code but command succeeded: $*" >&2
      exit 1
    fi
    if ! grep -q "^$code:" "$err_log"; then
      echo "SELF-TEST FAILED: expected $code, stderr was:" >&2
      cat "$err_log" >&2
      exit 1
    fi
    echo "    got expected $code"
  }

  step "register synthetic 3-row CSV fixture (known input)"
  mkdir -p "$tmp_root/synthetic_fixture"
  printf 'id,value\n1,alpha\n2,beta\n3,gamma\n' > "$tmp_root/synthetic_fixture/data.csv"
  show_sot "before register"
  run_self register synthetic_fixture \
    --source "self-test inline fixture" --revision "v1" \
    --license "n/a (synthetic)" --tests "verify_dataset.sh self-test"
  show_sot "after register"

  step "hand-computed expected hashes match the registered row (2+2=4 check)"
  grep -q "| $EXPECTED_DS_SHA | 3 | 32 |" "$manifest" \
    || { echo "SELF-TEST FAILED: MANIFEST row != hand-computed digest/rows/bytes" >&2; grep synthetic_fixture "$manifest" >&2; exit 1; }
  grep -q "\"sha256\": \"$EXPECTED_FILE_SHA\"" "$tmp_root/synthetic_fixture/manifest.json" \
    || { echo "SELF-TEST FAILED: manifest.json file sha != hand-computed" >&2; exit 1; }

  step "verify passes and is idempotent (same stdout + exit twice)"
  run_self synthetic_fixture > "$tmp_root/run1.log"
  run_self synthetic_fixture > "$tmp_root/run2.log"
  cmp "$tmp_root/run1.log" "$tmp_root/run2.log"
  grep -q '^\[OK\] synthetic_fixture ' "$tmp_root/run1.log"

  step "edge 1: tampered MANIFEST.md sha256 -> CALYX_DATASET_CHECKSUM_MISMATCH"
  show_sot "before tamper"
  sed -i.bak "s/$EXPECTED_DS_SHA/0000000000000000000000000000000000000000000000000000000000000000/" "$manifest"
  show_sot "after tamper"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH synthetic_fixture
  mv "$manifest.bak" "$manifest"
  show_sot "after restore"

  step "edge 2: correct sha256 but wrong MANIFEST.md rows -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  show_sot "before tamper"
  sed -i.bak "s/| 3 | 32 |/| 999 | 32 |/" "$manifest"
  show_sot "after tamper"
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH synthetic_fixture
  mv "$manifest.bak" "$manifest"

  step "edge 3: data bytes tampered on disk -> CALYX_DATASET_CHECKSUM_MISMATCH"
  echo "4,delta" >> "$tmp_root/synthetic_fixture/data.csv"
  echo "--- data.csv after tamper ---"; cat "$tmp_root/synthetic_fixture/data.csv"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH synthetic_fixture
  printf 'id,value\n1,alpha\n2,beta\n3,gamma\n' > "$tmp_root/synthetic_fixture/data.csv"
  run_self synthetic_fixture >/dev/null

  step "edge 3b: undecodable bytes in data file -> CALYX_DATASET_CHECKSUM_MISMATCH (not a traceback)"
  printf '\xff\xfe\x00garbage' > "$tmp_root/synthetic_fixture/data.csv"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH synthetic_fixture
  if grep -q 'Traceback' "$tmp_root/err.log"; then
    echo "SELF-TEST FAILED: corrupt data produced a raw traceback instead of a closed-catalog code" >&2
    exit 1
  fi
  printf 'id,value\n1,alpha\n2,beta\n3,gamma\n' > "$tmp_root/synthetic_fixture/data.csv"
  run_self synthetic_fixture >/dev/null

  step "edge 4: dataset dir missing -> CALYX_DATASET_NOT_FOUND"
  mv "$tmp_root/synthetic_fixture" "$tmp_root/.hidden_fixture"
  expect_fail CALYX_DATASET_NOT_FOUND synthetic_fixture
  mv "$tmp_root/.hidden_fixture" "$tmp_root/synthetic_fixture"

  step "edge 5: unknown dataset name -> CALYX_DATASET_NOT_FOUND"
  expect_fail CALYX_DATASET_NOT_FOUND no_such_dataset

  step "edge 6: corrupt manifest.json -> CALYX_DATASET_MANIFEST_INVALID"
  cp "$tmp_root/synthetic_fixture/manifest.json" "$tmp_root/mj.bak"
  echo '{not json' > "$tmp_root/synthetic_fixture/manifest.json"
  expect_fail CALYX_DATASET_MANIFEST_INVALID synthetic_fixture
  mv "$tmp_root/mj.bak" "$tmp_root/synthetic_fixture/manifest.json"

  step "edge 7: unregistered dataset dir fails ALL coverage -> CALYX_DATASET_NOT_FOUND"
  mkdir -p "$tmp_root/orphan_dataset"
  echo "data" > "$tmp_root/orphan_dataset/blob.bin"
  expect_fail CALYX_DATASET_NOT_FOUND ALL
  rm -rf "$tmp_root/orphan_dataset"

  step "edge 8: --rows-from excludes every countable file -> rows=0 AND verify stays green (#557 regression)"
  mkdir -p "$tmp_root/archive_fixture"
  printf 'id,label
1,a
2,b
' > "$tmp_root/archive_fixture/meta.csv"
  printf 'not-countable-binary' > "$tmp_root/archive_fixture/payload.zip"
  run_self register archive_fixture     --source "self-test" --revision "rows-from-excludes-countable"     --license "n/a" --tests "verify must reproduce register-time counting"     --rows-from "payload.zip"
  grep -E '^\| archive_fixture \|' "$manifest" | grep -q '| 0 |'     || { echo "SELF-TEST FAILED: archive_fixture rows != 0 in MANIFEST" >&2; exit 1; }
  run_self archive_fixture     || { echo "SELF-TEST FAILED: verify red after register with excluded countable file (rows_from empty-list collapse)" >&2; exit 1; }

  step "final: verify ALL green"
  run_self ALL
  echo "[SELF-TEST] all $pass steps passed"
}

case "${1:-}" in
  "")
    echo "usage: verify_dataset.sh <name|ALL> | register <name> --source S --revision R --license L --tests T [--rows-from g1,g2] | --self-test" >&2
    exit 1
    ;;
  --self-test) self_test ;;
  register) shift; run_python register "$@" ;;
  *) run_python verify "$1" ;;
esac
