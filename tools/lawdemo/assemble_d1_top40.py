#!/usr/bin/env python3
"""Assemble D1 doctrine-kernel physical centrality ranking (issue #1955).

kernel-members-centrality-physical.json = the kernel member cx_ids joined to the
spectral-communities physical report (centrality/community/fiedler/degree) and
enriched with the target-vault cx-list metadata. kernel-top40-physical.json =
the 40 highest eigenvector-centrality members. Mirrors the accepted issue #1476
artifact schema; every value derives from live readbacks of the new D1 vault.
"""
import argparse, json


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kernel-json", required=True)
    ap.add_argument("--spectral", required=True)
    ap.add_argument("--cx-list", required=True)
    ap.add_argument("--out-centrality", required=True)
    ap.add_argument("--out-top40", required=True)
    a = ap.parse_args()

    members = json.load(open(a.kernel_json, encoding="utf-8"))["kernel"]["members"]
    spec = {r["cx_id"]: r for r in json.load(open(a.spectral, encoding="utf-8"))["members"]}
    cx = {r["cx_id"]: r for r in json.load(open(a.cx_list, encoding="utf-8"))}

    rows = []
    for m in members:
        s = spec[m]
        c = cx[m]
        md = c["metadata"]
        rows.append({
            "cx_id": m,
            "community": s["community"],
            "fiedler_value": s["fiedler_value"],
            "centrality_score": s["centrality_score"],
            "frequency_weight": s["frequency_weight"],
            "degree": s["degree"],
            "opinion_id": str(md["opinion_id"]),
            "case_name": md.get("case_name"),
            "date_filed": md.get("date_filed"),
            "source_url": md.get("source_url"),
            "input_ref": c["input_ref"],
        })
    # deterministic: centrality desc, degree desc, cx_id asc
    rows.sort(key=lambda r: (-r["centrality_score"], -r["degree"], r["cx_id"]))
    json.dump(rows, open(a.out_centrality, "w", encoding="utf-8"), indent=1)
    json.dump(rows[:40], open(a.out_top40, "w", encoding="utf-8"), indent=1)
    print(json.dumps({"members": len(rows), "top40": min(40, len(rows)),
                      "top1": {"cx_id": rows[0]["cx_id"], "case_name": rows[0]["case_name"],
                               "centrality": rows[0]["centrality_score"]}}))


if __name__ == "__main__":
    main()
