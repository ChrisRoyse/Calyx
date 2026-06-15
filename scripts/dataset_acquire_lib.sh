# Shared helpers for PH69 dataset acquisition scripts (sourced, not run).
# Contract (PH69 T01/T07): pre-recorded sha256/bytes pins, fail-closed CALYX_*
# codes, token sent ONLY to huggingface.co, EXDEV-safe .tmp renames, and a
# gate-probe that turns upstream 401/403 into a graceful GATED_SKIP.
# Callers must set: DATASET_ROOT, VENV_DIR, and (for fetch) HF_HUB_TOKEN.

fail() {
  echo "$1: $2" >&2
  exit 1
}

resolve_python() {
  if [[ -n "${CALYX_DATASET_PYTHON:-}" ]]; then echo "$CALYX_DATASET_PYTHON"; return; fi
  if [[ -x "$VENV_DIR/bin/python3" ]]; then echo "$VENV_DIR/bin/python3"; return; fi
  local candidate
  for candidate in python3 python; do
    if "$candidate" -c 'import pyarrow' >/dev/null 2>&1; then echo "$candidate"; return; fi
  done
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: no python with pyarrow - run scripts/acquire_datasets.sh once or set CALYX_DATASET_PYTHON" >&2
  exit 1
}

# download_verified <url> <dest> <bytes> <sha256>
# sha256 "-" means the upstream masks LFS oids (gated repo): the pin is then
# bytes + the caller's structural contract, and verify_dataset.sh enforces
# the sha recorded at first successful fetch.
download_verified() {
  local url="$1" dest="$2" expected_bytes="$3" expected_sha="$4"
  if [[ -f "$dest" ]]; then
    if [[ "$expected_sha" == "-" ]]; then
      if [[ "$(stat -c%s "$dest")" == "$expected_bytes" ]]; then
        echo "  [cached] $dest (bytes-pinned; sha enforced by manifest)"
        return 0
      fi
      fail CALYX_DATASET_BYTES_MISMATCH \
        "$dest exists with $(stat -c%s "$dest") bytes != pinned $expected_bytes - delete it to re-acquire"
    fi
    local have_sha
    have_sha="$(sha256sum "$dest" | cut -d' ' -f1)"
    if [[ "$have_sha" == "$expected_sha" ]]; then
      echo "  [cached] $dest"
      return 0
    fi
    fail CALYX_DATASET_CHECKSUM_MISMATCH \
      "$dest exists with sha256 $have_sha != pinned $expected_sha - delete it to re-acquire"
  fi
  # Authorization goes ONLY to huggingface.co - never leak the token to
  # third-party hosts (openslr.org, robots.ox.ac.uk, cocodataset.org).
  local auth=()
  [[ "$url" == https://huggingface.co/* ]] && auth=(-H "Authorization: Bearer $HF_HUB_TOKEN")
  # .tmp beside the destination on the same mount (EXDEV-safe rename).
  curl -fsSL --retry 3 --retry-delay 5 "${auth[@]}" "$url" -o "$dest.tmp" \
    || fail CALYX_DATASET_DOWNLOAD_FAILED "$url"
  local actual_bytes actual_sha
  actual_bytes="$(stat -c%s "$dest.tmp")"
  if [[ "$actual_bytes" != "$expected_bytes" ]]; then
    rm -f "$dest.tmp"
    fail CALYX_DATASET_BYTES_MISMATCH "$url: bytes $actual_bytes != pinned $expected_bytes"
  fi
  if [[ "$expected_sha" != "-" ]]; then
    actual_sha="$(sha256sum "$dest.tmp" | cut -d' ' -f1)"
    if [[ "$actual_sha" != "$expected_sha" ]]; then
      rm -f "$dest.tmp"
      fail CALYX_DATASET_CHECKSUM_MISMATCH "$url: sha256 $actual_sha != pinned $expected_sha"
    fi
  fi
  mv "$dest.tmp" "$dest"
  echo "  [fetched] $dest ($actual_bytes bytes)"
}

# fetch_set "<dataset>|<url>|<local_name>|<bytes>|<sha256>" ...
fetch_set() {
  local spec dataset url local_name bytes sha
  for spec in "$@"; do
    IFS='|' read -r dataset url local_name bytes sha <<<"$spec"
    mkdir -p "$DATASET_ROOT/$dataset"
    download_verified "$url" "$DATASET_ROOT/$dataset/$local_name" "$bytes" "$sha"
  done
}

# gate_probe <name> <url>: 401/403 on a gated upstream is a graceful skip
# (return 1, loud CALYX_DATASET_GATED_SKIP notice, NO MANIFEST row, caller
# continues = exit 0 path). Any other non-2xx/302 stays fail-closed (A16).
gate_probe() {
  local name="$1" url="$2" code
  code="$(curl -s -o /dev/null -w '%{http_code}' -I \
    -H "Authorization: Bearer $HF_HUB_TOKEN" "$url" || echo 000)"
  if [[ "$code" == "401" || "$code" == "403" ]]; then
    echo "CALYX_DATASET_GATED_SKIP: $name (HTTP $code - accept the upstream license for the HF_HUB_TOKEN account, then rerun)"
    return 1
  fi
  if [[ "$code" != "200" && "$code" != "302" ]]; then
    fail CALYX_DATASET_DOWNLOAD_FAILED "$name gate probe HTTP $code: $url"
  fi
  return 0
}
