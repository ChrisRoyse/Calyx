#!/usr/bin/env bash
# PH69 T05 / issue #555 - acquire the graph/kernel corpora (WordNet 3.0,
# ConceptNet 5.7, Cora/LINQS, ogbn-arxiv) with pre-recorded sha256 pins,
# validate the graph structure from the bytes (synset/node/edge counts +
# referential integrity), then register each in the canonical MANIFEST via
# verify_dataset.sh register - the single catalog writer (PH69 T01).
#
#   acquire_graph_kernel.sh              acquire + validate + register all 4
#   acquire_graph_kernel.sh --self-test  hermetic synthetic-fixture battery
#
# Source pins (immutable where upstream allows, checksum-pinned otherwise):
#   wordnet    - nltk_data wordnet.zip at GitHub commit 984c35e1 (immutable URL);
#                WordNet 3.0: synsets 82115n+13767v+18156a+3621r = 117659.
#   conceptnet - conceptnet-assertions-5.7.0.csv.gz from the versioned S3
#                release (ETag stable since 2019); 34,074,917 edges.
#   cora       - LINQS original cora.tgz (2708 papers / 5429 directed cites -
#                the canonical RAW version; the Planetoid variant doubles
#                edges to 10556 undirected and is a different artifact).
#   ogbn       - OGB ogbn-arxiv arxiv.zip from snap.stanford.edu (ETag stable
#                since 2020); 169,343 nodes / 1,166,243 directed edges.
# Every file is verified against the sha256/bytes recorded below BEFORE this
# script ever registers anything; a silent upstream change is a loud
# CALYX_DATASET_CHECKSUM_MISMATCH, never absorbed.
#
# wiktionary_defn_graph (5th corpus named by the card) is NOT acquired here:
# neither kaikki.org nor dumps.wikimedia.org keeps immutable pinned artifacts
# (both rotate/overwrite), so a fail-closed checksum pin is impossible today.
# The card's gate needs >= 3 of 5 corpora; this card lands 4. The skip is
# printed loudly at every run (no silent caps, DOCTRINE S9).
#
# Catalog rows: these artifacts are zip/tgz/gz binaries, so the MANIFEST
# `rows` column is 0 (same precedent as voxceleb1_mini_issue608); the true
# structural counts are validated from bytes on every run and recorded in
# each row's revision field.
#
# Fail-closed (A16): first mismatch aborts with an exact CALYX_* code.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"

fail() {
  echo "$1: $2" >&2
  exit 1
}

# --- pinned upstream state (recorded pre-registration, 2026-06-12) -----------
NLTK_COMMIT="984c35e161a0e66bcc6666d46107e43697c6b3c1"
WORDNET_URL="https://raw.githubusercontent.com/nltk/nltk_data/$NLTK_COMMIT/packages/corpora/wordnet.zip"
CONCEPTNET_URL="https://s3.amazonaws.com/conceptnet/downloads/2019/edges/conceptnet-assertions-5.7.0.csv.gz"
CORA_URL="https://linqs-data.soe.ucsc.edu/public/lbc/cora.tgz"
OGBN_URL="https://snap.stanford.edu/ogb/data/nodeproppred/arxiv.zip"

# dataset|url|local_name|bytes|sha256
FILES=(
  "wordnet|$WORDNET_URL|wordnet.zip|10775600|cbda5ea6eef7f36a97a43d4a75f85e07fccbb4f23657d27b4ccbc93e2646ab59"
  "conceptnet|$CONCEPTNET_URL|conceptnet-assertions-5.7.0.csv.gz|497963447|accd65fe94038584295574ddc26e1500c1919c8c4532bf771811cafd0948af7e"
  "cora|$CORA_URL|cora.tgz|168052|0d4ed463d1627bb7f3e8420effe8f5545fd492ae8f88dab44ce86cee7b26d7e8"
  "ogbn|$OGBN_URL|arxiv.zip|83058288|49f85c801589ecdcc52cfaca99693aaea7b8af16a9ac3f41dd85a5f3193fe276"
)
DATASETS=(wordnet conceptnet cora ogbn)

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

# Subcommands: validate <name> | validate-spec <name> <json> | gen-fixture <dir> <case> <seed>
run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import gzip
import json
import os
import pathlib
import sys
import tarfile
import zipfile

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

