#!/usr/bin/env python3
"""#1233 validate CDCDB-supported combination rows through gates."""

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


ISSUE1231_ROOT = "/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1233-cdcdb-gate-validation-20260705T070000Z"

CLINICAL_BOUNDARY = (
    "CDCDB source-context validation is external combination-source triage only; "
    "ClinicalTrials.gov, Orange Book, patent, exact-pair, and multi-drug context "
    "rows are blockers or review inputs, not synergy, pair-interaction proof, "
    "efficacy, safety clearance, dosing guidance, treatment guidance, "
    "recommendation, clinical actionability, or cure evidence."
)

SOURCE_DATASET = "issue1233_cdcdb_gate_validation"
PROMOTION_STATUS = "blocked_requires_independent_safety_pair_interaction_outcome_and_human_review"

DEFAULT_INPUTS = {
    "issue1231_candidate_hits": f"{ISSUE1231_ROOT}/out/candidate_external_combo_hits.jsonl",
    "issue1231_candidate_status": f"{ISSUE1231_ROOT}/out/candidate_external_combo_status.jsonl",
    "issue1231_prior_no_hit_recheck": f"{ISSUE1231_ROOT}/out/prior_no_hit_recheck_status.jsonl",
    "issue1231_cdcdb_source_combinations": f"{ISSUE1231_ROOT}/out/cdcdb_source_combinations.jsonl",
    "issue1231_cdcdb_pair_index": f"{ISSUE1231_ROOT}/out/cdcdb_pair_index.jsonl",
    "issue1231_cdcdb_source_schema": f"{ISSUE1231_ROOT}/out/cdcdb_source_schema.json",
    "issue1231_validation_metrics": f"{ISSUE1231_ROOT}/out/validation_metrics.json",
    "issue1231_persisted_readback": f"{ISSUE1231_ROOT}/out/persisted_readback.json",
    "issue1231_calyx_readback": f"{ISSUE1231_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1231_output_manifest": f"{ISSUE1231_ROOT}/out/output_manifest.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1231_candidate_hits": "c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a",
    "issue1231_candidate_status": "f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4",
    "issue1231_prior_no_hit_recheck": "1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0",
    "issue1231_cdcdb_source_combinations": "004b07ee5b308501d048a8a919d6ce7af9dcd086897b25b777c17d43d6a32f79",
    "issue1231_cdcdb_pair_index": "55d77873ae1c7cd1a5d670c82050de6b22134f419fafccdd3a7edf7742c75714",
    "issue1231_cdcdb_source_schema": "d5f27a3c635bd23984dd303fff882e61c42c9b6e828df324950bdc88349ac7f2",
    "issue1231_validation_metrics": "ebdc0863663e14ebb9bc96d84fe9d5787a4b14c279f5bb5ca34c37750c1f041d",
    "issue1231_persisted_readback": "e730a41a3223fc517af5dd91ea3cb12c2799520926f7b3ec7abeefa6052ae825",
    "issue1231_calyx_readback": "2333f2a8423700fdd14336d6a94af669dabeee648f09c67395310d7b614269db",
    "issue1231_output_manifest": "b62fdabdfff60d34ee17d3dc983c8f2192a91add3bb100775df3361ac58ddf29",
}

ALLOWED_VALIDATION_STATUSES = {
    "blocked_no_safety",
    "blocked_component_safety_review",
    "blocked_no_pair_interaction",
    "blocked_no_outcome",
    "source_context_only",
}


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def rows_jsonl(path: str | Path) -> list[dict[str, Any]]:
    return list(UTIL.rows_jsonl(Path(path)))


def write_json(path: Path, value: Any) -> None:
    UTIL.write_json(path, value)


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    UTIL.write_jsonl(path, rows)


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    return UTIL.artifact(path, jsonl=jsonl)


def count_jsonl(path: Path) -> int:
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(bool(item) for item in assertions.values())


