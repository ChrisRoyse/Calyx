#!/usr/bin/env python3
"""#1239 gate #1238 PubMed structured rows through review preflight."""

from __future__ import annotations

import argparse
import importlib.util
import json
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


UTIL_PATH = Path(__file__).with_name("issue1259_rxnorm_twosides_safety_validation.py")
SPEC = importlib.util.spec_from_file_location("issue1259_utils", UTIL_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"Unable to import helpers from {UTIL_PATH}")
UTIL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UTIL)


CLINICAL_BOUNDARY = (
    "PubMed structured gate validation is safety/outcome/falsification preflight "
    "only; structured literature rows, counter-evidence rows, and missing-gate "
    "rows are blockers or review inputs, not causality, safety clearance, "
    "efficacy, treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, pair-interaction proof, or cure evidence."
)

SOURCE_DATASET = "issue1239_pubmed_structured_gate_validation"
PROMOTION_STATUS = "blocked_requires_independent_safety_outcome_falsification_and_human_review"
REQUIRED_GATES = [
    "component_safety",
    "pair_interaction",
    "outcome_endpoint",
    "falsification",
    "human_review",
]

ISSUE1238_ROOT = "/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T050000Z"

DEFAULT_INPUTS = {
    "issue1238_structured_extraction": f"{ISSUE1238_ROOT}/out/pubmed_structured_extraction.jsonl",
    "issue1238_candidate_rollup": f"{ISSUE1238_ROOT}/out/candidate_pair_pubmed_structured_rollup.jsonl",
    "issue1238_candidate_hits": f"{ISSUE1238_ROOT}/out/candidate_pair_pubmed_structured_hits.jsonl",
    "issue1238_persisted_readback": f"{ISSUE1238_ROOT}/out/persisted_readback.json",
    "issue1238_calyx_readback": f"{ISSUE1238_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1238_output_manifest": f"{ISSUE1238_ROOT}/out/output_manifest.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1238_structured_extraction": "10bc9ca90b97a089962bd85ee2f7881819668414859fdedb8040c86d78fa992e",
    "issue1238_candidate_rollup": "e0c8db57ee492727fa525c9044dfcce85bf03acbfd7a03587030ab2d8393c3e6",
    "issue1238_candidate_hits": "cb6ce01e363c6c320231cfaf01e59c9fafb891c6d437e1a9bc2f5ca924874c26",
    "issue1238_persisted_readback": "14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413",
    "issue1238_calyx_readback": "5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8",
    "issue1238_output_manifest": "8fc6728117d993bb74ea3e1090ee94136f449ded4c5c89f733ce19fe0ba9ce74",
}


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def count_rows(path: Path) -> int:
    return sum(1 for line in path.read_text(encoding="utf-8").splitlines() if line.strip())


def rows_jsonl(path: str | Path) -> list[dict[str, Any]]:
    return list(UTIL.rows_jsonl(Path(path)))


