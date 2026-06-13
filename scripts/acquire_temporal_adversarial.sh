#!/usr/bin/env bash
# PH69 T08 / issue #558 - acquire the final modality groups (PRD 28 section 3
# rows 9-12): temporal event logs, adversarial prompt-injection text, and the
# synthetic Polis personas; validate the existing drift pair (row 12, landed
# by issue #609 - its writer is acquire_drift_pair_issue609.sh, never this
# script). Every remote file is verified against the sha256/bytes recorded
# here BEFORE download, every dataset is contract-validated, then registered
# via verify_dataset.sh (single catalog writer, PH69 T01).
#
#   acquire_temporal_adversarial.sh              acquire + validate + register
#   acquire_temporal_adversarial.sh --self-test  hermetic synthetic battery
#
# Sources (recorded in MANIFEST revision fields):
#   temporal_logs     - NAB (Numenta Anomaly Benchmark) at a pinned commit:
#                       real machine-temperature telemetry + real freeway
#                       occupancy (daily recurrence), with the labeled
#                       anomaly windows. The raw machine_temperature file
#                       contains EXACTLY one upstream sensor replay (a
#                       backward step at row 10148, 12 duplicated
#                       timestamps) - pinned as-is, not "cleaned".
#   prompt_injection  - HF deepset/prompt-injections at a pinned commit
#                       (546 train / 116 test, labels {0,1} = benign/inject).
#   synthetic_personas- in-repo scripts/gen_personas.py, seed=42: 1000
#                       personas x 21 civic axes + 600 tie pairs whose
#                       ground truth is recomputed (not trusted) at
#                       validation time.
#   drift_pair        - validated only: split criteria must name two
#                       DIFFERENT periods, else CALYX_DATASET_DRIFT_SAME_PERIOD.
#
# Fail-closed (A16): first mismatch aborts with an exact CALYX_* code.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
# fail / resolve_python / download_verified / fetch_set / gate_probe
source "$SCRIPT_DIR/dataset_acquire_lib.sh"

# --- pinned upstream state (recorded pre-download, 2026-06-12) ---------------
NAB_REV="ea702d75cc2258d9d7dd35ca8e5e2539d71f3140"
PI_REV="4f61ecb038e9c3fb77e21034b22511b523772cdd"
NAB="https://raw.githubusercontent.com/numenta/NAB/$NAB_REV"
PI="https://huggingface.co/datasets/deepset/prompt-injections/resolve/$PI_REV"
PERSONA_SEED=42

# dataset|url|local_name|bytes|sha256
FILES=(
  "temporal_logs|$NAB/data/realKnownCause/machine_temperature_system_failure.csv|machine_temperature.csv|732223|92bf5b87fc7f9bba8ca0b7ec63ccaac8cb4a1371a258e8c29a10ae9c018d82a4"
  "temporal_logs|$NAB/data/realTraffic/occupancy_6005.csv|occupancy_6005.csv|59122|cd357d7820d675074270fd976d4af1fc1e7854ecb764783028cbcb18d980c91d"
  "temporal_logs|$NAB/labels/combined_windows.json|combined_windows.json|15359|1e1fbc4601321aad8d0f8b3784c8134299379f68f6c1f7777565f8ffd57ab6b1"
  "prompt_injection|$PI/data/train-00000-of-00001-9564e8b05b4757ab.parquet|train.parquet|40323|2e10bc7ab30f542c97e4e83e2a5683000b5057d25ec10908784c631d44124c04"
  "prompt_injection|$PI/data/test-00000-of-00001-701d16158af87368.parquet|test.parquet|10892|39ac797cabc157eeed58435a08593b2952bb6cb16fc394a2d383f447cc7b246e"
)

# Subcommands: validate <name> | validate-spec <name> <json> | gen-fixture <dir> <case> <seed>
run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import csv
import datetime
import json
import os
import pathlib
import sys

import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

