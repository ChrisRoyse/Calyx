#!/usr/bin/env python3
"""Focused #1225 infectious/immunology normalization repair.

This script is intentionally conservative: it resolves exact, unambiguous
terms from the #1188 unresolved domain slice and leaves narrative or ambiguous
phrases unresolved. Outputs are JSON/JSONL artifacts with physical readback
hashes so the repair can be audited without trusting stdout.
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
DEFAULT_DOMAIN_NORMALIZED = (
    "/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/"
    "infectious_immunology_normalized_annotations.jsonl"
)
DEFAULT_DOMAIN_UNRESOLVED = (
    "/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/"
    "infectious_immunology_unresolved_terms.jsonl"
)

CLINICAL_BOUNDARY = (
    "Normalization coverage repair only; not efficacy, safety, clinical actionability, "
    "treatment guidance, or cure evidence."
)

REQUIRED_TERMS = [
    "HLA-B27",
    "bronchial asthma",
    "acute asthma",
    "Tuberculosis",
    "CD40",
    "Sepsis",
    "Leukotriene",
    "Malaria",
    "Influenza vaccine",
]


def mapping_rows() -> list[dict[str, Any]]:
    return [
        mesh("asthma", "Asthma", "D001249", ["asthma", "bronchial asthma", "acute asthma"]),
        mesh("tuberculosis", "Tuberculosis", "D014376", ["Tuberculosis", "Myco tuberculosis"]),
        mesh("sepsis", "Sepsis", "D018805", ["Sepsis", "Septicemia"]),
        mesh("malaria", "Malaria", "D008288", ["Malaria"]),
        mesh("influenza_vaccines", "Influenza Vaccines", "D007252", ["Influenza vaccine"]),
        mesh("lupus_vulgaris", "Lupus Vulgaris", "D008177", ["Lupus vulgaris"]),
        mesh(
            "lupus_erythematosus_systemic",
            "Lupus Erythematosus, Systemic",
            "D008180",
            ["Systemic lupus erythematosus"],
            lookup_label="Lupus Erythematosus, Systemic",
        ),
        mesh(
            "arthritis_rheumatoid",
            "Arthritis, Rheumatoid",
            "D001172",
            ["Rheumatoid arthritis"],
            lookup_label="Arthritis, Rheumatoid",
        ),
        mesh("yellow_fever", "Yellow Fever", "D015004", ["yellow fever"]),
        chemical("leukotrienes", "Leukotrienes", "D015289", ["Leukotriene"]),
        chemical(
            "leukotriene_antagonists",
            "Leukotriene Antagonists",
            "D020024",
            ["Leukotriene antagonist", "Leukotriene antagonists", "Leukotriene receptor antagonist"],
        ),
        gene(
            "HLA-B",
            "3106",
            ["HLA-B27"],
            "allele_group_to_gene_locus",
            "hgnc_fetch:HLA-B",
            "https://rest.genenames.org/fetch/symbol/HLA-B",
        ),
        gene(
            "CD40",
            "958",
            ["CD40"],
            "exact_gene_symbol",
            "ncbi_gene_esearch:CD40",
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=gene&term=CD40%5Bsym%5D+AND+Homo+sapiens%5Borgn%5D&retmode=json&retmax=5",
        ),
        gene(
            "CD40LG",
            "959",
            ["CD40L"],
            "exact_gene_symbol",
            "ncbi_gene_esearch:CD40LG",
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=gene&term=CD40LG%5Bsym%5D+AND+Homo+sapiens%5Borgn%5D&retmode=json&retmax=5",
        ),
        gene(
            "IL2",
            "3558",
            ["Interleukin-2"],
            "exact_gene_symbol_synonym",
            "ncbi_gene_esearch:IL2",
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=gene&term=IL2%5Bsym%5D+AND+Homo+sapiens%5Borgn%5D&retmode=json&retmax=5",
        ),
    ]


def mesh(
    key: str,
    name: str,
    mesh_id: str,
    synonyms: list[str],
    lookup_label: str | None = None,
) -> dict[str, Any]:
    label = lookup_label or key.replace("_", " ")
    return {
        "concept_type": "disease",
        "normalized_id": "@" + "DISEASE_" + re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_"),
        "normalized_name": name,
        "source_db": "ncbi_mesh",
        "source_db_id": mesh_id,
        "synonyms": synonyms,
        "match_strategy": "exact_mesh_descriptor_or_entry_term",
        "lookup_key": f"nlm_mesh_lookup:{key}",
        "lookup_url": (
            "https://id.nlm.nih.gov/mesh/lookup/descriptor?label="
            f"{label.replace(' ', '+')}&match=exact&limit=10"
        ),
        "confidence": 1.0,
    }


def chemical(key: str, name: str, mesh_id: str, synonyms: list[str]) -> dict[str, Any]:
    row = mesh(key, name, mesh_id, synonyms)
    row["concept_type"] = "chemical"
    row["normalized_id"] = "@" + "CHEMICAL_" + re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_")
    return row


def gene(
    name: str,
    ncbi_id: str,
    synonyms: list[str],
    strategy: str,
    lookup_key: str,
    lookup_url: str,
) -> dict[str, Any]:
    return {
        "concept_type": "gene",
        "normalized_id": "@" + "GENE_" + re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_"),
        "normalized_name": name,
        "source_db": "ncbi_gene",
        "source_db_id": ncbi_id,
        "synonyms": synonyms,
        "match_strategy": strategy,
        "lookup_key": lookup_key,
        "lookup_url": lookup_url,
        "confidence": 0.95 if strategy == "allele_group_to_gene_locus" else 1.0,
    }


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
    escaped = re.escape(text)
    return re.compile(rf"(?<![A-Za-z0-9]){escaped}(?![A-Za-z0-9])", re.IGNORECASE)


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
        key = str(mapping["lookup_key"]).split(":", 1)[1].lower().replace("-", "_")
        if key == "hla_b":
            key = "hla_b_hgnc_fetch"
        path = lookup_files.get(key) or lookup_files.get(f"{key}_esearch")
        if path is not None:
            mapping["lookup_response_path"] = str(path)
            mapping["lookup_response_sha256"] = sha256_path(path)
    return mappings


def build_annotation(
    source_row: dict[str, Any],
    start: int,
    end: int,
    span: str,
    mapping: dict[str, Any],
    synonym: str,
) -> dict[str, Any]:
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
        "normalizer": "issue1225_deterministic_mapping_table_v1",
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
    annotations.sort(
        key=lambda row: (
            str(row.get("source_cx_id")),
            int(row.get("span_start") or 0),
            int(row.get("span_end") or 0),
            str(row.get("source_db")),
            str(row.get("source_db_id")),
        )
    )
    return annotations


def dedupe_annotations(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen = set()
    out = []
    for row in rows:
        key = (
            row.get("source_cx_id"),
            row.get("span_start"),
            row.get("span_end"),
            row.get("source_db"),
            row.get("source_db_id"),
        )
        if key in seen:
            continue
        seen.add(key)
        out.append(row)
    return out


def concept_key(row: dict[str, Any]) -> str:
    return "|".join(
        str(row.get(key) or "")
        for key in ["concept_type", "source_db", "source_db_id", "normalized_name"]
    )


def co_mentions(existing: list[dict[str, Any]], added: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_cx: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in existing:
        by_cx[str(row.get("source_cx_id"))].append(row)
    pairs: dict[tuple[str, str], dict[str, Any]] = {}
    for row in added:
        cx = str(row.get("source_cx_id"))
        repair_key = concept_key(row)
        for other in by_cx.get(cx, []):
            other_key = concept_key(other)
            if other_key == repair_key:
                continue
            left, right = sorted([repair_key, other_key])
            ent = pairs.setdefault(
                (left, right),
                {
                    "schema_version": 1,
                    "left": left,
                    "right": right,
                    "support_cx_ids": set(),
                    "example": {
                        "cx_id": cx,
                        "repair_span": row.get("span_text"),
                        "repair_name": row.get("normalized_name"),
                        "other_span": other.get("span_text"),
                        "other_name": other.get("normalized_name"),
                    },
                },
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
    examples = {}
    for required in REQUIRED_TERMS:
        needle = term_key(required)
        hits = [
            row
            for row in annotations
            if term_key(str(row.get("span_text") or "")) == needle
            or term_key(str(row.get("matched_synonym") or "")) == needle
        ]
        examples[required] = {
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
    return examples


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-expansion", default=DEFAULT_SOURCE_EXPANSION)
    parser.add_argument("--domain-normalized", default=DEFAULT_DOMAIN_NORMALIZED)
    parser.add_argument("--domain-unresolved", default=DEFAULT_DOMAIN_UNRESOLVED)
    parser.add_argument("--raw-lookup-dir")
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    source_path = Path(args.source_expansion)
    normalized_path = Path(args.domain_normalized)
    unresolved_path = Path(args.domain_unresolved)
    raw_lookup_dir = Path(args.raw_lookup_dir) if args.raw_lookup_dir else None

    source_rows = load_jsonl(source_path)
    existing = load_jsonl(normalized_path)
    unresolved = load_jsonl(unresolved_path)
    mappings = with_lookup_hashes(mapping_rows(), raw_lookup_dir)
    mapped_terms = {term_key(synonym) for mapping in mappings for synonym in mapping["synonyms"]}

    added = scan_repairs(source_rows, mappings)
    repaired = dedupe_annotations(existing + added)
    remaining_unresolved = [row for row in unresolved if term_key(str(row.get("term") or "")) not in mapped_terms]
    resolved_unresolved = [row for row in unresolved if term_key(str(row.get("term") or "")) in mapped_terms]
    coverage_pairs = co_mentions(existing, added)
    examples = required_examples(added)

    input_scope = {
        "schema_version": 1,
        "issue": 1225,
        "inputs": {
            "source_expansion": {"path": str(source_path), "bytes": source_path.stat().st_size, "sha256": sha256_path(source_path)},
            "domain_normalized": {"path": str(normalized_path), "bytes": normalized_path.stat().st_size, "sha256": sha256_path(normalized_path)},
            "domain_unresolved": {"path": str(unresolved_path), "bytes": unresolved_path.stat().st_size, "sha256": sha256_path(unresolved_path)},
        },
        "raw_lookup_dir": str(raw_lookup_dir) if raw_lookup_dir else None,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }

    resolved_rows = []
    added_by_synonym = Counter(term_key(str(row.get("matched_synonym") or "")) for row in added)
    for row in resolved_unresolved:
        key = term_key(str(row.get("term") or ""))
        mapping = next(
            mapping
            for mapping in mappings
            if key in {term_key(synonym) for synonym in mapping["synonyms"]}
        )
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

    before_after = {
        "schema_version": 1,
        "status": "ok",
        "before": {
            "normalized_annotation_rows": len(existing),
            "unresolved_terms": len(unresolved),
            "unique_normalized_concepts": len({concept_key(row) for row in existing}),
            "normalized_source_cx_ids": len({row.get("source_cx_id") for row in existing}),
        },
        "after": {
            "normalized_annotation_rows": len(repaired),
            "added_annotation_rows": len(added),
            "unresolved_terms": len(remaining_unresolved),
            "resolved_unresolved_terms": len(resolved_unresolved),
            "unique_normalized_concepts": len({concept_key(row) for row in repaired}),
            "normalized_source_cx_ids": len({row.get("source_cx_id") for row in repaired}),
            "new_co_mention_pair_rows": len(coverage_pairs),
        },
        "unresolved_reason_counts_after": dict(Counter(str(row.get("reason")) for row in remaining_unresolved)),
        "required_terms_present": {term: examples[term]["present"] for term in REQUIRED_TERMS},
        "clinical_boundary": CLINICAL_BOUNDARY,
    }

    artifact_rows = {
        "input_scope.json": input_scope,
        "deterministic_mapping_table.json": {"schema_version": 1, "mappings": mappings},
        "before_after_coverage.json": before_after,
        "required_term_examples.json": {"schema_version": 1, "examples": examples},
    }
    for name, value in artifact_rows.items():
        write_json(out_dir / name, value)
    write_jsonl(out_dir / "repaired_normalized_annotations_added.jsonl", added)
    write_jsonl(out_dir / "infectious_immunology_normalized_annotations.repaired.jsonl", repaired)
    write_jsonl(out_dir / "infectious_immunology_unresolved_terms.remaining.jsonl", remaining_unresolved)
    write_jsonl(out_dir / "resolved_terms_from_unresolved.jsonl", resolved_rows)
    write_jsonl(out_dir / "new_co_mention_association_coverage.jsonl", coverage_pairs)

    artifact_paths = [
        out_dir / "input_scope.json",
        out_dir / "deterministic_mapping_table.json",
        out_dir / "repaired_normalized_annotations_added.jsonl",
        out_dir / "infectious_immunology_normalized_annotations.repaired.jsonl",
        out_dir / "infectious_immunology_unresolved_terms.remaining.jsonl",
        out_dir / "resolved_terms_from_unresolved.jsonl",
        out_dir / "new_co_mention_association_coverage.jsonl",
        out_dir / "before_after_coverage.json",
        out_dir / "required_term_examples.json",
    ]
    readback = {
        "schema_version": 1,
        "status": "ok",
        "root": str(out_dir),
        "artifacts": {
            path.name: {"bytes": path.stat().st_size, "sha256": sha256_path(path)}
            for path in artifact_paths
        },
        "assertions": {
            "before_unresolved_terms": len(unresolved),
            "after_unresolved_terms": len(remaining_unresolved),
            "unresolved_decreased": len(remaining_unresolved) < len(unresolved),
            "added_rows_readback": line_count(out_dir / "repaired_normalized_annotations_added.jsonl"),
            "added_rows_match_metrics": line_count(out_dir / "repaired_normalized_annotations_added.jsonl") == len(added),
            "repaired_rows_readback": line_count(out_dir / "infectious_immunology_normalized_annotations.repaired.jsonl"),
            "required_terms_all_present": all(examples[term]["present"] for term in REQUIRED_TERMS),
            "new_co_mention_pair_rows": line_count(out_dir / "new_co_mention_association_coverage.jsonl"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    write_json(out_dir / "persisted_readback.json", readback)
    print(json.dumps(readback, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
