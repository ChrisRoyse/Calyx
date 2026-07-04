#!/usr/bin/env python3
"""#1252 effect-result and falsification gate for source-local endpoint rows.

This stage reads sealed #1251 Europe PMC/PMC source-local endpoint rows and
extracts structured result/falsification fields from bounded source windows.
It is a conservative text triage layer only: every row remains blocked unless
future endpoint-result, safety, falsification, and human-review gates are
physically proven.
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
    "Effect-result and falsification extraction is source-text triage only; "
    "not efficacy, safety, treatment guidance, dosing guidance, recommendation, "
    "clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = "effect_result_falsification_gate_not_clinical_actionability"

ISSUE1251_ROOT = "/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z"

DEFAULT_INPUTS = {
    "issue1251_rollup_status": f"{ISSUE1251_ROOT}/out/europepmc_source_local_endpoint_rollup_status.jsonl",
    "issue1251_evidence_review": f"{ISSUE1251_ROOT}/out/europepmc_source_local_endpoint_evidence_review.jsonl",
    "issue1251_persisted_readback": f"{ISSUE1251_ROOT}/out/persisted_readback.json",
    "issue1251_calyx_readback": f"{ISSUE1251_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1251_output_manifest": f"{ISSUE1251_ROOT}/out/output_manifest.json",
}

INPUT_ROLLUP_STATUS = "europepmc_source_local_endpoint_context_hit_still_blocked"
INPUT_EVIDENCE_STATUS = "europepmc_source_local_endpoint_context_hit_still_blocked"
PROMOTION_STATUS = "blocked_requires_real_effect_safety_falsification_and_human_review"

EVIDENCE_STATUS_VALUES = {
    "counter_or_safety_language_blocks_effect_result_candidate",
    "effect_result_candidate_with_magnitude_and_direction_still_blocked",
    "effect_result_candidate_direction_only_still_blocked",
    "endpoint_context_without_result_assertion_still_blocked",
}

ROLLUP_STATUS_VALUES = {
    "counter_or_safety_language_blocks_rollup",
    "effect_result_candidate_with_magnitude_and_direction_still_blocked",
    "effect_result_candidate_direction_only_still_blocked",
    "endpoint_context_without_result_assertion_still_blocked",
}

RESULT_ASSERTION_PATTERNS = [
    r"\bresults?\b",
    r"\bshow(?:ed|n|s)?\b",
    r"\bdemonstrat(?:ed|es|ing|ion)\b",
    r"\bobserv(?:ed|es|ation)\b",
    r"\bassociat(?:ed|ion)\b",
    r"\bindicat(?:ed|es|ing|ion)\b",
    r"\bfound\b",
    r"\breported\b",
    r"\brevealed\b",
    r"\bled to\b",
    r"\bresult(?:ed)? in\b",
    r"\bsignificant(?:ly)?\b",
    r"\btreated\b",
    r"\bresponse\b",
    r"\bsurvival\b",
    r"\bprogression\b",
]

SAFETY_PATTERNS = [
    r"\badverse events?\b",
    r"\bserious adverse\b",
    r"\bsafety\b",
    r"\btoxicit",
    r"\btoxic\b",
    r"\bfatal\b",
    r"\bdeath\b",
    r"\bmortality\b",
    r"\bharm\b",
    r"\brisk(?:s)?\b",
    r"\bcontraindicat",
    r"\bavoid\b",
    r"\bnot recommended\b",
]

COUNTER_PATTERNS = [
    r"\bno significant\b",
    r"\bnot significant\b",
    r"\bdid not\b",
    r"\bfailed\b",
    r"\bfailure\b",
    r"\bwithout benefit\b",
    r"\black of\b",
    r"\bno benefit\b",
    r"\bno effect\b",
    r"\bnot effective\b",
    r"\bno response\b",
    r"\bnegative\b",
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
    with path.open("r", encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


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


def classify_evidence(row: dict[str, Any]) -> dict[str, Any]:
    text = clean_text(row["source_text_window"])
    result_hits = pattern_hits(RESULT_ASSERTION_PATTERNS, text)
    safety_hits = pattern_hits(SAFETY_PATTERNS, text)
    counter_hits = pattern_hits(COUNTER_PATTERNS, text)
    magnitude_terms = uniq(row.get("effect_magnitude_terms", []))
    comparator_terms = uniq(row.get("comparator_terms", []))
    direction = row.get("effect_direction_language", "no_explicit_effect_direction_language")
    direction_present = direction != "no_explicit_effect_direction_language"
    has_result_assertion = bool(result_hits) and direction_present
    has_magnitude = bool(magnitude_terms)
    has_safety_or_counter = bool(safety_hits or counter_hits)
    if has_safety_or_counter:
        status = "counter_or_safety_language_blocks_effect_result_candidate"
    elif has_result_assertion and has_magnitude:
        status = "effect_result_candidate_with_magnitude_and_direction_still_blocked"
    elif has_result_assertion:
        status = "effect_result_candidate_direction_only_still_blocked"
    else:
        status = "endpoint_context_without_result_assertion_still_blocked"
    out = {
        "schema_version": 1,
        "result_review_id": "effect-result-evidence:" + stable_id(row["review_id"], status),
        "source_issue1251_review_id": row["review_id"],
        "pair_key": row["pair_key"],
        "representative_pair_id": row["representative_pair_id"],
        "drug_a": row["drug_a"],
        "drug_b": row["drug_b"],
        "source": row.get("source", "Europe PMC"),
        "source_id": row["source_id"],
        "source_url": row.get("source_url", ""),
        "source_text_sha256": row["source_text_sha256"],
        "source_text_window": text,
        "source_relation_class": row["source_relation_class"],
        "primary_model_system": row["primary_model_system"],
        "source_local_endpoint_status": row["source_local_endpoint_status"],
        "effect_result_gate_status": status,
        "endpoint_type": row["endpoint_type"],
        "effect_direction_language": direction,
        "endpoint_terms": uniq(row.get("endpoint_terms", [])),
        "effect_magnitude_terms": magnitude_terms,
        "comparator_terms": comparator_terms,
        "cohort_or_model_terms": uniq(row.get("cohort_or_model_terms", [])),
        "mechanistic_terms": uniq(row.get("mechanistic_terms", [])),
        "pharmacokinetic_exposure_terms": uniq(row.get("pharmacokinetic_exposure_terms", [])),
        "dose_exposure_terms": uniq(row.get("dose_exposure_terms", [])),
        "pair_term_presence": row["pair_term_presence"],
        "result_assertion_terms": uniq([hit["match"] for hit in result_hits]),
        "safety_terms": uniq([hit["match"] for hit in safety_hits]),
        "counter_or_negative_terms": uniq([hit["match"] for hit in counter_hits]),
        "result_assertion_spans": result_hits[:12],
        "safety_spans": safety_hits[:12],
        "counter_or_negative_spans": counter_hits[:12],
        "has_result_assertion_language": has_result_assertion,
        "has_effect_magnitude_language": has_magnitude,
        "has_comparator_language": bool(comparator_terms),
        "has_safety_or_counter_language": has_safety_or_counter,
        "promotion_status": PROMOTION_STATUS,
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    out["reason_codes"] = evidence_reason_codes(out)
    out["next_validation_experiment"] = (
        "Route through independent effect-result extraction with source table/figure context, safety/falsification, "
        "and human review before any promotion."
    )
    return out


def evidence_reason_codes(row: dict[str, Any]) -> list[str]:
    codes = [
        "effect_result_extraction_not_clinical_actionability",
        "requires_real_effect_safety_falsification_and_human_review",
    ]
    status = row["effect_result_gate_status"]
    if status == "counter_or_safety_language_blocks_effect_result_candidate":
        codes.append("safety_or_counter_language_present_blocks_promotion")
    elif status == "effect_result_candidate_with_magnitude_and_direction_still_blocked":
        codes.append("direction_and_magnitude_language_present_but_not_validated_result")
    elif status == "effect_result_candidate_direction_only_still_blocked":
        codes.append("direction_language_present_without_validated_magnitude")
    else:
        codes.append("endpoint_context_without_result_assertion")
    if row["has_comparator_language"]:
        codes.append("comparator_language_present_but_not_validated")
    return codes


def build_evidence_rows(evidence_rows: list[dict[str, Any]], scoped_pair_keys: set[str]) -> list[dict[str, Any]]:
    rows = [
        classify_evidence(row)
        for row in evidence_rows
        if row["pair_key"] in scoped_pair_keys
        and row["source_local_endpoint_status"] == INPUT_EVIDENCE_STATUS
    ]
    rows.sort(
        key=lambda row: (
            0 if row["effect_result_gate_status"] == "effect_result_candidate_with_magnitude_and_direction_still_blocked" else 1,
            row["effect_result_gate_status"],
            row["pair_key"],
            row["source_id"],
        )
    )
    return rows


def build_rollup_status(rollups: list[dict[str, Any]], result_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in result_rows:
        by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for rollup in rollups:
        evidence = by_key.get(rollup["pair_key"], [])
        status_counts = Counter(row["effect_result_gate_status"] for row in evidence)
        if status_counts.get("counter_or_safety_language_blocks_effect_result_candidate"):
            status = "counter_or_safety_language_blocks_rollup"
        elif status_counts.get("effect_result_candidate_with_magnitude_and_direction_still_blocked"):
            status = "effect_result_candidate_with_magnitude_and_direction_still_blocked"
        elif status_counts.get("effect_result_candidate_direction_only_still_blocked"):
            status = "effect_result_candidate_direction_only_still_blocked"
        else:
            status = "endpoint_context_without_result_assertion_still_blocked"
        out = {
            "schema_version": 1,
            "rollup_result_gate_id": "effect-result-rollup:" + stable_id(rollup["rollup_validation_id"], status),
            "source_issue1251_rollup_validation_id": rollup["rollup_validation_id"],
            "pair_id": rollup["pair_id"],
            "pair_key": rollup["pair_key"],
            "drug_a": rollup["drug_a"],
            "drug_b": rollup["drug_b"],
            "source_local_endpoint_status": rollup["source_local_endpoint_status"],
            "effect_result_gate_status": status,
            "result_evidence_rows": len(evidence),
            "status_counts": dict(sorted(status_counts.items())),
            "source_ids": uniq([row["source_id"] for row in evidence])[:30],
            "result_review_ids": [row["result_review_id"] for row in evidence[:30]],
            "endpoint_types": dict(sorted(Counter(row["endpoint_type"] for row in evidence).items())),
            "effect_direction_counts": dict(sorted(Counter(row["effect_direction_language"] for row in evidence).items())),
            "primary_model_system_counts": dict(sorted(Counter(row["primary_model_system"] for row in evidence).items())),
            "endpoint_terms": uniq([term for row in evidence for term in row["endpoint_terms"]])[:30],
            "effect_magnitude_terms": uniq([term for row in evidence for term in row["effect_magnitude_terms"]])[:30],
            "comparator_terms": uniq([term for row in evidence for term in row["comparator_terms"]])[:30],
            "cohort_or_model_terms": uniq([term for row in evidence for term in row["cohort_or_model_terms"]])[:30],
            "safety_terms": uniq([term for row in evidence for term in row["safety_terms"]])[:30],
            "counter_or_negative_terms": uniq([term for row in evidence for term in row["counter_or_negative_terms"]])[:30],
            "promotion_status": PROMOTION_STATUS,
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        out["reason_codes"] = rollup_reason_codes(out)
        out["next_validation_experiment"] = (
            "Route through independent source-table/figure effect verification, safety/falsification, and human review."
        )
        rows.append(out)
    rows.sort(
        key=lambda row: (
            0 if row["effect_result_gate_status"] == "effect_result_candidate_with_magnitude_and_direction_still_blocked" else 1,
            row["effect_result_gate_status"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rows


def rollup_reason_codes(row: dict[str, Any]) -> list[str]:
    codes = [
        "effect_result_rollup_gate_not_clinical_actionability",
        "requires_real_effect_safety_falsification_and_human_review",
    ]
    status = row["effect_result_gate_status"]
    if status == "counter_or_safety_language_blocks_rollup":
        codes.append("safety_or_counter_language_blocks_rollup")
    elif status == "effect_result_candidate_with_magnitude_and_direction_still_blocked":
        codes.append("candidate_has_direction_and_magnitude_language_not_validated")
    elif status == "effect_result_candidate_direction_only_still_blocked":
        codes.append("candidate_has_direction_language_without_validated_magnitude")
    else:
        codes.append("source_local_endpoint_context_without_result_assertion")
    return codes


def build_bridge_rows(rollup_status: list[dict[str, Any]], result_rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        text = (
            f"Effect result gate rollup {row['pair_id']} pair {row['pair_key']} {row['drug_a']} plus "
            f"{row['drug_b']} status {row['effect_result_gate_status']} evidence rows {row['result_evidence_rows']} "
            f"promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_result_gate_id"],
                "domain": "effect_result_falsification_rollup",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["effect_result_gate_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1252_effect_result_falsification_gate",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "effect_result_gate_status": row["effect_result_gate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in result_rows[:budget]:
        text = (
            f"Effect result evidence {row['result_review_id']} source {row['source_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['effect_result_gate_status']}."
        )
        rows.append(
            {
                "id": row["result_review_id"],
                "domain": "effect_result_falsification_evidence",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["result_review_id"], row["source_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["effect_result_gate_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1252_effect_result_falsification_gate",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_id": row["source_id"],
                    "effect_result_gate_status": row["effect_result_gate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(scoped_rollups: list[dict[str, Any]], result_rows: list[dict[str, Any]], rollup_status: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "scoped_source_local_endpoint_rollups": len(scoped_rollups),
        "result_evidence_rows": len(result_rows),
        "rollup_status_rows": len(rollup_status),
        "evidence_status_counts": dict(sorted(Counter(row["effect_result_gate_status"] for row in result_rows).items())),
        "rollup_status_counts": dict(sorted(Counter(row["effect_result_gate_status"] for row in rollup_status).items())),
        "evidence_rows_with_direction_and_magnitude": sum(
            1 for row in result_rows if row["effect_result_gate_status"] == "effect_result_candidate_with_magnitude_and_direction_still_blocked"
        ),
        "evidence_rows_with_direction_only": sum(
            1 for row in result_rows if row["effect_result_gate_status"] == "effect_result_candidate_direction_only_still_blocked"
        ),
        "evidence_rows_with_safety_or_counter": sum(1 for row in result_rows if row["has_safety_or_counter_language"]),
        "evidence_rows_with_comparator": sum(1 for row in result_rows if row["has_comparator_language"]),
        "rollups_with_direction_and_magnitude": sum(
            1 for row in rollup_status if row["effect_result_gate_status"] == "effect_result_candidate_with_magnitude_and_direction_still_blocked"
        ),
        "rollups_with_safety_or_counter_block": sum(
            1 for row in rollup_status if row["effect_result_gate_status"] == "counter_or_safety_language_blocks_rollup"
        ),
        "primary_model_system_counts": dict(sorted(Counter(row["primary_model_system"] for row in result_rows).items())),
        "top_candidates": [
            {
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["effect_result_gate_status"],
                "source_ids": row["source_ids"][:5],
                "effect_magnitude_terms": row["effect_magnitude_terms"][:8],
                "comparator_terms": row["comparator_terms"][:8],
                "safety_terms": row["safety_terms"][:8],
                "counter_or_negative_terms": row["counter_or_negative_terms"][:8],
            }
            for row in rollup_status[:50]
        ],
    }


def build_input_manifest(
    inputs: dict[str, str],
    rollups: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    scoped_rollups: list[dict[str, Any]],
    issue1251_persisted_readback: dict[str, Any],
    issue1251_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1252,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1251_rollup_status": artifact(Path(inputs["issue1251_rollup_status"]), jsonl=True),
            "issue1251_evidence_review": artifact(Path(inputs["issue1251_evidence_review"]), jsonl=True),
            "issue1251_persisted_readback": artifact(Path(inputs["issue1251_persisted_readback"])),
            "issue1251_calyx_readback": artifact(Path(inputs["issue1251_calyx_readback"])),
            "issue1251_output_manifest": artifact(Path(inputs["issue1251_output_manifest"])),
        },
        "source_contract": {
            "issue1251_persisted_assertions_all_true": all_assertions_true(issue1251_persisted_readback),
            "issue1251_calyx_assertions_all_true": all_assertions_true(issue1251_calyx_readback),
            "issue1251_rollup_rows": len(rollups),
            "issue1251_evidence_rows": len(evidence_rows),
            "scoped_source_local_endpoint_rollups": len(scoped_rollups),
            "input_filter": f"source_local_endpoint_status == {INPUT_ROLLUP_STATUS}",
            "result_fields_are_triage_not_claims": True,
        },
    }


def build_readback(
    out_dir: Path,
    scoped_rollups: list[dict[str, Any]],
    result_rows: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    issue1251_persisted_readback: dict[str, Any],
    issue1251_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "effect_result_evidence_review": artifact(out_dir / "effect_result_evidence_review.jsonl", jsonl=True),
        "effect_result_rollup_status": artifact(out_dir / "effect_result_rollup_status.jsonl", jsonl=True),
        "effect_result_bridge_rows": artifact(out_dir / "effect_result_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    scoped_ids = {row["rollup_validation_id"] for row in scoped_rollups}
    status_ids = {row["source_issue1251_rollup_validation_id"] for row in rollup_status}
    result_keys = {row["pair_key"] for row in result_rows}
    status_keys = {row["pair_key"] for row in rollup_status}
    return {
        "schema_version": 1,
        "issue": 1252,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1251_persisted_readback_all_true": all_assertions_true(issue1251_persisted_readback),
            "issue1251_calyx_readback_all_true": all_assertions_true(issue1251_calyx_readback),
            "rollup_status_for_every_scoped_rollup": status_ids == scoped_ids,
            "result_rows_cover_status_pair_keys": status_keys <= result_keys,
            "all_evidence_status_values_allowed": all(row["effect_result_gate_status"] in EVIDENCE_STATUS_VALUES for row in result_rows),
            "all_rollup_status_values_allowed": all(row["effect_result_gate_status"] in ROLLUP_STATUS_VALUES for row in rollup_status),
            "all_result_rows_have_pair_term_verification": all(
                row["pair_term_presence"]["both_present_in_source_window"] for row in result_rows
            ),
            "all_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in result_rows)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in result_rows)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status),
            "bridge_rows_1000_or_less": artifacts["effect_result_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "scoped_rollups": len(scoped_rollups),
            "result_evidence_rows": len(result_rows),
            "rollup_status_rows": len(rollup_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_rollups: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    rollups = rows_jsonl(Path(inputs["issue1251_rollup_status"]))
    evidence_rows = rows_jsonl(Path(inputs["issue1251_evidence_review"]))
    scoped_rollups = [row for row in rollups if row["source_local_endpoint_status"] == INPUT_ROLLUP_STATUS]
    if max_rollups is not None:
        scoped_rollups = scoped_rollups[:max_rollups]
    scoped_pair_keys = {row["pair_key"] for row in scoped_rollups}
    issue1251_persisted_readback = read_json(Path(inputs["issue1251_persisted_readback"]))
    issue1251_calyx_readback = read_json(Path(inputs["issue1251_calyx_readback"]))
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            rollups,
            evidence_rows,
            scoped_rollups,
            issue1251_persisted_readback,
            issue1251_calyx_readback,
        ),
    )
    result_rows = build_evidence_rows(evidence_rows, scoped_pair_keys)
    write_jsonl(out_dir / "effect_result_evidence_review.jsonl", result_rows)
    rollup_status = build_rollup_status(scoped_rollups, result_rows)
    write_jsonl(out_dir / "effect_result_rollup_status.jsonl", rollup_status)
    source_path = out_dir / "effect_result_rollup_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, result_rows, source_path, source_sha)
    write_jsonl(out_dir / "effect_result_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(scoped_rollups, result_rows, rollup_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1252,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "effect_result_evidence_review": artifact(out_dir / "effect_result_evidence_review.jsonl", jsonl=True),
            "effect_result_rollup_status": artifact(out_dir / "effect_result_rollup_status.jsonl", jsonl=True),
            "effect_result_bridge_rows": artifact(out_dir / "effect_result_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        scoped_rollups,
        result_rows,
        rollup_status,
        issue1251_persisted_readback,
        issue1251_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    if not all(readback["assertions"].values()):
        raise AssertionError(f"Persisted readback assertions failed: {readback['assertions']}")
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "result_evidence": output_manifest["artifacts"]["effect_result_evidence_review"],
            "rollup_status": output_manifest["artifacts"]["effect_result_rollup_status"],
            "bridge_rows": output_manifest["artifacts"]["effect_result_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1251-rollup-status")
    parser.add_argument("--issue1251-evidence-review")
    parser.add_argument("--issue1251-persisted-readback")
    parser.add_argument("--issue1251-calyx-readback")
    parser.add_argument("--issue1251-output-manifest")
    parser.add_argument("--max-rollups", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1251_rollup_status", "issue1251_rollup_status"),
        ("issue1251_evidence_review", "issue1251_evidence_review"),
        ("issue1251_persisted_readback", "issue1251_persisted_readback"),
        ("issue1251_calyx_readback", "issue1251_calyx_readback"),
        ("issue1251_output_manifest", "issue1251_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_rollups=args.max_rollups)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
