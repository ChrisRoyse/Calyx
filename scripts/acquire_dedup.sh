#!/usr/bin/env bash
# PH69 T06 / issues #556 + #605 - acquire QQP + PAWS dedup corpora,
# checksum-verified against the sha256/bytes pinned below, and emit the
# deterministic FSV pair subset used by dedup_qqp_paws_fsv.rs.
#
#   acquire_dedup.sh              acquire + validate + register QQP + PAWS
#   acquire_dedup.sh --self-test  hermetic synthetic-fixture battery
#
# Every file (including a pre-existing cached one) is verified against the
# pinned sha256 before use - a tampered or drifted byte is a loud
# CALYX_DATASET_CHECKSUM_MISMATCH, never trusted because the size matched.
# Both label partitions (is_duplicate=0 AND =1) must be present in every
# split, or no MANIFEST row is written (PH70 needs the never-merge negatives).
# Fail-closed (A16): any mismatch exits 1 with an exact CALYX_* code.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
QQP_DIR="$DATASET_ROOT/quora_qp"
PAWS_DIR="$DATASET_ROOT/paws"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"

QQP_URL="https://qim.fs.quoracdn.net/quora_duplicate_questions.tsv"
QQP_EXPECTED_ROWS=404290

PAWS_REVISION="161ece9501cf0a11f3e48bd356eaa82de46d6a09"
PAWS_BASE="https://huggingface.co/datasets/google-research-datasets/paws/resolve/$PAWS_REVISION/labeled_final"
PAWS_TRAIN_ROWS=49401
PAWS_DEV_ROWS=8000
PAWS_TEST_ROWS=8000
# 24.0.0 is the first pin verified to ship a cp314 wheel for aiwonder's Python 3.14.
PYARROW_PIN="${CALYX_PYARROW_PIN:-pyarrow==24.0.0}"

# --- pinned file state (sha256/bytes recorded from the verified catalog) -----
# name|url|dest_subpath|bytes|sha256
FILES=(
  "qqp|$QQP_URL|quora_qp/quora_duplicate_questions.tsv|58176133|b3350dbb1d98db1f5abca85d736ba514bf4da253fae53939303d79fe921ec7c8"
  "paws-train|$PAWS_BASE/train-00000-of-00001.parquet|paws/train.parquet|8433884|8dc9ad3e5f30ad9a86b290fe236d528ef23a5751fec9a35d99cbacf68ba277cf"
  "paws-validation|$PAWS_BASE/validation-00000-of-00001.parquet|paws/validation.parquet|1230379|7760d829453764ba342a6f562809a8ed21c2c3eec3fd9ffa544089f145d42f6d"
  "paws-test|$PAWS_BASE/test-00000-of-00001.parquet|paws/test.parquet|1235128|ae342ff12bb84b84b95f468abf5db6cb7c7bd578271299fe9c99be75b8132f4d"
)

fail() {
  echo "$1: $2" >&2
  exit 1
}

resolve_python() {
  if [[ -n "${CALYX_DATASET_PYTHON:-}" ]]; then
    echo "$CALYX_DATASET_PYTHON"
    return
  fi
  if [[ -x "$VENV_DIR/bin/python3" ]]; then
    echo "$VENV_DIR/bin/python3"
    return
  fi
  local candidate
  for candidate in python3 python; do
    if "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info[0] == 3 else 1)' \
        >/dev/null 2>&1; then
      echo "$candidate"
      return
    fi
  done
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: no python3 found - set CALYX_DATASET_PYTHON" >&2
  exit 1
}

