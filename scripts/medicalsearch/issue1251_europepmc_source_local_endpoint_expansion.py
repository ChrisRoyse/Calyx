#!/usr/bin/env python3
"""#1251 source-local endpoint expansion after ClinicalTrials.gov no-hit.

This stage reads sealed #1250 blocked rollups and #1247 Europe PMC/PMC source
windows, then verifies whether both candidate terms occur in the same bounded
source window. Only those source-local pair windows are eligible for endpoint
context extraction. The output is still blocked triage: not efficacy, safety,
treatment guidance, dosing guidance, recommendation, clinical actionability, or
cure evidence.
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
    "Europe PMC source-local endpoint expansion is source-text triage only; "
    "not efficacy, safety, treatment guidance, dosing guidance, recommendation, "
    "clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "europepmc_source_local_endpoint_context_not_clinical_actionability"
)

ISSUE1250_ROOT = "/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z"
ISSUE1247_ROOT = "/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z"

DEFAULT_INPUTS = {
    "issue1250_rollup_status": f"{ISSUE1250_ROOT}/out/clinicaltrials_endpoint_rollup_status.jsonl",
    "issue1250_pair_status": f"{ISSUE1250_ROOT}/out/clinicaltrials_endpoint_pair_status.jsonl",
    "issue1250_persisted_readback": f"{ISSUE1250_ROOT}/out/persisted_readback.json",
    "issue1250_calyx_readback": f"{ISSUE1250_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1250_output_manifest": f"{ISSUE1250_ROOT}/out/output_manifest.json",
    "issue1247_evidence_review": f"{ISSUE1247_ROOT}/out/europepmc_endpoint_outcome_evidence_review.jsonl",
    "issue1247_rollup": f"{ISSUE1247_ROOT}/out/candidate_europepmc_endpoint_outcome_rollup.jsonl",
}

PROMOTION_STATUS = (
    "blocked_requires_independent_effect_result_safety_falsification_and_human_review"
)

EVIDENCE_STATUS_VALUES = {
    "europepmc_source_local_endpoint_context_hit_still_blocked",
    "europepmc_source_local_pair_context_mechanistic_or_pk_only_still_blocked",
    "europepmc_source_window_not_pair_local_still_blocked",
}

ROLLUP_STATUS_VALUES = {
    "europepmc_source_local_endpoint_context_hit_still_blocked",
    "europepmc_source_local_pair_context_without_endpoint_still_blocked",
    "europepmc_no_source_local_pair_endpoint_hit_still_blocked",
}

ENDPOINT_CATEGORIES = {
    "clinical_endpoint_language_review",
    "endpoint_or_outcome_language_review",
    "preclinical_or_cell_endpoint_language_review",
    "pharmacokinetic_or_exposure_endpoint_review",
}

BENEFIT_PATTERNS = [
    r"\bbenefit\b",
    r"\bimprov(?:e|ed|ement|ing)?\b",
    r"\beffective(?:ness)?\b",
    r"\befficacy\b",
    r"\bresponse\b",
    r"\bremission\b",
    r"\breduc(?:e|ed|tion|ing)\b",
    r"\bdecreas(?:e|ed|ing)\b",
    r"\bsurvival\b",
]

WORSE_PATTERNS = [
    r"\bprogression\b",
    r"\brelapse\b",
    r"\brecurrence\b",
    r"\bworsen(?:ed|ing)?\b",
    r"\bincreas(?:e|ed|ing)\b",
    r"\bfailure\b",
]

COMPARATOR_PATTERNS = [
    r"\bversus\b",
    r"\bcompared with\b",
    r"\bcompared to\b",
    r"\bcomparison\b",
    r"\bplacebo\b",
    r"\bcontrol(?:led)?\b",
    r"\bstandard of care\b",
    r"\busual care\b",
    r"\bnon[- ]inferior",
    r"\bsuperior",
]

MAGNITUDE_PATTERNS = [
    r"\bhazard ratio\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\bodds ratio\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\brelative risk\b\s*(?:=|:|of)?\s*\d+(?:\.\d+)?",
    r"\bp\s*(?:=|<|>|<=|>=)\s*0?\.\d+",
    r"\b95%\s*CI\b",
    r"\bconfidence interval\b",
    r"\b\d+(?:\.\d+)?\s*%",
    r"\bAUC\b\s*(?:=|:)?\s*\d+(?:\.\d+)?",
]

COHORT_PATTERNS = [
    r"\bpatients?\b",
    r"\bhuman(?:s)?\b",
    r"\bcohort\b",
    r"\brandomi[sz]ed\b",
    r"\btrial\b",
    r"\bphase\s+(?:i|ii|iii|iv|1|2|3|4)\b",
    r"\bin vitro\b",
    r"\bcell(?:s| line| culture)?\b",
    r"\banimal(?:s)?\b",
    r"\bmice\b",
    r"\bmouse\b",
    r"\brats?\b",
    r"\bin silico\b",
    r"\bcomputational\b",
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


def norm_text(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def normalized_blob(value: object) -> str:
    return f" {norm_text(value)} "


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
    return " ".join(text.split())


def exact_presence(text: str, name: str) -> dict[str, Any]:
    raw = clean_text(name)
    query = query_name(raw)
    norm = norm_text(raw)
    raw_present = bool(raw and raw.lower() in text.lower())
    query_present = bool(query and query.lower() in text.lower())
    norm_present = bool(norm and f" {norm} " in normalized_blob(text))
    return {
        "name": raw,
        "query_name": query,
        "normalized_name": norm,
        "raw_case_insensitive_substring": raw_present,
        "query_case_insensitive_substring": query_present,
        "normalized_token_sequence": norm_present,
        "present": raw_present or query_present or norm_present,
        "exact": raw_present or query_present,
    }


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
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
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


def classify_direction(benefit_terms: list[str], worse_terms: list[str]) -> str:
    if benefit_terms and worse_terms:
        return "mixed_or_ambiguous_effect_language"
    if benefit_terms:
        return "benefit_or_improvement_language"
    if worse_terms:
        return "worsening_progression_or_increase_language"
    return "no_explicit_effect_direction_language"


def endpoint_type(row: dict[str, Any]) -> str:
    category = row["endpoint_outcome_review_category"]
    if category == "clinical_endpoint_language_review":
        return "clinical_endpoint_language"
    if category == "preclinical_or_cell_endpoint_language_review":
        return "preclinical_or_cell_endpoint_language"
    if category == "pharmacokinetic_or_exposure_endpoint_review":
        return "pharmacokinetic_or_exposure_language"
    if category == "endpoint_or_outcome_language_review":
        return "endpoint_or_outcome_language"
    if category == "combination_or_coexposure_context_review":
        return "combination_or_coexposure_context"
    if category == "mechanistic_endpoint_context_review":
        return "mechanistic_context"
    return "unclear_context"


def evidence_review(row: dict[str, Any]) -> dict[str, Any]:
    text = clean_text(row.get("source_text_window"))
    left_presence = exact_presence(text, row["drug_a"])
    right_presence = exact_presence(text, row["drug_b"])
    both_in_window = left_presence["present"] and right_presence["present"]
    exact_both = left_presence["exact"] and right_presence["exact"]
    benefit = pattern_hits(BENEFIT_PATTERNS, text)
    worse = pattern_hits(WORSE_PATTERNS, text)
    comparator = pattern_hits(COMPARATOR_PATTERNS, text)
    magnitude = pattern_hits(MAGNITUDE_PATTERNS, text)
    cohort = pattern_hits(COHORT_PATTERNS, text)
    endpoint_terms = uniq(row.get("clinical_endpoint_terms", []))
    effect_terms = uniq(row.get("effect_size_terms", []) + [hit["match"] for hit in magnitude])
    benefit_terms = uniq([hit["match"] for hit in benefit])
    worse_terms = uniq([hit["match"] for hit in worse])
    comparator_terms = uniq([hit["match"] for hit in comparator])
    cohort_terms = uniq([hit["match"] for hit in cohort])
    has_endpoint_context = (
        row["endpoint_outcome_review_category"] in ENDPOINT_CATEGORIES
        and (bool(endpoint_terms) or bool(effect_terms) or row.get("has_endpoint_or_outcome_language"))
    )
    if both_in_window and has_endpoint_context:
        status = "europepmc_source_local_endpoint_context_hit_still_blocked"
    elif both_in_window:
        status = "europepmc_source_local_pair_context_mechanistic_or_pk_only_still_blocked"
    else:
        status = "europepmc_source_window_not_pair_local_still_blocked"
    out = {
        "schema_version": 1,
        "review_id": "europepmc-source-local-endpoint-evidence:" + stable_id(row["review_id"], status),
        "source_issue1247_review_id": row["review_id"],
        "source_issue1247_validation_id": row["source_validation_id"],
        "source_evidence_id": row["source_evidence_id"],
        "pair_key": row["pair_key"],
        "representative_pair_id": row["representative_pair_id"],
        "drug_a": row["drug_a"],
        "drug_b": row["drug_b"],
        "source": row.get("source", "Europe PMC"),
        "source_id": row["source_id"],
        "source_url": row.get("source_url", ""),
        "source_text_sha256": row["source_text_sha256"],
        "source_text_window": text,
        "source_text_window_distance_chars": row.get("source_text_window_distance_chars"),
        "source_relation_class": row["source_relation_class"],
        "primary_model_system": row["primary_model_system"],
        "issue1247_endpoint_outcome_review_category": row["endpoint_outcome_review_category"],
        "source_local_endpoint_status": status,
        "endpoint_type": endpoint_type(row),
        "effect_direction_language": classify_direction(benefit_terms, worse_terms),
        "endpoint_terms": endpoint_terms,
        "effect_magnitude_terms": effect_terms,
        "benefit_or_improvement_terms": benefit_terms,
        "worsening_or_increase_terms": worse_terms,
        "comparator_terms": comparator_terms,
        "cohort_or_model_terms": cohort_terms,
        "mechanistic_terms": uniq(row.get("mechanistic_terms", [])),
        "pharmacokinetic_exposure_terms": uniq(row.get("pharmacokinetic_exposure_terms", [])),
        "dose_exposure_terms": uniq(row.get("dose_exposure_terms", [])),
        "combination_terms": uniq(row.get("combination_terms", [])),
        "trial_design_terms": uniq(row.get("trial_design_terms", [])),
        "pair_term_presence": {
            "left": left_presence,
            "right": right_presence,
            "both_present_in_source_window": both_in_window,
            "both_exact_in_source_window": exact_both,
        },
        "has_source_local_pair_context": both_in_window,
        "has_source_local_endpoint_context": status == "europepmc_source_local_endpoint_context_hit_still_blocked",
        "promotion_status": PROMOTION_STATUS,
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    out["reason_codes"] = reason_codes(out)
    out["next_validation_experiment"] = (
        "Independently validate effect direction/magnitude, comparator, cohort/model, safety/falsification, "
        "and human review before any promotion."
        if out["has_source_local_endpoint_context"]
        else "Continue source expansion; this bounded window does not prove source-local endpoint pair evidence."
    )
    return out


def reason_codes(row: dict[str, Any]) -> list[str]:
    codes = [
        "source_local_endpoint_expansion_not_clinical_actionability",
        "requires_independent_effect_result_safety_falsification_and_human_review",
    ]
    status = row["source_local_endpoint_status"]
    if status == "europepmc_source_local_endpoint_context_hit_still_blocked":
        codes.append("both_candidate_terms_present_in_bounded_source_window_with_endpoint_language")
    elif status == "europepmc_source_local_pair_context_mechanistic_or_pk_only_still_blocked":
        codes.append("both_candidate_terms_present_in_bounded_source_window_without_endpoint_language")
    else:
        codes.append("bounded_source_window_does_not_contain_both_candidate_terms")
    if row.get("effect_magnitude_terms"):
        codes.append("effect_magnitude_language_present_but_not_validated")
    if row.get("comparator_terms"):
        codes.append("comparator_language_present_but_not_validated")
    return codes


def build_evidence_reviews(evidence_rows: list[dict[str, Any]], scoped_pair_keys: set[str]) -> list[dict[str, Any]]:
    rows = [evidence_review(row) for row in evidence_rows if row["pair_key"] in scoped_pair_keys]
    rows.sort(
        key=lambda row: (
            0 if row["has_source_local_endpoint_context"] else 1,
            row["pair_key"],
            row["source_id"],
            row["review_id"],
        )
    )
    return rows


def build_rollup_status(
    rollups: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_reviews:
        by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for rollup in rollups:
        evidence = by_key.get(rollup["pair_key"], [])
        endpoint_evidence = [row for row in evidence if row["has_source_local_endpoint_context"]]
        pair_local_evidence = [row for row in evidence if row["has_source_local_pair_context"]]
        if endpoint_evidence:
            status = "europepmc_source_local_endpoint_context_hit_still_blocked"
        elif pair_local_evidence:
            status = "europepmc_source_local_pair_context_without_endpoint_still_blocked"
        else:
            status = "europepmc_no_source_local_pair_endpoint_hit_still_blocked"
        out = {
            "schema_version": 1,
            "rollup_validation_id": "europepmc-source-local-endpoint-rollup:" + stable_id(rollup["rollup_validation_id"], status),
            "source_issue1250_rollup_validation_id": rollup["rollup_validation_id"],
            "source_issue1247_rollup_review_id": rollup["source_issue1247_rollup_review_id"],
            "pair_id": rollup["pair_id"],
            "pair_key": rollup["pair_key"],
            "drug_a": rollup["drug_a"],
            "drug_b": rollup["drug_b"],
            "issue1250_clinicaltrials_endpoint_status": rollup["clinicaltrials_endpoint_status"],
            "issue1247_endpoint_outcome_review_category": rollup["issue1247_endpoint_outcome_review_category"],
            "source_local_endpoint_status": status,
            "evidence_review_rows": len(evidence),
            "source_local_pair_context_rows": len(pair_local_evidence),
            "source_local_endpoint_context_rows": len(endpoint_evidence),
            "source_ids": uniq([row["source_id"] for row in evidence])[:30],
            "source_local_endpoint_source_ids": uniq([row["source_id"] for row in endpoint_evidence])[:30],
            "evidence_review_ids": [row["review_id"] for row in evidence[:30]],
            "source_local_endpoint_review_ids": [row["review_id"] for row in endpoint_evidence[:30]],
            "endpoint_types": dict(sorted(Counter(row["endpoint_type"] for row in endpoint_evidence).items())),
            "effect_direction_counts": dict(sorted(Counter(row["effect_direction_language"] for row in endpoint_evidence).items())),
            "endpoint_terms": uniq([term for row in endpoint_evidence for term in row["endpoint_terms"]])[:30],
            "effect_magnitude_terms": uniq([term for row in endpoint_evidence for term in row["effect_magnitude_terms"]])[:30],
            "comparator_terms": uniq([term for row in endpoint_evidence for term in row["comparator_terms"]])[:30],
            "cohort_or_model_terms": uniq([term for row in endpoint_evidence for term in row["cohort_or_model_terms"]])[:30],
            "promotion_status": PROMOTION_STATUS,
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = rollup_reason_codes(out)
        out["next_validation_experiment"] = (
            "Independently validate source-local endpoint direction/magnitude, comparator, cohort/model, safety/falsification, "
            "and human review before any promotion."
            if endpoint_evidence
            else "Continue endpoint-source expansion beyond this Europe PMC window pass."
        )
        rows.append(out)
    rows.sort(
        key=lambda row: (
            0 if row["source_local_endpoint_context_rows"] > 0 else 1,
            -row["source_local_endpoint_context_rows"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rows


def rollup_reason_codes(row: dict[str, Any]) -> list[str]:
    codes = [
        "europepmc_source_local_endpoint_expansion_not_clinical_actionability",
        "requires_independent_effect_result_safety_falsification_and_human_review",
    ]
    status = row["source_local_endpoint_status"]
    if status == "europepmc_source_local_endpoint_context_hit_still_blocked":
        codes.append("has_bounded_source_window_pair_endpoint_context")
    elif status == "europepmc_source_local_pair_context_without_endpoint_still_blocked":
        codes.append("has_bounded_source_window_pair_context_without_endpoint")
    else:
        codes.append("no_bounded_source_window_pair_endpoint_hit")
    return codes


def build_bridge_rows(
    rollup_status: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        text = (
            f"Europe PMC source-local endpoint rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['source_local_endpoint_status']} "
            f"endpoint context rows {row['source_local_endpoint_context_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_validation_id"],
                "domain": "europepmc_source_local_endpoint_rollup",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_local_endpoint_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1251_europepmc_source_local_endpoint_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_local_endpoint_status": row["source_local_endpoint_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence_reviews[:budget]:
        text = (
            f"Europe PMC source-local endpoint evidence {row['review_id']} source {row['source_id']} pair "
            f"{row['pair_key']} {row['drug_a']} plus {row['drug_b']} status {row['source_local_endpoint_status']}."
        )
        rows.append(
            {
                "id": row["review_id"],
                "domain": "europepmc_source_local_endpoint_evidence",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["review_id"], row["source_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["source_local_endpoint_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1251_europepmc_source_local_endpoint_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_id": row["source_id"],
                    "source_local_endpoint_status": row["source_local_endpoint_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    rollups: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
) -> dict[str, Any]:
    endpoint_evidence = [row for row in evidence_reviews if row["has_source_local_endpoint_context"]]
    pair_local_evidence = [row for row in evidence_reviews if row["has_source_local_pair_context"]]
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1250_rollup_rows": len(rollups),
        "issue1250_pair_status_rows": len(pair_status),
        "evidence_review_rows": len(evidence_reviews),
        "rollup_status_rows": len(rollup_status),
        "source_local_pair_context_evidence_rows": len(pair_local_evidence),
        "source_local_endpoint_context_evidence_rows": len(endpoint_evidence),
        "rollups_with_source_local_pair_context": sum(1 for row in rollup_status if row["source_local_pair_context_rows"] > 0),
        "rollups_with_source_local_endpoint_context": sum(1 for row in rollup_status if row["source_local_endpoint_context_rows"] > 0),
        "evidence_status_counts": dict(sorted(Counter(row["source_local_endpoint_status"] for row in evidence_reviews).items())),
        "rollup_status_counts": dict(sorted(Counter(row["source_local_endpoint_status"] for row in rollup_status).items())),
        "endpoint_type_counts": dict(sorted(Counter(row["endpoint_type"] for row in endpoint_evidence).items())),
        "effect_direction_counts": dict(sorted(Counter(row["effect_direction_language"] for row in endpoint_evidence).items())),
        "primary_model_system_counts": dict(sorted(Counter(row["primary_model_system"] for row in endpoint_evidence).items())),
        "endpoint_evidence_rows_with_magnitude_language": sum(1 for row in endpoint_evidence if row["effect_magnitude_terms"]),
        "endpoint_evidence_rows_with_comparator_language": sum(1 for row in endpoint_evidence if row["comparator_terms"]),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["source_local_endpoint_status"],
                "endpoint_rows": row["source_local_endpoint_context_rows"],
                "source_ids": row["source_local_endpoint_source_ids"][:5],
                "endpoint_terms": row["endpoint_terms"][:8],
                "effect_magnitude_terms": row["effect_magnitude_terms"][:8],
            }
            for row in rollup_status
            if row["source_local_endpoint_context_rows"] > 0
        ][:50],
    }


def build_input_manifest(
    inputs: dict[str, str],
    rollups: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    issue1247_evidence: list[dict[str, Any]],
    issue1247_rollups: list[dict[str, Any]],
    issue1250_persisted_readback: dict[str, Any],
    issue1250_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1251,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1250_rollup_status": artifact(Path(inputs["issue1250_rollup_status"]), jsonl=True),
            "issue1250_pair_status": artifact(Path(inputs["issue1250_pair_status"]), jsonl=True),
            "issue1250_persisted_readback": artifact(Path(inputs["issue1250_persisted_readback"])),
            "issue1250_calyx_readback": artifact(Path(inputs["issue1250_calyx_readback"])),
            "issue1250_output_manifest": artifact(Path(inputs["issue1250_output_manifest"])),
            "issue1247_evidence_review": artifact(Path(inputs["issue1247_evidence_review"]), jsonl=True),
            "issue1247_rollup": artifact(Path(inputs["issue1247_rollup"]), jsonl=True),
        },
        "source_contract": {
            "issue1250_persisted_assertions_all_true": all_assertions_true(issue1250_persisted_readback),
            "issue1250_calyx_assertions_all_true": all_assertions_true(issue1250_calyx_readback),
            "issue1250_rollup_rows": len(rollups),
            "issue1250_pair_status_rows": len(pair_status),
            "issue1247_evidence_review_rows": len(issue1247_evidence),
            "issue1247_rollup_rows": len(issue1247_rollups),
            "hit_requires_both_candidate_terms_in_bounded_source_window": True,
            "endpoint_context_is_review_flag_not_efficacy_claim": True,
        },
    }


def build_readback(
    out_dir: Path,
    rollups: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_reviews: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    issue1250_persisted_readback: dict[str, Any],
    issue1250_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "europepmc_source_local_endpoint_evidence_review": artifact(
            out_dir / "europepmc_source_local_endpoint_evidence_review.jsonl", jsonl=True
        ),
        "europepmc_source_local_endpoint_rollup_status": artifact(
            out_dir / "europepmc_source_local_endpoint_rollup_status.jsonl", jsonl=True
        ),
        "europepmc_source_local_endpoint_bridge_rows": artifact(
            out_dir / "europepmc_source_local_endpoint_bridge_rows.jsonl", jsonl=True
        ),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    rollup_ids = {row["rollup_validation_id"] for row in rollups}
    status_ids = {row["source_issue1250_rollup_validation_id"] for row in rollup_status}
    pair_keys = {row["pair_key"] for row in pair_status}
    evidence_keys = {row["pair_key"] for row in evidence_reviews}
    endpoint_evidence_keys = {row["pair_key"] for row in evidence_reviews if row["has_source_local_endpoint_context"]}
    endpoint_rollup_keys = {row["pair_key"] for row in rollup_status if row["source_local_endpoint_context_rows"] > 0}
    return {
        "schema_version": 1,
        "issue": 1251,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1250_persisted_readback_all_true": all_assertions_true(issue1250_persisted_readback),
            "issue1250_calyx_readback_all_true": all_assertions_true(issue1250_calyx_readback),
            "rollup_status_for_every_issue1250_rollup": status_ids == rollup_ids,
            "evidence_rows_cover_pair_status_keys": pair_keys <= evidence_keys,
            "endpoint_rollup_keys_have_endpoint_evidence": endpoint_rollup_keys <= endpoint_evidence_keys,
            "all_evidence_status_values_allowed": all(row["source_local_endpoint_status"] in EVIDENCE_STATUS_VALUES for row in evidence_reviews),
            "all_rollup_status_values_allowed": all(row["source_local_endpoint_status"] in ROLLUP_STATUS_VALUES for row in rollup_status),
            "source_local_endpoint_evidence_has_both_terms": all(
                row["pair_term_presence"]["both_present_in_source_window"]
                for row in evidence_reviews
                if row["has_source_local_endpoint_context"]
            ),
            "all_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence_reviews)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_reviews)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status),
            "bridge_rows_1000_or_less": artifacts["europepmc_source_local_endpoint_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "issue1250_rollups": len(rollups),
            "issue1250_pair_status_rows": len(pair_status),
            "evidence_review_rows": len(evidence_reviews),
            "rollup_status_rows": len(rollup_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_rollups: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    rollups = rows_jsonl(Path(inputs["issue1250_rollup_status"]))
    if max_rollups is not None:
        keep_ids = {row["rollup_validation_id"] for row in rollups[:max_rollups]}
        rollups = [row for row in rollups if row["rollup_validation_id"] in keep_ids]
    pair_status = rows_jsonl(Path(inputs["issue1250_pair_status"]))
    issue1247_evidence = rows_jsonl(Path(inputs["issue1247_evidence_review"]))
    issue1247_rollups = rows_jsonl(Path(inputs["issue1247_rollup"]))
    scoped_pair_keys = {row["pair_key"] for row in rollups}
    issue1250_persisted_readback = read_json(Path(inputs["issue1250_persisted_readback"]))
    issue1250_calyx_readback = read_json(Path(inputs["issue1250_calyx_readback"]))
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            rollups,
            pair_status,
            issue1247_evidence,
            issue1247_rollups,
            issue1250_persisted_readback,
            issue1250_calyx_readback,
        ),
    )
    evidence_reviews = build_evidence_reviews(issue1247_evidence, scoped_pair_keys)
    write_jsonl(out_dir / "europepmc_source_local_endpoint_evidence_review.jsonl", evidence_reviews)
    rollup_status = build_rollup_status(rollups, evidence_reviews)
    write_jsonl(out_dir / "europepmc_source_local_endpoint_rollup_status.jsonl", rollup_status)
    source_path = out_dir / "europepmc_source_local_endpoint_rollup_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, evidence_reviews, source_path, source_sha)
    write_jsonl(out_dir / "europepmc_source_local_endpoint_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(rollups, pair_status, evidence_reviews, rollup_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1251,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "europepmc_source_local_endpoint_evidence_review": artifact(
                out_dir / "europepmc_source_local_endpoint_evidence_review.jsonl", jsonl=True
            ),
            "europepmc_source_local_endpoint_rollup_status": artifact(
                out_dir / "europepmc_source_local_endpoint_rollup_status.jsonl", jsonl=True
            ),
            "europepmc_source_local_endpoint_bridge_rows": artifact(
                out_dir / "europepmc_source_local_endpoint_bridge_rows.jsonl", jsonl=True
            ),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        rollups,
        pair_status,
        evidence_reviews,
        rollup_status,
        issue1250_persisted_readback,
        issue1250_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "evidence_review": output_manifest["artifacts"]["europepmc_source_local_endpoint_evidence_review"],
            "rollup_status": output_manifest["artifacts"]["europepmc_source_local_endpoint_rollup_status"],
            "bridge_rows": output_manifest["artifacts"]["europepmc_source_local_endpoint_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1250-rollup-status")
    parser.add_argument("--issue1250-pair-status")
    parser.add_argument("--issue1250-persisted-readback")
    parser.add_argument("--issue1250-calyx-readback")
    parser.add_argument("--issue1250-output-manifest")
    parser.add_argument("--issue1247-evidence-review")
    parser.add_argument("--issue1247-rollup")
    parser.add_argument("--max-rollups", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1250_rollup_status", "issue1250_rollup_status"),
        ("issue1250_pair_status", "issue1250_pair_status"),
        ("issue1250_persisted_readback", "issue1250_persisted_readback"),
        ("issue1250_calyx_readback", "issue1250_calyx_readback"),
        ("issue1250_output_manifest", "issue1250_output_manifest"),
        ("issue1247_evidence_review", "issue1247_evidence_review"),
        ("issue1247_rollup", "issue1247_rollup"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_rollups=args.max_rollups)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