# Expected values recorded from the upstream bytes at pin time (2026-06-12).
REAL_SPEC = {
    "temporal_logs": {
        "machine_temperature.csv": {"rows": 22695, "unique_ts": 22683,
                                    "backward_steps": [10148], "windows": 4},
        "occupancy_6005.csv": {"rows": 2380, "unique_ts": 2380,
                               "backward_steps": [], "windows": 1},
        "window_keys": {
            "machine_temperature.csv":
                "realKnownCause/machine_temperature_system_failure.csv",
            "occupancy_6005.csv": "realTraffic/occupancy_6005.csv",
        },
    },
    "prompt_injection": {
        "train.parquet": {"rows": 546, "injections": 203},
        "test.parquet": {"rows": 116, "injections": 60},
    },
    "synthetic_personas": {"personas": 1000, "pairs": 600, "axes": 21,
                           "communities": 8},
}


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def scan_event_csv(name, fname, spec):
    path = ROOT / name / fname
    if not path.is_file():
        fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
    if path.stat().st_size == 0:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: empty file")
    with path.open(newline="") as handle:
        rows = list(csv.reader(handle))
    if rows[0] != ["timestamp", "value"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: header {rows[0]}")
    body = rows[1:]
    if len(body) != spec["rows"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/{fname}: {len(body)} rows != {spec['rows']}")
    stamps = []
    for i, row in enumerate(body):
        if len(row) != 2:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} row {i}: {len(row)} fields != 2")
        try:
            stamps.append(datetime.datetime.strptime(row[0], "%Y-%m-%d %H:%M:%S"))
            value = float(row[1])
        except ValueError:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} row {i}: unparseable {row!r}")
        if value != value or value in (float("inf"), float("-inf")):
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} row {i}: non-finite value")
    if len(set(stamps)) != spec["unique_ts"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}/{fname}: {len(set(stamps))} unique ts != {spec['unique_ts']}")
    backward = [i for i in range(len(stamps) - 1) if stamps[i + 1] < stamps[i]]
    # The recurrence/next-occurrence ground truth needs an honestly pinned
    # time axis: the exact upstream replay positions, no more, no fewer.
    if backward != spec["backward_steps"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}/{fname}: backward timestamp steps at {backward} "
             f"!= pinned upstream replays {spec['backward_steps']}")
    return stamps[0], max(stamps)


def check_temporal(name, spec):
    report = {}
    spans = {}
    for fname in ("machine_temperature.csv", "occupancy_6005.csv"):
        spans[fname] = scan_event_csv(name, fname, spec[fname])
        report[fname] = spec[fname]["rows"]
    windows = json.loads((ROOT / name / "combined_windows.json").read_bytes())
    for fname, key in spec["window_keys"].items():
        if key not in windows:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: missing windows key {key!r}")
        spans_list = windows[key]
        if len(spans_list) != spec[fname]["windows"]:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"{name}/{key}: {len(spans_list)} windows != {spec[fname]['windows']}")
        lo, hi = spans[fname]
        for start, end in spans_list:
            s = datetime.datetime.strptime(start, "%Y-%m-%d %H:%M:%S.%f")
            e = datetime.datetime.strptime(end, "%Y-%m-%d %H:%M:%S.%f")
            # Referential integrity: labels must lie inside the data they label.
            if not (lo <= s < e <= hi):
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/{key}: window [{start}, {end}] outside data span [{lo}, {hi}]")
        report[f"{fname}:windows"] = len(spans_list)
    return report


def check_injection(name, spec):
    report = {}
    for fname, expected in sorted(spec.items()):
        path = ROOT / name / fname
        if not path.is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{path} missing")
        table = pq.read_table(path)
        if table.schema.names != ["text", "label"]:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: columns {table.schema.names}")
        if table.num_rows != expected["rows"]:
            fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
                 f"{name}/{fname}: rows {table.num_rows} != {expected['rows']}")
        data = table.to_pydict()
        labels = set(data["label"])
        if labels != {0, 1}:
            fail("CALYX_DATASET_LABEL_INVALID", f"{name}/{fname}: labels {sorted(labels)}")
        injections = sum(1 for v in data["label"] if v == 1)
        if injections != expected["injections"]:
            fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
                 f"{name}/{fname}: {injections} injections != {expected['injections']}")
        if any(t is None or not str(t).strip() for t in data["text"]):
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname}: empty text rows")
        report[fname] = {"rows": table.num_rows, "injections": injections}
    return report


