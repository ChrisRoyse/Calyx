#!/usr/bin/env python3
"""#1193 human-review biomedical hypothesis atlas builder.

The atlas is an inspectable research-review surface over persisted discovery
artifacts. It does not assert efficacy, safety, clinical actionability,
treatment guidance, recommendation, dosing, or cure evidence.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Human-review research atlas only; hypothesis-only rows, not efficacy, "
    "safety, clinical actionability, treatment guidance, dosing, "
    "recommendation, or cure evidence."
)

DEFAULT_INPUTS = {
    "1185_oncology": "/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl",
    "1186_metabolic_cardiovascular": "/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/out/metabolic_cardiovascular_hypotheses.jsonl",
    "1187_neuro_repaired": "/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187/out/neuro_hypotheses.jsonl",
    "1188_infectious_immunology": "/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_hypotheses.jsonl",
    "1189_rare_disease": "/home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z/out/rare_disease_hypotheses.jsonl",
}

DEFAULT_OVERLAYS = {
    "falsification_flags": "/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/candidate_falsification_flags.jsonl",
    "support_evidence": "/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/support_evidence.jsonl",
    "counter_evidence": "/home/croyse/calyx/fsv/issue1223-generated-candidate-falsification-20260704T121310Z/out/counter_evidence.jsonl",
    "novelty_combined": "/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z/out/combined_original_ranked.jsonl",
    "novelty_calibration": "/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z/out/calibration_known_positive_rows.jsonl",
    "novelty_research_leads": "/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z/out/novelty_prioritized_research_leads.jsonl",
    "safety_flags": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/candidate_safety_flags.jsonl",
    "mapped_safety": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/mapped_candidate_safety.jsonl",
    "clinicaltrials_rows": "/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z/parsed/clinicaltrials_trial_rows.jsonl",
    "open_targets_rows": "/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_association_rows.jsonl",
}


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def stable_id(*parts: object, length: int = 24) -> str:
    payload = "\x1f".join(str(part) for part in parts)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:length]


def clamp(value: float, low: float = 0.0, high: float = 1.0) -> float:
    if not math.isfinite(value):
        return low
    return max(low, min(high, value))


def clean_text(value: object) -> str:
    if value is None:
        return ""
    return " ".join(str(value).replace("\x00", " ").split())


def uniq(values: list[object]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for value in values:
        text = clean_text(value)
        if not text or text.lower() in seen:
            continue
        seen.add(text.lower())
        out.append(text)
    return out


def rows_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.exists():
        return rows
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


def manifest_entry(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    return artifact(path, jsonl=jsonl)


def row_id(row: dict[str, Any]) -> str:
    return clean_text(
        row.get("candidate_id")
        or row.get("hypothesis_id")
        or row.get("id")
        or stable_id(json.dumps(row, sort_keys=True))
    )


def source_issue(source_label: str) -> int:
    if source_label.startswith("1185"):
        return 1185
    if source_label.startswith("1186"):
        return 1186
    if source_label.startswith("1187"):
        return 1187
    if source_label.startswith("1188"):
        return 1188
    if source_label.startswith("1189"):
        return 1189
    return 0


def source_domain(source_label: str) -> str:
    mapping = {
        "1185_oncology": "oncology",
        "1186_metabolic_cardiovascular": "metabolic_cardiovascular_renal",
        "1187_neuro_repaired": "neurodegeneration_neuropsychiatric",
        "1188_infectious_immunology": "infectious_immunology_inflammation",
        "1189_rare_disease": "rare_disease",
    }
    return mapping.get(source_label, source_label)


def top_dict_values(rows: list[dict[str, Any]], key: str, limit: int = 3) -> list[Any]:
    values: list[Any] = []
    for row in rows:
        if key in row:
            values.append(row[key])
    return values[:limit]


def nested_strings(value: Any, *, keys: set[str], limit: int = 20) -> list[str]:
    out: list[str] = []

    def walk(item: Any) -> None:
        if len(out) >= limit:
            return
        if isinstance(item, dict):
            for k, v in item.items():
                lk = clean_text(k).lower()
                if lk in keys and not isinstance(v, (dict, list)):
                    text = clean_text(v)
                    if text:
                        out.append(text)
                walk(v)
        elif isinstance(item, list):
            for child in item:
                walk(child)

    walk(value)
    return uniq(out)[:limit]


def nested_source_hashes(value: Any, limit: int = 20) -> list[str]:
    keys = {"source_sha256", "raw_sha256", "api_response_sha256", "text_sha256"}
    return nested_strings(value, keys=keys, limit=limit)


def nested_snippets(value: Any, limit: int = 5) -> list[str]:
    keys = {"source_text", "text_snippet", "summary", "text_excerpt", "citation"}
    snippets = []
    for text in nested_strings(value, keys=keys, limit=limit * 2):
        if len(text) > 420:
            text = text[:417] + "..."
        snippets.append(text)
    return uniq(snippets)[:limit]


def nested_evidence_types(value: Any, limit: int = 20) -> list[str]:
    keys = {"kind", "evidence_type", "source_system", "source_class", "candidate_type"}
    return nested_strings(value, keys=keys, limit=limit)


def nested_pathway_tags(value: Any, limit: int = 20) -> list[str]:
    keys = {"interaction_types", "source_dbs", "hypothesis_family", "novelty_class"}
    tags: list[str] = []

    def walk(item: Any) -> None:
        if len(tags) >= limit:
            return
        if isinstance(item, dict):
            for k, v in item.items():
                lk = clean_text(k).lower()
                if lk in keys:
                    if isinstance(v, list):
                        tags.extend(clean_text(x) for x in v)
                    else:
                        tags.append(clean_text(v))
                walk(v)
        elif isinstance(item, list):
            for child in item:
                walk(child)

    walk(value)
    return uniq([t for t in tags if t])[:limit]


def names_for_row(row: dict[str, Any], source_label: str) -> dict[str, list[str]]:
    genes: list[object] = []
    drugs: list[object] = []
    diseases: list[object] = []
    disease_ids: list[object] = []
    target_ids: list[object] = []

    if source_label == "1185_oncology":
        genes.append(row.get("gene"))
        drugs.extend(row.get("therapies") or [])
        diseases.append(row.get("cancer_type"))
        if row.get("variant"):
            genes.append(row.get("variant"))
    elif source_label == "1186_metabolic_cardiovascular":
        genes.append(row.get("target_name"))
        target_ids.append(row.get("target_id"))
        drugs.append(row.get("drug_name"))
        diseases.append(row.get("disease_name"))
        disease_ids.append(row.get("disease_id"))
    elif source_label in {"1187_neuro_repaired", "1188_infectious_immunology"}:
        target = row.get("target")
        bridge = row.get("bridge")
        if isinstance(target, dict):
            if clean_text(target.get("type")).lower() in {"chemical", "drug"}:
                drugs.append(target.get("name"))
            elif "disease" in clean_text(target.get("type")).lower():
                diseases.append(target.get("name"))
                disease_ids.append(target.get("id"))
            else:
                genes.append(target.get("name"))
                target_ids.append(target.get("id"))
        if isinstance(bridge, dict):
            if clean_text(bridge.get("type")).lower() in {"chemical", "drug"}:
                drugs.append(bridge.get("name"))
            elif "disease" in clean_text(bridge.get("type")).lower():
                diseases.append(bridge.get("name"))
                disease_ids.append(bridge.get("id"))
            else:
                genes.append(bridge.get("name"))
                target_ids.append(bridge.get("id"))
        normalized = row.get("normalized_names")
        if isinstance(normalized, list):
            for item in normalized:
                if isinstance(item, dict):
                    typ = clean_text(item.get("type")).lower()
                    name = item.get("name") or item.get("normalized_name")
                    if "drug" in typ or "chemical" in typ:
                        drugs.append(name)
                    elif "disease" in typ:
                        diseases.append(name)
                        disease_ids.append(item.get("id"))
                    elif "gene" in typ or "target" in typ:
                        genes.append(name)
                        target_ids.append(item.get("id"))
                else:
                    diseases.append(item)
        evidence = row.get("evidence_paths") or []
        for ep in evidence[:10]:
            if isinstance(ep, dict):
                genes.extend(nested_strings(ep, keys={"gene", "target_name", "query_target_symbol"}, limit=3))
                drugs.extend(nested_strings(ep, keys={"drug", "intervention", "matched_intervention_value"}, limit=3))
                diseases.extend(nested_strings(ep, keys={"disease_name", "condition", "matched_condition_value"}, limit=3))
    elif source_label == "1189_rare_disease":
        disease = row.get("disease")
        gene = row.get("gene")
        drug = row.get("drug")
        if isinstance(disease, dict):
            diseases.append(disease.get("name") or disease.get("mondo_name"))
            disease_ids.extend([disease.get("id"), disease.get("mondo_id")])
        if isinstance(gene, dict):
            genes.append(gene.get("symbol") or gene.get("name"))
            target_ids.append(gene.get("id"))
        if isinstance(drug, dict):
            drugs.append(drug.get("name"))
        phenotypes = row.get("phenotypes")
        if isinstance(phenotypes, list):
            diseases.extend([p.get("hpo_name") for p in phenotypes[:3] if isinstance(p, dict)])

    return {
        "target_names": uniq(genes),
        "target_ids": uniq(target_ids),
        "drug_names": uniq(drugs),
        "disease_names": uniq(diseases),
        "disease_ids": uniq(disease_ids),
    }


def validation_summary(row: dict[str, Any], novelty: dict[str, Any] | None) -> dict[str, Any]:
    external = row.get("external_validation")
    open_targets = row.get("open_targets_context")
    evidence_paths = row.get("evidence_paths") or []
    has_open_targets = bool(open_targets) or any(
        isinstance(ep, dict) and "open_targets" in clean_text(ep.get("kind")).lower()
        for ep in evidence_paths
    )
    has_dgidb = any(
        isinstance(ep, dict) and "dgidb" in clean_text(ep.get("kind")).lower()
        for ep in evidence_paths
    )
    has_trials = bool(row.get("nct_ids")) or any(
        "clinicaltrial" in json.dumps(ep, sort_keys=True).lower()
        or "nct" in json.dumps(ep, sort_keys=True).lower()
        for ep in evidence_paths[:20]
    )
    has_civic = bool(external and "civic" in json.dumps(external, sort_keys=True).lower())
    if novelty and isinstance(novelty.get("evidence_shape"), dict):
        shape = novelty["evidence_shape"]
        has_open_targets = has_open_targets or bool(shape.get("has_open_targets"))
        has_dgidb = has_dgidb or bool(shape.get("has_dgidb"))
        has_trials = has_trials or bool(shape.get("has_trial_context"))
        has_civic = has_civic or bool(shape.get("has_civic"))
    sources = []
    if has_civic:
        sources.append("CIViC")
    if has_open_targets:
        sources.append("Open Targets")
    if has_dgidb:
        sources.append("DGIdb")
    if has_trials:
        sources.append("ClinicalTrials.gov")
    if external and not sources:
        sources.append(clean_text(external.get("source") if isinstance(external, dict) else "external_validation"))
    return {
        "source_systems": uniq(sources),
        "has_civic": has_civic,
        "has_open_targets": has_open_targets,
        "has_dgidb": has_dgidb,
        "has_trial_context": has_trials,
        "external_validation_present": bool(external or open_targets or sources),
        "summary": "; ".join(uniq(sources)) if sources else "no external validation overlay in current atlas inputs",
    }


def normalized_hypothesis_text(entry: dict[str, Any]) -> str:
    targets = ", ".join(entry.get("target_names") or [])
    drugs = ", ".join(entry.get("drug_names") or [])
    diseases = ", ".join(entry.get("disease_names") or [])
    pieces = []
    if targets:
        pieces.append(f"target(s) {targets}")
    if drugs:
        pieces.append(f"drug(s) {drugs}")
    if diseases:
        pieces.append(f"disease/context {diseases}")
    if not pieces:
        pieces.append(f"candidate {entry['candidate_id']}")
    return f"{entry['disease_area']} hypothesis: " + " | ".join(pieces)


def source_summary_snippet(entry: dict[str, Any]) -> str:
    parts = [
        f"Source {entry['source_label']} row {entry['source_row_index']}",
        f"candidate {entry['candidate_id']}",
        f"type {entry.get('candidate_type') or 'unknown'}",
        f"rank_score {entry.get('rank_score')}",
    ]
    return "; ".join(parts)


def trial_flags(row: dict[str, Any], flag: dict[str, Any] | None) -> list[str]:
    flags: list[object] = []
    flags.extend(row.get("trial_flags") or [])
    flags.extend(row.get("safety_trial_flags") or [])
    if row.get("nct_ids"):
        flags.append("nct_ids_present")
    if flag:
        for code in flag.get("reason_codes") or []:
            if "trial" in clean_text(code).lower():
                flags.append(code)
    return [item for item in uniq(flags) if item != "no_drug_candidate_in_row"]


def safety_flags(row: dict[str, Any], flag: dict[str, Any] | None) -> list[str]:
    flags: list[object] = []
    flags.extend(row.get("safety_flags") or [])
    flags.extend(row.get("safety_trial_flags") or [])
    if flag:
        for code in flag.get("reason_codes") or []:
            lower = clean_text(code).lower()
            if "safety" in lower or "risk" in lower:
                flags.append(code)
    return [item for item in uniq(flags) if item != "no_drug_candidate_in_row"]


def review_status(flag: dict[str, Any] | None, novelty: dict[str, Any] | None) -> str:
    if flag and flag.get("blocked_or_demoted"):
        return "blocked_or_demoted_before_human_review"
    if novelty and novelty.get("calibration_known_positive"):
        return "calibration_known_positive_reference"
    if flag and flag.get("sweep_status") == "complete_no_counterevidence_found_in_current_sources":
        return "ready_for_hypothesis_review"
    if flag:
        return "needs_evidence_review"
    return "missing_falsification_overlay_fail_closed"


def next_experiment(row: dict[str, Any], entry: dict[str, Any]) -> str:
    reasons = set(entry.get("falsification_reason_codes") or [])
    if entry["review_status"] == "calibration_known_positive_reference":
        return "Use as calibration/proof row; exclude from novelty-promotion claims."
    if "clinicaltrials_stopped_trial" in reasons or entry.get("counter_evidence_count", 0) > 0:
        return "Resolve counterevidence source rows and keep demoted unless a stronger persisted outcome instrument reverses it."
    if any("trial_source_missing" in r for r in reasons):
        return "Acquire exact drug-disease trial/outcome rows before any atlas promotion."
    if any("safety" in r or "risk" in r for r in reasons):
        return "Complete openFDA/FAERS safety triage and human safety review before any atlas promotion."
    if entry.get("drug_names") and entry.get("disease_names"):
        return "Run outcome-grounded drug-disease validation with safety, trial, and mechanism evidence."
    if entry.get("target_names") and entry.get("disease_names"):
        return "Validate target-disease association in an outcome-backed assay/model before drug inference."
    if source_issue(entry["source_label"]) == 1189:
        return "Validate rare disease-gene-phenotype bridge against ontology and model/assay outcome evidence."
    return "Human reviewer should inspect evidence bundle and define the next grounded outcome instrument."


def build_input_manifest(paths: dict[str, str]) -> dict[str, Any]:
    inputs: dict[str, Any] = {}
    for label, raw in paths.items():
        path = Path(raw)
        inputs[label] = manifest_entry(path, jsonl=path.suffix == ".jsonl")
    return {"schema_version": 1, "inputs": inputs}


def collect_rows(paths: dict[str, str]) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    source_rows: list[dict[str, Any]] = []
    id_to_source: dict[str, dict[str, Any]] = {}
    for label, raw in paths.items():
        path = Path(raw)
        file_sha = sha256_path(path)
        for idx, row in enumerate(rows_jsonl(path)):
            cid = row_id(row)
            wrapped = {
                "candidate_id": cid,
                "source_label": label,
                "source_issue": source_issue(label),
                "source_domain": source_domain(label),
                "source_path": str(path),
                "source_sha256": file_sha,
                "source_row_index": idx,
                "source_row_sha256": hashlib.sha256(
                    json.dumps(row, sort_keys=True, ensure_ascii=False).encode("utf-8")
                ).hexdigest(),
                "row": row,
            }
            source_rows.append(wrapped)
            current = id_to_source.get(cid)
            if current is None or float(row.get("rank_score") or 0.0) > float(current["row"].get("rank_score") or 0.0):
                id_to_source[cid] = wrapped
    return source_rows, id_to_source


def evidence_map(rows: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    out: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        out[clean_text(row.get("candidate_id"))].append(row)
    return out


def novelty_maps(overlays: dict[str, str]) -> dict[str, dict[str, dict[str, Any]]]:
    maps: dict[str, dict[str, dict[str, Any]]] = {
        "combined": {},
        "calibration": {},
        "novelty": {},
    }
    for name, key in [
        ("combined", "novelty_combined"),
        ("calibration", "novelty_calibration"),
        ("novelty", "novelty_research_leads"),
    ]:
        for row in rows_jsonl(Path(overlays[key])):
            maps[name][clean_text(row.get("candidate_id"))] = row
    return maps


def evidence_summaries(rows: list[dict[str, Any]], limit: int = 5) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    for row in rows[:limit]:
        summaries.append(
            {
                "source_system": row.get("source_system"),
                "reason_code": row.get("reason_code"),
                "summary": row.get("summary"),
                "weight": row.get("weight"),
                "source_sha256": row.get("source_sha256"),
                "source_path": row.get("source_path"),
                "source_row_index": row.get("source_row_index"),
            }
        )
    return summaries


def score_entry(entry: dict[str, Any], novelty: dict[str, Any] | None) -> dict[str, float]:
    rank_percentile = float(entry.get("rank_percentile_within_source") or 0.0)
    support_signal = clamp(float(entry.get("support_evidence_count") or 0.0) / 8.0)
    validation_signal = clamp(len(entry["validation_evidence"]["source_systems"]) / 4.0)
    counter_penalty = clamp(float(entry.get("counter_evidence_weight") or 0.0) / 8.0)
    missing_penalty = 0.18 if any("missing" in c for c in entry.get("falsification_reason_codes", [])) else 0.0
    safety_penalty = 0.18 if entry.get("safety_flags") else 0.0
    trial_penalty = 0.12 if any("missing" in c for c in entry.get("trial_flags", [])) else 0.0
    raw = (
        0.34 * rank_percentile
        + 0.25 * support_signal
        + 0.21 * validation_signal
        + 0.10 * (1.0 if entry["review_status"] == "ready_for_hypothesis_review" else 0.0)
        - 0.22 * counter_penalty
        - missing_penalty
        - safety_penalty
        - trial_penalty
    )
    confidence = clamp(raw)
    if entry["review_status"] == "blocked_or_demoted_before_human_review":
        confidence = min(confidence, 0.35)
    if entry["review_status"] == "missing_falsification_overlay_fail_closed":
        confidence = 0.0
    novelty_score = novelty.get("novelty_priority_score") if novelty else None
    if novelty_score is None:
        calibration_penalty = 0.35 if entry["review_status"] == "calibration_known_positive_reference" else 0.0
        drug_bonus = 0.08 if entry.get("drug_names") else 0.0
        rare_bonus = 0.07 if entry.get("source_issue") == 1189 else 0.0
        novelty_score = clamp(0.55 * rank_percentile + drug_bonus + rare_bonus - calibration_penalty)
    return {
        "confidence_score": round(confidence, 6),
        "novelty_score": round(clamp(float(novelty_score)), 6),
        "rank_percentile_within_source": round(rank_percentile, 6),
        "support_signal": round(support_signal, 6),
        "validation_signal": round(validation_signal, 6),
        "counter_penalty": round(counter_penalty, 6),
    }


def rank_percentiles(source_rows: list[dict[str, Any]]) -> dict[str, float]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for item in source_rows:
        grouped[item["source_label"]].append(item)
    out: dict[str, float] = {}
    for rows in grouped.values():
        rows_sorted = sorted(
            rows,
            key=lambda item: (
                float(item["row"].get("rank_score") or item["row"].get("typed_score") or 0.0),
                -int(item["source_row_index"]),
            ),
            reverse=True,
        )
        n = max(1, len(rows_sorted))
        for i, item in enumerate(rows_sorted):
            out[item["candidate_id"]] = 1.0 - (i / n)
    return out


def build_atlas(root: Path, inputs: dict[str, str], overlays: dict[str, str]) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    input_manifest = build_input_manifest({**inputs, **overlays})
    write_json(out_dir / "input_manifest.json", input_manifest)

    source_rows, id_to_source = collect_rows(inputs)
    percentiles = rank_percentiles(source_rows)
    flags = {
        clean_text(row.get("candidate_id")): row
        for row in rows_jsonl(Path(overlays["falsification_flags"]))
    }
    support_by_id = evidence_map(rows_jsonl(Path(overlays["support_evidence"])))
    counter_by_id = evidence_map(rows_jsonl(Path(overlays["counter_evidence"])))
    nmap = novelty_maps(overlays)

    atlas_rows: list[dict[str, Any]] = []
    missing_flags: list[str] = []
    for cid, wrapped in sorted(id_to_source.items(), key=lambda item: (item[1]["source_label"], item[1]["source_row_index"])):
        row = wrapped["row"]
        flag = flags.get(cid)
        if flag is None:
            missing_flags.append(cid)
        novelty = nmap["combined"].get(cid)
        names = names_for_row(row, wrapped["source_label"])
        support_rows = support_by_id.get(cid, [])
        counter_rows = counter_by_id.get(cid, [])
        status = review_status(flag, novelty)
        validation = validation_summary(row, novelty)
        snippets = nested_snippets(row)
        entry: dict[str, Any] = {
            "schema_version": 1,
            "atlas_id": f"issue1193:{stable_id(cid)}",
            "candidate_id": cid,
            "source_issue": wrapped["source_issue"],
            "source_label": wrapped["source_label"],
            "disease_area": wrapped["source_domain"],
            "hypothesis_only": True,
            "clinical_boundary": CLINICAL_BOUNDARY,
            "candidate_type": row.get("candidate_type") or row.get("source_class") or row.get("hypothesis_class"),
            "source_path": wrapped["source_path"],
            "source_sha256": wrapped["source_sha256"],
            "source_row_index": wrapped["source_row_index"],
            "source_row_sha256": wrapped["source_row_sha256"],
            "rank": row.get("rank"),
            "rank_score": row.get("rank_score") or row.get("typed_score"),
            "rank_percentile_within_source": percentiles.get(cid, 0.0),
            "target_names": names["target_names"],
            "target_ids": names["target_ids"],
            "drug_names": names["drug_names"],
            "disease_names": names["disease_names"],
            "disease_ids": names["disease_ids"],
            "pathway_names": nested_pathway_tags(row),
            "evidence_types": nested_evidence_types(row),
            "source_text_snippets": snippets,
            "source_hashes": uniq([wrapped["source_sha256"], wrapped["source_row_sha256"]] + nested_source_hashes(row)),
            "typed_evidence_path_count": len(row.get("evidence_paths") or []),
            "typed_evidence_path_kinds": uniq(
                [clean_text(ep.get("kind")) for ep in (row.get("evidence_paths") or []) if isinstance(ep, dict)]
            ),
            "validation_evidence": validation,
            "support_evidence_count": len(support_rows),
            "support_evidence_weight": round(sum(float(r.get("weight") or 0.0) for r in support_rows), 6),
            "support_evidence_examples": evidence_summaries(support_rows),
            "counter_evidence_count": len(counter_rows),
            "counter_evidence_weight": round(sum(float(r.get("weight") or 0.0) for r in counter_rows), 6),
            "counter_evidence_examples": evidence_summaries(counter_rows),
            "falsification_status": flag.get("sweep_status") if flag else "missing_falsification_overlay_fail_closed",
            "falsification_reason_codes": flag.get("reason_codes") if flag else ["missing_falsification_overlay_fail_closed"],
            "blocked_or_demoted": bool(flag and flag.get("blocked_or_demoted")),
            "safety_flags": safety_flags(row, flag),
            "trial_flags": trial_flags(row, flag),
            "calibration_known_positive": bool(novelty and novelty.get("calibration_known_positive")),
            "calibration_flags": novelty.get("calibration_flags") if novelty else [],
            "calibration_rank": nmap["calibration"].get(cid, {}).get("calibration_rank"),
            "novelty_rank": nmap["novelty"].get(cid, {}).get("novelty_rank"),
            "review_status": status,
            "filters": {
                "disease_area": wrapped["source_domain"],
                "source_issue": str(wrapped["source_issue"]),
                "review_status": status,
                "has_drug": bool(names["drug_names"]),
                "has_target": bool(names["target_names"]),
                "has_open_targets": validation["has_open_targets"],
                "has_dgidb": validation["has_dgidb"],
                "has_trial_context": validation["has_trial_context"],
                "has_safety_flags": bool(safety_flags(row, flag)),
                "calibration_known_positive": bool(novelty and novelty.get("calibration_known_positive")),
            },
        }
        entry["normalized_hypothesis"] = normalized_hypothesis_text(entry)
        if not entry["source_text_snippets"]:
            entry["source_text_snippets"] = [source_summary_snippet(entry)]
        scores = score_entry(entry, novelty)
        entry.update(scores)
        entry["confidence_score_meaning"] = "triage confidence for evidence completeness only; not clinical confidence"
        entry["novelty_score_meaning"] = "research-prioritization score only; not clinical novelty"
        entry["next_validation_experiment"] = next_experiment(row, entry)
        atlas_rows.append(entry)

    atlas_rows.sort(
        key=lambda row: (
            row["review_status"] != "ready_for_hypothesis_review",
            -float(row["novelty_score"]),
            -float(row["confidence_score"]),
            row["candidate_id"],
        )
    )
    for i, row in enumerate(atlas_rows, start=1):
        row["atlas_rank"] = i

    atlas_path = out_dir / "human_review_biomedical_hypothesis_atlas.jsonl"
    write_jsonl(atlas_path, atlas_rows)
    atlas_sha = sha256_path(atlas_path)

    tsv_path = out_dir / "human_review_biomedical_hypothesis_atlas.tsv"
    write_atlas_tsv(tsv_path, atlas_rows)
    filters = build_filters(atlas_rows)
    write_json(out_dir / "atlas_filters.json", filters)
    bundles = build_evidence_bundles(atlas_rows)
    write_json(out_dir / "top_evidence_bundles.json", bundles)
    bridge_rows = build_bridge_rows(atlas_rows[:1000], atlas_path, atlas_sha)
    write_jsonl(out_dir / "atlas_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(atlas_rows, source_rows, missing_flags, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1193,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "atlas_jsonl": artifact(atlas_path, jsonl=True),
            "atlas_tsv": artifact(tsv_path, jsonl=False),
            "atlas_filters": artifact(out_dir / "atlas_filters.json"),
            "top_evidence_bundles": artifact(out_dir / "top_evidence_bundles.json"),
            "atlas_bridge_rows": artifact(out_dir / "atlas_bridge_rows.jsonl", jsonl=True),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, atlas_rows, source_rows, flags, missing_flags)
    write_json(out_dir / "persisted_readback.json", readback)

    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "atlas_jsonl": output_manifest["artifacts"]["atlas_jsonl"],
            "atlas_bridge_rows": output_manifest["artifacts"]["atlas_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def write_atlas_tsv(path: Path, rows: list[dict[str, Any]]) -> None:
    fields = [
        "atlas_rank",
        "candidate_id",
        "normalized_hypothesis",
        "source_issue",
        "disease_area",
        "review_status",
        "confidence_score",
        "novelty_score",
        "target_names",
        "drug_names",
        "disease_names",
        "evidence_types",
        "falsification_status",
        "falsification_reason_codes",
        "next_validation_experiment",
    ]
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, delimiter="\t")
        writer.writeheader()
        for row in rows:
            writer.writerow(
                {
                    field: "; ".join(str(x) for x in row[field])
                    if isinstance(row.get(field), list)
                    else row.get(field)
                    for field in fields
                }
            )


def build_filters(rows: list[dict[str, Any]]) -> dict[str, Any]:
    counters = {
        "disease_area": Counter(),
        "review_status": Counter(),
        "source_issue": Counter(),
        "drug": Counter(),
        "target": Counter(),
        "disease": Counter(),
        "pathway": Counter(),
        "evidence_type": Counter(),
        "falsification_status": Counter(),
    }
    for row in rows:
        counters["disease_area"][row["disease_area"]] += 1
        counters["review_status"][row["review_status"]] += 1
        counters["source_issue"][str(row["source_issue"])] += 1
        counters["falsification_status"][row["falsification_status"]] += 1
        for key, values in [
            ("drug", row["drug_names"]),
            ("target", row["target_names"]),
            ("disease", row["disease_names"]),
            ("pathway", row["pathway_names"]),
            ("evidence_type", row["evidence_types"]),
        ]:
            for value in values:
                counters[key][value] += 1
    return {
        "schema_version": 1,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "filters": {
            key: [{"value": value, "count": count} for value, count in counter.most_common(100)]
            for key, counter in counters.items()
        },
    }


def build_evidence_bundles(rows: list[dict[str, Any]], limit: int = 50) -> dict[str, Any]:
    bundle_rows = []
    for row in rows[:limit]:
        bundle_rows.append(
            {
                "atlas_rank": row["atlas_rank"],
                "candidate_id": row["candidate_id"],
                "normalized_hypothesis": row["normalized_hypothesis"],
                "review_status": row["review_status"],
                "confidence_score": row["confidence_score"],
                "novelty_score": row["novelty_score"],
                "target_names": row["target_names"],
                "drug_names": row["drug_names"],
                "disease_names": row["disease_names"],
                "source_text_snippets": row["source_text_snippets"][:3],
                "typed_evidence_path_kinds": row["typed_evidence_path_kinds"][:8],
                "validation_evidence": row["validation_evidence"],
                "support_evidence_examples": row["support_evidence_examples"][:3],
                "counter_evidence_examples": row["counter_evidence_examples"][:3],
                "source_hashes": row["source_hashes"][:8],
                "next_validation_experiment": row["next_validation_experiment"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return {
        "schema_version": 1,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "bundle_count": len(bundle_rows),
        "bundles": bundle_rows,
    }


def build_bridge_rows(rows: list[dict[str, Any]], atlas_path: Path, atlas_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows:
        terms = uniq(
            [
                row["review_status"],
                row["disease_area"],
                *(row["target_names"][:2]),
                *(row["drug_names"][:2]),
                *(row["disease_names"][:2]),
            ]
        )[:6]
        text = (
            f"Human review biomedical atlas candidate {row['candidate_id']}: "
            f"{row['review_status']} in {row['disease_area']} for "
            f"{', '.join(row['target_names'][:2])} "
            f"{', '.join(row['drug_names'][:2])} "
            f"{', '.join(row['disease_names'][:2])}. "
            f"{row['normalized_hypothesis']}"
        )
        bridge_terms = [term for term in terms if term and term in text]
        if not bridge_terms:
            bridge_terms = [row["review_status"], row["disease_area"]]
            text += f" {row['review_status']} {row['disease_area']}."
        out.append(
            {
                "id": row["candidate_id"],
                "domain": "biomedical_human_review_atlas",
                "text": text,
                "bridge_terms": bridge_terms,
                "metadata": {
                    "source_dataset": "issue1193_biomedical_hypothesis_atlas",
                    "source_path": str(atlas_path),
                    "source_sha256": atlas_sha,
                    "atlas_rank": str(row["atlas_rank"]),
                    "review_status": row["review_status"],
                    "confidence_score": str(row["confidence_score"]),
                    "novelty_score": str(row["novelty_score"]),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def build_metrics(
    atlas_rows: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    missing_flags: list[str],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    status_counts = Counter(row["review_status"] for row in atlas_rows)
    source_counts = Counter(row["source_label"] for row in atlas_rows)
    area_counts = Counter(row["disease_area"] for row in atlas_rows)
    return {
        "schema_version": 1,
        "status": "ok",
        "input_source_rows": len(source_rows),
        "deduped_atlas_rows": len(atlas_rows),
        "bridge_rows": len(bridge_rows),
        "missing_falsification_flags": len(missing_flags),
        "hypothesis_only_rows": sum(1 for row in atlas_rows if row["hypothesis_only"]),
        "review_status_counts": dict(status_counts),
        "source_label_counts": dict(source_counts),
        "disease_area_counts": dict(area_counts),
        "blocked_or_demoted_rows": sum(1 for row in atlas_rows if row["blocked_or_demoted"]),
        "ready_for_hypothesis_review_rows": status_counts["ready_for_hypothesis_review"],
        "calibration_known_positive_rows": status_counts["calibration_known_positive_reference"],
        "rows_with_drug": sum(1 for row in atlas_rows if row["drug_names"]),
        "rows_with_target": sum(1 for row in atlas_rows if row["target_names"]),
        "rows_with_disease": sum(1 for row in atlas_rows if row["disease_names"]),
        "rows_with_source_snippets": sum(1 for row in atlas_rows if row["source_text_snippets"]),
        "rows_with_normalized_hypothesis": sum(1 for row in atlas_rows if row.get("normalized_hypothesis")),
        "rows_with_validation_evidence": sum(1 for row in atlas_rows if row["validation_evidence"]["external_validation_present"]),
        "rows_with_support_evidence": sum(1 for row in atlas_rows if row["support_evidence_count"] > 0),
        "rows_with_counter_evidence": sum(1 for row in atlas_rows if row["counter_evidence_count"] > 0),
        "clinical_boundary_rows": sum(1 for row in atlas_rows if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_ready_candidates": [
            {
                "candidate_id": row["candidate_id"],
                "atlas_rank": row["atlas_rank"],
                "disease_area": row["disease_area"],
                "confidence_score": row["confidence_score"],
                "novelty_score": row["novelty_score"],
                "target_names": row["target_names"][:3],
                "drug_names": row["drug_names"][:3],
                "disease_names": row["disease_names"][:3],
            }
            for row in atlas_rows
            if row["review_status"] == "ready_for_hypothesis_review"
        ][:10],
    }


def build_readback(
    out_dir: Path,
    atlas_rows: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    flags: dict[str, dict[str, Any]],
    missing_flags: list[str],
) -> dict[str, Any]:
    artifacts = {
        "atlas_jsonl": artifact(out_dir / "human_review_biomedical_hypothesis_atlas.jsonl", jsonl=True),
        "atlas_tsv": artifact(out_dir / "human_review_biomedical_hypothesis_atlas.tsv"),
        "atlas_filters": artifact(out_dir / "atlas_filters.json"),
        "top_evidence_bundles": artifact(out_dir / "top_evidence_bundles.json"),
        "atlas_bridge_rows": artifact(out_dir / "atlas_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
    }
    status_counts = Counter(row["review_status"] for row in atlas_rows)
    return {
        "schema_version": 1,
        "issue": 1193,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "deduped_rows_match_flags": len(atlas_rows) == len(flags) and not missing_flags,
            "atlas_jsonl_rows_match": artifacts["atlas_jsonl"]["rows"] == len(atlas_rows),
            "bridge_rows_1000_or_less": artifacts["atlas_bridge_rows"]["rows"] == min(1000, len(atlas_rows)),
            "all_rows_hypothesis_only": all(row["hypothesis_only"] for row in atlas_rows),
            "all_rows_have_clinical_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in atlas_rows),
            "all_rows_have_normalized_hypothesis": all(bool(row.get("normalized_hypothesis")) for row in atlas_rows),
            "all_rows_have_source_snippet": all(bool(row.get("source_text_snippets")) for row in atlas_rows),
            "all_rows_have_review_status": all(bool(row["review_status"]) for row in atlas_rows),
            "filters_present": artifacts["atlas_filters"]["bytes"] > 0,
            "top_evidence_bundles_present": artifacts["top_evidence_bundles"]["bytes"] > 0,
            "source_rows_deduped": len(source_rows) >= len(atlas_rows),
            "ready_rows_present": status_counts["ready_for_hypothesis_review"] > 0,
            "blocked_rows_present": status_counts["blocked_or_demoted_before_human_review"] > 0,
        },
        "row_counts": {
            "source_rows": len(source_rows),
            "deduped_atlas_rows": len(atlas_rows),
            "falsification_flags": len(flags),
            "missing_falsification_flags": len(missing_flags),
            "review_status_counts": dict(status_counts),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "root",
        nargs="?",
        default="/home/croyse/calyx/fsv/issue1193-human-review-atlas-20260704T000000Z",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    result = build_atlas(Path(args.root), DEFAULT_INPUTS, DEFAULT_OVERLAYS)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
