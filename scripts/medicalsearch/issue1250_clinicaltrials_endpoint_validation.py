#!/usr/bin/env python3
"""#1250 independent ClinicalTrials.gov endpoint validation for #1247 rollups.

This stage reads sealed #1247 endpoint/outcome review rows, queries the current
ClinicalTrials.gov v2 API for each unique pair key, requires both candidate
terms in the same trial intervention text before marking a registry source
hit, and extracts protocol/results outcome fields when present. Registry
endpoint documentation is not efficacy, safety, dosing, treatment guidance,
recommendation, clinical actionability, or cure evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "ClinicalTrials.gov endpoint validation is registry documentation triage "
    "only; not efficacy, safety, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "clinicaltrials_registry_endpoint_documentation_not_efficacy_or_safety_clearance"
)

ISSUE1247_ROOT = "/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z"

DEFAULT_INPUTS = {
    "issue1247_rollup": f"{ISSUE1247_ROOT}/out/candidate_europepmc_endpoint_outcome_rollup.jsonl",
    "issue1247_evidence_review": f"{ISSUE1247_ROOT}/out/europepmc_endpoint_outcome_evidence_review.jsonl",
    "issue1247_persisted_readback": f"{ISSUE1247_ROOT}/out/persisted_readback.json",
    "issue1247_calyx_readback": f"{ISSUE1247_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1247_output_manifest": f"{ISSUE1247_ROOT}/out/output_manifest.json",
}

API_BASE = "https://clinicaltrials.gov/api/v2/studies"
API_SPEC_URL = "https://clinicaltrials.gov/api/oas/v2"
API_DOC_URL = "https://clinicaltrials.gov/data-api/api"
API_ABOUT_URL = "https://clinicaltrials.gov/data-api/about-api"
PAGE_SIZE = 100
REQUEST_SLEEP_SECONDS = 0.05
USER_AGENT = "calyx-discovery/issue1250"
PROMOTION_STATUS = (
    "blocked_requires_independent_endpoint_result_safety_falsification_and_human_review"
)

STATUS_VALUES = {
    "clinicaltrials_registry_endpoint_exact_hit_still_blocked",
    "clinicaltrials_registry_endpoint_normalized_hit_still_blocked",
    "clinicaltrials_registry_intervention_hit_no_endpoint_still_blocked",
    "clinicaltrials_registry_no_hit_still_blocked",
    "clinicaltrials_registry_not_queryable_still_blocked",
}


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
    if isinstance(value, list):
        value = " ".join(clean_text(item) for item in value)
    return " ".join(str(value).replace("\x00", " ").split())


def norm_text(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def contains_norm(text: str, needle: str) -> bool:
    if not needle:
        return False
    return f" {needle} " in f" {norm_text(text)} "


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
    return " ".join(text.split())


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


def artifact(path: Path, *, jsonl: bool = False, source_url: str | None = None) -> dict[str, Any]:
    value = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            value["rows"] = sum(1 for line in handle if line.strip())
    if source_url:
        value["source_url"] = source_url
    return value


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [name for name, value in inputs.items() if not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(assertions.values())


def fetch_bytes(url: str, retries: int = 5) -> tuple[int, bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=45) as response:
                return response.status, response.read()
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            last_error = error
            time.sleep(min(8.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"Fetch failed after {retries} attempts for {url}: {last_error}")


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    sources = {
        "clinicaltrials_oas_v2": (API_SPEC_URL, "clinicaltrials_oas_v2.yaml"),
        "clinicaltrials_api_doc": (API_DOC_URL, "clinicaltrials_api.html"),
        "clinicaltrials_about_api": (API_ABOUT_URL, "clinicaltrials_about_api.html"),
    }
    raw_dir.mkdir(parents=True, exist_ok=True)
    artifacts: dict[str, dict[str, Any]] = {}
    for name, (url, filename) in sources.items():
        path = raw_dir / filename
        if not path.exists():
            status, payload = fetch_bytes(url)
            path.write_bytes(payload)
            (raw_dir / f"{filename}.status").write_text(str(status) + "\n", encoding="utf-8")
        artifacts[name] = artifact(path, source_url=url)
    return artifacts


def representative_pairs(rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_key: dict[str, dict[str, Any]] = {}
    for row in sorted(rollups, key=lambda item: (item["pair_key"], item["pair_id"])):
        if row["pair_key"] in by_key:
            by_key[row["pair_key"]]["source_rollup_ids"].append(row["rollup_review_id"])
            by_key[row["pair_key"]]["source_pair_ids"].append(row["pair_id"])
            continue
        left = query_name(row["drug_a"])
        right = query_name(row["drug_b"])
        by_key[row["pair_key"]] = {
            "schema_version": 1,
            "pair_key": row["pair_key"],
            "representative_pair_id": row["pair_id"],
            "drug_a": row["drug_a"],
            "drug_b": row["drug_b"],
            "query_left": left,
            "query_right": right,
            "query_text": " ".join(part for part in [left, right] if part),
            "queryable": bool(left and right),
            "source_rollup_ids": [row["rollup_review_id"]],
            "source_pair_ids": [row["pair_id"]],
        }
    return list(by_key.values())


def query_url_for_text(text: str, page_token: str | None = None) -> str:
    params = {
        "format": "json",
        "pageSize": str(PAGE_SIZE),
        "countTotal": "true",
        "query.intr": text,
    }
    if page_token:
        params["pageToken"] = page_token
    return f"{API_BASE}?{urllib.parse.urlencode(params)}"


def fetch_json(url: str, retries: int = 4) -> tuple[int, dict[str, Any], bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = response.read()
                return response.status, json.loads(payload), payload
        except urllib.error.HTTPError as error:
            payload = error.read()
            if error.code == 400:
                body = payload.decode("utf-8", errors="replace")
                return error.code, {
                    "totalCount": 0,
                    "studies": [],
                    "api_error": {
                        "http_status": error.code,
                        "reason": clean_text(error.reason),
                        "body": body[:1000],
                    },
                }, payload
            last_error = error
            time.sleep(min(8.0, 1.5 * (attempt + 1)))
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            last_error = error
            time.sleep(min(8.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"ClinicalTrials.gov fetch failed after {retries} attempts: {last_error}")


def fetch_all_pages(pair: dict[str, Any], request_sleep_seconds: float) -> tuple[dict[str, Any], bytes, list[str], list[str], list[int], list[int]]:
    if not pair["queryable"]:
        payload = json.dumps({"totalCount": 0, "studies": [], "not_queryable": True}, sort_keys=True).encode("utf-8")
        return {"totalCount": 0, "studies": [], "not_queryable": True, "pages": []}, payload, [], [], [], []
    url = query_url_for_text(pair["query_text"])
    pages: list[dict[str, Any]] = []
    page_urls: list[str] = []
    page_hashes: list[str] = []
    page_bytes: list[int] = []
    page_statuses: list[int] = []
    total_count: int | None = None
    while True:
        status, data, payload = fetch_json(url)
        pages.append(data)
        page_urls.append(url)
        page_hashes.append(sha256_bytes(payload))
        page_bytes.append(len(payload))
        page_statuses.append(status)
        if total_count is None:
            total_count = data.get("totalCount")
        token = data.get("nextPageToken")
        if not token:
            break
        url = query_url_for_text(pair["query_text"], page_token=token)
        time.sleep(request_sleep_seconds)
    merged = {
        "totalCount": total_count,
        "studies": [study for page in pages for study in page.get("studies", [])],
        "pages": pages,
    }
    errors = [page.get("api_error") for page in pages if page.get("api_error")]
    if errors:
        merged["api_errors"] = errors
    payload = json.dumps(merged, sort_keys=True).encode("utf-8")
    return merged, payload, page_urls, page_hashes, page_bytes, page_statuses


def cached_query_rows(path: Path, expected_keys: set[str]) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    rows = rows_jsonl(path)
    if {row["pair_key"] for row in rows} != expected_keys:
        return None
    if not all("response" in row for row in rows):
        return None
    return rows


def fetch_query_rows(
    pairs: list[dict[str, Any]],
    out_path: Path,
    request_sleep_seconds: float,
) -> list[dict[str, Any]]:
    expected = {row["pair_key"] for row in pairs}
    cached = cached_query_rows(out_path, expected)
    if cached is not None:
        return cached
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        data, payload, page_urls, page_hashes, page_bytes, page_statuses = fetch_all_pages(pair, request_sleep_seconds)
        rows.append(
            {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_left": pair["query_left"],
                "query_right": pair["query_right"],
                "query_text": pair["query_text"],
                "queryable": pair["queryable"],
                "query_url": page_urls[0] if page_urls else "",
                "page_urls": page_urls,
                "page_response_sha256s": page_hashes,
                "page_response_bytes": page_bytes,
                "page_http_statuses": page_statuses,
                "page_count": len(page_urls),
                "api_base": API_BASE,
                "page_size": PAGE_SIZE,
                "api_errors": data.get("api_errors") or [],
                "total_count": data.get("totalCount"),
                "returned_studies": len(data.get("studies", [])),
                "next_page_token_present": any(bool(page.get("nextPageToken")) for page in data.get("pages", [])),
                "response_bytes": len(payload),
                "response_sha256": sha256_bytes(payload),
                "response": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        if index % 20 == 0:
            print(f"fetched {index}/{len(pairs)} ClinicalTrials.gov endpoint queries", file=sys.stderr)
        time.sleep(request_sleep_seconds)
    write_jsonl(out_path, rows)
    return rows


def flatten_strings(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    if isinstance(value, (int, float, bool)):
        return [str(value)]
    if isinstance(value, list):
        out: list[str] = []
        for item in value:
            out.extend(flatten_strings(item))
        return out
    if isinstance(value, dict):
        out: list[str] = []
        for item in value.values():
            out.extend(flatten_strings(item))
        return out
    return [str(value)]


def outcome_rows(study: dict[str, Any]) -> list[dict[str, Any]]:
    protocol = study.get("protocolSection") or {}
    outcomes = protocol.get("outcomesModule") or {}
    results = study.get("resultsSection") or {}
    result_outcomes = (results.get("outcomeMeasuresModule") or {}).get("outcomeMeasures") or []
    rows: list[dict[str, Any]] = []
    for outcome_type, key in [
        ("primary", "primaryOutcomes"),
        ("secondary", "secondaryOutcomes"),
        ("other", "otherOutcomes"),
    ]:
        for idx, outcome in enumerate(outcomes.get(key) or []):
            rows.append(
                {
                    "source": "protocol",
                    "outcome_type": outcome_type,
                    "ordinal": idx,
                    "measure": clean_text(outcome.get("measure")),
                    "time_frame": clean_text(outcome.get("timeFrame")),
                    "description": clean_text(outcome.get("description")),
                }
            )
    for idx, outcome in enumerate(result_outcomes):
        rows.append(
            {
                "source": "results",
                "outcome_type": clean_text(outcome.get("type")) or "results",
                "ordinal": idx,
                "measure": clean_text(outcome.get("title") or outcome.get("measure")),
                "time_frame": clean_text(outcome.get("timeFrame")),
                "description": clean_text(outcome.get("description")),
                "analysis_count": len(outcome.get("analyses") or []),
                "class_count": len(outcome.get("classes") or []),
            }
        )
    return rows


def study_summary(study: dict[str, Any], left_norm: str, right_norm: str, left_raw: str, right_raw: str) -> dict[str, Any] | None:
    protocol = study.get("protocolSection") or {}
    identification = protocol.get("identificationModule") or {}
    status = protocol.get("statusModule") or {}
    design = protocol.get("designModule") or {}
    conditions = protocol.get("conditionsModule") or {}
    arms = protocol.get("armsInterventionsModule") or {}
    interventions = arms.get("interventions") or []
    intervention_strings: list[str] = []
    drug_interventions: list[dict[str, Any]] = []
    for intervention in interventions:
        if intervention.get("type") and intervention.get("type") != "DRUG":
            continue
        drug_interventions.append(intervention)
        intervention_strings.extend(flatten_strings(intervention))
    intervention_text = " ".join(intervention_strings)
    left_in_interventions = contains_norm(intervention_text, left_norm)
    right_in_interventions = contains_norm(intervention_text, right_norm)
    if not (left_in_interventions and right_in_interventions):
        return None
    lower_interventions = intervention_text.lower()
    exact = left_raw.lower() in lower_interventions and right_raw.lower() in lower_interventions
    outcomes = outcome_rows(study)
    result_outcome_count = sum(1 for row in outcomes if row["source"] == "results")
    outcome_text = " ".join(flatten_strings(outcomes))
    evidence_text = " ".join(
        [
            intervention_text,
            clean_text(identification.get("briefTitle")),
            clean_text(identification.get("officialTitle")),
            " ".join(flatten_strings(conditions.get("conditions"))),
            outcome_text,
        ]
    )
    return {
        "nct_id": identification.get("nctId"),
        "brief_title": identification.get("briefTitle"),
        "official_title": identification.get("officialTitle"),
        "overall_status": status.get("overallStatus"),
        "start_date": status.get("startDateStruct", {}).get("date"),
        "completion_date": status.get("completionDateStruct", {}).get("date"),
        "study_type": design.get("studyType"),
        "phases": design.get("phases") or [],
        "enrollment_count": (design.get("enrollmentInfo") or {}).get("count"),
        "enrollment_type": (design.get("enrollmentInfo") or {}).get("type"),
        "conditions": conditions.get("conditions") or [],
        "intervention_names": [
            clean_text(intervention.get("name"))
            for intervention in drug_interventions
        ],
        "intervention_match_status": "exact_text_match" if exact else "normalized_text_match",
        "intervention_text_sha256": sha256_bytes(intervention_text.encode("utf-8")),
        "endpoint_outcome_count": len(outcomes),
        "primary_outcome_count": sum(1 for row in outcomes if row["outcome_type"] == "primary"),
        "secondary_outcome_count": sum(1 for row in outcomes if row["outcome_type"] == "secondary"),
        "result_outcome_count": result_outcome_count,
        "has_protocol_endpoint": any(row["source"] == "protocol" for row in outcomes),
        "has_results_endpoint": result_outcome_count > 0,
        "outcomes": outcomes[:30],
        "outcome_text_sha256": sha256_bytes(outcome_text.encode("utf-8")),
        "evidence_text_sha256": sha256_bytes(evidence_text.encode("utf-8")),
    }


def evidence_for_response(raw: dict[str, Any]) -> tuple[str, list[dict[str, Any]]]:
    if not raw.get("queryable"):
        return "clinicaltrials_registry_not_queryable_still_blocked", []
    left_norm = norm_text(raw["drug_a"])
    right_norm = norm_text(raw["drug_b"])
    studies = raw.get("response", {}).get("studies", [])
    matched: list[dict[str, Any]] = []
    exact = False
    has_endpoint = False
    for study in studies:
        summary = study_summary(study, left_norm, right_norm, clean_text(raw["drug_a"]), clean_text(raw["drug_b"]))
        if not summary:
            continue
        matched.append(summary)
        exact = exact or summary["intervention_match_status"] == "exact_text_match"
        has_endpoint = has_endpoint or summary["endpoint_outcome_count"] > 0
    if matched and has_endpoint and exact:
        return "clinicaltrials_registry_endpoint_exact_hit_still_blocked", matched
    if matched and has_endpoint:
        return "clinicaltrials_registry_endpoint_normalized_hit_still_blocked", matched
    if matched:
        return "clinicaltrials_registry_intervention_hit_no_endpoint_still_blocked", matched
    return "clinicaltrials_registry_no_hit_still_blocked", []


def reason_codes(status: str, matched: list[dict[str, Any]]) -> list[str]:
    codes = [
        "clinicaltrials_registry_endpoint_validation_not_clinical_actionability",
        "requires_result_effect_direction_safety_falsification_and_human_review",
    ]
    if status == "clinicaltrials_registry_endpoint_exact_hit_still_blocked":
        codes.append("both_terms_exact_in_registry_intervention_with_endpoint_fields")
    elif status == "clinicaltrials_registry_endpoint_normalized_hit_still_blocked":
        codes.append("both_terms_normalized_in_registry_intervention_with_endpoint_fields")
    elif status == "clinicaltrials_registry_intervention_hit_no_endpoint_still_blocked":
        codes.append("both_terms_in_registry_intervention_but_no_endpoint_fields")
    elif status == "clinicaltrials_registry_not_queryable_still_blocked":
        codes.append("pair_not_queryable_fail_closed")
    else:
        codes.append("no_same_registry_intervention_pair_hit")
    if any(study.get("has_results_endpoint") for study in matched):
        codes.append("results_endpoint_module_present_but_not_effect_claim")
    return codes


def pair_status_rows(
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    by_key = {row["pair_key"]: row for row in query_rows}
    pair_status: list[dict[str, Any]] = []
    study_evidence: list[dict[str, Any]] = []
    for pair in pairs:
        raw = by_key[pair["pair_key"]]
        status, matched = evidence_for_response(raw)
        endpoint_studies = [study for study in matched if study["endpoint_outcome_count"] > 0]
        pair_evidence_id = "clinicaltrials-endpoint-pair:" + stable_id(pair["pair_key"], raw.get("response_sha256"))
        for study in matched:
            evidence_id = "clinicaltrials-endpoint-study:" + stable_id(pair["pair_key"], study.get("nct_id"), study["endpoint_outcome_count"])
            study_evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": evidence_id,
                    "pair_key": pair["pair_key"],
                    "representative_pair_id": pair["representative_pair_id"],
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source": "ClinicalTrials.gov",
                    "source_url": f"https://clinicaltrials.gov/study/{study.get('nct_id')}",
                    "nct_id": study.get("nct_id"),
                    "clinicaltrials_endpoint_status": status,
                    "study": study,
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": reason_codes(status, [study]),
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
        pair_status.append(
            {
                "schema_version": 1,
                "evidence_id": pair_evidence_id,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "clinicaltrials_endpoint_status": status,
                "queryable": pair["queryable"],
                "query_text": pair["query_text"],
                "total_count": raw.get("total_count"),
                "returned_studies": raw.get("returned_studies"),
                "page_count": raw.get("page_count"),
                "matched_study_count": len(matched),
                "endpoint_study_count": len(endpoint_studies),
                "result_endpoint_study_count": sum(1 for study in endpoint_studies if study.get("has_results_endpoint")),
                "matched_nct_ids": [study.get("nct_id") for study in matched[:30]],
                "endpoint_nct_ids": [study.get("nct_id") for study in endpoint_studies[:30]],
                "matched_studies": matched[:20],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes(status, matched),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": (
                    "Extract and independently validate result effect direction/magnitude, comparator, cohort, "
                    "safety/falsification, and human review before any promotion."
                    if endpoint_studies
                    else "Continue independent endpoint/source search; no registry endpoint promotion."
                ),
            }
        )
    pair_status.sort(
        key=lambda row: (
            0 if "endpoint_" in row["clinicaltrials_endpoint_status"] and "no_endpoint" not in row["clinicaltrials_endpoint_status"] else 1,
            -row["endpoint_study_count"],
            row["pair_key"],
        )
    )
    study_evidence.sort(key=lambda row: (row["pair_key"], row.get("nct_id") or ""))
    return pair_status, study_evidence


def rollup_status_rows(
    rollups: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    by_key = {row["pair_key"]: row for row in pair_status}
    rows: list[dict[str, Any]] = []
    for rollup in rollups:
        status = by_key[rollup["pair_key"]]
        rows.append(
            {
                "schema_version": 1,
                "rollup_validation_id": "clinicaltrials-endpoint-rollup:" + stable_id(rollup["rollup_review_id"], status["clinicaltrials_endpoint_status"]),
                "source_issue1247_rollup_review_id": rollup["rollup_review_id"],
                "pair_id": rollup["pair_id"],
                "pair_key": rollup["pair_key"],
                "drug_a": rollup["drug_a"],
                "drug_b": rollup["drug_b"],
                "issue1247_endpoint_outcome_review_category": rollup["endpoint_outcome_review_category"],
                "issue1247_has_endpoint_or_outcome_language": rollup["has_endpoint_or_outcome_language"],
                "issue1247_has_effect_size_language": rollup["has_effect_size_language"],
                "clinicaltrials_endpoint_status": status["clinicaltrials_endpoint_status"],
                "clinicaltrials_pair_evidence_id": status["evidence_id"],
                "matched_study_count": status["matched_study_count"],
                "endpoint_study_count": status["endpoint_study_count"],
                "result_endpoint_study_count": status["result_endpoint_study_count"],
                "matched_nct_ids": status["matched_nct_ids"][:30],
                "endpoint_nct_ids": status["endpoint_nct_ids"][:30],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": status["reason_codes"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": status["next_validation_experiment"],
            }
        )
    rows.sort(
        key=lambda row: (
            0 if "endpoint_" in row["clinicaltrials_endpoint_status"] and "no_endpoint" not in row["clinicaltrials_endpoint_status"] else 1,
            -row["endpoint_study_count"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rows


def build_bridge_rows(
    rollup_status: list[dict[str, Any]],
    study_evidence: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        ncts = ", ".join(row["endpoint_nct_ids"][:3]) or "none"
        text = (
            f"ClinicalTrials endpoint validation rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['clinicaltrials_endpoint_status']} "
            f"endpoint studies {row['endpoint_study_count']} NCT {ncts} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["rollup_validation_id"],
                "domain": "clinicaltrials_endpoint_rollup_validation",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["clinicaltrials_endpoint_status"], *row["endpoint_nct_ids"][:3]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1250_clinicaltrials_endpoint_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "clinicaltrials_endpoint_status": row["clinicaltrials_endpoint_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in study_evidence[:budget]:
        study = row["study"]
        text = (
            f"ClinicalTrials endpoint evidence {row['evidence_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} NCT {row['nct_id']} "
            f"endpoint count {study['endpoint_outcome_count']} status {row['clinicaltrials_endpoint_status']}."
        )
        rows.append(
            {
                "id": row["evidence_id"],
                "domain": "clinicaltrials_endpoint_study_evidence",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["nct_id"], row["clinicaltrials_endpoint_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1250_clinicaltrials_endpoint_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "nct_id": row["nct_id"],
                    "clinicaltrials_endpoint_status": row["clinicaltrials_endpoint_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    study_evidence: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1247_rollup_rows": len(rollups),
        "unique_pair_keys": len(pairs),
        "queryable_pair_keys": sum(1 for row in pairs if row["queryable"]),
        "clinicaltrials_query_response_rows": len(query_rows),
        "pair_status_rows": len(pair_status),
        "rollup_status_rows": len(rollup_status),
        "study_evidence_rows": len(study_evidence),
        "unique_matched_nct_ids": len({row.get("nct_id") for row in study_evidence if row.get("nct_id")}),
        "endpoint_study_evidence_rows": sum(1 for row in study_evidence if row["study"]["endpoint_outcome_count"] > 0),
        "result_endpoint_study_evidence_rows": sum(1 for row in study_evidence if row["study"]["has_results_endpoint"]),
        "total_api_pages_read": sum(int(row.get("page_count") or 0) for row in query_rows),
        "max_api_pages_for_a_pair": max((int(row.get("page_count") or 0) for row in query_rows), default=0),
        "responses_with_any_returned_study": sum(1 for row in query_rows if row.get("returned_studies", 0) > 0),
        "responses_with_next_page_token": sum(1 for row in query_rows if row.get("next_page_token_present")),
        "pair_status_counts": dict(sorted(Counter(row["clinicaltrials_endpoint_status"] for row in pair_status).items())),
        "rollup_status_counts": dict(sorted(Counter(row["clinicaltrials_endpoint_status"] for row in rollup_status).items())),
        "issue1247_category_counts": dict(sorted(Counter(row["issue1247_endpoint_outcome_review_category"] for row in rollup_status).items())),
        "top_hits": [
            {
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["clinicaltrials_endpoint_status"],
                "matched_study_count": row["matched_study_count"],
                "endpoint_study_count": row["endpoint_study_count"],
                "result_endpoint_study_count": row["result_endpoint_study_count"],
                "nct_ids": row["endpoint_nct_ids"][:5],
            }
            for row in pair_status
            if row["endpoint_study_count"] > 0
        ][:50],
    }


def build_input_manifest(
    inputs: dict[str, str],
    raw_artifacts: dict[str, dict[str, Any]],
    rollups: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    issue1247_persisted_readback: dict[str, Any],
    issue1247_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1250,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1247_rollup": artifact(Path(inputs["issue1247_rollup"]), jsonl=True),
            "issue1247_evidence_review": artifact(Path(inputs["issue1247_evidence_review"]), jsonl=True),
            "issue1247_persisted_readback": artifact(Path(inputs["issue1247_persisted_readback"])),
            "issue1247_calyx_readback": artifact(Path(inputs["issue1247_calyx_readback"])),
            "issue1247_output_manifest": artifact(Path(inputs["issue1247_output_manifest"])),
            **raw_artifacts,
        },
        "source_contract": {
            "issue1247_persisted_assertions_all_true": all_assertions_true(issue1247_persisted_readback),
            "issue1247_calyx_assertions_all_true": all_assertions_true(issue1247_calyx_readback),
            "issue1247_rollup_rows": len(rollups),
            "issue1247_evidence_review_rows": len(evidence_rows),
            "unique_pair_keys": len(pairs),
            "query_parameter": "query.intr",
            "hit_requires_both_candidate_terms_in_same_drug_intervention_text": True,
            "endpoint_hit_requires_protocol_or_results_outcome_fields": True,
        },
        "accepted_sources": [
            {
                "source": "ClinicalTrials.gov v2 API",
                "api_base": API_BASE,
                "api_spec_url": API_SPEC_URL,
                "api_doc_url": API_DOC_URL,
                "api_about_url": API_ABOUT_URL,
                "role": "independent registry endpoint documentation",
            }
        ],
    }


def build_readback(
    out_dir: Path,
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    study_evidence: list[dict[str, Any]],
    issue1247_persisted_readback: dict[str, Any],
    issue1247_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "clinicaltrials_endpoint_query_responses": artifact(out_dir / "clinicaltrials_endpoint_query_responses.jsonl", jsonl=True),
        "clinicaltrials_endpoint_pair_status": artifact(out_dir / "clinicaltrials_endpoint_pair_status.jsonl", jsonl=True),
        "clinicaltrials_endpoint_rollup_status": artifact(out_dir / "clinicaltrials_endpoint_rollup_status.jsonl", jsonl=True),
        "clinicaltrials_endpoint_study_evidence": artifact(out_dir / "clinicaltrials_endpoint_study_evidence.jsonl", jsonl=True),
        "clinicaltrials_endpoint_bridge_rows": artifact(out_dir / "clinicaltrials_endpoint_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    queryable_keys = {row["pair_key"] for row in pairs if row["queryable"]}
    query_keys = {row["pair_key"] for row in query_rows if row["queryable"]}
    pair_keys = {row["pair_key"] for row in pairs}
    status_keys = {row["pair_key"] for row in pair_status}
    rollup_ids = {row["rollup_review_id"] for row in rollups}
    status_rollup_ids = {row["source_issue1247_rollup_review_id"] for row in rollup_status}
    endpoint_hit_keys = {
        row["pair_key"]
        for row in pair_status
        if row["clinicaltrials_endpoint_status"] in {
            "clinicaltrials_registry_endpoint_exact_hit_still_blocked",
            "clinicaltrials_registry_endpoint_normalized_hit_still_blocked",
        }
    }
    endpoint_evidence_keys = {row["pair_key"] for row in study_evidence if row["study"]["endpoint_outcome_count"] > 0}
    return {
        "schema_version": 1,
        "issue": 1250,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1247_persisted_readback_all_true": all_assertions_true(issue1247_persisted_readback),
            "issue1247_calyx_readback_all_true": all_assertions_true(issue1247_calyx_readback),
            "query_response_for_every_queryable_pair_key": query_keys == queryable_keys,
            "pair_status_for_every_pair_key": status_keys == pair_keys,
            "rollup_status_for_every_issue1247_rollup": status_rollup_ids == rollup_ids,
            "all_pair_status_values_allowed": all(row["clinicaltrials_endpoint_status"] in STATUS_VALUES for row in pair_status),
            "all_rollup_status_values_allowed": all(row["clinicaltrials_endpoint_status"] in STATUS_VALUES for row in rollup_status),
            "endpoint_hits_have_endpoint_study_evidence": endpoint_hit_keys <= endpoint_evidence_keys,
            "all_study_rows_have_nct_ids": all(bool(row.get("nct_id")) for row in study_evidence),
            "all_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in pair_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in study_evidence),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in pair_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in study_evidence),
            "bridge_rows_1000_or_less": artifacts["clinicaltrials_endpoint_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "issue1247_rollups": len(rollups),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "pair_status_rows": len(pair_status),
            "rollup_status_rows": len(rollup_status),
            "study_evidence_rows": len(study_evidence),
        },
    }


def run(
    root: Path,
    inputs: dict[str, str],
    *,
    max_pairs: int | None,
    request_sleep_seconds: float,
) -> dict[str, Any]:
    require_inputs(inputs)
    raw_dir = root / "raw"
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_artifacts = fetch_raw_sources(raw_dir)
    rollups = rows_jsonl(Path(inputs["issue1247_rollup"]))
    evidence_rows = rows_jsonl(Path(inputs["issue1247_evidence_review"]))
    pairs = representative_pairs(rollups)
    if max_pairs is not None:
        keep = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in keep]
        rollups = [row for row in rollups if row["pair_key"] in keep]
    issue1247_persisted_readback = read_json(Path(inputs["issue1247_persisted_readback"]))
    issue1247_calyx_readback = read_json(Path(inputs["issue1247_calyx_readback"]))
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            raw_artifacts,
            rollups,
            evidence_rows,
            pairs,
            issue1247_persisted_readback,
            issue1247_calyx_readback,
        ),
    )
    query_rows = fetch_query_rows(
        pairs,
        out_dir / "clinicaltrials_endpoint_query_responses.jsonl",
        request_sleep_seconds,
    )
    pair_status, study_evidence = pair_status_rows(pairs, query_rows)
    write_jsonl(out_dir / "clinicaltrials_endpoint_pair_status.jsonl", pair_status)
    write_jsonl(out_dir / "clinicaltrials_endpoint_study_evidence.jsonl", study_evidence)
    rollup_status = rollup_status_rows(rollups, pair_status)
    write_jsonl(out_dir / "clinicaltrials_endpoint_rollup_status.jsonl", rollup_status)
    source_path = out_dir / "clinicaltrials_endpoint_rollup_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, study_evidence, source_path, source_sha)
    write_jsonl(out_dir / "clinicaltrials_endpoint_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(rollups, pairs, query_rows, pair_status, rollup_status, study_evidence)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1250,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "clinicaltrials_endpoint_query_responses": artifact(out_dir / "clinicaltrials_endpoint_query_responses.jsonl", jsonl=True),
            "clinicaltrials_endpoint_pair_status": artifact(out_dir / "clinicaltrials_endpoint_pair_status.jsonl", jsonl=True),
            "clinicaltrials_endpoint_rollup_status": artifact(out_dir / "clinicaltrials_endpoint_rollup_status.jsonl", jsonl=True),
            "clinicaltrials_endpoint_study_evidence": artifact(out_dir / "clinicaltrials_endpoint_study_evidence.jsonl", jsonl=True),
            "clinicaltrials_endpoint_bridge_rows": artifact(out_dir / "clinicaltrials_endpoint_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        rollups,
        pairs,
        query_rows,
        pair_status,
        rollup_status,
        study_evidence,
        issue1247_persisted_readback,
        issue1247_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "rollup_status": output_manifest["artifacts"]["clinicaltrials_endpoint_rollup_status"],
            "pair_status": output_manifest["artifacts"]["clinicaltrials_endpoint_pair_status"],
            "study_evidence": output_manifest["artifacts"]["clinicaltrials_endpoint_study_evidence"],
            "bridge_rows": output_manifest["artifacts"]["clinicaltrials_endpoint_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1247-rollup")
    parser.add_argument("--issue1247-evidence-review")
    parser.add_argument("--issue1247-persisted-readback")
    parser.add_argument("--issue1247-calyx-readback")
    parser.add_argument("--issue1247-output-manifest")
    parser.add_argument("--max-pairs", type=int)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1247_rollup", "issue1247_rollup"),
        ("issue1247_evidence_review", "issue1247_evidence_review"),
        ("issue1247_persisted_readback", "issue1247_persisted_readback"),
        ("issue1247_calyx_readback", "issue1247_calyx_readback"),
        ("issue1247_output_manifest", "issue1247_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(
        Path(args.root),
        inputs,
        max_pairs=args.max_pairs,
        request_sleep_seconds=args.request_sleep_seconds,
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
