#!/usr/bin/env bash
# PH69 T07 / issue #557 - acquire the audio corpora (VoxCeleb1/2 test splits,
# LibriSpeech test-clean, RAVDESS, IEMOCAP) for the PH70 Ward identity-lock +
# media-panel emotion-lens FSV (#606): verify every file against the
# sha256/bytes recorded here BEFORE download, validate the audio contract,
# then register via verify_dataset.sh (single catalog writer, PH69 T01).
#
#   acquire_audio.sh              acquire + validate + register all
#   acquire_audio.sh --self-test  hermetic synthetic-fixture battery
#
# Splits (recorded in each MANIFEST revision field): VoxCeleb1/2 TEST only
# (dev is 33GB/74GB - disk budget); LibriSpeech test-clean (sha256 equals the
# independently published value AND the openslr md5
# 32fa31d27d2e1cad72775fee3f4849a9 - double-anchored); RAVDESS via the HF
# parquet conversion (banking77 refs/convert precedent; sha equals the LFS
# oid); IEMOCAP via the AbstractTTS mirror (full 10039 utterances).
# voxceleb2/iemocap are gate-probed: 401/403 -> CALYX_DATASET_GATED_SKIP
# (exit 0, NO MANIFEST row, loud notice); anything else fail-closed (A16).
#
# Audio contract (speaker identity + emotion labels are PH70 ground truth):
# zips are CRC-checked member-by-member; the 37720-pair VoxCeleb1 list has
# referential integrity against the zip (labels {0,1}, both present); vox
# meta speakers reconcile exactly (incl. the 2 upstream-removed vox2 ids);
# LibriSpeech transcript utterance ids == flac ids; RAVDESS/IEMOCAP rows,
# label domains, actors/sessions pinned below.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
# fail / resolve_python / download_verified / fetch_set / gate_probe
source "$SCRIPT_DIR/dataset_acquire_lib.sh"

# --- pinned upstream state (recorded pre-download, 2026-06-12) ---------------
VOX_REV="b8a8825c87582831cc217b6ff84a36558ced84d6"
RAVDESS_REV="7417a62952e147d11e5291c46d8a7e6f994e1ecc"
IEMOCAP_REV="9f1696a135a65ce997d898d4121c952269a822ca"
HF="https://huggingface.co/datasets"
IEMOCAP_GATE_URL="$HF/AbstractTTS/IEMOCAP/resolve/$IEMOCAP_REV/data/train-00000-of-00003.parquet"
VOX2_GATE_URL="$HF/ProgramComputer/voxceleb/resolve/$VOX_REV/vox2/vox2_test_aac.zip"

# dataset|url|local_name|bytes|sha256
FILES_OPEN=(
  "voxceleb1|$HF/ProgramComputer/voxceleb/resolve/$VOX_REV/vox1/vox1_test_wav.zip|vox1_test_wav.zip|1072793438|8de57f347fe22b2c24526e9f444f689ecf5096fc2a92018cf420ff6b5b15eaea"
  "voxceleb1|$HF/ProgramComputer/voxceleb/resolve/$VOX_REV/vox1/vox1_meta.csv|vox1_meta.csv|40782|48bcb2f0024cae5d4ccf37e4480f686afb1161c5793a2a9a4c670c0e8dab0892"
  "voxceleb1|https://www.robots.ox.ac.uk/~vgg/data/voxceleb/meta/veri_test.txt|veri_test.txt|2338640|303b2b657042a27bf465d4c8aa84e12765373cdc01046665241ccd5783bd5976"
  "librispeech|https://www.openslr.org/resources/12/test-clean.tar.gz|test-clean.tar.gz|346663984|39fde525e59672dc6d1551919b1478f724438a95aa55f874b576be21967e6c23"
  "ravdess|$HF/narad/ravdess/resolve/$RAVDESS_REV/default/ravdess-train.parquet|ravdess-train.parquet|325074433|c44869147f330af47d57991b5b4745a5d5ae0d1bc19b6e1f48e7b3bb2d5cab58"
)
FILES_VOX2=(
  "voxceleb2|$VOX2_GATE_URL|vox2_test_aac.zip|2594018146|e4d9200107a7bc60f0b620d5dc04c3aab66681b649f9c218380ac43c6c722079"
  "voxceleb2|$HF/ProgramComputer/voxceleb/resolve/$VOX_REV/vox2/vox2_meta.csv|vox2_meta.csv|159126|9a4f437fcf78b8030e597941703ab82a4d1af314f6eba3952afafe8a3664bd1b"
)
FILES_IEMOCAP=(
  "iemocap|$HF/AbstractTTS/IEMOCAP/resolve/$IEMOCAP_REV/data/train-00000-of-00003.parquet|train-00000-of-00003.parquet|488845148|8a37ed291467501fa1a689d2c6588b44c9d74a3e77e6c5a0338a008f50a95170"
  "iemocap|$HF/AbstractTTS/IEMOCAP/resolve/$IEMOCAP_REV/data/train-00001-of-00003.parquet|train-00001-of-00003.parquet|455526508|7ebfcb4b0d4f2aa7e6bdf9d81261b37b1c6bdb027750a2eefda0d3626ff19870"
  "iemocap|$HF/AbstractTTS/IEMOCAP/resolve/$IEMOCAP_REV/data/train-00002-of-00003.parquet|train-00002-of-00003.parquet|462165713|f1c1a321e5902304c8ef6280be615e0babbee7f0985a0e7ae88d95e69aad73af"
)

