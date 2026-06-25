#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
PYTHON="${CALYX_DATASET_PYTHON:-$DATASET_ROOT/.dataset_tools_venv/bin/python3}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue561-lodestar-kernel-$STAMP}"
CORPORA_DIR="$EVIDENCE_ROOT/corpora"
SOURCE_READBACK="$EVIDENCE_ROOT/lodestar_source_readback.json"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
METRICS_DIR="${CALYX_LODESTAR_METRICS_DIR:-/zfs/hot/calyx/metrics}"

if [[ ! -x "$PYTHON" ]]; then
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: $PYTHON not executable" >&2
  exit 2
fi

mkdir -p "$EVIDENCE_ROOT" "$CORPORA_DIR" "$METRICS_DIR"

"$PYTHON" - "$DATASET_ROOT" "$CORPORA_DIR" "$SOURCE_READBACK" <<'PY'
import csv, gzip, hashlib, json, tarfile, zipfile, sys
from pathlib import Path

root = Path(sys.argv[1])
out_dir = Path(sys.argv[2])
readback_path = Path(sys.argv[3])

WORDNET_LIMIT = 900
CORA_LIMIT = 900
CONCEPTNET_LIMIT = 900

def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

def write_corpus(name, source, nodes, edges):
    body = {
        "name": name,
        "source_path": str(source),
        "source_sha256": sha256(source),
        "nodes": nodes,
        "edges": edges,
    }
    path = out_dir / f"{name}.json"
    path.write_text(json.dumps(body, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    return {
        "name": name,
        "source_path": str(source),
        "source_sha256": body["source_sha256"],
        "normalized_path": str(path),
        "normalized_sha256": sha256(path),
        "nodes": len(nodes),
        "edges": len(edges),
        "anchors": sum(1 for node in nodes if node.get("anchor")),
    }

def clean_token(value: str) -> str:
    return value.replace("_", " ").replace("-", " ")

def wordnet():
    path = root / "wordnet" / "wordnet.zip"
    nodes = []
    raw_edges = []
    selected = set()
    with zipfile.ZipFile(path) as zf:
        for pos in ["noun", "verb", "adj", "adv"]:
            member = f"wordnet/data.{pos}"
            with zf.open(member) as handle:
                for line in handle:
                    raw = line.decode("utf-8", errors="replace")
                    if raw.startswith("  "):
                        continue
                    text = raw.strip()
                    if not text:
                        continue
                    parts = text.split()
                    synset = f"{pos}:{parts[0]}"
                    w_cnt = int(parts[3], 16)
                    words = [clean_token(parts[4 + i * 2]) for i in range(w_cnt)]
                    pointer_idx = 4 + (w_cnt * 2)
                    p_cnt = int(parts[pointer_idx])
                    gloss = text.split("|", 1)[1].strip() if "|" in text else " ".join(words)
                    nodes.append({
                        "id": synset,
                        "text": f"{pos} {' '.join(words)} {gloss}",
                        "anchor": len(nodes) < 10,
                    })
                    selected.add(synset)
                    offset = pointer_idx + 1
                    for idx in range(p_cnt):
                        target_offset = parts[offset + idx * 4 + 1]
                        target_pos = parts[offset + idx * 4 + 2]
                        target_name = {"n": "noun", "v": "verb", "a": "adj", "s": "adj", "r": "adv"}.get(target_pos)
                        if target_name:
                            raw_edges.append((synset, f"{target_name}:{target_offset}"))
                    if len(nodes) >= WORDNET_LIMIT:
                        break
            if len(nodes) >= WORDNET_LIMIT:
                break
    edges = [[a, b] for a, b in raw_edges if a in selected and b in selected and a != b]
    return write_corpus("wordnet", path, nodes, edges)

def cora():
    path = root / "cora" / "cora.tgz"
    nodes = []
    id_set = set()
    label_anchor = set()
    with tarfile.open(path, "r:gz") as tf:
        with tf.extractfile("cora/cora.content") as handle:
            for raw in handle:
                parts = raw.decode("utf-8").strip().split()
                paper = parts[0]
                label = parts[-1]
                features = [float(bit) for bit in parts[1:-1]]
                nodes.append({
                    "id": paper,
                    "text": f"{paper} {label}",
                    "anchor": label not in label_anchor,
                    "features": features,
                })
                label_anchor.add(label)
                id_set.add(paper)
                if len(nodes) >= CORA_LIMIT:
                    break
        edges = []
        with tf.extractfile("cora/cora.cites") as handle:
            for raw in handle:
                a, b = raw.decode("utf-8").strip().split()
                if a in id_set and b in id_set and a != b:
                    edges.append([a, b])
                    edges.append([b, a])
    return write_corpus("cora", path, nodes, edges)

def conceptnet():
    path = root / "conceptnet" / "conceptnet-assertions-5.7.0.csv.gz"
    node_text = {}
    edges = []
    relation_anchor = set()
    with gzip.open(path, "rt", encoding="utf-8", errors="replace", newline="") as handle:
        reader = csv.reader(handle, delimiter="\t")
        for row in reader:
            if len(row) < 4:
                continue
            rel, start, end = row[1], row[2], row[3]
            if not (start.startswith("/c/en/") and end.startswith("/c/en/")):
                continue
            for uri in (start, end):
                node_text.setdefault(uri, clean_token(uri.split("/")[3]))
            if start != end:
                edges.append([start, end])
            if len(node_text) >= CONCEPTNET_LIMIT:
                break
    nodes = []
    for uri, text in sorted(node_text.items()):
        rel = "root" if not relation_anchor else "concept"
        nodes.append({"id": uri, "text": text, "anchor": rel not in relation_anchor})
        relation_anchor.add(rel)
    selected = {node["id"] for node in nodes}
    edges = [edge for edge in edges if edge[0] in selected and edge[1] in selected]
    if sum(1 for node in nodes if node["anchor"]) < 3:
        for node in nodes[:3]:
            node["anchor"] = True
    return write_corpus("conceptnet", path, nodes, edges)

out_dir.mkdir(parents=True, exist_ok=True)
summary = {"corpora": [wordnet(), cora(), conceptnet()]}
readback_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

"$PYTHON" "$FSV_BOUNDED" summarize "$SOURCE_READBACK" \
  --field wordnet_nodes=.corpora[0].nodes \
  --field cora_nodes=.corpora[1].nodes \
  --field conceptnet_nodes=.corpora[2].nodes

cd "$REPO_ROOT"
"$PYTHON" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/lodestar_kernel_validate.log" \
  --stderr "$EVIDENCE_ROOT/lodestar_kernel_validate.stderr" \
  -- cargo run -p calyx-cli -- lodestar kernel-validate \
  --corpora-dir "$CORPORA_DIR" \
  --metrics-dir "$METRICS_DIR" \
  --query-limit "${CALYX_LODESTAR_QUERY_LIMIT:-500}" \
  --top-k "${CALYX_LODESTAR_TOP_K:-10}" \
  --min-ratio "${CALYX_LODESTAR_MIN_RATIO:-0.95}"

printf 'LODESTAR_KERNEL_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
printf 'LODESTAR_KERNEL_METRICS_DIR=%s\n' "$METRICS_DIR"
