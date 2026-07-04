#!/usr/bin/env python3
"""#1244 source-text relation validation for #1243 Europe PMC hits.

This stage reads sealed #1243 Europe PMC pair-search artifacts, reopens the
persisted metadata/full-text source text, verifies that both candidate terms are
present in source text, and classifies bounded relation/safety/outcome context
with a deterministic conservative rule set. The output is research triage only:
not efficacy, safety, treatment guidance, dosing guidance, recommendation,
clinical actionability, or cure evidence.
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
    "Europe PMC source-text relation validation is literature triage only; "
    "not efficacy, safety, treatment guidance, dosing guidance, recommendation, "
    "clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "europepmc_source_text_relation_safety_outcome_validation_not_clinical_clearance"
)

ISSUE1243_ROOT = "/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z"

DEFAULT_INPUTS = {
    "issue1243_candidate_status": f"{ISSUE1243_ROOT}/out/candidate_europepmc_status.jsonl",
    "issue1243_pair_evidence": f"{ISSUE1243_ROOT}/out/europepmc_pair_evidence.jsonl",
    "issue1243_pair_query_responses": f"{ISSUE1243_ROOT}/out/europepmc_pair_query_responses.jsonl",
    "issue1243_persisted_readback": f"{ISSUE1243_ROOT}/out/persisted_readback.json",
    "issue1243_calyx_readback": f"{ISSUE1243_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1243_output_manifest": f"{ISSUE1243_ROOT}/out/output_manifest.json",
    "issue1243_fulltext_dir": f"{ISSUE1243_ROOT}/raw/fulltext",
}

EUROPEPMC_RECORD_URL = "https://europepmc.org/article/{source}/{id}"
STATUS_VALUES = {
    "source_text_relation_extracted_still_blocked",
    "source_text_safety_or_counter_review_required_still_blocked",
    "source_text_comention_only_still_blocked",
    "source_text_validation_fail_closed",
}

RELATION_RANK = {
    "unclear_source_text": 0,
    "co_mention_only": 1,
    "comparative_context": 2,
    "trial_or_outcome_context": 3,
    "combination_or_coexposure": 4,
    "mechanistic_or_interaction_context": 5,
    "safety_or_adverse_context": 6,
    "counter_or_negative_context": 7,
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
        r"\bhuman(?:s)?\b",
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
    ],
    "comparative_context": [
        r"\bversus\b",
        r"\bcompared with\b",
        r"\bcompared to\b",
        r"\bcomparison\b",
        r"\bnon[- ]inferior",
        r"\bsuperior",
    ],
    "mechanistic_or_interaction_context": [
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
        r"\bbinding\b",
        r"\bpathway\b",
        r"\btarget\b",
    ],
    "trial_or_outcome_context": [
        r"\befficacy\b",
        r"\beffective(?:ness)?\b",
        r"\bresponse\b",
        r"\bsurvival\b",
        r"\bremission\b",
        r"\bprogression\b",
        r"\bimprov",
        r"\boutcomes?\b",
        r"\bendpoint",
        r"\btreat(?:ed|ment|s|ing)?\b",
        r"\btherapy\b",
        r"\btherapeutic\b",
        r"\btrial\b",
    ],
    "safety_or_adverse_context": [
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
    ],
    "counter_or_negative_context": [
        r"\bno significant\b",
        r"\bnot significant\b",
        r"\bdid not\b",
        r"\bfailed\b",
        r"\bfailure\b",
        r"\bwithout benefit\b",
        r"\black of\b",
        r"\bcontraindicat",
        r"\bnot recommended\b",
        r"\badverse events?\b",
        r"\btoxicit",
        r"\bmortality\b",
        r"\brisk(?:s)?\b",
    ],
}

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
    RELATION_PATTERNS["combination_or_coexposure"]
    + RELATION_PATTERNS["comparative_context"]
    + RELATION_PATTERNS["mechanistic_or_interaction_context"]
    + RELATION_PATTERNS["trial_or_outcome_context"]
    + RELATION_PATTERNS["safety_or_adverse_context"]
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


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\bchembl[: ]?chembl", "chembl", text, flags=re.IGNORECASE)
    return " ".join(text.split())


def normalized_phrase(value: object) -> str:
    text = query_name(value).lower()
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def normalized_blob(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return f" {' '.join(text.split())} "


def exact_presence(text: str, name: str) -> dict[str, Any]:
    raw = clean_text(name)
    query = query_name(raw)
    norm = normalized_phrase(raw)
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
            handle.write(json.dumps(row, sort_keys=True) + "\n")


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"required artifact is missing: {path}")
    item = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
        "rows": len(rows_jsonl(path)) if jsonl else None,
    }
    return item


def require_inputs(inputs: dict[str, str]) -> None:
    missing = []
    for key, value in inputs.items():
        path = Path(value)
        if key == "issue1243_fulltext_dir":
            if not path.is_dir():
                missing.append(value)
        elif not path.exists():
            missing.append(value)
    if missing:
        raise SystemExit(
            json.dumps(
                {
                    "code": "CALYX_DISCOVERY_SOURCE_MISSING",
                    "message": "required #1243 source artifacts are missing",
                    "missing": missing,
                    "remediation": "finish #1243 and persist Europe PMC source/readback artifacts before running #1244",
                },
                indent=2,
            )
        )


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(assertions.values())


def strings_from_value(value: Any) -> list[str]:
    out: list[str] = []
    if value is None:
        return out
    if isinstance(value, str):
        if value.strip():
            out.append(value)
    elif isinstance(value, dict):
        for item in value.values():
            out.extend(strings_from_value(item))
    elif isinstance(value, list):
        for item in value:
            out.extend(strings_from_value(item))
    return out


def metadata_text(item: dict[str, Any]) -> str:
    parts = []
    for key in ["title", "abstractText", "authorString", "pubTypeList", "keywordList", "journalInfo", "subsetList"]:
        parts.extend(strings_from_value(item.get(key)))
    return clean_text(" ".join(parts))


def search_results(response_json: dict[str, Any]) -> list[dict[str, Any]]:
    result_list = response_json.get("resultList") or {}
    if not isinstance(result_list, dict):
        return []
    results = result_list.get("result") or []
    return [item for item in results if isinstance(item, dict)] if isinstance(results, list) else []


def pmcid_of(item: dict[str, Any]) -> str:
    pmcid = clean_text(item.get("pmcid"))
    if pmcid:
        return pmcid if pmcid.upper().startswith("PMC") else f"PMC{pmcid}"
    ids = item.get("fullTextIdList") or {}
    if isinstance(ids, dict):
        for value in strings_from_value(ids):
            text = clean_text(value)
            if text.upper().startswith("PMC"):
                return text
    return ""


def result_source_id(item: dict[str, Any]) -> str:
    return pmcid_of(item) or clean_text(item.get("pmid")) or clean_text(item.get("doi")) or clean_text(item.get("id"))


def strip_xml(text: str) -> str:
    text = re.sub(r"<[^>]+>", " ", text)
    text = re.sub(r"&[a-zA-Z0-9#]+;", " ", text)
    return clean_text(text)


def source_url(source_record: dict[str, Any]) -> str:
    source = clean_text(source_record.get("source"))
    source_id = clean_text(source_record.get("id"))
    if source and source_id:
        return EUROPEPMC_RECORD_URL.format(source=source, id=source_id)
    pmcid = clean_text(source_record.get("pmcid"))
    if pmcid:
        return f"https://europepmc.org/article/PMC/{pmcid.removeprefix('PMC')}"
    return ""


def query_result_lookup(query_rows: list[dict[str, Any]]) -> dict[tuple[str, str], dict[str, Any]]:
    out: dict[tuple[str, str], dict[str, Any]] = {}
    for row in query_rows:
        for item in search_results(row.get("response_json") or {}):
            source_id = result_source_id(item)
            if source_id:
                out[(row["pair_key"], source_id)] = item
    return out


def source_text_for_evidence(
    evidence: dict[str, Any],
    result_lookup: dict[tuple[str, str], dict[str, Any]],
    fulltext_dir: Path,
) -> dict[str, Any]:
    source_id = clean_text(evidence.get("source_id"))
    pair_key = clean_text(evidence.get("pair_key"))
    source_record = evidence.get("source_record") or {}
    if evidence.get("evidence_channel") == "pmcid_fulltext_xml":
        pmcid = source_id if source_id.upper().startswith("PMC") else clean_text(source_record.get("pmcid"))
        path = fulltext_dir / f"{pmcid}.xml"
        payload = path.read_bytes() if path.exists() else b""
        text = strip_xml(payload.decode("utf-8", errors="replace"))
        return {
            "source_text": text,
            "source_text_bytes": len(payload),
            "source_text_sha256": sha256_bytes(payload),
            "source_text_path": str(path) if path.exists() else "",
            "source_text_available": bool(payload),
            "source_text_channel": "pmcid_fulltext_xml",
        }
    item = result_lookup.get((pair_key, source_id), {})
    text = metadata_text(item)
    payload = text.encode("utf-8")
    return {
        "source_text": text,
        "source_text_bytes": len(payload),
        "source_text_sha256": sha256_bytes(payload),
        "source_text_path": "",
        "source_text_available": bool(text),
        "source_text_channel": "metadata_text",
    }


def snippet(text: str, start: int, end: int, window: int = 140) -> str:
    left = max(0, start - window)
    right = min(len(text), end + window)
    return clean_text(text[left:right])


def pattern_matches(patterns: list[str], text: str, *, limit: int = 10) -> list[dict[str, Any]]:
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


def name_positions(text: str, name: str) -> list[int]:
    lower = text.lower()
    raw = clean_text(name).lower()
    positions: list[int] = []
    if raw:
        positions.extend(match.start() for match in re.finditer(re.escape(raw), lower))
    query = query_name(name).lower()
    if query and query != raw:
        positions.extend(match.start() for match in re.finditer(re.escape(query), lower))
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


def nearest_pair_window(text: str, drug_a: str, drug_b: str, *, window: int = 700) -> dict[str, Any]:
    positions_a = name_positions(text, drug_a)
    positions_b = name_positions(text, drug_b)
    if not positions_a or not positions_b:
        return {
            "source_window": "",
            "source_window_distance_chars": None,
            "source_window_basis": "one_or_both_names_not_located",
            "positions_a": positions_a[:10],
            "positions_b": positions_b[:10],
        }
    best: tuple[int, int, int] | None = None
    for left in positions_a:
        for right in positions_b:
            distance = abs(right - left)
            if best is None or distance < best[0]:
                best = (distance, left, right)
    if best is None:
        return {
            "source_window": "",
            "source_window_distance_chars": None,
            "source_window_basis": "no_pair_window",
            "positions_a": positions_a[:10],
            "positions_b": positions_b[:10],
        }
    distance, left, right = best
    start = min(left, right)
    end = max(left, right)
    return {
        "source_window": snippet(text, start, end, window=window),
        "source_window_distance_chars": distance,
        "source_window_basis": "nearest_candidate_name_pair",
        "positions_a": positions_a[:10],
        "positions_b": positions_b[:10],
    }


def relation_direction(text: str, drug_a: str, drug_b: str) -> dict[str, Any]:
    positions_a = name_positions(text, drug_a)
    positions_b = name_positions(text, drug_b)
    if not positions_a or not positions_b:
        return {"direction": "undirected_or_unclear", "basis": "one_or_both_candidate_names_not_located", "snippet": ""}
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
    window_text = text[max(0, start - 120) : min(len(text), end + 220)]
    trigger = pattern_matches(DIRECTION_TRIGGER_PATTERNS, window_text, limit=1)
    if distance > 480 or not trigger:
        return {
            "direction": "undirected_or_unclear",
            "basis": "candidate_names_not_close_to_a_relation_trigger",
            "snippet": snippet(text, start, end, window=100),
        }
    return {
        "direction": "text_order_drug_a_then_drug_b_not_causal" if left < right else "text_order_drug_b_then_drug_a_not_causal",
        "basis": "candidate_name_text_order_near_relation_trigger_not_causal_direction",
        "distance_chars": distance,
        "trigger_terms": match_terms(trigger),
        "snippet": clean_text(window_text),
    }


def context_matches(text: str, source_record: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    enriched = clean_text(
        " ".join(
            [
                text,
                clean_text(source_record.get("title")),
                clean_text(source_record.get("source")),
                clean_text(source_record.get("pub_year")),
            ]
        )
    )
    return {label: pattern_matches(patterns, enriched, limit=6) for label, patterns in CONTEXT_PATTERNS.items()}


def model_systems_from_matches(matches: dict[str, list[dict[str, Any]]]) -> list[str]:
    systems = [label for label in MODEL_SYSTEM_PRIORITY if label != "unclear" and matches.get(label)]
    return systems or ["unclear"]


def primary_model_system(systems: list[str]) -> str:
    for system in MODEL_SYSTEM_PRIORITY:
        if system in systems:
            return system
    return "unclear"


def classify_relation(relation_matches: dict[str, list[dict[str, Any]]], left_present: bool, right_present: bool) -> str:
    if not (left_present and right_present):
        return "unclear_source_text"
    present = [label for label, matches in relation_matches.items() if matches]
    if not present:
        return "co_mention_only"
    return max(present, key=lambda label: RELATION_RANK[label])


def validation_status(best_relation_class: str, has_safety: bool, has_counter: bool) -> str:
    if best_relation_class == "unclear_source_text":
        return "source_text_validation_fail_closed"
    if has_safety or has_counter or best_relation_class in {"safety_or_adverse_context", "counter_or_negative_context"}:
        return "source_text_safety_or_counter_review_required_still_blocked"
    if best_relation_class == "co_mention_only":
        return "source_text_comention_only_still_blocked"
    return "source_text_relation_extracted_still_blocked"


def reason_codes(row: dict[str, Any]) -> list[str]:
    codes = [
        "source_text_relation_validation_not_clinical_clearance",
        "requires_independent_safety_outcome_falsification_and_human_review",
    ]
    if row["source_text_validation_status"] == "source_text_validation_fail_closed":
        codes.append("source_text_missing_or_terms_not_verified")
    if row["best_relation_class"] == "co_mention_only":
        codes.append("co_mention_only_no_asserted_relation")
    if row["has_safety_language"]:
        codes.append("safety_language_present_requires_review")
    if row["has_counter_or_negative_language"]:
        codes.append("counter_or_negative_language_present_requires_falsification_review")
    if row["has_outcome_language"]:
        codes.append("outcome_language_present_requires_endpoint_gate")
    if row["has_dose_or_exposure_language"]:
        codes.append("dose_or_exposure_language_present_not_dosing_guidance")
    if row["relation_direction"] == "undirected_or_unclear":
        codes.append("relation_direction_unclear")
    else:
        codes.append("direction_is_text_order_only_not_causal")
    return codes


def next_validation_experiment(row: dict[str, Any]) -> str:
    if row["source_text_validation_status"] == "source_text_validation_fail_closed":
        return "Acquire richer source text before using this beyond no-promotion triage."
    steps = ["falsification gate", "independent safety gate", "human review"]
    if row["has_outcome_language"]:
        steps.append("endpoint/outcome validation")
    else:
        steps.append("outcome endpoint extraction from richer source text")
    if row["has_dose_or_exposure_language"]:
        steps.append("dose/exposure normalization as non-guidance metadata")
    return "Run " + ", ".join(steps) + " with physical readback."


def build_validation_rows(
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    fulltext_dir: Path,
    *,
    max_rows: int | None = None,
) -> list[dict[str, Any]]:
    lookup = query_result_lookup(query_rows)
    hits = [row for row in evidence_rows if row.get("europepmc_status") in {"exact_hit", "normalized_hit"}]
    hits.sort(key=lambda row: (row["pair_key"], row["source_id"], row["evidence_channel"], row["evidence_id"]))
    if max_rows is not None:
        hits = hits[:max_rows]
    out: list[dict[str, Any]] = []
    for row in hits:
        source_text = source_text_for_evidence(row, lookup, fulltext_dir)
        text = source_text["source_text"]
        left = exact_presence(text, row["drug_a"])
        right = exact_presence(text, row["drug_b"])
        pair_window = nearest_pair_window(text, row["drug_a"], row["drug_b"])
        analysis_text = pair_window["source_window"] or text
        direction = relation_direction(text, row["drug_a"], row["drug_b"])
        source_record = row.get("source_record") or {}
        context = context_matches(analysis_text, source_record)
        systems = model_systems_from_matches(context)
        relation_matches = {
            label: pattern_matches(patterns, analysis_text, limit=10) for label, patterns in RELATION_PATTERNS.items()
        }
        dose = pattern_matches(DOSE_EXPOSURE_PATTERNS, analysis_text, limit=10)
        best = classify_relation(relation_matches, left["present"], right["present"])
        has_safety = bool(relation_matches["safety_or_adverse_context"])
        has_counter = bool(relation_matches["counter_or_negative_context"])
        has_outcome = bool(relation_matches["trial_or_outcome_context"])
        validation_id = "europepmc-relation-validation:" + stable_id(
            row["evidence_id"], row["source_id"], row["evidence_channel"], source_text["source_text_sha256"]
        )
        validation = {
            "schema_version": 1,
            "validation_id": validation_id,
            "source_evidence_id": row["evidence_id"],
            "pair_key": row["pair_key"],
            "representative_pair_id": row["representative_pair_id"],
            "drug_a": row["drug_a"],
            "drug_b": row["drug_b"],
            "source": row["source"],
            "source_id": row["source_id"],
            "source_url": source_url(source_record),
            "source_record": source_record,
            "evidence_channel": row["evidence_channel"],
            "source_text_channel": source_text["source_text_channel"],
            "source_text_path": source_text["source_text_path"],
            "source_text_available": source_text["source_text_available"],
            "source_text_sha256": source_text["source_text_sha256"],
            "source_text_bytes": source_text["source_text_bytes"],
            "source_text_window": pair_window["source_window"],
            "source_text_window_distance_chars": pair_window["source_window_distance_chars"],
            "source_text_window_basis": pair_window["source_window_basis"],
            "left_presence": left,
            "right_presence": right,
            "terms_verified_in_source_text": left["present"] and right["present"],
            "best_relation_class": best,
            "source_text_validation_status": validation_status(best, has_safety, has_counter),
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
            "dose_exposure_terms": match_terms(dose),
            "dose_exposure_spans": dose[:6],
            "has_safety_language": has_safety,
            "has_counter_or_negative_language": has_counter,
            "has_outcome_language": has_outcome,
            "has_dose_or_exposure_language": bool(dose),
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        validation["reason_codes"] = reason_codes(validation)
        validation["next_validation_experiment"] = next_validation_experiment(validation)
        out.append(validation)
    return out


def best_relation_class(rows: list[dict[str, Any]]) -> str:
    if not rows:
        return "unclear_source_text"
    return max((row["best_relation_class"] for row in rows), key=lambda value: RELATION_RANK.get(value, 0))


def candidate_rollups(
    candidate_rows: list[dict[str, Any]],
    validation_rows: list[dict[str, Any]],
    *,
    max_candidates: int | None = None,
) -> list[dict[str, Any]]:
    hit_candidates = [row for row in candidate_rows if row.get("europepmc_status") in {"exact_hit", "normalized_hit"}]
    hit_candidates.sort(key=lambda row: (row["pair_key"], row["pair_id"]))
    if max_candidates is not None:
        keep = {row["pair_key"] for row in hit_candidates[:max_candidates]}
        hit_candidates = [row for row in hit_candidates if row["pair_key"] in keep]
    by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in validation_rows:
        by_pair[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for candidate in hit_candidates:
        rows = by_pair.get(candidate["pair_key"], [])
        relation_counts = Counter(row["best_relation_class"] for row in rows)
        status_counts = Counter(row["source_text_validation_status"] for row in rows)
        model_counts = Counter(row["primary_model_system"] for row in rows)
        direction_counts = Counter(row["relation_direction"] for row in rows)
        best = best_relation_class(rows)
        has_safety = any(row["has_safety_language"] for row in rows)
        has_counter = any(row["has_counter_or_negative_language"] for row in rows)
        has_outcome = any(row["has_outcome_language"] for row in rows)
        has_dose = any(row["has_dose_or_exposure_language"] for row in rows)
        status = validation_status(best, has_safety, has_counter) if rows else "source_text_validation_fail_closed"
        rollup = {
            "schema_version": 1,
            "rollup_id": "europepmc-relation-rollup:" + stable_id(candidate["status_id"], candidate["pair_key"], best),
            "source_issue1243_status_id": candidate["status_id"],
            "pair_id": candidate["pair_id"],
            "pair_key": candidate["pair_key"],
            "drug_a": candidate["drug_a"],
            "drug_b": candidate["drug_b"],
            "issue1243_europepmc_status": candidate["europepmc_status"],
            "best_relation_class": best,
            "source_text_validation_status": status,
            "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
            "validation_evidence_rows": len(rows),
            "terms_verified_evidence_rows": sum(1 for row in rows if row["terms_verified_in_source_text"]),
            "relation_class_counts": dict(sorted(relation_counts.items())),
            "source_text_status_counts": dict(sorted(status_counts.items())),
            "primary_model_system_counts": dict(sorted(model_counts.items())),
            "relation_direction_counts": dict(sorted(direction_counts.items())),
            "has_safety_language": has_safety,
            "has_counter_or_negative_language": has_counter,
            "has_outcome_language": has_outcome,
            "has_dose_or_exposure_language": has_dose,
            "source_ids": uniq([row["source_id"] for row in rows])[:25],
            "validation_ids": [row["validation_id"] for row in rows[:25]],
            "safety_terms": uniq([term for row in rows for term in row["relation_terms"].get("safety_or_adverse_context", [])])[:30],
            "counter_or_negative_terms": uniq(
                [term for row in rows for term in row["relation_terms"].get("counter_or_negative_context", [])]
            )[:30],
            "outcome_terms": uniq([term for row in rows for term in row["relation_terms"].get("trial_or_outcome_context", [])])[:30],
            "dose_exposure_terms": uniq([term for row in rows for term in row["dose_exposure_terms"]])[:30],
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        rollup["reason_codes"] = reason_codes(
            {
                "source_text_validation_status": status,
                "best_relation_class": best,
                "has_safety_language": has_safety,
                "has_counter_or_negative_language": has_counter,
                "has_outcome_language": has_outcome,
                "has_dose_or_exposure_language": has_dose,
                "relation_direction": "undirected_or_unclear"
                if not rows
                else max(direction_counts, key=lambda key: direction_counts[key]),
            }
        )
        rollup["next_validation_experiment"] = next_validation_experiment(
            {
                "source_text_validation_status": status,
                "has_outcome_language": has_outcome,
                "has_dose_or_exposure_language": has_dose,
            }
        )
        out.append(rollup)
    out.sort(
        key=lambda row: (
            -RELATION_RANK.get(row["best_relation_class"], 0),
            -row["validation_evidence_rows"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return out


def bridge_terms(values: list[object], text: str) -> list[str]:
    return [term for term in uniq(values) if term and clean_text(term) in text]


def build_bridge_rows(
    validation_rows: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollups:
        text = (
            f"Europe PMC relation validation rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['source_text_validation_status']} "
            f"best relation {row['best_relation_class']} evidence rows {row['validation_evidence_rows']} "
            f"safety {row['has_safety_language']} counter {row['has_counter_or_negative_language']} "
            f"outcome {row['has_outcome_language']} dose {row['has_dose_or_exposure_language']} "
            f"promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_id"],
                "domain": "europepmc_relation_validation_rollup",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["pair_id"],
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["source_text_validation_status"],
                        row["best_relation_class"],
                    ],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1244_europepmc_relation_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "pair_key": row["pair_key"],
                    "source_text_validation_status": row["source_text_validation_status"],
                    "best_relation_class": row["best_relation_class"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in validation_rows[:budget]:
        text = (
            f"Europe PMC relation validation evidence {row['validation_id']} source {row['source_id']} "
            f"pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} relation "
            f"{row['best_relation_class']} status {row['source_text_validation_status']} "
            f"model {row['primary_model_system']} safety {row['has_safety_language']} "
            f"counter {row['has_counter_or_negative_language']} outcome {row['has_outcome_language']} "
            f"dose {row['has_dose_or_exposure_language']}."
        )
        rows.append(
            {
                "id": row["validation_id"],
                "domain": "europepmc_relation_validation_evidence",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["validation_id"],
                        row["source_id"],
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["best_relation_class"],
                        row["source_text_validation_status"],
                        row["primary_model_system"],
                    ],
                    text,
                ),
                "metadata": {
                    "source_dataset": "issue1244_europepmc_relation_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "source_id": row["source_id"],
                    "pair_key": row["pair_key"],
                    "source_text_validation_status": row["source_text_validation_status"],
                    "best_relation_class": row["best_relation_class"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    candidate_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    validation_rows: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
) -> dict[str, Any]:
    relation_counts = Counter(row["best_relation_class"] for row in validation_rows)
    status_counts = Counter(row["source_text_validation_status"] for row in validation_rows)
    rollup_status_counts = Counter(row["source_text_validation_status"] for row in rollups)
    model_counts = Counter(row["primary_model_system"] for row in validation_rows)
    direction_counts = Counter(row["relation_direction"] for row in validation_rows)
    hit_candidates = [row for row in candidate_rows if row.get("europepmc_status") in {"exact_hit", "normalized_hit"}]
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1243_candidate_rows": len(candidate_rows),
        "issue1243_hit_candidate_rows": len(hit_candidates),
        "issue1243_evidence_rows": len(evidence_rows),
        "source_text_validation_rows": len(validation_rows),
        "candidate_relation_rollup_rows": len(rollups),
        "candidate_relation_rollups_with_source_text_terms_verified": sum(
            1 for row in rollups if row["terms_verified_evidence_rows"] > 0
        ),
        "rows_with_safety_language": sum(1 for row in validation_rows if row["has_safety_language"]),
        "rows_with_counter_or_negative_language": sum(
            1 for row in validation_rows if row["has_counter_or_negative_language"]
        ),
        "rows_with_outcome_language": sum(1 for row in validation_rows if row["has_outcome_language"]),
        "rows_with_dose_or_exposure_language": sum(
            1 for row in validation_rows if row["has_dose_or_exposure_language"]
        ),
        "relation_class_counts": dict(sorted(relation_counts.items())),
        "source_text_status_counts": dict(sorted(status_counts.items())),
        "rollup_status_counts": dict(sorted(rollup_status_counts.items())),
        "primary_model_system_counts": dict(sorted(model_counts.items())),
        "relation_direction_counts": dict(sorted(direction_counts.items())),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "source_text_validation_status": row["source_text_validation_status"],
                "best_relation_class": row["best_relation_class"],
                "validation_evidence_rows": row["validation_evidence_rows"],
                "has_safety_language": row["has_safety_language"],
                "has_counter_or_negative_language": row["has_counter_or_negative_language"],
                "has_outcome_language": row["has_outcome_language"],
                "source_ids": row["source_ids"][:5],
            }
            for row in rollups[:25]
        ],
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidate_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    issue1243_persisted_readback: dict[str, Any],
    issue1243_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    hit_candidates = [row for row in candidate_rows if row.get("europepmc_status") in {"exact_hit", "normalized_hit"}]
    return {
        "schema_version": 1,
        "issue": 1244,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1243_candidate_status": artifact(Path(inputs["issue1243_candidate_status"]), jsonl=True),
            "issue1243_pair_evidence": artifact(Path(inputs["issue1243_pair_evidence"]), jsonl=True),
            "issue1243_pair_query_responses": artifact(Path(inputs["issue1243_pair_query_responses"]), jsonl=True),
            "issue1243_persisted_readback": artifact(Path(inputs["issue1243_persisted_readback"])),
            "issue1243_calyx_readback": artifact(Path(inputs["issue1243_calyx_readback"])),
            "issue1243_output_manifest": artifact(Path(inputs["issue1243_output_manifest"])),
        },
        "source_contract": {
            "issue1243_persisted_readback_status": issue1243_persisted_readback.get("status"),
            "issue1243_persisted_assertions_all_true": all_assertions_true(issue1243_persisted_readback),
            "issue1243_calyx_readback_status": issue1243_calyx_readback.get("status"),
            "issue1243_calyx_assertions_all_true": all_assertions_true(issue1243_calyx_readback),
            "issue1243_candidate_rows": len(candidate_rows),
            "issue1243_hit_candidate_rows": len(hit_candidates),
            "issue1243_evidence_rows": len(evidence_rows),
            "issue1243_query_response_rows": len(query_rows),
        },
        "method": {
            "type": "deterministic_source_text_relation_safety_outcome_validation",
            "uses_model": False,
            "fetches_new_source_data": False,
            "output_scope": "source-text relation/context/model/dose/outcome/safety/counter spans only",
        },
    }


def build_readback(
    out_dir: Path,
    candidate_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    validation_rows: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
    issue1243_persisted_readback: dict[str, Any],
    issue1243_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "europepmc_source_text_relation_validation": artifact(
            out_dir / "europepmc_source_text_relation_validation.jsonl", jsonl=True
        ),
        "candidate_europepmc_relation_rollup": artifact(
            out_dir / "candidate_europepmc_relation_rollup.jsonl", jsonl=True
        ),
        "candidate_europepmc_relation_review": artifact(
            out_dir / "candidate_europepmc_relation_review.jsonl", jsonl=True
        ),
        "europepmc_relation_bridge_rows": artifact(out_dir / "europepmc_relation_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    hit_candidates = [row for row in candidate_rows if row.get("europepmc_status") in {"exact_hit", "normalized_hit"}]
    evidence_ids = {row["evidence_id"] for row in evidence_rows}
    validation_evidence_ids = {row["source_evidence_id"] for row in validation_rows}
    return {
        "schema_version": 1,
        "issue": 1244,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1243_persisted_readback_all_true": all_assertions_true(issue1243_persisted_readback),
            "issue1243_calyx_readback_all_true": all_assertions_true(issue1243_calyx_readback),
            "validation_row_for_every_issue1243_evidence_row": len(validation_rows) == len(evidence_rows),
            "rollup_row_for_every_issue1243_hit_candidate": len(rollups) == len(hit_candidates),
            "all_validation_rows_reference_known_evidence": validation_evidence_ids <= evidence_ids,
            "all_validation_rows_have_source_text": all(row["source_text_available"] for row in validation_rows),
            "all_validation_rows_verify_terms": all(row["terms_verified_in_source_text"] for row in validation_rows),
            "all_validation_rows_have_boundary": all(
                row["clinical_boundary"] == CLINICAL_BOUNDARY for row in validation_rows
            ),
            "all_rollups_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollups),
            "all_validation_rows_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in validation_rows
            ),
            "all_rollups_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in rollups
            ),
            "all_rollup_status_values_allowed": all(row["source_text_validation_status"] in STATUS_VALUES for row in rollups),
            "bridge_rows_1000_or_less": artifacts["europepmc_relation_bridge_rows"]["rows"] <= 1000,
            "bridge_rows_cover_rollups_plus_budgeted_evidence": artifacts["europepmc_relation_bridge_rows"]["rows"]
            == min(1000, len(rollups) + len(validation_rows)),
        },
        "row_counts": {
            "issue1243_candidate_rows": len(candidate_rows),
            "issue1243_hit_candidate_rows": len(hit_candidates),
            "issue1243_evidence_rows": len(evidence_rows),
            "source_text_validation_rows": len(validation_rows),
            "candidate_relation_rollup_rows": len(rollups),
        },
    }


def run(
    root: Path,
    inputs: dict[str, str],
    *,
    max_evidence_rows: int | None,
    max_candidates: int | None,
) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    candidate_rows = rows_jsonl(Path(inputs["issue1243_candidate_status"]))
    evidence_rows = rows_jsonl(Path(inputs["issue1243_pair_evidence"]))
    query_rows = rows_jsonl(Path(inputs["issue1243_pair_query_responses"]))
    issue1243_persisted_readback = read_json(Path(inputs["issue1243_persisted_readback"]))
    issue1243_calyx_readback = read_json(Path(inputs["issue1243_calyx_readback"]))

    if max_evidence_rows is not None:
        evidence_rows = evidence_rows[:max_evidence_rows]

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            candidate_rows,
            evidence_rows,
            query_rows,
            issue1243_persisted_readback,
            issue1243_calyx_readback,
        ),
    )
    validation_rows = build_validation_rows(
        evidence_rows,
        query_rows,
        Path(inputs["issue1243_fulltext_dir"]),
        max_rows=max_evidence_rows,
    )
    write_jsonl(out_dir / "europepmc_source_text_relation_validation.jsonl", validation_rows)
    rollups = candidate_rollups(candidate_rows, validation_rows, max_candidates=max_candidates)
    write_jsonl(out_dir / "candidate_europepmc_relation_rollup.jsonl", rollups)
    review_rows = [
        row for row in rollups if row["source_text_validation_status"] != "source_text_comention_only_still_blocked"
    ]
    write_jsonl(out_dir / "candidate_europepmc_relation_review.jsonl", review_rows)
    source_path = out_dir / "europepmc_source_text_relation_validation.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(validation_rows, rollups, source_path, source_sha)
    write_jsonl(out_dir / "europepmc_relation_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(candidate_rows, evidence_rows, validation_rows, rollups)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1244,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "europepmc_source_text_relation_validation": artifact(
                out_dir / "europepmc_source_text_relation_validation.jsonl", jsonl=True
            ),
            "candidate_europepmc_relation_rollup": artifact(
                out_dir / "candidate_europepmc_relation_rollup.jsonl", jsonl=True
            ),
            "candidate_europepmc_relation_review": artifact(
                out_dir / "candidate_europepmc_relation_review.jsonl", jsonl=True
            ),
            "europepmc_relation_bridge_rows": artifact(out_dir / "europepmc_relation_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        candidate_rows,
        evidence_rows,
        validation_rows,
        rollups,
        issue1243_persisted_readback,
        issue1243_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "europepmc_source_text_relation_validation": output_manifest["artifacts"][
                "europepmc_source_text_relation_validation"
            ],
            "candidate_europepmc_relation_rollup": output_manifest["artifacts"][
                "candidate_europepmc_relation_rollup"
            ],
            "candidate_europepmc_relation_review": output_manifest["artifacts"][
                "candidate_europepmc_relation_review"
            ],
            "bridge_rows": output_manifest["artifacts"]["europepmc_relation_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1243-candidate-status")
    parser.add_argument("--issue1243-pair-evidence")
    parser.add_argument("--issue1243-pair-query-responses")
    parser.add_argument("--issue1243-persisted-readback")
    parser.add_argument("--issue1243-calyx-readback")
    parser.add_argument("--issue1243-output-manifest")
    parser.add_argument("--issue1243-fulltext-dir")
    parser.add_argument("--max-evidence-rows", type=int)
    parser.add_argument("--max-candidates", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1243_candidate_status", "issue1243_candidate_status"),
        ("issue1243_pair_evidence", "issue1243_pair_evidence"),
        ("issue1243_pair_query_responses", "issue1243_pair_query_responses"),
        ("issue1243_persisted_readback", "issue1243_persisted_readback"),
        ("issue1243_calyx_readback", "issue1243_calyx_readback"),
        ("issue1243_output_manifest", "issue1243_output_manifest"),
        ("issue1243_fulltext_dir", "issue1243_fulltext_dir"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(
        Path(args.root),
        inputs,
        max_evidence_rows=args.max_evidence_rows,
        max_candidates=args.max_candidates,
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
