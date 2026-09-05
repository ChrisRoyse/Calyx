#!/usr/bin/env python3
"""Deterministically build the six-column opinion_cx_aliases.csv consumed by
`calyx materialize-opinion-aliases`.

Columns: opinion_id,cx_id,canonical_opinion_id,content_sha256,is_canonical,source_url
cx_id = target-vault cx of the row canonical_opinion_id (alias rows share their
canonical cx). All other columns come from the stable opinion_aliases.jsonl.
Reconstructs the never-committed builder (see #1970 / #1955).
"""
import argparse, csv, json


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cx-list", required=True)
    ap.add_argument("--aliases", required=True, help="opinion_aliases.jsonl")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    cx = {}
    for row in json.load(open(a.cx_list, encoding="utf-8")):
        op = str(row["metadata"]["canonical_opinion_id"])
        cx[op] = row["cx_id"]
    entries = alias = 0
    distinct = set()
    with open(a.out, "w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh)
        w.writerow(["opinion_id", "cx_id", "canonical_opinion_id", "content_sha256", "is_canonical", "source_url"])
        with open(a.aliases, encoding="utf-8") as af:
            for line in af:
                line = line.strip()
                if not line:
                    continue
                r = json.loads(line)
                opid = str(r["opinion_id"]); canon = str(r["canonical_opinion_id"])
                if canon not in cx:
                    raise SystemExit(f"canonical {canon} (opinion {opid}) absent from cx-list")
                w.writerow([opid, cx[canon], canon, r["content_sha256"],
                            str(bool(r["is_canonical"])).lower(), r.get("source_url", "")])
                entries += 1; distinct.add(cx[canon])
                if opid != canon:
                    alias += 1
    print(json.dumps({"entries": entries, "distinct_cx": len(distinct), "alias_entries": alias}))


if __name__ == "__main__":
    main()
