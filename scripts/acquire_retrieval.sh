#!/usr/bin/env bash
# PH69 T02 / issue #552 - acquire the retrieval benchmark corpora (BEIR
# scifact / MS MARCO / Natural Questions / TREC-COVID via the BeIR HF
# mirrors) as pinned-revision parquet + qrels TSVs, verify every file against
# the sha256/bytes recorded here BEFORE download (HF LFS API at the pinned
# commits; the small git-blob qrels TSVs were hashed at pin time), validate
# the retrieval contract per dataset, then register each in the canonical
# MANIFEST via verify_dataset.sh register (single catalog writer, PH69 T01).
#
#   acquire_retrieval.sh              acquire + validate + register all 4
#   acquire_retrieval.sh --self-test  hermetic synthetic-fixture battery
#
# Subset selection (per the card): BEIR uses the scifact split (small, fully
# self-contained); MS MARCO uses the dev qrels subset with the FULL 8.8M
# passage corpus (qrels without their corpus cannot prove recall); NQ and
# TREC-COVID use their full test sets.
#
# Retrieval contract (these qrels are PH70's Sextant recall ground truth, so
# the truth-carrying structure is validated, not just file counts):
#   corpus/queries parquet - exact row counts, _id present, no nulls, unique;
#   qrels TSV - exact BEIR header (query-id/corpus-id/score), exact row
#   counts, integer scores, >= 1 positive (a qrels file with no positive
#   judgments cannot measure recall -> LABEL_PARTITION_MISSING), and
#   REFERENTIAL INTEGRITY: every qrels row must reference a query id and a
#   corpus id that exist -> CALYX_DATASET_SCHEMA_MISMATCH otherwise.
#
# Fail-closed (A16): first mismatch aborts with an exact CALYX_* code on
# stderr. No fallback sources, no skipped checks, no fabricated rows.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"

fail() {
  echo "$1: $2" >&2
  exit 1
}

# --- pinned upstream state (recorded pre-download, 2026-06-12) ---------------
SCIFACT_REV="b3b5335604bf5ee3c4447671af975ea25143d4f5"
SCIFACT_QRELS_REV="2938d17dc3b09882fdb8c12bbbe2e2dc0e75a029"
MSMARCO_REV="a918e0d11a77ed33f42f29d98340b655593b96ad"
MSMARCO_QRELS_REV="253fbf8a3f8d4a0932b63882b5162bedc84779f5"
NQ_REV="b7253e6c379163d024ddb1d6948152a91a2e3b46"
NQ_QRELS_REV="519acd4e48bb3e5da22b2b888ce36c614f4f2bc9"
TRECCOVID_REV="7e16fde3016c639c7f856e803f4bab92645562c4"
TRECCOVID_QRELS_REV="532ac68ee6756ac22c9346eebf65bd3c6a042e10"

