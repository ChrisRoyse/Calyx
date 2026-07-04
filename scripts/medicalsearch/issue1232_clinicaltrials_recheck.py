#!/usr/bin/env python3
"""#1232 current ClinicalTrials.gov recheck for remaining combo no-hit rows.

This is a direct dated registry pass over the #1231 remaining no-hit set. It
stores one raw ClinicalTrials.gov v2 API response per pair, then derives
deterministic source-attributed combination evidence from the persisted
response bytes. Registry co-occurrence is not efficacy, safety, dosing,
treatment guidance, recommendation, clinical actionability, or cure evidence.
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
from collections import Counter
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "ClinicalTrials.gov registry evidence is source-attributed combination "
    "documentation only; not efficacy, safety, clinical actionability, "
    "treatment guidance, dosing, recommendation, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "current_clinicaltrials_registry_combination_documentation_not_outcome_or_safety_clearance"
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z"

DEFAULT_INPUTS = {
    "clinicaltrials_oas": f"{DEFAULT_ROOT}/raw/clinicaltrials_oas_v2.yaml",
    "issue1231_prior_no_hit_recheck": "/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/prior_no_hit_recheck_status.jsonl",
}

API_BASE = "https://clinicaltrials.gov/api/v2/studies"
API_SPEC_URL = "https://clinicaltrials.gov/api/oas/v2"
API_DOC_URL = "https://clinicaltrials.gov/data-api/api"
API_ABOUT_URL = "https://clinicaltrials.gov/data-api/about-api"
PAGE_SIZE = 100
REQUEST_SLEEP_SECONDS = 0.05


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


def norm_text(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def contains_norm(text: str, needle: str) -> bool:
    if not needle:
        return False
    haystack = f" {norm_text(text)} "
    return f" {needle} " in haystack


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


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True) + "\n")


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
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


def remaining_no_hit_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = [
        row
        for row in rows
        if row.get("prior_issue1229_external_synergy_status") == "no_external_hit"
        and row.get("cdcdb_external_combo_status") == "no_external_hit"
    ]
    out.sort(key=lambda row: (row.get("disease_area") or "", row["pair_key"], row["pair_id"]))
    return out


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
    return " ".join(text.split())


def query_text(row: dict[str, Any]) -> str:
    return " ".join(part for part in [query_name(row["drug_a"]), query_name(row["drug_b"])] if part)


def query_url(row: dict[str, Any]) -> str:
    return query_url_for_text(query_text(row))


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


def fetch_json(url: str, retries: int = 4) -> tuple[dict[str, Any], bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "calyx-discovery/issue1232"})
            with urllib.request.urlopen(request, timeout=45) as response:
                payload = response.read()
            return json.loads(payload), payload
        except urllib.error.HTTPError as error:
            payload = error.read()
            if error.code == 400:
                body = payload.decode("utf-8", errors="replace")
                return {
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


def fetch_all_pages(row: dict[str, Any]) -> tuple[dict[str, Any], bytes, list[str], list[str], list[int]]:
    text = query_text(row)
    url = query_url_for_text(text)
    pages: list[dict[str, Any]] = []
    page_urls: list[str] = []
    page_hashes: list[str] = []
    page_bytes: list[int] = []
    total_count: int | None = None
    while True:
        data, payload = fetch_json(url)
        pages.append(data)
        page_urls.append(url)
        page_hashes.append(sha256_bytes(payload))
        page_bytes.append(len(payload))
        if total_count is None:
            total_count = data.get("totalCount")
        token = data.get("nextPageToken")
        if not token:
            break
        url = query_url_for_text(text, page_token=token)
        time.sleep(REQUEST_SLEEP_SECONDS)
    merged = {
        "totalCount": total_count,
        "studies": [study for page in pages for study in page.get("studies", [])],
        "pages": pages,
    }
    errors = [page.get("api_error") for page in pages if page.get("api_error")]
    if errors:
        merged["api_errors"] = errors
    payload = json.dumps(merged, sort_keys=True).encode("utf-8")
    return merged, payload, page_urls, page_hashes, page_bytes


def fetch_raw_responses(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    raw: list[dict[str, Any]] = []
    for index, row in enumerate(rows, start=1):
        data, payload, page_urls, page_hashes, page_bytes = fetch_all_pages(row)
        response_sha = sha256_bytes(payload)
        raw.append(
            {
                "schema_version": 1,
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "query_text": query_text(row),
                "query_url": page_urls[0],
                "page_urls": page_urls,
                "page_response_sha256s": page_hashes,
                "page_response_bytes": page_bytes,
                "page_count": len(page_urls),
                "api_base": API_BASE,
                "page_size": PAGE_SIZE,
                "api_errors": data.get("api_errors") or [],
                "total_count": data.get("totalCount"),
                "returned_studies": len(data.get("studies", [])),
                "next_page_token_present": any(bool(page.get("nextPageToken")) for page in data.get("pages", [])),
                "response_bytes": len(payload),
                "response_sha256": response_sha,
                "response": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        if index % 100 == 0:
            print(f"fetched {index}/{len(rows)} ClinicalTrials.gov pair responses", file=sys.stderr)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return raw


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


def study_summary(study: dict[str, Any], left_norm: str, right_norm: str, left_raw: str, right_raw: str) -> dict[str, Any] | None:
    protocol = study.get("protocolSection") or {}
    identification = protocol.get("identificationModule") or {}
    status = protocol.get("statusModule") or {}
    design = protocol.get("designModule") or {}
    conditions = protocol.get("conditionsModule") or {}
    arms = protocol.get("armsInterventionsModule") or {}
    interventions = arms.get("interventions") or []
    intervention_strings: list[str] = []
    for intervention in interventions:
        if intervention.get("type") and intervention.get("type") != "DRUG":
            continue
        intervention_strings.extend(flatten_strings(intervention))
    intervention_text = " ".join(intervention_strings)
    full_text = " ".join(
        [
            intervention_text,
            clean_text(identification.get("briefTitle")),
            clean_text(identification.get("officialTitle")),
            " ".join(flatten_strings(conditions.get("conditions"))),
            " ".join(flatten_strings(arms.get("armGroups"))),
        ]
    )
    left_in_interventions = contains_norm(intervention_text, left_norm)
    right_in_interventions = contains_norm(intervention_text, right_norm)
    if not (left_in_interventions and right_in_interventions):
        return None
    lower_interventions = intervention_text.lower()
    exact = left_raw.lower() in lower_interventions and right_raw.lower() in lower_interventions
    return {
        "nct_id": identification.get("nctId"),
        "brief_title": identification.get("briefTitle"),
        "overall_status": status.get("overallStatus"),
        "phases": design.get("phases") or [],
        "conditions": conditions.get("conditions") or [],
        "intervention_names": [
            clean_text(intervention.get("name"))
            for intervention in interventions
            if not intervention.get("type") or intervention.get("type") == "DRUG"
        ],
        "intervention_match_status": "exact_text_match" if exact else "normalized_text_match",
        "intervention_text_sha256": sha256_bytes(intervention_text.encode("utf-8")),
        "full_text_sha256": sha256_bytes(full_text.encode("utf-8")),
    }


def evidence_for_response(raw: dict[str, Any]) -> tuple[str, list[dict[str, Any]]]:
    left_norm = norm_text(raw["drug_a"])
    right_norm = norm_text(raw["drug_b"])
    studies = raw.get("response", {}).get("studies", [])
    matched: list[dict[str, Any]] = []
    exact = False
    for study in studies:
        summary = study_summary(study, left_norm, right_norm, clean_text(raw["drug_a"]), clean_text(raw["drug_b"]))
        if not summary:
            continue
        matched.append(summary)
        exact = exact or summary["intervention_match_status"] == "exact_text_match"
    if exact:
        return "exact_hit", matched
    if matched:
        return "normalized_hit", matched
    return "no_external_hit", []


def joined_status_rows(raw_rows: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    joined: list[dict[str, Any]] = []
    study_rows: list[dict[str, Any]] = []
    for raw in raw_rows:
        status, matched = evidence_for_response(raw)
        evidence_id = f"clinicaltrials-pair:{stable_id(raw['pair_id'], raw['response_sha256'])}"
        source_types = ["clinicaltrials.gov"] if matched else []
        reason_codes = ["external_clinicaltrials_current_hit_not_clearance"] if matched else [
            "external_clinicaltrials_current_missing_fail_closed"
        ]
        for study in matched:
            study_rows.append(
                {
                    "schema_version": 1,
                    "evidence_id": f"clinicaltrials-study:{stable_id(raw['pair_id'], study.get('nct_id'))}",
                    "pair_id": raw["pair_id"],
                    "pair_key": raw["pair_key"],
                    "drug_a": raw["drug_a"],
                    "drug_b": raw["drug_b"],
                    "status": status,
                    "source": "ClinicalTrials.gov",
                    "source_url": f"https://clinicaltrials.gov/study/{study.get('nct_id')}",
                    "study": study,
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
        joined.append(
            {
                "schema_version": 1,
                "evidence_id": evidence_id,
                "pair_id": raw["pair_id"],
                "pair_key": raw["pair_key"],
                "drug_a": raw["drug_a"],
                "drug_b": raw["drug_b"],
                "clinicaltrials_current_status": status,
                "clinicaltrials_match": status != "no_external_hit",
                "matched_study_count": len(matched),
                "total_count": raw.get("total_count"),
                "returned_studies": raw.get("returned_studies"),
                "next_page_token_present": raw.get("next_page_token_present"),
                "matched_studies": matched[:20],
                "source_types": source_types,
                "combination_status": (
                    "external_clinicaltrials_current_documented_still_blocked"
                    if matched
                    else "blocked_no_external_combination_evidence_after_current_clinicaltrials"
                ),
                "reason_codes": reason_codes,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": (
                    "Inspect registry source, then require independent synergy, safety, and outcome gates."
                    if matched
                    else "Acquire additional open external combination evidence before promotion."
                ),
            }
        )
    joined.sort(
        key=lambda row: (
            0 if row["clinicaltrials_current_status"] != "no_external_hit" else 1,
            -row["matched_study_count"],
            row["pair_key"],
        )
    )
    study_rows.sort(key=lambda row: (row["pair_key"], row["study"].get("nct_id") or ""))
    return joined, study_rows


def build_metrics(raw_rows: list[dict[str, Any]], joined: list[dict[str, Any]], study_rows: list[dict[str, Any]]) -> dict[str, Any]:
    status_counts = Counter(row["clinicaltrials_current_status"] for row in joined)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "queried_remaining_no_hit_rows": len(raw_rows),
        "raw_api_response_rows": len(raw_rows),
        "clinicaltrials_study_evidence_rows": len(study_rows),
        "unique_matched_nct_ids": len({row["study"].get("nct_id") for row in study_rows}),
        "candidate_rows_with_current_clinicaltrials_hit": sum(
            1 for row in joined if row["clinicaltrials_current_status"] != "no_external_hit"
        ),
        "remaining_no_hit_after_current_clinicaltrials": sum(
            1 for row in joined if row["clinicaltrials_current_status"] == "no_external_hit"
        ),
        "status_counts": dict(status_counts),
        "responses_with_next_page_token": sum(1 for row in raw_rows if row.get("next_page_token_present")),
        "total_api_pages_read": sum(int(row.get("page_count") or 0) for row in raw_rows),
        "max_api_pages_for_a_pair": max((int(row.get("page_count") or 0) for row in raw_rows), default=0),
        "responses_with_any_returned_study": sum(1 for row in raw_rows if row.get("returned_studies", 0) > 0),
        "clinical_boundary_rows": sum(1 for row in joined if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_hits": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["clinicaltrials_current_status"],
                "matched_study_count": row["matched_study_count"],
                "total_count": row.get("total_count"),
                "nct_ids": [study.get("nct_id") for study in row["matched_studies"][:5]],
            }
            for row in joined
            if row["clinicaltrials_current_status"] != "no_external_hit"
        ][:25],
    }


def build_bridge_rows(rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows[:1000]:
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row["clinicaltrials_current_status"],
                row["combination_status"],
                *[study.get("nct_id") for study in row.get("matched_studies", [])[:3]],
            ]
        )
        nct_ids = ", ".join(study.get("nct_id") or "" for study in row.get("matched_studies", [])[:3]) or "none"
        text = (
            f"ClinicalTrials.gov current recheck {row['pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} has status {row['clinicaltrials_current_status']} with "
            f"{row['matched_study_count']} matched registry studies ({nct_ids}) and remains "
            f"{row['combination_status']}."
        )
        out.append(
            {
                "id": row["pair_id"],
                "domain": "external_drug_combination_current_clinicaltrials_recheck",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1232_clinicaltrials_current_recheck",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "clinicaltrials_current_status": row["clinicaltrials_current_status"],
                    "combination_status": row["combination_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def build_input_manifest(inputs: dict[str, str], rows: list[dict[str, Any]]) -> dict[str, Any]:
    oas_path = Path(inputs["clinicaltrials_oas"])
    return {
        "schema_version": 1,
        "issue": 1232,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "clinicaltrials_oas": {
                **artifact(oas_path),
                "api_spec_url": API_SPEC_URL,
                "api_doc_url": API_DOC_URL,
                "api_about_url": API_ABOUT_URL,
            },
            "issue1231_prior_no_hit_recheck": artifact(Path(inputs["issue1231_prior_no_hit_recheck"]), jsonl=True),
        },
        "accepted_sources": [
            {
                "source": "ClinicalTrials.gov v2 API current direct recheck",
                "role": "current source-attributed registry combination documentation",
                "api_base": API_BASE,
                "api_spec_url": API_SPEC_URL,
                "api_doc_url": API_DOC_URL,
                "api_about_url": API_ABOUT_URL,
                "query_parameter": "query.intr",
                "page_size": PAGE_SIZE,
            }
        ],
        "query_universe": {
            "remaining_no_hit_rows": len(rows),
            "source": "issue1231 prior_no_hit_recheck_status rows with CDCDB no_external_hit",
        },
    }


def build_readback(
    out_dir: Path,
    raw_rows: list[dict[str, Any]],
    joined: list[dict[str, Any]],
    study_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "clinicaltrials_raw_responses": artifact(out_dir / "clinicaltrials_raw_responses.jsonl", jsonl=True),
        "clinicaltrials_pair_status": artifact(out_dir / "clinicaltrials_pair_status.jsonl", jsonl=True),
        "clinicaltrials_pair_hits": artifact(out_dir / "clinicaltrials_pair_hits.jsonl", jsonl=True),
        "clinicaltrials_study_evidence": artifact(out_dir / "clinicaltrials_study_evidence.jsonl", jsonl=True),
        "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    return {
        "schema_version": 1,
        "issue": 1232,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "raw_response_for_every_remaining_no_hit": len(raw_rows) == len(joined),
            "deterministic_status_for_every_row": all(
                row["clinicaltrials_current_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                for row in joined
            ),
            "all_joined_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in joined),
            "all_hits_have_matched_studies": all(
                row["matched_study_count"] > 0
                for row in joined
                if row["clinicaltrials_current_status"] != "no_external_hit"
            ),
            "all_study_rows_have_nct_ids": all(bool(row["study"].get("nct_id")) for row in study_rows),
            "bridge_rows_1000_or_less": artifacts["external_combo_bridge_rows"]["rows"] == min(1000, len(joined)),
        },
        "row_counts": {
            "raw_rows": len(raw_rows),
            "joined_rows": len(joined),
            "study_rows": len(study_rows),
        },
    }


def run(root: Path, inputs: dict[str, str], max_rows: int | None = None) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    prior_rows = rows_jsonl(Path(inputs["issue1231_prior_no_hit_recheck"]))
    remaining = remaining_no_hit_rows(prior_rows)
    if max_rows is not None:
        remaining = remaining[:max_rows]
    write_json(out_dir / "input_manifest.json", build_input_manifest(inputs, remaining))
    raw_rows = fetch_raw_responses(remaining)
    write_jsonl(out_dir / "clinicaltrials_raw_responses.jsonl", raw_rows)
    joined, study_rows = joined_status_rows(raw_rows)
    write_jsonl(out_dir / "clinicaltrials_pair_status.jsonl", joined)
    hits = [row for row in joined if row["clinicaltrials_current_status"] != "no_external_hit"]
    write_jsonl(out_dir / "clinicaltrials_pair_hits.jsonl", hits)
    write_jsonl(out_dir / "clinicaltrials_study_evidence.jsonl", study_rows)
    source_path = out_dir / "clinicaltrials_pair_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(joined, source_path, source_sha)
    write_jsonl(out_dir / "external_combo_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(raw_rows, joined, study_rows)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1232,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "clinicaltrials_raw_responses": artifact(out_dir / "clinicaltrials_raw_responses.jsonl", jsonl=True),
            "clinicaltrials_pair_status": artifact(out_dir / "clinicaltrials_pair_status.jsonl", jsonl=True),
            "clinicaltrials_pair_hits": artifact(out_dir / "clinicaltrials_pair_hits.jsonl", jsonl=True),
            "clinicaltrials_study_evidence": artifact(out_dir / "clinicaltrials_study_evidence.jsonl", jsonl=True),
            "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, raw_rows, joined, study_rows)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "pair_status": output_manifest["artifacts"]["clinicaltrials_pair_status"],
            "hits": output_manifest["artifacts"]["clinicaltrials_pair_hits"],
            "study_evidence": output_manifest["artifacts"]["clinicaltrials_study_evidence"],
            "bridge_rows": output_manifest["artifacts"]["external_combo_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def inputs_for_root(root: Path) -> dict[str, str]:
    inputs = dict(DEFAULT_INPUTS)
    inputs["clinicaltrials_oas"] = str(root / "raw" / "clinicaltrials_oas_v2.yaml")
    return inputs


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--clinicaltrials-oas")
    parser.add_argument("--issue1231-prior-no-hit-recheck")
    parser.add_argument("--max-rows", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    inputs = inputs_for_root(root)
    if args.clinicaltrials_oas:
        inputs["clinicaltrials_oas"] = args.clinicaltrials_oas
    if args.issue1231_prior_no_hit_recheck:
        inputs["issue1231_prior_no_hit_recheck"] = args.issue1231_prior_no_hit_recheck
    result = run(root, inputs, max_rows=args.max_rows)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
