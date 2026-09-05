#!/usr/bin/env python3
"""Deterministically generate judge-intel-core.json (schema_version 1) for the
Cuyahoga law demo (issue #1955 / closes #1969).

Per-judge aggregation over the authored-opinion citation diet:
  authored set  = opinion_id / canonical_opinion_id from the judge-<P>.jsonl metadata.
  frontier authorities = citation_frontier_edges.csv edges whose citing_opinion_id
                         is authored (cited targets live OUTSIDE the corpus).
  in-corpus authorities = citations_cuyahoga.csv edges whose citing_opinion_id is
                         authored (cited targets live INSIDE the 12,596-opinion corpus),
                         enriched from the target vault cx-list.
  profile = census over the authored opinions plus the in-corpus citation-age diet.
All values derive from the three stable inputs + the live target cx-list readback;
no hand-entered constants.
"""
import argparse, csv, json, statistics
from collections import defaultdict


def load_authored(path):
    """rows: list of (opinion_id, canonical_opinion_id, opinion_type, date_filed,
    precedential_status). One row per jsonl record (source authored)."""
    rows = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            m = json.loads(line)["metadata"]
            rows.append((str(m["opinion_id"]), str(m["canonical_opinion_id"]),
                         m.get("opinion_type", ""), m.get("date_filed", ""),
                         m.get("precedential_status", "")))
    return rows


def load_prod_cx(path):
    """canonical_opinion_id -> {cx_id, case_name, date_filed, opinion_type,
    precedential_status, source_url}."""
    idx = {}
    for r in json.load(open(path, encoding="utf-8")):
        m = r["metadata"]
        op = str(m["canonical_opinion_id"])
        idx[op] = {
            "cx_id": r["cx_id"],
            "case_name": m.get("case_name"),
            "date_filed": m.get("date_filed"),
            "opinion_type": m.get("opinion_type"),
            "precedential_status": m.get("precedential_status"),
            "source_url": m.get("source_url"),
            "opinion_id": str(m.get("opinion_id")),
        }
    return idx


def agg_edges(csv_path, citing_col, cited_col, depth_col, authored_ids):
    """cited_opinion_id -> [edge_count, total_depth] for citing in authored_ids."""
    agg = defaultdict(lambda: [0, 0])
    with open(csv_path, encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            if row[citing_col] in authored_ids:
                cited = row[cited_col]
                agg[cited][0] += 1
                agg[cited][1] += int(row[depth_col])
    return agg


def top20(agg):
    items = [(cited, ec, td) for cited, (ec, td) in agg.items()]
    # deterministic: total_depth desc, edge_count desc, opinion_id asc (numeric)
    items.sort(key=lambda t: (-t[2], -t[1], int(t[0])))
    return items[:20]


def build_judge(person_id, name, jsonl, cit_cuya, frontier, prod_cx):
    authored = load_authored(jsonl)
    authored_ids = set(a[0] for a in authored)
    canonical_ids = set(a[1] for a in authored)

    fr = agg_edges(frontier, "citing_opinion_id", "cited_opinion_id", "depth", authored_ids)
    ic = agg_edges(cit_cuya, "citing_opinion_id", "cited_opinion_id", "depth", authored_ids)

    frontier_top = [{"edge_count": ec, "opinion_id": cid, "total_depth": td}
                    for cid, ec, td in top20(fr)]
    in_corpus_top = []
    for cid, ec, td in top20(ic):
        meta = prod_cx.get(cid, {})
        in_corpus_top.append({
            "case_name": meta.get("case_name"),
            "cx_id": meta.get("cx_id"),
            "date_filed": meta.get("date_filed"),
            "edge_count": ec,
            "opinion_id": cid,
            "opinion_type": meta.get("opinion_type"),
            "precedential_status": meta.get("precedential_status"),
            "source_url": meta.get("source_url"),
            "total_depth": td,
        })

    # profile census over authored opinions
    by_year = defaultdict(int); type_mix = defaultdict(int); prec_mix = defaultdict(int)
    dissent_n = 0
    for opid, canon, otype, dfiled, prec in authored:
        if dfiled:
            by_year[dfiled[:4]] += 1
        type_mix[otype] += 1
        prec_mix[prec] += 1
        if otype == "040dissent":
            dissent_n += 1

    # authority age diet over in-corpus edges (per edge)
    citing_date = {a[0]: a[3] for a in authored}
    ages = []; negative = 0; missing = 0
    with open(cit_cuya, encoding="utf-8") as fh:
        for row in csv.DictReader(fh):
            if row["citing_opinion_id"] not in authored_ids:
                continue
            cd = citing_date.get(row["citing_opinion_id"])
            cited_meta = prod_cx.get(row["cited_opinion_id"], {})
            ced = cited_meta.get("date_filed")
            if not cd or not ced:
                missing += 1
                continue
            from datetime import date
            y1 = date.fromisoformat(cd); y0 = date.fromisoformat(ced)
            age = (y1 - y0).days / 365.2425
            if age < 0:
                negative += 1
                continue
            ages.append(age)
    canonical_authored_n = len(authored)  # per-record count (matches old)
    profile = {
        "authority_age_excluded": {"missing_date": missing, "negative": negative},
        "authority_age_n": len(ages),
        "dissent_n": dissent_n,
        "dissent_rate": dissent_n / canonical_authored_n if canonical_authored_n else 0.0,
        "median_cited_authority_age_years": statistics.median(ages) if ages else None,
        "opinion_type_mix": dict(sorted(type_mix.items())),
        "opinions_by_year": dict(sorted(by_year.items())),
        "precedential_mix": dict(sorted(prec_mix.items())),
    }
    fr_edge = sum(ec for _, ec in ((c, v[0]) for c, v in fr.items()))
    ic_edge = sum(v[0] for v in ic.values())
    return {
        "authority_age_years": [],
        "canonical_authored_n": canonical_authored_n,
        "frontier_authorities": {"distinct_n": len(fr), "edge_n": fr_edge, "top20": frontier_top},
        "in_corpus_authorities": {"distinct_n": len(ic), "edge_n": ic_edge, "top20": in_corpus_top},
        "judge": name,
        "person_id": person_id,
        "profile": profile,
        "source_authored_n": len(authored),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--citations-cuyahoga", required=True)
    ap.add_argument("--frontier-edges", required=True)
    ap.add_argument("--prod-cx-list", required=True)
    ap.add_argument("--judge", action="append", required=True,
                    help="PERSON_ID:Full Name:/path/judge-<P>.jsonl")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    prod_cx = load_prod_cx(a.prod_cx_list)
    judges = []
    for spec in a.judge:
        pid, name, jsonl = spec.split(":", 2)
        judges.append(build_judge(int(pid), name, jsonl, a.citations_cuyahoga, a.frontier_edges, prod_cx))
    doc = {"judges": judges, "schema_version": 1}
    with open(a.out, "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=1, sort_keys=True)
        fh.write("\n")
    print(json.dumps({"judges": len(judges), "out": a.out}))


if __name__ == "__main__":
    main()