def check_personas(name, spec):
    base = ROOT / name
    for fname in ("personas.jsonl", "tie_pairs.jsonl", "gen_meta.json"):
        if not (base / fname).is_file():
            fail("CALYX_DATASET_NOT_FOUND", f"{base / fname} missing")
    if (base / "personas.jsonl").stat().st_size == 0:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/personas.jsonl: empty file")
    personas = {}
    for i, line in enumerate((base / "personas.jsonl").read_text().splitlines()):
        row = json.loads(line)
        axes = row["axes"]
        if len(axes) != spec["axes"]:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name} row {i}: {len(axes)} axes")
        for j, axis in enumerate(axes):
            if not isinstance(axis, float) or axis != axis or axis == 0 \
                    or axis in (float("inf"), float("-inf")):
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/personas.jsonl row {i}: axis {j + 1} invalid ({axis!r})")
        if row["persona_id"] in personas:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name} row {i}: duplicate id")
        personas[row["persona_id"]] = [1 if a > 0 else -1 for a in axes]
    if len(personas) != spec["personas"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}: {len(personas)} personas != {spec['personas']}")
    pairs = ties = 0
    for i, line in enumerate((base / "tie_pairs.jsonl").read_text().splitlines()):
        row = json.loads(line)
        for end in ("persona_a", "persona_b"):
            if row[end] not in personas:
                fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name} pair {i}: unknown {row[end]!r}")
        signs_a = personas[row["persona_a"]]
        signs_b = personas[row["persona_b"]]
        # Ground truth is RECOMPUTED from the axes, never trusted: the tie
        # label and disagree list must equal the actual sign disagreements.
        disagree = [j + 1 for j in range(spec["axes"]) if signs_a[j] != signs_b[j]]
        if row["disagree_slots"] != disagree or row["tie"] != (not disagree):
            fail("CALYX_DATASET_LABEL_INVALID",
                 f"{name}/tie_pairs.jsonl row {i}: recorded tie={row['tie']} "
                 f"disagree={row['disagree_slots']} != recomputed {disagree}")
        ties += 1 if row["tie"] else 0
        pairs += 1
    if pairs != spec["pairs"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}: {pairs} pairs != {spec['pairs']}")
    if ties == 0 or ties == pairs:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{name}: degenerate tie partition {ties}/{pairs}")
    return {"pairs": pairs, "personas": len(personas), "ties": ties}


def check_drift(name, _spec):
    meta_path = ROOT / name / "acquisition_meta.json"
    if not meta_path.is_file():
        fail("CALYX_DATASET_NOT_FOUND",
             f"{meta_path} missing - run scripts/acquire_drift_pair_issue609.sh (its single writer)")
    meta = json.loads(meta_path.read_bytes())
    criteria = meta.get("split_criteria", {})
    side_a = criteria.get("month_a")
    side_b = criteria.get("month_b")
    if not side_a or not side_b:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: split_criteria must name month_a and month_b (got {sorted(criteria)})")
    if side_a == side_b:
        # A drift pair drawn from one period cannot witness distribution
        # shift - refuse the row rather than record a vacuous dataset.
        fail("CALYX_DATASET_DRIFT_SAME_PERIOD",
             f"{name}: month_a and month_b share the same split criterion ({side_a!r})")
    counts = meta.get("row_counts", {})
    if not counts.get("month_a") or not counts.get("month_b"):
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}: empty month split ({counts})")
    return {"month_a": counts["month_a"], "month_b": counts["month_b"],
            "criteria": {"month_a": side_a, "month_b": side_b}}


CHECKERS = {"temporal_logs": check_temporal, "prompt_injection": check_injection,
            "synthetic_personas": check_personas, "drift_pair": check_drift}


def validate(name, spec, checker):
    try:
        report = checker(name, spec)
    except SystemExit:
        raise
    except Exception as err:
        # Corrupt/unreadable bytes are an integrity failure - closed catalog
        # code, never a raw traceback (#553/#555 contract).
        fail("CALYX_DATASET_CHECKSUM_MISMATCH",
             f"{name}: unreadable/corrupt data: {type(err).__name__}: {err}")
    print(json.dumps({name: report}, sort_keys=True))


mode = sys.argv[1]
if mode == "validate":
    name = sys.argv[2]
    if name not in REAL_SPEC and name != "drift_pair":
        fail("CALYX_DATASET_NOT_FOUND", f"no validation spec for {name!r}")
    validate(name, REAL_SPEC.get(name), CHECKERS[name])
