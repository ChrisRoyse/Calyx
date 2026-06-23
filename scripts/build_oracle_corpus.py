import hashlib
import json
import sys
import urllib.request
from pathlib import Path

from datasets import load_dataset

BASE = "https://raw.githubusercontent.com/swe-bench/experiments/main/evaluation/lite"


def fetch_bytes(url):
    return urllib.request.urlopen(url, timeout=30).read()


def sha256_path(path):
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def row_text(item):
    problem = (item.get("problem_statement") or "").strip()
    hints = (item.get("hints_text") or "").strip() or "(no hints)"
    repo = (item.get("repo") or "").strip()
    base_commit = (item.get("base_commit") or "").strip()
    return "\n\n".join(
        [
            f"Repository: {repo}",
            f"Base commit: {base_commit}",
            "Problem:",
            problem[:4000],
            "Hints:",
            hints[:2000],
        ]
    )


oracle = sys.argv[1] if len(sys.argv) > 1 else "20240402_sweagent_gpt4"
out = Path(sys.argv[2] if len(sys.argv) > 2 else "/home/croyse/calyx/data/oracle_sufficiency")
out.mkdir(parents=True, exist_ok=True)

results_url = f"{BASE}/{oracle}/results/results.json"
results_bytes = fetch_bytes(results_url)
results = json.loads(results_bytes)
resolved = set(results["resolved"])
applied = set(results["applied"])
lite = load_dataset("princeton-nlp/SWE-bench_Lite")["test"]
items = [item for item in lite if item["instance_id"] in applied]

rows_path = out / "rows.jsonl"
with rows_path.open("w", encoding="utf-8") as handle:
    for item in items:
        instance_id = item["instance_id"]
        row = {
            "id": instance_id,
            "split": "test",
            "label": 1 if instance_id in resolved else 0,
            "text": row_text(item),
            "anchor_leaks_into_input": False,
            "trivial_anchor": False,
            "grounded_gate_eligible": True,
        }
        handle.write(json.dumps(row, separators=(",", ":")) + "\n")

readback = {
    "oracle_model": oracle,
    "dataset": "princeton-nlp/SWE-bench_Lite",
    "anchor": "test_pass_fail(resolved)",
    "n": len(items),
    "resolved": sum(1 for item in items if item["instance_id"] in resolved),
    "results_url": results_url,
    "results_sha256": hashlib.sha256(results_bytes).hexdigest(),
    "rows_jsonl": str(rows_path),
    "rows_jsonl_sha256": sha256_path(rows_path),
}
(out / "source_readback.json").write_text(json.dumps(readback, indent=2) + "\n", encoding="utf-8")
print(json.dumps(readback, sort_keys=True))
