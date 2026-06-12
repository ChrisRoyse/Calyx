#!/usr/bin/env bash
# PH69 T07 / issue #557 - acquire the image corpora (ImageNet-1k validation
# split / CIFAR-100 / COCO val2017 + captions) for the PH70 cross-modal lens
# FSV (#607), verify every file against the sha256/bytes recorded here BEFORE
# download, validate the image contract per dataset, then register each in
# the canonical MANIFEST via verify_dataset.sh register (PH69 T01).
#
#   acquire_image.sh              acquire + validate + register all
#   acquire_image.sh --self-test  hermetic synthetic-fixture battery
#
# ImageNet is GATED on HF (gate not yet accepted for the project token - see
# issue #683). HF masks lfs.oid for gated repos, so the pin is the immutable
# commit revision + per-file byte sizes + the structural contract (50000
# rows, label domain exactly {0..999}); register records the real sha256 on
# first successful fetch and verify enforces it thereafter. A 401/403 gate
# probe yields CALYX_DATASET_GATED_SKIP (exit 0, NO MANIFEST row) per the
# card; every other failure is fail-closed.
#
# Image contract (these labels are PH70's cross-modal ground truth):
#   cifar100  - exact split rows (50000+10000), fine labels exactly {0..99},
#               coarse labels exactly {0..19}, no nulls;
#   coco      - val2017.zip exactly 5000 jpgs; captions_val2017.json: 5000
#               unique images, file names EXACTLY equal to the zip names
#               (referential integrity), 25014 captions, none empty, every
#               image captioned, instances_val2017.json: 80 categories;
#   imagenet  - 14 validation shards, 50000 rows total, labels {0..999}.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
# fail / resolve_python / download_verified / fetch_set / gate_probe
source "$SCRIPT_DIR/dataset_acquire_lib.sh"

# --- pinned upstream state (recorded pre-download, 2026-06-12) ---------------
CIFAR_REV="aadb3af77e9048adbea6b47c21a81e47dd092ae5"
IMAGENET_REV="49e2ee26f3810fb5a7536bbf732a7b07389a47b5"
IMAGENET_GATE_URL="https://huggingface.co/datasets/ILSVRC/imagenet-1k/resolve/$IMAGENET_REV/data/validation-00000-of-00014.parquet"

# dataset|url|local_name|bytes|sha256   (sha "-" = gated upstream masks LFS
# oids; bytes + structural contract pin it, register records the real sha)
HF="https://huggingface.co/datasets"
FILES=(
  "cifar100|$HF/uoft-cs/cifar100/resolve/$CIFAR_REV/cifar100/train-00000-of-00001.parquet|train.parquet|118518617|694865d6b990e234804f01268586c41e88bcbbb75e20858432c05ad4081aca23"
  "cifar100|$HF/uoft-cs/cifar100/resolve/$CIFAR_REV/cifar100/test-00000-of-00001.parquet|test.parquet|23772751|98776c529bb146a9c791229df74a5cf076be9b43d82dbbd334b6a7788d73dc68"
  "coco|http://images.cocodataset.org/zips/val2017.zip|val2017.zip|815585330|4f7e2ccb2866ec5041993c9cf2a952bbed69647b115d0f74da7ce8f4bef82f05"
  "coco|http://images.cocodataset.org/annotations/annotations_trainval2017.zip|annotations_trainval2017.zip|252907541|113a836d90195ee1f884e704da6304dfaaecff1f023f49b6ca93c4aaae470268"
)
IMAGENET_FILES=(
  "validation-00000-of-00014.parquet|479922797" "validation-00001-of-00014.parquet|484701821"
  "validation-00002-of-00014.parquet|471258806" "validation-00003-of-00014.parquet|477547459"
  "validation-00004-of-00014.parquet|477666614" "validation-00005-of-00014.parquet|476033225"
  "validation-00006-of-00014.parquet|479594555" "validation-00007-of-00014.parquet|475711811"
  "validation-00008-of-00014.parquet|476090886" "validation-00009-of-00014.parquet|470165919"
  "validation-00010-of-00014.parquet|475451921" "validation-00011-of-00014.parquet|471615473"
  "validation-00012-of-00014.parquet|492972415" "validation-00013-of-00014.parquet|484360024"
)

