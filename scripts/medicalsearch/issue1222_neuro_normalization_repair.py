#!/usr/bin/env python3
"""Focused #1222 neuro normalization repair.

Accepts only deterministic, source-backed neuro concept mappings. Ambiguous
phrases remain unresolved with explicit accounting. The script also emits full
repaired normalization inputs that can be used to rerun the #1187 hunt script.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter, defaultdict
from itertools import combinations
from pathlib import Path
from typing import Any


DEFAULT_SOURCE_EXPANSION = (
    "/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/"
    "complete_cxid_source_expansion.jsonl"
)
DEFAULT_FULL_NORMALIZED = (
    "/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z/"
    "normalized_concept_annotations.jsonl"
)
DEFAULT_FULL_UNRESOLVED = (
    "/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z/"
    "unresolved_or_ambiguous_concepts.jsonl"
)
DEFAULT_DOMAIN_NORMALIZED = (
    "/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z/out/"
    "neuro_normalized_annotations.jsonl"
)
DEFAULT_DOMAIN_UNRESOLVED = (
    "/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z/out/"
    "neuro_unresolved_terms.jsonl"
)

CLINICAL_BOUNDARY = (
    "Neuro normalization coverage repair only; not efficacy, safety, clinical actionability, "
    "treatment guidance, or cure evidence."
)

REQUIRED_TERMS = [
    "Parkinsonism",
    "Parkinson's disease",
    "Seizures",
    "Vascular dementia",
    "Ischemic stroke",
    "Optic glioma",
    "Spinocerebellar ataxia",
    "Multiple sclerosis",
    "Migraine",
    "Paranoid schizophrenia",
]


def mesh(
    key: str,
    name: str,
    mesh_id: str,
    synonyms: list[str],
    kind: str = "disease",
    lookup_label: str | None = None,
) -> dict[str, Any]:
    label = lookup_label or name
    prefix = "DISEASE" if kind == "disease" else "ANATOMY"
    return {
        "concept_type": kind,
        "normalized_id": "@" + prefix + "_" + re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_"),
        "normalized_name": name,
        "source_db": "ncbi_mesh",
        "source_db_id": mesh_id,
        "synonyms": synonyms,
        "match_strategy": "exact_mesh_descriptor_or_entry_term",
        "lookup_key": key,
        "lookup_url": (
            "https://id.nlm.nih.gov/mesh/lookup/descriptor?label="
            f"{label.replace(' ', '+')}&match=exact&limit=10"
        ),
        "confidence": 1.0,
    }


def mapping_rows() -> list[dict[str, Any]]:
    return [
        mesh("parkinsonian_disorders", "Parkinsonian Disorders", "D020734", ["Parkinsonism"]),
        mesh(
            "parkinson_disease",
            "Parkinson Disease",
            "D010300",
            ["Parkinson's disease", "Parkinson disease"],
        ),
        mesh("seizures", "Seizures", "D012640", ["Seizures", "Myoclonic seizures", "Myoclonic seizure"]),
        mesh("epilepsies_partial", "Epilepsies, Partial", "D004828", ["simple partial seizure", "complex partial seizures", "Simple paial seizure"], lookup_label="Epilepsies, Partial"),
        mesh("epilepsy_absence", "Epilepsy, Absence", "D004832", ["absence (petit mal) seizures", "Typical absence seizure"], lookup_label="Epilepsy, Absence"),
        mesh("epilepsy_tonic_clonic", "Epilepsy, Tonic-Clonic", "D004830", ["tonic-clonic (grand mal) seizures"], lookup_label="Epilepsy, Tonic-Clonic"),
        mesh("epilepsy", "Epilepsy", "D004827", ["Myoclonic epilepsy"]),
        mesh("dementia_vascular", "Dementia, Vascular", "D015140", ["Vascular dementia"], lookup_label="Dementia, Vascular"),
        mesh("dementia_multi_infarct", "Dementia, Multi-Infarct", "D015161", ["Multi infarct dementia", "Multi-infarct dementia"], lookup_label="Dementia, Multi-Infarct"),
        mesh("lewy_body_disease", "Lewy Body Disease", "D020961", ["Lewy body dementia"]),
        mesh("ischemic_stroke", "Ischemic Stroke", "D000083242", ["Ischemic stroke"]),
        mesh("stroke", "Stroke", "D020521", ["Stroke"]),
        mesh("cerebral_hemorrhage", "Cerebral Hemorrhage", "D002543", ["Intracerebral parenchymal hemorrhage", "Intracerebral hemorrhage"]),
        mesh("subarachnoid_hemorrhage", "Subarachnoid Hemorrhage", "D013345", ["Subarachnoid hematoma"]),
        mesh("glioma", "Glioma", "D005910", ["Glioma"]),
        mesh("optic_nerve_glioma", "Optic Nerve Glioma", "D020339", ["Optic glioma"]),
        mesh("meningioma", "Meningioma", "D008579", ["falx meningioma"]),
        mesh("spinocerebellar_ataxias", "Spinocerebellar Ataxias", "D020754", ["Spinocerebellar ataxia"]),
        mesh("ataxia", "Ataxia", "D001259", ["Ataxia"]),
        mesh("multiple_sclerosis", "Multiple Sclerosis", "D009103", ["Multiple sclerosis", "Multiple Sclerosis", "Secondry progressive multiple sclerosis", "Primary progresive multiple sclerosis"]),
        mesh("neurofibrillary_tangles", "Neurofibrillary Tangles", "D016874", ["Neuro fibrillary tangle", "Neuro fibrillory tangles", "Neurofibrillary tangle"]),
        mesh("neurotoxicity_syndromes", "Neurotoxicity Syndromes", "D020258", ["Neurotoxicity"]),
        mesh("neuroglia", "Neuroglia", "D009457", ["Neuroglial cells"], kind="anatomy"),
        mesh("korsakoff_syndrome", "Korsakoff Syndrome", "D020915", ["Korsakoff psychosis"]),
        mesh("ulnar_neuropathies", "Ulnar Neuropathies", "D020424", ["Ulnar neuropathies"]),
        mesh("peripheral_nervous_system_diseases", "Peripheral Nervous System Diseases", "D010523", ["Peripheral neuropathy"]),
        mesh("migraine_disorders", "Migraine Disorders", "D008881", ["Migraine"]),
        mesh("oligodendroglioma", "Oligodendroglioma", "D009837", ["Oligodendroglioma"]),
        mesh("obsessive_compulsive_disorder", "Obsessive-Compulsive Disorder", "D009771", ["Obsessive compulsive disorder"]),
        mesh("schizophrenia_paranoid", "Schizophrenia, Paranoid", "D012563", ["Paranoid schizophrenia"], lookup_label="Schizophrenia, Paranoid"),
        mesh("neurofibromatoses", "Neurofibromatoses", "D017253", ["Neurofibromatosis"]),
        mesh("neurofibroma", "Neurofibroma", "D009455", ["Neurofibromas"]),
        mesh("neurofibromatosis_1", "Neurofibromatosis 1", "D009456", ["Neurofibromatosis 1"]),
        mesh("neurocutaneous_syndromes", "Neurocutaneous Syndromes", "D020752", ["Neurocutaneous syndrome"]),
        mesh("tinnitus", "Tinnitus", "D014012", ["Pulsatile tinnitus"]),
        mesh("otosclerosis", "Otosclerosis", "D010040", ["Otosclerosis reduces compliance"]),
        mesh("optic_atrophy", "Optic Atrophy", "D009896", ["Optic neurosis"]),
        mesh("mental_status_and_dementia_tests", "Mental Status and Dementia Tests", "D000073216", ["30 point test to evaluate cognitive function"]),
    ]


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=False) + "\n")


def line_count(path: Path) -> int:
    with path.open(encoding="utf-8", errors="replace") as handle:
        return sum(1 for line in handle if line.strip())


def term_key(value: str) -> str:
    return re.sub(r"\s+", " ", value.strip().lower())


def pattern_for(text: str) -> re.Pattern[str]:
    return re.compile(rf"(?<![A-Za-z0-9]){re.escape(text)}(?![A-Za-z0-9])", re.IGNORECASE)


def source_text(row: dict[str, Any]) -> str:
    return str(row.get("source_row", {}).get("text") or "")


def source_meta(row: dict[str, Any]) -> dict[str, Any]:
    src = row.get("source_row", {})
    base = row.get("base_metadata", {})
    return {
        "cx_id": row.get("cx_id"),
        "source_dataset": src.get("dataset") or base.get("source_dataset"),
        "source_id": src.get("source_id") or base.get("source_id"),
        "source_sha256": src.get("source_sha256") or base.get("source_sha256"),
        "text_sha256": src.get("text_sha256"),
        "source_line": src.get("source_line"),
    }


def with_lookup_hashes(mappings: list[dict[str, Any]], raw_lookup_dir: Path | None) -> list[dict[str, Any]]:
    if raw_lookup_dir is None:
        return mappings
    lookup_files = {p.stem: p for p in raw_lookup_dir.rglob("*.json")}
    for mapping in mappings:
        key = str(mapping["lookup_key"])
        path = lookup_files.get(key)
        if path is not None:
            mapping["lookup_response_path"] = str(path)
            mapping["lookup_response_sha256"] = sha256_path(path)
    return mappings


def build_annotation(source_row: dict[str, Any], start: int, end: int, span: str, mapping: dict[str, Any], synonym: str) -> dict[str, Any]:
    meta = source_meta(source_row)
    return {
        "schema_version": 1,
        "status": "normalized",
        "evidence_kind": "cxid_source_row",
        "evidence_id": meta["cx_id"],
        "source_cx_id": meta["cx_id"],
        "source_dataset": meta["source_dataset"],
        "source_id": meta["source_id"],
        "source_sha256": meta["source_sha256"],
        "span_start": start,
        "span_end": end,
        "span_text": span,
        "scope": "exact-string-span",
        "concept_type": mapping["concept_type"],
        "normalized_id": mapping["normalized_id"],
        "normalized_name": mapping["normalized_name"],
        "source_db": mapping["source_db"],
        "source_db_id": mapping["source_db_id"],
        "confidence": mapping["confidence"],
        "normalizer": "issue1222_deterministic_mapping_table_v1",
        "normalizer_url": mapping["lookup_url"],
        "normalizer_response_sha256": mapping.get("lookup_response_sha256"),
        "match_strategy": mapping["match_strategy"],
        "matched_synonym": synonym,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def scan_repairs(source_rows: list[dict[str, Any]], mappings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    compiled = []
    for mapping in mappings:
        for synonym in mapping["synonyms"]:
            compiled.append((len(synonym), synonym, pattern_for(synonym), mapping))
    compiled.sort(key=lambda item: (-item[0], term_key(item[1])))
    annotations = []
    for row in source_rows:
        text = source_text(row)
        if not text:
            continue
        occupied: list[tuple[int, int]] = []
        for _, synonym, pattern, mapping in compiled:
            for match in pattern.finditer(text):
                span = match.span()
                if any(max(span[0], old[0]) < min(span[1], old[1]) for old in occupied):
                    continue
                occupied.append(span)
                annotations.append(build_annotation(row, span[0], span[1], match.group(0), mapping, synonym))
    annotations.sort(key=lambda row: (str(row.get("source_cx_id")), int(row.get("span_start") or 0), int(row.get("span_end") or 0), str(row.get("source_db")), str(row.get("source_db_id"))))
    return annotations


def dedupe_annotations(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen = set()
    out = []
    for row in rows:
        key = (row.get("source_cx_id"), row.get("span_start"), row.get("span_end"), row.get("source_db"), row.get("source_db_id"))
        if key in seen:
            continue
        seen.add(key)
        out.append(row)
    return out


def concept_key(row: dict[str, Any]) -> str:
    return "|".join(str(row.get(key) or "") for key in ["concept_type", "source_db", "source_db_id", "normalized_name"])


def overlay_delta(added: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    nodes = {}
    edges = []
    for row in added:
        concept_id = f"concept:{row['source_db']}:{row['source_db_id']}"
        nodes[concept_id] = {
            "schema_version": 1,
            "node_id": concept_id,
            "node_type": "concept",
            "concept_type": row["concept_type"],
            "normalized_name": row["normalized_name"],
            "source_db": row["source_db"],
            "source_db_id": row["source_db_id"],
        }
        cx_id = f"cx:{row['source_cx_id']}"
        nodes[cx_id] = {"schema_version": 1, "node_id": cx_id, "node_type": "source_cx"}
        edges.append(
            {
                "schema_version": 1,
                "edge_type": "mentions",
                "source": cx_id,
                "target": concept_id,
                "span_text": row["span_text"],
                "span_start": row["span_start"],
                "span_end": row["span_end"],
                "source_sha256": row["source_sha256"],
            }
        )
    return sorted(nodes.values(), key=lambda row: row["node_id"]), sorted(edges, key=lambda row: (row["source"], row["target"], row["span_start"]))


def co_mentions(existing: list[dict[str, Any]], added: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_cx: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in existing:
        by_cx[str(row.get("source_cx_id"))].append(row)
    pairs: dict[tuple[str, str], dict[str, Any]] = {}
    for row in added:
        repair_key = concept_key(row)
        cx = str(row.get("source_cx_id"))
        for other in by_cx.get(cx, []):
            other_key = concept_key(other)
            if other_key == repair_key:
                continue
            left, right = sorted([repair_key, other_key])
            ent = pairs.setdefault(
                (left, right),
                {"schema_version": 1, "left": left, "right": right, "support_cx_ids": set(), "example": {"cx_id": cx, "repair_span": row.get("span_text"), "repair_name": row.get("normalized_name"), "other_span": other.get("span_text"), "other_name": other.get("normalized_name")}},
            )
            ent["support_cx_ids"].add(cx)
    out = []
    for ent in pairs.values():
        support = sorted(ent.pop("support_cx_ids"))
        ent["support_count"] = len(support)
        ent["support_cx_ids"] = support[:20]
        out.append(ent)
    out.sort(key=lambda row: (-row["support_count"], row["left"], row["right"]))
    return out


def required_examples(annotations: list[dict[str, Any]]) -> dict[str, Any]:
    out = {}
    for required in REQUIRED_TERMS:
        needle = term_key(required)
        hits = [row for row in annotations if term_key(str(row.get("span_text") or "")) == needle or term_key(str(row.get("matched_synonym") or "")) == needle]
        out[required] = {
            "present": bool(hits),
            "count": len(hits),
            "examples": [
                {
                    "source_cx_id": hit.get("source_cx_id"),
                    "span_text": hit.get("span_text"),
                    "normalized_name": hit.get("normalized_name"),
                    "source_db": hit.get("source_db"),
                    "source_db_id": hit.get("source_db_id"),
                    "source_sha256": hit.get("source_sha256"),
                    "span_start": hit.get("span_start"),
                    "span_end": hit.get("span_end"),
                }
                for hit in hits[:5]
            ],
        }
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-expansion", default=DEFAULT_SOURCE_EXPANSION)
    parser.add_argument("--full-normalized", default=DEFAULT_FULL_NORMALIZED)
    parser.add_argument("--full-unresolved", default=DEFAULT_FULL_UNRESOLVED)
    parser.add_argument("--domain-normalized", default=DEFAULT_DOMAIN_NORMALIZED)
    parser.add_argument("--domain-unresolved", default=DEFAULT_DOMAIN_UNRESOLVED)
    parser.add_argument("--raw-lookup-dir")
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    source_path = Path(args.source_expansion)
    full_normalized_path = Path(args.full_normalized)
    full_unresolved_path = Path(args.full_unresolved)
    domain_normalized_path = Path(args.domain_normalized)
    domain_unresolved_path = Path(args.domain_unresolved)
    raw_lookup_dir = Path(args.raw_lookup_dir) if args.raw_lookup_dir else None

    source_rows = load_jsonl(source_path)
    full_normalized = load_jsonl(full_normalized_path)
    full_unresolved = load_jsonl(full_unresolved_path)
    domain_normalized = load_jsonl(domain_normalized_path)
    domain_unresolved = load_jsonl(domain_unresolved_path)
    mappings = with_lookup_hashes(mapping_rows(), raw_lookup_dir)
    mapped_terms = {term_key(synonym) for mapping in mappings for synonym in mapping["synonyms"]}

    added = scan_repairs(source_rows, mappings)
    domain_repaired = dedupe_annotations(domain_normalized + added)
    full_repaired = dedupe_annotations(full_normalized + added)
    remaining_domain = [row for row in domain_unresolved if term_key(str(row.get("term") or "")) not in mapped_terms]
    remaining_full = [row for row in full_unresolved if term_key(str(row.get("term") or "")) not in mapped_terms]
    resolved_domain = [row for row in domain_unresolved if term_key(str(row.get("term") or "")) in mapped_terms]
    nodes, edges = overlay_delta(added)
    coverage_pairs = co_mentions(domain_normalized, added)
    examples = required_examples(added)

    resolved_rows = []
    added_by_synonym = Counter(term_key(str(row.get("matched_synonym") or "")) for row in added)
    for row in resolved_domain:
        key = term_key(str(row.get("term") or ""))
        mapping = next(mapping for mapping in mappings if key in {term_key(synonym) for synonym in mapping["synonyms"]})
        resolved_rows.append(
            {
                "schema_version": 1,
                "term": row.get("term"),
                "before_reason": row.get("reason"),
                "before_occurrence_count": row.get("occurrence_count"),
                "added_annotation_rows": added_by_synonym[key],
                "normalized_name": mapping["normalized_name"],
                "source_db": mapping["source_db"],
                "source_db_id": mapping["source_db_id"],
                "match_strategy": mapping["match_strategy"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )

    input_scope = {
        "schema_version": 1,
        "issue": 1222,
        "inputs": {
            "source_expansion": {"path": str(source_path), "bytes": source_path.stat().st_size, "sha256": sha256_path(source_path)},
            "full_normalized": {"path": str(full_normalized_path), "bytes": full_normalized_path.stat().st_size, "sha256": sha256_path(full_normalized_path)},
            "full_unresolved": {"path": str(full_unresolved_path), "bytes": full_unresolved_path.stat().st_size, "sha256": sha256_path(full_unresolved_path)},
            "domain_normalized": {"path": str(domain_normalized_path), "bytes": domain_normalized_path.stat().st_size, "sha256": sha256_path(domain_normalized_path)},
            "domain_unresolved": {"path": str(domain_unresolved_path), "bytes": domain_unresolved_path.stat().st_size, "sha256": sha256_path(domain_unresolved_path)},
        },
        "raw_lookup_dir": str(raw_lookup_dir) if raw_lookup_dir else None,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    before_after = {
        "schema_version": 1,
        "status": "ok",
        "before": {
            "domain_normalized_annotation_rows": len(domain_normalized),
            "domain_unresolved_terms": len(domain_unresolved),
            "unique_domain_concepts": len({concept_key(row) for row in domain_normalized}),
            "domain_normalized_source_cx_ids": len({row.get("source_cx_id") for row in domain_normalized}),
        },
        "after": {
            "domain_normalized_annotation_rows": len(domain_repaired),
            "added_annotation_rows": len(added),
            "domain_unresolved_terms": len(remaining_domain),
            "resolved_domain_unresolved_terms": len(resolved_domain),
            "unique_domain_concepts": len({concept_key(row) for row in domain_repaired}),
            "domain_normalized_source_cx_ids": len({row.get("source_cx_id") for row in domain_repaired}),
            "new_co_mention_pair_rows": len(coverage_pairs),
            "overlay_delta_nodes": len(nodes),
            "overlay_delta_edges": len(edges),
        },
        "unresolved_reason_counts_after": dict(Counter(str(row.get("reason")) for row in remaining_domain)),
        "required_terms_present": {term: examples[term]["present"] for term in REQUIRED_TERMS},
        "clinical_boundary": CLINICAL_BOUNDARY,
    }

    write_json(out_dir / "input_scope.json", input_scope)
    write_json(out_dir / "deterministic_mapping_table.json", {"schema_version": 1, "mappings": mappings})
    write_json(out_dir / "before_after_coverage.json", before_after)
    write_json(out_dir / "required_term_examples.json", {"schema_version": 1, "examples": examples})
    write_jsonl(out_dir / "neuro_normalized_annotations_added.jsonl", added)
    write_jsonl(out_dir / "neuro_normalized_annotations.repaired.jsonl", domain_repaired)
    write_jsonl(out_dir / "neuro_unresolved_terms.remaining.jsonl", remaining_domain)
    write_jsonl(out_dir / "resolved_terms_from_unresolved.jsonl", resolved_rows)
    write_jsonl(out_dir / "full_normalized_concept_annotations.repaired.jsonl", full_repaired)
    write_jsonl(out_dir / "full_unresolved_or_ambiguous_concepts.repaired.jsonl", remaining_full)
    write_jsonl(out_dir / "typed_overlay_delta_nodes.jsonl", nodes)
    write_jsonl(out_dir / "typed_overlay_delta_edges.jsonl", edges)
    write_jsonl(out_dir / "new_co_mention_association_coverage.jsonl", coverage_pairs)

    artifacts = [
        "input_scope.json",
        "deterministic_mapping_table.json",
        "neuro_normalized_annotations_added.jsonl",
        "neuro_normalized_annotations.repaired.jsonl",
        "neuro_unresolved_terms.remaining.jsonl",
        "resolved_terms_from_unresolved.jsonl",
        "full_normalized_concept_annotations.repaired.jsonl",
        "full_unresolved_or_ambiguous_concepts.repaired.jsonl",
        "typed_overlay_delta_nodes.jsonl",
        "typed_overlay_delta_edges.jsonl",
        "new_co_mention_association_coverage.jsonl",
        "before_after_coverage.json",
        "required_term_examples.json",
    ]
    untyped_edges = [edge for edge in edges if edge.get("edge_type") != "mentions" or not str(edge.get("target", "")).startswith("concept:")]
    readback = {
        "schema_version": 1,
        "status": "ok",
        "root": str(out_dir),
        "artifacts": {name: {"bytes": (out_dir / name).stat().st_size, "sha256": sha256_path(out_dir / name)} for name in artifacts},
        "assertions": {
            "before_unresolved_terms": len(domain_unresolved),
            "after_unresolved_terms": len(remaining_domain),
            "unresolved_decreased": len(remaining_domain) < len(domain_unresolved),
            "accepted_term_rows": len(resolved_domain),
            "added_rows_readback": line_count(out_dir / "neuro_normalized_annotations_added.jsonl"),
            "added_rows_match_metrics": line_count(out_dir / "neuro_normalized_annotations_added.jsonl") == len(added),
            "repaired_rows_readback": line_count(out_dir / "neuro_normalized_annotations.repaired.jsonl"),
            "required_terms_all_present": all(examples[term]["present"] for term in REQUIRED_TERMS),
            "overlay_delta_edges": line_count(out_dir / "typed_overlay_delta_edges.jsonl"),
            "overlay_delta_untyped_edges": len(untyped_edges),
            "new_co_mention_pair_rows": line_count(out_dir / "new_co_mention_association_coverage.jsonl"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    write_json(out_dir / "persisted_readback.json", readback)
    print(json.dumps(readback, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
