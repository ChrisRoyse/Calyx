#!/usr/bin/env python3
"""#1247 endpoint/outcome review for #1244 Europe PMC relation rollups.

This stage reads sealed #1244 relation-validation artifacts and reviews the
rollups whose bounded source text had relation context without safety/counter
language. It separates endpoint/outcome, pharmacokinetic/exposure,
mechanistic, trial-design, and effect-size language while keeping every row as
blocked research triage. It is not efficacy, safety, treatment guidance,
dosing guidance, recommendation, clinical actionability, or cure evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Europe PMC endpoint/outcome review is literature triage only; not efficacy, "
    "safety, treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "europepmc_endpoint_outcome_review_not_clinical_actionability"
)

ISSUE1244_ROOT = "/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z"

DEFAULT_INPUTS = {
    "issue1244_candidate_rollup": f"{ISSUE1244_ROOT}/out/candidate_europepmc_relation_rollup.jsonl",
    "issue1244_relation_validation": f"{ISSUE1244_ROOT}/out/europepmc_source_text_relation_validation.jsonl",
    "issue1244_persisted_readback": f"{ISSUE1244_ROOT}/out/persisted_readback.json",
    "issue1244_calyx_readback": f"{ISSUE1244_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1244_output_manifest": f"{ISSUE1244_ROOT}/out/output_manifest.json",
}

INPUT_STATUS = "source_text_relation_extracted_still_blocked"
REVIEW_STATUS = "endpoint_outcome_review_complete_still_blocked"
VALIDATION_STATUS = "endpoint_outcome_not_validated_still_blocked"
PROMOTION_STATUS = (
    "blocked_requires_independent_endpoint_outcome_safety_falsification_and_human_review"
)

STATUS_VALUES = {
    "clinical_endpoint_language_review",
    "preclinical_or_cell_endpoint_language_review",
    "endpoint_or_outcome_language_review",
    "pharmacokinetic_or_exposure_endpoint_review",
    "combination_or_coexposure_context_review",
    "mechanistic_endpoint_context_review",
    "study_design_without_endpoint_magnitude_review",
    "relation_context_without_endpoint_language_review",
    "endpoint_outcome_unclear_fail_closed",
}

HUMAN_MODEL_SYSTEMS = {
    "human_clinical_or_patient",
    "human_healthy_volunteer",
    "review_or_guideline",
}

PRECLINICAL_MODEL_SYSTEMS = {
    "animal_or_xenograft",
    "in_vitro_or_cell_system",
    "computational_or_in_silico",
}

CLINICAL_ENDPOINT_PATTERNS = [
    r"\boverall survival\b",
    r"\bprogression[- ]free survival\b",
    r"\bevent[- ]free survival\b",
    r"\bdisease[- ]free survival\b",
    r"\brelapse[- ]free survival\b",
    r"\bsurvival\b",
    r"\bremission\b",
    r"\bresponse rate\b",
    r"\bobjective response\b",
    r"\bcomplete response\b",
    r"\bpartial response\b",
    r"\btumou?r response\b",
    r"\bclinical response\b",
    r"\bclinical benefit\b",
    r"\bprogression\b",
    r"\brelapse\b",
    r"\brecurrence\b",
    r"\bendpoints?\b",
    r"\boutcomes?\b",
    r"\befficacy\b",
    r"\beffective(?:ness)?\b",
    r"\bbenefit\b",
    r"\bimprov(?:e|ed|ement|ing)?\b",
    r"\bdecreas(?:e|ed|ing)\b",
    r"\breduc(?:e|ed|tion|ing)\b",
    r"\bincreas(?:e|ed|ing)\b",
]

PHARMACOKINETIC_PATTERNS = [
    r"\bpharmacokinetic",
    r"\bpharmacodynamic",
    r"\bexposure\b",
    r"\bbioavailability\b",
    r"\bplasma concentration",
    r"\bserum concentration",
    r"\bconcentration\b",
    r"\bclearance\b",
    r"\bhalf[- ]life\b",
    r"\bt1/2\b",
    r"\bcmax\b",
    r"\btmax\b",
    r"\bauc(?:0|inf|tau|last)?\b",
    r"\bmetabolism\b",
    r"\bmetabolite",
    r"\babsorption\b",
    r"\bdistribution\b",
    r"\belimination\b",
]

MECHANISTIC_PATTERNS = [
    r"\bdrug[- ]drug interaction\b",
    r"\binteract",
    r"\bsynerg",
    r"\bantagon",
    r"\bpotentiat",
    r"\binhibit",
    r"\binduc",
    r"\bactivate",
    r"\bblock",
    r"\bbinding\b",
    r"\bbinder\b",
    r"\breceptor\b",
    r"\btarget\b",
    r"\bpathway\b",
    r"\benzyme\b",
    r"\bsubstrate\b",
    r"\btransporter\b",
]

TRIAL_DESIGN_PATTERNS = [
    r"\bclinical trial\b",
    r"\brandomi[sz]ed\b",
    r"\bdouble[- ]blind\b",
    r"\bsingle[- ]blind\b",
    r"\bplacebo\b",
    r"\bcohort\b",
    r"\bcase[- ]control\b",
    r"\bpatients?\b",
    r"\bhuman(?:s)?\b",
    r"\bphase\s+(?:i|ii|iii|iv|1|2|3|4)\b",
    r"\btrial\b",
    r"\bstudy\b",
]

PRECLINICAL_PATTERNS = [
    r"\bin vitro\b",
    r"\bcell(?:s| line| culture)?\b",
    r"\bcellular\b",
    r"\banimals?\b",
    r"\bmice\b",
    r"\bmouse\b",
    r"\brats?\b",
    r"\bxenograft",
    r"\bin vivo\b",
    r"\bin silico\b",
    r"\bcomputational\b",
    r"\bsimulation\b",
]

COMBINATION_PATTERNS = [
    r"\bcombination\b",
    r"\bcombined\b",
    r"\bcoadministr",
    r"\bco-admin",
    r"\bconcomitant",
    r"\btogether\b",
    r"\bplus\b",
    r"\bfollowed by\b",
    r"\badjunct",
    r"\badd[- ]on\b",
]

EFFECT_SIZE_PATTERNS = [
    r"\bhazard ratio\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\bodds ratio\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\brelative risk\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\bp\s*(?:=|<|>|<=|>=)\s*0?\.\d+",
    r"\b95%\s*CI\b",
    r"\bconfidence interval\b",
    r"\b\d+(?:\.\d+)?\s*%",
    r"\bAUC\b\s*(?:=|:)?\s*\d+(?:\.\d+)?",
]

DOSE_EXPOSURE_PATTERNS = [
    r"\b\d+(?:\.\d+)?\s*(?:mg|mcg|ug|g|kg|ml|l|iu|units?|mmol|umol|nmol|mol|%)\b",
    r"\b\d+(?:\.\d+)?\s*(?:mg|mcg|ug|g|iu|units?)/(?:kg|m2|day|d|h|hr|hour)\b",
    r"\b(?:single|multiple|high|low|daily|weekly|monthly)[- ]dose\b",
    r"\bdose[- ]response\b",
    r"\bdose(?:s|d)?\b",
    r"\bcoadministr",
    r"\bconcomitant\b",
]

CATEGORY_PRIORITY = [
    "clinical_endpoint_language_review",
    "preclinical_or_cell_endpoint_language_review",
    "endpoint_or_outcome_language_review",
    "pharmacokinetic_or_exposure_endpoint_review",
    "combination_or_coexposure_context_review",
    "mechanistic_endpoint_context_review",
    "study_design_without_endpoint_magnitude_review",
    "relation_context_without_endpoint_language_review",
    "endpoint_outcome_unclear_fail_closed",
]


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
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
    if isinstance(value, list):
        value = " ".join(clean_text(item) for item in value)
    return " ".join(str(value).replace("\x00", " ").split())


def uniq(values: list[object]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for value in values:
        text = clean_text(value)
        if not text:
            continue
        key = text.lower()
        if key in seen:
            continue
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


def read_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=True) + "\n")


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    value = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            value["rows"] = sum(1 for line in handle if line.strip())
    return value


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [name for name, value in inputs.items() if not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(assertions.values())


def pattern_hits(patterns: list[str], text: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for pattern in patterns:
        for match in re.finditer(pattern, text, flags=re.IGNORECASE):
            out.append(
                {
                    "pattern": pattern,
                    "match": clean_text(match.group(0)),
                    "start": match.start(),
                    "end": match.end(),
                }
            )
    return out


def term_list(row: dict[str, Any], key: str) -> list[str]:
    values = row.get("relation_terms", {}).get(key, [])
    return uniq(values if isinstance(values, list) else [])


def rollup_term_list(row: dict[str, Any], key: str) -> list[str]:
    values = row.get(key, [])
    return uniq(values if isinstance(values, list) else [])


def classify_endpoint(row: dict[str, Any]) -> dict[str, Any]:
    text = clean_text(row.get("source_text_window"))
    clinical = pattern_hits(CLINICAL_ENDPOINT_PATTERNS, text)
    pk = pattern_hits(PHARMACOKINETIC_PATTERNS, text)
    mechanistic = pattern_hits(MECHANISTIC_PATTERNS, text)
    trial = pattern_hits(TRIAL_DESIGN_PATTERNS, text)
    preclinical = pattern_hits(PRECLINICAL_PATTERNS, text)
    combo = pattern_hits(COMBINATION_PATTERNS, text)
    effect = pattern_hits(EFFECT_SIZE_PATTERNS, text)
    dose = pattern_hits(DOSE_EXPOSURE_PATTERNS, text)

    issue1244_outcome_terms = term_list(row, "trial_or_outcome_context")
    clinical_terms = uniq(issue1244_outcome_terms + [hit["match"] for hit in clinical])
    pk_terms = uniq([hit["match"] for hit in pk])
    mechanistic_terms = uniq(term_list(row, "mechanistic_or_interaction_context") + [hit["match"] for hit in mechanistic])
    trial_terms = uniq([hit["match"] for hit in trial])
    preclinical_terms = uniq([hit["match"] for hit in preclinical])
    combination_terms = uniq(term_list(row, "combination_or_coexposure") + [hit["match"] for hit in combo])
    effect_terms = uniq([hit["match"] for hit in effect])
    dose_terms = uniq(list(row.get("dose_exposure_terms") or []) + [hit["match"] for hit in dose])

    model = row.get("primary_model_system", "unclear")
    human_context = model in HUMAN_MODEL_SYSTEMS or bool(trial_terms)
    preclinical_context = model in PRECLINICAL_MODEL_SYSTEMS or bool(preclinical_terms)

    has_explicit_endpoint = bool(clinical)
    if has_explicit_endpoint and human_context:
        category = "clinical_endpoint_language_review"
    elif has_explicit_endpoint and preclinical_context:
        category = "preclinical_or_cell_endpoint_language_review"
    elif clinical_terms:
        category = "endpoint_or_outcome_language_review"
    elif pk_terms or dose_terms:
        category = "pharmacokinetic_or_exposure_endpoint_review"
    elif combination_terms:
        category = "combination_or_coexposure_context_review"
    elif mechanistic_terms:
        category = "mechanistic_endpoint_context_review"
    elif trial_terms:
        category = "study_design_without_endpoint_magnitude_review"
    elif text:
        category = "relation_context_without_endpoint_language_review"
    else:
        category = "endpoint_outcome_unclear_fail_closed"

    return {
        "endpoint_outcome_review_category": category,
        "clinical_endpoint_terms": clinical_terms,
        "pharmacokinetic_exposure_terms": pk_terms,
        "mechanistic_terms": mechanistic_terms,
        "trial_design_terms": trial_terms,
        "preclinical_terms": preclinical_terms,
        "combination_terms": combination_terms,
        "effect_size_terms": effect_terms,
        "dose_exposure_terms": dose_terms,
        "clinical_endpoint_spans": clinical[:10],
        "pharmacokinetic_exposure_spans": pk[:10],
        "mechanistic_spans": mechanistic[:10],
        "trial_design_spans": trial[:10],
        "preclinical_spans": preclinical[:10],
        "combination_spans": combo[:10],
        "effect_size_spans": effect[:10],
        "dose_exposure_spans": dose[:10],
        "has_endpoint_or_outcome_language": bool(clinical_terms),
        "has_pharmacokinetic_or_exposure_language": bool(pk_terms or dose_terms),
        "has_mechanistic_language": bool(mechanistic_terms),
        "has_trial_design_language": bool(trial_terms),
        "has_preclinical_language": bool(preclinical_terms),
        "has_combination_or_coexposure_language": bool(combination_terms),
        "has_effect_size_language": bool(effect_terms),
    }


def reason_codes(review: dict[str, Any]) -> list[str]:
    codes = [
        "endpoint_outcome_review_not_clinical_actionability",
        "requires_independent_endpoint_outcome_validation",
        "requires_safety_falsification_and_human_review",
    ]
    category = review["endpoint_outcome_review_category"]
    if category == "clinical_endpoint_language_review":
        codes.append("clinical_endpoint_language_present_not_validated")
    elif category == "preclinical_or_cell_endpoint_language_review":
        codes.append("preclinical_endpoint_language_present_not_human_efficacy")
    elif category == "endpoint_or_outcome_language_review":
        codes.append("endpoint_or_outcome_language_present_context_unclear")
    elif category == "pharmacokinetic_or_exposure_endpoint_review":
        codes.append("pk_or_exposure_language_present_not_efficacy")
    elif category == "combination_or_coexposure_context_review":
        codes.append("combination_or_coexposure_language_present_not_validated_outcome")
    elif category == "mechanistic_endpoint_context_review":
        codes.append("mechanistic_language_present_not_endpoint_outcome")
    elif category == "study_design_without_endpoint_magnitude_review":
        codes.append("study_design_language_present_without_validated_endpoint")
    else:
        codes.append("endpoint_language_absent_or_unclear")
    if review.get("effect_size_terms"):
        codes.append("effect_size_language_present_but_not_validated")
    if review.get("dose_exposure_terms"):
        codes.append("dose_or_exposure_language_present_not_dosing_guidance")
    return codes


def build_evidence_reviews(validation_rows: list[dict[str, Any]], scoped_pair_keys: set[str]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in validation_rows:
        if row["pair_key"] not in scoped_pair_keys:
            continue
        if row["source_text_validation_status"] != INPUT_STATUS:
            continue
        review = classify_endpoint(row)
        review_id = "europepmc-endpoint-outcome-evidence:" + stable_id(
            row["validation_id"], review["endpoint_outcome_review_category"]
        )
        out = {
            "schema_version": 1,
            "review_id": review_id,
            "source_validation_id": row["validation_id"],
            "source_evidence_id": row["source_evidence_id"],
            "pair_key": row["pair_key"],
            "representative_pair_id": row["representative_pair_id"],
            "drug_a": row["drug_a"],
            "drug_b": row["drug_b"],
            "source": row["source"],
            "source_id": row["source_id"],
            "source_url": row.get("source_url", ""),
            "source_text_sha256": row["source_text_sha256"],
            "source_text_window": row["source_text_window"],
            "source_text_window_distance_chars": row["source_text_window_distance_chars"],
            "source_relation_class": row["best_relation_class"],
            "relation_direction": row.get("relation_direction", "undirected_or_unclear"),
            "primary_model_system": row["primary_model_system"],
            "review_status": REVIEW_STATUS,
            "endpoint_validation_status": VALIDATION_STATUS,
            "promotion_status": PROMOTION_STATUS,
            "has_safety_language": row["has_safety_language"],
            "has_counter_or_negative_language": row["has_counter_or_negative_language"],
            "has_outcome_language_from_issue1244": row["has_outcome_language"],
            "has_dose_or_exposure_language_from_issue1244": row["has_dose_or_exposure_language"],
            **review,
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = reason_codes(out)
        out["next_validation_experiment"] = (
            "Validate endpoint/outcome independently against structured trial, label, or source-text "
            "evidence with effect direction, magnitude, comparator, cohort/model, safety/falsification, "
            "and human review; keep blocked until those gates clear."
        )
        rows.append(out)
    rows.sort(key=lambda row: (row["pair_key"], row["source_id"], row["review_id"]))
    return rows


def choose_rollup_category(evidence: list[dict[str, Any]]) -> str:
    if not evidence:
        return "endpoint_outcome_unclear_fail_closed"
    counts = Counter(row["endpoint_outcome_review_category"] for row in evidence)
    for category in CATEGORY_PRIORITY:
        if counts.get(category, 0):
            return category
    return "endpoint_outcome_unclear_fail_closed"


def build_rollup_reviews(scoped_rollups: list[dict[str, Any]], evidence_reviews: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_reviews:
        by_pair[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for rollup in scoped_rollups:
        evidence = by_pair.get(rollup["pair_key"], [])
        top_category = choose_rollup_category(evidence)
        category_counts = Counter(row["endpoint_outcome_review_category"] for row in evidence)
        model_counts = Counter(row["primary_model_system"] for row in evidence)
        relation_counts = Counter(row["source_relation_class"] for row in evidence)
        out = {
            "schema_version": 1,
            "rollup_review_id": "europepmc-endpoint-outcome-rollup:" + stable_id(rollup["rollup_id"], top_category),
            "source_issue1244_rollup_id": rollup["rollup_id"],
            "pair_id": rollup["pair_id"],
            "pair_key": rollup["pair_key"],
            "drug_a": rollup["drug_a"],
            "drug_b": rollup["drug_b"],
            "source_text_validation_status": rollup["source_text_validation_status"],
            "source_issue1244_best_relation_class": rollup["best_relation_class"],
            "endpoint_outcome_review_category": top_category,
            "review_status": REVIEW_STATUS,
            "endpoint_validation_status": VALIDATION_STATUS,
            "promotion_status": PROMOTION_STATUS,
            "evidence_review_rows": len(evidence),
            "category_counts": dict(sorted(category_counts.items())),
            "primary_model_system_counts": dict(sorted(model_counts.items())),
            "source_relation_class_counts": dict(sorted(relation_counts.items())),
            "source_ids": uniq([row["source_id"] for row in evidence])[:30],
            "evidence_review_ids": [row["review_id"] for row in evidence[:30]],
            "has_endpoint_or_outcome_language": any(row["has_endpoint_or_outcome_language"] for row in evidence),
            "has_pharmacokinetic_or_exposure_language": any(
                row["has_pharmacokinetic_or_exposure_language"] for row in evidence
            ),
            "has_mechanistic_language": any(row["has_mechanistic_language"] for row in evidence),
            "has_trial_design_language": any(row["has_trial_design_language"] for row in evidence),
            "has_preclinical_language": any(row["has_preclinical_language"] for row in evidence),
            "has_combination_or_coexposure_language": any(
                row["has_combination_or_coexposure_language"] for row in evidence
            ),
            "has_effect_size_language": any(row["has_effect_size_language"] for row in evidence),
            "clinical_endpoint_terms": uniq(
                rollup_term_list(rollup, "outcome_terms")
                + [term for row in evidence for term in row["clinical_endpoint_terms"]]
            )[:30],
            "pharmacokinetic_exposure_terms": uniq(
                [term for row in evidence for term in row["pharmacokinetic_exposure_terms"]]
            )[:30],
            "mechanistic_terms": uniq(
                [term for row in evidence for term in row["mechanistic_terms"]]
            )[:30],
            "trial_design_terms": uniq(
                [term for row in evidence for term in row["trial_design_terms"]]
            )[:30],
            "preclinical_terms": uniq(
                [term for row in evidence for term in row["preclinical_terms"]]
            )[:30],
            "combination_terms": uniq(
                [term for row in evidence for term in row["combination_terms"]]
            )[:30],
            "effect_size_terms": uniq(
                [term for row in evidence for term in row["effect_size_terms"]]
            )[:30],
            "dose_exposure_terms": uniq(
                rollup_term_list(rollup, "dose_exposure_terms")
                + [term for row in evidence for term in row["dose_exposure_terms"]]
            )[:30],
            "requires_independent_endpoint_source_validation": True,
            "requires_safety_falsification": True,
            "requires_human_review": True,
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = reason_codes(out)
        out["next_validation_experiment"] = (
            "Run independent endpoint/outcome validation against structured trial/source evidence; "
            "extract effect direction/magnitude, comparator, cohort/model, and safety/falsification before any promotion."
        )
        rows.append(out)
    rows.sort(
        key=lambda row: (
            CATEGORY_PRIORITY.index(row["endpoint_outcome_review_category"]),
            -row["evidence_review_rows"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rows


def bridge_terms(values: list[object], text: str) -> list[str]:
    return [term for term in uniq(values) if term and clean_text(term) in text]


def build_bridge_rows(
    rollup_reviews: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_reviews:
        text = (
            f"Europe PMC endpoint outcome rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} category {row['endpoint_outcome_review_category']} "
            f"review status {row['review_status']} evidence rows {row['evidence_review_rows']} "
            f"promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_review_id"],
                "domain": "europepmc_endpoint_outcome_rollup",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["pair_id"],
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["endpoint_outcome_review_category"],
                    ],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1247_europepmc_endpoint_outcome_review",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "pair_key": row["pair_key"],
                    "endpoint_outcome_review_category": row["endpoint_outcome_review_category"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence_reviews[:budget]:
        text = (
            f"Europe PMC endpoint outcome evidence {row['review_id']} source {row['source_id']} pair "
            f"{row['pair_key']} {row['drug_a']} plus {row['drug_b']} category "
            f"{row['endpoint_outcome_review_category']} review status {row['review_status']}."
        )
        rows.append(
            {
                "id": row["review_id"],
                "domain": "europepmc_endpoint_outcome_evidence",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["review_id"],
                        row["source_id"],
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["endpoint_outcome_review_category"],
                    ],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1247_europepmc_endpoint_outcome_review",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "source_id": row["source_id"],
                    "pair_key": row["pair_key"],
                    "endpoint_outcome_review_category": row["endpoint_outcome_review_category"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    scoped_rollups: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    rollup_reviews: list[dict[str, Any]],
) -> dict[str, Any]:
    evidence_category_counts = Counter(row["endpoint_outcome_review_category"] for row in evidence_reviews)
    rollup_category_counts = Counter(row["endpoint_outcome_review_category"] for row in rollup_reviews)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "scoped_issue1244_rollups": len(scoped_rollups),
        "evidence_review_rows": len(evidence_reviews),
        "rollup_review_rows": len(rollup_reviews),
        "evidence_category_counts": dict(sorted(evidence_category_counts.items())),
        "rollup_category_counts": dict(sorted(rollup_category_counts.items())),
        "source_issue1244_relation_class_counts": dict(
            sorted(Counter(row["best_relation_class"] for row in scoped_rollups).items())
        ),
        "evidence_primary_model_system_counts": dict(
            sorted(Counter(row["primary_model_system"] for row in evidence_reviews).items())
        ),
        "rollups_with_endpoint_or_outcome_language": sum(
            1 for row in rollup_reviews if row["has_endpoint_or_outcome_language"]
        ),
        "rollups_with_pharmacokinetic_or_exposure_language": sum(
            1 for row in rollup_reviews if row["has_pharmacokinetic_or_exposure_language"]
        ),
        "rollups_with_mechanistic_language": sum(1 for row in rollup_reviews if row["has_mechanistic_language"]),
        "rollups_with_trial_design_language": sum(1 for row in rollup_reviews if row["has_trial_design_language"]),
        "rollups_with_preclinical_language": sum(1 for row in rollup_reviews if row["has_preclinical_language"]),
        "rollups_with_combination_or_coexposure_language": sum(
            1 for row in rollup_reviews if row["has_combination_or_coexposure_language"]
        ),
        "rollups_with_effect_size_language": sum(1 for row in rollup_reviews if row["has_effect_size_language"]),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "category": row["endpoint_outcome_review_category"],
                "evidence_review_rows": row["evidence_review_rows"],
                "source_ids": row["source_ids"][:5],
                "clinical_endpoint_terms": row["clinical_endpoint_terms"][:8],
                "pharmacokinetic_exposure_terms": row["pharmacokinetic_exposure_terms"][:8],
                "mechanistic_terms": row["mechanistic_terms"][:8],
                "effect_size_terms": row["effect_size_terms"][:8],
            }
            for row in rollup_reviews[:25]
        ],
    }


def build_input_manifest(
    inputs: dict[str, str],
    issue1244_rollups: list[dict[str, Any]],
    validation_rows: list[dict[str, Any]],
    issue1244_persisted_readback: dict[str, Any],
    issue1244_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    scoped = [row for row in issue1244_rollups if row.get("source_text_validation_status") == INPUT_STATUS]
    return {
        "schema_version": 1,
        "issue": 1247,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1244_candidate_rollup": artifact(Path(inputs["issue1244_candidate_rollup"]), jsonl=True),
            "issue1244_relation_validation": artifact(Path(inputs["issue1244_relation_validation"]), jsonl=True),
            "issue1244_persisted_readback": artifact(Path(inputs["issue1244_persisted_readback"])),
            "issue1244_calyx_readback": artifact(Path(inputs["issue1244_calyx_readback"])),
            "issue1244_output_manifest": artifact(Path(inputs["issue1244_output_manifest"])),
        },
        "source_contract": {
            "issue1244_persisted_assertions_all_true": all_assertions_true(issue1244_persisted_readback),
            "issue1244_calyx_assertions_all_true": all_assertions_true(issue1244_calyx_readback),
            "issue1244_rollup_rows": len(issue1244_rollups),
            "issue1244_relation_validation_rows": len(validation_rows),
            "scoped_relation_context_rollups": len(scoped),
            "input_filter": f"source_text_validation_status == {INPUT_STATUS}",
        },
    }


def build_readback(
    out_dir: Path,
    scoped_rollups: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    rollup_reviews: list[dict[str, Any]],
    issue1244_persisted_readback: dict[str, Any],
    issue1244_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "europepmc_endpoint_outcome_evidence_review": artifact(
            out_dir / "europepmc_endpoint_outcome_evidence_review.jsonl", jsonl=True
        ),
        "candidate_europepmc_endpoint_outcome_rollup": artifact(
            out_dir / "candidate_europepmc_endpoint_outcome_rollup.jsonl", jsonl=True
        ),
        "europepmc_endpoint_outcome_bridge_rows": artifact(
            out_dir / "europepmc_endpoint_outcome_bridge_rows.jsonl", jsonl=True
        ),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    scoped_pair_ids = {row["pair_id"] for row in scoped_rollups}
    reviewed_pair_ids = {row["pair_id"] for row in rollup_reviews}
    return {
        "schema_version": 1,
        "issue": 1247,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1244_persisted_readback_all_true": all_assertions_true(issue1244_persisted_readback),
            "issue1244_calyx_readback_all_true": all_assertions_true(issue1244_calyx_readback),
            "rollup_review_for_every_scoped_rollup": len(rollup_reviews) == len(scoped_rollups),
            "reviewed_pair_ids_match_scope": reviewed_pair_ids == scoped_pair_ids,
            "all_rollup_reviews_have_evidence": all(row["evidence_review_rows"] > 0 for row in rollup_reviews),
            "all_evidence_reviews_have_source_windows": all(bool(row["source_text_window"]) for row in evidence_reviews),
            "all_evidence_reviews_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence_reviews),
            "all_rollup_reviews_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_reviews),
            "all_evidence_reviews_have_allowed_categories": all(
                row["endpoint_outcome_review_category"] in STATUS_VALUES for row in evidence_reviews
            ),
            "all_rollup_reviews_have_allowed_categories": all(
                row["endpoint_outcome_review_category"] in STATUS_VALUES for row in rollup_reviews
            ),
            "all_evidence_reviews_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_reviews),
            "all_rollup_reviews_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_reviews),
            "bridge_rows_1000_or_less": artifacts["europepmc_endpoint_outcome_bridge_rows"]["rows"] <= 1000,
            "bridge_rows_cover_rollups_plus_budgeted_evidence": artifacts["europepmc_endpoint_outcome_bridge_rows"]["rows"]
            == min(1000, len(rollup_reviews) + len(evidence_reviews)),
        },
        "row_counts": {
            "scoped_rollups": len(scoped_rollups),
            "evidence_review_rows": len(evidence_reviews),
            "rollup_review_rows": len(rollup_reviews),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_rollups: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    issue1244_rollups = rows_jsonl(Path(inputs["issue1244_candidate_rollup"]))
    validation_rows = rows_jsonl(Path(inputs["issue1244_relation_validation"]))
    issue1244_persisted_readback = read_json(Path(inputs["issue1244_persisted_readback"]))
    issue1244_calyx_readback = read_json(Path(inputs["issue1244_calyx_readback"]))
    scoped_rollups = [row for row in issue1244_rollups if row.get("source_text_validation_status") == INPUT_STATUS]
    scoped_rollups.sort(key=lambda row: (row["pair_key"], row["pair_id"]))
    if max_rollups is not None:
        scoped_rollups = scoped_rollups[:max_rollups]
    scoped_pair_keys = {row["pair_key"] for row in scoped_rollups}

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            issue1244_rollups,
            validation_rows,
            issue1244_persisted_readback,
            issue1244_calyx_readback,
        ),
    )
    evidence_reviews = build_evidence_reviews(validation_rows, scoped_pair_keys)
    write_jsonl(out_dir / "europepmc_endpoint_outcome_evidence_review.jsonl", evidence_reviews)
    rollup_reviews = build_rollup_reviews(scoped_rollups, evidence_reviews)
    write_jsonl(out_dir / "candidate_europepmc_endpoint_outcome_rollup.jsonl", rollup_reviews)
    source_path = out_dir / "candidate_europepmc_endpoint_outcome_rollup.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_reviews, evidence_reviews, source_path, source_sha)
    write_jsonl(out_dir / "europepmc_endpoint_outcome_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(scoped_rollups, evidence_reviews, rollup_reviews)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1247,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "europepmc_endpoint_outcome_evidence_review": artifact(
                out_dir / "europepmc_endpoint_outcome_evidence_review.jsonl", jsonl=True
            ),
            "candidate_europepmc_endpoint_outcome_rollup": artifact(
                out_dir / "candidate_europepmc_endpoint_outcome_rollup.jsonl", jsonl=True
            ),
            "europepmc_endpoint_outcome_bridge_rows": artifact(
                out_dir / "europepmc_endpoint_outcome_bridge_rows.jsonl", jsonl=True
            ),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        scoped_rollups,
        evidence_reviews,
        rollup_reviews,
        issue1244_persisted_readback,
        issue1244_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "europepmc_endpoint_outcome_evidence_review": output_manifest["artifacts"][
                "europepmc_endpoint_outcome_evidence_review"
            ],
            "candidate_europepmc_endpoint_outcome_rollup": output_manifest["artifacts"][
                "candidate_europepmc_endpoint_outcome_rollup"
            ],
            "bridge_rows": output_manifest["artifacts"]["europepmc_endpoint_outcome_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1244-candidate-rollup")
    parser.add_argument("--issue1244-relation-validation")
    parser.add_argument("--issue1244-persisted-readback")
    parser.add_argument("--issue1244-calyx-readback")
    parser.add_argument("--issue1244-output-manifest")
    parser.add_argument("--max-rollups", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1244_candidate_rollup", "issue1244_candidate_rollup"),
        ("issue1244_relation_validation", "issue1244_relation_validation"),
        ("issue1244_persisted_readback", "issue1244_persisted_readback"),
        ("issue1244_calyx_readback", "issue1244_calyx_readback"),
        ("issue1244_output_manifest", "issue1244_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_rollups=args.max_rollups)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
