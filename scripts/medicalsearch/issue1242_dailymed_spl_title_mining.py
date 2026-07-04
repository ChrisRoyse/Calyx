#!/usr/bin/env python3
"""#1242 DailyMed SPL-title source mining for remaining no-hit rows.

This stage reads the sealed #1240 RxNorm status rows, filters to candidates
still lacking external support, and queries current NLM DailyMed v2 SPL metadata
for deterministic pair-name co-occurrence in label titles. The output is
source-attributed research triage only: not label-content interpretation,
efficacy, safety, interaction clearance, dosing, treatment guidance, clinical
actionability, recommendation, or cure evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
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
    "DailyMed SPL-title metadata evidence is source-attributed label metadata "
    "co-mention only; not label-content interpretation, efficacy, safety, pair "
    "interaction clearance, treatment guidance, dosing guidance, recommendation, "
    "clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "dailymed_spl_title_metadata_comention_not_safety_efficacy_or_interaction_clearance"
)

ISSUE1240_ROOT = "/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z"

DEFAULT_INPUTS = {
    "issue1240_candidate_status": f"{ISSUE1240_ROOT}/out/candidate_rxnorm_status.jsonl",
    "issue1240_persisted_readback": f"{ISSUE1240_ROOT}/out/persisted_readback.json",
    "issue1240_calyx_readback": f"{ISSUE1240_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1240_output_manifest": f"{ISSUE1240_ROOT}/out/output_manifest.json",
}

DAILYMED_SPLS_ENDPOINT = "https://dailymed.nlm.nih.gov/dailymed/services/v2/spls.json"
DAILYMED_WEB_SERVICES_URL = "https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm"
DAILYMED_SPLS_API_URL = "https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm"
DAILYMED_ABOUT_URL = "https://dailymed.nlm.nih.gov/dailymed/about-dailymed.cfm"
DAILYMED_HOME_URL = "https://dailymed.nlm.nih.gov/dailymed/"
DAILYMED_LABEL_URL = "https://dailymed.nlm.nih.gov/dailymed/drugInfo.cfm"

REQUEST_SLEEP_SECONDS = 0.07
USER_AGENT = "calyx-discovery/issue1242"
STATUS_VALUES = {"exact_hit", "normalized_hit", "no_external_hit"}


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
    return " ".join(str(value).split())


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\bchembl[: ]?chembl", "chembl", text, flags=re.IGNORECASE)
    return " ".join(text.split())


def norm_name(value: object) -> str:
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
    norm = norm_name(raw)
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


def artifact(path: Path, *, jsonl: bool = False, source_url: str | None = None) -> dict[str, Any]:
    item = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
        "rows": len(rows_jsonl(path)) if jsonl else None,
    }
    if source_url:
        item["source_page_url"] = source_url
    return item


def fetch_bytes(url: str) -> tuple[int, bytes]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return int(response.status), response.read()
    except urllib.error.HTTPError as error:
        return int(error.code), error.read()


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    sources = {
        "dailymed_web_services": (DAILYMED_WEB_SERVICES_URL, "dailymed_web_services.html"),
        "dailymed_spls_api": (DAILYMED_SPLS_API_URL, "dailymed_spls_api.html"),
        "dailymed_about": (DAILYMED_ABOUT_URL, "dailymed_about.html"),
        "dailymed_home": (DAILYMED_HOME_URL, "dailymed_home.html"),
    }
    out: dict[str, dict[str, Any]] = {}
    for key, (url, filename) in sources.items():
        path = raw_dir / filename
        if not path.exists():
            status, payload = fetch_bytes(url)
            if status != 200:
                raise RuntimeError(f"source fetch failed {status}: {url}")
            path.write_bytes(payload)
            time.sleep(REQUEST_SLEEP_SECONDS)
        out[key] = artifact(path, source_url=url)
    return out


def load_candidates(path: Path) -> list[dict[str, Any]]:
    rows = rows_jsonl(path)
    out = [
        row
        for row in rows
        if row.get("overall_external_source_status_after_issue1240") == "no_external_hit"
        or row.get("dailymed_title_status") == "no_external_hit"
    ]
    out.sort(key=lambda row: (clean_text(row.get("pair_key")), clean_text(row.get("pair_id"))))
    return out


def pair_rows(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        grouped[clean_text(row.get("pair_key"))].append(row)
    rows: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        drug_a = clean_text(first.get("drug_a"))
        drug_b = clean_text(first.get("drug_b"))
        query_a = query_name(drug_a)
        query_b = query_name(drug_b)
        rows.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "representative_pair_id": clean_text(first.get("pair_id")),
                "drug_a": drug_a,
                "drug_b": drug_b,
                "query_drug_a": query_a,
                "query_drug_b": query_b,
                "candidate_row_count": len(members),
                "queryable": bool(query_a and query_b),
            }
        )
    return rows


def dailymed_spls_url(query: str) -> str:
    params = {
        "drug_name": query,
        "name_type": "both",
        "pagesize": "100",
        "page": "1",
    }
    return f"{DAILYMED_SPLS_ENDPOINT}?{urllib.parse.urlencode(params)}"


def cached_query_rows(path: Path, expected_keys: set[str]) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    try:
        rows = rows_jsonl(path)
    except Exception:
        return None
    keys = {clean_text(row.get("pair_key")) for row in rows}
    if keys == expected_keys:
        return rows
    return None


def spl_items(response_json: dict[str, Any]) -> list[dict[str, Any]]:
    data = response_json.get("data") or []
    if isinstance(data, list):
        return [item for item in data if isinstance(item, dict)]
    return []


def fetch_dailymed_queries(
    pairs: list[dict[str, Any]], out_path: Path, request_sleep_seconds: float
) -> list[dict[str, Any]]:
    expected = {row["pair_key"] for row in pairs if row["queryable"]}
    cached = cached_query_rows(out_path, expected)
    if cached is not None:
        return cached
    tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    queryable_rows = [row for row in pairs if row["queryable"]]
    with tmp_path.open("w", encoding="utf-8") as handle:
        for index, pair in enumerate(queryable_rows, start=1):
            directions = [
                {
                    "direction": "drug_a_space_drug_b",
                    "query": f"{pair['query_drug_a']} {pair['query_drug_b']}",
                },
                {
                    "direction": "drug_b_space_drug_a",
                    "query": f"{pair['query_drug_b']} {pair['query_drug_a']}",
                },
            ]
            responses = []
            for direction in directions:
                url = dailymed_spls_url(direction["query"])
                status, payload = fetch_bytes(url)
                try:
                    response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
                except json.JSONDecodeError:
                    response_json = {"raw_decode_error": payload.decode("utf-8", errors="replace")[:1000]}
                metadata = response_json.get("metadata") if isinstance(response_json, dict) else None
                items = spl_items(response_json if isinstance(response_json, dict) else {})
                responses.append(
                    {
                        "direction": direction["direction"],
                        "query": direction["query"],
                        "query_url": url,
                        "http_status": status,
                        "response_bytes": len(payload),
                        "response_sha256": sha256_bytes(payload),
                        "spl_count": len(items),
                        "metadata": metadata if isinstance(metadata, dict) else {},
                        "response_json": response_json,
                    }
                )
                time.sleep(request_sleep_seconds)
            row = {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "api_endpoint": DAILYMED_SPLS_ENDPOINT,
                "query_responses": responses,
                "total_spl_count": sum(item["spl_count"] for item in responses),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            handle.write(json.dumps(row, sort_keys=True) + "\n")
            handle.flush()
            print(
                f"DailyMed SPL title query {index}/{len(queryable_rows)} spls={row['total_spl_count']}",
                file=sys.stderr,
            )
    os.replace(tmp_path, out_path)
    return rows


def match_spl_title(item: dict[str, Any], left: str, right: str) -> dict[str, Any] | None:
    title = clean_text(item.get("title"))
    left_presence = exact_presence(title, left)
    right_presence = exact_presence(title, right)
    if not (left_presence["present"] and right_presence["present"]):
        return None
    match_kind = "exact_hit" if left_presence["exact"] and right_presence["exact"] else "normalized_hit"
    return {
        "match_kind": match_kind,
        "left_presence": left_presence,
        "right_presence": right_presence,
        "title": title,
    }


def build_evidence_rows(query_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for row in query_rows:
        for response in row["query_responses"]:
            for item in spl_items(response["response_json"]):
                match = match_spl_title(item, row["query_drug_a"], row["query_drug_b"])
                if not match:
                    continue
                setid = clean_text(item.get("setid"))
                if not setid:
                    continue
                key = (row["pair_key"], setid)
                if key in seen:
                    continue
                seen.add(key)
                evidence_id = "dailymed-title-evidence:" + stable_id(
                    row["pair_key"], setid, item.get("spl_version"), match["match_kind"]
                )
                evidence.append(
                    {
                        "schema_version": 1,
                        "evidence_id": evidence_id,
                        "pair_key": row["pair_key"],
                        "representative_pair_id": row["representative_pair_id"],
                        "drug_a": row["drug_a"],
                        "drug_b": row["drug_b"],
                        "dailymed_status": match["match_kind"],
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "source": "NLM DailyMed v2 SPL metadata",
                        "source_api_url": response["query_url"],
                        "source_label_url": f"{DAILYMED_LABEL_URL}?{urllib.parse.urlencode({'setid': setid})}",
                        "source_setid": setid,
                        "spl_version": item.get("spl_version"),
                        "published_date": clean_text(item.get("published_date")),
                        "title": match["title"],
                        "direction": response["direction"],
                        "response_sha256": response["response_sha256"],
                        "left_presence": match["left_presence"],
                        "right_presence": match["right_presence"],
                        "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
    evidence.sort(key=lambda row: (row["pair_key"], row["source_setid"], row["evidence_id"]))
    return evidence


def pair_status_rows(
    pairs: list[dict[str, Any]], query_rows: list[dict[str, Any]], evidence: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    query_by_key = {row["pair_key"]: row for row in query_rows}
    evidence_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence:
        evidence_by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for pair in pairs:
        query = query_by_key.get(pair["pair_key"])
        ev = evidence_by_key.get(pair["pair_key"], [])
        if any(row["dailymed_status"] == "exact_hit" for row in ev):
            status = "exact_hit"
        elif ev:
            status = "normalized_hit"
        else:
            status = "no_external_hit"
        rows.append(
            {
                "schema_version": 1,
                "pair_status_id": "dailymed-title-pair-status:" + stable_id(pair["pair_key"], status),
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "queryable": pair["queryable"],
                "dailymed_title_status": status,
                "total_spl_count": query["total_spl_count"] if query else 0,
                "query_response_sha256": stable_id(*(r["response_sha256"] for r in query["query_responses"])) if query else None,
                "title_evidence_rows": len(ev),
                "source_setids": uniq([row["source_setid"] for row in ev])[:50],
                "evidence_ids": [row["evidence_id"] for row in ev[:50]],
                "overall_external_source_status_after_issue1242": status,
                "reason_codes": [
                    "dailymed_spl_title_metadata_not_clinical_clearance",
                    "daily_med_label_title_copresence_only" if ev else "no_dailymed_spl_title_pair_match",
                ],
                "next_validation_experiment": (
                    "Run label-content safety/outcome/falsification and human-review gates before any promotion."
                    if ev
                    else "Continue source expansion or richer synonym normalization; do not promote this pair."
                ),
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def candidate_status_rows(
    candidates: list[dict[str, Any]], pair_status: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    status_by_pair = {row["pair_key"]: row for row in pair_status}
    rows: list[dict[str, Any]] = []
    for row in candidates:
        pair = status_by_pair[row["pair_key"]]
        rows.append(
            {
                "schema_version": 1,
                "status_id": "dailymed-title-candidate-status:" + stable_id(row["status_id"], pair["pair_status_id"]),
                "pair_id": clean_text(row.get("pair_id")),
                "pair_key": row["pair_key"],
                "drug_a": clean_text(row.get("drug_a")),
                "drug_b": clean_text(row.get("drug_b")),
                "source_issue1240_status_id": clean_text(row.get("status_id")),
                "previous_overall_external_source_status": clean_text(
                    row.get("overall_external_source_status_after_issue1240")
                ),
                "dailymed_title_status": pair["dailymed_title_status"],
                "overall_external_source_status_after_issue1242": pair[
                    "overall_external_source_status_after_issue1242"
                ],
                "pair_status_id": pair["pair_status_id"],
                "title_evidence_rows": pair["title_evidence_rows"],
                "source_setids": pair["source_setids"],
                "evidence_ids": pair["evidence_ids"],
                "reason_codes": pair["reason_codes"],
                "next_validation_experiment": pair["next_validation_experiment"],
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def build_metrics(
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
) -> dict[str, Any]:
    status_counts = Counter(row["dailymed_title_status"] for row in pair_status)
    candidate_counts = Counter(row["dailymed_title_status"] for row in candidate_status)
    http_counts = Counter()
    directional_requests = 0
    total_spl_results = 0
    for row in query_rows:
        for response in row["query_responses"]:
            directional_requests += 1
            http_counts[str(response["http_status"])] += 1
            total_spl_results += response["spl_count"]
    return {
        "schema_version": 1,
        "status": "ok",
        "issue1240_remaining_no_hit_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "queryable_pair_keys": sum(1 for row in pairs if row["queryable"]),
        "dailymed_query_response_rows": len(query_rows),
        "directional_http_requests": directional_requests,
        "http_status_counts": dict(sorted(http_counts.items())),
        "total_spl_results_returned": total_spl_results,
        "query_rows_with_spls": sum(1 for row in query_rows if row["total_spl_count"] > 0),
        "dailymed_title_evidence_rows": len(evidence),
        "candidate_rows_with_issue1242_hit": sum(
            1 for row in candidate_status if row["dailymed_title_status"] in {"exact_hit", "normalized_hit"}
        ),
        "remaining_no_hit_after_issue1242": sum(
            1 for row in candidate_status if row["dailymed_title_status"] == "no_external_hit"
        ),
        "pair_status_counts": dict(status_counts),
        "candidate_status_counts": dict(candidate_counts),
        "top_hits": [
            {
                "pair_key": row["pair_key"],
                "status": row["dailymed_title_status"],
                "title_evidence_rows": row["title_evidence_rows"],
                "source_setids": row["source_setids"][:5],
            }
            for row in sorted(
                [row for row in pair_status if row["title_evidence_rows"] > 0],
                key=lambda item: (-item["title_evidence_rows"], item["pair_key"]),
            )[:20]
        ],
        "response_schema": schema_fingerprint(query_rows),
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def schema_fingerprint(query_rows: list[dict[str, Any]]) -> dict[str, Any]:
    data_keys: set[str] = set()
    metadata_keys: set[str] = set()
    for row in query_rows:
        for response in row["query_responses"]:
            response_json = response.get("response_json", {})
            if isinstance(response_json, dict):
                metadata = response_json.get("metadata") or {}
                if isinstance(metadata, dict):
                    metadata_keys.update(metadata.keys())
                for item in spl_items(response_json):
                    data_keys.update(item.keys())
    payload = json.dumps(
        {"data_keys": sorted(data_keys), "metadata_keys": sorted(metadata_keys)},
        sort_keys=True,
    ).encode("utf-8")
    return {
        "data_keys": sorted(data_keys),
        "metadata_keys": sorted(metadata_keys),
        "schema_fingerprint_sha256": sha256_bytes(payload),
    }


def bridge_terms(values: list[object]) -> list[str]:
    return [value for value in uniq(values) if value]


def build_bridge_rows(
    pair_status: list[dict[str, Any]], evidence: list[dict[str, Any]], source_path: Path, source_sha: str
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        text = (
            f"DailyMed SPL title pair {row['pair_key']} status for {row['drug_a']} + {row['drug_b']}: "
            f"{row['dailymed_title_status']}; evidence_rows={row['title_evidence_rows']}; "
            f"boundary={CLINICAL_BOUNDARY}"
        )
        rows.append(
            {
                "id": "issue1242-dailymed-title-pair:" + stable_id(row["pair_status_id"]),
                "domain": "dailymed_spl_title_pair_status",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["dailymed_title_status"],
                    ]
                    + row["source_setids"][:10]
                ),
                "metadata": {
                    "issue": "1242",
                    "pair_key": row["pair_key"],
                    "status": row["dailymed_title_status"],
                    "source_dataset": "dailymed_spl_title_pair_status",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence[:budget]:
        text = (
            f"DailyMed SPL title metadata hit for pair {row['pair_key']} status {row['dailymed_status']} "
            f"for {row['drug_a']} + {row['drug_b']}: {row['title']} "
            f"setid={row['source_setid']}; boundary={CLINICAL_BOUNDARY}"
        )
        rows.append(
            {
                "id": "issue1242-dailymed-title-evidence:" + stable_id(row["evidence_id"]),
                "domain": "dailymed_spl_title_evidence",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["source_setid"],
                        row["dailymed_status"],
                    ]
                ),
                "metadata": {
                    "issue": "1242",
                    "pair_key": row["pair_key"],
                    "evidence_id": row["evidence_id"],
                    "source_setid": row["source_setid"],
                    "status": row["dailymed_status"],
                    "source_dataset": "dailymed_spl_title_evidence",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(assertions.values())


def build_input_manifest(
    inputs: dict[str, str],
    raw_artifacts: dict[str, dict[str, Any]],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    issue1240_persisted_readback: dict[str, Any],
    issue1240_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1242,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1240_candidate_status": artifact(Path(inputs["issue1240_candidate_status"]), jsonl=True),
            "issue1240_persisted_readback": artifact(Path(inputs["issue1240_persisted_readback"])),
            "issue1240_calyx_readback": artifact(Path(inputs["issue1240_calyx_readback"])),
            "issue1240_output_manifest": artifact(Path(inputs["issue1240_output_manifest"])),
            **raw_artifacts,
        },
        "source_contract": {
            "issue1240_persisted_readback_status": issue1240_persisted_readback.get("status"),
            "issue1240_persisted_assertions_all_true": all_assertions_true(issue1240_persisted_readback),
            "issue1240_calyx_readback_status": issue1240_calyx_readback.get("status"),
            "issue1240_calyx_assertions_all_true": all_assertions_true(issue1240_calyx_readback),
            "remaining_no_hit_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "daily_med_api_resource": DAILYMED_SPLS_ENDPOINT,
            "pagesize": 100,
            "queried_pages_per_direction": 1,
        },
        "accepted_sources": [
            {
                "source": "NLM DailyMed v2 SPL metadata",
                "role": "current SPL title metadata source mining",
                "api_endpoint": DAILYMED_SPLS_ENDPOINT,
                "docs_url": DAILYMED_WEB_SERVICES_URL,
                "spls_api_url": DAILYMED_SPLS_API_URL,
                "about_url": DAILYMED_ABOUT_URL,
                "license_observation": "DailyMed/NLM pages state NLM provides DailyMed to the public and the API retrieves current SPL information.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        ],
    }


def build_persisted_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    issue1240_persisted_readback: dict[str, Any],
    issue1240_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "dailymed_spl_title_query_responses": artifact(
            out_dir / "dailymed_spl_title_query_responses.jsonl", jsonl=True
        ),
        "dailymed_spl_title_evidence": artifact(out_dir / "dailymed_spl_title_evidence.jsonl", jsonl=True),
        "dailymed_spl_title_pair_status": artifact(
            out_dir / "dailymed_spl_title_pair_status.jsonl", jsonl=True
        ),
        "candidate_dailymed_title_status": artifact(
            out_dir / "candidate_dailymed_title_status.jsonl", jsonl=True
        ),
        "dailymed_spl_title_bridge_rows": artifact(out_dir / "dailymed_spl_title_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    queryable_keys = {row["pair_key"] for row in pairs if row["queryable"]}
    response_keys = {row["pair_key"] for row in query_rows}
    evidence_keys = {row["pair_key"] for row in evidence}
    hit_keys = {
        row["pair_key"]
        for row in pair_status
        if row["dailymed_title_status"] in {"exact_hit", "normalized_hit"}
    }
    return {
        "schema_version": 1,
        "issue": 1242,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1240_persisted_readback_all_true": all_assertions_true(issue1240_persisted_readback),
            "issue1240_calyx_readback_all_true": all_assertions_true(issue1240_calyx_readback),
            "candidate_status_rows_for_every_remaining_no_hit": len(candidate_status) == len(candidates),
            "pair_status_rows_for_every_unique_pair_key": len(pair_status) == len(pairs),
            "query_response_for_every_queryable_pair_key": queryable_keys == response_keys,
            "two_directional_responses_per_query_row": all(len(row["query_responses"]) == 2 for row in query_rows),
            "all_pair_status_values_allowed": all(row["dailymed_title_status"] in STATUS_VALUES for row in pair_status),
            "all_candidate_status_values_allowed": all(
                row["dailymed_title_status"] in STATUS_VALUES for row in candidate_status
            ),
            "all_status_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in pair_status)
            and all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in candidate_status),
            "all_hits_have_evidence": hit_keys <= evidence_keys,
            "all_evidence_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in evidence),
            "all_evidence_rows_have_setid": all(bool(row.get("source_setid")) for row in evidence),
            "all_candidate_rows_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in candidate_status
            ),
            "bridge_rows_1000_or_less": artifacts["dailymed_spl_title_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "remaining_no_hit_candidates": len(candidates),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence),
            "pair_status_rows": len(pair_status),
            "candidate_status_rows": len(candidate_status),
        },
    }


def run(root: Path, inputs: dict[str, str], max_pairs: int | None, request_sleep_seconds: float) -> dict[str, Any]:
    raw_dir = root / "raw"
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_artifacts = fetch_raw_sources(raw_dir)
    candidates = load_candidates(Path(inputs["issue1240_candidate_status"]))
    pairs = pair_rows(candidates)
    if max_pairs is not None:
        keep = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in keep]
        candidates = [row for row in candidates if row["pair_key"] in keep]
    issue1240_persisted_readback = read_json(Path(inputs["issue1240_persisted_readback"]))
    issue1240_calyx_readback = read_json(Path(inputs["issue1240_calyx_readback"]))

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            raw_artifacts,
            candidates,
            pairs,
            issue1240_persisted_readback,
            issue1240_calyx_readback,
        ),
    )
    query_rows = fetch_dailymed_queries(pairs, out_dir / "dailymed_spl_title_query_responses.jsonl", request_sleep_seconds)
    evidence = build_evidence_rows(query_rows)
    write_jsonl(out_dir / "dailymed_spl_title_evidence.jsonl", evidence)
    pair_status = pair_status_rows(pairs, query_rows, evidence)
    write_jsonl(out_dir / "dailymed_spl_title_pair_status.jsonl", pair_status)
    candidate_status = candidate_status_rows(candidates, pair_status)
    write_jsonl(out_dir / "candidate_dailymed_title_status.jsonl", candidate_status)
    source_path = out_dir / "dailymed_spl_title_pair_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(pair_status, evidence, source_path, source_sha)
    write_jsonl(out_dir / "dailymed_spl_title_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(candidates, pairs, query_rows, evidence, pair_status, candidate_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1242,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "dailymed_spl_title_query_responses": artifact(
                out_dir / "dailymed_spl_title_query_responses.jsonl", jsonl=True
            ),
            "dailymed_spl_title_evidence": artifact(out_dir / "dailymed_spl_title_evidence.jsonl", jsonl=True),
            "dailymed_spl_title_pair_status": artifact(
                out_dir / "dailymed_spl_title_pair_status.jsonl", jsonl=True
            ),
            "candidate_dailymed_title_status": artifact(
                out_dir / "candidate_dailymed_title_status.jsonl", jsonl=True
            ),
            "dailymed_spl_title_bridge_rows": artifact(out_dir / "dailymed_spl_title_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_persisted_readback(
        out_dir,
        candidates,
        pairs,
        query_rows,
        evidence,
        pair_status,
        candidate_status,
        issue1240_persisted_readback,
        issue1240_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "candidate_dailymed_title_status": output_manifest["artifacts"]["candidate_dailymed_title_status"],
            "dailymed_spl_title_evidence": output_manifest["artifacts"]["dailymed_spl_title_evidence"],
            "bridge_rows": output_manifest["artifacts"]["dailymed_spl_title_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1240-candidate-status")
    parser.add_argument("--issue1240-persisted-readback")
    parser.add_argument("--issue1240-calyx-readback")
    parser.add_argument("--issue1240-output-manifest")
    parser.add_argument("--max-pairs", type=int)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1240_candidate_status", "issue1240_candidate_status"),
        ("issue1240_persisted_readback", "issue1240_persisted_readback"),
        ("issue1240_calyx_readback", "issue1240_calyx_readback"),
        ("issue1240_output_manifest", "issue1240_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, args.max_pairs, args.request_sleep_seconds)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
