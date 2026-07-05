#!/usr/bin/env python3
"""#1241 validate #1236 openFDA label hits through safety/interaction gates."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
from collections import Counter, defaultdict
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
    "openFDA label gate validation is safety/source/falsification triage only; "
    "label safety rows, interaction-section rows, co-mentions, and false-positive "
    "context rows are blockers or review inputs, not causality, safety clearance, "
    "efficacy, treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, pair-interaction proof, or cure evidence."
)

SOURCE_DATASET = "issue1241_openfda_label_gate_validation"
PROMOTION_STATUS = "blocked_requires_independent_safety_outcome_falsification_and_human_review"
DIRECT_PAIR_WINDOW_TOKENS = 42
FALSE_POSITIVE_DISTANCE_TOKENS = 240

ISSUE1236_ROOT = "/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1241-openfda-label-gate-validation-20260705T040000Z"

DEFAULT_INPUTS = {
    "issue1236_pair_evidence": f"{ISSUE1236_ROOT}/out/openfda_label_pair_evidence.jsonl",
    "issue1236_candidate_status": f"{ISSUE1236_ROOT}/out/candidate_openfda_label_status.jsonl",
    "issue1236_pair_status": f"{ISSUE1236_ROOT}/out/openfda_label_pair_status.jsonl",
    "issue1236_persisted_readback": f"{ISSUE1236_ROOT}/out/persisted_readback.json",
    "issue1236_calyx_readback": f"{ISSUE1236_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1236_output_manifest": f"{ISSUE1236_ROOT}/out/output_manifest.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1236_pair_evidence": "d2376732cea19e4f17d4f803e5e7c0b07cbd6ef4898fb7b527a6a1509c304ab3",
    "issue1236_candidate_status": "0e8893d8bef722f66ae656ece2335a7f7eabcb226bf429c6658b0e5ad9ee791b",
    "issue1236_pair_status": "9a1680d5243a90ab52af7bb93b76f8e7677c7b7cef338282bf1cd781b5d7524a",
    "issue1236_persisted_readback": "9e738bb57b5cf53d54981c82d61ae5bb44181d667bc19b23a15c5cde838677db",
    "issue1236_calyx_readback": "5bf8cc3e01e7c55d70d300b6a87f1a167e1d6d36e8257383b722d66ec4db3b6e",
    "issue1236_output_manifest": "c69cb405c3d36dce0536eb37361937aecaf4fd9d0d82d041a83b398053934420",
}

SAFETY_FIELDS = {
    "boxed_warning",
    "contraindications",
    "warnings",
    "warnings_and_cautions",
    "precautions",
    "adverse_reactions",
    "adverse_reactions_table",
}

INTERACTION_FIELDS = {
    "drug_interactions",
    "drug_interactions_table",
    "clinical_pharmacology",
    "pharmacodynamics",
    "pharmacokinetics",
}

PAIR_CONNECTORS = {
    "with",
    "and",
    "plus",
    "coadministered",
    "coadministration",
    "concomitant",
    "combination",
    "combined",
    "interaction",
    "interactions",
    "administered",
}

SAFETY_TERMS = {
    "adverse",
    "contraindicated",
    "contraindication",
    "warning",
    "warnings",
    "caution",
    "monitor",
    "toxicity",
    "bleeding",
    "risk",
    "serious",
    "reaction",
    "reactions",
    "dose",
    "exposure",
}


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def count_rows(path: Path) -> int:
    return sum(1 for line in path.read_text(encoding="utf-8").splitlines() if line.strip())


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


def rows_jsonl(path: str | Path) -> list[dict[str, Any]]:
    return list(UTIL.rows_jsonl(Path(path)))


def tokens(value: str) -> list[str]:
    return re.sub(r"[^a-z0-9]+", " ", UTIL.clean_text(value).lower()).split()


def name_tokens(value: str) -> list[str]:
    return tokens(UTIL.norm_name(value))


def token_positions(haystack: list[str], needle: list[str]) -> list[int]:
    if not needle:
        return []
    return [idx for idx in range(0, len(haystack) - len(needle) + 1) if haystack[idx : idx + len(needle)] == needle]


def min_pair_distance(section: dict[str, Any], left: str, right: str) -> int | None:
    section_tokens = tokens(section.get("snippet") or "")
    left_positions = token_positions(section_tokens, name_tokens(left))
    right_positions = token_positions(section_tokens, name_tokens(right))
    if not left_positions or not right_positions:
        return None
    return min(abs(left - right) for left in left_positions for right in right_positions)


def connector_between(section: dict[str, Any], left: str, right: str) -> bool:
    section_tokens = tokens(section.get("snippet") or "")
    left_positions = token_positions(section_tokens, name_tokens(left))
    right_positions = token_positions(section_tokens, name_tokens(right))
    for left_pos in left_positions:
        for right_pos in right_positions:
            lo = min(left_pos + len(name_tokens(left)), right_pos + len(name_tokens(right)))
            hi = max(left_pos, right_pos)
            if hi < lo:
                continue
            window = section_tokens[lo:hi]
            if len(window) <= DIRECT_PAIR_WINDOW_TOKENS and PAIR_CONNECTORS.intersection(window):
                return True
    return False


def section_features(row: dict[str, Any]) -> list[dict[str, Any]]:
    features = []
    for section in row.get("matched_sections") or []:
        field = UTIL.clean_text(section.get("field"))
        section_tokens = tokens(section.get("snippet") or "")
        distance = min_pair_distance(section, row["drug_a"], row["drug_b"])
        features.append(
            {
                "field": field,
                "is_safety_section": bool(section.get("is_safety_section")) or field in SAFETY_FIELDS,
                "is_interaction_section": bool(section.get("is_interaction_section")) or field in INTERACTION_FIELDS,
                "min_pair_token_distance": distance,
                "connector_between_pair_terms": connector_between(section, row["drug_a"], row["drug_b"]),
                "safety_terms": sorted(SAFETY_TERMS.intersection(section_tokens)),
                "section_text_sha256": section.get("section_text_sha256"),
                "snippet_sha256": UTIL.sha256_bytes(UTIL.clean_text(section.get("snippet")).encode("utf-8")),
            }
        )
    return features


def classify_evidence(row: dict[str, Any]) -> tuple[str, list[str], dict[str, Any]]:
    features = section_features(row)
    direct_sections = [
        feature
        for feature in features
        if feature["is_interaction_section"]
        and feature["min_pair_token_distance"] is not None
        and feature["min_pair_token_distance"] <= DIRECT_PAIR_WINDOW_TOKENS
        and feature["connector_between_pair_terms"]
    ]
    close_list_sections = [
        feature
        for feature in features
        if feature["min_pair_token_distance"] is not None
        and feature["min_pair_token_distance"] <= DIRECT_PAIR_WINDOW_TOKENS
        and not feature["connector_between_pair_terms"]
    ]
    safety_sections = [feature for feature in features if feature["is_safety_section"] or feature["safety_terms"]]
    distances = [feature["min_pair_token_distance"] for feature in features if feature["min_pair_token_distance"] is not None]
    min_distance = min(distances) if distances else None

    reason_codes = [
        "openfda_label_gate_not_clinical_clearance",
        "requires_independent_safety_outcome_falsification_and_human_review",
    ]
    if direct_sections:
        classification = "pair_interaction_language_review_blocker"
        reason_codes.append("label_has_close_connector_pair_interaction_language")
    elif safety_sections and row.get("safety_section_match"):
        classification = "component_specific_safety_language_review_blocker"
        reason_codes.append("label_has_safety_section_language_without_pair_clearance")
    elif min_distance is None or min_distance > FALSE_POSITIVE_DISTANCE_TOKENS:
        classification = "likely_false_positive_context_blocker"
        reason_codes.append("label_pair_terms_missing_or_too_distant_in_review_snippet")
    elif close_list_sections:
        classification = "broad_label_list_comention_blocker"
        reason_codes.append("label_terms_are_close_list_members_not_pair_interaction")
    else:
        classification = "broad_label_comention_blocker"
        reason_codes.append("label_has_broad_section_comention_without_direct_pair_language")
    if row.get("interaction_section_match"):
        reason_codes.append("label_interaction_section_match_requires_review")
    if row.get("safety_section_match"):
        reason_codes.append("label_safety_section_match_requires_review")
    return classification, UTIL.uniq(reason_codes), {
        "section_features": features,
        "min_pair_token_distance": min_distance,
        "direct_pair_section_count": len(direct_sections),
        "close_list_section_count": len(close_list_sections),
        "safety_section_count": len(safety_sections),
    }


def aggregate_candidate_status(classifications: list[str]) -> str:
    if any(value == "pair_interaction_language_review_blocker" for value in classifications):
        return "blocked_label_pair_interaction_review_required"
    if any(value == "component_specific_safety_language_review_blocker" for value in classifications):
        return "blocked_component_safety_label_review_required"
    if all(value == "likely_false_positive_context_blocker" for value in classifications):
        return "blocked_likely_false_positive_label_context"
    return "blocked_broad_label_comention_only"


def build_source_rows(input_hashes: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "schema_version": 1,
            "source_row_id": "issue1241-source:" + UTIL.stable_id(name, info["sha256"]),
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


def build_evidence_gate_rows(evidence_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for evidence in evidence_rows:
        classification, reason_codes, features = classify_evidence(evidence)
        rows.append(
            {
                "schema_version": 1,
                "evidence_gate_id": "issue1241-evidence-gate:" + UTIL.stable_id(evidence["evidence_id"], classification),
                "source_issue1236_evidence_id": evidence["evidence_id"],
                "pair_key": evidence["pair_key"],
                "drug_a": evidence["drug_a"],
                "drug_b": evidence["drug_b"],
                "openfda_label_id": evidence.get("openfda_label_id"),
                "dailymed_url": evidence.get("dailymed_url"),
                "source_url": evidence.get("source_url"),
                "label_result_sha256": evidence.get("label_result_sha256"),
                "match_kind": evidence.get("match_kind"),
                "matched_section_fields": evidence.get("matched_section_fields") or [],
                "safety_section_match": bool(evidence.get("safety_section_match")),
                "interaction_section_match": bool(evidence.get("interaction_section_match")),
                "gate_classification": classification,
                "gate_status": "blocked_label_gate_review_input_only",
                "evidence_role": "counter_or_negative_context" if classification == "likely_false_positive_context_blocker" else "supporting_review_input",
                "min_pair_token_distance": features["min_pair_token_distance"],
                "direct_pair_section_count": features["direct_pair_section_count"],
                "close_list_section_count": features["close_list_section_count"],
                "safety_section_count": features["safety_section_count"],
                "section_features": features["section_features"],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["gate_classification"], row["source_issue1236_evidence_id"]))
    return rows


def build_pair_rollups(pair_status_rows: list[dict[str, Any]], evidence_gate_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    gates_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_gate_rows:
        gates_by_pair[row["pair_key"]].append(row)
    rows = []
    for pair in pair_status_rows:
        gates = gates_by_pair.get(pair["pair_key"], [])
        if not gates:
            continue
        classifications = [row["gate_classification"] for row in gates]
        gate_status = aggregate_candidate_status(classifications)
        rows.append(
            {
                "schema_version": 1,
                "pair_label_gate_rollup_id": "issue1241-pair-rollup:" + UTIL.stable_id(pair["pair_status_id"], gate_status),
                "source_issue1236_pair_status_id": pair["pair_status_id"],
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "candidate_pair_ids": pair.get("candidate_pair_ids") or [],
                "candidate_row_count": pair.get("candidate_row_count"),
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "issue1236_openfda_label_status": pair.get("openfda_label_status"),
                "label_evidence_rows": len(gates),
                "gate_classification_counts": dict(Counter(classifications)),
                "gate_status": gate_status,
                "all_evidence_blocked": True,
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": UTIL.uniq([code for row in gates for code in row["reason_codes"]] + [gate_status]),
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["pair_label_gate_rollup_id"]))
    return rows


def build_candidate_gate_rows(candidate_status_rows: list[dict[str, Any]], pair_rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rollup_by_pair = {row["pair_key"]: row for row in pair_rollups}
    rows = []
    for candidate in candidate_status_rows:
        if candidate.get("openfda_label_status") == "no_external_hit":
            continue
        rollup = rollup_by_pair.get(candidate["pair_key"])
        if not rollup:
            raise RuntimeError(f"Missing pair rollup for hit candidate {candidate['pair_id']} {candidate['pair_key']}")
        rows.append(
            {
                "schema_version": 1,
                "candidate_label_gate_status_id": "issue1241-candidate-gate:" + UTIL.stable_id(candidate["status_id"], rollup["gate_status"]),
                "source_issue1236_candidate_status_id": candidate["status_id"],
                "pair_id": candidate["pair_id"],
                "pair_key": candidate["pair_key"],
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "source_issue1236_pair_status_id": candidate.get("pair_status_id"),
                "pair_label_gate_rollup_id": rollup["pair_label_gate_rollup_id"],
                "issue1236_openfda_label_status": candidate.get("openfda_label_status"),
                "label_evidence_rows": rollup["label_evidence_rows"],
                "gate_classification_counts": rollup["gate_classification_counts"],
                "candidate_gate_status": rollup["gate_status"],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": rollup["reason_codes"],
                "next_validation_experiment": "Acquire independent component safety, exact pair-interaction, outcome, falsification, and human-review evidence before any promotion.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["pair_id"]))
    return rows


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
    pair_rollups: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows = []
    for row in source_rows:
        terms = UTIL.uniq(["issue1241", row["input_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1241_source_input",
                "text": f"Issue1241 sealed input {row['input_name']} sha256 {row['sha256']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["source_path"], row["sha256"]),
            }
        )
    for row in evidence_gate_rows:
        terms = UTIL.uniq([row["pair_key"], row["openfda_label_id"], row["gate_classification"], row["gate_status"]])
        rows.append(
            {
                "id": row["evidence_gate_id"],
                "domain": "issue1241_label_evidence_gate",
                "text": (
                    f"Issue1241 label evidence gate {row['source_issue1236_evidence_id']} {row['pair_key']} "
                    f"{row['gate_classification']} {row['gate_status']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in pair_rollups:
        terms = UTIL.uniq([row["pair_key"], row["gate_status"], row["drug_a"], row["drug_b"]])
        rows.append(
            {
                "id": row["pair_label_gate_rollup_id"],
                "domain": "issue1241_pair_label_gate_rollup",
                "text": (
                    f"Issue1241 pair label gate rollup {row['pair_key']} {row['drug_a']} {row['drug_b']} "
                    f"{row['gate_status']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in candidate_gate_rows:
        terms = UTIL.uniq([row["pair_key"], row["pair_id"], row["candidate_gate_status"]])
        rows.append(
            {
                "id": row["candidate_label_gate_status_id"],
                "domain": "issue1241_candidate_label_gate_status",
                "text": (
                    f"Issue1241 candidate label gate status {row['pair_id']} {row['pair_key']} "
                    f"{row['candidate_gate_status']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    return rows


def build_metrics(
    source_rows: list[dict[str, Any]],
    evidence_gate_rows: list[dict[str, Any]],
    pair_rollups: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "source_rows": len(source_rows),
        "label_evidence_gate_rows": len(evidence_gate_rows),
        "pair_label_gate_rollup_rows": len(pair_rollups),
        "candidate_label_gate_status_rows": len(candidate_gate_rows),
        "all_rows_blocked": True,
        "gate_classification_counts": dict(Counter(row["gate_classification"] for row in evidence_gate_rows)),
        "candidate_gate_status_counts": dict(Counter(row["candidate_gate_status"] for row in candidate_gate_rows)),
        "pair_gate_status_counts": dict(Counter(row["gate_status"] for row in pair_rollups)),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
    }


def build_readback(
    out_dir: Path,
    input_hashes: dict[str, dict[str, Any]],
    evidence_gate_rows: list[dict[str, Any]],
    pair_rollups: list[dict[str, Any]],
    candidate_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    artifact_paths = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_rows": out_dir / "source_rows.jsonl",
        "label_evidence_gate_rows": out_dir / "label_evidence_gate_rows.jsonl",
        "pair_label_gate_rollups": out_dir / "pair_label_gate_rollups.jsonl",
        "candidate_label_gate_status": out_dir / "candidate_label_gate_status.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
        "issue1241_bridge_rows": out_dir / "issue1241_bridge_rows.jsonl",
    }
    artifacts = {name: UTIL.artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifact_paths.items()}
    assertions = {
        "expected_input_hashes_match": all(item["match"] for item in input_hashes.values()),
        "evidence_gate_rows_40": len(evidence_gate_rows) == 40,
        "candidate_gate_rows_22": len(candidate_gate_rows) == 22,
        "pair_rollup_rows_9": len(pair_rollups) == 9,
        "all_evidence_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_gate_rows),
        "all_candidate_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in candidate_gate_rows),
        "all_pair_rollups_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in pair_rollups),
        "every_candidate_has_rollup": all(row.get("pair_label_gate_rollup_id") for row in candidate_gate_rows),
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
    evidence_gate_rows = build_evidence_gate_rows(rows_jsonl(inputs["issue1236_pair_evidence"]))
    pair_rollups = build_pair_rollups(rows_jsonl(inputs["issue1236_pair_status"]), evidence_gate_rows)
    candidate_gate_rows = build_candidate_gate_rows(rows_jsonl(inputs["issue1236_candidate_status"]), pair_rollups)

    UTIL.write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    UTIL.write_jsonl(out_dir / "label_evidence_gate_rows.jsonl", evidence_gate_rows)
    UTIL.write_jsonl(out_dir / "pair_label_gate_rollups.jsonl", pair_rollups)
    UTIL.write_jsonl(out_dir / "candidate_label_gate_status.jsonl", candidate_gate_rows)

    source_path = out_dir / "candidate_label_gate_status.jsonl"
    source_sha = UTIL.sha256_path(source_path)
    bridge_rows = build_bridge_rows(source_rows, evidence_gate_rows, pair_rollups, candidate_gate_rows, source_path, source_sha)
    UTIL.write_jsonl(out_dir / "issue1241_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(source_rows, evidence_gate_rows, pair_rollups, candidate_gate_rows, bridge_rows)
    UTIL.write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1241,
        "created_at": now_utc(),
        "inputs": inputs,
        "input_hashes": input_hashes,
        "artifacts": {
            "source_rows": UTIL.artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "label_evidence_gate_rows": UTIL.artifact(out_dir / "label_evidence_gate_rows.jsonl", jsonl=True),
            "pair_label_gate_rollups": UTIL.artifact(out_dir / "pair_label_gate_rollups.jsonl", jsonl=True),
            "candidate_label_gate_status": UTIL.artifact(out_dir / "candidate_label_gate_status.jsonl", jsonl=True),
            "issue1241_bridge_rows": UTIL.artifact(out_dir / "issue1241_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": UTIL.artifact(out_dir / "validation_metrics.json"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    UTIL.write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, input_hashes, evidence_gate_rows, pair_rollups, candidate_gate_rows, bridge_rows)
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
                    "label_evidence_gate_rows": UTIL.artifact(out_dir / "label_evidence_gate_rows.jsonl", jsonl=True),
                    "pair_label_gate_rollups": UTIL.artifact(out_dir / "pair_label_gate_rollups.jsonl", jsonl=True),
                    "candidate_label_gate_status": UTIL.artifact(out_dir / "candidate_label_gate_status.jsonl", jsonl=True),
                    "bridge_rows": UTIL.artifact(out_dir / "issue1241_bridge_rows.jsonl", jsonl=True),
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