# Subcommands: validate <name> | validate-spec <name> <json> | gen-fixture <dir> <case> <seed>
run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import json
import os
import pathlib
import sys
import zipfile

import pyarrow as pa
import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

# Expected values recorded from the upstream bytes at pin time (2026-06-12).
REAL_SPEC = {
    "cifar100": {"splits": {"train.parquet": 50000, "test.parquet": 10000},
                 "fine": 100, "coarse": 20},
    "coco": {"images": 5000, "captions": 25014, "categories": 80,
             "instances": 36781},
    "imagenet_subset": {"shards": 14, "rows": 50000, "classes": 1000},
}


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def label_domain(name, fname, table, column, classes):
    col = table.column(column)
    if col.null_count:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname}: {col.null_count} null {column} values")
    got = set(col.to_pylist())
    if got != set(range(classes)):
        fail("CALYX_DATASET_LABEL_INVALID",
             f"{name}/{fname}: {column} domain has {len(got)} classes, "
             f"min {min(got)} max {max(got)} != exactly {{0..{classes - 1}}}")


def check_cifar(name, spec):
    report = {}
    for fname, expected in sorted(spec["splits"].items()):
        path = ROOT / name / fname
        if not path.is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
        table = pq.read_table(path, columns=["fine_label", "coarse_label"])
        if table.num_rows != expected:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"{name}/{fname}: rows {table.num_rows} != expected {expected}")
        label_domain(name, fname, table, "fine_label", spec["fine"])
        label_domain(name, fname, table, "coarse_label", spec["coarse"])
        report[fname] = expected
    return report


