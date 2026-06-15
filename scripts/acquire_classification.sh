#!/usr/bin/env bash
# PH69 T03 / issue #553 - acquire the five text-classification corpora
# (ag_news / imdb / sst2 / banking77 / dbpedia_14) as pinned-revision parquet,
# verify every file against the sha256/bytes recorded here BEFORE download
# (HF LFS API metadata at the pinned commit; banking77 convert-ref blobs were
# hashed at pin time), validate per-split row counts and label domains, then
# register each dataset in the canonical MANIFEST via verify_dataset.sh
# register - the single catalog writer (PH69 T01, issue #551).
#
#   acquire_classification.sh              acquire + validate + register all 5
#   acquire_classification.sh --self-test  hermetic synthetic-fixture battery
#
# Revision-pin policy: every source is addressed by an immutable HF commit
# sha, so a silent upstream update can never change what we download; the
# recorded sha256 then proves the bytes that arrived are the bytes that were
# pinned. PolyAI/banking77 publishes only a loading script on main, so its pin
# is the auto-converted parquet ref (refs/convert/parquet commit).
#
# Label-domain policy (per split, exact): labeled splits must contain exactly
# their full class domain with zero nulls; sst2/test and imdb/unsupervised are
# unlabeled BY DESIGN upstream (every label is -1, GLUE hidden-test
# convention) and are pinned to the domain {-1} so a half-labeled file can
# never masquerade as either a labeled or an unlabeled split.
#
# Fail-closed (A16): first mismatch aborts with an exact CALYX_* code on
# stderr. No fallback sources, no skipped checks, no fabricated rows.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
# 24.0.0 is the first pin verified to ship a cp314 wheel for aiwonder's Python 3.14.
PYARROW_PIN="${CALYX_PYARROW_PIN:-pyarrow==24.0.0}"

fail() {
  echo "$1: $2" >&2
  exit 1
}

# --- pinned upstream state (recorded pre-download, 2026-06-12) ---------------
AG_NEWS_REV="eb185aade064a813bc0b7f42de02595523103ca4"
IMDB_REV="e6281661ce1c48d982bc483cf8a173c1bbeb5d31"
SST2_REV="8d51e7e4887a4caaa95b3fbebbf53c0490b58bbb"
BANKING77_REV="689fcc406cf47a5fbe15f09393be5f206a009fcc" # refs/convert/parquet
DBPEDIA_REV="9abd46cf7fc8b4c64290f26993c540b92aa145ac"

