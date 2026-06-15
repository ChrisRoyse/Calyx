#!/usr/bin/env bash
# PH69 T04 / issue #554 - acquire the code-oracle corpora (SWE-bench Lite /
# HumanEval / MBPP-sanitized) as pinned-revision parquet, verify every file
# against the sha256/bytes recorded here BEFORE download (HF LFS API metadata
# at the pinned commits), validate row counts AND the oracle schema contract
# per file, then register each dataset in the canonical MANIFEST via
# verify_dataset.sh register - the single catalog writer (PH69 T01).
#
#   acquire_code_oracle.sh              acquire + validate + register all 3
#   acquire_code_oracle.sh --self-test  hermetic synthetic-fixture battery
#
# Schema contract (the reason these corpora exist): they are PH70's
# deterministic pass/fail ground truth, so the fields that CARRY the ground
# truth are validated per row, not just per file:
#   swebench_lite - all 8 card fields present; instance_id unique + non-empty;
#     FAIL_TO_PASS parses as a JSON list with >= 1 test (it IS the oracle);
#     PASS_TO_PASS parses as a JSON list; patch/test_patch non-empty.
#   humaneval - task_id unique; prompt/canonical_solution/test/entry_point
#     non-empty for every problem.
#   mbpp - sanitized config (the 427 hand-verified problems; the standard
#     integrity split - the card's "374" count is full/train, a different,
#     noisier config); task_id unique across splits; test_list >= 1 per row;
#     code non-empty.
# Any violation -> CALYX_DATASET_SCHEMA_MISMATCH (closed catalog, calyx-core),
# and no MANIFEST row is written.
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
SWEBENCH_REV="6ec7bb89b9342f664a54a6e0a6ea6501d3437cc2"
HUMANEVAL_REV="7dce6050a7d6d172f3cc5c32aa97f52fa1a2e544"
MBPP_REV="4bb6404fdc6cacfda99d4ac4205087b89d32030c"

# dataset|hf_repo|revision|remote_path|local_name|bytes|sha256
FILES=(
  "swebench_lite|princeton-nlp/SWE-bench_Lite|$SWEBENCH_REV|data/test-00000-of-00001.parquet|test.parquet|1119540|7a21f37b8bc179c7db5beeb14e88ac538ba283455c776e6b2535bbfb6e3551b4"
  "humaneval|openai/openai_humaneval|$HUMANEVAL_REV|openai_humaneval/test-00000-of-00001.parquet|test.parquet|83920|2f2871a15fbc95b6c683043359f4ed8e144c5a1c4f24f25f66bc51f598dfcfb6"
  "mbpp|google-research-datasets/mbpp|$MBPP_REV|sanitized/train-00000-of-00001.parquet|train.parquet|33854|d95f8ad6d2fff08fe4826122d6e3e31f75716825d0c5c340d297aca5e9e0de0e"
  "mbpp|google-research-datasets/mbpp|$MBPP_REV|sanitized/test-00000-of-00001.parquet|test.parquet|60864|e9e9efa2c0d59ef5e55537a9d126b8f875d5ac010a8d75628d76824884e15850"
  "mbpp|google-research-datasets/mbpp|$MBPP_REV|sanitized/validation-00000-of-00001.parquet|validation.parquet|13987|27e065fcab3c863959933328a7fdbf404e1bcb5464b1be6fe0dcd9530e420204"
  "mbpp|google-research-datasets/mbpp|$MBPP_REV|sanitized/prompt-00000-of-00001.parquet|prompt.parquet|6717|73c623309b7b5d65fd5661204b35f779f8e66301aa9832d1ad4b8fc3b21151fd"
)
DATASETS=(swebench_lite humaneval mbpp)

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
import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

SWEBENCH_FIELDS = [
    "instance_id", "repo", "problem_statement", "hints_text",
    "patch", "test_patch", "FAIL_TO_PASS", "PASS_TO_PASS",
]
# Per-file row counts recorded from the HF size API at pin time; "checks"
# selects the per-row oracle-contract validator.
REAL_SPEC = {
    "swebench_lite": {"checks": "swebench", "files": {"test.parquet": 300}},
    "humaneval": {"checks": "humaneval", "files": {"test.parquet": 164}},
    "mbpp": {"checks": "mbpp", "files": {
        "train.parquet": 120, "test.parquet": 257,
        "validation.parquet": 43, "prompt.parquet": 7,
    }},
}


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def require_columns(name, fname, table, columns):
    missing = [c for c in columns if c not in table.column_names]
    if missing:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: missing column(s) {missing}")
    for column in columns:
        nulls = table.column(column).null_count
        if nulls:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: {nulls} null values in {column!r}")