# dataset|hf_repo|revision|remote_path|local_name|bytes|sha256
FILES=(
  "beir|BeIR/scifact|$SCIFACT_REV|corpus/corpus-00000-of-00001.parquet|corpus.parquet|4469916|243324b35f03d82bd6d98a5f575966876e86cad7ce16e5333a35b1b793dc4f45"
  "beir|BeIR/scifact|$SCIFACT_REV|queries/queries-00000-of-00001.parquet|queries.parquet|64982|1c37956c5dc8b810b60302323c24d1a9e79e26411ba8f5ad9d0888642e2a9034"
  "beir|BeIR/scifact-qrels|$SCIFACT_QRELS_REV|train.tsv|qrels-train.tsv|14502|a53f2114831916c096b6c37d9e54da68cef4efdcdbd5ed46533601af972acf1d"
  "beir|BeIR/scifact-qrels|$SCIFACT_QRELS_REV|test.tsv|qrels-test.tsv|5389|0864bb985e0ca2367ba217977e72004d549054b2b06666ed9d4825ac7c21284c"
  "msmarco|BeIR/msmarco|$MSMARCO_REV|corpus/corpus-00000-of-00001.parquet|corpus.parquet|1632573546|57d6fc19851d4a363fff8b3e9acb0b549d97d4b141bd2aedd1522149dfeb8bd6"
  "msmarco|BeIR/msmarco|$MSMARCO_REV|queries/queries-00000-of-00001.parquet|queries.parquet|15437162|9f43da825b7788be9303603cc48ae21705da7a75e4a69e6121af56e25b34cdd6"
  "msmarco|BeIR/msmarco-qrels|$MSMARCO_QRELS_REV|dev.tsv|qrels-dev.tsv|135889|ec0a7d2ee847ce9196c5eae9471a3caff2206395cd5c145dade2e242a02cf0f7"
  "natural_questions|BeIR/nq|$NQ_REV|corpus/corpus-00000-of-00001.parquet|corpus.parquet|764191851|b7e8d5a99cfe94a1a0f175e274ae6d8f33fe0630f1ab529cd8537e82bd0aaa9e"
  "natural_questions|BeIR/nq|$NQ_REV|queries/queries-00000-of-00001.parquet|queries.parquet|138432|c8f54a071a7e9efa95f65251e0a9e9f74ca232120b67d05dff6952900ebf51ce"
  "natural_questions|BeIR/nq-qrels|$NQ_QRELS_REV|test.tsv|qrels-test.tsv|87138|6df0cd2cbbe88504b64c68f21e946a759b62c4d864225720cec256f0196e2210"
  "trec_covid|BeIR/trec-covid|$TRECCOVID_REV|corpus/corpus-00000-of-00001.parquet|corpus.parquet|110609513|d76cea1b2304dbe67a1a54f7376a61de294976682a1d7d58d82de27141f3ba4a"
  "trec_covid|BeIR/trec-covid|$TRECCOVID_REV|queries/queries-00000-of-00001.parquet|queries.parquet|4865|80bd564b1218a519ef0a396fa7b874941b7188d8240933e8d6fa867d7db59d6f"
  "trec_covid|BeIR/trec-covid-qrels|$TRECCOVID_QRELS_REV|test.tsv|qrels-test.tsv|980831|10669ab7d526cb04f52079139fd88c3d467a0776441b046567f540582798982b"
)
DATASETS=(beir msmarco natural_questions trec_covid)

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
import csv
import json
import os
import pathlib
import sys

import pyarrow as pa
import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])
QRELS_HEADER = ["query-id", "corpus-id", "score"]

# Expected counts recorded from the HF size API at pin time.
REAL_SPEC = {
    "beir": {"corpus": 5183, "queries": 1109,
             "qrels": {"qrels-train.tsv": 919, "qrels-test.tsv": 339}},
    "msmarco": {"corpus": 8841823, "queries": 509962,
                "qrels": {"qrels-dev.tsv": 7437}},
    "natural_questions": {"corpus": 2681468, "queries": 3452,
                          "qrels": {"qrels-test.tsv": 4201}},
    "trec_covid": {"corpus": 171332, "queries": 50,
                   "qrels": {"qrels-test.tsv": 66336}},
}


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def id_set(name, fname, expected_rows):
    """Load the _id column of a corpus/queries parquet; exact count, no
    nulls, unique."""
    path = ROOT / name / fname
    if not path.is_file():
        fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
    table = pq.read_table(path, columns=["_id"])
    if table.num_rows != expected_rows:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/{fname}: rows {table.num_rows} != expected {expected_rows}")
    column = table.column("_id")
    if column.null_count:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: {column.null_count} null _id values")
    ids = set(column.to_pylist())
    if len(ids) != expected_rows:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname}: {expected_rows - len(ids)} duplicate _id values")
    return ids


