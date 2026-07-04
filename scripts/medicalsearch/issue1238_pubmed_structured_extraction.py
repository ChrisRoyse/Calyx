#!/usr/bin/env python3
"""#1238 structured extraction over #1237 PubMed validation rows.

This stage reads the sealed #1237 PubMed source-text validation artifacts and
extracts deterministic relation/safety/outcome fields from title/abstract text.
It emits structured literature-triage rows only. It is not efficacy, safety,
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
    "PubMed structured extraction is literature triage only; not efficacy, "
    "safety, clinical actionability, treatment guidance, dosing, "
    "recommendation, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "pubmed_title_abstract_structured_relation_safety_outcome_extraction_not_clinical_clearance"
)

ISSUE1237_ROOT = "/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1238-pubmed-structured-extraction-20260704T164501Z"

DEFAULT_INPUTS = {
    "issue1237_pubmed_evidence_validation": f"{ISSUE1237_ROOT}/out/pubmed_evidence_validation.jsonl",
    "issue1237_candidate_rollup": f"{ISSUE1237_ROOT}/out/candidate_pair_pubmed_validation_rollup.jsonl",
    "issue1237_source_records": f"{ISSUE1237_ROOT}/out/pubmed_source_records.jsonl",
    "issue1237_persisted_readback": f"{ISSUE1237_ROOT}/out/persisted_readback.json",
    "issue1237_calyx_readback": f"{ISSUE1237_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1237_output_manifest": f"{ISSUE1237_ROOT}/out/output_manifest.json",
}

PUBMED_RECORD_URL = "https://pubmed.ncbi.nlm.nih.gov/{pmid}/"

ELIGIBLE_RELATION_CLASSES = {
    "asserted_combination",
    "asserted_interaction",
    "asserted_outcome",
    "counter_evidence",
}

RELATION_RANK = {
    "insufficient_text": 0,
    "co_mention_only": 1,
    "asserted_outcome": 2,
    "asserted_combination": 3,
    "asserted_interaction": 4,
    "counter_evidence": 5,
}

MODEL_SYSTEM_PRIORITY = [
    "human_clinical_or_patient",
    "human_healthy_volunteer",
    "animal_or_xenograft",
    "in_vitro_or_cell_system",
    "computational_or_in_silico",
    "review_or_guideline",
    "unclear",
]

CONTEXT_PATTERNS = {
    "human_clinical_or_patient": [
        r"\bpatients?\b",
        r"\bhumans?\b",
        r"\bclinical\b",
        r"\bcase report\b",
        r"\bcohort\b",
        r"\brandomi[sz]ed\b",
        r"\bdouble[- ]blind\b",
        r"\btrial\b",
        r"\bphase\s+(?:i|ii|iii|iv|1|2|3|4)\b",
    ],
    "human_healthy_volunteer": [
        r"\bhealthy volunteers?\b",
        r"\bvolunteers?\b",
    ],
    "animal_or_xenograft": [
        r"\banimals?\b",
        r"\bmice\b",
        r"\bmouse\b",
        r"\brats?\b",
        r"\brabbits?\b",
        r"\bdogs?\b",
        r"\bxenograft",
        r"\bin vivo\b",
    ],
    "in_vitro_or_cell_system": [
        r"\bin vitro\b",
        r"\bcell(?:s| line| culture)?\b",
        r"\bculture\b",
        r"\bplasma\b",
    ],
    "computational_or_in_silico": [
        r"\bin silico\b",
        r"\bcomputational\b",
        r"\bsimulation\b",
        r"\bmodel(?:ing|ling)?\b",
        r"\bdatabase\b",
    ],
    "review_or_guideline": [
        r"\breview\b",
        r"\bguideline\b",
        r"\bmeta[- ]analysis\b",
        r"\bsystematic review\b",
    ],
}

RELATION_PATTERNS = {
    "combination_or_coexposure": [
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
        r"\bversus\b",
    ],
    "interaction_or_mechanism": [
        r"\bdrug[- ]drug interaction\b",
        r"\binteract",
        r"\bsynerg",
        r"\bantagon",
        r"\bpotentiat",
        r"\binhibit",
        r"\binduc",
        r"\bmetabolism\b",
        r"\bpharmacokinetic",
        r"\bpharmacodynamic",
        r"\bbioequivalence\b",
        r"\bbiosimilar",
    ],
}

OUTCOME_PATTERNS = [
    r"\befficacy\b",
    r"\beffective(?:ness)?\b",
    r"\bresponse\b",
    r"\bsurvival\b",
    r"\bremission\b",
    r"\bprogression\b",
    r"\bimprov",
    r"\boutcomes?\b",
    r"\bendpoint",
    r"\bpatient[- ]reported outcomes?\b",
    r"\btreat(?:ed|ment|s|ing)?\b",
    r"\btherapy\b",
    r"\btherapeutic\b",
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
    r"\bhepatotoxic",
    r"\bnephrotoxic",
    r"\barrhythm",
    r"\bimmunogenicity\b",
]

NEGATION_COUNTER_PATTERNS = [
    r"\bno significant\b",
    r"\bnot significant\b",
    r"\bdid not\b",
    r"\bfailed\b",
    r"\bfailure\b",
    r"\bwithout benefit\b",
    r"\black of\b",
    r"\bcontraindicat",
    r"\badverse events?\b",
    r"\btoxicit",
    r"\bmortality\b",
    r"\brisk(?:s)?\b",
]

DOSE_EXPOSURE_PATTERNS = [
    r"\b\d+(?:\.\d+)?\s*(?:mg|mcg|ug|g|kg|ml|l|iu|units?|mmol|umol|nmol|mol|%)\b",
    r"\b\d+(?:\.\d+)?\s*(?:mg|mcg|ug|g|iu|units?)/(?:kg|m2|day|d|h|hr|hour)\b",
    r"\b(?:single|multiple|high|low|daily|weekly|monthly)[- ]dose\b",
    r"\bdose[- ]response\b",
    r"\bdose(?:s|d)?\b",
    r"\bexposure\b",
    r"\bcoadministr",
    r"\bconcomitant\b",
    r"\bpharmacokinetic",
    r"\bbioavailability\b",
    r"\bauc\b",
    r"\bcmax\b",
    r"\bclearance\b",
]

DIRECTION_TRIGGER_PATTERNS = (
    RELATION_PATTERNS["interaction_or_mechanism"]
    + RELATION_PATTERNS["combination_or_coexposure"]
    + OUTCOME_PATTERNS
    + SAFETY_PATTERNS
)


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def stable_id(*parts: object, length: int = 24) -> str:
    payload = "\x1f".join(str(part) for part in parts)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:length]


def clean_text(value: object) -> str:
    if value is None:
        return ""
    return " ".join(str(value).replace("\x00", " ").split())


def normalized_phrase(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def normalized_blob(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return f" {' '.join(text.split())} "


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
    entry: dict[str, Any] = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            entry["rows"] = sum(1 for line in handle if line.strip())
    else:
        entry["rows"] = None
    return entry


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [str(path) for path in map(Path, inputs.values()) if not path.exists()]
    if missing:
        raise SystemExit(
            json.dumps(
                {
                    "code": "CALYX_DISCOVERY_SOURCE_MISSING",
                    "message": "required #1237 source artifacts are missing",
                    "missing": missing,
                    "remediation": "finish #1237 and persist its source/readback artifacts before running #1238",
                },
                indent=2,
            )
        )


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


def source_text_for_record(record: dict[str, Any] | None) -> str:
    if not record:
        return ""
    return clean_text(
        " ".join(
            [
                record.get("title") or "",
                record.get("abstract_text") or "",
                record.get("other_abstract_text") or "",
            ]
        )
    )


def snippet(text: str, start: int, end: int, window: int = 110) -> str:
    left = max(0, start - window)
    right = min(len(text), end + window)
    return clean_text(text[left:right])


def pattern_matches(patterns: list[str], text: str, *, limit: int = 8) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for pattern in patterns:
        for match in re.finditer(pattern, text, flags=re.IGNORECASE):
            out.append(
                {
                    "pattern": pattern,
                    "match": clean_text(match.group(0)),
                    "start": match.start(),
                    "end": match.end(),
                    "snippet": snippet(text, match.start(), match.end()),
                }
            )
            if len(out) >= limit:
                return out
    return out


def match_terms(matches: list[dict[str, Any]]) -> list[str]:
    return uniq([match.get("match") for match in matches])


def context_matches(text: str, publication_types: list[str], mesh_terms: list[str]) -> dict[str, list[dict[str, Any]]]:
    enriched_text = clean_text(
        " ".join(
            [
                text,
                " ".join(clean_text(item) for item in publication_types),
                " ".join(clean_text(item) for item in mesh_terms),
            ]
        )
    )
    return {
        label: pattern_matches(patterns, enriched_text, limit=6)
        for label, patterns in CONTEXT_PATTERNS.items()
    }


def model_systems_from_matches(matches: dict[str, list[dict[str, Any]]]) -> list[str]:
    systems = [label for label in MODEL_SYSTEM_PRIORITY if label != "unclear" and matches.get(label)]
    return systems or ["unclear"]


def primary_model_system(systems: list[str]) -> str:
    for system in MODEL_SYSTEM_PRIORITY:
        if system in systems:
            return system
    return "unclear"


def name_positions(text: str, name: str) -> list[int]:
    lower = text.lower()
    raw = clean_text(name).lower()
    positions: list[int] = []
    if raw:
        positions.extend(match.start() for match in re.finditer(re.escape(raw), lower))
    norm = normalized_phrase(name)
    if norm:
        blob = normalized_blob(text)
        offset = 0
        needle = f" {norm} "
        while True:
            index = blob.find(needle, offset)
            if index < 0:
                break
            positions.append(index)
            offset = index + 1
    return sorted(set(positions))


def relation_direction(text: str, drug_a: str, drug_b: str) -> dict[str, Any]:
    positions_a = name_positions(text, drug_a)
    positions_b = name_positions(text, drug_b)
    if not positions_a or not positions_b:
        return {
            "direction": "undirected_or_unclear",
            "basis": "one_or_both_candidate_names_not_located_in_source_text",
            "snippet": "",
        }
    best: tuple[int, int, int] | None = None
    for left in positions_a:
        for right in positions_b:
            distance = abs(right - left)
            if best is None or distance < best[0]:
                best = (distance, left, right)
    if best is None:
        return {"direction": "undirected_or_unclear", "basis": "no_name_pair_window", "snippet": ""}
    distance, left, right = best
    start = min(left, right)
    end = max(left, right)
    window_text = text[max(0, start - 80) : min(len(text), end + 160)]
    trigger = pattern_matches(DIRECTION_TRIGGER_PATTERNS, window_text, limit=1)
    if distance > 360 or not trigger:
        return {
            "direction": "undirected_or_unclear",
            "basis": "candidate_names_not_close_to_a_relation_trigger",
            "snippet": snippet(text, start, end, window=90),
        }
    if left < right:
        direction = "text_order_drug_a_then_drug_b_not_causal"
    else:
        direction = "text_order_drug_b_then_drug_a_not_causal"
    return {
        "direction": direction,
        "basis": "candidate_name_text_order_near_relation_trigger_not_causal_direction",
        "distance_chars": distance,
        "trigger_terms": match_terms(trigger),
        "snippet": clean_text(window_text),
    }


def extraction_status(relation_class: str) -> str:
    if relation_class == "counter_evidence":
        return "counter_evidence_structured_review_required_still_blocked"
    if relation_class == "asserted_interaction":
        return "structured_interaction_relation_still_blocked"
    if relation_class == "asserted_combination":
        return "structured_combination_relation_still_blocked"
    if relation_class == "asserted_outcome":
        return "structured_outcome_language_still_blocked"
    return "not_eligible_fail_closed"


def extraction_reason_codes(
    relation_class: str,
    safety_matches: list[dict[str, Any]],
    outcome_matches: list[dict[str, Any]],
    dose_matches: list[dict[str, Any]],
    direction: dict[str, Any],
) -> list[str]:
    codes = [
        "structured_extraction_not_clinical_clearance",
        "requires_independent_safety_outcome_falsification_and_human_review",
    ]
    if relation_class == "counter_evidence":
        codes.append("counter_evidence_preserved_for_falsification_review")
    elif relation_class == "asserted_interaction":
        codes.append("interaction_language_extracted_not_mechanistic_proof")
    elif relation_class == "asserted_combination":
        codes.append("combination_language_extracted_not_efficacy_or_safety_proof")
    elif relation_class == "asserted_outcome":
        codes.append("outcome_language_extracted_not_endpoint_validation")
    if safety_matches:
        codes.append("safety_language_present_requires_review")
    if outcome_matches:
        codes.append("outcome_language_present_requires_endpoint_gate")
    if dose_matches:
        codes.append("dose_or_exposure_language_present_not_dosing_guidance")
    if direction.get("direction") == "undirected_or_unclear":
        codes.append("relation_direction_unclear")
    else:
        codes.append("direction_is_text_order_only_not_causal")
    return codes


def next_validation_experiment(relation_class: str, has_safety: bool, has_outcome: bool, has_dose: bool) -> str:
    if relation_class == "counter_evidence":
        return "Route through falsification and safety review before any downstream ranking."
    required = ["independent safety gate", "falsification gate", "human review"]
    if has_outcome:
        required.append("endpoint/outcome validation")
    else:
        required.append("endpoint extraction from richer source text")
    if has_dose:
        required.append("dose/exposure normalization as non-guidance metadata")
    return "Run " + ", ".join(required) + " with physical readback."


def build_extraction_row(row: dict[str, Any], source_record: dict[str, Any] | None) -> dict[str, Any]:
    source_text = source_text_for_record(source_record)
    relation_class = row["relation_class"]
    publication_types = source_record.get("publication_types") if source_record else row.get("publication_types") or []
    mesh_terms = source_record.get("mesh_terms") if source_record else []
    context = context_matches(source_text, publication_types, mesh_terms)
    systems = model_systems_from_matches(context)
    direction = relation_direction(source_text, row["drug_a"], row["drug_b"])
    relation_matches = {
        label: pattern_matches(patterns, source_text, limit=10)
        for label, patterns in RELATION_PATTERNS.items()
    }
    outcome = pattern_matches(OUTCOME_PATTERNS, source_text, limit=10)
    safety = pattern_matches(SAFETY_PATTERNS, source_text, limit=10)
    negation = pattern_matches(NEGATION_COUNTER_PATTERNS, source_text, limit=10)
    dose = pattern_matches(DOSE_EXPOSURE_PATTERNS, source_text, limit=10)
    extraction_id = f"pubmed-structured-extraction:{stable_id(row['validation_id'], row['pmid'], relation_class)}"
    status = extraction_status(relation_class)
    return {
        "schema_version": 1,
        "extraction_id": extraction_id,
        "source_validation_id": row["validation_id"],
        "source_evidence_id": row["source_evidence_id"],
        "pair_key": row["pair_key"],
        "representative_pair_id": row["representative_pair_id"],
        "drug_a": row["drug_a"],
        "drug_b": row["drug_b"],
        "pmid": row["pmid"],
        "source_url": row.get("source_url") or PUBMED_RECORD_URL.format(pmid=row["pmid"]),
        "source_text_sha256": row.get("source_text_sha256"),
        "source_text_bytes": row.get("source_text_bytes", 0),
        "title": row.get("title", ""),
        "pubdate": row.get("pubdate", ""),
        "journal": row.get("journal", ""),
        "publication_types": publication_types,
        "source_record_present": source_record is not None,
        "source_relation_class": relation_class,
        "structured_extraction_status": status,
        "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
        "relation_direction": direction["direction"],
        "relation_direction_basis": direction["basis"],
        "relation_direction_window": direction.get("snippet", ""),
        "relation_direction_trigger_terms": direction.get("trigger_terms", []),
        "model_systems": systems,
        "primary_model_system": primary_model_system(systems),
        "context_terms": {label: match_terms(matches) for label, matches in context.items() if matches},
        "context_spans": {label: matches[:4] for label, matches in context.items() if matches},
        "relation_terms": {label: match_terms(matches) for label, matches in relation_matches.items() if matches},
        "relation_spans": {label: matches[:4] for label, matches in relation_matches.items() if matches},
        "outcome_endpoint_terms": match_terms(outcome),
        "outcome_endpoint_spans": outcome[:6],
        "safety_adverse_event_terms": match_terms(safety),
        "safety_adverse_event_spans": safety[:6],
        "negation_counterevidence_terms": match_terms(negation),
        "negation_counterevidence_spans": negation[:6],
        "dose_exposure_terms": match_terms(dose),
        "dose_exposure_spans": dose[:6],
        "has_outcome_language": bool(outcome),
        "has_safety_language": bool(safety),
        "has_negation_or_counterevidence_language": bool(negation),
        "has_dose_or_exposure_language": bool(dose),
        "reason_codes": extraction_reason_codes(relation_class, safety, outcome, dose, direction),
        "next_validation_experiment": next_validation_experiment(
            relation_class,
            has_safety=bool(safety),
            has_outcome=bool(outcome),
            has_dose=bool(dose),
        ),
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def candidate_rollups(
    issue1237_rollups: list[dict[str, Any]],
    extractions: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in extractions:
        by_pair[row["pair_key"]].append(row)
    rollups: list[dict[str, Any]] = []
    for prior in issue1237_rollups:
        rows = by_pair.get(prior["pair_key"], [])
        relation_counts = Counter(row["source_relation_class"] for row in rows)
        status_counts = Counter(row["structured_extraction_status"] for row in rows)
        model_counts = Counter(row["primary_model_system"] for row in rows)
        direction_counts = Counter(row["relation_direction"] for row in rows)
        has_counter = relation_counts.get("counter_evidence", 0) > 0
        has_safety = any(row["has_safety_language"] for row in rows)
        has_outcome = any(row["has_outcome_language"] for row in rows)
        has_dose = any(row["has_dose_or_exposure_language"] for row in rows)
        if has_counter:
            status = "counter_evidence_structured_review_required_still_blocked"
            reason = "counter_evidence_preserved_for_falsification_review"
            next_step = "Route counter-evidence rows through falsification and safety review."
        elif rows:
            status = "structured_relation_extracted_still_blocked"
            reason = "structured_fields_extracted_requires_downstream_gates"
            next_step = "Run independent safety, outcome, falsification, and human-review gates with physical readback."
        else:
            status = "no_structured_extraction_fail_closed"
            reason = "no_eligible_asserted_or_counter_evidence_rows"
            next_step = "Acquire asserted-relation source text before using this pair beyond co-mention triage."
        best_relation = best_relation_class(rows)
        rollups.append(
            {
                "schema_version": 1,
                "rollup_id": f"pubmed-structured-rollup:{stable_id(prior['rollup_id'], prior['pair_key'])}",
                "source_issue1237_rollup_id": prior["rollup_id"],
                "pair_id": prior["pair_id"],
                "pair_key": prior["pair_key"],
                "drug_a": prior["drug_a"],
                "drug_b": prior["drug_b"],
                "issue1237_best_relation_class": prior["best_relation_class"],
                "best_structured_relation_class": best_relation,
                "source_text_validated_rows": prior["source_text_validated_rows"],
                "issue1237_counter_evidence_rows": prior["counter_evidence_rows"],
                "eligible_structured_evidence_rows": len(rows),
                "structured_extraction_status": status,
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "structured_relation_class_counts": dict(sorted(relation_counts.items())),
                "structured_status_counts": dict(sorted(status_counts.items())),
                "primary_model_system_counts": dict(sorted(model_counts.items())),
                "relation_direction_counts": dict(sorted(direction_counts.items())),
                "has_counter_evidence": has_counter,
                "has_safety_language": has_safety,
                "has_outcome_language": has_outcome,
                "has_dose_or_exposure_language": has_dose,
                "representative_pmids": uniq([row["pmid"] for row in rows])[:12],
                "representative_extraction_ids": [row["extraction_id"] for row in rows[:12]],
                "safety_terms": uniq([term for row in rows for term in row["safety_adverse_event_terms"]])[:30],
                "outcome_terms": uniq([term for row in rows for term in row["outcome_endpoint_terms"]])[:30],
                "dose_exposure_terms": uniq([term for row in rows for term in row["dose_exposure_terms"]])[:30],
                "reason_codes": [
                    "structured_extraction_not_clinical_clearance",
                    reason,
                    "requires_independent_safety_outcome_falsification_and_human_review",
                ],
                "next_validation_experiment": next_step,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rollups.sort(
        key=lambda row: (
            RELATION_RANK.get(row["best_structured_relation_class"], 0) * -1,
            -row["eligible_structured_evidence_rows"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rollups


def best_relation_class(rows: list[dict[str, Any]]) -> str:
    if not rows:
        return "insufficient_text"
    return max(
        (row["source_relation_class"] for row in rows),
        key=lambda value: RELATION_RANK.get(value, 0),
    )


def build_bridge_rows(
    extractions: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollups:
        text = (
            f"PubMed structured extraction rollup {row['pair_id']}: {row['drug_a']} plus {row['drug_b']} "
            f"has status {row['structured_extraction_status']} with best structured relation "
            f"{row['best_structured_relation_class']}, {row['eligible_structured_evidence_rows']} eligible "
            f"evidence rows, safety language {row['has_safety_language']}, outcome language "
            f"{row['has_outcome_language']}, dose exposure language {row['has_dose_or_exposure_language']}, "
            f"and remains {row['promotion_status']}."
        )
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row["structured_extraction_status"],
                row["best_structured_relation_class"],
                *row.get("representative_pmids", [])[:3],
            ]
        )
        rows.append(
            {
                "id": row["rollup_id"],
                "domain": "pubmed_structured_extraction_rollup",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1238_pubmed_structured_extraction",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "structured_extraction_status": row["structured_extraction_status"],
                    "best_structured_relation_class": row["best_structured_relation_class"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in extractions:
        text = (
            f"PubMed structured extraction evidence {row['source_evidence_id']}: PMID {row['pmid']} for "
            f"{row['drug_a']} plus {row['drug_b']} is {row['source_relation_class']} with direction "
            f"{row['relation_direction']}, model {row['primary_model_system']}, safety language "
            f"{row['has_safety_language']}, outcome language {row['has_outcome_language']}, dose exposure "
            f"language {row['has_dose_or_exposure_language']}, and remains {row['structured_extraction_status']}."
        )
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row["pmid"],
                row["source_relation_class"],
                row["relation_direction"],
                row["primary_model_system"],
                row["structured_extraction_status"],
            ]
        )
        rows.append(
            {
                "id": row["extraction_id"],
                "domain": "pubmed_structured_extraction_evidence",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1238_pubmed_structured_extraction",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pmid": row["pmid"],
                    "source_relation_class": row["source_relation_class"],
                    "structured_extraction_status": row["structured_extraction_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows[:1000]


def build_metrics(
    validation_rows: list[dict[str, Any]],
    issue1237_rollups: list[dict[str, Any]],
    source_records: list[dict[str, Any]],
    extractions: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
) -> dict[str, Any]:
    relation_counts = Counter(row["source_relation_class"] for row in extractions)
    status_counts = Counter(row["structured_extraction_status"] for row in extractions)
    rollup_status_counts = Counter(row["structured_extraction_status"] for row in rollups)
    model_counts = Counter(row["primary_model_system"] for row in extractions)
    direction_counts = Counter(row["relation_direction"] for row in extractions)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1237_validation_rows": len(validation_rows),
        "issue1237_candidate_rollup_rows": len(issue1237_rollups),
        "issue1237_source_records": len(source_records),
        "eligible_relation_classes": sorted(ELIGIBLE_RELATION_CLASSES),
        "eligible_evidence_rows": len([row for row in validation_rows if row["relation_class"] in ELIGIBLE_RELATION_CLASSES]),
        "structured_extraction_rows": len(extractions),
        "candidate_pair_structured_rollup_rows": len(rollups),
        "structured_extraction_hits": sum(1 for row in rollups if row["eligible_structured_evidence_rows"] > 0),
        "counter_evidence_extraction_rows": relation_counts.get("counter_evidence", 0),
        "rows_with_safety_language": sum(1 for row in extractions if row["has_safety_language"]),
        "rows_with_outcome_language": sum(1 for row in extractions if row["has_outcome_language"]),
        "rows_with_dose_or_exposure_language": sum(1 for row in extractions if row["has_dose_or_exposure_language"]),
        "rows_with_unclear_direction": sum(1 for row in extractions if row["relation_direction"] == "undirected_or_unclear"),
        "structured_relation_class_counts": dict(sorted(relation_counts.items())),
        "structured_status_counts": dict(sorted(status_counts.items())),
        "rollup_status_counts": dict(sorted(rollup_status_counts.items())),
        "primary_model_system_counts": dict(sorted(model_counts.items())),
        "relation_direction_counts": dict(sorted(direction_counts.items())),
        "clinical_boundary_rows": sum(1 for row in extractions if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "structured_extraction_status": row["structured_extraction_status"],
                "best_structured_relation_class": row["best_structured_relation_class"],
                "eligible_structured_evidence_rows": row["eligible_structured_evidence_rows"],
                "has_safety_language": row["has_safety_language"],
                "has_outcome_language": row["has_outcome_language"],
                "representative_pmids": row["representative_pmids"][:5],
            }
            for row in rollups[:25]
        ],
    }


def all_assertions_true(readback: dict[str, Any]) -> bool:
    assertions = readback.get("assertions") or {}
    return bool(assertions) and all(value is True for value in assertions.values())


def build_input_manifest(
    inputs: dict[str, str],
    validation_rows: list[dict[str, Any]],
    issue1237_rollups: list[dict[str, Any]],
    source_records: list[dict[str, Any]],
    issue1237_persisted_readback: dict[str, Any],
    issue1237_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1238,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1237_pubmed_evidence_validation": artifact(
                Path(inputs["issue1237_pubmed_evidence_validation"]), jsonl=True
            ),
            "issue1237_candidate_rollup": artifact(Path(inputs["issue1237_candidate_rollup"]), jsonl=True),
            "issue1237_source_records": artifact(Path(inputs["issue1237_source_records"]), jsonl=True),
            "issue1237_persisted_readback": artifact(Path(inputs["issue1237_persisted_readback"])),
            "issue1237_calyx_readback": artifact(Path(inputs["issue1237_calyx_readback"])),
            "issue1237_output_manifest": artifact(Path(inputs["issue1237_output_manifest"])),
        },
        "source_contract": {
            "issue1237_persisted_readback_status": issue1237_persisted_readback.get("status"),
            "issue1237_persisted_assertions_all_true": all_assertions_true(issue1237_persisted_readback),
            "issue1237_calyx_readback_status": issue1237_calyx_readback.get("status"),
            "issue1237_calyx_assertions_all_true": all_assertions_true(issue1237_calyx_readback),
            "validation_rows": len(validation_rows),
            "candidate_rollup_rows": len(issue1237_rollups),
            "source_records": len(source_records),
            "eligible_relation_classes": sorted(ELIGIBLE_RELATION_CLASSES),
        },
        "method": {
            "type": "deterministic_source_text_structured_extraction",
            "uses_model": False,
            "uses_external_clinical_claims": False,
            "output_scope": "relation/context/model_system/dose_exposure/outcome/safety/negation spans only",
        },
    }


def build_readback(
    out_dir: Path,
    validation_rows: list[dict[str, Any]],
    issue1237_rollups: list[dict[str, Any]],
    source_records: list[dict[str, Any]],
    extractions: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
    issue1237_persisted_readback: dict[str, Any],
    issue1237_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "pubmed_structured_extraction": artifact(out_dir / "pubmed_structured_extraction.jsonl", jsonl=True),
        "candidate_pair_pubmed_structured_rollup": artifact(
            out_dir / "candidate_pair_pubmed_structured_rollup.jsonl", jsonl=True
        ),
        "candidate_pair_pubmed_structured_hits": artifact(
            out_dir / "candidate_pair_pubmed_structured_hits.jsonl", jsonl=True
        ),
        "pubmed_structured_extraction_bridge_rows": artifact(
            out_dir / "pubmed_structured_extraction_bridge_rows.jsonl", jsonl=True
        ),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    eligible_rows = [row for row in validation_rows if row["relation_class"] in ELIGIBLE_RELATION_CLASSES]
    return {
        "schema_version": 1,
        "issue": 1238,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1237_persisted_readback_all_true": all_assertions_true(issue1237_persisted_readback),
            "issue1237_calyx_readback_all_true": all_assertions_true(issue1237_calyx_readback),
            "extraction_row_for_every_eligible_validation_row": len(extractions) == len(eligible_rows),
            "rollup_row_for_every_issue1237_candidate_rollup": len(rollups) == len(issue1237_rollups),
            "all_extractions_have_source_records": all(row["source_record_present"] for row in extractions),
            "all_extractions_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in extractions),
            "all_rollups_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollups),
            "all_extractions_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in extractions
            ),
            "all_rollups_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in rollups
            ),
            "counter_evidence_rows_preserved": sum(
                1 for row in extractions if row["source_relation_class"] == "counter_evidence"
            )
            == sum(1 for row in validation_rows if row["relation_class"] == "counter_evidence"),
            "no_co_mention_or_insufficient_rows_extracted": all(
                row["source_relation_class"] in ELIGIBLE_RELATION_CLASSES for row in extractions
            ),
            "bridge_rows_1000_or_less": artifacts["pubmed_structured_extraction_bridge_rows"]["rows"] <= 1000,
            "bridge_rows_cover_rollups_and_extractions": artifacts["pubmed_structured_extraction_bridge_rows"][
                "rows"
            ]
            == min(1000, len(rollups) + len(extractions)),
            "source_records_cover_extraction_pmids": {row["pmid"] for row in extractions}.issubset(
                {row["pmid"] for row in source_records}
            ),
        },
        "row_counts": {
            "issue1237_validation_rows": len(validation_rows),
            "eligible_validation_rows": len(eligible_rows),
            "issue1237_candidate_rollup_rows": len(issue1237_rollups),
            "source_records": len(source_records),
            "structured_extraction_rows": len(extractions),
            "structured_rollup_rows": len(rollups),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_rows: int | None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    validation_rows = rows_jsonl(Path(inputs["issue1237_pubmed_evidence_validation"]))
    issue1237_rollups = rows_jsonl(Path(inputs["issue1237_candidate_rollup"]))
    source_records = rows_jsonl(Path(inputs["issue1237_source_records"]))
    issue1237_persisted_readback = read_json(Path(inputs["issue1237_persisted_readback"]))
    issue1237_calyx_readback = read_json(Path(inputs["issue1237_calyx_readback"]))

    source_by_pmid = {row["pmid"]: row for row in source_records}
    eligible_rows = [row for row in validation_rows if row["relation_class"] in ELIGIBLE_RELATION_CLASSES]
    eligible_rows.sort(key=lambda row: (row["pair_key"], row["pmid"], row["validation_id"]))
    if max_rows is not None:
        eligible_rows = eligible_rows[:max_rows]

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            validation_rows,
            issue1237_rollups,
            source_records,
            issue1237_persisted_readback,
            issue1237_calyx_readback,
        ),
    )

    extractions = [build_extraction_row(row, source_by_pmid.get(row["pmid"])) for row in eligible_rows]
    write_jsonl(out_dir / "pubmed_structured_extraction.jsonl", extractions)
    rollups = candidate_rollups(issue1237_rollups, extractions)
    write_jsonl(out_dir / "candidate_pair_pubmed_structured_rollup.jsonl", rollups)
    hits = [row for row in rollups if row["eligible_structured_evidence_rows"] > 0]
    write_jsonl(out_dir / "candidate_pair_pubmed_structured_hits.jsonl", hits)
    source_path = out_dir / "pubmed_structured_extraction.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(extractions, rollups, source_path, source_sha)
    write_jsonl(out_dir / "pubmed_structured_extraction_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(validation_rows, issue1237_rollups, source_records, extractions, rollups)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1238,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "pubmed_structured_extraction": artifact(out_dir / "pubmed_structured_extraction.jsonl", jsonl=True),
            "candidate_pair_pubmed_structured_rollup": artifact(
                out_dir / "candidate_pair_pubmed_structured_rollup.jsonl", jsonl=True
            ),
            "candidate_pair_pubmed_structured_hits": artifact(
                out_dir / "candidate_pair_pubmed_structured_hits.jsonl", jsonl=True
            ),
            "pubmed_structured_extraction_bridge_rows": artifact(
                out_dir / "pubmed_structured_extraction_bridge_rows.jsonl", jsonl=True
            ),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        validation_rows,
        issue1237_rollups,
        source_records,
        extractions,
        rollups,
        issue1237_persisted_readback,
        issue1237_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "pubmed_structured_extraction": output_manifest["artifacts"]["pubmed_structured_extraction"],
            "candidate_pair_pubmed_structured_rollup": output_manifest["artifacts"][
                "candidate_pair_pubmed_structured_rollup"
            ],
            "candidate_pair_pubmed_structured_hits": output_manifest["artifacts"][
                "candidate_pair_pubmed_structured_hits"
            ],
            "bridge_rows": output_manifest["artifacts"]["pubmed_structured_extraction_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1237-pubmed-evidence-validation")
    parser.add_argument("--issue1237-candidate-rollup")
    parser.add_argument("--issue1237-source-records")
    parser.add_argument("--issue1237-persisted-readback")
    parser.add_argument("--issue1237-calyx-readback")
    parser.add_argument("--issue1237-output-manifest")
    parser.add_argument("--max-rows", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1237_pubmed_evidence_validation", "issue1237_pubmed_evidence_validation"),
        ("issue1237_candidate_rollup", "issue1237_candidate_rollup"),
        ("issue1237_source_records", "issue1237_source_records"),
        ("issue1237_persisted_readback", "issue1237_persisted_readback"),
        ("issue1237_calyx_readback", "issue1237_calyx_readback"),
        ("issue1237_output_manifest", "issue1237_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(root, inputs, max_rows=args.max_rows)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
