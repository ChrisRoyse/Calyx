#!/usr/bin/env python3
"""Deterministically build the citation-overlay idmap (calyx-cuyahoga-citation-overlay-input-v1).

opinion_to_cx.csv = for each row of opinion_aliases.jsonl emit
(opinion_id, cx_of[canonical_opinion_id]) where canonical_opinion_id -> cx_id
comes from the target vault base cx-list. Yields one entry per alias-generation
row (canonical + alias), with alias opinion_ids sharing their canonical cx.
Reconstructs the never-committed builder referenced by #1970 (see #1955).
"""
import argparse, csv, json, sys


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cx-list", required=True, help="readback cx-list JSON of target vault")
    ap.add_argument("--aliases", required=True, help="opinion_aliases.jsonl (opinion_id, canonical_opinion_id)")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    cx = {}
    for row in json.load(open(a.cx_list, encoding="utf-8")):
        op = str(row["metadata"]["canonical_opinion_id"])
        if op in cx and cx[op] != row["cx_id"]:
            raise SystemExit(f"conflicting cx for opinion {op}")
        cx[op] = row["cx_id"]
    entries = alias = 0
    distinct = set()
    with open(a.out, "w", newline="", encoding="utf-8") as fh:
        w = csv.writer(fh)
        w.writerow(["opinion_id", "cx_id"])
        with open(a.aliases, encoding="utf-8") as af:
            for line in af:
                line = line.strip()
                if not line:
                    continue
                r = json.loads(line)
                opid = str(r["opinion_id"])
                canon = str(r["canonical_opinion_id"])
                if canon not in cx:
                    raise SystemExit(f"canonical {canon} (opinion {opid}) absent from cx-list")
                w.writerow([opid, cx[canon]])
                entries += 1
                distinct.add(cx[canon])
                if opid != canon:
                    alias += 1
    print(json.dumps({"entries": entries, "distinct_cx": len(distinct), "alias_entries": alias}))


if __name__ == "__main__":
    main()