def require_nonempty(name, fname, rows, fields):
    for i, row in enumerate(rows):
        for field in fields:
            if not str(row[field]).strip():
                fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} row {i}: empty {field!r}")


def require_unique(name, fname, values, field):
    seen = set()
    for value in values:
        if value in seen:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: duplicate {field} {value!r}")
        seen.add(value)


def json_test_list(name, fname, row, i, field, min_len):
    raw = row[field]
    try:
        parsed = json.loads(raw)
    except (TypeError, json.JSONDecodeError) as err:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname} row {i} ({row['instance_id']}): {field} is not valid JSON: {err}")
    if not isinstance(parsed, list) or not all(isinstance(t, str) for t in parsed):
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname} row {i} ({row['instance_id']}): {field} is not a list of strings")
    if len(parsed) < min_len:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname} row {i} ({row['instance_id']}): {field} has {len(parsed)} tests, need >= {min_len}")
    return parsed


def check_swebench(name, fname, table, report):
    require_columns(name, fname, table, SWEBENCH_FIELDS)
    rows = table.select(SWEBENCH_FIELDS).to_pylist()
    require_nonempty(name, fname, rows, ["instance_id", "repo", "problem_statement", "patch", "test_patch"])
    require_unique(name, fname, [r["instance_id"] for r in rows], "instance_id")
    fail_to_pass_total = 0
    for i, row in enumerate(rows):
        fail_to_pass_total += len(json_test_list(name, fname, row, i, "FAIL_TO_PASS", 1))
        json_test_list(name, fname, row, i, "PASS_TO_PASS", 0)
    report.update(unique_instance_ids=len(rows), fail_to_pass_tests=fail_to_pass_total)


def check_humaneval(name, fname, table, report):
    fields = ["task_id", "prompt", "canonical_solution", "test", "entry_point"]
    require_columns(name, fname, table, fields)
    rows = table.select(fields).to_pylist()
    require_nonempty(name, fname, rows, fields)
    require_unique(name, fname, [r["task_id"] for r in rows], "task_id")
    report.update(unique_task_ids=len(rows))


def check_mbpp(name, fname, table, report, seen_task_ids):
    fields = ["source_file", "task_id", "prompt", "code", "test_imports", "test_list"]
    require_columns(name, fname, table, fields)
    rows = table.select(fields).to_pylist()
    require_nonempty(name, fname, rows, ["prompt", "code"])
    tests_total = 0
    for i, row in enumerate(rows):
        if row["task_id"] in seen_task_ids:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{name}/{fname} row {i}: duplicate task_id {row['task_id']} across splits")
        seen_task_ids.add(row["task_id"])
        if not row["test_list"]:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{name}/{fname} row {i} (task_id {row['task_id']}): empty test_list")
        tests_total += len(row["test_list"])
    report.update(tests=tests_total)


def validate(name, spec):
    ds_dir = ROOT / name
    reports = {}
    mbpp_seen = set()
    for fname, expected_rows in sorted(spec["files"].items()):
        path = ds_dir / fname
        if not path.is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
        table = pq.read_table(path)
        if table.num_rows != expected_rows:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"{name}/{fname}: rows {table.num_rows} != expected {expected_rows}")
        report = {"rows": table.num_rows}
        if spec["checks"] == "swebench":
            check_swebench(name, fname, table, report)
        elif spec["checks"] == "humaneval":
            check_humaneval(name, fname, table, report)
        elif spec["checks"] == "mbpp":
            check_mbpp(name, fname, table, report, mbpp_seen)
        else:
            fail("CALYX_DATASET_MANIFEST_INVALID", f"unknown checks kind {spec['checks']!r}")
        reports[fname] = report
    print(json.dumps({name: reports}, sort_keys=True))