# name|source|revision|license|tests|rows_from (- = natural parquet rows)
REGISTERS=(
  "voxceleb1|huggingface:ProgramComputer/voxceleb vox1 + robots.ox.ac.uk veri_test.txt|$VOX_REV (TEST split only - disk budget: 4874 wavs / 40 speakers / 37720 verification pairs; dev is 33GB)|CC-BY-4.0 (VoxCeleb1, research)|Ward identity-lock + speaker-MI FSV - speaker-identity ground truth (PH70 issue #606)|vox1_test_wav.zip"
  "voxceleb2|huggingface:ProgramComputer/voxceleb vox2|$VOX_REV (TEST split only - disk budget: 36237 m4a / 118 speakers; meta lists 120 test ids, id04170+id05348 audio removed upstream)|CC-BY-4.0 (VoxCeleb2, research)|Ward identity-lock + speaker-MI FSV - second speaker corpus (PH70 issue #606)|vox2_test_aac.zip"
  "librispeech|openslr.org/resources/12 (LibriSpeech)|test-clean split - disk budget (2620 utterances / 40 speakers / 87 chapters; sha256 cross-anchored vs published + openslr md5)|CC-BY-4.0 (LibriSpeech)|Ward identity-lock FSV - clean speech speaker ground truth (PH70 issue #606)|test-clean.tar.gz"
  "ravdess|huggingface:narad/ravdess refs/convert/parquet (zenodo RAVDESS speech)|$RAVDESS_REV (1440 speech clips / 24 actors / 8 emotions; audio-only split of the 7356-file full RAVDESS)|CC-BY-NC-SA-4.0 (RAVDESS)|media-panel emotion-lens FSV - 8-class emotion labels (PH70 issue #606)|-"
  "iemocap|huggingface:AbstractTTS/IEMOCAP (mirror of USC IEMOCAP)|$IEMOCAP_REV (full 10039 utterances / 5 sessions / 10 major emotions + soft scores)|IEMOCAP research license (USC SAIL)|media-panel emotion-lens FSV - acted dyadic emotion ground truth (PH70 issue #606)|-"
)

# Subcommands: validate <name> | validate-spec <name> <json> | gen-fixture <dir> <case> <seed>
run_python() {
  local py
  py="$(resolve_python)"
  CALYX_DATASET_ROOT="$DATASET_ROOT" "$py" - "$@" <<'PY'
import csv, json, os, pathlib, sys, tarfile, zipfile
import pyarrow as pa
import pyarrow.parquet as pq

ROOT = pathlib.Path(os.environ["CALYX_DATASET_ROOT"])

# Expected values recorded from the upstream bytes at pin time (2026-06-12).
REAL_SPEC = {
    "voxceleb1": {"wavs": 4874, "speakers": 40, "pairs": 37720, "meta_rows": 1251, "meta_test": 40},
    "voxceleb2": {"clips": 36237, "speakers": 118, "meta_rows": 6114, "meta_test": 120,
                  "absent": ["id04170", "id05348"]},
    "librispeech": {"flac": 2620, "speakers": 40, "trans": 87},
    "ravdess": {"rows": 1440, "labels": 8, "actors": 24, "genders": ["female", "male"], "statements": 2},
    "iemocap": {"rows": 10039, "sessions": 5, "genders": ["Female", "Male"],
                "emotions": ["angry", "disgust", "excited", "fear", "frustrated", "happy",
                             "neutral", "other", "sad", "surprise"]},
}

def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)