def check_coco(name, spec):
    zpath = ROOT / name / "val2017.zip"
    apath = ROOT / name / "annotations_trainval2017.zip"
    for path in (zpath, apath):
        if not path.is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
    if zpath.stat().st_size == 0 or apath.stat().st_size == 0:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}: empty archive")
    for path in (zpath, apath):
        # namelist() only reads the central directory; testzip() CRC-checks
        # every member so corrupt bytes inside the archive cannot pass.
        bad = zipfile.ZipFile(path).testzip()
        if bad is not None:
            fail("CALYX_DATASET_CHECKSUM_MISMATCH",
                 f"{name}/{path.name}: member {bad!r} fails CRC")
    zip_names = {n.split("/")[-1] for n in zipfile.ZipFile(zpath).namelist()
                 if n.endswith(".jpg")}
    if len(zip_names) != spec["images"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/val2017.zip: {len(zip_names)} jpgs != expected {spec['images']}")
    ann = zipfile.ZipFile(apath)
    cap = json.loads(ann.read("annotations/captions_val2017.json"))
    ids = {img["id"] for img in cap["images"]}
    if len(cap["images"]) != spec["images"] or len(ids) != spec["images"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/captions: {len(cap['images'])} images ({len(ids)} unique) "
             f"!= expected {spec['images']}")
    json_names = {img["file_name"] for img in cap["images"]}
    if json_names != zip_names:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: captions file_name set != zip jpg set "
             f"({len(json_names - zip_names)} ghost, {len(zip_names - json_names)} unlabeled)")
    if len(cap["annotations"]) != spec["captions"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/captions: {len(cap['annotations'])} != expected {spec['captions']}")
    covered = set()
    for i, entry in enumerate(cap["annotations"]):
        if entry["image_id"] not in ids:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{name}/captions ann {i}: unknown image_id {entry['image_id']}")
        if not entry["caption"].strip():
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/captions ann {i}: empty caption")
        covered.add(entry["image_id"])
    if covered != ids:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{name}: {len(ids - covered)} images have no caption")
    inst = json.loads(ann.read("annotations/instances_val2017.json"))
    if len(inst["categories"]) != spec["categories"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/instances: {len(inst['categories'])} categories != {spec['categories']}")
    if len(inst["annotations"]) != spec["instances"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/instances: {len(inst['annotations'])} != expected {spec['instances']}")
    return {"images": spec["images"], "captions": spec["captions"]}


def check_imagenet(name, spec):
    rows = 0
    labels = set()
    shards = sorted((ROOT / name).glob("validation-*.parquet"))
    if len(shards) != spec["shards"]:
        fail("CALYX_DATASET_NOT_FOUND",
             f"{name}: {len(shards)} shards on disk != expected {spec['shards']}")
    for shard in shards:
        table = pq.read_table(shard, columns=["label"])
        if table.column("label").null_count:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{shard.name}: null labels")
        rows += table.num_rows
        labels.update(table.column("label").to_pylist())
    if rows != spec["rows"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}: rows {rows} != expected {spec['rows']}")
    if labels != set(range(spec["classes"])):
        fail("CALYX_DATASET_LABEL_INVALID",
             f"{name}: label domain {len(labels)} classes != exactly {{0..{spec['classes'] - 1}}}")
    return {"rows": rows, "classes": len(labels)}


def check_fixture(name, spec):
    """Fixture contract: a micro coco-shaped pair (zip of jpgs + captions
    json) plus a micro cifar-shaped parquet - same primitives as production."""
    zf = zipfile.ZipFile(ROOT / name / "images.zip")
    bad = zf.testzip()
    if bad is not None:
        fail("CALYX_DATASET_CHECKSUM_MISMATCH",
             f"{name}/images.zip: member {bad!r} fails CRC")
    zip_names = {n.split("/")[-1] for n in zf.namelist() if n.endswith(".jpg")}
    cpath = ROOT / name / "captions.json"
    if cpath.stat().st_size == 0:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/captions.json: empty file")
    cap = json.loads(cpath.read_bytes())
    ids = {img["id"] for img in cap["images"]}
    if {img["file_name"] for img in cap["images"]} != zip_names:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: captions file_name set != zip jpg set")
    for i, entry in enumerate(cap["annotations"]):
        if entry["image_id"] not in ids:
            fail("CALYX_DATASET_SCHEMA_MISMATCH",
                 f"{name}/captions ann {i}: unknown image_id {entry['image_id']}")
    if len(cap["annotations"]) != spec["captions"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/captions: {len(cap['annotations'])} != expected {spec['captions']}")
    table = pq.read_table(ROOT / name / "labels.parquet")
    if table.num_rows != spec["rows"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/labels.parquet: rows {table.num_rows} != expected {spec['rows']}")
    label_domain(name, "labels.parquet", table, "fine_label", spec["classes"])
    return {"images": len(zip_names), "captions": len(cap["annotations"]),
            "rows": table.num_rows}


def validate(name, spec, checker):
    try:
        report = checker(name, spec)
    except SystemExit:
        raise
    except Exception as err:
        # Corrupt/truncated bytes are an integrity failure - closed catalog
        # code, never a raw traceback (#553/#555 contract).
        fail("CALYX_DATASET_CHECKSUM_MISMATCH",
             f"{name}: unreadable/corrupt data: {type(err).__name__}: {err}")
    print(json.dumps({name: report}, sort_keys=True))


def gen_fixture(target_dir, case, seed):
    # Deterministic micro image dataset: 2 "jpgs" in a zip (fixed ZipInfo
    # timestamps), a captions json (3 captions), a 6-row labels parquet with
    # fine_label domain {0,1,2}. Content derives from (seed, index) only.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    names = [f"fx-{seed}-{i}.jpg" for i in range(1, 3)]
    with zipfile.ZipFile(target / "images.zip", "w", zipfile.ZIP_STORED) as zf:
        for i, fname in enumerate(names):
            info = zipfile.ZipInfo(f"val/{fname}", date_time=(1980, 1, 1, 0, 0, 0))
            zf.writestr(info, b"\xff\xd8\xff\xe0" + f"{seed}-{i}".encode() * 8)
    images = [{"id": i + 1, "file_name": fname} for i, fname in enumerate(names)]
    annotations = [
        {"image_id": 1, "caption": f"caption {seed} one"},
        {"image_id": 1, "caption": f"caption {seed} two"},
        {"image_id": 2, "caption": f"caption {seed} three"},
    ]
    if case == "ghost-image":
        annotations[2]["image_id"] = 9
    rows = 6
    if case == "short":
        rows = 5
    labels = [i % 3 for i in range(rows)]
    if case == "bad-label":
        labels[3] = 99
    if case == "zero-json":
        (target / "captions.json").write_bytes(b"")
    else:
        (target / "captions.json").write_bytes(json.dumps(
            {"images": images, "annotations": annotations}, sort_keys=True).encode())
    pq.write_table(pa.table({
        "fine_label": pa.array(labels, pa.int64()),
        "name": pa.array([f"img-{seed}-{i}" for i in range(rows)], pa.string()),
    }), target / "labels.parquet")
    print(json.dumps({"case": case, "images": len(names),
                      "captions": len(annotations), "rows": rows}))


mode = sys.argv[1]
if mode == "validate":
    name = sys.argv[2]
    if name not in REAL_SPEC:
        fail("CALYX_DATASET_NOT_FOUND", f"no validation spec for {name!r}")
    checker = {"cifar100": check_cifar, "coco": check_coco,
               "imagenet_subset": check_imagenet}[name]
    validate(name, REAL_SPEC[name], checker)
elif mode == "validate-spec":
    validate(sys.argv[2], json.loads(sys.argv[3]), check_fixture)
elif mode == "gen-fixture":
    gen_fixture(sys.argv[2], sys.argv[3], sys.argv[4])
else:
    fail("CALYX_DATASET_MANIFEST_INVALID", f"unknown python mode {mode!r}")
PY
}

acquire_all() {
  # Secrets gate before ANY directory is created (fail-closed, no partial state).
  if [[ -z "${HF_HUB_TOKEN:-${HF_TOKEN:-}}" ]]; then
    fail CALYX_SECRET_MISSING "HF_HUB_TOKEN"
  fi
  export HF_HUB_TOKEN="${HF_HUB_TOKEN:-$HF_TOKEN}"
  if [[ ! -d "$DATASET_ROOT" ]]; then
    fail CALYX_DATASET_NOT_FOUND "dataset root missing: $DATASET_ROOT (PH00 ZFS provisioning)"
  fi
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"

  echo "=== download (pinned revisions + pre-recorded sha256) ==="
  fetch_set "${FILES[@]}"

  echo "=== imagenet_subset (gated: probe before download, #683) ==="
  if gate_probe imagenet_subset "$IMAGENET_GATE_URL"; then
    local entry fname fbytes
    for entry in "${IMAGENET_FILES[@]}"; do
      IFS='|' read -r fname fbytes <<<"$entry"
      mkdir -p "$DATASET_ROOT/imagenet_subset"
      download_verified \
        "$HF/ILSVRC/imagenet-1k/resolve/$IMAGENET_REV/data/$fname" \
        "$DATASET_ROOT/imagenet_subset/$fname" "$fbytes" "-"
    done
    run_python validate imagenet_subset
    bash "$SCRIPT_DIR/verify_dataset.sh" register imagenet_subset \
      --source "huggingface:ILSVRC/imagenet-1k (gated; gate accepted per #683)" \
      --revision "$IMAGENET_REV (FULL 50000-image validation split, 14 parquet shards - disk budget allows; sha pinned at first fetch, lfs.oid masked upstream)" \
      --license "ImageNet research terms (non-commercial)" \
      --tests "media-panel cross-modal lens FSV - 1000-class image labels (PH70 issue #607)"
  fi

  echo "=== validate (image contract: counts, label domains, referential integrity) ==="
  run_python validate cifar100
  run_python validate coco

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  bash "$SCRIPT_DIR/verify_dataset.sh" register cifar100 \
    --source "huggingface:uoft-cs/cifar100" \
    --revision "$CIFAR_REV (full train 50000 + test 10000; 100 fine / 20 coarse classes)" \
    --license "CIFAR-100 (research, University of Toronto)" \
    --tests "media-panel cross-modal lens FSV - fine+coarse image labels (PH70 issue #607)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register coco \
    --source "cocodataset.org val2017 + annotations_trainval2017" \
    --revision "val2017 (5000 images, 25014 captions, 80 instance categories; archives kept whole, counts validated from bytes)" \
    --license "CC-BY-4.0 (annotations); image licenses per COCO terms" \
    --tests "media-panel cross-modal lens FSV - image captions + categories (PH70 issue #607)" \
    --rows-from "val2017.zip"

  echo "acquire_image: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Hand-computed from the literal fixture bytes (seed s1): captions.json and
# images.zip are byte-deterministic (sorted json / fixed ZipInfo timestamps).
CAPTIONS_FIXTURE_SHA="b662bcd00475f8674a1e12f05b841399a659419fb51f59c004c70b464b643a28"
IMAGESZIP_FIXTURE_SHA="0cb7ebd083cc07e2c447d9cbb6b11c7242a974da33966a687f549099c3b232ab"

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

  local spec_good='{"captions":3,"rows":6,"classes":3}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "synthetic fixture: hand-computed checksums + contract green"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  local got_sha
  got_sha="$(sha256sum "$tmp_root/fixture_good/captions.json" | cut -d' ' -f1)"
  [[ "$got_sha" == "$CAPTIONS_FIXTURE_SHA" ]] \
    || { echo "SELF-TEST FAILED: captions.json sha256 $got_sha != pinned $CAPTIONS_FIXTURE_SHA" >&2; exit 1; }
  got_sha="$(sha256sum "$tmp_root/fixture_good/images.zip" | cut -d' ' -f1)"
  [[ "$got_sha" == "$IMAGESZIP_FIXTURE_SHA" ]] \
    || { echo "SELF-TEST FAILED: images.zip sha256 $got_sha != pinned $IMAGESZIP_FIXTURE_SHA" >&2; exit 1; }
  local val_out
  val_out="$(run_python validate-spec fixture_good "$spec_good")"
  echo "    $val_out"
  [[ "$val_out" == '{"fixture_good": {"captions": 3, "images": 2, "rows": 6}}' ]] \
    || { echo "SELF-TEST FAILED: validate output != hand-computed expectation" >&2; exit 1; }

  step "edge 1: zero-byte captions json -> CALYX_DATASET_ROWCOUNT_MISMATCH, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_zero" zero-json s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_zero "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: caption references missing image -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_ghost" ghost-image s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_ghost "$spec_good"

  step "edge 3: label outside the pinned class domain -> CALYX_DATASET_LABEL_INVALID"
  run_python gen-fixture "$tmp_root/fixture_badlabel" bad-label s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_INVALID \
    bash "$SCRIPT_PATH" --validate-spec fixture_badlabel "$spec_good"

  step "edge 4: parquet short one row (CIFAR rowcount analogue) -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 5: register, then invert one zip byte -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_image.sh self-test"
  show_catalog "after register"
  "$(resolve_python)" - "$tmp_root/fixture_good/images.zip" <<'TAMPER'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
# Byte 50 sits inside the first member's DATA (local header 30B + name 15B
# = data from offset 45): testzip's CRC must see it. Invert, never
# overwrite-with-constant (#556 lesson).
data[50] ^= 0xFF
path.write_bytes(data)
TAMPER
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