def gen_fixture(target_dir, case, seed):
    # Deterministic 5-instance SWE-bench-format fixture: known instance_ids,
    # text derived from (seed, index), no randomness sources - the same
    # (case, seed) always produces byte-identical parquet under pinned pyarrow.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    count = 4 if case == "short" else 5
    ids = [f"synthetic__{seed}-{i:04d}" for i in range(count)]
    if case == "dup-id":
        ids[-1] = ids[0]
    columns = {
        "instance_id": ids,
        "repo": [f"synthetic/repo-{seed}"] * count,
        "problem_statement": [f"problem {seed}-{i}" for i in range(count)],
        "hints_text": [""] * count,
        "patch": [f"--- a/f.py\n+++ b/f.py\n+# fix {seed}-{i}\n" for i in range(count)],
        "test_patch": [f"--- a/t.py\n+++ b/t.py\n+# test {seed}-{i}\n" for i in range(count)],
        "FAIL_TO_PASS": [json.dumps([f"t::test_fix_{seed}_{i}", f"t::test_edge_{seed}_{i}"]) for i in range(count)],
        "PASS_TO_PASS": [json.dumps([f"t::test_old_{seed}_{i}"]) for i in range(count)],
    }
    if case == "missing-column":
        del columns["FAIL_TO_PASS"]
    elif case == "empty-ftp":
        columns["FAIL_TO_PASS"][2] = json.dumps([])
    elif case == "bad-json":
        columns["FAIL_TO_PASS"][2] = "not json {"
    table = pa.table({k: pa.array(v, pa.string()) for k, v in columns.items()})
    pq.write_table(table, target / "test.parquet")
    print(json.dumps({"case": case, "rows": count, "instance_ids": ids}))


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

  echo "=== validate (row counts + oracle schema contract) ==="
  local name
  for name in "${DATASETS[@]}"; do
    run_python validate "$name"
  done

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register swebench_lite \
    --source "huggingface:princeton-nlp/SWE-bench_Lite" \
    --revision "$SWEBENCH_REV splits=test (the paper's 300-instance instantiation)" \
    --license "MIT (SWE-bench); instances from public GitHub repos" \
    --tests "Oracle sufficiency vs 0.46 deficit - real pass/fail ground truth (PH70 issue #563)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register humaneval \
    --source "huggingface:openai/openai_humaneval" \
    --revision "$HUMANEVAL_REV splits=test" \
    --license "MIT (OpenAI HumanEval)" \
    --tests "Oracle deterministic test pass/fail (PH70 issue #563)" \
    --rows-from "*.parquet"
  bash "$SCRIPT_DIR/verify_dataset.sh" register mbpp \
    --source "huggingface:google-research-datasets/mbpp sanitized" \
    --revision "$MBPP_REV splits=train,test,validation,prompt (427 hand-verified)" \
    --license "CC-BY-4.0" \
    --tests "Oracle deterministic test pass/fail (PH70 issue #563)" \
    --rows-from "*.parquet"

  echo "acquire_code_oracle: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Known input -> hand-derived expected output. The fixture parquet bytes are
# deterministic under the pinned pyarrow; this constant pins both the fixture
# generator and the toolchain (a pyarrow drift fails loudly here, never
# silently downstream).
FIXTURE_SHA="ee0288f673329291ba38989fbb3f2a919591250db238a64a3e490aab7faddf3b"

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

  local spec_good='{"checks":"swebench","files":{"test.parquet":5}}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic 5-instance SWE-bench fixture: known instance_ids + pinned sha256"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  [[ "$gen_out" == *'"instance_ids": ["synthetic__s1-0000", "synthetic__s1-0001", "synthetic__s1-0002", "synthetic__s1-0003", "synthetic__s1-0004"]'* ]] \
    || { echo "SELF-TEST FAILED: generator instance_ids != hand-computed expectation" >&2; exit 1; }
  local fixture_sha
  fixture_sha="$(sha256sum "$tmp_root/fixture_good/test.parquet" | cut -d' ' -f1)"
  if [[ "$fixture_sha" != "$FIXTURE_SHA" ]]; then
    echo "SELF-TEST FAILED: fixture parquet sha256 $fixture_sha != pinned $FIXTURE_SHA (pyarrow/toolchain drift)" >&2
    exit 1
  fi
  run_python validate-spec fixture_good "$spec_good"

  step "edge 1: FAIL_TO_PASS column absent -> CALYX_DATASET_SCHEMA_MISMATCH, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_nocol" missing-column s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_nocol "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: empty FAIL_TO_PASS list -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_empty" empty-ftp s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_empty "$spec_good"

  step "edge 3: FAIL_TO_PASS not valid JSON -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_badjson" bad-json s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_badjson "$spec_good"

  step "edge 4: duplicate instance_id -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_dup" dup-id s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_dup "$spec_good"

  step "edge 5: short row count (filtered split) -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 6: register then truncate parquet -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_code_oracle.sh self-test" \
    --rows-from "*.parquet"
  show_catalog "after register"
  head -c 100 "$tmp_root/fixture_good/test.parquet" > "$tmp_root/fixture_good/test.parquet.trunc"
  mv "$tmp_root/fixture_good/test.parquet.trunc" "$tmp_root/fixture_good/test.parquet"
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