# dataset|hf_repo|revision|remote_path|local_name|bytes|sha256
FILES=(
  "ag_news|fancyzhx/ag_news|$AG_NEWS_REV|data/train-00000-of-00001.parquet|train.parquet|18585438|fc508d6d9868594e3da960a8cfeb63ab5a4746598b93428c224397080c1f52ee"
  "ag_news|fancyzhx/ag_news|$AG_NEWS_REV|data/test-00000-of-00001.parquet|test.parquet|1234829|71de87ec66bc5737752a2502204dfa6d7fe9856ade3ea444dc6317789a4f13fb"
  "imdb|stanfordnlp/imdb|$IMDB_REV|plain_text/train-00000-of-00001.parquet|train.parquet|20979968|db47d16b5c297cc0dd625e519c81319c24c9149e70e8496de5475f6fa928342c"
  "imdb|stanfordnlp/imdb|$IMDB_REV|plain_text/test-00000-of-00001.parquet|test.parquet|20470363|b52e26e2f872d282ffac460bf9770b25ac6f102cda0e6ca7158df98c94e8b3da"
  "imdb|stanfordnlp/imdb|$IMDB_REV|plain_text/unsupervised-00000-of-00001.parquet|unsupervised.parquet|41996509|74d14fbfcbb39fb7d299c38ca9f0ae6d231bf97108da85d620027ba437b6d52e"
  "sst2|stanfordnlp/sst2|$SST2_REV|data/train-00000-of-00001.parquet|train.parquet|3110458|c7921283b75a42e685f50edecb96798607ea0fcbfd0739ee8975f22c12d55f09"
  "sst2|stanfordnlp/sst2|$SST2_REV|data/validation-00000-of-00001.parquet|validation.parquet|72813|fb00fe008f6828f86ba2beda8415a4cf5da0c884f21c5f238c87131b5aa19529"
  "sst2|stanfordnlp/sst2|$SST2_REV|data/test-00000-of-00001.parquet|test.parquet|147787|20d27a86c0c59acb746a41a481ebb1fc71edb72d94b5ccee7f23b9041b17adcf"
  "banking77|PolyAI/banking77|$BANKING77_REV|default/train/0000.parquet|train.parquet|298170|45d8553240ae20a498392d1e5f80b7f55630365f726e819363d056829df00b8e"
  "banking77|PolyAI/banking77|$BANKING77_REV|default/test/0000.parquet|test.parquet|93870|529c7c3e2a074928d462989c349742d142d681332dff9534ef3cc6a5fad74f7d"
  "dbpedia_14|fancyzhx/dbpedia_14|$DBPEDIA_REV|dbpedia_14/train-00000-of-00001.parquet|train.parquet|106151899|0640e4664a99cc94c47db1d7b2e01c14455d5bbecb8183ad1f93bde59f3f28ee"
  "dbpedia_14|fancyzhx/dbpedia_14|$DBPEDIA_REV|dbpedia_14/test-00000-of-00001.parquet|test.parquet|13272475|05fed41640e97f93ffd442757f6a84170348cf0c7500ecbda9e95ddcd928c631"
)
DATASETS=(ag_news imdb sst2 banking77 dbpedia_14)

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
    if "$candidate" -c 'import pyarrow' >/dev/null 2>&1; then
      echo "$candidate"
      return
    fi
  done
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: no python with pyarrow - run scripts/acquire_datasets.sh once or set CALYX_DATASET_PYTHON" >&2
  exit 1
}

# Subcommands: validate <name> | validate-spec <name> <json> | gen-fixture <dir> <case> <seed>
run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import json
import os
import pathlib
import sys

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

# Per-split expectations recorded from upstream dataset cards / HF size API at
# pin time. "labels" is the EXACT unique-label domain the split must contain.
REAL_SPEC = {
    "ag_news": {"label_col": "label", "files": {
        "train.parquet": {"rows": 120000, "labels": list(range(4))},
        "test.parquet": {"rows": 7600, "labels": list(range(4))},
    }},
    "imdb": {"label_col": "label", "files": {
        "train.parquet": {"rows": 25000, "labels": [0, 1]},
        "test.parquet": {"rows": 25000, "labels": [0, 1]},
        "unsupervised.parquet": {"rows": 50000, "labels": [-1]},
    }},
    "sst2": {"label_col": "label", "files": {
        "train.parquet": {"rows": 67349, "labels": [0, 1]},
        "validation.parquet": {"rows": 872, "labels": [0, 1]},
        "test.parquet": {"rows": 1821, "labels": [-1]},
    }},
    "banking77": {"label_col": "label", "files": {
        "train.parquet": {"rows": 10003, "labels": list(range(77))},
        "test.parquet": {"rows": 3080, "labels": list(range(77))},
    }},
    "dbpedia_14": {"label_col": "label", "files": {
        "train.parquet": {"rows": 560000, "labels": list(range(14))},
        "test.parquet": {"rows": 70000, "labels": list(range(14))},
    }},
}


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate(name, spec):
    ds_dir = ROOT / name
    report = {}
    for fname, expected in sorted(spec["files"].items()):
        path = ds_dir / fname
        if not path.is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
        table = pq.read_table(path, columns=[spec["label_col"]])
        if table.num_rows != expected["rows"]:
            fail(
                "CALYX_DATASET_ROWCOUNT_MISMATCH",
                f"{name}/{fname}: rows {table.num_rows} != expected {expected['rows']}",
            )
        column = table.column(spec["label_col"])
        if column.null_count:
            fail(
                "CALYX_DATASET_LABEL_INVALID",
                f"{name}/{fname}: {column.null_count} null labels",
            )
        uniq = sorted(pc.unique(column.combine_chunks()).to_pylist())
        if uniq != sorted(expected["labels"]):
            fail(
                "CALYX_DATASET_LABEL_PARTITION_MISSING",
                f"{name}/{fname}: label domain {uniq[:20]} != expected {sorted(expected['labels'])[:20]}",
            )
        report[fname] = {"rows": table.num_rows, "classes": len(uniq)}
    print(json.dumps({name: report}, sort_keys=True))