# pairs_check <tsv> <expected_rows>: the dedup-pair label contract, shared by
# the production QQP path and the self-test fixtures. Rows must parse with an
# is_duplicate label in {0,1}; BOTH partitions must be non-empty.
# gen_fixture <dir> <case> <seed>: deterministic 6-row synthetic pair TSV.
run_python() {
  local py
  py="$(resolve_python)"
  "$py" - "$@" <<'PY'
import csv
import json
import pathlib
import sys

csv.field_size_limit(2**31 - 1)


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def pairs_check(tsv, expected_rows):
    rows = 0
    dups = 0
    with open(tsv, "r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t", quoting=csv.QUOTE_MINIMAL)
        for row in reader:
            label = row.get("is_duplicate")
            if label not in ("0", "1"):
                fail("CALYX_DATASET_LABEL_INVALID", f"{tsv} row {rows}: label {label!r}")
            dups += int(label)
            rows += 1
    if rows != expected_rows:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{tsv}: rows {rows} != expected {expected_rows}")
    if dups == 0 or dups == rows:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{tsv}: dup_count {dups} of {rows} - both label partitions required")
    print(json.dumps({"rows": rows, "dups": dups}))


def gen_fixture(target_dir, case, seed):
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    count = 5 if case == "short" else 6
    lines = ["id\tqid1\tqid2\tquestion1\tquestion2\tis_duplicate"]
    for i in range(count):
        label = 1 if i < 3 else 0
        if case == "all-dup":
            label = 1
        lines.append(f"{i}\t{2 * i}\t{2 * i + 1}\tq-{seed}-{i}-a\tq-{seed}-{i}-b\t{label}")
    if case == "bad-label":
        parts = lines[3].split("\t")
        parts[-1] = "2"
        lines[3] = "\t".join(parts)
    (target / "pairs.tsv").write_bytes(("\n".join(lines) + "\n").encode())
    print(json.dumps({"case": case, "rows": count}))


mode = sys.argv[1]
if mode == "pairs-check":
    pairs_check(sys.argv[2], int(sys.argv[3]))
elif mode == "gen-fixture":
    gen_fixture(sys.argv[2], sys.argv[3], sys.argv[4])
else:
    fail("CALYX_DATASET_MANIFEST_INVALID", f"unknown python mode {mode!r}")
PY
}

download_verified() {
  local url="$1" dest="$2" expected_bytes="$3" expected_sha="$4"
  if [[ -f "$dest" ]]; then
    local have_sha
    have_sha="$(sha256sum "$dest" | cut -d' ' -f1)"
    if [[ "$have_sha" == "$expected_sha" ]]; then
      echo "  [cached] $dest"
      return 0
    fi
    fail CALYX_DATASET_CHECKSUM_MISMATCH \
      "$dest exists with sha256 $have_sha != pinned $expected_sha - delete it to re-acquire"
  fi
  curl -fsSL --retry 3 --retry-delay 5 "$url" -o "$dest.tmp" \
    || fail CALYX_DATASET_DOWNLOAD_FAILED "$url"
  local actual_bytes actual_sha
  actual_bytes="$(stat -c%s "$dest.tmp")"
  if [[ "$actual_bytes" != "$expected_bytes" ]]; then
    rm -f "$dest.tmp"
    fail CALYX_DATASET_BYTES_MISMATCH "$url: bytes $actual_bytes != pinned $expected_bytes"
  fi
  actual_sha="$(sha256sum "$dest.tmp" | cut -d' ' -f1)"
  if [[ "$actual_sha" != "$expected_sha" ]]; then
    rm -f "$dest.tmp"
    fail CALYX_DATASET_CHECKSUM_MISMATCH "$url: sha256 $actual_sha != pinned $expected_sha"
  fi
  mv "$dest.tmp" "$dest" # same-mount rename, never cross-mount
  echo "  [fetched] $dest ($actual_bytes bytes, sha256 $actual_sha)"
}

