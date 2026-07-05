#!/usr/bin/env python3
"""#1261 FAERS case-quality/confounder overlay for combination ranker rows."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
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
    "FAERS case-quality ranker overlay is safety/source/falsification triage "
    "only; case-quality rows, confounder features, and rank penalties are "
    "blockers or review inputs, not causality, safety clearance, efficacy, "
    "treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, pair-interaction proof, or cure evidence."
)

SOURCE_DATASET = "issue1261_faers_case_quality_ranker_overlay"
PROMOTION_STATUS = "blocked_requires_case_level_safety_falsification_and_human_review"
CASE_FAMILY_KEY = "metformin||trametinib"

ISSUE1190_ROOT = "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z"
ISSUE1260_ROOT = "/home/croyse/calyx/fsv/issue1260-metformin-trametinib-faers-case-20260705T020000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1261-faers-case-quality-ranker-overlay-20260705T030000Z"

DEFAULT_INPUTS = {
    "issue1190_candidate_pairs": f"{ISSUE1190_ROOT}/out/candidate_pair_inputs.jsonl",
    "issue1190_hypotheses": f"{ISSUE1190_ROOT}/out/drug_combination_hypotheses.jsonl",
    "issue1190_flags": f"{ISSUE1190_ROOT}/out/combination_safety_interaction_flags.jsonl",
    "issue1190_persisted_readback": f"{ISSUE1190_ROOT}/out/persisted_readback.json",
    "issue1190_calyx_readback": f"{ISSUE1190_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1260_faers_query_rows": f"{ISSUE1260_ROOT}/out/faers_query_rows.jsonl",
    "issue1260_faers_case_rows": f"{ISSUE1260_ROOT}/out/faers_case_rows.jsonl",
    "issue1260_case_rollups": f"{ISSUE1260_ROOT}/out/case_rollups.jsonl",
    "issue1260_candidate_case_status": f"{ISSUE1260_ROOT}/out/candidate_case_status.jsonl",
    "issue1260_persisted_readback": f"{ISSUE1260_ROOT}/out/persisted_readback.json",
    "issue1260_calyx_readback": f"{ISSUE1260_ROOT}/out/calyx_bridge_corpus_readback.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1190_candidate_pairs": "d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654",
    "issue1190_hypotheses": "f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484",
    "issue1190_flags": "86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab",
    "issue1190_persisted_readback": "9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2",
    "issue1190_calyx_readback": "444ba74aa3fc6fbf4ca41407d5b24ac086b64d4a5df7408c5f9fc67aa4579d0f",
    "issue1260_faers_query_rows": "6b2de6cfdad1ffbcd7b86427f54121972837b57a7e48155c8b55713191a8501a",
    "issue1260_faers_case_rows": "30f6908118eae47ba7fdec35a414766caea916a4762a910a45fc9aee7a1b3524",
    "issue1260_case_rollups": "f1f045ed882eae25a8da0fda8025a35dd6b9a02f0b7db368cce5d89c127777a5",
    "issue1260_candidate_case_status": "cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656",
    "issue1260_persisted_readback": "c44de7d913ed74d44f91e5158ad547d314189f8b45ce1f6004fb59b2d2cf1ae0",
    "issue1260_calyx_readback": "a4f070383487e8ad0d0d2878a0cf8f79d534992d7aee6ab582ca4f1006815045",
}


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


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


def count_rows(path: Path) -> int:
    return sum(1 for line in path.read_text().splitlines() if line.strip())


def load_jsonl(path: str | Path) -> list[dict[str, Any]]:
    return list(UTIL.rows_jsonl(Path(path)))


def norm_token(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", UTIL.clean_text(value).lower()).strip()


def canonical_drug(value: str) -> str:
    norm = norm_token(value)
    if norm in {"trametinib dimethyl sulfoxide", "trametinib"}:
        return "trametinib"
    if norm == "mekinist":
        return "trametinib"
    if norm == "metformin":
        return "metformin"
    return norm


def family_key(left: str, right: str) -> str:
    parts = sorted([canonical_drug(left), canonical_drug(right)])
    return "||".join(parts)


def pair_key_from_row(row: dict[str, Any], candidate: dict[str, Any] | None = None) -> str:
    if candidate and candidate.get("pair_key"):
        return UTIL.clean_text(candidate.get("pair_key"))
    return "||".join(sorted([norm_token(row.get("drug_a", "")), norm_token(row.get("drug_b", ""))]))


def case_quality_penalty(case_rollup: dict[str, Any]) -> float:
    penalty = 0.10
    if case_rollup.get("pair_drugs_are_concomitant"):
        penalty += 0.10
    if case_rollup.get("primary_suspect_not_pair_drug"):
        penalty += 0.10
    if case_rollup.get("anticoagulant_confounders"):
        penalty += 0.10
    if int(case_rollup.get("drug_count") or 0) >= 10:
        penalty += 0.05
    return round(penalty, 6)


def build_source_rows(input_hashes: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for name, info in input_hashes.items():
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": "issue1261-source:" + UTIL.stable_id(name, info["sha256"]),
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
        )
    return rows


def build_overlay(inputs: dict[str, str]) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    candidates = {row["pair_id"]: row for row in load_jsonl(inputs["issue1190_candidate_pairs"])}
    hypotheses = load_jsonl(inputs["issue1190_hypotheses"])
    flags = {row["pair_id"]: row for row in load_jsonl(inputs["issue1190_flags"])}
    faers_queries = load_jsonl(inputs["issue1260_faers_query_rows"])
    faers_cases = load_jsonl(inputs["issue1260_faers_case_rows"])
    case_rollups = load_jsonl(inputs["issue1260_case_rollups"])
    case_status_rows = load_jsonl(inputs["issue1260_candidate_case_status"])
    if len(case_rollups) != 1 or len(faers_cases) != 1 or len(faers_queries) != 1:
        raise RuntimeError("Expected exactly one #1260 query, case, and rollup row")
    case_rollup = case_rollups[0]
    faers_case = faers_cases[0]
    faers_query = faers_queries[0]
    if family_key(*case_rollup["pair_key"].split("||", 1)) != CASE_FAMILY_KEY:
        raise RuntimeError(f"Unexpected #1260 case family key: {case_rollup['pair_key']}")

    direct_status_by_pair_id = {row["pair_id"]: row for row in case_status_rows}
    penalty = case_quality_penalty(case_rollup)
    feature_rows = []
    overlay_rows = []
    for row in hypotheses:
        candidate = candidates.get(row["pair_id"])
        row_family_key = family_key(row.get("drug_a", ""), row.get("drug_b", ""))
        if row_family_key != CASE_FAMILY_KEY:
            continue
        flag = flags.get(row["pair_id"], {})
        direct_status = direct_status_by_pair_id.get(row["pair_id"])
        original_score = float(row.get("rank_score") or 0.0)
        adjusted_score = round(max(0.0, original_score - penalty), 6)
        reason_codes = UTIL.uniq(
            list(row.get("reason_codes") or [])
            + list((direct_status or {}).get("reason_codes") or [])
            + [
                "faers_case_quality_confounded_blocker",
                "faers_pair_concomitant_not_primary_suspect",
                "faers_anticoagulant_confounder_present",
                "faers_polypharmacy_case_report_not_pair_causality",
            ]
        )
        feature_rows.append(
            {
                "schema_version": 1,
                "case_quality_feature_id": "issue1261-case-quality:" + UTIL.stable_id(row["pair_id"], case_rollup["case_rollup_id"]),
                "pair_id": row["pair_id"],
                "pair_key": pair_key_from_row(row, candidate),
                "pair_family_key": row_family_key,
                "direct_issue1260_candidate_status": bool(direct_status),
                "source_issue1260_candidate_case_status_id": (direct_status or {}).get("candidate_case_status_id"),
                "case_rollup_id": case_rollup["case_rollup_id"],
                "faers_source_id": case_rollup["source_id"],
                "faers_query_url": faers_query["query_url"],
                "faers_raw_response_sha256": faers_query["raw_response_sha256"],
                "exact_faers_case_count": 1,
                "serious_case_count": 1 if case_rollup.get("serious") else 0,
                "pair_drugs_present": all(faers_case.get("pair_drug_presence", {}).values()),
                "pair_drugs_are_concomitant": bool(case_rollup.get("pair_drugs_are_concomitant")),
                "pair_primary_suspect": not bool(case_rollup.get("primary_suspect_not_pair_drug")),
                "primary_suspect_drugs": case_rollup.get("suspected_drug_names") or [],
                "anticoagulant_confounders": case_rollup.get("anticoagulant_confounders") or [],
                "anticoagulant_confounder_count": len(case_rollup.get("anticoagulant_confounders") or []),
                "unique_anticoagulant_confounder_count": len(set(case_rollup.get("anticoagulant_confounders") or [])),
                "polypharmacy_count": int(case_rollup.get("drug_count") or 0),
                "reactions": case_rollup.get("reactions") or [],
                "case_quality_class": "confounded_case_report_blocker",
                "case_quality_penalty_points": penalty,
                "ranker_effect": "demote_or_hold_blocked_never_promote",
                "overlay_status": "blocked_case_quality_confounded_still_blocked",
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        overlay_rows.append(
            {
                "schema_version": 1,
                "ranker_overlay_id": "issue1261-ranker-overlay:" + UTIL.stable_id(row["pair_id"], case_rollup["case_rollup_id"]),
                "pair_id": row["pair_id"],
                "pair_key": pair_key_from_row(row, candidate),
                "pair_family_key": row_family_key,
                "drug_a": row.get("drug_a"),
                "drug_b": row.get("drug_b"),
                "disease": row.get("disease"),
                "disease_area": row.get("disease_area"),
                "original_rank": row.get("rank"),
                "original_rank_score": original_score,
                "case_quality_penalty_points": penalty,
                "case_quality_adjusted_score": adjusted_score,
                "original_combination_status": row.get("combination_status"),
                "original_flag_blocked": flag.get("blocked"),
                "overlay_combination_status": "blocked_case_quality_confounded_still_blocked",
                "blocked": True,
                "no_promotion": True,
                "direct_issue1260_candidate_status": bool(direct_status),
                "case_quality_feature_id": feature_rows[-1]["case_quality_feature_id"],
                "reason_codes": reason_codes,
                "next_validation_experiment": "Treat FAERS co-report as confounded safety blocker; require independent unconfounded safety/outcome evidence before any promotion.",
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )

    summary_rows = [
        {
            "schema_version": 1,
            "family_summary_id": "issue1261-family-summary:" + UTIL.stable_id(CASE_FAMILY_KEY, case_rollup["case_rollup_id"]),
            "pair_family_key": CASE_FAMILY_KEY,
            "source_case_rollup_id": case_rollup["case_rollup_id"],
            "faers_source_id": case_rollup["source_id"],
            "affected_ranker_rows": len(overlay_rows),
            "direct_issue1260_candidate_status_rows": sum(1 for row in feature_rows if row["direct_issue1260_candidate_status"]),
            "ingredient_family_context_rows": sum(1 for row in feature_rows if not row["direct_issue1260_candidate_status"]),
            "case_quality_class": "confounded_case_report_blocker",
            "case_quality_penalty_points": penalty,
            "all_rows_blocked": all(row["blocked"] for row in overlay_rows),
            "reason_codes": UTIL.uniq([code for row in overlay_rows for code in row["reason_codes"]]),
            "promotion_status": PROMOTION_STATUS,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
    ]
    return feature_rows, overlay_rows, summary_rows


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
    feature_rows: list[dict[str, Any]],
    overlay_rows: list[dict[str, Any]],
    summary_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows = []
    for row in source_rows:
        terms = UTIL.uniq(["issue1261", row["input_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1261_source_input",
                "text": f"Issue1261 sealed input {row['input_name']} sha256 {row['sha256']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["source_path"], row["sha256"]),
            }
        )
    for row in feature_rows:
        terms = UTIL.uniq(
            [
                row["pair_family_key"],
                row["pair_id"],
                row["faers_source_id"],
                row["case_quality_class"],
                row["overlay_status"],
            ]
        )
        rows.append(
            {
                "id": row["case_quality_feature_id"],
                "domain": "issue1261_case_quality_feature",
                "text": (
                    f"Issue1261 case quality feature {row['pair_id']} {row['pair_family_key']} "
                    f"FAERS {row['faers_source_id']} {row['case_quality_class']} {row['overlay_status']} "
                    f"bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in overlay_rows:
        terms = UTIL.uniq([row["pair_family_key"], row["pair_id"], row["overlay_combination_status"], row["drug_a"], row["drug_b"]])
        rows.append(
            {
                "id": row["ranker_overlay_id"],
                "domain": "issue1261_ranker_overlay",
                "text": (
                    f"Issue1261 ranker overlay {row['pair_id']} {row['drug_a']} {row['drug_b']} "
                    f"{row['overlay_combination_status']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in summary_rows:
        terms = UTIL.uniq([row["pair_family_key"], row["faers_source_id"], row["case_quality_class"]])
        rows.append(
            {
                "id": row["family_summary_id"],
                "domain": "issue1261_family_summary",
                "text": (
                    f"Issue1261 family summary {row['pair_family_key']} FAERS {row['faers_source_id']} "
                    f"{row['case_quality_class']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    return rows


def build_metrics(
    source_rows: list[dict[str, Any]],
    feature_rows: list[dict[str, Any]],
    overlay_rows: list[dict[str, Any]],
    summary_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "source_rows": len(source_rows),
        "case_quality_feature_rows": len(feature_rows),
        "ranker_overlay_rows": len(overlay_rows),
        "family_summary_rows": len(summary_rows),
        "direct_issue1260_candidate_status_rows": sum(1 for row in feature_rows if row["direct_issue1260_candidate_status"]),
        "ingredient_family_context_rows": sum(1 for row in feature_rows if not row["direct_issue1260_candidate_status"]),
        "all_overlay_rows_blocked": all(row["blocked"] for row in overlay_rows),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
        "case_quality_class_counts": dict(Counter(row["case_quality_class"] for row in feature_rows)),
        "overlay_status_counts": dict(Counter(row["overlay_combination_status"] for row in overlay_rows)),
    }


def build_readback(
    out_dir: Path,
    input_hashes: dict[str, dict[str, Any]],
    feature_rows: list[dict[str, Any]],
    overlay_rows: list[dict[str, Any]],
    summary_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    artifact_paths = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_rows": out_dir / "source_rows.jsonl",
        "case_quality_features": out_dir / "case_quality_features.jsonl",
        "ranker_case_quality_overlay": out_dir / "ranker_case_quality_overlay.jsonl",
        "family_case_quality_summary": out_dir / "family_case_quality_summary.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
        "issue1261_bridge_rows": out_dir / "issue1261_bridge_rows.jsonl",
    }
    artifacts = {name: UTIL.artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifact_paths.items()}
    direct_rows = [row for row in feature_rows if row["direct_issue1260_candidate_status"]]
    family_rows = [row for row in feature_rows if not row["direct_issue1260_candidate_status"]]
    assertions = {
        "expected_input_hashes_match": all(item["match"] for item in input_hashes.values()),
        "affected_ranker_rows_5": len(feature_rows) == 5 and len(overlay_rows) == 5,
        "direct_issue1260_rows_4": len(direct_rows) == 4,
        "ingredient_family_context_rows_1": len(family_rows) == 1,
        "family_summary_one": len(summary_rows) == 1,
        "family_key_matches": all(row["pair_family_key"] == CASE_FAMILY_KEY for row in feature_rows + overlay_rows + summary_rows),
        "all_overlay_rows_blocked": all(row["blocked"] and row["no_promotion"] for row in overlay_rows),
        "case_quality_confounded": all(row["case_quality_class"] == "confounded_case_report_blocker" for row in feature_rows),
        "penalty_positive": all(float(row["case_quality_penalty_points"]) > 0 for row in feature_rows + overlay_rows + summary_rows),
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
    feature_rows, overlay_rows, summary_rows = build_overlay(inputs)
    UTIL.write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    UTIL.write_jsonl(out_dir / "case_quality_features.jsonl", feature_rows)
    UTIL.write_jsonl(out_dir / "ranker_case_quality_overlay.jsonl", overlay_rows)
    UTIL.write_jsonl(out_dir / "family_case_quality_summary.jsonl", summary_rows)

    source_path = out_dir / "ranker_case_quality_overlay.jsonl"
    source_sha = UTIL.sha256_path(source_path)
    bridge_rows = build_bridge_rows(source_rows, feature_rows, overlay_rows, summary_rows, source_path, source_sha)
    UTIL.write_jsonl(out_dir / "issue1261_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(source_rows, feature_rows, overlay_rows, summary_rows, bridge_rows)
    UTIL.write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1261,
        "created_at": now_utc(),
        "inputs": inputs,
        "input_hashes": input_hashes,
        "artifacts": {
            "source_rows": UTIL.artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "case_quality_features": UTIL.artifact(out_dir / "case_quality_features.jsonl", jsonl=True),
            "ranker_case_quality_overlay": UTIL.artifact(out_dir / "ranker_case_quality_overlay.jsonl", jsonl=True),
            "family_case_quality_summary": UTIL.artifact(out_dir / "family_case_quality_summary.jsonl", jsonl=True),
            "issue1261_bridge_rows": UTIL.artifact(out_dir / "issue1261_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": UTIL.artifact(out_dir / "validation_metrics.json"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    UTIL.write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, input_hashes, feature_rows, overlay_rows, summary_rows, bridge_rows)
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
                    "case_quality_features": UTIL.artifact(out_dir / "case_quality_features.jsonl", jsonl=True),
                    "ranker_case_quality_overlay": UTIL.artifact(out_dir / "ranker_case_quality_overlay.jsonl", jsonl=True),
                    "family_case_quality_summary": UTIL.artifact(out_dir / "family_case_quality_summary.jsonl", jsonl=True),
                    "bridge_rows": UTIL.artifact(out_dir / "issue1261_bridge_rows.jsonl", jsonl=True),
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