def checked_zip_names(name, fname, suffix):
    """CRC-check every member (central-dir namelist alone never reads the
    data bytes), then return the member names with the given suffix."""
    zf = zipfile.ZipFile(ROOT / name / fname)
    bad = zf.testzip()
    if bad is not None:
        fail("CALYX_DATASET_CHECKSUM_MISMATCH", f"{name}/{fname}: member {bad!r} fails CRC")
    return {n for n in zf.namelist() if n.endswith(suffix)}

def check_pairs(name, fname, wav_names, expected_pairs):
    path = ROOT / name / fname
    if path.stat().st_size == 0:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: empty pairs file")
    rows, labels = 0, set()
    for i, line in enumerate(path.read_text().splitlines()):
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) != 3:
            fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/{fname} line {i}: {len(parts)} fields != 3")
        label, ref_a, ref_b = parts
        if label not in ("0", "1"):
            fail("CALYX_DATASET_LABEL_INVALID", f"{name}/{fname} line {i}: label {label!r} not in {{0,1}}")
        for ref in (ref_a, ref_b):
            # The official pairs list omits the zip's wav/ prefix.
            if f"wav/{ref}" not in wav_names:
                fail("CALYX_DATASET_SCHEMA_MISMATCH",
                     f"{name}/{fname} line {i}: pair member {ref!r} not in the test zip")
        labels.add(label)
        rows += 1
    if rows != expected_pairs:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: {rows} pairs != expected {expected_pairs}")
    if labels != {"0", "1"}:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{name}/{fname}: labels {sorted(labels)} - need both same/different classes")
    return rows

def meta_ids(name, fname, delimiter, expected_rows, expected_test):
    with (ROOT / name / fname).open(encoding="utf-8-sig", newline="") as handle:
        body = [[cell.strip() for cell in row] for row in list(csv.reader(handle, delimiter=delimiter))[1:] if row]
    if len(body) != expected_rows:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: {len(body)} rows != expected {expected_rows}")
    test = {row[0] for row in body if row[-1] == "test"}
    if len(test) != expected_test:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/{fname}: {len(test)} test ids != expected {expected_test}")
    return test

def check_voxceleb1(name, spec):
    wavs = checked_zip_names(name, "vox1_test_wav.zip", ".wav")
    speakers = {n.split("/")[-3] for n in wavs}
    if len(wavs) != spec["wavs"] or len(speakers) != spec["speakers"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}: {len(wavs)} wavs / {len(speakers)} speakers != {spec['wavs']} / {spec['speakers']}")
    test_ids = meta_ids(name, "vox1_meta.csv", "\t", spec["meta_rows"], spec["meta_test"])
    if speakers != test_ids:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: zip speakers != meta test ids ({sorted(speakers ^ test_ids)[:4]} ...)")
    pairs = check_pairs(name, "veri_test.txt", wavs, spec["pairs"])
    return {"wavs": len(wavs), "speakers": len(speakers), "pairs": pairs}