def verify_inputs(inputs: dict[str, str], skip: bool = False) -> dict[str, dict[str, Any]]:
    missing = [name for name, value in inputs.items() if not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")
    rows: dict[str, dict[str, Any]] = {}
    for name, expected in EXPECTED_INPUT_SHA256.items():
        path = Path(inputs[name])
        observed = UTIL.sha256_path(path)
        ok = observed == expected
        if not ok and not skip:
            raise RuntimeError(f"Input hash mismatch for {name}: observed {observed} expected {expected}")
        rows[name] = {
            "input_name": name,
            "path": str(path),
            "rows": count_rows(path) if path.suffix == ".jsonl" else None,
            "bytes": path.stat().st_size,
            "sha256": observed,
            "expected_sha256": expected,
            "match": ok,
        }
    return rows


def build_source_rows(input_hashes: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "schema_version": 1,
            "source_row_id": "issue1239-source:" + UTIL.stable_id(name, info["sha256"]),
            "input_name": name,
            "source_path": info["path"],
            "rows": info["rows"],
            "bytes": info["bytes"],
            "sha256": info["sha256"],
            "expected_sha256": info["expected_sha256"],
            "hash_match": info["match"],
            "classification": "sealed_input_artifact",
            "promotion_status": PROMOTION_STATUS,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        for name, info in sorted(input_hashes.items())
    ]


def evidence_classification(row: dict[str, Any]) -> str:
    relation = row.get("source_relation_class")
    if relation == "counter_evidence":
        return "counter_evidence_falsification_review_required_still_blocked"
    if row.get("has_negation_or_counterevidence_language"):
        return "pubmed_negation_language_review_input_still_blocked"
    if relation == "asserted_interaction":
        return "pubmed_pair_interaction_language_review_input_still_blocked"
    if relation == "asserted_combination":
        return "pubmed_combination_language_review_input_still_blocked"
    if relation == "asserted_outcome":
        return "pubmed_outcome_language_review_input_still_blocked"
    return "pubmed_structured_context_review_input_still_blocked"


def build_evidence_gate_rows(extractions: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for row in extractions:
        classification = evidence_classification(row)
        reason_codes = UTIL.uniq(
            list(row.get("reason_codes") or [])
            + [
                "pubmed_structured_gate_not_clinical_clearance",
                "requires_independent_safety_outcome_falsification_and_human_review",
                classification,
            ]
        )
        rows.append(
            {
                "schema_version": 1,
                "pubmed_gate_evidence_id": "issue1239-evidence-gate:" + UTIL.stable_id(row["extraction_id"], classification),
                "source_issue1238_extraction_id": row["extraction_id"],
                "source_evidence_id": row.get("source_evidence_id"),
                "source_validation_id": row.get("source_validation_id"),
                "pair_key": row.get("pair_key"),
                "representative_pair_id": row.get("representative_pair_id"),
                "drug_a": row.get("drug_a"),
                "drug_b": row.get("drug_b"),
                "pmid": row.get("pmid"),
                "source_url": row.get("source_url"),
                "source_text_sha256": row.get("source_text_sha256"),
                "source_relation_class": row.get("source_relation_class"),
                "structured_extraction_status": row.get("structured_extraction_status"),
                "gate_classification": classification,
                "primary_model_system": row.get("primary_model_system"),
                "has_safety_language": bool(row.get("has_safety_language")),
                "has_outcome_language": bool(row.get("has_outcome_language")),
                "has_dose_or_exposure_language": bool(row.get("has_dose_or_exposure_language")),
                "has_counter_evidence": bool(row.get("has_negation_or_counterevidence_language")),
                "relation_direction": row.get("relation_direction"),
                "relation_direction_basis": row.get("relation_direction_basis"),
                "safety_terms": row.get("safety_adverse_event_terms") or [],
                "outcome_terms": row.get("outcome_endpoint_terms") or [],
                "dose_exposure_terms": row.get("dose_exposure_terms") or [],
                "counter_terms": row.get("negation_counterevidence_terms") or [],
                "gate_status": "blocked_pubmed_structured_review_input_only",
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["pmid"], row["pubmed_gate_evidence_id"]))
    return rows


def available_gate_state(row: dict[str, Any], gate: str) -> tuple[str, list[str]]:
    if gate == "component_safety":
        if row.get("has_safety_language"):
            return "source_language_present_not_independent_component_safety_clearance", [
                "pubmed_safety_language_present",
                "independent_component_safety_gate_not_cleared",
            ]
        return "missing_independent_component_safety_fail_closed", ["component_safety_evidence_missing"]
    if gate == "pair_interaction":
        relation_counts = row.get("structured_relation_class_counts") or {}
        if relation_counts.get("asserted_interaction") or relation_counts.get("asserted_combination"):
            return "source_relation_present_not_independent_pair_interaction_clearance", [
                "pubmed_pair_relation_language_present",
                "independent_pair_interaction_gate_not_cleared",
            ]
        return "missing_independent_pair_interaction_fail_closed", ["pair_interaction_evidence_missing"]
    if gate == "outcome_endpoint":
        if row.get("has_outcome_language"):
            return "source_outcome_language_present_not_grounded_outcome_clearance", [
                "pubmed_outcome_language_present",
                "grounded_outcome_endpoint_gate_not_cleared",
            ]
        return "missing_grounded_outcome_endpoint_fail_closed", ["outcome_endpoint_evidence_missing"]
    if gate == "falsification":
        if row.get("has_counter_evidence"):
            return "counter_evidence_present_requires_falsification_review", [
                "counter_evidence_preserved_for_falsification_review",
                "falsification_gate_not_cleared",
            ]
        return "missing_falsification_review_fail_closed", ["falsification_review_missing"]
    if gate == "human_review":
        return "missing_human_review_fail_closed", ["human_review_missing"]
    raise ValueError(f"unknown gate {gate}")


def candidate_gate_status(row: dict[str, Any]) -> str:
    if row.get("has_counter_evidence"):
        return "blocked_counter_evidence_falsification_review_required"
    if int(row.get("eligible_structured_evidence_rows") or 0) == 0:
        return "blocked_no_structured_extraction_fail_closed"
    if row.get("has_safety_language") and row.get("has_outcome_language"):
        return "blocked_pubmed_safety_outcome_language_missing_independent_gates"
    if row.get("has_outcome_language"):
        return "blocked_pubmed_outcome_language_missing_safety_or_falsification"
    return "blocked_pubmed_structured_relation_missing_independent_gates"


def build_missing_gate_rows(rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for rollup in rollups:
        for gate in REQUIRED_GATES:
            state, reason_codes = available_gate_state(rollup, gate)
            rows.append(
                {
                    "schema_version": 1,
                    "missing_gate_id": "issue1239-missing-gate:" + UTIL.stable_id(rollup["rollup_id"], gate, state),
                    "source_issue1238_rollup_id": rollup["rollup_id"],
                    "pair_id": rollup.get("pair_id"),
                    "pair_key": rollup.get("pair_key"),
                    "drug_a": rollup.get("drug_a"),
                    "drug_b": rollup.get("drug_b"),
                    "required_gate": gate,
                    "gate_state": state,
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": UTIL.uniq(reason_codes + ["pubmed_structured_rows_do_not_clear_required_gate"]),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    rows.sort(key=lambda row: (row["pair_key"], row["required_gate"], row["missing_gate_id"]))
    return rows


def build_candidate_gate_rows(rollups: list[dict[str, Any]], missing_gate_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    missing_by_rollup: dict[str, list[dict[str, Any]]] = {}
    for row in missing_gate_rows:
        missing_by_rollup.setdefault(row["source_issue1238_rollup_id"], []).append(row)
    rows = []
    for rollup in rollups:
        status = candidate_gate_status(rollup)
        missing_rows = missing_by_rollup[rollup["rollup_id"]]
        rows.append(
            {
                "schema_version": 1,
                "candidate_pubmed_gate_status_id": "issue1239-candidate-gate:" + UTIL.stable_id(rollup["rollup_id"], status),
                "source_issue1238_rollup_id": rollup["rollup_id"],
                "source_issue1237_rollup_id": rollup.get("source_issue1237_rollup_id"),
                "pair_id": rollup.get("pair_id"),
                "pair_key": rollup.get("pair_key"),
                "drug_a": rollup.get("drug_a"),
                "drug_b": rollup.get("drug_b"),
                "eligible_structured_evidence_rows": int(rollup.get("eligible_structured_evidence_rows") or 0),
                "representative_pmids": rollup.get("representative_pmids") or [],
                "structured_relation_class_counts": rollup.get("structured_relation_class_counts") or {},
                "structured_status_counts": rollup.get("structured_status_counts") or {},
                "primary_model_system_counts": rollup.get("primary_model_system_counts") or {},
                "has_counter_evidence": bool(rollup.get("has_counter_evidence")),
                "has_safety_language": bool(rollup.get("has_safety_language")),
                "has_outcome_language": bool(rollup.get("has_outcome_language")),
                "has_dose_or_exposure_language": bool(rollup.get("has_dose_or_exposure_language")),
                "candidate_gate_status": status,
                "missing_gate_count": len([row for row in missing_rows if row["gate_state"].startswith("missing")]),
                "not_cleared_gate_count": len(missing_rows),
                "gate_states": {row["required_gate"]: row["gate_state"] for row in missing_rows},
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": UTIL.uniq(
                    list(rollup.get("reason_codes") or [])
                    + [status]
                    + [code for row in missing_rows for code in row["reason_codes"]]
                ),
                "next_validation_experiment": "Run independent component safety, exact pair-interaction, grounded outcome, falsification, and human-review gates with physical readback.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["candidate_gate_status"], row["pair_key"], row["pair_id"] or ""))
    return rows


def build_summary_rows(evidence_gate_rows: list[dict[str, Any]], candidate_gate_rows: list[dict[str, Any]], missing_gate_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "schema_version": 1,
            "summary_id": "issue1239-summary:" + UTIL.stable_id("pubmed-structured-gate", len(candidate_gate_rows), len(evidence_gate_rows)),
            "evidence_gate_rows": len(evidence_gate_rows),
            "candidate_gate_rows": len(candidate_gate_rows),
            "missing_gate_rows": len(missing_gate_rows),
            "counter_evidence_candidate_rows": sum(1 for row in candidate_gate_rows if row["has_counter_evidence"]),
            "candidate_gate_status_counts": dict(Counter(row["candidate_gate_status"] for row in candidate_gate_rows)),
            "evidence_gate_classification_counts": dict(Counter(row["gate_classification"] for row in evidence_gate_rows)),
            "required_gate_state_counts": dict(Counter(row["gate_state"] for row in missing_gate_rows)),
            "all_rows_blocked": True,
            "promotion_status": PROMOTION_STATUS,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
    ]


def bridge_metadata(source_path: str | Path, source_sha: str, **extra: Any) -> dict[str, Any]:
    metadata = {
        "source_dataset": SOURCE_DATASET,
        "source_path": str(source_path),
        "source_sha256": source_sha,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    metadata.update(extra)
    return metadata


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    evidence_gate_rows: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    summary_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows = []
    for row in source_rows:
        terms = UTIL.uniq(["issue1239", row["input_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1239_source_input",
                "text": f"Issue1239 sealed input {row['input_name']} sha256 {row['sha256']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["source_path"], row["sha256"]),
            }
        )
    for row in summary_rows:
        terms = UTIL.uniq(["issue1239", "pubmed_structured_gate_summary", row["summary_id"]])
        rows.append(
            {
                "id": row["summary_id"],
                "domain": "issue1239_gate_summary",
                "text": f"Issue1239 PubMed structured gate summary {row['summary_id']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in candidate_gate_rows:
        terms = UTIL.uniq([row["pair_key"], row["pair_id"], row["candidate_gate_status"]])
        rows.append(
            {
                "id": row["candidate_pubmed_gate_status_id"],
                "domain": "issue1239_candidate_gate_status",
                "text": (
                    f"Issue1239 candidate gate {row['pair_id']} {row['pair_key']} "
                    f"{row['candidate_gate_status']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in evidence_gate_rows:
        terms = UTIL.uniq([row["pair_key"], row["pmid"], row["gate_classification"]])
        rows.append(
            {
                "id": row["pubmed_gate_evidence_id"],
                "domain": "issue1239_pubmed_gate_evidence",
                "text": (
                    f"Issue1239 PubMed gate evidence {row['pmid']} {row['pair_key']} "
                    f"{row['gate_classification']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in missing_gate_rows:
        terms = UTIL.uniq([row["pair_key"], row["required_gate"], row["gate_state"]])
        rows.append(
            {
                "id": row["missing_gate_id"],
                "domain": "issue1239_missing_gate",
                "text": (
                    f"Issue1239 missing gate {row['pair_key']} {row['required_gate']} {row['gate_state']} "
                    f"bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
        if len(rows) >= 1000:
            break
    return rows[:1000]


def build_metrics(
    source_rows: list[dict[str, Any]],
    evidence_gate_rows: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "source_rows": len(source_rows),
        "evidence_gate_rows": len(evidence_gate_rows),
        "candidate_gate_rows": len(candidate_gate_rows),
        "missing_gate_rows": len(missing_gate_rows),
        "all_rows_blocked": True,
        "evidence_gate_classification_counts": dict(Counter(row["gate_classification"] for row in evidence_gate_rows)),
        "candidate_gate_status_counts": dict(Counter(row["candidate_gate_status"] for row in candidate_gate_rows)),
        "required_gate_state_counts": dict(Counter(row["gate_state"] for row in missing_gate_rows)),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
    }


def build_readback(
    out_dir: Path,
    input_hashes: dict[str, dict[str, Any]],
    evidence_gate_rows: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    artifact_paths = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_rows": out_dir / "source_rows.jsonl",
        "pubmed_structured_gate_evidence": out_dir / "pubmed_structured_gate_evidence.jsonl",
        "candidate_pubmed_gate_status": out_dir / "candidate_pubmed_gate_status.jsonl",
        "pubmed_missing_gate_rows": out_dir / "pubmed_missing_gate_rows.jsonl",
        "gate_summary": out_dir / "gate_summary.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
        "issue1239_bridge_rows": out_dir / "issue1239_bridge_rows.jsonl",
    }
    artifacts = {name: UTIL.artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifact_paths.items()}
    assertions = {
        "expected_input_hashes_match": all(item["match"] for item in input_hashes.values()),
        "evidence_gate_rows_510": len(evidence_gate_rows) == 510,
        "candidate_gate_rows_301": len(candidate_gate_rows) == 301,
        "missing_gate_rows_1505": len(missing_gate_rows) == 1505,
        "counter_evidence_preserved": sum(
            1
            for row in evidence_gate_rows
            if row["source_relation_class"] == "counter_evidence"
            and row["gate_classification"] == "counter_evidence_falsification_review_required_still_blocked"
        ) == 224,
        "all_evidence_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_gate_rows),
        "all_candidate_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in candidate_gate_rows),
        "all_missing_gate_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in missing_gate_rows),
        "every_candidate_has_five_required_gate_rows": all(
            len([row for row in missing_gate_rows if row["source_issue1238_rollup_id"] == candidate["source_issue1238_rollup_id"]]) == len(REQUIRED_GATES)
            for candidate in candidate_gate_rows
        ),
        "bridge_rows_1000_or_less": len(bridge_rows) <= 1000,
        "bridge_terms_present_in_text": all(
            all(UTIL.clean_text(term).lower() in UTIL.clean_text(row.get("text")).lower() for term in row.get("bridge_terms", []))
            for row in bridge_rows
        ),
        "bridge_metadata_source_dataset_present": all(
            row.get("metadata", {}).get("source_dataset") == SOURCE_DATASET for row in bridge_rows
        ),
    }
    return {
        "schema_version": 1,
        "status": "ok" if all(assertions.values()) else "failed",
        "assertions": assertions,
        "artifacts": artifacts,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--skip-input-sha-check", action="store_true")
    for key in DEFAULT_INPUTS:
        parser.add_argument(f"--{key.replace('_', '-')}")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.root)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    inputs = dict(DEFAULT_INPUTS)
    for key in list(inputs):
        override = getattr(args, key, None)
        if override:
            inputs[key] = override

    input_hashes = verify_inputs(inputs, skip=args.skip_input_sha_check)
    UTIL.write_json(out_dir / "input_manifest.json", {"schema_version": 1, "inputs": inputs, "input_hashes": input_hashes})

    source_rows = build_source_rows(input_hashes)
    evidence_gate_rows = build_evidence_gate_rows(rows_jsonl(inputs["issue1238_structured_extraction"]))
    rollups = rows_jsonl(inputs["issue1238_candidate_rollup"])
    missing_gate_rows = build_missing_gate_rows(rollups)
    candidate_gate_rows = build_candidate_gate_rows(rollups, missing_gate_rows)
    summary_rows = build_summary_rows(evidence_gate_rows, candidate_gate_rows, missing_gate_rows)

    UTIL.write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    UTIL.write_jsonl(out_dir / "pubmed_structured_gate_evidence.jsonl", evidence_gate_rows)
    UTIL.write_jsonl(out_dir / "candidate_pubmed_gate_status.jsonl", candidate_gate_rows)
    UTIL.write_jsonl(out_dir / "pubmed_missing_gate_rows.jsonl", missing_gate_rows)
    UTIL.write_jsonl(out_dir / "gate_summary.jsonl", summary_rows)

    source_path = out_dir / "candidate_pubmed_gate_status.jsonl"
    source_sha = UTIL.sha256_path(source_path)
    bridge_rows = build_bridge_rows(source_rows, evidence_gate_rows, candidate_gate_rows, missing_gate_rows, summary_rows, source_path, source_sha)
    UTIL.write_jsonl(out_dir / "issue1239_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(source_rows, evidence_gate_rows, candidate_gate_rows, missing_gate_rows, bridge_rows)
    UTIL.write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1239,
        "created_at": now_utc(),
        "inputs": inputs,
        "input_hashes": input_hashes,
        "artifacts": {
            "source_rows": UTIL.artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "pubmed_structured_gate_evidence": UTIL.artifact(out_dir / "pubmed_structured_gate_evidence.jsonl", jsonl=True),
            "candidate_pubmed_gate_status": UTIL.artifact(out_dir / "candidate_pubmed_gate_status.jsonl", jsonl=True),
            "pubmed_missing_gate_rows": UTIL.artifact(out_dir / "pubmed_missing_gate_rows.jsonl", jsonl=True),
            "gate_summary": UTIL.artifact(out_dir / "gate_summary.jsonl", jsonl=True),
            "issue1239_bridge_rows": UTIL.artifact(out_dir / "issue1239_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": UTIL.artifact(out_dir / "validation_metrics.json"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    UTIL.write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, input_hashes, evidence_gate_rows, candidate_gate_rows, missing_gate_rows, bridge_rows)
    UTIL.write_json(out_dir / "persisted_readback.json", readback)
    if readback["status"] != "ok":
        raise RuntimeError(f"Persisted readback failed: {readback['assertions']}")

    print(
        json.dumps(
            {
                "status": "ok",
                "root": str(root),
                "metrics": metrics,
                "artifacts": {
                    "pubmed_structured_gate_evidence": UTIL.artifact(out_dir / "pubmed_structured_gate_evidence.jsonl", jsonl=True),
                    "candidate_pubmed_gate_status": UTIL.artifact(out_dir / "candidate_pubmed_gate_status.jsonl", jsonl=True),
                    "pubmed_missing_gate_rows": UTIL.artifact(out_dir / "pubmed_missing_gate_rows.jsonl", jsonl=True),
                    "bridge_rows": UTIL.artifact(out_dir / "issue1239_bridge_rows.jsonl", jsonl=True),
                    "persisted_readback": UTIL.artifact(out_dir / "persisted_readback.json"),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
