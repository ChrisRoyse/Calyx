#!/usr/bin/env python3
import csv
import hashlib
import json
import os
import sys
from collections import Counter, defaultdict
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def full_name(row):
    return " ".join(
        value.strip()
        for value in (
            row.get("name_first", ""),
            row.get("name_middle", ""),
            row.get("name_last", ""),
            row.get("name_suffix", ""),
        )
        if value and value.strip()
    )


def main():
    if len(sys.argv) != 8:
        raise SystemExit(
            "usage: analyze.py CX_LIST SPECTRAL OPINION_JUDGES JUDGES GRAPH_CF OVERLAY_REPORT OUT"
        )
    cx_path, spectral_path, opinion_judges_path, judges_path, graph_path, overlay_path, out_path = map(
        Path, sys.argv[1:]
    )

    cx_rows = json.loads(cx_path.read_text(encoding="utf-8"))
    if len(cx_rows) != 12596:
        raise RuntimeError(f"physical cx row count differs: {len(cx_rows)}")
    physical = {}
    cx_to_opinion = {}
    for row in cx_rows:
        cx_id = row["cx_id"]
        metadata = row["metadata"]
        opinion_id = int(metadata["canonical_opinion_id"])
        if cx_id in cx_to_opinion or opinion_id in physical:
            raise RuntimeError("physical cx/opinion identity is duplicated")
        cx_to_opinion[cx_id] = opinion_id
        physical[opinion_id] = {
            "cx_id": cx_id,
            "case_name": metadata["case_name"],
            "date_filed": metadata["date_filed"],
            "opinion_type": metadata["opinion_type"],
            "docket_number": metadata["docket_number"],
            "source_url": metadata["source_url"],
        }

    spectral = json.loads(spectral_path.read_text(encoding="utf-8"))
    members = spectral["members"]
    if spectral["node_count"] != len(cx_rows) or len(members) != len(cx_rows):
        raise RuntimeError("spectral/physical node count differs")
    community_by_cx = {row["cx_id"]: int(row["community"]) for row in members}
    if set(community_by_cx) != set(cx_to_opinion):
        raise RuntimeError("spectral membership differs from physical cx set")

    opinion_to_person = {}
    with opinion_judges_path.open(newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            opinion_id = int(row["opinion_id"])
            if opinion_id in opinion_to_person:
                raise RuntimeError("judge opinion mapping is duplicated")
            opinion_to_person[opinion_id] = None if row["person_id"] == "UNRESOLVED" else int(row["person_id"])
    if len(opinion_to_person) != 12605:
        raise RuntimeError("judge mapping population differs")

    people = {}
    for line in judges_path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        people[int(row["person_id"])] = full_name(row)

    type_counts = Counter(row["opinion_type"] for row in physical.values())
    dissent_ids = {opinion_id for opinion_id, row in physical.items() if row["opinion_type"] == "040dissent"}
    concurrence_types = {"030concurrence", "035concurrenceinpart"}
    concurrence_ids = {
        opinion_id for opinion_id, row in physical.items() if row["opinion_type"] in concurrence_types
    }

    judge_counts = defaultdict(lambda: Counter(total=0, dissent=0, concurrence_family=0))
    unresolved_physical = 0
    resolved_type_counts = Counter()
    unresolved_type_counts = Counter()
    for opinion_id, row in physical.items():
        person_id = opinion_to_person.get(opinion_id)
        if person_id is None:
            unresolved_physical += 1
            unresolved_type_counts[row["opinion_type"]] += 1
            continue
        resolved_type_counts[row["opinion_type"]] += 1
        counts = judge_counts[person_id]
        counts["total"] += 1
        counts["dissent"] += row["opinion_type"] == "040dissent"
        counts["concurrence_family"] += row["opinion_type"] in concurrence_types

    per_judge = []
    for person_id, counts in judge_counts.items():
        total = counts["total"]
        per_judge.append(
            {
                "person_id": person_id,
                "judge": people.get(person_id, f"person {person_id}"),
                "resolved_physical_n": total,
                "dissents_n": counts["dissent"],
                "dissent_rate": counts["dissent"] / total,
                "concurrence_family_n": counts["concurrence_family"],
                "provisional_below_50": total < 50,
            }
        )
    per_judge.sort(key=lambda row: (-row["dissents_n"], -row["dissent_rate"], row["judge"]))

    communities = []
    for community in sorted(set(community_by_cx.values())):
        opinion_ids = [cx_to_opinion[cx] for cx, value in community_by_cx.items() if value == community]
        dissent_members = [opinion_id for opinion_id in opinion_ids if opinion_id in dissent_ids]
        concurrence_members = [opinion_id for opinion_id in opinion_ids if opinion_id in concurrence_ids]
        sample = sorted(
            (
                {
                    "opinion_id": opinion_id,
                    "case_name": physical[opinion_id]["case_name"],
                    "date_filed": physical[opinion_id]["date_filed"],
                }
                for opinion_id in dissent_members
            ),
            key=lambda row: (row["date_filed"], row["opinion_id"]),
            reverse=True,
        )[:5]
        communities.append(
            {
                "community": community,
                "members_n": len(opinion_ids),
                "dissents_n": len(dissent_members),
                "dissent_density": len(dissent_members) / len(opinion_ids),
                "concurrence_family_n": len(concurrence_members),
                "concurrence_family_density": len(concurrence_members) / len(opinion_ids),
                "dissent_samples": sample,
            }
        )
    communities.sort(key=lambda row: (-row["dissent_density"], -row["dissents_n"], row["community"]))

    overlay = json.loads(overlay_path.read_text(encoding="utf-8"))
    if overlay.get("status") != "ok" or overlay.get("readback", {}).get("edge_rows_written") != 28834:
        raise RuntimeError("accepted physical overlay report differs")
    skip = overlay.get("skip_report", {})
    # Overlay report schema evolved: 16 duplicate pairs are now coalesced (kept,
    # deduplicated) rather than skipped/dropped. Accept either spelling and assert
    # the honest invariant total_rows - duplicate_pairs == edges_built.
    dup_pairs = skip.get("coalesced_duplicate_pair", skip.get("skipped_duplicate_pair", 0))
    if (
        skip.get("total_rows") != 28850
        or skip.get("edges_built") != 28834
        or dup_pairs != 16
        or skip.get("edges_built") + dup_pairs != skip.get("total_rows")
    ):
        raise RuntimeError("accepted overlay source/skip accounting differs")

    traction = defaultdict(lambda: {"citing": set(), "depth_sum": 0, "max_depth": 0})
    seen_pairs = set()
    graph_rows = 0
    graph_edges = 0
    latest_graph_rows = {}
    dissent_target_edges = 0
    dissent_target_edges_missing_physical_citing = 0
    dissent_target_edges_not_later = 0
    dissent_target_edges_later = 0
    with graph_path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            graph_rows += 1
            fields = line.rstrip("\n").split("\t")
            if len(fields) != 8 or fields[0] != "CF" or fields[1] != "graph" or fields[6] != "VALUE":
                raise RuntimeError(f"physical Graph CF row is malformed: {line_number}")
            latest_graph_rows[fields[5]] = fields[7]
    for value_hex in latest_graph_rows.values():
        try:
            value = json.loads(bytes.fromhex(value_hex).decode("utf-8"))
        except (ValueError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        if not isinstance(value, dict) or value.get("edge_type") != "cites":
            continue
        if not {"citing_opinion_id", "cited_opinion_id", "depth", "weight"}.issubset(value):
            continue
        graph_edges += 1
        if value.get("provenance_dataset") != "courtlistener-citation-map-2026-06-30":
            raise RuntimeError("physical cites edge provenance differs")
        citing = int(value["citing_opinion_id"])
        cited = int(value["cited_opinion_id"])
        depth = int(value["depth"])
        expected_weight = min(depth, 10) / 10
        if depth <= 0 or float(value["weight"]) != expected_weight:
            raise RuntimeError("physical cites edge depth/weight differs")
        pair = (citing, cited)
        if pair in seen_pairs:
            raise RuntimeError("physical cites edge pair is duplicated")
        seen_pairs.add(pair)
        if cited not in dissent_ids:
            continue
        dissent_target_edges += 1
        if citing not in physical:
            dissent_target_edges_missing_physical_citing += 1
            continue
        citing_date = physical[citing]["date_filed"]
        cited_date = physical[cited]["date_filed"]
        if not citing_date or not cited_date or citing_date <= cited_date:
            dissent_target_edges_not_later += 1
            continue
        dissent_target_edges_later += 1
        value = traction[cited]
        value["citing"].add(citing)
        value["depth_sum"] += depth
        value["max_depth"] = max(value["max_depth"], depth)
    if graph_edges != 28834 or len(seen_pairs) != 28834:
        raise RuntimeError("physical Graph CF edge accounting differs")

    traction_rows = []
    for opinion_id, value in traction.items():
        citing = sorted(
            (
                {
                    "opinion_id": citing_id,
                    "case_name": physical[citing_id]["case_name"],
                    "date_filed": physical[citing_id]["date_filed"],
                    "source_url": physical[citing_id]["source_url"],
                }
                for citing_id in value["citing"]
            ),
            key=lambda row: (row["date_filed"], row["opinion_id"]),
        )
        traction_rows.append(
            {
                "dissent_opinion_id": opinion_id,
                "case_name": physical[opinion_id]["case_name"],
                "date_filed": physical[opinion_id]["date_filed"],
                "source_url": physical[opinion_id]["source_url"],
                "later_citing_opinions_n": len(citing),
                "summed_citation_depth": value["depth_sum"],
                "max_citation_depth": value["max_depth"],
                "citing_opinions": citing,
            }
        )
    traction_rows.sort(
        key=lambda row: (-row["later_citing_opinions_n"], -row["summed_citation_depth"], row["dissent_opinion_id"])
    )

    report = {
        "format": "calyx-cuyahoga-dissent-intelligence-v1",
        "scope": "physical Cuyahoga production Base rows and accepted citation overlay",
        "inputs": {
            str(path): sha256(path)
            for path in (cx_path, spectral_path, opinion_judges_path, judges_path, graph_path, overlay_path)
        },
        "physical_opinions_n": len(physical),
        "opinion_type_counts": dict(sorted(type_counts.items())),
        "dissents_n": len(dissent_ids),
        "concurrence_family_n": len(concurrence_ids),
        "resolved_physical_n": len(physical) - unresolved_physical,
        "unresolved_physical_n": unresolved_physical,
        "resolved_opinion_type_counts": dict(sorted(resolved_type_counts.items())),
        "unresolved_opinion_type_counts": dict(sorted(unresolved_type_counts.items())),
        "per_judge": per_judge,
        "communities": communities,
        "citation_overlay": {
            "source_rows_n": skip["total_rows"],
            "duplicate_pairs_n": dup_pairs,
            "accepted_unique_edges_n": len(seen_pairs),
            "physical_graph_sst_rows_n": graph_rows,
            "latest_logical_graph_keys_n": len(latest_graph_rows),
            "historical_duplicate_graph_rows_n": graph_rows - len(latest_graph_rows),
            "physical_cites_edge_values_n": graph_edges,
            "edges_targeting_dissents_n": dissent_target_edges,
            "dissent_target_edges_missing_physical_citing_n": dissent_target_edges_missing_physical_citing,
            "dissent_target_edges_not_later_n": dissent_target_edges_not_later,
            "dissent_target_edges_later_n": dissent_target_edges_later,
            "dissents_with_later_citations_n": len(traction_rows),
            "dissents_without_later_citations_n": len(dissent_ids) - len(traction_rows),
        },
        "traction": traction_rows,
    }
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    temp = out_path.with_name(out_path.name + ".tmp")
    temp.write_text(encoded, encoding="utf-8")
    os.replace(temp, out_path)
    reread = json.loads(out_path.read_text(encoding="utf-8"))
    if reread != report:
        raise RuntimeError("published report readback differs")
    print(json.dumps({"status": "verified", "out": str(out_path), "sha256": sha256(out_path), "dissents": len(dissent_ids), "traction": len(traction_rows)}, sort_keys=True))


if __name__ == "__main__":
    main()
