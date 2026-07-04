#!/usr/bin/env python3
"""#1190 fail-closed drug-combination hypothesis miner.

This miner proposes reviewable drug-pair hypotheses from the human-review atlas,
then blocks promotion unless component safety, pair interaction, and external
synergy evidence are all present. It is not treatment guidance.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import re
import time
from collections import Counter, defaultdict
from itertools import combinations
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Drug-combination research triage only; not efficacy, safety, clinical "
    "actionability, treatment guidance, dosing, recommendation, or cure evidence."
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z"

DEFAULT_INPUTS = {
    "atlas": "/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T124751Z/out/human_review_biomedical_hypothesis_atlas.jsonl",
    "drug_safety_terms": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/drug_safety_terms.jsonl",
    "parsed_safety_rows": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/parsed_safety_rows.jsonl",
    "mapped_candidate_safety": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/mapped_candidate_safety.jsonl",
    "clinicaltrials_rows": "/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z/parsed/clinicaltrials_trial_rows.jsonl",
    "drugcomb_summary": "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/raw/summary_table_v1.4.csv",
}

DRUGCOMB_MD5 = "c11efbdcae4a860c2374c1505a66599b"

GENERIC_DRUG_TERMS = {
    "null",
    "none",
    "antibiotic",
    "antiviral agent",
    "anticonvulsant agent",
    "therapeutic corticosteroid",
    "steroids",
    "anti-ctla-4 monoclonal antibody",
}


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def md5_path(path: Path) -> str:
    h = hashlib.md5()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def stable_id(*parts: object, length: int = 24) -> str:
    payload = "\x1f".join(str(part) for part in parts)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:length]


def clean_text(value: object) -> str:
    if value is None:
        return ""
    return " ".join(str(value).replace("\x00", " ").split())


def norm_name(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def pair_key(left: object, right: object) -> str:
    a, b = sorted([norm_name(left), norm_name(right)])
    return f"{a}||{b}"


def uniq(values: list[object]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for value in values:
        text = clean_text(value)
        if not text:
            continue
        key = text.lower()
        if key not in seen:
            seen.add(key)
            out.append(text)
    return out


def rows_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=False) + "\n")


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    info: dict[str, Any] = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            info["rows"] = sum(1 for line in handle if line.strip())
    else:
        info["rows"] = None
    return info


def numeric(value: object) -> float | None:
    try:
        result = float(value)
    except (TypeError, ValueError):
        return None
    return result if math.isfinite(result) else None


def specific_drug(name: str) -> bool:
    n = norm_name(name)
    if not n or n in GENERIC_DRUG_TERMS:
        return False
    if n.startswith("chembl chembl") and len(n) > 24:
        return False
    return len(n) >= 3


def build_input_manifest(inputs: dict[str, str]) -> dict[str, Any]:
    rows: dict[str, Any] = {}
    for name, raw in inputs.items():
        path = Path(raw)
        entry = artifact(path, jsonl=path.suffix == ".jsonl")
        if name == "drugcomb_summary":
            entry["md5"] = md5_path(path)
            entry["expected_md5"] = DRUGCOMB_MD5
            entry["source_url"] = "https://zenodo.org/records/11102665/files/summary_table_v1.4.csv?download=1"
            entry["source_record"] = "https://zenodo.org/records/11102665"
        rows[name] = entry
    return {
        "schema_version": 1,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": rows,
        "external_source_notes": [
            {
                "name": "DrugComb summary table v1.4",
                "source": "Zenodo record 10.5281/zenodo.11102665",
                "role": "external preclinical synergy screen evidence; not clinical efficacy",
                "expected_md5": DRUGCOMB_MD5,
            },
            {
                "name": "NCI ALMANAC",
                "source": "https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html",
                "role": "identified source for future broad oncology combination ingest",
                "status": "researched, not downloaded in this #1190 bounded run",
            },
        ],
    }


def component_review_priority(row: dict[str, Any]) -> float:
    status_bonus = {
        "ready_for_hypothesis_review": 0.40,
        "calibration_known_positive_reference": 0.20,
        "blocked_or_demoted_before_human_review": -0.35,
    }.get(row.get("review_status"), -0.5)
    return (
        float(row.get("novelty_score") or 0.0)
        + float(row.get("confidence_score") or 0.0)
        + status_bonus
    )


def build_components(atlas_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    components: list[dict[str, Any]] = []
    for row in atlas_rows:
        disease_names = row.get("disease_names") or []
        if not disease_names:
            continue
        disease = clean_text(disease_names[0])
        for drug in row.get("drug_names") or []:
            drug_text = clean_text(drug)
            if not specific_drug(drug_text):
                continue
            components.append(
                {
                    "component_id": f"component:{stable_id(row['candidate_id'], drug_text)}",
                    "candidate_id": row["candidate_id"],
                    "atlas_rank": row.get("atlas_rank"),
                    "drug": drug_text,
                    "drug_norm": norm_name(drug_text),
                    "disease_area": row.get("disease_area"),
                    "disease": disease,
                    "disease_norm": norm_name(disease),
                    "target_names": row.get("target_names") or [],
                    "pathway_names": row.get("pathway_names") or [],
                    "review_status": row.get("review_status"),
                    "confidence_score": row.get("confidence_score"),
                    "novelty_score": row.get("novelty_score"),
                    "falsification_status": row.get("falsification_status"),
                    "falsification_reason_codes": row.get("falsification_reason_codes") or [],
                    "source_hashes": row.get("source_hashes") or [],
                    "source_path": row.get("source_path"),
                    "source_sha256": row.get("source_sha256"),
                    "source_text_snippets": row.get("source_text_snippets") or [],
                    "priority": component_review_priority(row),
                }
            )
    return components


def build_safety_index(
    safety_terms: list[dict[str, Any]],
    parsed_safety: list[dict[str, Any]],
) -> tuple[dict[str, dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    terms: dict[str, dict[str, Any]] = {}
    for row in safety_terms:
        drug = clean_text(row.get("drug_term"))
        if drug:
            terms[norm_name(drug)] = row
    sections: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in parsed_safety:
        drug = clean_text(row.get("drug_term"))
        if drug:
            sections[norm_name(drug)].append(row)
    return terms, sections


def interaction_section_summary(
    drug_norm: str,
    other_drug_norm: str,
    sections: dict[str, list[dict[str, Any]]],
) -> dict[str, Any]:
    rows = [row for row in sections.get(drug_norm, []) if row.get("section") == "drug_interactions"]
    mentions = []
    for row in rows:
        excerpt_norm = norm_name(row.get("text_excerpt"))
        if other_drug_norm and other_drug_norm in excerpt_norm:
            mentions.append(row)
    return {
        "component_has_drug_interaction_section": bool(rows),
        "pair_specific_label_mention": bool(mentions),
        "section_rows": len(rows),
        "mention_rows": len(mentions),
        "examples": [
            {
                "drug_term": row.get("drug_term"),
                "section": row.get("section"),
                "text_sha256": row.get("text_sha256"),
                "text_excerpt": clean_text(row.get("text_excerpt"))[:280],
            }
            for row in (mentions or rows)[:2]
        ],
    }


def trial_pair_evidence(
    drug_a: str,
    drug_b: str,
    disease: str,
    trials: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    na, nb, nd = norm_name(drug_a), norm_name(drug_b), norm_name(disease)
    hits: list[dict[str, Any]] = []
    for row in trials:
        interventions = " ".join(clean_text(x) for x in row.get("intervention_names") or [])
        conditions = " ".join(clean_text(x) for x in row.get("conditions") or [])
        blob_i = norm_name(interventions)
        blob_c = norm_name(conditions)
        if na in blob_i and nb in blob_i and (not nd or nd in blob_c):
            hits.append(
                {
                    "nct_id": row.get("nct_id"),
                    "overall_status": row.get("overall_status"),
                    "phases": row.get("phases"),
                    "has_results": row.get("has_results"),
                    "raw_sha256": row.get("raw_sha256"),
                    "brief_title": row.get("brief_title"),
                }
            )
    return hits[:5]


def make_component_groups(components: list[dict[str, Any]]) -> dict[tuple[str, str], list[dict[str, Any]]]:
    grouped: dict[tuple[str, str], dict[str, dict[str, Any]]] = defaultdict(dict)
    for component in components:
        key = (component["disease_area"], component["disease_norm"])
        current = grouped[key].get(component["drug_norm"])
        if current is None or component["priority"] > current["priority"]:
            grouped[key][component["drug_norm"]] = component
    trimmed: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for key, by_drug in grouped.items():
        values = sorted(by_drug.values(), key=lambda row: row["priority"], reverse=True)
        if len(values) >= 2:
            trimmed[key] = values[:12]
    return trimmed


def score_pair(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    targets_a = set(norm_name(x) for x in left.get("target_names") or [] if norm_name(x))
    targets_b = set(norm_name(x) for x in right.get("target_names") or [] if norm_name(x))
    pathways_a = set(norm_name(x) for x in left.get("pathway_names") or [] if norm_name(x))
    pathways_b = set(norm_name(x) for x in right.get("pathway_names") or [] if norm_name(x))
    target_overlap = sorted((targets_a & targets_b) - {""})
    pathway_overlap = sorted((pathways_a & pathways_b) - {""})
    complementary_targets = bool(targets_a and targets_b and not target_overlap)
    convergent_pathway = bool(pathway_overlap or target_overlap)
    mechanism_score = 0.0
    if complementary_targets:
        mechanism_score += 0.25
    if convergent_pathway:
        mechanism_score += 0.20
    if targets_a or targets_b:
        mechanism_score += 0.10
    review_score = (float(left.get("priority") or 0.0) + float(right.get("priority") or 0.0)) / 2.0
    return {
        "mechanism_score": round(max(0.0, mechanism_score), 6),
        "component_review_score": round(review_score, 6),
        "complementary_targets": complementary_targets,
        "convergent_pathway": convergent_pathway,
        "target_overlap": target_overlap,
        "pathway_overlap": pathway_overlap,
    }


def candidate_pairs(components: list[dict[str, Any]]) -> list[dict[str, Any]]:
    pairs: list[dict[str, Any]] = []
    grouped = make_component_groups(components)
    for (area, disease_norm), values in grouped.items():
        for left, right in combinations(values, 2):
            pair = {
                "pair_id": f"issue1190:{stable_id(area, disease_norm, left['drug_norm'], right['drug_norm'])}",
                "disease_area": area,
                "disease": left["disease"],
                "disease_norm": disease_norm,
                "drug_a": left["drug"],
                "drug_b": right["drug"],
                "drug_a_norm": left["drug_norm"],
                "drug_b_norm": right["drug_norm"],
                "component_a": left,
                "component_b": right,
                "pair_key": pair_key(left["drug"], right["drug"]),
            }
            pair.update(score_pair(left, right))
            pairs.append(pair)
    pairs.sort(
        key=lambda row: (
            -(row["mechanism_score"] + row["component_review_score"]),
            row["disease_area"],
            row["disease_norm"],
            row["drug_a_norm"],
            row["drug_b_norm"],
        )
    )
    return pairs[:5000]


def scan_drugcomb(path: Path, keys: set[str]) -> dict[str, dict[str, Any]]:
    found: dict[str, dict[str, Any]] = {}
    with path.open("r", encoding="utf-8", errors="replace", newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            key = pair_key(row.get("drug_row"), row.get("drug_col"))
            if key not in keys:
                continue
            entry = found.setdefault(
                key,
                {
                    "row_count": 0,
                    "cell_lines": set(),
                    "max_synergy_zip": None,
                    "max_synergy_bliss": None,
                    "max_synergy_loewe": None,
                    "max_synergy_hsa": None,
                    "max_css": None,
                    "examples": [],
                },
            )
            entry["row_count"] += 1
            if row.get("cell_line_name"):
                entry["cell_lines"].add(row["cell_line_name"])
            for field in ["synergy_zip", "synergy_bliss", "synergy_loewe", "synergy_hsa", "css"]:
                val = numeric(row.get(field))
                max_key = f"max_{field}"
                if val is not None and (entry[max_key] is None or val > entry[max_key]):
                    entry[max_key] = val
            if len(entry["examples"]) < 3:
                entry["examples"].append(
                    {
                        "block_id": row.get("block_id"),
                        "drug_row": row.get("drug_row"),
                        "drug_col": row.get("drug_col"),
                        "cell_line_name": row.get("cell_line_name"),
                        "synergy_zip": row.get("synergy_zip"),
                        "synergy_bliss": row.get("synergy_bliss"),
                        "synergy_loewe": row.get("synergy_loewe"),
                        "synergy_hsa": row.get("synergy_hsa"),
                        "css": row.get("css"),
                    }
                )
    for entry in found.values():
        entry["cell_line_count"] = len(entry["cell_lines"])
        entry["cell_lines"] = sorted(entry["cell_lines"])[:20]
        for key, value in list(entry.items()):
            if isinstance(value, float):
                entry[key] = round(value, 6)
    return found


def evaluate_pair(
    pair: dict[str, Any],
    safety_terms: dict[str, dict[str, Any]],
    safety_sections: dict[str, list[dict[str, Any]]],
    trials: list[dict[str, Any]],
    synergy: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any]]:
    a, b = pair["drug_a_norm"], pair["drug_b_norm"]
    safety_a = safety_terms.get(a)
    safety_b = safety_terms.get(b)
    inter_ab = interaction_section_summary(a, b, safety_sections)
    inter_ba = interaction_section_summary(b, a, safety_sections)
    trial_hits = trial_pair_evidence(pair["drug_a"], pair["drug_b"], pair["disease"], trials)
    synergy_hit = synergy.get(pair["pair_key"])
    reasons: list[str] = []
    if pair["component_a"]["review_status"] == "blocked_or_demoted_before_human_review" or pair["component_b"]["review_status"] == "blocked_or_demoted_before_human_review":
        reasons.append("component_blocked_or_demoted_before_combination")
    if not (safety_a and safety_b):
        reasons.append("component_safety_missing_fail_closed")
    exact_pair_interaction = inter_ab["pair_specific_label_mention"] or inter_ba["pair_specific_label_mention"] or bool(trial_hits)
    if not exact_pair_interaction:
        reasons.append("pair_interaction_evidence_missing_fail_closed")
    if not synergy_hit:
        reasons.append("external_synergy_evidence_missing_fail_closed")
    flags_a = set(safety_a.get("flags") or []) if safety_a else set()
    flags_b = set(safety_b.get("flags") or []) if safety_b else set()
    shared_flags = sorted(flags_a & flags_b)
    if shared_flags:
        reasons.append("overlapping_component_safety_flags_review_required")
    if not (pair["complementary_targets"] or pair["convergent_pathway"]):
        reasons.append("weak_mechanism_pairing_review_required")
    if reasons:
        status = "blocked_missing_safety_interaction_or_synergy"
    else:
        status = "reviewable_preclinical_synergy_not_clinical_claim"
    evidence_score = pair["mechanism_score"] + max(0.0, pair["component_review_score"])
    if synergy_hit:
        evidence_score += 0.35
    if exact_pair_interaction:
        evidence_score += 0.20
    if safety_a and safety_b:
        evidence_score += 0.15
    hypothesis = {
        "schema_version": 1,
        "pair_id": pair["pair_id"],
        "clinical_boundary": CLINICAL_BOUNDARY,
        "derived_status": "combination_research_hypothesis_only",
        "combination_status": status,
        "falsification_status": status,
        "reason_codes": uniq(reasons),
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "disease_area": pair["disease_area"],
        "disease": pair["disease"],
        "component_candidate_ids": [
            pair["component_a"]["candidate_id"],
            pair["component_b"]["candidate_id"],
        ],
        "component_review_statuses": [
            pair["component_a"]["review_status"],
            pair["component_b"]["review_status"],
        ],
        "target_rationale": {
            "drug_a_targets": pair["component_a"].get("target_names") or [],
            "drug_b_targets": pair["component_b"].get("target_names") or [],
            "complementary_targets": pair["complementary_targets"],
            "convergent_pathway": pair["convergent_pathway"],
            "target_overlap": pair["target_overlap"],
            "pathway_overlap": pair["pathway_overlap"],
            "mechanism_score": pair["mechanism_score"],
        },
        "safety_evidence": {
            "drug_a_safety_available": bool(safety_a),
            "drug_b_safety_available": bool(safety_b),
            "drug_a_flags": sorted(flags_a),
            "drug_b_flags": sorted(flags_b),
            "shared_safety_flags": shared_flags,
            "drug_a_faers_total_reports": safety_a.get("faers_total_reports") if safety_a else None,
            "drug_b_faers_total_reports": safety_b.get("faers_total_reports") if safety_b else None,
        },
        "interaction_evidence": {
            "exact_pair_interaction_evidence": exact_pair_interaction,
            "drug_a_label_interaction": inter_ab,
            "drug_b_label_interaction": inter_ba,
            "clinicaltrials_pair_hits": trial_hits,
        },
        "synergy_evidence": {
            "source": "DrugComb summary_table_v1.4.csv",
            "matched": bool(synergy_hit),
            "summary": synergy_hit,
            "role": "preclinical/cell-line synergy evidence only, not clinical efficacy",
        },
        "score_components": {
            "mechanism_score": pair["mechanism_score"],
            "component_review_score": pair["component_review_score"],
            "safety_available_bonus": 0.15 if safety_a and safety_b else 0.0,
            "pair_interaction_bonus": 0.20 if exact_pair_interaction else 0.0,
            "synergy_bonus": 0.35 if synergy_hit else 0.0,
        },
        "rank_score": round(evidence_score, 6),
        "next_validation_experiment": next_validation(status, reasons, bool(synergy_hit), exact_pair_interaction),
    }
    flag = {
        "schema_version": 1,
        "pair_id": pair["pair_id"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "disease": pair["disease"],
        "blocked": status.startswith("blocked"),
        "combination_status": status,
        "reason_codes": hypothesis["reason_codes"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    return hypothesis, flag


def next_validation(status: str, reasons: list[str], has_synergy: bool, has_interaction: bool) -> str:
    if status.startswith("blocked"):
        if "external_synergy_evidence_missing_fail_closed" in reasons:
            return "Acquire exact external synergy/model evidence for this drug pair before promotion."
        if "pair_interaction_evidence_missing_fail_closed" in reasons:
            return "Acquire exact pair interaction evidence from label/trial/pharmacology sources before promotion."
        if "component_safety_missing_fail_closed" in reasons:
            return "Complete component safety triage for both drugs before promotion."
        return "Resolve all block reason codes before any human-review promotion."
    if has_synergy and has_interaction:
        return "Human reviewer should inspect preclinical synergy and interaction evidence, then define outcome and safety gates."
    return "Run additional safety, interaction, and outcome validation before promotion."


def build_bridge_rows(rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows[:1000]:
        terms = uniq([
            row["drug_a"],
            row["drug_b"],
            row["disease"],
            row["combination_status"],
            row["disease_area"],
        ])
        text = (
            f"Drug combination hypothesis {row['pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} for {row['disease']} in {row['disease_area']} has "
            f"status {row['combination_status']}."
        )
        out.append(
            {
                "id": row["pair_id"],
                "domain": "drug_combination_hypothesis",
                "text": text,
                "bridge_terms": [term for term in terms if term and term in text],
                "metadata": {
                    "source_dataset": "issue1190_drug_combination_hypotheses",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "rank_score": str(row["rank_score"]),
                    "combination_status": row["combination_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def build_miner(root: Path, inputs: dict[str, str]) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    input_manifest = build_input_manifest(inputs)
    write_json(out_dir / "input_manifest.json", input_manifest)
    if input_manifest["inputs"]["drugcomb_summary"]["md5"] != DRUGCOMB_MD5:
        raise SystemExit("DrugComb md5 mismatch; refusing to mine combinations")

    atlas_rows = rows_jsonl(Path(inputs["atlas"]))
    components = build_components(atlas_rows)
    write_jsonl(out_dir / "combination_candidate_inputs.jsonl", components)
    safety_terms, safety_sections = build_safety_index(
        rows_jsonl(Path(inputs["drug_safety_terms"])),
        rows_jsonl(Path(inputs["parsed_safety_rows"])),
    )
    safety_index_rows = [
        {
            "drug_norm": key,
            "drug_term": row.get("drug_term"),
            "flags": row.get("flags"),
            "faers_total_reports": row.get("faers_total_reports"),
            "label_available": row.get("label_available"),
            "clinical_boundary": "Component safety triage only; not a safety clearance.",
        }
        for key, row in sorted(safety_terms.items())
    ]
    write_jsonl(out_dir / "drug_component_safety_index.jsonl", safety_index_rows)
    pairs = candidate_pairs(components)
    write_jsonl(out_dir / "candidate_pair_inputs.jsonl", pairs)
    synergy = scan_drugcomb(Path(inputs["drugcomb_summary"]), {row["pair_key"] for row in pairs})
    synergy_rows = [
        {"pair_key": key, **value, "clinical_boundary": "Preclinical synergy evidence only; not clinical efficacy."}
        for key, value in sorted(synergy.items())
    ]
    write_jsonl(out_dir / "drugcomb_pair_matches.jsonl", synergy_rows)
    trials = rows_jsonl(Path(inputs["clinicaltrials_rows"]))
    hypotheses: list[dict[str, Any]] = []
    flags: list[dict[str, Any]] = []
    for pair in pairs:
        hypothesis, flag = evaluate_pair(pair, safety_terms, safety_sections, trials, synergy)
        hypotheses.append(hypothesis)
        flags.append(flag)
    hypotheses.sort(key=lambda row: (-row["rank_score"], row["pair_id"]))
    for idx, row in enumerate(hypotheses, start=1):
        row["rank"] = idx
    write_jsonl(out_dir / "drug_combination_hypotheses.jsonl", hypotheses)
    write_jsonl(out_dir / "combination_safety_interaction_flags.jsonl", flags)
    blocked = [row for row in hypotheses if row["combination_status"].startswith("blocked")]
    write_jsonl(out_dir / "blocked_combination_rows.jsonl", blocked)
    top_review = {
        "schema_version": 1,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "rows": hypotheses[:50],
    }
    write_json(out_dir / "top_combination_review_queue.json", top_review)
    source_path = out_dir / "drug_combination_hypotheses.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(hypotheses, source_path, source_sha)
    write_jsonl(out_dir / "combination_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(components, pairs, synergy, hypotheses, flags, safety_index_rows)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1190,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "combination_candidate_inputs": artifact(out_dir / "combination_candidate_inputs.jsonl", jsonl=True),
            "drug_component_safety_index": artifact(out_dir / "drug_component_safety_index.jsonl", jsonl=True),
            "candidate_pair_inputs": artifact(out_dir / "candidate_pair_inputs.jsonl", jsonl=True),
            "drugcomb_pair_matches": artifact(out_dir / "drugcomb_pair_matches.jsonl", jsonl=True),
            "drug_combination_hypotheses": artifact(out_dir / "drug_combination_hypotheses.jsonl", jsonl=True),
            "combination_safety_interaction_flags": artifact(out_dir / "combination_safety_interaction_flags.jsonl", jsonl=True),
            "blocked_combination_rows": artifact(out_dir / "blocked_combination_rows.jsonl", jsonl=True),
            "top_combination_review_queue": artifact(out_dir / "top_combination_review_queue.json"),
            "combination_bridge_rows": artifact(out_dir / "combination_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, components, pairs, hypotheses, flags, synergy)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "hypotheses": output_manifest["artifacts"]["drug_combination_hypotheses"],
            "flags": output_manifest["artifacts"]["combination_safety_interaction_flags"],
            "bridge_rows": output_manifest["artifacts"]["combination_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def build_metrics(
    components: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    synergy: dict[str, dict[str, Any]],
    hypotheses: list[dict[str, Any]],
    flags: list[dict[str, Any]],
    safety_index_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    status_counts = Counter(row["combination_status"] for row in hypotheses)
    reason_counts = Counter(reason for row in hypotheses for reason in row["reason_codes"])
    area_counts = Counter(row["disease_area"] for row in hypotheses)
    return {
        "schema_version": 1,
        "status": "ok",
        "component_input_rows": len(components),
        "candidate_pair_rows": len(pairs),
        "drugcomb_matched_pair_rows": len(synergy),
        "combination_hypothesis_rows": len(hypotheses),
        "safety_interaction_flag_rows": len(flags),
        "blocked_combination_rows": sum(1 for row in hypotheses if row["combination_status"].startswith("blocked")),
        "reviewable_preclinical_rows": status_counts["reviewable_preclinical_synergy_not_clinical_claim"],
        "component_safety_index_rows": len(safety_index_rows),
        "status_counts": dict(status_counts),
        "reason_code_counts": dict(reason_counts),
        "disease_area_counts": dict(area_counts),
        "rows_with_drugcomb_match": sum(1 for row in hypotheses if row["synergy_evidence"]["matched"]),
        "rows_with_exact_pair_interaction": sum(1 for row in hypotheses if row["interaction_evidence"]["exact_pair_interaction_evidence"]),
        "rows_with_component_safety_available": sum(
            1
            for row in hypotheses
            if row["safety_evidence"]["drug_a_safety_available"] and row["safety_evidence"]["drug_b_safety_available"]
        ),
        "clinical_boundary_rows": sum(1 for row in hypotheses if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_rows": [
            {
                "rank": row["rank"],
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "disease": row["disease"],
                "status": row["combination_status"],
                "rank_score": row["rank_score"],
                "reason_codes": row["reason_codes"],
            }
            for row in hypotheses[:10]
        ],
    }


def build_readback(
    out_dir: Path,
    components: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    hypotheses: list[dict[str, Any]],
    flags: list[dict[str, Any]],
    synergy: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "combination_candidate_inputs": artifact(out_dir / "combination_candidate_inputs.jsonl", jsonl=True),
        "candidate_pair_inputs": artifact(out_dir / "candidate_pair_inputs.jsonl", jsonl=True),
        "drugcomb_pair_matches": artifact(out_dir / "drugcomb_pair_matches.jsonl", jsonl=True),
        "drug_combination_hypotheses": artifact(out_dir / "drug_combination_hypotheses.jsonl", jsonl=True),
        "combination_safety_interaction_flags": artifact(out_dir / "combination_safety_interaction_flags.jsonl", jsonl=True),
        "blocked_combination_rows": artifact(out_dir / "blocked_combination_rows.jsonl", jsonl=True),
        "combination_bridge_rows": artifact(out_dir / "combination_bridge_rows.jsonl", jsonl=True),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
    }
    return {
        "schema_version": 1,
        "issue": 1190,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "component_inputs_present": len(components) > 0,
            "candidate_pairs_present": len(pairs) > 0,
            "hypothesis_rows_match_pairs": len(hypotheses) == len(pairs),
            "flag_rows_match_hypotheses": len(flags) == len(hypotheses),
            "drugcomb_source_read": artifacts["drugcomb_pair_matches"]["rows"] == len(synergy),
            "all_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in hypotheses),
            "blocked_rows_present": any(row["combination_status"].startswith("blocked") for row in hypotheses),
            "bridge_rows_1000_or_less": artifacts["combination_bridge_rows"]["rows"] == min(1000, len(hypotheses)),
            "no_promoted_clinical_rows": all("clinical_claim" not in row["combination_status"] for row in hypotheses),
        },
        "row_counts": {
            "component_inputs": len(components),
            "candidate_pairs": len(pairs),
            "hypotheses": len(hypotheses),
            "flags": len(flags),
            "drugcomb_matches": len(synergy),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    result = build_miner(Path(args.root), DEFAULT_INPUTS)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