def check_voxceleb2(name, spec):
    clips = checked_zip_names(name, "vox2_test_aac.zip", ".m4a")
    speakers = {n.split("/")[-3] for n in clips}
    if len(clips) != spec["clips"] or len(speakers) != spec["speakers"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}: {len(clips)} clips / {len(speakers)} speakers != {spec['clips']} / {spec['speakers']}")
    test_ids = meta_ids(name, "vox2_meta.csv", ",", spec["meta_rows"], spec["meta_test"])
    if not speakers <= test_ids:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: zip speakers not a subset of meta test ids: {sorted(speakers - test_ids)[:4]}")
    if sorted(test_ids - speakers) != spec["absent"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: meta-test ids without audio {sorted(test_ids - speakers)} != pinned removals {spec['absent']}")
    return {"clips": len(clips), "speakers": len(speakers)}

def check_librispeech(name, spec):
    flac_ids, trans_ids, speakers = set(), set(), set()
    trans_files = 0
    # Full r:gz iteration decompresses everything - the gzip CRC trailer
    # guards the bytes; member reads collect the referential id sets.
    with tarfile.open(ROOT / name / "test-clean.tar.gz", "r:gz") as tar:
        for member in tar:
            parts = member.name.split("/")
            if member.name.endswith(".flac"):
                flac_ids.add(parts[-1][:-5])
                speakers.add(parts[2])
            elif member.name.endswith(".trans.txt"):
                trans_files += 1
                for line in tar.extractfile(member).read().decode().splitlines():
                    if line.strip():
                        trans_ids.add(line.split(maxsplit=1)[0])
    if len(flac_ids) != spec["flac"] or len(speakers) != spec["speakers"] or trans_files != spec["trans"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH",
             f"{name}: {len(flac_ids)} flac / {len(speakers)} speakers / {trans_files} transcripts "
             f"!= expected {spec['flac']} / {spec['speakers']} / {spec['trans']}")
    if flac_ids != trans_ids:
        fail("CALYX_DATASET_SCHEMA_MISMATCH",
             f"{name}: flac utterance ids != transcript ids ({len(flac_ids ^ trans_ids)} differ)")
    return {"flac": len(flac_ids), "speakers": len(speakers)}

def check_ravdess(name, spec):
    table = pq.read_table(ROOT / name / "ravdess-train.parquet",
                          columns=["labels", "speaker_id", "speaker_gender", "text"])
    if table.num_rows != spec["rows"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}: rows {table.num_rows} != expected {spec['rows']}")
    labels = set(table.column("labels").to_pylist())
    if labels != set(range(spec["labels"])):
        fail("CALYX_DATASET_LABEL_INVALID",
             f"{name}: emotion labels {sorted(labels)} != exactly {{0..{spec['labels'] - 1}}}")
    actors = set(table.column("speaker_id").to_pylist())
    if actors != {str(i) for i in range(1, spec["actors"] + 1)}:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: {len(actors)} actors != expected 1..{spec['actors']}")
    if sorted(set(table.column("speaker_gender").to_pylist())) != spec["genders"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: gender domain drift")
    if len(set(table.column("text").to_pylist())) != spec["statements"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: statement count != the {spec['statements']} RAVDESS prompts")
    return {"rows": table.num_rows, "labels": len(labels), "actors": len(actors)}

def check_iemocap(name, spec):
    rows, files, emotions, genders, sessions = 0, [], set(), set(), set()
    for shard in sorted((ROOT / name).glob("train-*.parquet")):
        table = pq.read_table(shard, columns=["file", "major_emotion", "gender"])
        rows += table.num_rows
        files.extend(table.column("file").to_pylist())
        emotions.update(table.column("major_emotion").to_pylist())
        genders.update(table.column("gender").to_pylist())
    sessions = {f[:5] for f in files}
    if rows != spec["rows"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}: rows {rows} != expected {spec['rows']}")
    if len(set(files)) != len(files):
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: {len(files) - len(set(files))} duplicate file ids")
    if sorted(emotions) != spec["emotions"]:
        fail("CALYX_DATASET_LABEL_INVALID",
             f"{name}: major_emotion domain {sorted(emotions)} != pinned {spec['emotions']}")
    if sorted(genders) != spec["genders"] or len(sessions) != spec["sessions"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}: genders {sorted(genders)} / sessions {sorted(sessions)} drift")
    return {"rows": rows, "emotions": len(emotions), "sessions": len(sessions)}

def check_fixture(name, spec):
    """Fixture contract: the card's synthetic 3-clip metadata CSV + a wav zip
    + a verification-pairs file - the same primitives as production."""
    with (ROOT / name / "clips.csv").open(newline="") as handle:
        rows = list(csv.reader(handle))
    if [cell.strip() for cell in rows[0]] != ["clip_id", "speaker", "emotion"]:
        fail("CALYX_DATASET_SCHEMA_MISMATCH", f"{name}/clips.csv: bad header {rows[0]}")
    body = rows[1:]
    if len(body) != spec["clips"]:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"{name}/clips.csv: {len(body)} rows != expected {spec['clips']}")
    emotions = {row[2] for row in body}
    if len(emotions) != spec["classes"]:
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"{name}/clips.csv: {len(emotions)} emotion classes != expected {spec['classes']} (need >=2)")
    pairs = check_pairs(name, "pairs.txt", checked_zip_names(name, "clips.zip", ".wav"), spec["pairs"])
    return {"clips": len(body), "classes": len(emotions), "pairs": pairs}

CHECKERS = {"voxceleb1": check_voxceleb1, "voxceleb2": check_voxceleb2,
            "librispeech": check_librispeech, "ravdess": check_ravdess, "iemocap": check_iemocap}

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
    # Deterministic micro audio dataset (the card's unit fixture): 3 clips
    # over 2 speakers x 2 emotions, a zip of "wavs" (fixed ZipInfo
    # timestamps), and a 3-line verification-pairs file.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    clips = [(f"fx-{seed}-1", "spkA", "happy"), (f"fx-{seed}-2", "spkB", "sad"),
             (f"fx-{seed}-3", "spkA", "sad")]
    if case == "mono-label":
        clips = [(c, s, "happy") for c, s, _ in clips]
    if case == "short":
        clips = clips[:2]
    (target / "clips.csv").write_bytes(
        ("\n".join(["clip_id,speaker,emotion"] + [",".join(c) for c in clips]) + "\n").encode())
    with zipfile.ZipFile(target / "clips.zip", "w", zipfile.ZIP_STORED) as zf:
        for i, (clip, _, _) in enumerate(clips):
            info = zipfile.ZipInfo(f"wav/{clip}.wav", date_time=(1980, 1, 1, 0, 0, 0))
            zf.writestr(info, b"RIFF" + f"{seed}-{i}".encode() * 8)
    pair_rows = [("1", f"{clips[0][0]}.wav", f"{clips[0][0]}.wav"),
                 ("0", f"{clips[0][0]}.wav", f"{clips[1][0]}.wav"),
                 ("1", f"{clips[-1][0]}.wav", f"{clips[-1][0]}.wav")]
    if case == "ghost-pair":
        pair_rows[2] = ("1", f"fx-{seed}-9.wav", f"{clips[0][0]}.wav")
    elif case == "bad-pair-label":
        pair_rows[2] = ("2",) + pair_rows[2][1:]
    if case == "zero-byte":
        (target / "pairs.txt").write_bytes(b"")
    else:
        (target / "pairs.txt").write_bytes(("\n".join(" ".join(r) for r in pair_rows) + "\n").encode())
    print(json.dumps({"case": case, "clips": len(clips), "pairs": len(pair_rows)}))