acquire_all() {
  # Secrets gate runs before ANY directory is created (fail-closed: a missing
  # token must not leave partial state behind).
  if [[ -z "${HF_HUB_TOKEN:-${HF_TOKEN:-}}" ]]; then
    fail CALYX_SECRET_MISSING "HF_HUB_TOKEN"
  fi
  if [[ ! -d "$DATASET_ROOT" ]]; then
    fail CALYX_DATASET_NOT_FOUND "dataset root missing: $DATASET_ROOT (PH00 ZFS provisioning)"
  fi

  echo "=== download (pinned sources + pre-recorded sha256) ==="
  local spec name url subpath bytes sha
  for spec in "${FILES[@]}"; do
    IFS='|' read -r name url subpath bytes sha <<<"$spec"
    mkdir -p "$DATASET_ROOT/$(dirname "$subpath")"
    download_verified "$url" "$DATASET_ROOT/$subpath" "$bytes" "$sha"
  done

  echo "=== validate (QQP label contract via shared pairs-check) ==="
  run_python pairs-check "$QQP_DIR/quora_duplicate_questions.tsv" "$QQP_EXPECTED_ROWS"

  # --- venv with pinned pyarrow for parquet -> tsv ---
  if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
    python3 -m venv "$VENV_DIR" || fail CALYX_DATASET_VENV_FAILED "python3 -m venv $VENV_DIR"
  fi
  if ! "$VENV_DIR/bin/python3" -c 'import pyarrow' 2>/dev/null; then
    "$VENV_DIR/bin/pip" install --quiet "$PYARROW_PIN" \
      || fail CALYX_DATASET_VENV_FAILED "pip install $PYARROW_PIN"
  fi

  "$VENV_DIR/bin/python3" - "$QQP_DIR" "$PAWS_DIR" "$DATASET_ROOT" <<'PY'
import csv
import hashlib
import json
import pathlib
import sys

import pyarrow.parquet as pq

qqp_dir = pathlib.Path(sys.argv[1])
paws_dir = pathlib.Path(sys.argv[2])
root = pathlib.Path(sys.argv[3])

QQP_EXPECTED_ROWS = 404290
PAWS_EXPECTED = {"train": 49401, "validation": 8000, "test": 8000}
QQP_PER_BUCKET = 256
PAWS_PER_LABEL = 200
MAX_TEXT_CHARS = 1000

def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)

def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()

def sanitize(text):
    return " ".join(text.split())

def text_sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()

# --- QQP parse + label partition check ---
qqp_raw = qqp_dir / "quora_duplicate_questions.tsv"
qqp_rows = []
with qqp_raw.open("r", encoding="utf-8", newline="") as handle:
    reader = csv.DictReader(handle, delimiter="\t", quoting=csv.QUOTE_MINIMAL)
    for row in reader:
        label = row.get("is_duplicate")
        q1 = row.get("question1") or ""
        q2 = row.get("question2") or ""
        if label not in ("0", "1"):
            fail("CALYX_DATASET_LABEL_INVALID", f"qqp row {len(qqp_rows)} label {label!r}")
        qqp_rows.append((row["id"], sanitize(q1), sanitize(q2), int(label)))
if len(qqp_rows) != QQP_EXPECTED_ROWS:
    fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"qqp rows {len(qqp_rows)} != {QQP_EXPECTED_ROWS}")
qqp_dup = sum(1 for r in qqp_rows if r[3] == 1)
if qqp_dup == 0 or qqp_dup == len(qqp_rows):
    fail("CALYX_DATASET_LABEL_PARTITION_MISSING", f"qqp dup_count {qqp_dup}")

# --- PAWS parquet -> tsv ---
paws_meta = {}
for split, expected in PAWS_EXPECTED.items():
    table = pq.read_table(paws_dir / f"{split}.parquet")
    rows = table.to_pylist()
    if len(rows) != expected:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"paws {split} rows {len(rows)} != {expected}")
    dup = sum(1 for r in rows if int(r["label"]) == 1)
    if dup == 0 or dup == len(rows):
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING", f"paws {split} dup_count {dup}")
    out = paws_dir / f"{split}.tsv"
    with out.open("w", encoding="utf-8", newline="") as handle:
        handle.write("id\tsentence1\tsentence2\tlabel\n")
        for r in rows:
            s1, s2 = sanitize(r["sentence1"]), sanitize(r["sentence2"])
            handle.write(f"{r['id']}\t{s1}\t{s2}\t{int(r['label'])}\n")
    paws_meta[split] = {"rows": len(rows), "dup_count": dup, "tsv_sha256": sha256_file(out)}

# --- deterministic FSV pair subset (file order, first-N per bucket) ---
def qqp_buckets():
    buckets = {("calib", 1): [], ("calib", 0): [], ("eval", 1): [], ("eval", 0): []}
    for pair_id, q1, q2, label in qqp_rows:
        if not q1 or not q2 or len(q1) > MAX_TEXT_CHARS or len(q2) > MAX_TEXT_CHARS:
            continue
        for split in ("calib", "eval"):
            bucket = buckets[(split, label)]
            if len(bucket) < QQP_PER_BUCKET:
                bucket.append((split, pair_id, q1, q2, label))
                break
    for key, bucket in buckets.items():
        if len(bucket) != QQP_PER_BUCKET:
            fail("CALYX_DATASET_SUBSET_SHORT", f"qqp bucket {key} has {len(bucket)}")
    return buckets

