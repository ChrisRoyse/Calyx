#!/usr/bin/env python3
"""#1246 falsification/safety review for #1244 Europe PMC relation rollups.

This stage reads sealed #1244 source-text validation artifacts, filters the
rollups requiring safety/counter review, and separates bounded source-window
signals into conservative review categories. It emits research-triage review
rows only: not safety clearance, contraindication guidance, treatment guidance,
recommendation, clinical actionability, or cure evidence.
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
    "Europe PMC safety/counter review is literature triage only; not safety "
    "clearance, contraindication guidance, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "europepmc_safety_counter_falsification_review_not_clinical_clearance"
)

ISSUE1244_ROOT = "/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z"

DEFAULT_INPUTS = {
    "issue1244_candidate_rollup": f"{ISSUE1244_ROOT}/out/candidate_europepmc_relation_rollup.jsonl",
    "issue1244_relation_validation": f"{ISSUE1244_ROOT}/out/europepmc_source_text_relation_validation.jsonl",
    "issue1244_persisted_readback": f"{ISSUE1244_ROOT}/out/persisted_readback.json",
    "issue1244_calyx_readback": f"{ISSUE1244_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1244_output_manifest": f"{ISSUE1244_ROOT}/out/output_manifest.json",
}

REVIEW_STATUS = "safety_counter_review_required_still_blocked"
PROMOTION_STATUS = "blocked_requires_independent_safety_falsification_and_human_review"

FATALITY_PATTERNS = [r"\bfatal\b", r"\bdeath\b", r"\bmortality\b"]
CONTRAINDICATION_PATTERNS = [r"\bcontraindicat", r"\bavoid\b", r"\bnot recommended\b"]
TOXICITY_PATTERNS = [r"\btoxicit", r"\btoxic\b", r"\bhepatotoxic", r"\bnephrotoxic", r"\barrhythm"]
ADVERSE_PATTERNS = [r"\badverse events?\b", r"\bserious adverse\b", r"\bsafety\b", r"\bharm\b"]
NEGATIVE_PATTERNS = [r"\bno significant\b", r"\bnot significant\b", r"\bdid not\b", r"\bfailed\b", r"\bfailure\b", r"\bwithout benefit\b", r"\black of\b"]
RISK_PATTERNS = [r"\brisk(?:s)?\b", r"\bincreased risk\b", r"\breduced risk\b"]


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
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True) + "\n")


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"required artifact is missing: {path}")
    return {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
        "rows": len(rows_jsonl(path)) if jsonl else None,
    }


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [value for value in inputs.values() if not Path(value).exists()]
    if missing:
        raise SystemExit(
            json.dumps(
                {
                    "code": "CALYX_DISCOVERY_SOURCE_MISSING",
                    "message": "required #1244 source artifacts are missing",
                    "missing": missing,
                    "remediation": "finish #1244 and persist relation-validation artifacts before running #1246",
                },
                indent=2,
            )
        )


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


def classify_review(row: dict[str, Any]) -> dict[str, Any]:
    text = clean_text(row.get("source_text_window"))
    fatality = pattern_hits(FATALITY_PATTERNS, text)
    contraindication = pattern_hits(CONTRAINDICATION_PATTERNS, text)
    toxicity = pattern_hits(TOXICITY_PATTERNS, text)
    adverse = pattern_hits(ADVERSE_PATTERNS, text)
    negative = pattern_hits(NEGATIVE_PATTERNS, text)
    risk = pattern_hits(RISK_PATTERNS, text)
    safety_terms = uniq(term_list(row, "safety_or_adverse_context") + [hit["match"] for hit in adverse + toxicity + fatality + contraindication + risk])
    counter_terms = uniq(term_list(row, "counter_or_negative_context") + [hit["match"] for hit in negative])
    if fatality:
        category = "fatality_or_mortality_language_review"
    elif contraindication:
        category = "contraindication_or_avoidance_language_review"
    elif toxicity:
        category = "toxicity_language_review"
    elif counter_terms and safety_terms:
        category = "counter_negative_and_safety_language_review"
    elif counter_terms:
        category = "counter_negative_language_review"
    elif safety_terms:
        category = "safety_adverse_language_review"
    elif risk:
        category = "generic_risk_language_review"
    else:
        category = "safety_counter_review_unclear_fail_closed"
    return {
        "safety_counter_review_category": category,
        "fatality_terms": uniq([hit["match"] for hit in fatality]),
        "contraindication_terms": uniq([hit["match"] for hit in contraindication]),
        "toxicity_terms": uniq([hit["match"] for hit in toxicity]),
        "adverse_safety_terms": safety_terms,
        "counter_negative_terms": counter_terms,
        "risk_terms": uniq([hit["match"] for hit in risk]),
        "fatality_spans": fatality[:6],
        "contraindication_spans": contraindication[:6],
        "toxicity_spans": toxicity[:6],
        "adverse_safety_spans": (adverse + toxicity + fatality + contraindication + risk)[:10],
        "counter_negative_spans": negative[:10],
        "risk_spans": risk[:6],
    }


def reason_codes(review: dict[str, Any]) -> list[str]:
    codes = [
        "safety_counter_review_not_clinical_clearance",
        "requires_independent_safety_falsification_and_human_review",
    ]
    if review["fatality_terms"]:
        codes.append("fatality_or_mortality_language_present_requires_review")
    if review["contraindication_terms"]:
        codes.append("contraindication_or_avoidance_language_present_not_guidance")
    if review["toxicity_terms"]:
        codes.append("toxicity_language_present_requires_review")
    if review["counter_negative_terms"]:
        codes.append("counter_or_negative_language_present_requires_falsification_review")
    if review["adverse_safety_terms"]:
        codes.append("safety_or_adverse_language_present_requires_review")
    if review["risk_terms"]:
        codes.append("risk_language_present_requires_review")
    return codes


def build_evidence_reviews(validation_rows: list[dict[str, Any]], scoped_pair_keys: set[str]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in validation_rows:
        if row["pair_key"] not in scoped_pair_keys:
            continue
        if row["source_text_validation_status"] != "source_text_safety_or_counter_review_required_still_blocked":
            continue
        review = classify_review(row)
        review_id = "europepmc-safety-counter-evidence:" + stable_id(row["validation_id"], review["safety_counter_review_category"])
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
            "primary_model_system": row["primary_model_system"],
            "review_status": REVIEW_STATUS,
            "promotion_status": PROMOTION_STATUS,
            "has_safety_language": row["has_safety_language"],
            "has_counter_or_negative_language": row["has_counter_or_negative_language"],
            "has_outcome_language": row["has_outcome_language"],
            "has_dose_or_exposure_language": row["has_dose_or_exposure_language"],
            **review,
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = reason_codes(out)
        out["next_validation_experiment"] = (
            "Route through independent safety/falsification review and human review with physical readback; "
            "do not emit safety clearance or clinical guidance."
        )
        rows.append(out)
    rows.sort(key=lambda row: (row["pair_key"], row["source_id"], row["review_id"]))
    return rows


def build_rollup_reviews(scoped_rollups: list[dict[str, Any]], evidence_reviews: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_reviews:
        by_pair[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for rollup in scoped_rollups:
        evidence = by_pair.get(rollup["pair_key"], [])
        category_counts = Counter(row["safety_counter_review_category"] for row in evidence)
        model_counts = Counter(row["primary_model_system"] for row in evidence)
        fatality = any(row["fatality_terms"] for row in evidence)
        contraindication = any(row["contraindication_terms"] for row in evidence)
        toxicity = any(row["toxicity_terms"] for row in evidence)
        counter = any(row["counter_negative_terms"] for row in evidence)
        safety = any(row["adverse_safety_terms"] for row in evidence)
        if fatality:
            top_category = "fatality_or_mortality_language_review"
        elif contraindication:
            top_category = "contraindication_or_avoidance_language_review"
        elif toxicity:
            top_category = "toxicity_language_review"
        elif counter and safety:
            top_category = "counter_negative_and_safety_language_review"
        elif counter:
            top_category = "counter_negative_language_review"
        elif safety:
            top_category = "safety_adverse_language_review"
        else:
            top_category = "safety_counter_review_unclear_fail_closed"
        out = {
            "schema_version": 1,
            "rollup_review_id": "europepmc-safety-counter-rollup:" + stable_id(rollup["rollup_id"], top_category),
            "source_issue1244_rollup_id": rollup["rollup_id"],
            "pair_id": rollup["pair_id"],
            "pair_key": rollup["pair_key"],
            "drug_a": rollup["drug_a"],
            "drug_b": rollup["drug_b"],
            "source_text_validation_status": rollup["source_text_validation_status"],
            "safety_counter_review_category": top_category,
            "review_status": REVIEW_STATUS,
            "promotion_status": PROMOTION_STATUS,
            "evidence_review_rows": len(evidence),
            "category_counts": dict(sorted(category_counts.items())),
            "primary_model_system_counts": dict(sorted(model_counts.items())),
            "source_ids": uniq([row["source_id"] for row in evidence])[:30],
            "evidence_review_ids": [row["review_id"] for row in evidence[:30]],
            "has_fatality_or_mortality_language": fatality,
            "has_contraindication_or_avoidance_language": contraindication,
            "has_toxicity_language": toxicity,
            "has_counter_or_negative_language": counter,
            "has_safety_or_adverse_language": safety,
            "fatality_terms": uniq([term for row in evidence for term in row["fatality_terms"]])[:30],
            "contraindication_terms": uniq([term for row in evidence for term in row["contraindication_terms"]])[:30],
            "toxicity_terms": uniq([term for row in evidence for term in row["toxicity_terms"]])[:30],
            "counter_negative_terms": uniq([term for row in evidence for term in row["counter_negative_terms"]])[:30],
            "adverse_safety_terms": uniq([term for row in evidence for term in row["adverse_safety_terms"]])[:30],
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = reason_codes(
            {
                "fatality_terms": out["fatality_terms"],
                "contraindication_terms": out["contraindication_terms"],
                "toxicity_terms": out["toxicity_terms"],
                "counter_negative_terms": out["counter_negative_terms"],
                "adverse_safety_terms": out["adverse_safety_terms"],
                "risk_terms": [],
            }
        )
        out["next_validation_experiment"] = (
            "Run independent safety/falsification review and human review; preserve this row as blocked triage."
        )
        rows.append(out)
    rows.sort(
        key=lambda row: (
            -row["evidence_review_rows"],
            row["safety_counter_review_category"],
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
            f"Europe PMC safety counter rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} category {row['safety_counter_review_category']} "
            f"review status {row['review_status']} evidence rows {row['evidence_review_rows']} "
            f"promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_review_id"],
                "domain": "europepmc_safety_counter_rollup",
                "text": text,
                "bridge_terms": bridge_terms(
                    [row["pair_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["safety_counter_review_category"]],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1246_europepmc_safety_counter_review",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "pair_key": row["pair_key"],
                    "safety_counter_review_category": row["safety_counter_review_category"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence_reviews[:budget]:
        text = (
            f"Europe PMC safety counter evidence {row['review_id']} source {row['source_id']} pair "
            f"{row['pair_key']} {row['drug_a']} plus {row['drug_b']} category "
            f"{row['safety_counter_review_category']} review status {row['review_status']}."
        )
        rows.append(
            {
                "id": row["review_id"],
                "domain": "europepmc_safety_counter_evidence",
                "text": text,
                "bridge_terms": bridge_terms(
                    [row["review_id"], row["source_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["safety_counter_review_category"]],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1246_europepmc_safety_counter_review",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "source_id": row["source_id"],
                    "pair_key": row["pair_key"],
                    "safety_counter_review_category": row["safety_counter_review_category"],
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
    evidence_category_counts = Counter(row["safety_counter_review_category"] for row in evidence_reviews)
    rollup_category_counts = Counter(row["safety_counter_review_category"] for row in rollup_reviews)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "scoped_issue1244_rollups": len(scoped_rollups),
        "evidence_review_rows": len(evidence_reviews),
        "rollup_review_rows": len(rollup_reviews),
        "evidence_category_counts": dict(sorted(evidence_category_counts.items())),
        "rollup_category_counts": dict(sorted(rollup_category_counts.items())),
        "rollups_with_fatality_or_mortality_language": sum(
            1 for row in rollup_reviews if row["has_fatality_or_mortality_language"]
        ),
        "rollups_with_contraindication_or_avoidance_language": sum(
            1 for row in rollup_reviews if row["has_contraindication_or_avoidance_language"]
        ),
        "rollups_with_toxicity_language": sum(1 for row in rollup_reviews if row["has_toxicity_language"]),
        "rollups_with_counter_or_negative_language": sum(
            1 for row in rollup_reviews if row["has_counter_or_negative_language"]
        ),
        "rollups_with_safety_or_adverse_language": sum(
            1 for row in rollup_reviews if row["has_safety_or_adverse_language"]
        ),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "category": row["safety_counter_review_category"],
                "evidence_review_rows": row["evidence_review_rows"],
                "source_ids": row["source_ids"][:5],
                "fatality_terms": row["fatality_terms"][:5],
                "contraindication_terms": row["contraindication_terms"][:5],
                "toxicity_terms": row["toxicity_terms"][:5],
                "counter_negative_terms": row["counter_negative_terms"][:5],
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
    scoped = [
        row
        for row in issue1244_rollups
        if row.get("source_text_validation_status") == "source_text_safety_or_counter_review_required_still_blocked"
    ]
    return {
        "schema_version": 1,
        "issue": 1246,
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
            "scoped_safety_counter_rollups": len(scoped),
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
        "europepmc_safety_counter_evidence_review": artifact(
            out_dir / "europepmc_safety_counter_evidence_review.jsonl", jsonl=True
        ),
        "candidate_europepmc_safety_counter_rollup": artifact(
            out_dir / "candidate_europepmc_safety_counter_rollup.jsonl", jsonl=True
        ),
        "europepmc_safety_counter_bridge_rows": artifact(
            out_dir / "europepmc_safety_counter_bridge_rows.jsonl", jsonl=True
        ),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    scoped_pair_ids = {row["pair_id"] for row in scoped_rollups}
    reviewed_pair_ids = {row["pair_id"] for row in rollup_reviews}
    return {
        "schema_version": 1,
        "issue": 1246,
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
            "all_evidence_reviews_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_reviews),
            "all_rollup_reviews_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_reviews),
            "bridge_rows_1000_or_less": artifacts["europepmc_safety_counter_bridge_rows"]["rows"] <= 1000,
            "bridge_rows_cover_rollups_plus_budgeted_evidence": artifacts["europepmc_safety_counter_bridge_rows"]["rows"]
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
    scoped_rollups = [
        row
        for row in issue1244_rollups
        if row.get("source_text_validation_status") == "source_text_safety_or_counter_review_required_still_blocked"
    ]
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
    write_jsonl(out_dir / "europepmc_safety_counter_evidence_review.jsonl", evidence_reviews)
    rollup_reviews = build_rollup_reviews(scoped_rollups, evidence_reviews)
    write_jsonl(out_dir / "candidate_europepmc_safety_counter_rollup.jsonl", rollup_reviews)
    source_path = out_dir / "europepmc_safety_counter_evidence_review.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_reviews, evidence_reviews, source_path, source_sha)
    write_jsonl(out_dir / "europepmc_safety_counter_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(scoped_rollups, evidence_reviews, rollup_reviews)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1246,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "europepmc_safety_counter_evidence_review": artifact(
                out_dir / "europepmc_safety_counter_evidence_review.jsonl", jsonl=True
            ),
            "candidate_europepmc_safety_counter_rollup": artifact(
                out_dir / "candidate_europepmc_safety_counter_rollup.jsonl", jsonl=True
            ),
            "europepmc_safety_counter_bridge_rows": artifact(
                out_dir / "europepmc_safety_counter_bridge_rows.jsonl", jsonl=True
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
            "europepmc_safety_counter_evidence_review": output_manifest["artifacts"][
                "europepmc_safety_counter_evidence_review"
            ],
            "candidate_europepmc_safety_counter_rollup": output_manifest["artifacts"][
                "candidate_europepmc_safety_counter_rollup"
            ],
            "bridge_rows": output_manifest["artifacts"]["europepmc_safety_counter_bridge_rows"],
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
