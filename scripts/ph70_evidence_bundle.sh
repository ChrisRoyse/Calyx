#!/usr/bin/env bash
set -euo pipefail

METRICS_DIR=${CALYX_METRICS_DIR:-/zfs/hot/calyx/metrics}
OUT_DIR=${CALYX_PH70_BUNDLE_DIR:-$METRICS_DIR/ph70_evidence_bundle}
ISSUE=${CALYX_PH70_ISSUE:-564}
REPO=${CALYX_GITHUB_REPO:-ChrisRoyse/Calyx}

usage() {
  cat <<'EOF'
usage: scripts/ph70_evidence_bundle.sh [--self-test]

Collects PH70 metric readbacks and screenshots from CALYX_METRICS_DIR into a
hash-manifested tarball. Set CALYX_PH70_POST_ISSUE=1 to post the manifest and
bundle path to the GitHub issue with gh.
EOF
}

required_files() {
  cat <<EOF
anneal_j_series.jsonl
anneal_j_summary.json
anneal_p99_delta.txt
anneal_goodhart.txt
anneal_j_grafana.png
EOF
}

collect_bundle() {
  local bundle_root="$OUT_DIR/files"
  rm -rf -- "$OUT_DIR"
  mkdir -p "$bundle_root"
  while IFS= read -r name; do
    if [[ ! -f "$METRICS_DIR/$name" ]]; then
      echo "missing required PH70 evidence file: $METRICS_DIR/$name" >&2
      exit 1
    fi
    cp -a -- "$METRICS_DIR/$name" "$bundle_root/$name"
  done < <(required_files)
  while IFS= read -r path; do
    local dest="$bundle_root/$(basename "$path")"
    if [[ ! -e "$dest" ]]; then
      cp -a -- "$path" "$dest"
    fi
  done < <(
    find "$METRICS_DIR" -maxdepth 1 -type f \
      \( -name '*recall*' -o -name '*bits*' -o -name '*kernel*' -o -name '*guard*' -o -name '*oracle*' -o -name '*anneal*' \)
  )
  if [[ -n "${CALYX_SYNAPSE_AUDIT_BUNDLE:-}" && -f "${CALYX_SYNAPSE_AUDIT_BUNDLE:-}" ]]; then
    cp -a -- "$CALYX_SYNAPSE_AUDIT_BUNDLE" "$bundle_root/synapse_audit_export_bundle.json"
  fi
  cat >"$OUT_DIR/synapse_audit_export_request.json" <<EOF
{
  "synapse_tool": "audit_export_bundle",
  "profile_id": "vscode",
  "output_path": "$OUT_DIR/synapse_audit_export_bundle.json",
  "redaction_policy": "strict",
  "note": "Agent performs the actual Synapse MCP call and places/cites its output with this bundle."
}
EOF
  write_manifest "$bundle_root"
  tar -C "$OUT_DIR" -czf "$OUT_DIR/ph70_evidence_bundle.tar.gz" files ph70_evidence_manifest.json synapse_audit_export_request.json
  write_issue_body
  maybe_post_issue
  printf '%s\n' "$OUT_DIR/ph70_evidence_bundle.tar.gz"
}

write_manifest() {
  local bundle_root=${1:?}
  python3 - "$bundle_root" "$OUT_DIR/ph70_evidence_manifest.json" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
files = []
for path in sorted(root.iterdir()):
    if path.is_file():
        data = path.read_bytes()
        files.append({
            "name": path.name,
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })
manifest = {
    "source_of_truth": str(root),
    "file_count": len(files),
    "files": files,
}
out.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
PY
}

write_issue_body() {
  local manifest="$OUT_DIR/ph70_evidence_manifest.json"
  local manifest_bytes
  local manifest_sha
  local manifest_file_count
  manifest_bytes="$(wc -c <"$manifest")"
  manifest_sha="$(sha256sum "$manifest" | cut -d' ' -f1)"
  manifest_file_count="$(python3 - "$manifest" <<'PY'
import json, sys
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    print(json.load(handle)["file_count"])
PY
)"
  cat >"$OUT_DIR/github_issue_comment.md" <<EOF
PH70 evidence bundle generated.

- Metrics source: \`$METRICS_DIR\`
- Bundle: \`$OUT_DIR/ph70_evidence_bundle.tar.gz\`
- Manifest: \`$OUT_DIR/ph70_evidence_manifest.json\`
- Manifest bytes: \`$manifest_bytes\`
- Manifest SHA256: \`$manifest_sha\`
- Manifest file count: \`$manifest_file_count\`
- Synapse audit request: \`$OUT_DIR/synapse_audit_export_request.json\`
EOF
}

maybe_post_issue() {
  if [[ "${CALYX_PH70_POST_ISSUE:-0}" != "1" ]]; then
    return
  fi
  gh issue comment "$ISSUE" --repo "$REPO" --body-file "$OUT_DIR/github_issue_comment.md"
}

self_test() {
  local tmp
  tmp=$(mktemp -d)
  mkdir -p "$tmp/metrics"
  printf '{"j":1}\n' >"$tmp/metrics/anneal_j_series.jsonl"
  printf '{"j_growing":true}\n' >"$tmp/metrics/anneal_j_summary.json"
  printf 'p99_pass=true\n' >"$tmp/metrics/anneal_p99_delta.txt"
  printf '{"goodhart_pass":true}\n' >"$tmp/metrics/anneal_goodhart.txt"
  python3 - "$tmp/metrics/anneal_j_grafana.png" <<'PY'
import pathlib, sys
pathlib.Path(sys.argv[1]).write_bytes(
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc```\x00\x00\x00\x04\x00\x01\xf6\x178U\x00\x00\x00\x00IEND\xaeB`\x82"
)
PY
  METRICS_DIR="$tmp/metrics" OUT_DIR="$tmp/bundle" collect_bundle >/dev/null
  test -f "$tmp/bundle/ph70_evidence_bundle.tar.gz"
  test -f "$tmp/bundle/ph70_evidence_manifest.json"
  rm -rf "$tmp"
}

main() {
  case "${1:-}" in
    -h|--help)
      usage
      ;;
    --self-test)
      self_test
      ;;
    "")
      collect_bundle
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