mode = sys.argv[1]
if mode == "validate":
    name = sys.argv[2]
    if name not in REAL_SPEC:
        fail("CALYX_DATASET_NOT_FOUND", f"no validation spec for {name!r}")
    validate(name, REAL_SPEC[name], CHECKERS[name])
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
  local acquired=()

  echo "=== voxceleb1 + librispeech + ravdess (open upstreams) ==="
  fetch_set "${FILES_OPEN[@]}"
  run_python validate voxceleb1
  run_python validate librispeech
  run_python validate ravdess
  acquired+=(voxceleb1 librispeech ravdess)

  echo "=== voxceleb2 + iemocap (gate-probed upstreams) ==="
  if gate_probe voxceleb2 "$VOX2_GATE_URL"; then
    fetch_set "${FILES_VOX2[@]}"
    run_python validate voxceleb2
    acquired+=(voxceleb2)
  fi
  if gate_probe iemocap "$IEMOCAP_GATE_URL"; then
    fetch_set "${FILES_IEMOCAP[@]}"
    run_python validate iemocap
    acquired+=(iemocap)
  fi

  echo "=== register (canonical MANIFEST writer, PH69 T01) ==="
  local entry name source revision license tests rows_from
  for entry in "${REGISTERS[@]}"; do
    IFS='|' read -r name source revision license tests rows_from <<<"$entry"
    [[ " ${acquired[*]} " == *" $name "* ]] || continue
    local cmd=(bash "$SCRIPT_DIR/verify_dataset.sh" register "$name"
               --source "$source" --revision "$revision" --license "$license" --tests "$tests")
    [[ "$rows_from" != "-" ]] && cmd+=(--rows-from "$rows_from")
    "${cmd[@]}"
  done
  echo "acquire_audio: OK (${acquired[*]})"
}