elif mode == "validate-spec":
    spec = json.loads(sys.argv[3])
    checker = CHECKERS[spec.pop("checker")]
    validate(sys.argv[2], spec, checker)
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

  echo "=== generate synthetic personas (seed=$PERSONA_SEED, deterministic) ==="
  "$CALYX_DATASET_PYTHON" "$SCRIPT_DIR/gen_personas.py" \
    "$DATASET_ROOT/synthetic_personas" --seed "$PERSONA_SEED" --personas 1000 --pairs 600
  # The generator is byte-deterministic, so the production corpus is sha-
  # pinnable like any download (computed from two independent generations).
  local got
  got="$(sha256sum "$DATASET_ROOT/synthetic_personas/personas.jsonl" | cut -d' ' -f1)"
  if [[ "$got" != "59017a3b8c21193043ea60d63696b8bd0392881a38d4594b026fe97a92a9eee5" ]]; then
    fail CALYX_DATASET_CHECKSUM_MISMATCH \
      "synthetic_personas/personas.jsonl: sha256 $got != pinned (generator or python drift)"
  fi
  got="$(sha256sum "$DATASET_ROOT/synthetic_personas/tie_pairs.jsonl" | cut -d' ' -f1)"
  if [[ "$got" != "64a51a68e47854667b757a3096c16ad9de372a6eea031cf46190d1ae2c2a46fa" ]]; then
    fail CALYX_DATASET_CHECKSUM_MISMATCH \
      "synthetic_personas/tie_pairs.jsonl: sha256 $got != pinned (generator or python drift)"
  fi

  echo "=== validate (temporal/adversarial/persona/drift contracts) ==="
  run_python validate temporal_logs
  run_python validate prompt_injection
  run_python validate synthetic_personas
  run_python validate drift_pair

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  bash "$SCRIPT_DIR/verify_dataset.sh" register temporal_logs \
    --source "github:numenta/NAB (real machine temperature + freeway occupancy + labeled anomaly windows)" \
    --revision "$NAB_REV (22695 + 2380 events; raw upstream bytes incl. the single sensor replay at row 10148)" \
    --license "AGPL-3.0 (NAB, Numenta)" \
    --tests "temporal recurrence / next-occurrence intelligence (PH49/PH70; PRD 28 row 9)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register prompt_injection \
    --source "huggingface:deepset/prompt-injections" \
    --revision "$PI_REV (train 546 = 343 benign + 203 injection; test 116 = 56 benign + 60 injection)" \
    --license "Apache-2.0 (deepset)" \
    --tests "Ward injection-block >=99% at calibrated FAR (PH70 issue #562; PRD 28 row 10)"
  bash "$SCRIPT_DIR/verify_dataset.sh" register synthetic_personas \
    --source "in-repo scripts/gen_personas.py (synthetic=true, privacy-safe)" \
    --revision "seed=$PERSONA_SEED (1000 personas x 21 civic axes, 8 communities, 600 tie pairs; ground truth recomputed at validation)" \
    --license "MIT (synthetic, in-repo generator)" \
    --tests "Polis constellation/guard tie-formation (PH70 issue #611; PRD 28 row 11)"
  # drift_pair already has its catalog row from issue #609 - this script
  # validates it (above) but never writes it (single-writer principle).

  echo "=== coverage gate (DATA BUILD_DONE clause, PRD 28 section 7) ==="
  bash "$SCRIPT_DIR/check_manifest_coverage.sh"

  echo "acquire_temporal_adversarial: OK"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Hand-verified pins (seed s1 fixture + gen_personas seed 42 corpus): the
# fixture jsonl bytes are fully deterministic; the production persona corpus
# sha256s were computed from two independent generations + hashlib/coreutils.
PERSONAS_FIXTURE_SHA="48beba8f42a794d3ecb52531bd4e8350a0f796cf1167dda5fa1847fb2b6344f6"

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

  local spec_persona='{"checker":"synthetic_personas","personas":3,"pairs":2,"axes":21,"communities":2}'
  local spec_drift='{"checker":"drift_pair"}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    exit 1
  fi

  step "persona fixture: hand-computed checksum + contract green"
  local gen_out
  gen_out="$("$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  local got_sha
  got_sha="$(sha256sum "$tmp_root/fixture_good/personas.jsonl" | cut -d' ' -f1)"
  [[ "$got_sha" == "$PERSONAS_FIXTURE_SHA" ]] \
    || { echo "SELF-TEST FAILED: personas.jsonl sha256 $got_sha != pinned $PERSONAS_FIXTURE_SHA" >&2; exit 1; }
  local val_out
  val_out="$(run_python validate-spec fixture_good "$spec_persona")"
  echo "    $val_out"
  [[ "$val_out" == '{"fixture_good": {"pairs": 2, "personas": 3, "ties": 1}}' ]] \
    || { echo "SELF-TEST FAILED: validate output != hand-computed expectation" >&2; exit 1; }
  run_python validate-spec fixture_good "$spec_drift" >/dev/null \
    || { echo "SELF-TEST FAILED: drift criteria on good fixture should pass" >&2; exit 1; }

  step "determinism: gen_personas.py seed=42 twice -> identical sha256 (card edge 2)"
  local py
  py="$(resolve_python)"
  "$py" "$SCRIPT_DIR/gen_personas.py" "$tmp_root/gen_a" --seed 42 --personas 64 --pairs 24 >/dev/null
  "$py" "$SCRIPT_DIR/gen_personas.py" "$tmp_root/gen_b" --seed 42 --personas 64 --pairs 24 >/dev/null
  local sha_a sha_b
  sha_a="$(cat "$tmp_root/gen_a/personas.jsonl" "$tmp_root/gen_a/tie_pairs.jsonl" | sha256sum | cut -d' ' -f1)"
  sha_b="$(cat "$tmp_root/gen_b/personas.jsonl" "$tmp_root/gen_b/tie_pairs.jsonl" | sha256sum | cut -d' ' -f1)"
  [[ "$sha_a" == "$sha_b" ]] \
    || { echo "SELF-TEST FAILED: same-seed generations differ ($sha_a vs $sha_b)" >&2; exit 1; }
  run_python validate-spec gen_a '{"checker":"synthetic_personas","personas":64,"pairs":24,"axes":21,"communities":8}' >/dev/null \
    || { echo "SELF-TEST FAILED: generated corpus fails its own contract" >&2; exit 1; }
  echo "    seed-42 corpus sha256 $sha_a (identical twice), contract green"

  step "edge 1: zero-byte personas.jsonl -> CALYX_DATASET_ROWCOUNT_MISMATCH, no MANIFEST row"
  show_catalog "before"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_zero" zero-byte s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_zero "$spec_persona"
  show_catalog "after (must be unchanged)"

  step "edge 2: persona with 20 axes -> CALYX_DATASET_SCHEMA_MISMATCH (21-slot schema)"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_short" short-axes s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_persona"

  step "edge 3: zero-valued axis -> CALYX_DATASET_SCHEMA_MISMATCH (Gtau needs signed nonzero)"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_zaxis" zero-axis s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_zaxis "$spec_persona"

  step "edge 4: tie pair references unknown persona -> CALYX_DATASET_SCHEMA_MISMATCH"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_ghost" ghost-persona s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_ghost "$spec_persona"

  step "edge 5: tie label contradicts the axes -> CALYX_DATASET_LABEL_INVALID (ground truth recomputed)"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_lie" mislabeled-tie s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_INVALID \
    bash "$SCRIPT_PATH" --validate-spec fixture_lie "$spec_persona"

  step "edge 6: drift splits from the SAME period -> CALYX_DATASET_DRIFT_SAME_PERIOD (card edge 3)"
  "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_same" same-period s1 >/dev/null
  expect_fail CALYX_DATASET_DRIFT_SAME_PERIOD \
    bash "$SCRIPT_PATH" --validate-spec fixture_same "$spec_drift"

  step "edge 7: register, then invert one jsonl byte -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_temporal_adversarial.sh self-test"
  show_catalog "after register"
  "$(resolve_python)" - "$tmp_root/fixture_good/personas.jsonl" <<'TAMPER'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
data[40] ^= 0xFF  # invert, never overwrite-with-constant (#556 lesson)
path.write_bytes(data)
TAMPER
  expect_fail CALYX_DATASET_CHECKSUM_MISMATCH \
    bash "$SCRIPT_DIR/verify_dataset.sh" fixture_good

  step "round-trip property: register->verify green for 3 distinct seeded fixtures"
  local seed
  for seed in s2 s3 s4; do
    "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$tmp_root/fixture_rt_$seed" good "$seed" >/dev/null
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
  --gen-fixture) shift; "$(resolve_python)" "$SCRIPT_DIR/gen_personas.py" fixture "$@" ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