def verify_inputs(inputs: dict[str, str], skip: bool = False) -> list[dict[str, Any]]:
    missing = [name for name, value in inputs.items() if not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")
    rows: list[dict[str, Any]] = []
    for name in sorted(EXPECTED_INPUT_SHA256):
        expected = EXPECTED_INPUT_SHA256[name]
        path = Path(inputs[name])
        observed = UTIL.sha256_path(path)
        ok = observed == expected
        if not ok and not skip:
            raise RuntimeError(f"Input hash mismatch for {name}: observed {observed} expected {expected}")
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": "issue1233-source:" + UTIL.stable_id(name, observed),
                "input_name": name,
                "path": str(path),
                "rows": count_jsonl(path) if path.suffix == ".jsonl" else None,
                "bytes": path.stat().st_size,
                "sha256": observed,
                "expected_sha256": expected,
                "hash_match": ok,
                "classification": "sealed_issue1231_input",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def source_context_class(source_types: list[str]) -> str:
    values = sorted(source_types)
    if not values:
        return "no_cdcdb_source_context"
    if values == ["clinicaltrials.gov"]:
        return "clinicaltrials_registry_context"
    if values == ["patents"]:
        return "patent_context"
    if values == ["orangebook"]:
        return "orange_book_context"
    return "mixed_" + "_".join(value.replace(".", "").replace(" ", "_") for value in values)


def pair_record_class(summary: dict[str, Any]) -> str:
    two = int(summary.get("two_drug_record_count") or 0)
    multi = int(summary.get("multi_drug_context_count") or 0)
    if two and multi:
        return "two_drug_and_multi_drug_context"
    if two:
        return "exact_two_drug_source_record"
    if multi:
        return "multi_drug_context_only"
    return "source_summary_without_record_count"


def gate_values(row: dict[str, Any]) -> dict[str, str]:
    reasons = set(row.get("reason_codes") or [])
    if "component_safety_missing_fail_closed" in reasons:
        component = "component_safety_missing_fail_closed"
    elif "overlapping_component_safety_flags_review_required" in reasons:
        component = "component_safety_flags_review_required_not_clearance"
    else:
        component = "component_safety_not_independently_validated_still_blocked"
    if "pair_interaction_evidence_missing_fail_closed" in reasons:
        pair = "pair_interaction_evidence_missing_fail_closed"
    else:
        pair = "cdcdb_source_context_not_pair_interaction_or_synergy_proof"
    return {
        "component_safety": component,
        "pair_interaction": pair,
        "outcome_endpoint": "grounded_outcome_endpoint_missing_fail_closed",
        "human_review": "missing_human_review_fail_closed",
    }


def validation_status(row: dict[str, Any]) -> str:
    gates = gate_values(row)
    if gates["component_safety"] == "component_safety_missing_fail_closed":
        return "blocked_no_safety"
    if gates["component_safety"] == "component_safety_flags_review_required_not_clearance":
        return "blocked_component_safety_review"
    if gates["pair_interaction"] == "pair_interaction_evidence_missing_fail_closed":
        return "blocked_no_pair_interaction"
    if row.get("cdcdb_match"):
        return "blocked_no_outcome"
    return "source_context_only"


def build_source_context_rows(hits: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for hit in hits:
        summary = hit.get("cdcdb_summary") or {}
        examples = summary.get("examples") or []
        if not examples:
            examples = [{}]
        for index, example in enumerate(examples):
            source_type = UTIL.clean_text(example.get("source")) or "unknown"
            combo_size = int(example.get("source_combination_size") or 0)
            rows.append(
                {
                    "schema_version": 1,
                    "source_context_id": "issue1233-context:" + UTIL.stable_id(hit["pair_id"], index, example.get("combo_id")),
                    "pair_id": hit["pair_id"],
                    "pair_key": hit["pair_key"],
                    "drug_a": hit["drug_a"],
                    "drug_b": hit["drug_b"],
                    "disease": hit.get("disease"),
                    "disease_area": hit.get("disease_area"),
                    "cdcdb_source_context_class": source_context_class(hit.get("cdcdb_source_types") or []),
                    "cdcdb_pair_record_class": pair_record_class(summary),
                    "cdcdb_external_combo_status": hit.get("cdcdb_external_combo_status"),
                    "source_type": source_type,
                    "source_id": example.get("source_id"),
                    "combo_id": example.get("combo_id"),
                    "source_combination_size": combo_size,
                    "is_exact_two_drug_source_record": combo_size == 2,
                    "is_multi_drug_context_record": combo_size > 2,
                    "left_primary": example.get("left_primary"),
                    "right_primary": example.get("right_primary"),
                    "left_aliases": example.get("left_aliases") or [],
                    "right_aliases": example.get("right_aliases") or [],
                    "left_drugbank_ids": example.get("left_drugbank_ids") or [],
                    "right_drugbank_ids": example.get("right_drugbank_ids") or [],
                    "left_pubchem_ids": example.get("left_pubchem_ids") or [],
                    "right_pubchem_ids": example.get("right_pubchem_ids") or [],
                    "source_context_gate": "cdcdb_source_context_not_clinical_or_pair_interaction_clearance",
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    rows.sort(key=lambda item: (item["pair_key"], item["pair_id"], item["source_type"], item["source_id"] or "", item["source_context_id"]))
    return rows


def build_pair_status_rows(hits: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for hit in hits:
        summary = hit.get("cdcdb_summary") or {}
        gates = gate_values(hit)
        status = validation_status(hit)
        source_types = hit.get("cdcdb_source_types") or []
        reason_codes = UTIL.uniq(
            [
                *(hit.get("reason_codes") or []),
                status,
                "cdcdb_source_context_not_clearance",
                "cdcdb_not_synergy_or_pair_interaction_proof",
                "human_review_required",
            ]
        )
        rows.append(
            {
                "schema_version": 1,
                "pair_validation_id": "issue1233-pair:" + UTIL.stable_id(hit["pair_id"], status),
                "source_issue1231_pair_id": hit["pair_id"],
                "pair_id": hit["pair_id"],
                "pair_key": hit["pair_key"],
                "drug_a": hit["drug_a"],
                "drug_b": hit["drug_b"],
                "disease": hit.get("disease"),
                "disease_area": hit.get("disease_area"),
                "validation_status": status,
                "cdcdb_external_combo_status": hit.get("cdcdb_external_combo_status"),
                "overall_external_evidence_status": hit.get("overall_external_evidence_status"),
                "source_context_class": source_context_class(source_types),
                "pair_record_class": pair_record_class(summary),
                "source_types": source_types,
                "source_type_counts": summary.get("source_type_counts") or {},
                "source_record_count": int(summary.get("source_record_count") or 0),
                "two_drug_record_count": int(summary.get("two_drug_record_count") or 0),
                "multi_drug_context_count": int(summary.get("multi_drug_context_count") or 0),
                "example_source_ids": [example.get("source_id") for example in (summary.get("examples") or [])[:20]],
                "component_safety_gate": gates["component_safety"],
                "pair_interaction_gate": gates["pair_interaction"],
                "outcome_gate": gates["outcome_endpoint"],
                "human_review_gate": gates["human_review"],
                "prior_issue1229_external_synergy_status": hit.get("prior_issue1229_external_synergy_status"),
                "prior_issue1229_drugcomb_match": bool(hit.get("prior_issue1229_drugcomb_match")),
                "prior_issue1229_almanac_match": bool(hit.get("prior_issue1229_almanac_match")),
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda item: (item["validation_status"], item["pair_key"], item["pair_id"]))
    return rows


def build_missing_gate_rows(pair_status: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        gates = {
            "component_safety": row["component_safety_gate"],
            "pair_interaction": row["pair_interaction_gate"],
            "outcome_endpoint": row["outcome_gate"],
            "human_review": row["human_review_gate"],
        }
        for gate, gate_status in gates.items():
            rows.append(
                {
                    "schema_version": 1,
                    "missing_gate_id": "issue1233-gate:" + UTIL.stable_id(row["pair_id"], gate, gate_status),
                    "pair_validation_id": row["pair_validation_id"],
                    "pair_id": row["pair_id"],
                    "pair_key": row["pair_key"],
                    "drug_a": row["drug_a"],
                    "drug_b": row["drug_b"],
                    "gate": gate,
                    "gate_status": gate_status,
                    "validation_status": row["validation_status"],
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    rows.sort(key=lambda item: (item["pair_key"], item["gate"], item["pair_id"]))
    return rows


def build_bridge_rows(pair_status: list[dict[str, Any]], source_context: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        text = (
            f"CDCDB gate validation pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"source context {row['source_context_class']} record class {row['pair_record_class']} "
            f"status {row['validation_status']} source records {row['source_record_count']}."
        )
        rows.append(
            {
                "id": row["pair_validation_id"],
                "domain": "cdcdb_pair_gate_status",
                "text": text,
                "bridge_terms": [
                    term
                    for term in UTIL.uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["validation_status"], row["source_context_class"], row["pair_record_class"]])
                    if term and UTIL.clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": SOURCE_DATASET,
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "validation_status": row["validation_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in source_context[:budget]:
        text = (
            f"CDCDB source context {row['source_context_id']} pair {row['pair_key']} "
            f"source {row['source_type']} source id {row.get('source_id') or 'none'} "
            f"combination size {row['source_combination_size']} class {row['cdcdb_pair_record_class']}."
        )
        rows.append(
            {
                "id": row["source_context_id"],
                "domain": "cdcdb_source_context",
                "text": text,
                "bridge_terms": [
                    term
                    for term in UTIL.uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_type"], row.get("source_id"), row["cdcdb_pair_record_class"]])
                    if term and UTIL.clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": SOURCE_DATASET,
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_type": row["source_type"],
                    "source_id": row.get("source_id"),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    hits: list[dict[str, Any]],
    prior_no_hit_rows: list[dict[str, Any]],
    source_context: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "created_at": now_utc(),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "cdcdb_hit_rows_checked": len(hits),
        "prior_no_hit_recheck_rows_checked": len(prior_no_hit_rows),
        "prior_no_hit_cdcdb_hit_rows": sum(1 for row in hits if row.get("prior_issue1229_external_synergy_status") == "no_external_hit"),
        "source_context_rows": len(source_context),
        "pair_validation_status_rows": len(pair_status),
        "missing_gate_rows": len(missing_gate_rows),
        "bridge_rows": len(bridge_rows),
        "unique_pair_keys": len({row["pair_key"] for row in pair_status}),
        "validation_status_counts": dict(sorted(Counter(row["validation_status"] for row in pair_status).items())),
        "source_context_class_counts": dict(sorted(Counter(row["source_context_class"] for row in pair_status).items())),
        "pair_record_class_counts": dict(sorted(Counter(row["pair_record_class"] for row in pair_status).items())),
        "cdcdb_external_combo_status_counts": dict(sorted(Counter(row["cdcdb_external_combo_status"] for row in pair_status).items())),
        "source_type_context_rows": dict(sorted(Counter(row["source_type"] for row in source_context).items())),
        "gate_status_counts": dict(sorted(Counter(row["gate_status"] for row in missing_gate_rows).items())),
        "two_drug_pair_rows": sum(1 for row in pair_status if row["two_drug_record_count"] > 0),
        "multi_drug_only_pair_rows": sum(1 for row in pair_status if row["two_drug_record_count"] == 0 and row["multi_drug_context_count"] > 0),
    }


def build_readback(
    out_dir: Path,
    source_rows: list[dict[str, Any]],
    hits: list[dict[str, Any]],
    prior_no_hit_rows: list[dict[str, Any]],
    source_context: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1231_persisted_readback: dict[str, Any],
    issue1231_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    hit_ids = {row["pair_id"] for row in hits}
    status_ids = {row["pair_id"] for row in pair_status}
    context_ids = {row["pair_id"] for row in source_context}
    prior_hit_ids = {row["pair_id"] for row in prior_no_hit_rows if row.get("cdcdb_match")}
    return {
        "schema_version": 1,
        "issue": 1233,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "source_rows": artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "cdcdb_source_context_rows": artifact(out_dir / "cdcdb_source_context_rows.jsonl", jsonl=True),
            "cdcdb_pair_validation_status": artifact(out_dir / "cdcdb_pair_validation_status.jsonl", jsonl=True),
            "cdcdb_missing_gate_rows": artifact(out_dir / "cdcdb_missing_gate_rows.jsonl", jsonl=True),
            "issue1233_bridge_rows": artifact(out_dir / "issue1233_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "output_manifest": artifact(out_dir / "output_manifest.json"),
        },
        "assertions": {
            "issue1231_persisted_readback_all_true": all_assertions_true(issue1231_persisted_readback),
            "issue1231_calyx_readback_status_ok": issue1231_calyx_readback.get("status") == "ok",
            "input_hashes_match_expected": all(row["hash_match"] for row in source_rows),
            "source_rows_cover_expected_inputs": len(source_rows) == len(EXPECTED_INPUT_SHA256),
            "cdcdb_hit_rows_checked": len(hits) == 173,
            "prior_no_hit_rows_checked": len(prior_no_hit_rows) == 1682,
            "prior_no_hit_cdcdb_hits_match_issue1231": len(prior_hit_ids) == 136,
            "pair_status_for_every_hit": hit_ids == status_ids,
            "source_context_for_every_hit": hit_ids.issubset(context_ids),
            "allowed_validation_statuses_only": all(row["validation_status"] in ALLOWED_VALIDATION_STATUSES for row in pair_status),
            "all_pair_status_rows_blocked": all(row["validation_status"].startswith("blocked") or row["validation_status"] == "source_context_only" for row in pair_status),
            "all_rows_carry_clinical_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in [*source_context, *pair_status, *missing_gate_rows]),
            "cdcdb_never_counts_as_synergy_or_pair_interaction_proof": all("not_pair_interaction" in row["pair_interaction_gate"] or "missing" in row["pair_interaction_gate"] for row in pair_status),
            "bridge_rows_bounded": len(bridge_rows) <= 1000,
        },
    }


def run(root: Path, inputs: dict[str, str], skip_hash_check: bool = False) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    source_rows = verify_inputs(inputs, skip=skip_hash_check)
    hits = rows_jsonl(inputs["issue1231_candidate_hits"])
    prior_no_hit_rows = rows_jsonl(inputs["issue1231_prior_no_hit_recheck"])
    issue1231_persisted_readback = UTIL.read_json(Path(inputs["issue1231_persisted_readback"]))
    issue1231_calyx_readback = UTIL.read_json(Path(inputs["issue1231_calyx_readback"]))
    source_context = build_source_context_rows(hits)
    pair_status = build_pair_status_rows(hits)
    missing_gate_rows = build_missing_gate_rows(pair_status)
    pair_status_path = out_dir / "cdcdb_pair_validation_status.jsonl"
    write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    write_jsonl(out_dir / "cdcdb_source_context_rows.jsonl", source_context)
    write_jsonl(pair_status_path, pair_status)
    pair_status_sha = UTIL.sha256_path(pair_status_path)
    write_jsonl(out_dir / "cdcdb_missing_gate_rows.jsonl", missing_gate_rows)
    bridge_rows = build_bridge_rows(pair_status, source_context, pair_status_path, pair_status_sha)
    write_jsonl(out_dir / "issue1233_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(hits, prior_no_hit_rows, source_context, pair_status, missing_gate_rows, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)
    write_json(
        out_dir / "input_manifest.json",
        {
            "schema_version": 1,
            "issue": 1233,
            "clinical_boundary": CLINICAL_BOUNDARY,
            "inputs": {row["input_name"]: row for row in source_rows},
            "accepted_source_contract": {
                "sealed_issue1231_cdcdb_artifacts": True,
                "cdcdb_source_context_not_synergy_or_pair_interaction_proof": True,
                "cdcdb_not_component_safety_clearance": True,
                "cdcdb_not_outcome_or_clinical_actionability": True,
            },
        },
    )
    output_manifest = {
        "schema_version": 1,
        "issue": 1233,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "source_rows": artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "cdcdb_source_context_rows": artifact(out_dir / "cdcdb_source_context_rows.jsonl", jsonl=True),
            "cdcdb_pair_validation_status": artifact(pair_status_path, jsonl=True),
            "cdcdb_missing_gate_rows": artifact(out_dir / "cdcdb_missing_gate_rows.jsonl", jsonl=True),
            "issue1233_bridge_rows": artifact(out_dir / "issue1233_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        source_rows,
        hits,
        prior_no_hit_rows,
        source_context,
        pair_status,
        missing_gate_rows,
        bridge_rows,
        issue1231_persisted_readback,
        issue1231_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "pair_status": output_manifest["artifacts"]["cdcdb_pair_validation_status"],
            "source_context": output_manifest["artifacts"]["cdcdb_source_context_rows"],
            "missing_gates": output_manifest["artifacts"]["cdcdb_missing_gate_rows"],
            "bridge_rows": output_manifest["artifacts"]["issue1233_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
        "assertions": readback["assertions"],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    for name in DEFAULT_INPUTS:
        parser.add_argument("--" + name.replace("_", "-"))
    parser.add_argument("--skip-hash-check", action="store_true")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for name in DEFAULT_INPUTS:
        value = getattr(args, name)
        if value:
            inputs[name] = value
    result = run(Path(args.root), inputs, skip_hash_check=args.skip_hash_check)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