def paws_bucket():
    rows = []
    counts = {0: 0, 1: 0}
    test_tsv = paws_dir / "test.tsv"
    with test_tsv.open("r", encoding="utf-8") as handle:
        next(handle)
        for line in handle:
            pair_id, s1, s2, label = line.rstrip("\n").split("\t")
            label = int(label)
            if counts[label] >= PAWS_PER_LABEL or len(s1) > MAX_TEXT_CHARS or len(s2) > MAX_TEXT_CHARS:
                continue
            counts[label] += 1
            rows.append(("paws", pair_id, s1, s2, label))
    if counts[0] != PAWS_PER_LABEL or counts[1] != PAWS_PER_LABEL:
        fail("CALYX_DATASET_SUBSET_SHORT", f"paws counts {counts}")
    return rows

fsv_path = root / "dedup_fsv_pairs.tsv"
with fsv_path.open("w", encoding="utf-8", newline="") as handle:
    handle.write("source\tsplit\tpair_id\tlabel\ttext_a_sha256\ttext_b_sha256\ttext_a\ttext_b\n")
    for (split, _), bucket in sorted(qqp_buckets().items(), key=lambda kv: (kv[0][0], -kv[0][1])):
        for _, pair_id, q1, q2, label in bucket:
            handle.write(
                f"qqp\t{split}\t{pair_id}\t{label}\t{text_sha(q1)}\t{text_sha(q2)}\t{q1}\t{q2}\n"
            )
    for source, pair_id, s1, s2, label in paws_bucket():
        handle.write(
            f"{source}\tadversarial\t{pair_id}\t{label}\t{text_sha(s1)}\t{text_sha(s2)}\t{s1}\t{s2}\n"
        )