def check_qrels(name, fname, expected_rows, query_ids, corpus_ids):
    path = ROOT / name / fname
    if not path.is_file():
        fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
    rows = 0
    positives = 0
    with path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.reader(handle, delimiter="\t")
        header = next(reader, None)
        if header is None:
            # Empty file: a row-count failure (verify_dataset.sh count_rows
            # precedent), not a schema failure - there is no schema to judge.
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: empty qrels file")
        if header != QRELS_HEADER:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{name}/{fname}: header {header!r} != {QRELS_HEADER}")
        for i, row in enumerate(reader):
            if len(row) != 3:
                fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} row {i}: {len(row)} fields != 3")
            qid, did, score = row
            try:
                score = int(score)
            except ValueError:
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/{fname} row {i}: score {row[2]!r} is not an integer")
            # Referential integrity: a judgment must reference real ids.
            if qid not in query_ids:
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/{fname} row {i}: unknown query-id {qid!r}")
            if did not in corpus_ids:
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/{fname} row {i}: unknown corpus-id {did!r}")
            if score > 0:
                positives += 1
            rows += 1
    if rows != expected_rows:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/{fname}: rows {rows} != expected {expected_rows}")
    if positives == 0:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{name}/{fname}: no positive judgments - cannot measure recall")
    return {"rows": rows, "positives": positives}


def validate(name, spec):
    try:
        corpus_ids = id_set(name, "corpus.parquet", spec["corpus"])
        query_ids = id_set(name, "queries.parquet", spec["queries"])
        report = {"corpus": len(corpus_ids), "queries": len(query_ids)}
        for fname, expected in sorted(spec["qrels"].items()):
            report[fname] = check_qrels(name, fname, expected, query_ids, corpus_ids)
    except SystemExit:
        raise
    except Exception as err:
        # Corrupt/truncated bytes are an integrity failure - closed catalog
        # code, never a raw traceback (same contract as #553/#555 fixes).
        fail("CALYX_DATASET_CHECKSUM_MISMATCH",
             f"{name}: unreadable/corrupt data: {type(err).__name__}: {err}")
    print(json.dumps({name: report}, sort_keys=True))


