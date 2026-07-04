#!/usr/bin/env python3
"""#1243 Europe PMC pair-search source mining for remaining no-hit rows.

This stage reads the sealed #1242 DailyMed status rows, filters to candidates
still lacking external support, and queries Europe PMC Articles REST search for
deterministic pair co-occurrence evidence. Search-hit counts alone are not
promoted: a hit requires both candidate terms in returned metadata text or in a
bounded fetched PMCID full-text XML record. Output is source-attributed
research triage only: not efficacy, safety, treatment guidance, dosing guidance,
clinical actionability, recommendation, or cure evidence.
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
    "Europe PMC pair-search evidence is source-attributed literature/index "
    "co-mention only; not efficacy, safety, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "europepmc_pair_search_comention_not_safety_efficacy_treatment_or_cure_evidence"
)

ISSUE1242_ROOT = "/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z"

DEFAULT_INPUTS = {
    "issue1242_candidate_status": f"{ISSUE1242_ROOT}/out/candidate_dailymed_title_status.jsonl",
    "issue1242_persisted_readback": f"{ISSUE1242_ROOT}/out/persisted_readback.json",
    "issue1242_calyx_readback": f"{ISSUE1242_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1242_output_manifest": f"{ISSUE1242_ROOT}/out/output_manifest.json",
}

EUROPEPMC_SEARCH_ENDPOINT = "https://www.ebi.ac.uk/europepmc/webservices/rest/search"
EUROPEPMC_FULLTEXT_BASE = "https://www.ebi.ac.uk/europepmc/webservices/rest"
EUROPEPMC_REST_DOCS_URL = "https://europepmc.org/RestfulWebService"
EUROPEPMC_ANNOTATIONS_DOCS_URL = "https://europepmc.org/AnnotationsApi"
EUROPEPMC_DEVELOPERS_URL = "https://europepmc.org/developers"
EUROPEPMC_ABOUT_URL = "https://europepmc.org/About"

REQUEST_SLEEP_SECONDS = 0.07
USER_AGENT = "calyx-discovery/issue1243"
STATUS_VALUES = {"exact_hit", "normalized_hit", "no_external_hit"}
PAGE_SIZE = 5
MAX_FULLTEXT_FETCHES_PER_PAIR = 3
MAX_FULLTEXT_FETCHES_TOTAL = 1200


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
        with urllib.request.urlopen(request, timeout=90) as response:
            return int(response.status), response.read()
    except urllib.error.HTTPError as error:
        return int(error.code), error.read()


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    sources = {
        "europepmc_rest_docs": (EUROPEPMC_REST_DOCS_URL, "europepmc_rest_docs.html"),
        "europepmc_annotations_docs": (EUROPEPMC_ANNOTATIONS_DOCS_URL, "europepmc_annotations_docs.html"),
        "europepmc_developers": (EUROPEPMC_DEVELOPERS_URL, "europepmc_developers.html"),
        "europepmc_about": (EUROPEPMC_ABOUT_URL, "europepmc_about.html"),
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
        if row.get("overall_external_source_status_after_issue1242") == "no_external_hit"
        or row.get("europepmc_status") == "no_external_hit"
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


def europepmc_query(left: str, right: str) -> str:
    return f'"{left}" AND "{right}"'


def europepmc_search_url(query: str) -> str:
    params = {
        "query": query,
        "format": "json",
        "resultType": "core",
        "pageSize": str(PAGE_SIZE),
        "cursorMark": "*",
        "synonym": "false",
    }
    return f"{EUROPEPMC_SEARCH_ENDPOINT}?{urllib.parse.urlencode(params)}"


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


def search_results(response_json: dict[str, Any]) -> list[dict[str, Any]]:
    result_list = response_json.get("resultList") or {}
    if not isinstance(result_list, dict):
        return []
    results = result_list.get("result") or []
    return [item for item in results if isinstance(item, dict)] if isinstance(results, list) else []


def fetch_europepmc_queries(
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
            query = europepmc_query(pair["query_drug_a"], pair["query_drug_b"])
            url = europepmc_search_url(query)
            status, payload = fetch_bytes(url)
            try:
                response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
            except json.JSONDecodeError:
                response_json = {"raw_decode_error": payload.decode("utf-8", errors="replace")[:1000]}
            results = search_results(response_json if isinstance(response_json, dict) else {})
            row = {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "api_endpoint": EUROPEPMC_SEARCH_ENDPOINT,
                "query": query,
                "query_url": url,
                "http_status": status,
                "response_bytes": len(payload),
                "response_sha256": sha256_bytes(payload),
                "hit_count": int(response_json.get("hitCount") or 0)
                if isinstance(response_json, dict) and str(response_json.get("hitCount") or "0").isdigit()
                else 0,
                "returned_result_count": len(results),
                "response_json": response_json,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            handle.write(json.dumps(row, sort_keys=True) + "\n")
            handle.flush()
            print(
                f"Europe PMC pair query {index}/{len(queryable_rows)} hits={row['hit_count']} returned={row['returned_result_count']}",
                file=sys.stderr,
            )
            time.sleep(request_sleep_seconds)
    os.replace(tmp_path, out_path)
    return rows


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


def fulltext_url(pmcid: str) -> str:
    return f"{EUROPEPMC_FULLTEXT_BASE}/{pmcid}/fullTextXML"


def strip_xml(text: str) -> str:
    text = re.sub(r"<[^>]+>", " ", text)
    text = re.sub(r"&[a-zA-Z0-9#]+;", " ", text)
    return clean_text(text)


def match_text(text: str, left: str, right: str) -> dict[str, Any] | None:
    left_presence = exact_presence(text, left)
    right_presence = exact_presence(text, right)
    if not (left_presence["present"] and right_presence["present"]):
        return None
    return {
        "match_kind": "exact_hit" if left_presence["exact"] and right_presence["exact"] else "normalized_hit",
        "left_presence": left_presence,
        "right_presence": right_presence,
    }


def result_identity(item: dict[str, Any]) -> dict[str, str]:
    return {
        "source": clean_text(item.get("source")),
        "id": clean_text(item.get("id")),
        "pmid": clean_text(item.get("pmid")),
        "pmcid": pmcid_of(item),
        "doi": clean_text(item.get("doi")),
        "title": clean_text(item.get("title")),
        "pub_year": clean_text(item.get("pubYear")),
        "is_open_access": clean_text(item.get("isOpenAccess")),
        "in_pmc": clean_text(item.get("inPMC")),
    }


def fetch_fulltext_if_needed(
    row: dict[str, Any],
    item: dict[str, Any],
    fulltext_dir: Path,
    request_sleep_seconds: float,
    fulltext_fetch_count: int,
) -> tuple[dict[str, Any] | None, int]:
    if fulltext_fetch_count >= MAX_FULLTEXT_FETCHES_TOTAL:
        return None, fulltext_fetch_count
    pmcid = pmcid_of(item)
    if not pmcid:
        return None, fulltext_fetch_count
    fulltext_dir.mkdir(parents=True, exist_ok=True)
    path = fulltext_dir / f"{pmcid}.xml"
    url = fulltext_url(pmcid)
    if path.exists():
        payload = path.read_bytes()
        status = 200
    else:
        status, payload = fetch_bytes(url)
        if status == 200:
            path.write_bytes(payload)
        time.sleep(request_sleep_seconds)
    fulltext_fetch_count += 1
    text = strip_xml(payload.decode("utf-8", errors="replace"))
    match = match_text(text, row["query_drug_a"], row["query_drug_b"])
    return (
        {
            "schema_version": 1,
            "pair_key": row["pair_key"],
            "result_id": clean_text(item.get("id")),
            "source": clean_text(item.get("source")),
            "pmcid": pmcid,
            "url": url,
            "http_status": status,
            "bytes": len(payload),
            "sha256": sha256_bytes(payload),
            "matched": bool(match),
            "match_kind": match["match_kind"] if match else "no_external_hit",
            "left_presence": match["left_presence"] if match else exact_presence(text, row["query_drug_a"]),
            "right_presence": match["right_presence"] if match else exact_presence(text, row["query_drug_b"]),
            "clinical_boundary": CLINICAL_BOUNDARY,
        },
        fulltext_fetch_count,
    )


def build_evidence_rows(
    query_rows: list[dict[str, Any]], fulltext_dir: Path, request_sleep_seconds: float
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    evidence: list[dict[str, Any]] = []
    fulltext_fetches: list[dict[str, Any]] = []
    seen: set[tuple[str, str, str]] = set()
    fulltext_fetch_count = 0
    for row in query_rows:
        per_pair_fulltext = 0
        for item in search_results(row["response_json"]):
            identity = result_identity(item)
            text = metadata_text(item)
            match = match_text(text, row["query_drug_a"], row["query_drug_b"])
            evidence_channel = "metadata_text"
            fulltext_fetch: dict[str, Any] | None = None
            if not match and per_pair_fulltext < MAX_FULLTEXT_FETCHES_PER_PAIR:
                fulltext_fetch, fulltext_fetch_count = fetch_fulltext_if_needed(
                    row, item, fulltext_dir, request_sleep_seconds, fulltext_fetch_count
                )
                if fulltext_fetch is not None:
                    fulltext_fetches.append(fulltext_fetch)
                    per_pair_fulltext += 1
                    if fulltext_fetch["matched"]:
                        match = {
                            "match_kind": fulltext_fetch["match_kind"],
                            "left_presence": fulltext_fetch["left_presence"],
                            "right_presence": fulltext_fetch["right_presence"],
                        }
                        evidence_channel = "pmcid_fulltext_xml"
            if not match:
                continue
            source_id = identity["pmcid"] or identity["pmid"] or identity["doi"] or identity["id"]
            key = (row["pair_key"], source_id, evidence_channel)
            if key in seen:
                continue
            seen.add(key)
            evidence_id = "europepmc-pair-evidence:" + stable_id(
                row["pair_key"], source_id, evidence_channel, match["match_kind"]
            )
            evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": evidence_id,
                    "pair_key": row["pair_key"],
                    "representative_pair_id": row["representative_pair_id"],
                    "drug_a": row["drug_a"],
                    "drug_b": row["drug_b"],
                    "europepmc_status": match["match_kind"],
                    "evidence_channel": evidence_channel,
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "source": "Europe PMC Articles REST API",
                    "source_api_url": row["query_url"],
                    "source_id": source_id,
                    "source_record": identity,
                    "hit_count": row["hit_count"],
                    "response_sha256": row["response_sha256"],
                    "fulltext_fetch_sha256": fulltext_fetch["sha256"] if fulltext_fetch and fulltext_fetch["matched"] else None,
                    "left_presence": match["left_presence"],
                    "right_presence": match["right_presence"],
                    "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    evidence.sort(key=lambda row: (row["pair_key"], row["source_id"], row["evidence_channel"]))
    return evidence, fulltext_fetches


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
        if any(row["europepmc_status"] == "exact_hit" for row in ev):
            status = "exact_hit"
        elif ev:
            status = "normalized_hit"
        else:
            status = "no_external_hit"
        rows.append(
            {
                "schema_version": 1,
                "pair_status_id": "europepmc-pair-status:" + stable_id(pair["pair_key"], status),
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "queryable": pair["queryable"],
                "europepmc_status": status,
                "hit_count": query["hit_count"] if query else 0,
                "returned_result_count": query["returned_result_count"] if query else 0,
                "query_response_sha256": query["response_sha256"] if query else None,
                "evidence_rows": len(ev),
                "source_ids": uniq([row["source_id"] for row in ev])[:50],
                "evidence_channels": uniq([row["evidence_channel"] for row in ev])[:20],
                "evidence_ids": [row["evidence_id"] for row in ev[:50]],
                "overall_external_source_status_after_issue1243": status,
                "reason_codes": [
                    "europepmc_literature_comention_not_clinical_clearance",
                    "verified_metadata_or_fulltext_terms_present" if ev else "no_verified_europepmc_pair_text_match",
                ],
                "next_validation_experiment": (
                    "Run source-text relation, safety/outcome/falsification, and human-review gates before any promotion."
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
                "status_id": "europepmc-candidate-status:" + stable_id(row["status_id"], pair["pair_status_id"]),
                "pair_id": clean_text(row.get("pair_id")),
                "pair_key": row["pair_key"],
                "drug_a": clean_text(row.get("drug_a")),
                "drug_b": clean_text(row.get("drug_b")),
                "source_issue1242_status_id": clean_text(row.get("status_id")),
                "previous_overall_external_source_status": clean_text(
                    row.get("overall_external_source_status_after_issue1242")
                ),
                "europepmc_status": pair["europepmc_status"],
                "overall_external_source_status_after_issue1243": pair[
                    "overall_external_source_status_after_issue1243"
                ],
                "pair_status_id": pair["pair_status_id"],
                "evidence_rows": pair["evidence_rows"],
                "source_ids": pair["source_ids"],
                "evidence_ids": pair["evidence_ids"],
                "reason_codes": pair["reason_codes"],
                "next_validation_experiment": pair["next_validation_experiment"],
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def schema_fingerprint(query_rows: list[dict[str, Any]]) -> dict[str, Any]:
    wrapper_keys: set[str] = set()
    request_keys: set[str] = set()
    result_keys: set[str] = set()
    for row in query_rows:
        response_json = row.get("response_json", {})
        if isinstance(response_json, dict):
            wrapper_keys.update(response_json.keys())
            request = response_json.get("request") or {}
            if isinstance(request, dict):
                request_keys.update(request.keys())
            for item in search_results(response_json):
                result_keys.update(item.keys())
    payload = json.dumps(
        {
            "wrapper_keys": sorted(wrapper_keys),
            "request_keys": sorted(request_keys),
            "result_keys": sorted(result_keys),
        },
        sort_keys=True,
    ).encode("utf-8")
    return {
        "wrapper_keys": sorted(wrapper_keys),
        "request_keys": sorted(request_keys),
        "result_keys": sorted(result_keys),
        "schema_fingerprint_sha256": sha256_bytes(payload),
    }


def build_metrics(
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    fulltext_fetches: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
) -> dict[str, Any]:
    status_counts = Counter(row["europepmc_status"] for row in pair_status)
    candidate_counts = Counter(row["europepmc_status"] for row in candidate_status)
    http_counts = Counter(str(row["http_status"]) for row in query_rows)
    fulltext_status_counts = Counter(str(row["http_status"]) for row in fulltext_fetches)
    return {
        "schema_version": 1,
        "status": "ok",
        "issue1242_remaining_no_hit_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "queryable_pair_keys": sum(1 for row in pairs if row["queryable"]),
        "europepmc_query_response_rows": len(query_rows),
        "http_status_counts": dict(sorted(http_counts.items())),
        "pairs_with_search_hit_count_gt_0": sum(1 for row in query_rows if row["hit_count"] > 0),
        "total_europepmc_hit_count": sum(row["hit_count"] for row in query_rows),
        "returned_result_rows": sum(row["returned_result_count"] for row in query_rows),
        "fulltext_fetch_rows": len(fulltext_fetches),
        "fulltext_status_counts": dict(sorted(fulltext_status_counts.items())),
        "fulltext_matched_rows": sum(1 for row in fulltext_fetches if row["matched"]),
        "europepmc_evidence_rows": len(evidence),
        "candidate_rows_with_issue1243_hit": sum(
            1 for row in candidate_status if row["europepmc_status"] in {"exact_hit", "normalized_hit"}
        ),
        "remaining_no_hit_after_issue1243": sum(
            1 for row in candidate_status if row["europepmc_status"] == "no_external_hit"
        ),
        "pair_status_counts": dict(status_counts),
        "candidate_status_counts": dict(candidate_counts),
        "evidence_channel_counts": dict(Counter(row["evidence_channel"] for row in evidence)),
        "top_hits": [
            {
                "pair_key": row["pair_key"],
                "status": row["europepmc_status"],
                "evidence_rows": row["evidence_rows"],
                "hit_count": row["hit_count"],
                "source_ids": row["source_ids"][:5],
                "evidence_channels": row["evidence_channels"],
            }
            for row in sorted(
                [row for row in pair_status if row["evidence_rows"] > 0],
                key=lambda item: (-item["evidence_rows"], -item["hit_count"], item["pair_key"]),
            )[:20]
        ],
        "response_schema": schema_fingerprint(query_rows),
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def bridge_terms(values: list[object]) -> list[str]:
    return [value for value in uniq(values) if value]


def build_bridge_rows(
    pair_status: list[dict[str, Any]], evidence: list[dict[str, Any]], source_path: Path, source_sha: str
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        text = (
            f"Europe PMC pair {row['pair_key']} status {row['europepmc_status']} "
            f"for {row['drug_a']} + {row['drug_b']}; evidence_rows={row['evidence_rows']}; "
            f"hit_count={row['hit_count']}; boundary={CLINICAL_BOUNDARY}"
        )
        rows.append(
            {
                "id": "issue1243-europepmc-pair:" + stable_id(row["pair_status_id"]),
                "domain": "europepmc_pair_status",
                "text": text,
                "bridge_terms": bridge_terms(
                    [row["pair_key"], row["drug_a"], row["drug_b"], row["europepmc_status"]]
                ),
                "metadata": {
                    "issue": "1243",
                    "pair_key": row["pair_key"],
                    "status": row["europepmc_status"],
                    "source_dataset": "europepmc_pair_status",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence[:budget]:
        text = (
            f"Europe PMC evidence {row['source_id']} channel {row['evidence_channel']} "
            f"status {row['europepmc_status']} for pair {row['pair_key']} "
            f"{row['drug_a']} + {row['drug_b']}; boundary={CLINICAL_BOUNDARY}"
        )
        rows.append(
            {
                "id": "issue1243-europepmc-evidence:" + stable_id(row["evidence_id"]),
                "domain": "europepmc_pair_evidence",
                "text": text,
                "bridge_terms": bridge_terms(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["source_id"],
                        row["evidence_channel"],
                        row["europepmc_status"],
                    ]
                ),
                "metadata": {
                    "issue": "1243",
                    "pair_key": row["pair_key"],
                    "evidence_id": row["evidence_id"],
                    "source_id": row["source_id"],
                    "status": row["europepmc_status"],
                    "source_dataset": "europepmc_pair_evidence",
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
    issue1242_persisted_readback: dict[str, Any],
    issue1242_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1243,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1242_candidate_status": artifact(Path(inputs["issue1242_candidate_status"]), jsonl=True),
            "issue1242_persisted_readback": artifact(Path(inputs["issue1242_persisted_readback"])),
            "issue1242_calyx_readback": artifact(Path(inputs["issue1242_calyx_readback"])),
            "issue1242_output_manifest": artifact(Path(inputs["issue1242_output_manifest"])),
            **raw_artifacts,
        },
        "source_contract": {
            "issue1242_persisted_readback_status": issue1242_persisted_readback.get("status"),
            "issue1242_persisted_assertions_all_true": all_assertions_true(issue1242_persisted_readback),
            "issue1242_calyx_readback_status": issue1242_calyx_readback.get("status"),
            "issue1242_calyx_assertions_all_true": all_assertions_true(issue1242_calyx_readback),
            "remaining_no_hit_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "europepmc_search_endpoint": EUROPEPMC_SEARCH_ENDPOINT,
            "page_size": PAGE_SIZE,
            "max_fulltext_fetches_per_pair": MAX_FULLTEXT_FETCHES_PER_PAIR,
            "max_fulltext_fetches_total": MAX_FULLTEXT_FETCHES_TOTAL,
        },
        "accepted_sources": [
            {
                "source": "Europe PMC Articles REST API",
                "role": "current literature/index pair-search source mining",
                "api_endpoint": EUROPEPMC_SEARCH_ENDPOINT,
                "fulltext_endpoint_pattern": f"{EUROPEPMC_FULLTEXT_BASE}/{{PMCID}}/fullTextXML",
                "docs_url": EUROPEPMC_REST_DOCS_URL,
                "developers_url": EUROPEPMC_DEVELOPERS_URL,
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
    fulltext_fetches: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    issue1242_persisted_readback: dict[str, Any],
    issue1242_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "europepmc_pair_query_responses": artifact(out_dir / "europepmc_pair_query_responses.jsonl", jsonl=True),
        "europepmc_fulltext_fetches": artifact(out_dir / "europepmc_fulltext_fetches.jsonl", jsonl=True),
        "europepmc_pair_evidence": artifact(out_dir / "europepmc_pair_evidence.jsonl", jsonl=True),
        "europepmc_pair_status": artifact(out_dir / "europepmc_pair_status.jsonl", jsonl=True),
        "candidate_europepmc_status": artifact(out_dir / "candidate_europepmc_status.jsonl", jsonl=True),
        "europepmc_bridge_rows": artifact(out_dir / "europepmc_bridge_rows.jsonl", jsonl=True),
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
        if row["europepmc_status"] in {"exact_hit", "normalized_hit"}
    }
    fulltext_by_key = {row["pair_key"] for row in fulltext_fetches if row["matched"]}
    evidence_fulltext_keys = {row["pair_key"] for row in evidence if row["evidence_channel"] == "pmcid_fulltext_xml"}
    return {
        "schema_version": 1,
        "issue": 1243,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1242_persisted_readback_all_true": all_assertions_true(issue1242_persisted_readback),
            "issue1242_calyx_readback_all_true": all_assertions_true(issue1242_calyx_readback),
            "candidate_status_rows_for_every_remaining_no_hit": len(candidate_status) == len(candidates),
            "pair_status_rows_for_every_unique_pair_key": len(pair_status) == len(pairs),
            "query_response_for_every_queryable_pair_key": queryable_keys == response_keys,
            "all_query_http_status_200": all(row["http_status"] == 200 for row in query_rows),
            "all_pair_status_values_allowed": all(row["europepmc_status"] in STATUS_VALUES for row in pair_status),
            "all_candidate_status_values_allowed": all(
                row["europepmc_status"] in STATUS_VALUES for row in candidate_status
            ),
            "all_status_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in pair_status)
            and all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in candidate_status),
            "all_hits_have_evidence": hit_keys <= evidence_keys,
            "fulltext_matches_have_evidence": fulltext_by_key <= evidence_fulltext_keys,
            "all_evidence_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in evidence),
            "all_evidence_rows_have_source_id": all(bool(row.get("source_id")) for row in evidence),
            "all_candidate_rows_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in candidate_status
            ),
            "bridge_rows_1000_or_less": artifacts["europepmc_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "remaining_no_hit_candidates": len(candidates),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "fulltext_fetch_rows": len(fulltext_fetches),
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
    candidates = load_candidates(Path(inputs["issue1242_candidate_status"]))
    pairs = pair_rows(candidates)
    if max_pairs is not None:
        keep = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in keep]
        candidates = [row for row in candidates if row["pair_key"] in keep]
    issue1242_persisted_readback = read_json(Path(inputs["issue1242_persisted_readback"]))
    issue1242_calyx_readback = read_json(Path(inputs["issue1242_calyx_readback"]))

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            raw_artifacts,
            candidates,
            pairs,
            issue1242_persisted_readback,
            issue1242_calyx_readback,
        ),
    )
    query_rows = fetch_europepmc_queries(pairs, out_dir / "europepmc_pair_query_responses.jsonl", request_sleep_seconds)
    evidence, fulltext_fetches = build_evidence_rows(query_rows, raw_dir / "fulltext", request_sleep_seconds)
    write_jsonl(out_dir / "europepmc_fulltext_fetches.jsonl", fulltext_fetches)
    write_jsonl(out_dir / "europepmc_pair_evidence.jsonl", evidence)
    pair_status = pair_status_rows(pairs, query_rows, evidence)
    write_jsonl(out_dir / "europepmc_pair_status.jsonl", pair_status)
    candidate_status = candidate_status_rows(candidates, pair_status)
    write_jsonl(out_dir / "candidate_europepmc_status.jsonl", candidate_status)
    source_path = out_dir / "europepmc_pair_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(pair_status, evidence, source_path, source_sha)
    write_jsonl(out_dir / "europepmc_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(candidates, pairs, query_rows, evidence, fulltext_fetches, pair_status, candidate_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1243,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "europepmc_pair_query_responses": artifact(out_dir / "europepmc_pair_query_responses.jsonl", jsonl=True),
            "europepmc_fulltext_fetches": artifact(out_dir / "europepmc_fulltext_fetches.jsonl", jsonl=True),
            "europepmc_pair_evidence": artifact(out_dir / "europepmc_pair_evidence.jsonl", jsonl=True),
            "europepmc_pair_status": artifact(out_dir / "europepmc_pair_status.jsonl", jsonl=True),
            "candidate_europepmc_status": artifact(out_dir / "candidate_europepmc_status.jsonl", jsonl=True),
            "europepmc_bridge_rows": artifact(out_dir / "europepmc_bridge_rows.jsonl", jsonl=True),
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
        fulltext_fetches,
        pair_status,
        candidate_status,
        issue1242_persisted_readback,
        issue1242_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "candidate_europepmc_status": output_manifest["artifacts"]["candidate_europepmc_status"],
            "europepmc_pair_evidence": output_manifest["artifacts"]["europepmc_pair_evidence"],
            "bridge_rows": output_manifest["artifacts"]["europepmc_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1242-candidate-status")
    parser.add_argument("--issue1242-persisted-readback")
    parser.add_argument("--issue1242-calyx-readback")
    parser.add_argument("--issue1242-output-manifest")
    parser.add_argument("--max-pairs", type=int)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1242_candidate_status", "issue1242_candidate_status"),
        ("issue1242_persisted_readback", "issue1242_persisted_readback"),
        ("issue1242_calyx_readback", "issue1242_calyx_readback"),
        ("issue1242_output_manifest", "issue1242_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, args.max_pairs, args.request_sleep_seconds)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