# --- self-test: hermetic synthetic fixtures + edge battery -------------------
# Hand-computed from the literal fixture bytes (seed s1): clips.csv and
# clips.zip are byte-deterministic (fixed content / fixed ZipInfo timestamps).
CLIPSCSV_FIXTURE_SHA="754f8e548c694ce6d9da5ceff432eea11422a551237d0537b7658a85c361e7b0"
CLIPSZIP_FIXTURE_SHA="7329197776f3a6a51e6198ffce20342fc2509ac3fb620d1549ef16a738eb009d"

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

  local spec_good='{"clips":3,"classes":2,"pairs":3}'

  step "missing HF_HUB_TOKEN -> CALYX_SECRET_MISSING, no partial dirs created"
  expect_fail CALYX_SECRET_MISSING \
    env -u HF_HUB_TOKEN -u HF_TOKEN CALYX_DATASET_ROOT="$tmp_root" bash "$SCRIPT_PATH"
  if compgen -G "$tmp_root/*/" >/dev/null; then
    echo "SELF-TEST FAILED: token gate left partial directories behind" >&2
    ls -la "$tmp_root" >&2
    exit 1
  fi

  step "card unit fixture (3-clip CSV, 2 emotion classes): hand-computed checksums + contract green"
  local gen_out
  gen_out="$(run_python gen-fixture "$tmp_root/fixture_good" good s1)"
  echo "    $gen_out"
  local got_sha
  got_sha="$(sha256sum "$tmp_root/fixture_good/clips.csv" | cut -d' ' -f1)"
  [[ "$got_sha" == "$CLIPSCSV_FIXTURE_SHA" ]] \
    || { echo "SELF-TEST FAILED: clips.csv sha256 $got_sha != pinned $CLIPSCSV_FIXTURE_SHA" >&2; exit 1; }
  got_sha="$(sha256sum "$tmp_root/fixture_good/clips.zip" | cut -d' ' -f1)"
  [[ "$got_sha" == "$CLIPSZIP_FIXTURE_SHA" ]] \
    || { echo "SELF-TEST FAILED: clips.zip sha256 $got_sha != pinned $CLIPSZIP_FIXTURE_SHA" >&2; exit 1; }
  local val_out
  val_out="$(run_python validate-spec fixture_good "$spec_good")"
  echo "    $val_out"
  [[ "$val_out" == '{"fixture_good": {"classes": 2, "clips": 3, "pairs": 3}}' ]] \
    || { echo "SELF-TEST FAILED: validate output != hand-computed expectation" >&2; exit 1; }

  step "edge 1: zero-byte pairs file -> CALYX_DATASET_ROWCOUNT_MISMATCH, no MANIFEST row"
  show_catalog "before"
  run_python gen-fixture "$tmp_root/fixture_zero" zero-byte s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_zero "$spec_good"
  show_catalog "after (must be unchanged)"

  step "edge 2: pair references a wav missing from the zip -> CALYX_DATASET_SCHEMA_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_ghost" ghost-pair s1 >/dev/null
  expect_fail CALYX_DATASET_SCHEMA_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_ghost "$spec_good"

  step "edge 3: pair label outside {0,1} -> CALYX_DATASET_LABEL_INVALID"
  run_python gen-fixture "$tmp_root/fixture_badlabel" bad-pair-label s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_INVALID \
    bash "$SCRIPT_PATH" --validate-spec fixture_badlabel "$spec_good"

  step "edge 4: single emotion class -> CALYX_DATASET_LABEL_PARTITION_MISSING"
  run_python gen-fixture "$tmp_root/fixture_mono" mono-label s1 >/dev/null
  expect_fail CALYX_DATASET_LABEL_PARTITION_MISSING \
    bash "$SCRIPT_PATH" --validate-spec fixture_mono "$spec_good"

  step "edge 5: clips.csv short one row -> CALYX_DATASET_ROWCOUNT_MISMATCH"
  run_python gen-fixture "$tmp_root/fixture_short" short s1 >/dev/null
  expect_fail CALYX_DATASET_ROWCOUNT_MISMATCH \
    bash "$SCRIPT_PATH" --validate-spec fixture_short "$spec_good"

  step "edge 6: register, then invert one zip DATA byte -> CALYX_DATASET_CHECKSUM_MISMATCH"
  export CALYX_DATASET_PYTHON="${CALYX_DATASET_PYTHON:-$(resolve_python)}"
  bash "$SCRIPT_DIR/verify_dataset.sh" register fixture_good \
    --source "self-test fixture" --revision "s1" \
    --license "n/a (synthetic)" --tests "acquire_audio.sh self-test" \
    --rows-from "clips.csv"
  show_catalog "after register"
  "$(resolve_python)" - "$tmp_root/fixture_good/clips.zip" <<'TAMPER'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
# Local header 30B + name "wav/fx-s1-1.wav" 15B = member data from offset 45;
# byte 50 is DATA, so testzip's CRC must see it. Invert, never
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
      --license "n/a (synthetic)" --tests "round-trip property" \
      --rows-from "clips.csv" >/dev/null
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