def gen_fixture(target_dir, case, seed):
    # Deterministic 4-class fixture: 12 rows, labels 0..3 repeated 3x in file
    # order, text derived from (seed, index). No randomness sources - the same
    # (case, seed) always produces byte-identical parquet under pinned pyarrow.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    count = 11 if case == "short" else 12
    texts = [f"fixture-{seed}-row-{i:02d}" for i in range(count)]
    if case == "nulls":
        labels = [None] * count
    elif case == "missing-class":
        labels = [i % 3 for i in range(count)]
    else:
        labels = [i % 4 for i in range(count)]
    table = pa.table(
        {"text": pa.array(texts, pa.string()), "label": pa.array(labels, pa.int64())}
    )
    pq.write_table(table, target / "data.parquet")
    distribution = [labels.count(c) for c in range(4)]
    print(json.dumps({"case": case, "rows": count, "distribution": distribution}))


mode = sys.argv[1]
if mode == "validate":
    name = sys.argv[2]
    if name not in REAL_SPEC:
        fail("CALYX_DATASET_NOT_FOUND", f"no validation spec for {name!r}")
    validate(name, REAL_SPEC[name])
elif mode == "validate-spec":
    validate(sys.argv[2], json.loads(sys.argv[3]))
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
  curl -fsSL --retry 3 --retry-delay 5 \
    -H "Authorization: Bearer $HF_HUB_TOKEN" \
    "$url" -o "$dest.tmp" \
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
  export HF_HUB_TOKEN="${HF_HUB_TOKEN:-$HF_TOKEN}"
  if [[ ! -d "$DATASET_ROOT" ]]; then
    fail CALYX_DATASET_NOT_FOUND "dataset root missing: $DATASET_ROOT (PH00 ZFS provisioning)"
  fi

  echo "=== download (pinned revisions + pre-recorded sha256) ==="
  local spec dataset repo revision remote_path local_name bytes sha
  for spec in "${FILES[@]}"; do
    IFS='|' read -r dataset repo revision remote_path local_name bytes sha <<<"$spec"
    mkdir -p "$DATASET_ROOT/$dataset"
    download_verified \
      "https://huggingface.co/datasets/$repo/resolve/$revision/$remote_path" \
      "$DATASET_ROOT/$dataset/$local_name" "$bytes" "$sha"
  done

  echo "=== validate (row counts + exact label domains) ==="
  local name
  for name in "${DATASETS[@]}"; do
    run_python validate "$name"
  done

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register ag_news \
    --source "huggingface:fancyzhx/ag_news" \
    --revision "$AG_NEWS_REV splits=train,test" \
    --license "AG News corpus - free for research (Antonio Gulli)" \
    --tests "Assay bits/MI 4-class news anchor (PH70 issue #560)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register imdb \
    --source "huggingface:stanfordnlp/imdb plain_text" \
    --revision "$IMDB_REV splits=train,test,unsupervised" \
    --license "ACL IMDB (Maas et al. 2011) - research use" \
    --tests "Assay bits/MI 2-class sentiment anchor (PH70 issue #560)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register sst2 \
    --source "huggingface:stanfordnlp/sst2" \
    --revision "$SST2_REV splits=train,validation,test(unlabeled -1)" \
    --license "GLUE SST-2 (Socher et al. 2013) - research use" \
    --tests "Assay bits/MI 2-class sentiment anchor, held-out=validation (PH70 issue #560)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register banking77 \
    --source "huggingface:PolyAI/banking77 refs/convert/parquet" \
    --revision "$BANKING77_REV splits=train,test" \
    --license "CC-BY-4.0" \
    --tests "Assay bits/MI 77-class intent anchor (PH70 issue #560)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register dbpedia_14 \
    --source "huggingface:fancyzhx/dbpedia_14" \
    --revision "$DBPEDIA_REV splits=train,test" \
    --license "CC-BY-SA-3.0 (DBpedia)" \
    --tests "Assay bits/MI 14-class ontology anchor (PH70 issue #560)" \
    --rows-from "*.parquet"

  echo "acquire_classification: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Known input -> hand-derived expected output. The fixture parquet bytes are
# deterministic under the pinned pyarrow; this constant pins both the fixture
# generator and the toolchain (a pyarrow drift fails loudly here, never
# silently downstream).
FIXTURE_SHA="66d4f653d0a47b52396f9b526ac633dcb2633cbd4993e5adbf0204c37247982c"

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

  local spec_good='{"label_col":"label","files":{"data.parquet":{"rows":12,"labels":[0,1,2,3]}}}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic 12-row 4-class fixture: known distribution + pinned sha256"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  [[ "$gen_out" == '{"case": "good", "rows": 12, "distribution": [3, 3, 3, 3]}' ]] \
    || { echo "SELF-TEST FAILED: generator output != hand-computed expectation" >&2; exit 1; }
  local fixture_sha
  fixture_sha="$(sha256sum "$tmp_root/fixture_good/data.parquet" | cut -d' ' -f1)"
  if [[ "$fixture_sha" != "$FIXTURE_SHA" ]]; then
    echo "SELF-TEST FAILED: fixture parquet sha256 $fixture_sha != pinned $FIXTURE_SHA (pyarrow/toolchain drift)" >&2
    exit 1
  fi
  run_python validate-spec fixture_good "$spec_good"

  step "edge 1: all-null labels -> CALYX_DATASET_LABEL_INVALID, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_nulls" nulls s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_INVALID \
    bash "$SCRIPT_PATH" --validate-spec fixture_nulls "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: missing class in label domain -> CALYX_DATASET_LABEL_PARTITION_MISSING"
  run_python gen-fixture "$tmp_root/fixture_missing" missing-class s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_PARTITION_MISSING \
    bash "$SCRIPT_PATH" --validate-spec fixture_missing "$spec_good"

  step "edge 3: short row count -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 4: register then truncate parquet -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_classification.sh self-test" \
    --rows-from "*.parquet"
  show_catalog "after register"
  head -c 100 "$tmp_root/fixture_good/data.parquet" > "$tmp_root/fixture_good/data.parquet.trunc"
  mv "$tmp_root/fixture_good/data.parquet.trunc" "$tmp_root/fixture_good/data.parquet"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH \
    bash "$SCRIPT_DIR/verify_dataset.sh" fixture_good

  step "round-trip property: register->verify green for 3 distinct seeded fixtures"
  local seed
  for seed in s2 s3 s4; do
    run_python gen-fixture "$tmp_root/fixture_rt_$seed" good "$seed" >/dev/null
    bash "$SCRIPT_DIR/verify_dataset.sh" register "fixture_rt_$seed" \
      --source "self-test fixture" --revision "$seed" \
      --license "n/a (synthetic)" --tests "round-trip property" \
      --rows-from "*.parquet" >/dev/null
    bash "$SCRIPT_DIR/verify_dataset.sh" "fixture_rt_$seed"
  done

  echo "[SELF-TEST] all $pass steps passed"
}

case "${1:-acquire}" in
  acquire) acquire_all ;;
  --self-test) self_test ;;
  --validate-spec) shift; run_python validate-spec "$@" ;;
  --gen-fixture) shift; run_python gen-fixture "$@" ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