# Structural ground truth recorded at pin time (2026-06-12), hand-checked
# against the published numbers for each corpus.
WORDNET_SYNSETS = {"noun": 82115, "verb": 13767, "adj": 18156, "adv": 3621}  # = 117659
CONCEPTNET_EDGES = 34074917
CORA_NODES, CORA_CITES = 2708, 5429
OGBN_NODES, OGBN_EDGES = 169343, 1166243


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_wordnet(ds_dir, report):
    with zipfile.ZipFile(ds_dir / "wordnet.zip") as zf:
        for pos, expected in WORDNET_SYNSETS.items():
            with zf.open(f"wordnet/data.{pos}") as handle:
                count = sum(1 for line in handle if not line.startswith(b"  "))
            if count != expected:
                fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                     f"wordnet data.{pos}: {count} synsets != expected {expected}")
            report[f"synsets_{pos}"] = count
    report["synsets_total"] = sum(WORDNET_SYNSETS.values())


def check_conceptnet(ds_dir, report):
    path = ds_dir / "conceptnet-assertions-5.7.0.csv.gz"
    count = 0
    first = None
    with gzip.open(path, "rb") as handle:
        for line in handle:
            if first is None:
                first = line
            count += 1
    if first is None or len(first.split(b"\t")) != 5 or not first.startswith(b"/a/"):
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"conceptnet: first assertion is not a 5-field /a/ edge: {first!r:.80}")
    if count != CONCEPTNET_EDGES:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"conceptnet: {count} edges != expected {CONCEPTNET_EDGES}")
    report["edges"] = count


def check_cora(ds_dir, report):
    with tarfile.open(ds_dir / "cora.tgz", "r:gz") as tf:
        nodes = set()
        with tf.extractfile("cora/cora.content") as handle:
            for i, line in enumerate(handle):
                parts = line.rstrip(b"\n").split(b"\t")
                if len(parts) != 1435 or not parts[0]:
                    fail("CALYX_DATASET_SCHEMA_MISMATCH",
                         f"cora.content line {i}: {len(parts)} fields != 1435 (id + 1433 features + label)")
                nodes.add(parts[0])
        if len(nodes) != CORA_NODES:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"cora.content: {len(nodes)} unique papers != expected {CORA_NODES}")
        edges = 0
        self_loops = 0
        with tf.extractfile("cora/cora.cites") as handle:
            for i, line in enumerate(handle):
                parts = line.split()
                if len(parts) != 2:
                    fail("CALYX_DATASET_SCHEMA_MISMATCH",
                         f"cora.cites line {i}: {len(parts)} fields != 2 (cited citing)")
                # Referential integrity: a citation must reference known papers.
                for node in parts:
                    if node not in nodes:
                        fail("CALYX_DATASET_SCHEMA_MISMATCH",
                             f"cora.cites line {i}: unknown paper id {node.decode()!r}")
                if parts[0] == parts[1]:
                    self_loops += 1  # recorded, not an error (a paper citing itself)
                edges += 1
        if edges != CORA_CITES:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"cora.cites: {edges} edges != expected {CORA_CITES}")
    report.update(nodes=len(nodes), edges=edges, self_loops=self_loops)


def gz_member_int(zf, member):
    with zf.open(member) as handle:
        return int(gzip.decompress(handle.read()).strip())


def check_ogbn(ds_dir, report):
    with zipfile.ZipFile(ds_dir / "arxiv.zip") as zf:
        nodes = gz_member_int(zf, "arxiv/raw/num-node-list.csv.gz")
        edges = gz_member_int(zf, "arxiv/raw/num-edge-list.csv.gz")
        if nodes != OGBN_NODES:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"ogbn-arxiv: {nodes} nodes != expected {OGBN_NODES}")
        if edges != OGBN_EDGES:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"ogbn-arxiv: {edges} edges != expected {OGBN_EDGES}")
        with zf.open("arxiv/raw/edge.csv.gz") as handle:
            first = gzip.open(handle).readline()
        parts = first.strip().split(b",")
        if len(parts) != 2 or not all(p.isdigit() for p in parts):
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"ogbn-arxiv edge.csv: first edge is not 'int,int': {first!r:.40}")
    report.update(nodes=nodes, edges=edges)


CHECKS = {"wordnet": check_wordnet, "conceptnet": check_conceptnet,
          "cora": check_cora, "ogbn": check_ogbn}


