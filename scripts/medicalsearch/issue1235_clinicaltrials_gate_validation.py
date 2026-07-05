#!/usr/bin/env python3
"""#1235 validate #1232 ClinicalTrials.gov hits through trial/safety gates."""

from __future__ import annotations

import argparse
import importlib.util
import json
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


ISSUE1232_ROOT = "/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1235-clinicaltrials-gate-validation-20260705T060000Z"

CLINICAL_BOUNDARY = (
    "ClinicalTrials.gov trial-context validation is registry/source triage only; "
    "same-arm context, adverse-event modules, and outcome fields are blockers "
    "or review inputs, not efficacy, safety clearance, dosing guidance, "
    "treatment guidance, recommendation, clinical actionability, "
    "pair-interaction proof, or cure evidence."
)

SOURCE_DATASET = "issue1235_clinicaltrials_gate_validation"
PROMOTION_STATUS = "blocked_requires_independent_safety_pair_interaction_outcome_and_human_review"

DEFAULT_INPUTS = {
    "issue1232_pair_hits": f"{ISSUE1232_ROOT}/out/clinicaltrials_pair_hits.jsonl",
    "issue1232_study_evidence": f"{ISSUE1232_ROOT}/out/clinicaltrials_study_evidence.jsonl",
    "issue1232_pair_status": f"{ISSUE1232_ROOT}/out/clinicaltrials_pair_status.jsonl",
    "issue1232_raw_responses": f"{ISSUE1232_ROOT}/out/clinicaltrials_raw_responses.jsonl",
    "issue1232_persisted_readback": f"{ISSUE1232_ROOT}/out/persisted_readback.json",
    "issue1232_calyx_readback": f"{ISSUE1232_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1232_output_manifest": f"{ISSUE1232_ROOT}/out/output_manifest.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1232_pair_hits": "026cc1d5698f6b7b2fdcf2cd095cf0a605a804f1ed3f40ed0bd151342c006c38",
    "issue1232_study_evidence": "cb25d98c9fd5c623d63c31a9dcf7b7fbc55add65cc0a7b9d265fe83e5f875970",
    "issue1232_pair_status": "ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc",
    "issue1232_raw_responses": "1f8d40df3bac91b30807e9eae785a70f6710ebccf8304b7c8e2eb678ee2b6e2d",
    "issue1232_persisted_readback": "606decab759d282df659071b4e13f8f295d69d52b6d10670fa26765e138d3ce1",
    "issue1232_calyx_readback": "ffb805ce073618cb0e674ab2e80e040982f7da7b3bb08486e7bfc5dad8a23b1a",
    "issue1232_output_manifest": "ad6e55d387c2fe0f6558561ee403ffc4f2350a9bc5f81ad08f05e76afd123d1d",
}

ALLOWED_VALIDATION_STATUSES = {
    "blocked_no_safety",
    "blocked_no_pair_interaction",
    "blocked_no_outcome",
    "registry_context_only",
    "rejected_false_positive",
    "reviewable_preclinical_only",
}