# --- manifests ---
qqp_manifest = {
    "dataset": "quora_qp",
    "source": "https://qim.fs.quoracdn.net/quora_duplicate_questions.tsv",
    "raw_sha256": sha256_file(qqp_raw),
    "raw_bytes": qqp_raw.stat().st_size,
    "rows": len(qqp_rows),
    "dup_count": qqp_dup,
    "license": "Quora custom / non-commercial research",
    "tests": "TCT cosine-Gtau dedup correctness (PH70 issue #605)",
}
paws_manifest = {
    "dataset": "paws",
    "source": "huggingface:google-research-datasets/paws labeled_final",
    "revision": "161ece9501cf0a11f3e48bd356eaa82de46d6a09",
    "parquet_sha256": {
        split: sha256_file(paws_dir / f"{split}.parquet")
        for split in ("train", "validation", "test")
    },
    "splits": paws_meta,
    "license": "Provided 'AS IS' by Google (PAWS release); free for any purpose",
    "tests": "conflicting-anchor never-merge on adversarial high-overlap pairs (PH70 issue #605)",
}
fsv_sha = sha256_file(fsv_path)
summary = {
    "fsv_pairs": str(fsv_path),
    "fsv_pairs_sha256": fsv_sha,
    "qqp": qqp_manifest,
    "paws": paws_manifest,
}
(root / "dedup_fsv_pairs.manifest.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(summary, sort_keys=True))
PY

  # --- canonical registration: manifest.json + MANIFEST.md row + verify -------
  # verify_dataset.sh register is the single writer of catalog rows (PH69 T01);
  # it recomputes per-file sha256/bytes/rows from the bytes on disk and then
  # byte-verifies its own output. PAWS counts rows from the pinned parquet
  # splits only - the derived *.tsv files hold the same records.
  export CALYX_DATASET_PYTHON="$VENV_DIR/bin/python3"
  bash "$SCRIPT_DIR/verify_dataset.sh" register quora_qp \
    --source "$QQP_URL" \
    --revision "2017-03-06" \
    --license "Quora custom / non-commercial research" \
    --tests "TCT cosine-Gtau dedup correctness (PH70 issue #605)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register paws \
    --source "huggingface:google-research-datasets/paws labeled_final" \
    --revision "$PAWS_REVISION" \
    --license "Provided 'AS IS' by Google (PAWS release); free for any purpose" \
    --tests "conflicting-anchor never-merge on adversarial high-overlap pairs (PH70 issue #605)" \
    --rows-from "*.parquet"

  echo "acquire_dedup: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Known input -> hand-derived expected output. Plain-byte fixture (stdlib
# only); the sha256 below was hand-computed from the literal fixture bytes.
FIXTURE_SHA="fa5bb773c6a96ecd88a7f8d3f9d1e7120d19b4b0bb1f34a433a248b7a1d9eb0b"

self_test() {
  local tmp_root
  tmp_root="$(mktemp -d)"
  trap "rm -rf '$tmp_root'" EXIT
  export CALYX_DATASET_ROOT="$tmp_root"
  DATASET_ROOT="$tmp_root"
  local manifest="$tmp_root/MANIFEST.md"
  local pass=0

  step() { pass=$((pass + 1)); echo "[SELF-TEST $pass] $1"; }
  show_catalog() {
    echo "--- catalog $1 ---"
    if [[ -f "$manifest" ]]; then grep -E '^\| fixture' "$manifest" || echo "(no fixture row)"; else echo "(no MANIFEST.md)"; fi
  }
  expect_fail() {
    local code="$1"; shift
    local err_log="$tmp_root/err.log"
    if "$@" >"$tmp_root/out.log" 2>"$err_log"; then
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

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic 6-row pair TSV (3 dup / 3 non-dup): known counts + pinned sha256"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  local fixture_sha
  fixture_sha="$(sha256sum "$tmp_root/fixture_good/pairs.tsv" | cut -d' ' -f1)"
  if [[ "$fixture_sha" != "$FIXTURE_SHA" ]]; then
    echo "SELF-TEST FAILED: fixture sha256 $fixture_sha != pinned $FIXTURE_SHA (generator drift)" >&2
    exit 1
  fi
  local check_out
  check_out="$(run_python pairs-check "$tmp_root/fixture_good/pairs.tsv" 6)"
  echo "    $check_out"
  [[ "$check_out" == '{"rows": 6, "dups": 3}' ]] \
    || { echo "SELF-TEST FAILED: pairs-check output != hand-computed {rows:6,dups:3}" >&2; exit 1; }

  step "edge 1: only is_duplicate=1 rows -> CALYX_DATASET_LABEL_PARTITION_MISSING, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_alldup" all-dup s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_PARTITION_MISSING \
    bash "$SCRIPT_PATH" --pairs-check "$tmp_root/fixture_alldup/pairs.tsv" 6
  show_catalog "after (must be unchanged)"

  step "edge 2: invalid label value -> CALYX_DATASET_LABEL_INVALID"
  run_python gen-fixture "$tmp_root/fixture_badlabel" bad-label s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_INVALID \
    bash "$SCRIPT_PATH" --pairs-check "$tmp_root/fixture_badlabel/pairs.tsv" 6

  step "edge 3: partial file (5 of 6 rows) -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --pairs-check "$tmp_root/fixture_short/pairs.tsv" 6

  step "edge 4: register then tamper bytes -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_dedup.sh self-test"
  show_catalog "after register"
  printf '99\t198\t199\ttampered\ttampered\t1\n' >> "$tmp_root/fixture_good/pairs.tsv"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH \
    bash "$SCRIPT_DIR/verify_dataset.sh" fixture_good

  step "round-trip property: register->verify green for 3 distinct seeded fixtures"
  local seed
  for seed in s2 s3 s4; do
    run_python gen-fixture "$tmp_root/fixture_rt_$seed" good "$seed" >/dev/null
    bash "$SCRIPT_DIR/verify_dataset.sh" register "fixture_rt_$seed" \
      --source "self-test fixture" --revision "$seed" \
      --license "n/a (synthetic)" --tests "round-trip property" >/dev/null
    bash "$SCRIPT_DIR/verify_dataset.sh" "fixture_rt_$seed"
  done

  echo "[SELF-TEST] all $pass steps passed"
}

case "${1:-acquire}" in
  acquire) acquire_all ;;
  --self-test) self_test ;;
  --pairs-check) shift; run_python pairs-check "$@" ;;
  --gen-fixture) shift; run_python gen-fixture "$@" ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
