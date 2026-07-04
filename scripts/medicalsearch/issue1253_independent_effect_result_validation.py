#!/usr/bin/env python3
"""#1253 independent validation for #1252 effect-result candidates.

This stage reads sealed #1252 candidate rows and checks current independent
source surfaces for source-local pair evidence outside the #1251 source
windows. It is still source triage only: every row remains blocked until real
endpoint-result, safety/falsification, and human-review gates are proven.
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
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Independent effect-result validation is source triage only; not efficacy, "
    "safety, treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "independent_effect_result_source_triage_not_clinical_actionability"
)

ISSUE1252_ROOT = "/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1253-independent-effect-result-validation-20260704T202500Z"

DEFAULT_INPUTS = {
    "issue1252_rollup_status": f"{ISSUE1252_ROOT}/out/effect_result_rollup_status.jsonl",
    "issue1252_evidence_review": f"{ISSUE1252_ROOT}/out/effect_result_evidence_review.jsonl",
    "issue1252_persisted_readback": f"{ISSUE1252_ROOT}/out/persisted_readback.json",
    "issue1252_calyx_readback": f"{ISSUE1252_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1252_output_manifest": f"{ISSUE1252_ROOT}/out/output_manifest.json",
}

EUROPEPMC_SEARCH_ENDPOINT = "https://www.ebi.ac.uk/europepmc/webservices/rest/search"
EUROPEPMC_REST_DOCS_URL = "https://europepmc.org/RestfulWebService"
CLINICALTRIALS_API_ENDPOINT = "https://clinicaltrials.gov/api/v2/studies"
CLINICALTRIALS_OAS_URL = "https://clinicaltrials.gov/api/oas/v2"
PUBMED_ESEARCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi"
PUBMED_EFETCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi"
NCBI_EUTILITIES_DOC_URL = "https://www.ncbi.nlm.nih.gov/books/NBK25497/"

REQUEST_SLEEP_SECONDS = 0.15
USER_AGENT = "calyx-discovery/issue1253"
PUBMED_TOOL = "calyx"
PUBMED_EMAIL = "opensource@example.com"
PAGE_SIZE = 10

CANDIDATE_STATUSES = {
    "effect_result_candidate_with_magnitude_and_direction_still_blocked",
    "effect_result_candidate_direction_only_still_blocked",
}

PROMOTION_STATUS = (
    "blocked_requires_independent_endpoint_result_safety_falsification_and_human_review"
)

EVIDENCE_STATUS_VALUES = {
    "independent_counter_or_safety_blocks_still_blocked",
    "independent_result_language_hit_still_blocked",
    "independent_pair_context_without_result_assertion_still_blocked",
}

ROLLUP_STATUS_VALUES = {
    "independent_counter_or_safety_blocks_rollup",
    "independent_result_language_hit_still_blocked",
    "independent_pair_context_without_result_assertion_still_blocked",
    "same_source_only_no_independent_validation_still_blocked",
    "no_independent_endpoint_result_source_hit_still_blocked",
}

RESULT_ASSERTION_PATTERNS = [
    r"\bresults?\b",
    r"\bshow(?:ed|n|s)?\b",
    r"\bdemonstrat(?:ed|es|ing|ion)\b",
    r"\bobserv(?:ed|es|ation)\b",
    r"\bassociat(?:ed|ion)\b",
    r"\bindicat(?:ed|es|ing|ion)\b",
    r"\bfound\b",
    r"\breported\b",
    r"\brevealed\b",
    r"\bled to\b",
    r"\bresult(?:ed)? in\b",
    r"\bsignificant(?:ly)?\b",
    r"\btreated\b",
    r"\bresponse\b",
    r"\bsurvival\b",
    r"\bprogression\b",
    r"\befficacy\b",
    r"\beffective\b",
    r"\bimprov(?:ed|ement|es|ing)\b",
    r"\breduc(?:ed|tion|es|ing)\b",
    r"\bincreas(?:ed|e|es|ing)\b",
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
]

COUNTER_PATTERNS = [
    r"\bno significant\b",
    r"\bnot significant\b",
    r"\bdid not\b",
    r"\bfailed\b",
    r"\bfailure\b",
    r"\bwithout benefit\b",
    r"\black of\b",
    r"\bno benefit\b",
    r"\bno effect\b",
    r"\bnot effective\b",
    r"\bno response\b",
    r"\bnegative\b",
]

MAGNITUDE_PATTERNS = [
    r"\b\d+(?:\.\d+)?\s?%",
    r"\b\d+(?:\.\d+)?\s?(?:fold|x)\b",
    r"\b(?:p|P)\s?[<=>]\s?0?\.\d+\b",
    r"\bCI\b",
    r"\bhazard ratio\b",
    r"\bodds ratio\b",
]

COMPARATOR_PATTERNS = [
    r"\bcontrol(?:led)?\b",
    r"\bplacebo\b",
    r"\bcompared (?:to|with)\b",
    r"\bversus\b",
    r"\bvs\.?\b",
]


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


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
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


def pattern_hits(patterns: list[str], text: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for pattern in patterns:
        for match in re.finditer(pattern, text, flags=re.IGNORECASE):
            out.append(
                {
                    "pattern": pattern,
                    "match": clean_text(match.group(0)),
                    "start": match.start(),
                    "end": match.end(),
                }
            )
    return out


def fetch_bytes(url: str, retries: int = 4) -> tuple[int, bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=60) as response:
                return int(response.status), response.read()
        except urllib.error.HTTPError as error:
            return int(error.code), error.read()
        except (urllib.error.URLError, TimeoutError) as error:
            last_error = error
            time.sleep(min(8.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"Fetch failed after {retries} attempts for {url}: {last_error}")


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    sources = {
        "europepmc_rest_docs": (EUROPEPMC_REST_DOCS_URL, "europepmc_rest_docs.html"),
        "clinicaltrials_oas_v2": (CLINICALTRIALS_OAS_URL, "clinicaltrials_oas_v2.yaml"),
        "ncbi_eutilities_docs": (NCBI_EUTILITIES_DOC_URL, "ncbi_eutilities_docs.html"),
    }
    out: dict[str, dict[str, Any]] = {}
    for key, (url, filename) in sources.items():
        path = raw_dir / filename
        if not path.exists():
            status, payload = fetch_bytes(url)
            path.write_bytes(payload)
            (raw_dir / f"{filename}.status").write_text(str(status) + "\n", encoding="utf-8")
            time.sleep(REQUEST_SLEEP_SECONDS)
        out[key] = artifact(path, source_url=url)
    return out


def candidate_rows(rollups: list[dict[str, Any]], max_rollups: int | None = None) -> list[dict[str, Any]]:
    rows = [row for row in rollups if row.get("effect_result_gate_status") in CANDIDATE_STATUSES]
    rows.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_rollups is not None:
        rows = rows[:max_rollups]
    return rows


def pair_rows(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        grouped[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        source_ids: list[str] = []
        endpoint_terms: list[str] = []
        for member in members:
            source_ids.extend(member.get("source_ids", []))
            endpoint_terms.extend(member.get("endpoint_terms", []))
        out.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "representative_pair_id": first["pair_id"],
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query_drug_a": query_name(first["drug_a"]),
                "query_drug_b": query_name(first["drug_b"]),
                "candidate_rollup_ids": [row["rollup_result_gate_id"] for row in members],
                "candidate_pair_ids": [row["pair_id"] for row in members],
                "excluded_source_ids": uniq(source_ids),
                "endpoint_terms": uniq(endpoint_terms),
                "queryable": bool(query_name(first["drug_a"]) and query_name(first["drug_b"])),
            }
        )
    return out


def europepmc_search_url(left: str, right: str) -> tuple[str, str]:
    query = f'"{left}" AND "{right}"'
    params = {
        "query": query,
        "format": "json",
        "resultType": "core",
        "pageSize": str(PAGE_SIZE),
        "cursorMark": "*",
        "synonym": "false",
    }
    return query, f"{EUROPEPMC_SEARCH_ENDPOINT}?{urllib.parse.urlencode(params)}"


def clinicaltrials_search_url(left: str, right: str) -> tuple[str, str]:
    query = f"{left} {right}"
    params = {"format": "json", "pageSize": str(PAGE_SIZE), "query.term": query}
    return query, f"{CLINICALTRIALS_API_ENDPOINT}?{urllib.parse.urlencode(params)}"


def pubmed_esearch_url(left: str, right: str) -> tuple[str, str]:
    query = f'("{left}"[Title/Abstract] AND "{right}"[Title/Abstract])'
    params = {
        "db": "pubmed",
        "retmode": "json",
        "retmax": str(PAGE_SIZE),
        "tool": PUBMED_TOOL,
        "email": PUBMED_EMAIL,
        "term": query,
    }
    return query, f"{PUBMED_ESEARCH_URL}?{urllib.parse.urlencode(params)}"


def pubmed_efetch_url(pmids: list[str]) -> str:
    params = {
        "db": "pubmed",
        "retmode": "xml",
        "tool": PUBMED_TOOL,
        "email": PUBMED_EMAIL,
        "id": ",".join(pmids),
    }
    return f"{PUBMED_EFETCH_URL}?{urllib.parse.urlencode(params)}"


def result_list_europepmc(response_json: dict[str, Any]) -> list[dict[str, Any]]:
    result_list = response_json.get("resultList") or {}
    if not isinstance(result_list, dict):
        return []
    results = result_list.get("result") or []
    return [item for item in results if isinstance(item, dict)] if isinstance(results, list) else []


def flatten_strings(value: Any) -> list[str]:
    out: list[str] = []
    if isinstance(value, str):
        if value.strip():
            out.append(value)
    elif isinstance(value, list):
        for item in value:
            out.extend(flatten_strings(item))
    elif isinstance(value, dict):
        for item in value.values():
            out.extend(flatten_strings(item))
    return out


def parse_pubmed_articles(payload: bytes) -> list[dict[str, Any]]:
    if not payload:
        return []
    root = ET.fromstring(payload)
    rows: list[dict[str, Any]] = []
    for article in root.findall(".//PubmedArticle"):
        pmid = clean_text(article.findtext(".//PMID"))
        title = clean_text(" ".join(article.itertext()))[:20000]
        article_title = clean_text(article.findtext(".//ArticleTitle"))
        abstract_parts = [clean_text(node.text) for node in article.findall(".//AbstractText") if node.text]
        article_ids = []
        for node in article.findall(".//ArticleId"):
            id_type = node.attrib.get("IdType", "")
            text = clean_text(node.text)
            if text:
                article_ids.append({"type": id_type, "value": text})
        rows.append(
            {
                "source_id": pmid,
                "source_ids": uniq([pmid] + [item["value"] for item in article_ids]),
                "source_url": f"https://pubmed.ncbi.nlm.nih.gov/{pmid}/" if pmid else "",
                "title": article_title,
                "abstract": " ".join(abstract_parts),
                "source_text": f"{article_title} {' '.join(abstract_parts)}" if abstract_parts else title,
            }
        )
    return rows


def query_sources(pairs: list[dict[str, Any]], raw_dir: Path, max_pairs: int | None = None) -> list[dict[str, Any]]:
    selected = pairs[:max_pairs] if max_pairs is not None else pairs
    raw_dir.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(selected, start=1):
        if not pair["queryable"]:
            continue
        for source_system in ["europepmc", "clinicaltrials", "pubmed"]:
            if source_system == "europepmc":
                query, url = europepmc_search_url(pair["query_drug_a"], pair["query_drug_b"])
            elif source_system == "clinicaltrials":
                query, url = clinicaltrials_search_url(pair["query_drug_a"], pair["query_drug_b"])
            else:
                query, url = pubmed_esearch_url(pair["query_drug_a"], pair["query_drug_b"])
            status, payload = fetch_bytes(url)
            raw_path = raw_dir / f"{source_system}_{stable_id(pair['pair_key'], query)}.raw"
            raw_path.write_bytes(payload)
            response_json: dict[str, Any] = {}
            pmids: list[str] = []
            efetch_artifact: dict[str, Any] | None = None
            efetch_rows: list[dict[str, Any]] = []
            if source_system in {"europepmc", "clinicaltrials", "pubmed"}:
                try:
                    response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
                except json.JSONDecodeError:
                    response_json = {"decode_error": payload.decode("utf-8", errors="replace")[:1000]}
            if source_system == "pubmed":
                ids = response_json.get("esearchresult", {}).get("idlist", []) if isinstance(response_json, dict) else []
                pmids = [clean_text(item) for item in ids if clean_text(item)]
                if pmids:
                    efetch_url = pubmed_efetch_url(pmids)
                    efetch_status, efetch_payload = fetch_bytes(efetch_url)
                    efetch_path = raw_dir / f"pubmed_efetch_{stable_id(pair['pair_key'], ','.join(pmids))}.xml"
                    efetch_path.write_bytes(efetch_payload)
                    efetch_artifact = {
                        "source_url": efetch_url,
                        "http_status": efetch_status,
                        "path": str(efetch_path),
                        "bytes": len(efetch_payload),
                        "sha256": sha256_bytes(efetch_payload),
                    }
                    efetch_rows = parse_pubmed_articles(efetch_payload)
                    time.sleep(REQUEST_SLEEP_SECONDS)
            row = {
                "schema_version": 1,
                "source_system": source_system,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query": query,
                "query_url": url,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "response_json": response_json,
                "pubmed_pmids": pmids,
                "pubmed_efetch_artifact": efetch_artifact,
                "pubmed_efetch_rows": efetch_rows,
                "excluded_source_ids": pair["excluded_source_ids"],
                "endpoint_terms": pair["endpoint_terms"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            print(
                f"#1253 query {index}/{len(selected)} {source_system} pair={pair['pair_key']} bytes={len(payload)}",
                file=sys.stderr,
            )
            time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def source_id_set(values: list[object]) -> set[str]:
    out: set[str] = set()
    for value in values:
        text = clean_text(value)
        if not text:
            continue
        out.add(text.lower())
        if text.upper().startswith("PMC"):
            out.add(text[3:].lower())
    return out


def source_is_independent(source_ids: list[str], excluded: list[str]) -> bool:
    source_set = source_id_set(source_ids)
    excluded_set = source_id_set(excluded)
    return bool(source_set) and source_set.isdisjoint(excluded_set)


def bounded_text(value: str, limit: int = 2400) -> str:
    text = clean_text(value)
    return text[:limit]


def classify_source_text(text: str, candidate_endpoint_terms: list[str]) -> dict[str, Any]:
    result_hits = pattern_hits(RESULT_ASSERTION_PATTERNS, text)
    safety_hits = pattern_hits(SAFETY_PATTERNS, text)
    counter_hits = pattern_hits(COUNTER_PATTERNS, text)
    magnitude_hits = pattern_hits(MAGNITUDE_PATTERNS, text)
    comparator_hits = pattern_hits(COMPARATOR_PATTERNS, text)
    endpoint_hits = []
    for term in candidate_endpoint_terms:
        if term and term.lower() in text.lower():
            endpoint_hits.append(term)
    direction_present = bool(result_hits)
    has_safety_or_counter = bool(safety_hits or counter_hits)
    if has_safety_or_counter:
        status = "independent_counter_or_safety_blocks_still_blocked"
    elif direction_present:
        status = "independent_result_language_hit_still_blocked"
    else:
        status = "independent_pair_context_without_result_assertion_still_blocked"
    return {
        "independent_evidence_status": status,
        "has_result_assertion_language": direction_present,
        "has_safety_or_counter_language": has_safety_or_counter,
        "result_assertion_terms": uniq([hit["match"] for hit in result_hits]),
        "safety_terms": uniq([hit["match"] for hit in safety_hits]),
        "counter_or_negative_terms": uniq([hit["match"] for hit in counter_hits]),
        "effect_magnitude_terms": uniq([hit["match"] for hit in magnitude_hits]),
        "comparator_terms": uniq([hit["match"] for hit in comparator_hits]),
        "endpoint_terms": uniq(endpoint_hits),
        "result_assertion_spans": result_hits[:8],
        "safety_spans": safety_hits[:8],
        "counter_or_negative_spans": counter_hits[:8],
    }


def europepmc_evidence_items(row: dict[str, Any]) -> list[dict[str, Any]]:
    results = result_list_europepmc(row.get("response_json") or {})
    out: list[dict[str, Any]] = []
    for item in results:
        pmid = clean_text(item.get("pmid"))
        pmcid = clean_text(item.get("pmcid"))
        source_ids = uniq([pmcid, pmid, item.get("id")])
        text = " ".join(
            clean_text(item.get(key))
            for key in ["title", "abstractText", "journalTitle", "authorString", "pubYear"]
        )
        out.append(
            {
                "source_system": "europepmc",
                "source_id": pmcid or pmid or clean_text(item.get("id")),
                "source_ids": source_ids,
                "source_url": f"https://europepmc.org/article/MED/{pmid}" if pmid else "",
                "source_title": clean_text(item.get("title")),
                "source_text": text,
            }
        )
    return out


def clinicaltrials_evidence_items(row: dict[str, Any]) -> list[dict[str, Any]]:
    studies = (row.get("response_json") or {}).get("studies") or []
    out: list[dict[str, Any]] = []
    for study in studies if isinstance(studies, list) else []:
        protocol = study.get("protocolSection", {}) if isinstance(study, dict) else {}
        identification = protocol.get("identificationModule", {}) if isinstance(protocol, dict) else {}
        nct_id = clean_text(identification.get("nctId"))
        brief_title = clean_text(identification.get("briefTitle"))
        text = " ".join(flatten_strings(study))
        out.append(
            {
                "source_system": "clinicaltrials",
                "source_id": nct_id,
                "source_ids": [nct_id] if nct_id else [],
                "source_url": f"https://clinicaltrials.gov/study/{nct_id}" if nct_id else "",
                "source_title": brief_title,
                "source_text": text,
            }
        )
    return out


def pubmed_evidence_items(row: dict[str, Any]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for item in row.get("pubmed_efetch_rows", []):
        out.append(
            {
                "source_system": "pubmed",
                "source_id": clean_text(item.get("source_id")),
                "source_ids": item.get("source_ids", []),
                "source_url": clean_text(item.get("source_url")),
                "source_title": clean_text(item.get("title")),
                "source_text": clean_text(item.get("source_text")),
            }
        )
    return out


def query_evidence_items(row: dict[str, Any]) -> list[dict[str, Any]]:
    if row["source_system"] == "europepmc":
        return europepmc_evidence_items(row)
    if row["source_system"] == "clinicaltrials":
        return clinicaltrials_evidence_items(row)
    if row["source_system"] == "pubmed":
        return pubmed_evidence_items(row)
    return []


def build_evidence_rows(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence_rows: list[dict[str, Any]] = []
    seen: set[tuple[str, str, str]] = set()
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        for item in query_evidence_items(query):
            source_text = bounded_text(item["source_text"])
            left = exact_presence(source_text, pair["drug_a"])
            right = exact_presence(source_text, pair["drug_b"])
            if not (left["present"] and right["present"]):
                continue
            independent = source_is_independent(item.get("source_ids", []), pair["excluded_source_ids"])
            if not independent:
                continue
            dedupe_key = (query["pair_key"], item["source_system"], item["source_id"])
            if dedupe_key in seen:
                continue
            seen.add(dedupe_key)
            classified = classify_source_text(source_text, pair["endpoint_terms"])
            row = {
                "schema_version": 1,
                "independent_review_id": "independent-effect-evidence:"
                + stable_id(query["pair_key"], item["source_system"], item["source_id"]),
                "pair_key": query["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "source_system": item["source_system"],
                "source_id": item["source_id"],
                "source_ids": item.get("source_ids", []),
                "source_url": item["source_url"],
                "source_title": item["source_title"],
                "source_text_window": source_text,
                "source_text_sha256": hashlib.sha256(source_text.encode("utf-8")).hexdigest(),
                "query_url": query["query_url"],
                "raw_response_sha256": query["raw_response_sha256"],
                "raw_response_path": query["raw_response_path"],
                "pair_term_presence": {
                    "left": left,
                    "right": right,
                    "both_present_in_source_text": left["present"] and right["present"],
                    "both_exact_in_source_text": left["exact"] and right["exact"],
                },
                "independent_from_issue1251_source_ids": independent,
                "excluded_issue1251_source_ids": pair["excluded_source_ids"],
                "candidate_endpoint_terms": pair["endpoint_terms"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "independent_source_text_triage_not_clinical_actionability",
                    "requires_real_endpoint_result_safety_falsification_and_human_review",
                ],
            }
            row.update(classified)
            evidence_rows.append(row)
    evidence_rows.sort(key=lambda row: (row["pair_key"], row["source_system"], row["source_id"]))
    return evidence_rows


def query_same_source_overlap_counts(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> dict[str, int]:
    counts: Counter[str] = Counter()
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        for item in query_evidence_items(query):
            source_text = bounded_text(item["source_text"])
            left = exact_presence(source_text, pair["drug_a"])
            right = exact_presence(source_text, pair["drug_b"])
            if not (left["present"] and right["present"]):
                continue
            if not source_is_independent(item.get("source_ids", []), pair["excluded_source_ids"]):
                counts[query["pair_key"]] += 1
    return dict(counts)


def rollup_reason_codes(status: str, evidence: list[dict[str, Any]]) -> list[str]:
    codes = [
        "independent_validation_not_clinical_actionability",
        "requires_real_endpoint_result_safety_falsification_and_human_review",
    ]
    if status == "independent_counter_or_safety_blocks_rollup":
        codes.append("independent_counter_or_safety_language_blocks")
    elif status == "independent_result_language_hit_still_blocked":
        codes.append("independent_result_language_requires_manual_endpoint_review")
    elif status == "independent_pair_context_without_result_assertion_still_blocked":
        codes.append("independent_pair_context_lacks_result_assertion")
    elif status == "same_source_only_no_independent_validation_still_blocked":
        codes.append("only_same_issue1251_source_overlap_seen")
    else:
        codes.append("no_independent_pair_endpoint_result_source_hit")
    if any(row.get("effect_magnitude_terms") for row in evidence):
        codes.append("magnitude_language_present_but_not_validated")
    return codes


def build_rollup_status(
    candidates: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    same_source_counts: dict[str, int],
) -> list[dict[str, Any]]:
    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_pair[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for candidate in candidates:
        rows = evidence_by_pair.get(candidate["pair_key"], [])
        status = "no_independent_endpoint_result_source_hit_still_blocked"
        if any(row["independent_evidence_status"] == "independent_counter_or_safety_blocks_still_blocked" for row in rows):
            status = "independent_counter_or_safety_blocks_rollup"
        elif any(row["independent_evidence_status"] == "independent_result_language_hit_still_blocked" for row in rows):
            status = "independent_result_language_hit_still_blocked"
        elif rows:
            status = "independent_pair_context_without_result_assertion_still_blocked"
        elif same_source_counts.get(candidate["pair_key"], 0):
            status = "same_source_only_no_independent_validation_still_blocked"
        out.append(
            {
                "schema_version": 1,
                "independent_rollup_status_id": "independent-effect-rollup:"
                + stable_id(candidate["rollup_result_gate_id"], candidate["pair_key"]),
                "source_issue1252_rollup_result_gate_id": candidate["rollup_result_gate_id"],
                "source_issue1252_pair_id": candidate["pair_id"],
                "pair_key": candidate["pair_key"],
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "source_issue1252_gate_status": candidate["effect_result_gate_status"],
                "source_issue1251_source_ids": candidate.get("source_ids", []),
                "independent_validation_status": status,
                "independent_evidence_review_ids": [row["independent_review_id"] for row in rows],
                "independent_evidence_count": len(rows),
                "same_source_overlap_count": same_source_counts.get(candidate["pair_key"], 0),
                "source_system_counts": dict(sorted(Counter(row["source_system"] for row in rows).items())),
                "effect_magnitude_terms": uniq(sum((row.get("effect_magnitude_terms", []) for row in rows), [])),
                "comparator_terms": uniq(sum((row.get("comparator_terms", []) for row in rows), [])),
                "safety_terms": uniq(sum((row.get("safety_terms", []) for row in rows), [])),
                "counter_or_negative_terms": uniq(sum((row.get("counter_or_negative_terms", []) for row in rows), [])),
                "reason_codes": rollup_reason_codes(status, rows),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return out


def build_bridge_rows(
    rollup_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        rows.append(
            {
                "id": row["independent_rollup_status_id"],
                "domain": "independent_effect_result_rollup",
                "text": (
                    f"Independent effect-result validation rollup {row['source_issue1252_pair_id']} "
                    f"pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} status "
                    f"{row['independent_validation_status']} evidence {row['independent_evidence_count']} "
                    f"promotion {row['promotion_status']}."
                ),
                "bridge_terms": uniq(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["independent_validation_status"],
                    ]
                    + row.get("safety_terms", [])
                    + row.get("counter_or_negative_terms", [])
                ),
                "metadata": {
                    "source_dataset": "issue1253_independent_effect_result_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "independent_validation_status": row["independent_validation_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in evidence_rows:
        rows.append(
            {
                "id": row["independent_review_id"],
                "domain": "independent_effect_result_evidence",
                "text": (
                    f"Independent evidence {row['source_system']} {row['source_id']} for pair "
                    f"{row['pair_key']} status {row['independent_evidence_status']} title "
                    f"{row['source_title']}."
                ),
                "bridge_terms": uniq(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["source_system"],
                        row["source_id"],
                        row["independent_evidence_status"],
                    ]
                    + row.get("result_assertion_terms", [])
                    + row.get("safety_terms", [])
                    + row.get("counter_or_negative_terms", [])
                ),
                "metadata": {
                    "source_dataset": "issue1253_independent_effect_result_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_system": row["source_system"],
                    "source_id": row["source_id"],
                    "independent_evidence_status": row["independent_evidence_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in query_rows:
        rows.append(
            {
                "id": "independent-effect-query:" + stable_id(row["source_system"], row["pair_key"]),
                "domain": "independent_effect_result_query",
                "text": (
                    f"Independent source query {row['source_system']} for pair {row['pair_key']} "
                    f"{row['drug_a']} plus {row['drug_b']} HTTP {row['http_status']}."
                ),
                "bridge_terms": uniq(
                    [
                        row["pair_key"],
                        row["drug_a"],
                        row["drug_b"],
                        row["source_system"],
                        str(row["http_status"]),
                    ]
                ),
                "metadata": {
                    "source_dataset": "issue1253_independent_effect_result_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_system": row["source_system"],
                    "http_status": str(row["http_status"]),
                    "raw_response_sha256": row["raw_response_sha256"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def build_metrics(
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rollups": len(candidates),
        "unique_candidate_pairs": len(pairs),
        "query_response_rows": len(query_rows),
        "independent_evidence_rows": len(evidence_rows),
        "rollup_status_rows": len(rollup_status),
        "query_source_counts": dict(sorted(Counter(row["source_system"] for row in query_rows).items())),
        "evidence_source_counts": dict(sorted(Counter(row["source_system"] for row in evidence_rows).items())),
        "evidence_status_counts": dict(sorted(Counter(row["independent_evidence_status"] for row in evidence_rows).items())),
        "rollup_status_counts": dict(sorted(Counter(row["independent_validation_status"] for row in rollup_status).items())),
        "rollups_with_independent_result_language": sum(
            1 for row in rollup_status if row["independent_validation_status"] == "independent_result_language_hit_still_blocked"
        ),
        "rollups_with_independent_safety_or_counter": sum(
            1 for row in rollup_status if row["independent_validation_status"] == "independent_counter_or_safety_blocks_rollup"
        ),
        "all_rows_blocked": True,
        "top_rollups": [
            {
                "pair_id": row["source_issue1252_pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["independent_validation_status"],
                "independent_evidence_count": row["independent_evidence_count"],
                "source_system_counts": row["source_system_counts"],
            }
            for row in rollup_status[:50]
        ],
    }


def build_input_manifest(
    inputs: dict[str, str],
    rollups: list[dict[str, Any]],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    raw_sources: dict[str, dict[str, Any]],
    issue1252_persisted_readback: dict[str, Any],
    issue1252_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1253,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1252_rollup_status": artifact(Path(inputs["issue1252_rollup_status"]), jsonl=True),
            "issue1252_evidence_review": artifact(Path(inputs["issue1252_evidence_review"]), jsonl=True),
            "issue1252_persisted_readback": artifact(Path(inputs["issue1252_persisted_readback"])),
            "issue1252_calyx_readback": artifact(Path(inputs["issue1252_calyx_readback"])),
            "issue1252_output_manifest": artifact(Path(inputs["issue1252_output_manifest"])),
        },
        "raw_source_docs": raw_sources,
        "source_contract": {
            "issue1252_persisted_assertions_all_true": all_assertions_true(issue1252_persisted_readback),
            "issue1252_calyx_assertions_all_true": all_assertions_true(issue1252_calyx_readback),
            "issue1252_rollup_rows": len(rollups),
            "candidate_rollups": len(candidates),
            "unique_candidate_pairs": len(pairs),
            "input_filter": f"effect_result_gate_status in {sorted(CANDIDATE_STATUSES)}",
            "independent_sources_required": True,
            "result_fields_are_triage_not_claims": True,
        },
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    issue1252_persisted_readback: dict[str, Any],
    issue1252_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "independent_source_query_responses": artifact(out_dir / "independent_source_query_responses.jsonl", jsonl=True),
        "independent_effect_evidence_review": artifact(out_dir / "independent_effect_evidence_review.jsonl", jsonl=True),
        "independent_effect_rollup_status": artifact(out_dir / "independent_effect_rollup_status.jsonl", jsonl=True),
        "independent_effect_bridge_rows": artifact(out_dir / "independent_effect_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    candidate_ids = {row["rollup_result_gate_id"] for row in candidates}
    status_ids = {row["source_issue1252_rollup_result_gate_id"] for row in rollup_status}
    query_keys = {(row["pair_key"], row["source_system"]) for row in query_rows}
    expected_query_keys = {(pair["pair_key"], source) for pair in pairs for source in ["europepmc", "clinicaltrials", "pubmed"]}
    return {
        "schema_version": 1,
        "issue": 1253,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1252_persisted_readback_all_true": all_assertions_true(issue1252_persisted_readback),
            "issue1252_calyx_readback_all_true": all_assertions_true(issue1252_calyx_readback),
            "rollup_status_for_every_candidate_rollup": status_ids == candidate_ids,
            "query_response_for_every_pair_and_source": query_keys == expected_query_keys,
            "all_evidence_status_values_allowed": all(
                row["independent_evidence_status"] in EVIDENCE_STATUS_VALUES for row in evidence_rows
            ),
            "all_rollup_status_values_allowed": all(
                row["independent_validation_status"] in ROLLUP_STATUS_VALUES for row in rollup_status
            ),
            "all_independent_evidence_has_source_provenance": all(
                row["source_id"]
                and row["source_url"]
                and row["source_text_sha256"]
                and row["raw_response_sha256"]
                and row["raw_response_path"]
                for row in evidence_rows
            ),
            "all_independent_evidence_has_pair_terms": all(
                row["pair_term_presence"]["both_present_in_source_text"] for row in evidence_rows
            ),
            "all_independent_evidence_excludes_issue1251_sources": all(
                row["independent_from_issue1251_source_ids"] for row in evidence_rows
            ),
            "all_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence_rows)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in query_rows),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_rows)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status),
            "bridge_rows_1000_or_less": artifacts["independent_effect_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "candidate_rollups": len(candidates),
            "unique_candidate_pairs": len(pairs),
            "query_response_rows": len(query_rows),
            "independent_evidence_rows": len(evidence_rows),
            "rollup_status_rows": len(rollup_status),
        },
    }


def run(
    root: Path,
    inputs: dict[str, str],
    *,
    max_rollups: int | None = None,
    max_pairs: int | None = None,
) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    raw_dir = root / "raw"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    rollups = rows_jsonl(Path(inputs["issue1252_rollup_status"]))
    issue1252_persisted_readback = read_json(Path(inputs["issue1252_persisted_readback"]))
    issue1252_calyx_readback = read_json(Path(inputs["issue1252_calyx_readback"]))
    candidates = candidate_rows(rollups, max_rollups=max_rollups)
    pairs = pair_rows(candidates)
    if max_pairs is not None:
        selected_pair_keys = {pair["pair_key"] for pair in pairs[:max_pairs]}
        pairs = [pair for pair in pairs if pair["pair_key"] in selected_pair_keys]
        candidates = [row for row in candidates if row["pair_key"] in selected_pair_keys]
    pair_lookup = {row["pair_key"]: row for row in pairs}
    raw_sources = fetch_raw_sources(raw_dir)
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            rollups,
            candidates,
            pairs,
            raw_sources,
            issue1252_persisted_readback,
            issue1252_calyx_readback,
        ),
    )
    query_rows = query_sources(pairs, raw_dir)
    write_jsonl(out_dir / "independent_source_query_responses.jsonl", query_rows)
    evidence_rows = build_evidence_rows(query_rows, pair_lookup)
    write_jsonl(out_dir / "independent_effect_evidence_review.jsonl", evidence_rows)
    same_source_counts = query_same_source_overlap_counts(query_rows, pair_lookup)
    rollup_status = build_rollup_status(candidates, evidence_rows, same_source_counts)
    write_jsonl(out_dir / "independent_effect_rollup_status.jsonl", rollup_status)
    source_path = out_dir / "independent_effect_rollup_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, evidence_rows, query_rows, source_path, source_sha)
    write_jsonl(out_dir / "independent_effect_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(candidates, pairs, query_rows, evidence_rows, rollup_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1253,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "independent_source_query_responses": artifact(
                out_dir / "independent_source_query_responses.jsonl", jsonl=True
            ),
            "independent_effect_evidence_review": artifact(
                out_dir / "independent_effect_evidence_review.jsonl", jsonl=True
            ),
            "independent_effect_rollup_status": artifact(
                out_dir / "independent_effect_rollup_status.jsonl", jsonl=True
            ),
            "independent_effect_bridge_rows": artifact(
                out_dir / "independent_effect_bridge_rows.jsonl", jsonl=True
            ),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        candidates,
        pairs,
        query_rows,
        evidence_rows,
        rollup_status,
        issue1252_persisted_readback,
        issue1252_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    if not all(readback["assertions"].values()):
        raise AssertionError(f"Persisted readback assertions failed: {readback['assertions']}")
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "query_responses": output_manifest["artifacts"]["independent_source_query_responses"],
            "evidence_review": output_manifest["artifacts"]["independent_effect_evidence_review"],
            "rollup_status": output_manifest["artifacts"]["independent_effect_rollup_status"],
            "bridge_rows": output_manifest["artifacts"]["independent_effect_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1252-rollup-status")
    parser.add_argument("--issue1252-evidence-review")
    parser.add_argument("--issue1252-persisted-readback")
    parser.add_argument("--issue1252-calyx-readback")
    parser.add_argument("--issue1252-output-manifest")
    parser.add_argument("--max-rollups", type=int)
    parser.add_argument("--max-pairs", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1252_rollup_status", "issue1252_rollup_status"),
        ("issue1252_evidence_review", "issue1252_evidence_review"),
        ("issue1252_persisted_readback", "issue1252_persisted_readback"),
        ("issue1252_calyx_readback", "issue1252_calyx_readback"),
        ("issue1252_output_manifest", "issue1252_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_rollups=args.max_rollups, max_pairs=args.max_pairs)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