def gen_fixture(target_dir, case, seed):
    # Deterministic micro retrieval dataset: 4 docs, 2 queries, 3 qrels
    # triples (2 positive, 1 zero). Content derived from (seed, index) only.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    docs = [f"fx-{seed}-d{i}" for i in range(1, 5)]
    queries = [f"fx-{seed}-q{i}" for i in range(1, 3)]
    pq.write_table(pa.table({
        "_id": pa.array(docs, pa.string()),
        "title": pa.array([f"title {seed} {d}" for d in docs], pa.string()),
        "text": pa.array([f"text {seed} {d}" for d in docs], pa.string()),
    }), target / "corpus.parquet")
    pq.write_table(pa.table({
        "_id": pa.array(queries, pa.string()),
        "text": pa.array([f"query {seed} {q}" for q in queries], pa.string()),
    }), target / "queries.parquet")
    triples = [(queries[0], docs[0], "1"), (queries[0], docs[1], "0"), (queries[1], docs[2], "2")]
    if case == "unknown-doc":
        triples[2] = (queries[1], f"fx-{seed}-d9", "2")
    elif case == "bad-score":
        triples[2] = (queries[1], docs[2], "high")
    elif case == "no-positive":
        triples = [(q, d, "0") for q, d, _ in triples]
    elif case == "short":
        triples = triples[:2]
    lines = ["\t".join(QRELS_HEADER)] + ["\t".join(t) for t in triples]
    qrels = target / "qrels-test.tsv"
    if case == "zero-byte":
        qrels.write_bytes(b"")
    else:
        qrels.write_bytes(("\n".join(lines) + "\n").encode())
    print(json.dumps({"case": case, "docs": len(docs), "queries": len(queries), "qrels": len(triples)}))


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
  # .tmp lives beside the destination on the same ZFS mount - never /tmp then
  # rename (EXDEV risk on cross-mount).
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
  mv "$dest.tmp" "$dest"
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

  echo "=== validate (retrieval contract: counts, qrels schema, referential integrity) ==="
  local name
  for name in "${DATASETS[@]}"; do
    run_python validate "$name"
  done

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register beir \
    --source "huggingface:BeIR/scifact + BeIR/scifact-qrels" \
    --revision "$SCIFACT_REV + qrels $SCIFACT_QRELS_REV (scifact: corpus 5183, queries 1109, qrels train+test)" \
    --license "Apache-2.0 (BEIR); CC-BY-NC-2.5 (SciFact data)" \
    --tests "Sextant recall delta>=15% qrels - scientific claims (PH70 issue #559)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register msmarco \
    --source "huggingface:BeIR/msmarco + BeIR/msmarco-qrels" \
    --revision "$MSMARCO_REV + qrels $MSMARCO_QRELS_REV (full 8.8M passage corpus, dev qrels subset)" \
    --license "MS MARCO non-commercial research (Microsoft)" \
    --tests "Sextant recall delta>=15% qrels - web passages at scale (PH70 issue #559)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register natural_questions \
    --source "huggingface:BeIR/nq + BeIR/nq-qrels" \
    --revision "$NQ_REV + qrels $NQ_QRELS_REV (full test set)" \
    --license "CC-BY-SA-3.0 (Natural Questions / BEIR)" \
    --tests "Sextant recall delta>=15% qrels - open-domain QA (PH70 issue #559)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register trec_covid \
    --source "huggingface:BeIR/trec-covid + BeIR/trec-covid-qrels" \
    --revision "$TRECCOVID_REV + qrels $TRECCOVID_QRELS_REV (full test set, graded judgments)" \
    --license "Dataset usage per TREC-COVID/CORD-19 terms (research)" \
    --tests "Sextant recall delta>=15% qrels - biomedical, graded relevance (PH70 issue #559)"

  echo "acquire_retrieval: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# The qrels fixture sha was hand-computed from the literal fixture bytes (the
# card's known-line checksum); parquet fixtures are validated structurally.
QRELS_FIXTURE_SHA="24cdfd13183928eaf3b85a978702b8a20cd723563949f32c50a9694d9d88fec6"

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

  local spec_good='{"corpus":4,"queries":2,"qrels":{"qrels-test.tsv":3}}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic 3-triple qrels fixture: hand-computed checksum + contract green"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  local qrels_sha
  qrels_sha="$(sha256sum "$tmp_root/fixture_good/qrels-test.tsv" | cut -d' ' -f1)"
  if [[ "$qrels_sha" != "$QRELS_FIXTURE_SHA" ]]; then
    echo "SELF-TEST FAILED: qrels fixture sha256 $qrels_sha != pinned $QRELS_FIXTURE_SHA" >&2
    exit 1
  fi
  local val_out
  val_out="$(run_python validate-spec fixture_good "$spec_good")"
  echo "    $val_out"
  [[ "$val_out" == '{"fixture_good": {"corpus": 4, "qrels-test.tsv": {"positives": 2, "rows": 3}, "queries": 2}}' ]] \
    || { echo "SELF-TEST FAILED: validate output != hand-computed expectation" >&2; exit 1; }

  step "edge 1: zero-byte qrels file -> CALYX_DATASET_ROWCOUNT_MISMATCH, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_zero" zero-byte s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_zero "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: qrels references unknown corpus-id -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_ghost" unknown-doc s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_ghost "$spec_good"

  step "edge 3: non-integer relevance score -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_badscore" bad-score s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_badscore "$spec_good"

  step "edge 4: qrels with no positive judgment -> CALYX_DATASET_LABEL_PARTITION_MISSING"
  run_python gen-fixture "$tmp_root/fixture_nopos" no-positive s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_PARTITION_MISSING \
    bash "$SCRIPT_PATH" --validate-spec fixture_nopos "$spec_good"

  step "edge 5: partial qrels (2 of 3) -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 6: register then truncate corpus parquet -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_retrieval.sh self-test"
  show_catalog "after register"
  head -c 64 "$tmp_root/fixture_good/corpus.parquet" > "$tmp_root/fixture_good/corpus.parquet.trunc"
  mv "$tmp_root/fixture_good/corpus.parquet.trunc" "$tmp_root/fixture_good/corpus.parquet"
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH \
    bash "$SCRIPT_DIR/verify_dataset.sh" fixture_good
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_good "$spec_good"

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
  --validate) shift; run_python validate "$@" ;;
  --validate-spec) shift; run_python validate-spec "$@" ;;
  --gen-fixture) shift; run_python gen-fixture "$@" ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