SALT_OR_FORM_TOKENS = {
    "acetate",
    "calcium",
    "dimethyl",
    "disodium",
    "fumarate",
    "hcl",
    "hydrochloride",
    "magnesium",
    "maleate",
    "monohydrate",
    "monophosphate",
    "phosphate",
    "potassium",
    "sodium",
    "succinate",
    "sulfate",
    "sulfoxide",
    "tartrate",
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


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(bool(item) for item in assertions.values())


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
            "schema_version": 1,
            "source_row_id": "issue1235-source:" + UTIL.stable_id(name, observed),
            "input_name": name,
            "path": str(path),
            "rows": count_jsonl(path) if path.suffix == ".jsonl" else None,
            "bytes": path.stat().st_size,
            "sha256": observed,
            "expected_sha256": expected,
            "hash_match": ok,
            "classification": "sealed_issue1232_input",
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
    return rows


def count_jsonl(path: Path) -> int:
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def study_nct_id(study: dict[str, Any]) -> str:
    return UTIL.clean_text(
        study.get("protocolSection", {})
        .get("identificationModule", {})
        .get("nctId")
    )


def term_present(text: object, name: object) -> bool:
    norm = UTIL.norm_name(name)
    if not norm:
        return False
    return f" {norm} " in UTIL.normalized_blob(text)


def reduced_term_tokens(value: object) -> set[str]:
    tokens = set(UTIL.norm_name(value).split())
    return {token for token in tokens if token not in SALT_OR_FORM_TOKENS}


def likely_self_or_salt_pair(drug_a: object, drug_b: object) -> bool:
    left = reduced_term_tokens(drug_a)
    right = reduced_term_tokens(drug_b)
    if not left or not right:
        return False
    return left == right or left.issubset(right) or right.issubset(left)


def module(study: dict[str, Any], *path: str) -> dict[str, Any]:
    current: Any = study
    for key in path:
        if not isinstance(current, dict):
            return {}
        current = current.get(key)
    return current if isinstance(current, dict) else {}


def list_module(study: dict[str, Any], *path: str) -> list[dict[str, Any]]:
    current: Any = study
    for key in path:
        if not isinstance(current, dict):
            return []
        current = current.get(key)
    return current if isinstance(current, list) else []


def trial_phase(study: dict[str, Any]) -> list[str]:
    phases = module(study, "protocolSection", "designModule").get("phases") or []
    return UTIL.uniq(phases)


def overall_status(study: dict[str, Any]) -> str:
    return UTIL.clean_text(module(study, "protocolSection", "statusModule").get("overallStatus"))


def brief_title(study: dict[str, Any]) -> str:
    return UTIL.clean_text(module(study, "protocolSection", "identificationModule").get("briefTitle"))


def conditions(study: dict[str, Any]) -> list[str]:
    return UTIL.uniq(module(study, "protocolSection", "conditionsModule").get("conditions") or [])


def arm_group_rows(study: dict[str, Any]) -> list[dict[str, Any]]:
    return list_module(study, "protocolSection", "armsInterventionsModule", "armGroups")


def intervention_rows(study: dict[str, Any]) -> list[dict[str, Any]]:
    return list_module(study, "protocolSection", "armsInterventionsModule", "interventions")


def arm_labels(study: dict[str, Any]) -> list[str]:
    return UTIL.uniq(row.get("label") for row in arm_group_rows(study))


def intervention_names(study: dict[str, Any]) -> list[str]:
    names = [row.get("name") for row in intervention_rows(study)]
    for arm in arm_group_rows(study):
        names.extend(arm.get("interventionNames") or [])
    return UTIL.uniq(names)


def protocol_outcome_titles(study: dict[str, Any]) -> list[str]:
    outcomes = module(study, "protocolSection", "outcomesModule")
    titles: list[str] = []
    for key in ["primaryOutcomes", "secondaryOutcomes", "otherOutcomes"]:
        for row in outcomes.get(key) or []:
            titles.append(row.get("measure"))
    return UTIL.uniq(titles)


def results_outcome_titles(study: dict[str, Any]) -> list[str]:
    outcomes = list_module(study, "resultsSection", "outcomeMeasuresModule", "outcomeMeasures")
    return UTIL.uniq(row.get("title") for row in outcomes)


def adverse_event_counts(study: dict[str, Any]) -> dict[str, int]:
    adverse = module(study, "resultsSection", "adverseEventsModule")
    return {
        "event_group_count": len(adverse.get("eventGroups") or []),
        "serious_event_count": len(adverse.get("seriousEvents") or []),
        "other_event_count": len(adverse.get("otherEvents") or []),
    }


def result_module_keys(study: dict[str, Any]) -> list[str]:
    return sorted(module(study, "resultsSection").keys())


def arm_match_summary(study: dict[str, Any], drug_a: str, drug_b: str) -> dict[str, Any]:
    arm_matches: list[dict[str, Any]] = []
    for arm in arm_group_rows(study):
        text = UTIL.clean_text([arm.get("label"), arm.get("type"), arm.get("description"), arm.get("interventionNames")])
        left = term_present(text, drug_a)
        right = term_present(text, drug_b)
        arm_matches.append(
            {
                "label": UTIL.clean_text(arm.get("label")),
                "type": UTIL.clean_text(arm.get("type")),
                "matches_drug_a": left,
                "matches_drug_b": right,
                "matches_both": left and right,
            }
        )
    labels_by_term = {"drug_a": set(), "drug_b": set()}
    for intervention in intervention_rows(study):
        text = UTIL.clean_text([intervention.get("name"), intervention.get("otherNames"), intervention.get("description")])
        labels = set(UTIL.clean_text(label) for label in intervention.get("armGroupLabels") or [] if UTIL.clean_text(label))
        if term_present(text, drug_a):
            labels_by_term["drug_a"].update(labels)
        if term_present(text, drug_b):
            labels_by_term["drug_b"].update(labels)
    shared_labels = labels_by_term["drug_a"].intersection(labels_by_term["drug_b"])
    full_intervention_text = UTIL.clean_text([arm_group_rows(study), intervention_rows(study)])
    return {
        "arm_matches": arm_matches,
        "shared_arm_labels": sorted(shared_labels),
        "drug_a_arm_labels": sorted(labels_by_term["drug_a"]),
        "drug_b_arm_labels": sorted(labels_by_term["drug_b"]),
        "same_arm_match": bool(shared_labels) or any(row["matches_both"] for row in arm_matches),
        "both_terms_in_intervention_text": term_present(full_intervention_text, drug_a) and term_present(full_intervention_text, drug_b),
    }


def classify_trial_context(study: dict[str, Any], hit: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    drug_a = UTIL.clean_text(hit["drug_a"])
    drug_b = UTIL.clean_text(hit["drug_b"])
    arm_summary = arm_match_summary(study, drug_a, drug_b)
    self_pair = likely_self_or_salt_pair(drug_a, drug_b)
    if self_pair:
        return "likely_false_positive_self_or_salt_duplicate", arm_summary
    if arm_summary["same_arm_match"]:
        return "same_arm_combination_context", arm_summary
    if arm_summary["drug_a_arm_labels"] and arm_summary["drug_b_arm_labels"]:
        return "comparator_only_cooccurrence", arm_summary
    if arm_summary["both_terms_in_intervention_text"]:
        return "broad_intervention_list_context", arm_summary
    return "registry_context_only", arm_summary


def study_context_row(hit: dict[str, Any], study: dict[str, Any], source_row: dict[str, Any]) -> dict[str, Any]:
    context_class, arm_summary = classify_trial_context(study, hit)
    protocol_titles = protocol_outcome_titles(study)
    result_titles = results_outcome_titles(study)
    adverse_counts = adverse_event_counts(study)
    nct_id = study_nct_id(study)
    has_adverse = any(adverse_counts.values())
    has_outcomes = bool(protocol_titles or result_titles)
    return {
        "schema_version": 1,
        "trial_context_id": "issue1235-trial:" + UTIL.stable_id(hit["pair_id"], nct_id, context_class),
        "source_issue1232_evidence_id": source_row.get("evidence_id"),
        "source_issue1232_pair_evidence_id": hit.get("evidence_id"),
        "pair_id": hit["pair_id"],
        "pair_key": hit["pair_key"],
        "drug_a": hit["drug_a"],
        "drug_b": hit["drug_b"],
        "nct_id": nct_id,
        "source_url": f"https://clinicaltrials.gov/study/{nct_id}" if nct_id else None,
        "brief_title": brief_title(study),
        "overall_status": overall_status(study),
        "phases": trial_phase(study),
        "conditions": conditions(study),
        "trial_context_class": context_class,
        "arm_labels": arm_labels(study),
        "intervention_names": intervention_names(study),
        "arm_match_summary": arm_summary,
        "has_results": bool(study.get("hasResults")),
        "result_module_keys": result_module_keys(study),
        "protocol_outcome_count": len(protocol_titles),
        "protocol_outcome_titles": protocol_titles[:20],
        "result_outcome_count": len(result_titles),
        "result_outcome_titles": result_titles[:20],
        "adverse_event_counts": adverse_counts,
        "has_adverse_event_module_rows": has_adverse,
        "has_outcome_fields": has_outcomes,
        "component_safety_gate": (
            "trial_adverse_event_context_present_not_safety_clearance"
            if has_adverse
            else "component_safety_evidence_missing_fail_closed"
        ),
        "outcome_gate": (
            "trial_outcome_fields_present_not_grounded_outcome_clearance"
            if has_outcomes
            else "grounded_outcome_evidence_missing_fail_closed"
        ),
        "pair_interaction_gate": "not_cleared_registry_context_is_not_synergy_or_pair_interaction_proof",
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def raw_studies_by_pair(inputs: dict[str, str], pair_ids: set[str]) -> dict[str, dict[str, dict[str, Any]]]:
    by_pair: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    with Path(inputs["issue1232_raw_responses"]).open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            row = json.loads(line)
            pair_id = row.get("pair_id")
            if pair_id not in pair_ids:
                continue
            for page in row.get("response", {}).get("pages", []):
                for study in page.get("studies", []):
                    nct_id = study_nct_id(study)
                    if nct_id:
                        by_pair[pair_id][nct_id] = study
    return by_pair


def build_trial_context_rows(
    hits: list[dict[str, Any]],
    study_evidence: list[dict[str, Any]],
    raw_studies: dict[str, dict[str, dict[str, Any]]],
) -> list[dict[str, Any]]:
    hit_by_pair = {row["pair_id"]: row for row in hits}
    rows: list[dict[str, Any]] = []
    for source_row in study_evidence:
        hit = hit_by_pair.get(source_row["pair_id"])
        if hit is None:
            continue
        nct_id = UTIL.clean_text(source_row.get("study", {}).get("nct_id"))
        study = raw_studies.get(source_row["pair_id"], {}).get(nct_id)
        if study is None:
            fallback = source_row.get("study", {})
            study = {
                "hasResults": False,
                "protocolSection": {
                    "identificationModule": {
                        "nctId": fallback.get("nct_id"),
                        "briefTitle": fallback.get("brief_title"),
                    },
                    "statusModule": {"overallStatus": fallback.get("overall_status")},
                    "designModule": {"phases": fallback.get("phases") or []},
                    "conditionsModule": {"conditions": fallback.get("conditions") or []},
                    "armsInterventionsModule": {
                        "armGroups": [],
                        "interventions": [
                            {
                                "name": name,
                                "armGroupLabels": [],
                                "type": "DRUG",
                            }
                            for name in fallback.get("intervention_names") or []
                        ],
                    },
                },
            }
        rows.append(study_context_row(hit, study, source_row))
    rows.sort(key=lambda row: (row["pair_key"], row["pair_id"], row["nct_id"], row["trial_context_id"]))
    return rows


def pair_status_from_context(hit: dict[str, Any], contexts: list[dict[str, Any]]) -> dict[str, Any]:
    classes = Counter(row["trial_context_class"] for row in contexts)
    same_arm = classes.get("same_arm_combination_context", 0)
    false_positive = classes.get("likely_false_positive_self_or_salt_duplicate", 0)
    adverse = sum(1 for row in contexts if row["has_adverse_event_module_rows"])
    outcomes = sum(1 for row in contexts if row["has_outcome_fields"])
    result_rows = sum(1 for row in contexts if row["has_results"])
    if contexts and false_positive == len(contexts):
        status = "rejected_false_positive"
    elif same_arm == 0:
        status = "registry_context_only"
    elif adverse == 0:
        status = "blocked_no_safety"
    elif outcomes == 0:
        status = "blocked_no_outcome"
    else:
        status = "blocked_no_pair_interaction"
    reason_codes = [
        status,
        "clinicaltrials_registry_context_not_clinical_clearance",
        "registry_cooccurrence_not_synergy_or_pair_interaction_proof",
        "human_review_required",
    ]
    if same_arm:
        reason_codes.append("same_arm_context_present")
    if adverse:
        reason_codes.append("trial_adverse_event_context_present")
    else:
        reason_codes.append("component_safety_missing_or_not_independent")
    if outcomes:
        reason_codes.append("trial_outcome_fields_present")
    else:
        reason_codes.append("grounded_outcome_missing")
    return {
        "schema_version": 1,
        "pair_validation_id": "issue1235-pair:" + UTIL.stable_id(hit["pair_id"], status),
        "source_issue1232_pair_evidence_id": hit.get("evidence_id"),
        "pair_id": hit["pair_id"],
        "pair_key": hit["pair_key"],
        "drug_a": hit["drug_a"],
        "drug_b": hit["drug_b"],
        "validation_status": status,
        "trial_context_class_counts": dict(sorted(classes.items())),
        "matched_study_count": len(contexts),
        "same_arm_study_count": same_arm,
        "adverse_event_context_study_count": adverse,
        "outcome_field_study_count": outcomes,
        "has_results_study_count": result_rows,
        "sample_nct_ids": [row["nct_id"] for row in contexts[:20]],
        "component_safety_gate": (
            "trial_adverse_event_context_present_not_safety_clearance"
            if adverse
            else "component_safety_evidence_missing_fail_closed"
        ),
        "pair_interaction_gate": "not_cleared_registry_cooccurrence_not_synergy_or_pair_interaction_proof",
        "outcome_gate": (
            "trial_outcome_fields_present_not_grounded_outcome_clearance"
            if outcomes
            else "grounded_outcome_evidence_missing_fail_closed"
        ),
        "human_review_gate": "missing_human_review_fail_closed",
        "promotion_status": PROMOTION_STATUS,
        "reason_codes": UTIL.uniq(reason_codes),
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def build_pair_status_rows(hits: list[dict[str, Any]], context_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    contexts_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in context_rows:
        contexts_by_pair[row["pair_id"]].append(row)
    rows = [pair_status_from_context(hit, contexts_by_pair.get(hit["pair_id"], [])) for hit in hits]
    rows.sort(key=lambda row: (row["validation_status"], row["pair_key"], row["pair_id"]))
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
                    "missing_gate_id": "issue1235-gate:" + UTIL.stable_id(row["pair_id"], gate, gate_status),
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


def build_bridge_rows(pair_status: list[dict[str, Any]], context_rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        ncts = ", ".join(row["sample_nct_ids"][:5]) or "none"
        text = (
            f"ClinicalTrials gate validation pair {row['pair_key']} {row['drug_a']} plus "
            f"{row['drug_b']} status {row['validation_status']} same-arm studies "
            f"{row['same_arm_study_count']} safety-context studies "
            f"{row['adverse_event_context_study_count']} outcome-field studies "
            f"{row['outcome_field_study_count']} NCT {ncts}."
        )
        rows.append(
            {
                "id": row["pair_validation_id"],
                "domain": "clinicaltrials_pair_gate_status",
                "text": text,
                "bridge_terms": [
                    term
                    for term in UTIL.uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["validation_status"], *row["sample_nct_ids"][:5]])
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
    for row in context_rows[:budget]:
        text = (
            f"ClinicalTrials trial context {row['nct_id']} for {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} class {row['trial_context_class']} "
            f"status {row['overall_status']} phases {', '.join(row['phases']) or 'none'}."
        )
        rows.append(
            {
                "id": row["trial_context_id"],
                "domain": "clinicaltrials_trial_context",
                "text": text,
                "bridge_terms": [
                    term
                    for term in UTIL.uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["nct_id"], row["trial_context_class"]])
                    if term and UTIL.clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": SOURCE_DATASET,
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "nct_id": row["nct_id"],
                    "trial_context_class": row["trial_context_class"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(hits: list[dict[str, Any]], study_evidence: list[dict[str, Any]], context_rows: list[dict[str, Any]], pair_status: list[dict[str, Any]], missing_gate_rows: list[dict[str, Any]], bridge_rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "created_at": now_utc(),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1232_pair_hit_rows_checked": len(hits),
        "issue1232_study_evidence_rows_checked": len(study_evidence),
        "trial_context_rows": len(context_rows),
        "pair_validation_status_rows": len(pair_status),
        "missing_gate_rows": len(missing_gate_rows),
        "bridge_rows": len(bridge_rows),
        "unique_pair_keys": len({row["pair_key"] for row in pair_status}),
        "unique_nct_ids": len({row["nct_id"] for row in context_rows if row["nct_id"]}),
        "validation_status_counts": dict(sorted(Counter(row["validation_status"] for row in pair_status).items())),
        "trial_context_class_counts": dict(sorted(Counter(row["trial_context_class"] for row in context_rows).items())),
        "gate_status_counts": dict(sorted(Counter(row["gate_status"] for row in missing_gate_rows).items())),
        "same_arm_pair_rows": sum(1 for row in pair_status if row["same_arm_study_count"] > 0),
        "pairs_with_adverse_event_context": sum(1 for row in pair_status if row["adverse_event_context_study_count"] > 0),
        "pairs_with_outcome_fields": sum(1 for row in pair_status if row["outcome_field_study_count"] > 0),
        "top_same_arm_contexts": [
            {
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "validation_status": row["validation_status"],
                "same_arm_study_count": row["same_arm_study_count"],
                "sample_nct_ids": row["sample_nct_ids"][:5],
            }
            for row in pair_status
            if row["same_arm_study_count"] > 0
        ][:25],
    }


def build_readback(
    out_dir: Path,
    source_rows: list[dict[str, Any]],
    hits: list[dict[str, Any]],
    study_evidence: list[dict[str, Any]],
    raw_studies: dict[str, dict[str, dict[str, Any]]],
    context_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    missing_gate_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1232_persisted_readback: dict[str, Any],
    issue1232_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    hit_pair_ids = {row["pair_id"] for row in hits}
    status_pair_ids = {row["pair_id"] for row in pair_status}
    context_evidence_ids = {row["source_issue1232_evidence_id"] for row in context_rows}
    study_evidence_ids = {row["evidence_id"] for row in study_evidence}
    return {
        "schema_version": 1,
        "issue": 1235,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "source_rows": artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "clinicaltrials_trial_context_rows": artifact(out_dir / "clinicaltrials_trial_context_rows.jsonl", jsonl=True),
            "clinicaltrials_pair_validation_status": artifact(out_dir / "clinicaltrials_pair_validation_status.jsonl", jsonl=True),
            "clinicaltrials_missing_gate_rows": artifact(out_dir / "clinicaltrials_missing_gate_rows.jsonl", jsonl=True),
            "issue1235_bridge_rows": artifact(out_dir / "issue1235_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "output_manifest": artifact(out_dir / "output_manifest.json"),
        },
        "assertions": {
            "issue1232_persisted_readback_all_true": all_assertions_true(issue1232_persisted_readback),
            "issue1232_calyx_readback_all_true": all_assertions_true(issue1232_calyx_readback),
            "input_hashes_match_expected": all(row["hash_match"] for row in source_rows),
            "source_rows_cover_expected_inputs": len(source_rows) == len(EXPECTED_INPUT_SHA256),
            "pair_hit_rows_checked": len(hits) == 204,
            "study_evidence_rows_checked": len(study_evidence) == 1192,
            "raw_response_found_for_every_hit_pair": hit_pair_ids.issubset(set(raw_studies.keys())),
            "pair_status_for_every_hit": hit_pair_ids == status_pair_ids,
            "context_for_every_issue1232_study_evidence": study_evidence_ids == context_evidence_ids,
            "allowed_validation_statuses_only": all(row["validation_status"] in ALLOWED_VALIDATION_STATUSES for row in pair_status),
            "all_pair_status_rows_blocked_or_rejected": all(row["validation_status"].startswith("blocked") or row["validation_status"].startswith("registry") or row["validation_status"].startswith("rejected") or row["validation_status"].startswith("reviewable") for row in pair_status),
            "all_rows_carry_clinical_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in [*context_rows, *pair_status, *missing_gate_rows]),
            "registry_context_not_counted_as_pair_interaction_proof": all("not_cleared" in row["pair_interaction_gate"] for row in pair_status),
            "bridge_rows_bounded": len(bridge_rows) <= 1000,
        },
    }


def run(root: Path, inputs: dict[str, str], skip_hash_check: bool = False) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    input_hashes = verify_inputs(inputs, skip=skip_hash_check)
    source_rows = [input_hashes[name] for name in sorted(input_hashes)]
    hits = rows_jsonl(inputs["issue1232_pair_hits"])
    study_evidence = rows_jsonl(inputs["issue1232_study_evidence"])
    issue1232_persisted_readback = UTIL.read_json(Path(inputs["issue1232_persisted_readback"]))
    issue1232_calyx_readback = UTIL.read_json(Path(inputs["issue1232_calyx_readback"]))
    raw_studies = raw_studies_by_pair(inputs, {row["pair_id"] for row in hits})
    context_rows = build_trial_context_rows(hits, study_evidence, raw_studies)
    pair_status = build_pair_status_rows(hits, context_rows)
    missing_gate_rows = build_missing_gate_rows(pair_status)
    source_path = out_dir / "clinicaltrials_pair_validation_status.jsonl"
    write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    write_jsonl(out_dir / "clinicaltrials_trial_context_rows.jsonl", context_rows)
    write_jsonl(source_path, pair_status)
    source_sha = UTIL.sha256_path(source_path)
    write_jsonl(out_dir / "clinicaltrials_missing_gate_rows.jsonl", missing_gate_rows)
    bridge_rows = build_bridge_rows(pair_status, context_rows, source_path, source_sha)
    write_jsonl(out_dir / "issue1235_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(hits, study_evidence, context_rows, pair_status, missing_gate_rows, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)
    write_json(
        out_dir / "input_manifest.json",
        {
            "schema_version": 1,
            "issue": 1235,
            "clinical_boundary": CLINICAL_BOUNDARY,
            "inputs": {row["input_name"]: row for row in source_rows},
            "accepted_source_contract": {
                "sealed_issue1232_current_clinicaltrials_artifacts": True,
                "trial_registry_context_not_pair_interaction_proof": True,
                "adverse_event_modules_not_safety_clearance": True,
                "outcome_fields_not_efficacy_clearance": True,
            },
        },
    )
    output_manifest = {
        "schema_version": 1,
        "issue": 1235,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "source_rows": artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "clinicaltrials_trial_context_rows": artifact(out_dir / "clinicaltrials_trial_context_rows.jsonl", jsonl=True),
            "clinicaltrials_pair_validation_status": artifact(source_path, jsonl=True),
            "clinicaltrials_missing_gate_rows": artifact(out_dir / "clinicaltrials_missing_gate_rows.jsonl", jsonl=True),
            "issue1235_bridge_rows": artifact(out_dir / "issue1235_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        source_rows,
        hits,
        study_evidence,
        raw_studies,
        context_rows,
        pair_status,
        missing_gate_rows,
        bridge_rows,
        issue1232_persisted_readback,
        issue1232_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "pair_status": output_manifest["artifacts"]["clinicaltrials_pair_validation_status"],
            "trial_context": output_manifest["artifacts"]["clinicaltrials_trial_context_rows"],
            "missing_gates": output_manifest["artifacts"]["clinicaltrials_missing_gate_rows"],
            "bridge_rows": output_manifest["artifacts"]["issue1235_bridge_rows"],
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