def check_edgelist(ds_dir, report, expect):
    """Generic node/edge-list validator (self-test fixtures): nodes.txt one id
    per line; edges.tsv 'a<TAB>b' per line; every edge endpoint must be a
    known node; self-loops are recorded, never a crash."""
    nodes = [l for l in (ds_dir / "nodes.txt").read_bytes().splitlines() if l]
    node_set = set(nodes)
    if len(node_set) != expect["nodes"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{ds_dir.name}: {len(node_set)} nodes != expected {expect['nodes']}")
    edges = 0
    self_loops = 0
    for i, line in enumerate(l for l in (ds_dir / "edges.tsv").read_bytes().splitlines() if l):
        parts = line.split(b"\t")
        if len(parts) != 2 or not parts[0] or not parts[1]:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{ds_dir.name} edges.tsv line {i}: malformed edge {line!r}")
        for node in parts:
            if node not in node_set:
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{ds_dir.name} edges.tsv line {i}: unknown node {node.decode()!r}")
        if parts[0] == parts[1]:
            self_loops += 1
        edges += 1
    if edges != expect["edges"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{ds_dir.name}: {edges} edges != expected {expect['edges']}")
    report.update(nodes=len(node_set), edges=edges, self_loops=self_loops)


def gen_fixture(target_dir, case, seed):
    # Deterministic 5-node / 7-edge graph fixture (one self-loop), content
    # derived from (seed, index) only - byte-identical on every platform.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    nodes = [f"{seed}-n{i}" for i in range(5)]
    edges = [(0, 1), (0, 2), (1, 2), (2, 3), (3, 4), (4, 0), (2, 2)]  # last = self-loop
    if case == "short":
        edges = edges[:-1]
    lines = [f"{nodes[a]}\t{nodes[b]}" for a, b in edges]
    if case == "malformed":
        lines[3] = nodes[2]  # missing second endpoint
    elif case == "unknown-node":
        lines[3] = f"{nodes[2]}\t{seed}-ghost"
    (target / "nodes.txt").write_bytes(("\n".join(nodes) + "\n").encode())
    (target / "edges.tsv").write_bytes(("\n".join(lines) + "\n").encode())
    print(json.dumps({"case": case, "nodes": len(nodes), "edges": len(lines)}))


mode = sys.argv[1]
if mode == "validate":
    name = sys.argv[2]
    if name not in CHECKS:
        fail("CALYX_DATASET_NOT_FOUND", f"no validation spec for {name!r}")
    if not (ROOT / name).is_dir():
        fail("CALYX_DATASET_NOT_FOUND", f"dataset dir missing: {ROOT / name}")
    report = {}
    try:
        CHECKS[name](ROOT / name, report)
    except SystemExit:
        raise
    except (zipfile.BadZipFile, tarfile.ReadError, gzip.BadGzipFile, EOFError,
            KeyError, OSError, ValueError) as err:
        # Corrupt/truncated archive bytes are an integrity failure - closed
        # catalog code, never a raw traceback (same contract as
        # verify_dataset.sh count_rows, fixed in #553).
        fail("CALYX_DATASET_CHECKSUM_MISMATCH",
             f"{name}: unreadable/corrupt archive: {type(err).__name__}: {err}")
    print(json.dumps({name: report}, sort_keys=True))
elif mode == "validate-spec":
    name, expect = sys.argv[2], json.loads(sys.argv[3])
    report = {}
    check_edgelist(ROOT / name, report, expect)
    print(json.dumps({name: report}, sort_keys=True))
elif mode == "gen-fixture":
    gen_fixture(sys.argv[2], sys.argv[3], sys.argv[4])
else:
    fail("CALYX_DATASET_MANIFEST_INVALID", f"unknown python mode {mode!r}")
PY
}

download_verified() {
  # Public non-HF hosts: never send the HF token (no credential leakage).
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
  # Secrets gate (PH69 acquisition contract) runs before ANY directory is
  # created, even though these public mirrors need no auth - a box without
  # provisioned secrets must not get a partial catalog.
  if [[ -z "${HF_HUB_TOKEN:-${HF_TOKEN:-}}" ]]; then
    fail CALYX_SECRET_MISSING "HF_HUB_TOKEN"
  fi
  if [[ ! -d "$DATASET_ROOT" ]]; then
    fail CALYX_DATASET_NOT_FOUND "dataset root missing: $DATASET_ROOT (PH00 ZFS provisioning)"
  fi

  echo "=== download (pinned sources + pre-recorded sha256) ==="
  local spec dataset url local_name bytes sha
  for spec in "${FILES[@]}"; do
    IFS='|' read -r dataset url local_name bytes sha <<<"$spec"
    mkdir -p "$DATASET_ROOT/$dataset"
    download_verified "$url" "$DATASET_ROOT/$dataset/$local_name" "$bytes" "$sha"
  done

  echo "=== validate (graph structure from bytes) ==="
  local name
  for name in "${DATASETS[@]}"; do
    run_python validate "$name"
  done

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register wordnet \
    --source "$WORDNET_URL" \
    --revision "WordNet 3.0 via nltk_data@${NLTK_COMMIT:0:12} synsets=117659" \
    --license "WordNet 3.0 license (Princeton, free incl. commercial)" \
    --tests "Lodestar kernel-only recall >=0.95 - lexical graph (PH70 issue #561)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register conceptnet \
    --source "$CONCEPTNET_URL" \
    --revision "5.7.0 (2019 release) edges=34074917" \
    --license "CC-BY-SA-4.0 (ConceptNet)" \
    --tests "Lodestar kernel-only recall >=0.95 - commonsense graph (PH70 issue #561)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register cora \
    --source "$CORA_URL" \
    --revision "LINQS original (raw, directed) nodes=2708 edges=5429" \
    --license "LINQS research distribution" \
    --tests "Lodestar kernel-only recall >=0.95 - citation graph (PH70 issue #561)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register ogbn \
    --source "$OGBN_URL" \
    --revision "ogbn-arxiv (OGB 2020-05-04) nodes=169343 edges=1166243" \
    --license "ODC-BY (OGB ogbn-arxiv)" \
    --tests "Lodestar kernel-only recall >=0.95 - citation graph at scale (PH70 issue #561)"

  echo "NOTICE: wiktionary_defn_graph NOT acquired - no immutable upstream pin exists"
  echo "        (kaikki.org and dumps.wikimedia.org rotate artifacts in place)."
  echo "        Card gate needs >=3 of 5 graph corpora; this run verified 4 of 5."
  echo "acquire_graph_kernel: OK (4/5 corpora; wiktionary deferred LOUDLY)"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Known input -> hand-derived expected output. Plain-byte fixtures (no parquet
# dependency), so the pinned sha is platform-independent.
FIXTURE_SHA="cdb76ac0c5fdf456310e826588a07516b3ff5735205cea350de396c3b6d747ff"

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

  local spec_good='{"nodes":5,"edges":7}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic 5-node/7-edge fixture (incl. self-loop): known counts + pinned sha256"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  [[ "$gen_out" == '{"case": "good", "nodes": 5, "edges": 7}' ]] \
    || { echo "SELF-TEST FAILED: generator output != hand-computed expectation" >&2; exit 1; }
  local fixture_sha
  fixture_sha="$(cat "$tmp_root/fixture_good/nodes.txt" "$tmp_root/fixture_good/edges.tsv" | sha256sum | cut -d' ' -f1)"
  if [[ "$fixture_sha" != "$FIXTURE_SHA" ]]; then
    echo "SELF-TEST FAILED: fixture sha256 $fixture_sha != pinned $FIXTURE_SHA (generator drift)" >&2
    exit 1
  fi
  local val_out
  val_out="$(run_python validate-spec fixture_good "$spec_good")"
  echo "    $val_out"
  [[ "$val_out" == '{"fixture_good": {"edges": 7, "nodes": 5, "self_loops": 1}}' ]] \
    || { echo "SELF-TEST FAILED: self-loop not recorded as expected" >&2; exit 1; }

  step "edge 1: malformed line (missing second node) -> CALYX_DATASET_SCHEMA_MISMATCH, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_malformed" malformed s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_malformed "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: edge references unknown node -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_ghost" unknown-node s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_ghost "$spec_good"

  step "edge 3: partial edge list (6 of 7) -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 4: register then tamper bytes -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_graph_kernel.sh self-test"
  show_catalog "after register"
  printf 'tampered\n' >> "$tmp_root/fixture_good/edges.tsv"
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
  --validate) shift; run_python validate "$@" ;;
  --validate-spec) shift; run_python validate-spec "$@" ;;
  --gen-fixture) shift; run_python gen-fixture "$@" ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
