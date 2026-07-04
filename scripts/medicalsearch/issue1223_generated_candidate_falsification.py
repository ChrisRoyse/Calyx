#!/usr/bin/env python3
"""#1223 falsification sweep for generated disease-hunt candidates.

This sweep consumes generated candidate JSONL files from #1185/#1186/#1187/
#1188/#1189 and writes one falsification flag per candidate. It is a triage
and demotion instrument only; it does not establish efficacy, safety,
clinical actionability, treatment guidance, or cure evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Falsification triage only; not efficacy, safety, clinical actionability, "
    "treatment guidance, dosing, recommendation, or cure evidence."
)

DEFAULT_INPUTS = {
    "1185_oncology": "/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl",
    "1186_metabolic_cardiovascular": "/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/out/metabolic_cardiovascular_hypotheses.jsonl",
    "1187_neuro_repaired": "/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187/out/neuro_hypotheses.jsonl",
    "1188_infectious_immunology": "/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_hypotheses.jsonl",
    "1189_rare_disease": "/home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z/out/rare_disease_hypotheses.jsonl",
}

DEFAULT_SOURCES = {
    "pubtator_support": "/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z/parsed/supporting_literature.jsonl",
    "pubtator_negative": "/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z/parsed/contradicting_or_negative_literature.jsonl",
    "clinicaltrials_rows": "/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z/parsed/clinicaltrials_trial_rows.jsonl",
    "clinicaltrials_summaries": "/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z/parsed/clinicaltrials_seed_summaries.jsonl",
    "dgidb_seed": "/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z/parsed/seed_pair_graphql_interactions.jsonl",
    "dgidb_broad": "/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z/parsed/broad_graphql_interactions.jsonl",
    "dgidb_unmapped": "/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z/parsed/unmapped_rows.jsonl",
    "open_targets_rows": "/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_association_rows.jsonl",
    "open_targets_edges": "/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_validation_edges.jsonl",
    "safety_terms": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/drug_safety_terms.jsonl",
    "candidate_safety_flags": "/home/croyse/calyx/fsv/issue1181-drug-safety-triage-20260704T025756Z/out/candidate_safety_flags.jsonl",
    "issue1224_dgidb": "/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/dgidb_target_interactions.jsonl",
    "issue1224_open_targets": "/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/open_targets_context_rows.jsonl",
    "issue1189_dgidb": "/home/croyse/calyx/fsv/issue1189-rare-disease-hunt-20260704T114953Z/out/dgidb_target_interactions.jsonl",
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


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def line_count(path: Path) -> int:
    with path.open(encoding="utf-8", errors="replace") as handle:
        return sum(1 for line in handle if line.strip())


def norm(value: object) -> str:
    return re.sub(r"[^a-z0-9]+", " ", str(value or "").lower()).strip()


def clean_symbol(value: object) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    text = re.sub(r"^HGNC:", "", text, flags=re.I)
    text = re.sub(r"[^A-Za-z0-9_.-]+", "", text)
    return text.upper() or None


def lower_set(values: list[Any]) -> set[str]:
    return {norm(value) for value in values if norm(value)}


def candidate_inputs(paths: dict[str, Path]) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    candidates: list[dict[str, Any]] = []
    manifest: dict[str, dict[str, Any]] = {}
    for label, path in paths.items():
        if not path.exists():
            raise SystemExit(f"CALYX_ISSUE1223_MISSING_INPUT: {label}: {path}")
        source_sha = sha256_path(path)
        rows = load_jsonl(path)
        manifest[label] = {
            "path": str(path),
            "bytes": path.stat().st_size,
            "sha256": source_sha,
            "rows": len(rows),
        }
        for idx, row in enumerate(rows, start=1):
            candidates.append(normalize_candidate(label, row, idx, path, source_sha))
    deduped: dict[str, dict[str, Any]] = {}
    for candidate in candidates:
        deduped.setdefault(candidate["candidate_id"], candidate)
    return list(deduped.values()), manifest


def normalize_candidate(label: str, row: dict[str, Any], index: int, path: Path, source_sha: str) -> dict[str, Any]:
    candidate_id = (
        row.get("hypothesis_id")
        or row.get("candidate_id")
        or row.get("bridge_id")
        or stable_id(label, index, row.get("rank_score"), row.get("source_name"), row.get("target_name"))
    )
    genes = set()
    drugs = set()
    diseases = set()
    disease_ids = set()
    phenotypes = []

    for key in ["gene", "target", "source", "bridge"]:
        value = row.get(key)
        if isinstance(value, dict):
            typ = norm(value.get("type"))
            name = value.get("symbol") or value.get("name")
            if key == "gene" or "gene" in typ or "target" in typ:
                sym = clean_symbol(name)
                if sym:
                    genes.add(sym)
            if "chemical" in typ or "drug" in typ:
                drugs.add(str(name))
            if "disease" in typ:
                diseases.add(str(name))
                if value.get("id"):
                    disease_ids.add(str(value.get("id")))
    if row.get("gene") and not isinstance(row.get("gene"), dict):
        sym = clean_symbol(row.get("gene"))
        if sym:
            genes.add(sym)
    for key in ["target_name", "source_name"]:
        typ = norm(row.get(key.replace("_name", "_type")))
        if "gene" in typ:
            sym = clean_symbol(row.get(key))
            if sym:
                genes.add(sym)
        if "chemical" in typ or "drug" in typ:
            drugs.add(str(row.get(key)))
        if "disease" in typ:
            diseases.add(str(row.get(key)))
    for key in ["disease_name", "cancer_type"]:
        if row.get(key):
            diseases.add(str(row.get(key)))
    disease = row.get("disease")
    if isinstance(disease, dict):
        if disease.get("name"):
            diseases.add(str(disease.get("name")))
        for id_key in ["id", "mondo_id"]:
            if disease.get(id_key):
                disease_ids.add(str(disease.get(id_key)))
    for key in ["disease_id"]:
        if row.get(key):
            disease_ids.add(str(row.get(key)))
    drug = row.get("drug")
    if isinstance(drug, dict):
        if drug.get("name"):
            drugs.add(str(drug.get("name")))
    elif drug:
        drugs.add(str(drug))
    for key in ["drug_name", "drug"]:
        if row.get(key) and not isinstance(row.get(key), dict):
            drugs.add(str(row.get(key)))
    therapies = row.get("therapies")
    if isinstance(therapies, list):
        drugs.update(str(item) for item in therapies if item)
    elif isinstance(therapies, str):
        drugs.update(part.strip() for part in re.split(r";|,", therapies) if part.strip() and part.strip().lower() != "none")
    for concept in row.get("mapped_concepts") or []:
        if not isinstance(concept, dict):
            continue
        typ = norm(concept.get("concept_type") or concept.get("role"))
        name = concept.get("normalized_name") or concept.get("term")
        if "gene" in typ:
            sym = clean_symbol(name)
            if sym:
                genes.add(sym)
        if "disease" in typ and name:
            diseases.add(str(name))
    for phenotype in row.get("phenotypes") or []:
        if isinstance(phenotype, dict):
            phenotypes.append(phenotype.get("hpo_name") or phenotype.get("hpo_id"))
        else:
            phenotypes.append(phenotype)
    pull_from_evidence(row, genes, drugs, diseases, disease_ids)
    candidate_type = row.get("hypothesis_class") or row.get("candidate_type") or row.get("source_class") or row.get("type") or "generated_candidate"
    return {
        "schema_version": 1,
        "candidate_id": str(candidate_id),
        "source_label": label,
        "input_path": str(path),
        "input_sha256": source_sha,
        "input_row_index": index,
        "candidate_type": str(candidate_type),
        "domain": row.get("domain") or label,
        "genes": sorted(genes),
        "drugs": sorted({d for d in drugs if d and norm(d) not in {"none", "null"}}),
        "diseases": sorted({d for d in diseases if d and norm(d) not in {"none", "null"}}),
        "disease_ids": sorted(disease_ids),
        "phenotypes": sorted({str(p) for p in phenotypes if p}),
        "existing_falsification_status": row.get("falsification_status"),
        "existing_falsification_score": row.get("falsification_score"),
        "rank_score": row.get("rank_score") or row.get("score"),
        "safety_trial_flags": row.get("safety_trial_flags") or [],
        "derived_status": row.get("derived_status"),
        "clinical_boundary": row.get("clinical_boundary") or CLINICAL_BOUNDARY,
        "raw_candidate": row,
    }


def pull_from_evidence(
    row: dict[str, Any],
    genes: set[str],
    drugs: set[str],
    diseases: set[str],
    disease_ids: set[str],
) -> None:
    for path in row.get("evidence_paths") or []:
        if not isinstance(path, dict):
            continue
        inner = path.get("row") if isinstance(path.get("row"), dict) else path
        for key in ["target_symbol", "target_name", "query_target_symbol", "gene"]:
            sym = clean_symbol(inner.get(key))
            if sym:
                genes.add(sym)
        for key in ["drug", "molecule_pref_name"]:
            if inner.get(key):
                drugs.add(str(inner.get(key)))
        for key in ["disease_name"]:
            if inner.get(key):
                diseases.add(str(inner.get(key)))
        for key in ["disease_id"]:
            if inner.get(key):
                disease_ids.add(str(inner.get(key)))
    ext = row.get("external_validation")
    if isinstance(ext, dict):
        for ot in ext.get("open_targets") or ext.get("open_targets_rows") or []:
            if isinstance(ot, dict):
                sym = clean_symbol(ot.get("target_name") or ot.get("query_target_symbol"))
                if sym:
                    genes.add(sym)
                if ot.get("disease_name"):
                    diseases.add(str(ot.get("disease_name")))
                if ot.get("disease_id"):
                    disease_ids.add(str(ot.get("disease_id")))
        for dg in ext.get("dgidb") or []:
            if isinstance(dg, dict):
                sym = clean_symbol(dg.get("gene") or dg.get("target_symbol"))
                if sym:
                    genes.add(sym)
                if dg.get("drug"):
                    drugs.add(str(dg.get("drug")))


def source_manifest(paths: dict[str, Path]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for label, path in paths.items():
        if path.exists():
            out[label] = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
        else:
            out[label] = {"path": str(path), "missing": True}
    return out


def build_source_indexes(paths: dict[str, Path]) -> dict[str, Any]:
    indexes: dict[str, Any] = {
        "dgidb": defaultdict(list),
        "open_targets": defaultdict(list),
        "trials": defaultdict(list),
        "safety": defaultdict(list),
        "pubtator_support": [],
        "pubtator_counter": [],
    }
    for label in ["dgidb_seed", "dgidb_broad", "issue1224_dgidb", "issue1189_dgidb"]:
        path = paths[label]
        for idx, row in enumerate(load_jsonl(path), start=1):
            drug = norm(row.get("drug") or row.get("drug_name"))
            gene = clean_symbol(row.get("gene") or row.get("target_symbol") or row.get("gene_name"))
            if drug and gene:
                indexes["dgidb"][(drug, gene)].append((label, path, idx, row))
    for label in ["open_targets_rows", "open_targets_edges", "issue1224_open_targets"]:
        path = paths[label]
        for idx, row in enumerate(load_jsonl(path), start=1):
            gene = clean_symbol(row.get("target_name") or row.get("query_target_symbol") or row.get("target_symbol"))
            disease_keys = [norm(row.get("disease_name")), norm(row.get("disease_id"))]
            for key in disease_keys:
                if gene and key:
                    indexes["open_targets"][(gene, key)].append((label, path, idx, row))
    for label in ["clinicaltrials_rows", "clinicaltrials_summaries"]:
        path = paths[label]
        for idx, row in enumerate(load_jsonl(path), start=1):
            drug_values = [row.get("query_intervention"), row.get("matched_intervention_alias"), row.get("matched_intervention_value"), row.get("intervention")]
            disease_values = [row.get("query_condition"), row.get("matched_condition_alias"), row.get("matched_condition_value"), row.get("condition")]
            for drug in lower_set(drug_values):
                for disease in lower_set(disease_values):
                    indexes["trials"][(drug, disease)].append((label, path, idx, row))
    for label in ["safety_terms", "candidate_safety_flags"]:
        path = paths[label]
        for idx, row in enumerate(load_jsonl(path), start=1):
            drugs = [row.get("drug_term"), row.get("drug"), row.get("therapy")]
            if isinstance(row.get("therapies"), list):
                drugs.extend(row.get("therapies"))
            for drug in lower_set(drugs):
                indexes["safety"][drug].append((label, path, idx, row))
    for label, target in [("pubtator_support", "pubtator_support"), ("pubtator_negative", "pubtator_counter")]:
        path = paths[label]
        for idx, row in enumerate(load_jsonl(path), start=1):
            endpoints = pubtator_terms(row)
            indexes[target].append((label, path, idx, row, endpoints))
    return indexes


def pubtator_terms(row: dict[str, Any]) -> set[str]:
    terms = {norm(row.get("left_id")), norm(row.get("right_id"))}
    for annotation in row.get("annotations") or []:
        if isinstance(annotation, dict):
            terms.add(norm(annotation.get("name")))
            terms.add(norm(annotation.get("accession")))
    for relation in row.get("relations") or []:
        for role in relation.get("roles") or []:
            terms.add(norm(role.get("name")))
            terms.add(norm(role.get("accession")))
    return {term for term in terms if term}


def evidence_row(
    candidate: dict[str, Any],
    kind: str,
    source_system: str,
    reason_code: str,
    source_path: Path | str,
    source_sha256: str,
    source_row_index: int,
    weight: float,
    summary: str,
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "candidate_id": candidate["candidate_id"],
        "evidence_kind": kind,
        "source_system": source_system,
        "reason_code": reason_code,
        "source_path": str(source_path),
        "source_sha256": source_sha256,
        "source_row_index": source_row_index,
        "weight": round(float(weight), 6),
        "summary": summary[:700],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def evaluate_candidate(candidate: dict[str, Any], indexes: dict[str, Any], source_hash: dict[str, dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    support: list[dict[str, Any]] = []
    counter: list[dict[str, Any]] = []
    support.append(
        evidence_row(
            candidate,
            "support",
            "generated_candidate_source",
            "candidate_has_persisted_source_row",
            candidate["input_path"],
            candidate["input_sha256"],
            candidate["input_row_index"],
            0.25,
            f"{candidate['source_label']} generated candidate row persisted before falsification sweep",
        )
    )
    add_open_targets(candidate, indexes, support, counter)
    add_dgidb(candidate, indexes, support, counter)
    add_trials(candidate, indexes, support, counter)
    add_safety(candidate, indexes, support, counter)
    add_pubtator(candidate, indexes, support, counter)
    add_intrinsic_candidate_flags(candidate, support, counter)
    return support, counter


def add_open_targets(candidate: dict[str, Any], indexes: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    for gene in candidate["genes"]:
        rows = []
        for disease in candidate["diseases"] + candidate["disease_ids"]:
            rows.extend(indexes["open_targets"].get((gene, norm(disease)), [])[:3])
        for label, path, idx, row in rows[:6]:
            score = float(row.get("score") or 0.0)
            kind = "support" if score >= 0.05 else "counter"
            reason = "open_targets_same_target_disease_support" if kind == "support" else "open_targets_low_score_or_weak_pair"
            target = support if kind == "support" else counter
            target.append(
                evidence_row(
                    candidate,
                    kind,
                    "open_targets",
                    reason,
                    path,
                    sha256_path(path),
                    idx,
                    min(max(score, 0.05), 1.0) if kind == "support" else 0.5,
                    f"Open Targets {row.get('target_name')} / {row.get('disease_name')} score {score}",
                )
            )


def add_dgidb(candidate: dict[str, Any], indexes: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    if not candidate["drugs"] or not candidate["genes"]:
        return
    matched = False
    for drug in candidate["drugs"]:
        for gene in candidate["genes"]:
            rows = indexes["dgidb"].get((norm(drug), gene), [])
            for label, path, idx, row in rows[:4]:
                matched = True
                score = float(row.get("interaction_score") or 0.0)
                evidence_score = float(row.get("evidence_score") or 0.0)
                support.append(
                    evidence_row(
                        candidate,
                        "support",
                        "dgidb",
                        "dgidb_exact_drug_gene_interaction",
                        path,
                        sha256_path(path),
                        idx,
                        1.0 + min(score, 2.0) + min(evidence_score, 5.0) * 0.1,
                        f"DGIdb exact drug/gene row {drug} / {gene}; interaction_score={score}; evidence_score={evidence_score}",
                    )
                )
    if not matched:
        counter.append(
            evidence_row(
                candidate,
                "counter",
                "dgidb",
                "dgidb_exact_drug_gene_missing_current_sources",
                candidate["input_path"],
                candidate["input_sha256"],
                candidate["input_row_index"],
                0.35,
                "Drug-bearing candidate has no exact drug/gene DGIdb row in current persisted/live source set",
            )
        )


def add_trials(candidate: dict[str, Any], indexes: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    if not candidate["drugs"] or not candidate["diseases"]:
        return
    matched = False
    for drug in candidate["drugs"]:
        for disease in candidate["diseases"]:
            rows = indexes["trials"].get((norm(drug), norm(disease)), [])
            for label, path, idx, row in rows[:4]:
                matched = True
                status = str(row.get("overall_status") or "")
                has_results = bool(row.get("has_results"))
                if status in {"TERMINATED", "WITHDRAWN", "SUSPENDED"}:
                    counter.append(
                        evidence_row(candidate, "counter", "clinicaltrials", "clinicaltrials_stopped_trial", path, sha256_path(path), idx, 1.0, f"{status} trial for {drug} / {disease}")
                    )
                else:
                    support.append(
                        evidence_row(candidate, "support", "clinicaltrials", "clinicaltrials_registry_or_completed_context", path, sha256_path(path), idx, 0.5 + (0.5 if has_results else 0.0), f"ClinicalTrials context {status or 'summary'} for {drug} / {disease}")
                    )
    if not matched:
        counter.append(
            evidence_row(candidate, "counter", "clinicaltrials", "trial_source_missing_for_drug_disease", candidate["input_path"], candidate["input_sha256"], candidate["input_row_index"], 0.75, "No exact persisted ClinicalTrials drug/disease row for this generated drug-bearing candidate")
        )


def add_safety(candidate: dict[str, Any], indexes: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    if not candidate["drugs"]:
        return
    matched = False
    for drug in candidate["drugs"]:
        for label, path, idx, row in indexes["safety"].get(norm(drug), [])[:4]:
            matched = True
            flags = row.get("ranker_blocks") or row.get("flags") or row.get("representative_flags") or []
            text = json.dumps(flags).lower()
            if "unavailable" in text or "boxed" in text or "contraindication" in text or "serious" in text or "death" in text or "high-risk" in text:
                counter.append(
                    evidence_row(candidate, "counter", "openfda_safety", "safety_block_or_high_risk_label", path, sha256_path(path), idx, 1.25, f"Safety triage flags for {drug}: {flags}")
                )
            else:
                support.append(
                    evidence_row(candidate, "support", "openfda_safety", "safety_source_present_no_block_flag", path, sha256_path(path), idx, 0.2, f"Safety source present for {drug}")
                )
    if not matched:
        counter.append(
            evidence_row(candidate, "counter", "openfda_safety", "safety_source_missing_fail_closed", candidate["input_path"], candidate["input_sha256"], candidate["input_row_index"], 1.25, "No persisted safety/adverse-event triage row for this drug-bearing candidate")
        )


def add_pubtator(candidate: dict[str, Any], indexes: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    terms = lower_set(candidate["genes"] + candidate["drugs"] + candidate["diseases"])
    if len(terms) < 2:
        return
    for label, path, idx, row, endpoints in indexes["pubtator_support"]:
        if len(terms & endpoints) >= 2:
            support.append(
                evidence_row(candidate, "support", "pubtator", "pubtator_relation_or_comention_support", path, sha256_path(path), idx, 0.8, f"PubTator row overlaps candidate endpoints PMID {row.get('pmid')}")
            )
    for label, path, idx, row, endpoints in indexes["pubtator_counter"]:
        if len(terms & endpoints) >= 2:
            counter.append(
                evidence_row(candidate, "counter", "pubtator", "pubtator_negative_text_signal", path, sha256_path(path), idx, 2.5, f"PubTator negative signal {row.get('negative_signal_match')} PMID {row.get('pmid')}")
            )


def add_intrinsic_candidate_flags(candidate: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> None:
    status = str(candidate.get("existing_falsification_status") or "")
    if status.startswith("complete_counterevidence_found"):
        counter.append(
            evidence_row(candidate, "counter", "candidate_existing_flag", "existing_counterevidence_status", candidate["input_path"], candidate["input_sha256"], candidate["input_row_index"], 1.0, status)
        )
    elif status.startswith("complete_no_counterevidence"):
        support.append(
            evidence_row(candidate, "support", "candidate_existing_flag", "existing_sweep_no_counterevidence", candidate["input_path"], candidate["input_sha256"], candidate["input_row_index"], 0.4, status)
        )
    for flag in candidate.get("safety_trial_flags") or []:
        text = str(flag).lower()
        if "pending" in text or "missing" in text or "gap" in text or "block" in text or "unavailable" in text:
            counter.append(
                evidence_row(candidate, "counter", "candidate_embedded_safety_trial_flag", "embedded_safety_or_trial_gap", candidate["input_path"], candidate["input_sha256"], candidate["input_row_index"], 0.75, str(flag))
            )


def flag_candidate(candidate: dict[str, Any], support: list[dict[str, Any]], counter: list[dict[str, Any]]) -> dict[str, Any]:
    support_weight = sum(float(row["weight"]) for row in support)
    counter_weight = sum(float(row["weight"]) for row in counter)
    reason_codes = sorted({row["reason_code"] for row in counter}) or ["no_counter_evidence_found_in_current_sources"]
    if counter:
        if any("missing" in code or "fail_closed" in code for code in reason_codes):
            status = "blocked_missing_required_evidence_or_safety"
        else:
            status = "demoted_counterevidence_found"
    else:
        status = "complete_no_counterevidence_found_in_current_sources"
    score = counter_weight / (counter_weight + support_weight + 1.0)
    return {
        "schema_version": 1,
        "candidate_id": candidate["candidate_id"],
        "source_label": candidate["source_label"],
        "candidate_type": candidate["candidate_type"],
        "domain": candidate["domain"],
        "genes": candidate["genes"],
        "drugs": candidate["drugs"],
        "diseases": candidate["diseases"],
        "support_evidence_count": len(support),
        "counter_evidence_count": len(counter),
        "support_weight": round(support_weight, 6),
        "counter_weight": round(counter_weight, 6),
        "counter_evidence_weight": round(counter_weight, 6),
        "falsification_score": round(score, 6),
        "reason_codes": reason_codes,
        "sweep_status": status,
        "blocked_or_demoted": bool(counter),
        "human_review_atlas_status": "blocked_or_demoted_before_atlas" if counter else "eligible_for_human_review_triage_still_hypothesis",
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def bridge_rows(flags: list[dict[str, Any]], source_path: Path, source_sha: str, limit: int) -> list[dict[str, Any]]:
    rows = []
    for flag in flags[:limit]:
        drug_text = ", ".join(flag["drugs"][:2]) if flag["drugs"] else "no drug"
        gene_text = ", ".join(flag["genes"][:2]) if flag["genes"] else "no gene"
        disease_text = ", ".join(flag["diseases"][:2]) if flag["diseases"] else "no disease"
        status = flag["sweep_status"]
        text = f"Generated candidate falsification flag {flag['candidate_id']}: {status} for {gene_text} {drug_text} {disease_text}."
        terms = [status, gene_text, drug_text, disease_text]
        rows.append(
            {
                "id": flag["candidate_id"],
                "domain": "generated_candidate_falsification",
                "text": text,
                "bridge_terms": sorted({term for term in terms if term and term in text}),
                "metadata": {
                    "source_dataset": "issue1223_generated_candidate_falsification",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "sweep_status": status,
                    "falsification_score": str(flag["falsification_score"]),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def artifact(path: Path, rows: int | None = None) -> dict[str, Any]:
    if not path.exists():
        return {"path": str(path), "bytes": None, "sha256": None, "rows": rows, "missing": True}
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path), "rows": rows}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root")
    parser.add_argument("--bridge-limit", type=int, default=1000)
    args = parser.parse_args()

    root = Path(args.root)
    out = root / "out"
    out.mkdir(parents=True, exist_ok=True)
    input_paths = {key: Path(value) for key, value in DEFAULT_INPUTS.items()}
    source_paths = {key: Path(value) for key, value in DEFAULT_SOURCES.items()}

    candidates, input_manifest = candidate_inputs(input_paths)
    source_hash = source_manifest(source_paths)
    indexes = build_source_indexes(source_paths)

    support_rows: list[dict[str, Any]] = []
    counter_rows: list[dict[str, Any]] = []
    flags: list[dict[str, Any]] = []
    for candidate in sorted(candidates, key=lambda row: row["candidate_id"]):
        support, counter = evaluate_candidate(candidate, indexes, source_hash)
        support_rows.extend(support)
        counter_rows.extend(counter)
        flags.append(flag_candidate(candidate, support, counter))
    flags.sort(key=lambda row: (-row["falsification_score"], row["candidate_id"]))

    files = {
        "input_candidate_manifest": out / "input_candidate_manifest.json",
        "raw_query_manifest": out / "raw_query_manifest.jsonl",
        "normalized_generated_candidates": out / "normalized_generated_candidates.jsonl",
        "support_evidence": out / "support_evidence.jsonl",
        "counter_evidence": out / "counter_evidence.jsonl",
        "candidate_falsification_flags": out / "candidate_falsification_flags.jsonl",
        "top_demoted_candidates": out / "top_demoted_candidates.json",
        "validation_metrics": out / "validation_metrics.json",
        "bridge_corpus_rows": out / "generated_candidate_falsification_bridge_rows.jsonl",
        "output_manifest": out / "output_manifest.json",
    }
    raw_manifest = [
        {
            "source_system": label,
            "source_path": data["path"],
            "source_sha256": data.get("sha256"),
            "bytes": data.get("bytes"),
            "missing": data.get("missing", False),
        }
        for label, data in source_hash.items()
    ]
    metrics = {
        "input_candidate_rows": sum(item["rows"] for item in input_manifest.values()),
        "deduped_candidate_rows": len(candidates),
        "support_evidence_rows": len(support_rows),
        "counter_evidence_rows": len(counter_rows),
        "flag_rows": len(flags),
        "blocked_or_demoted_rows": sum(1 for row in flags if row["blocked_or_demoted"]),
        "rows_with_missing_required_evidence": sum(1 for row in flags if row["sweep_status"] == "blocked_missing_required_evidence_or_safety"),
        "rows_with_hard_counterevidence": sum(1 for row in flags if row["sweep_status"] == "demoted_counterevidence_found"),
        "status_counts": dict(Counter(row["sweep_status"] for row in flags)),
        "source_label_counts": dict(Counter(row["source_label"] for row in candidates)),
        "reason_code_counts": dict(Counter(code for row in flags for code in row["reason_codes"])),
    }

    write_json(files["input_candidate_manifest"], {"schema_version": 1, "inputs": input_manifest, "sources": source_hash})
    write_jsonl(files["raw_query_manifest"], raw_manifest)
    write_jsonl(files["normalized_generated_candidates"], candidates)
    write_jsonl(files["support_evidence"], support_rows)
    write_jsonl(files["counter_evidence"], counter_rows)
    write_jsonl(files["candidate_falsification_flags"], flags)
    write_json(files["top_demoted_candidates"], flags[:50])
    write_json(files["validation_metrics"], metrics)
    bridge = bridge_rows(flags, files["candidate_falsification_flags"], sha256_path(files["candidate_falsification_flags"]), args.bridge_limit)
    write_jsonl(files["bridge_corpus_rows"], bridge)
    manifest = {
        "schema_version": 1,
        "issue": 1223,
        "artifacts": {
            key: artifact(path, line_count(path) if path.suffix == ".jsonl" else None)
            for key, path in files.items()
            if key != "output_manifest"
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    write_json(files["output_manifest"], manifest)
    readback = {
        "schema_version": 1,
        "status": "ok",
        "issue": 1223,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "metrics": metrics,
        "artifacts": {key: artifact(path, line_count(path) if path.suffix == ".jsonl" else None) for key, path in files.items()},
        "assertions": {
            "one_flag_per_candidate": metrics["flag_rows"] == metrics["deduped_candidate_rows"],
            "support_rows_present": metrics["support_evidence_rows"] > 0,
            "counter_rows_present": metrics["counter_evidence_rows"] > 0,
            "raw_manifest_present": len(raw_manifest) > 0,
            "top_demoted_present": bool(flags),
            "clinical_boundary_all_flags": True,
        },
        "top_demoted": [
            {
                "candidate_id": row["candidate_id"],
                "source_label": row["source_label"],
                "status": row["sweep_status"],
                "score": row["falsification_score"],
                "reason_codes": row["reason_codes"][:6],
                "genes": row["genes"][:3],
                "drugs": row["drugs"][:3],
                "diseases": row["diseases"][:3],
            }
            for row in flags[:12]
        ],
    }
    readback_path = out / "persisted_readback.json"
    write_json(readback_path, readback)
    print(json.dumps({"status": "ok", "root": str(root), "metrics": metrics}, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
