#!/usr/bin/env python3
"""#1237 PubMed source-text validation for #1234 co-mention hits.

This stage fetches PubMed EFetch XML for #1234 PubMed evidence PMIDs, extracts
title/abstract source text, verifies that both candidate drug names physically
occur in that title/abstract text, and classifies the observed relation with a
deterministic conservative rule set.

The output is still research triage only. It is not efficacy, safety, treatment
guidance, dosing, recommendation, clinical actionability, or cure evidence.
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
    "PubMed source-text validation is literature triage only; not efficacy, "
    "safety, clinical actionability, treatment guidance, dosing, "
    "recommendation, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "pubmed_title_abstract_source_text_relation_validation_not_clinical_clearance"
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1237-pubmed-source-text-validation-20260704T173000Z"
ISSUE1234_ROOT = "/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z"

DEFAULT_INPUTS = {
    "issue1234_pubmed_evidence": f"{ISSUE1234_ROOT}/out/pubmed_pair_literature_evidence.jsonl",
    "issue1234_candidate_hits": f"{ISSUE1234_ROOT}/out/candidate_external_source_hits.jsonl",
    "issue1234_pubmed_esearch": f"{ISSUE1234_ROOT}/out/pubmed_esearch_responses.jsonl",
    "issue1234_pubmed_esummary": f"{ISSUE1234_ROOT}/out/pubmed_esummary_responses.jsonl",
    "issue1234_persisted_readback": f"{ISSUE1234_ROOT}/out/persisted_readback.json",
    "issue1234_calyx_readback": f"{ISSUE1234_ROOT}/out/calyx_bridge_corpus_readback.json",
    "ncbi_eutilities_intro": f"{ISSUE1234_ROOT}/raw/ncbi_eutilities_intro.html",
    "nlm_eutilities_guide": f"{ISSUE1234_ROOT}/raw/nlm_eutilities_guide.html",
    "ncbi_eutilities_parameters": f"{DEFAULT_ROOT}/raw/ncbi_eutilities_parameters.html",
}

NCBI_EUTILITIES_INTRO_URL = "https://www.ncbi.nlm.nih.gov/books/NBK25497/"
NCBI_EUTILITIES_PARAMETERS_URL = "https://www.ncbi.nlm.nih.gov/books/NBK25499/"
NLM_EUTILITIES_GUIDE_URL = "https://www.nlm.nih.gov/dataguide/eutilities/utilities.html"
PUBMED_EFETCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi"
PUBMED_RECORD_URL = "https://pubmed.ncbi.nlm.nih.gov/{pmid}/"

PUBMED_TOOL = "calyx"
PUBMED_EMAIL = "opensource@example.com"
REQUEST_SLEEP_SECONDS = 0.40
EFETCH_CHUNK_SIZE = 150
USER_AGENT = "calyx-discovery/issue1237"

RELATION_RANK = {
    "insufficient_text": 0,
    "co_mention_only": 1,
    "asserted_outcome": 2,
    "asserted_combination": 3,
    "asserted_interaction": 4,
    "counter_evidence": 5,
}

COUNTER_PATTERNS = [
    r"\bcontraindicat",
    r"\badverse\b",
    r"\btoxicit",
    r"\btoxic\b",
    r"\bfatal\b",
    r"\bdeath\b",
    r"\bmortality\b",
    r"\bharm\b",
    r"\brisk\b",
    r"\bhepatotoxic",
    r"\bnephrotoxic",
    r"\barrhythm",
    r"\bavoid\b",
    r"\bnot recommended\b",
    r"\bno significant\b",
    r"\bfailed\b",
    r"\bdid not\b",
    r"\bwithout benefit\b",
]

INTERACTION_PATTERNS = [
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
]

COMBINATION_PATTERNS = [
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
]

OUTCOME_PATTERNS = [
    r"\btreat",
    r"\btherapy\b",
    r"\btherapeutic",
    r"\befficacy\b",
    r"\beffective",
    r"\bresponse\b",
    r"\bsurvival\b",
    r"\bimprov",
    r"\btrial\b",
    r"\bpatients?\b",
    r"\bcase report\b",
    r"\bclinical\b",
    r"\boutcome\b",
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


def exact_name_presence(text: str, name: str) -> dict[str, Any]:
    raw = clean_text(name)
    raw_present = bool(raw and raw.lower() in text.lower())
    norm = normalized_phrase(raw)
    norm_present = bool(norm and f" {norm} " in normalized_blob(text))
    return {
        "name": raw,
        "normalized_name": norm,
        "raw_case_insensitive_substring": raw_present,
        "normalized_token_sequence": norm_present,
        "present": raw_present or norm_present,
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
                    "remediation": "persist #1234 artifacts and E-utilities documentation pages before running #1237",
                },
                indent=2,
            )
        )


def fetch_bytes(url: str, retries: int = 5) -> bytes:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=75) as response:
                return response.read()
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            last_error = error
            if isinstance(error, urllib.error.HTTPError) and error.code in {400, 404}:
                raise
            time.sleep(min(10.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"fetch failed after {retries} attempts: {last_error}")


def pubmed_query_url(params: dict[str, str]) -> str:
    return f"{PUBMED_EFETCH_URL}?{urllib.parse.urlencode(params)}"


def chunked(values: list[str], size: int) -> list[list[str]]:
    return [values[index : index + size] for index in range(0, len(values), size)]


def cached_jsonl_exact(path: Path, expected_keys: set[str], key_field: str) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    rows = rows_jsonl(path)
    observed = {row.get(key_field) for row in rows}
    if observed == expected_keys:
        return rows
    return None


def unique_pmids(evidence_rows: list[dict[str, Any]]) -> list[str]:
    return sorted({str(row["pmid"]) for row in evidence_rows if clean_text(row.get("pmid"))}, key=int)


def fetch_pubmed_efetch(
    pmids: list[str],
    out_path: Path,
    *,
    chunk_size: int,
    request_sleep_seconds: float,
    email: str,
) -> list[dict[str, Any]]:
    expected_keys = {",".join(chunk) for chunk in chunked(pmids, chunk_size)}
    cached = cached_jsonl_exact(out_path, expected_keys, "pmid_chunk_key")
    if cached is not None:
        return cached
    tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
    rows: list[dict[str, Any]] = []
    out_path.parent.mkdir(parents=True, exist_ok=True)
    chunks = chunked(pmids, chunk_size)
    with tmp_path.open("w", encoding="utf-8") as handle:
        for chunk_index, ids in enumerate(chunks, start=1):
            params = {
                "db": "pubmed",
                "retmode": "xml",
                "id": ",".join(ids),
                "tool": PUBMED_TOOL,
                "email": email,
            }
            url = pubmed_query_url(params)
            payload = fetch_bytes(url)
            row = {
                "schema_version": 1,
                "chunk_index": chunk_index,
                "pmid_chunk_key": ",".join(ids),
                "pmids": ids,
                "query_url": url,
                "api_base": PUBMED_EFETCH_URL,
                "retmode": "xml",
                "response_bytes": len(payload),
                "response_sha256": sha256_bytes(payload),
                "response_xml": payload.decode("utf-8", errors="replace"),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            handle.write(json.dumps(row, sort_keys=True) + "\n")
            handle.flush()
            print(f"fetched PubMed EFetch chunk {chunk_index}/{len(chunks)}", file=sys.stderr)
            time.sleep(request_sleep_seconds)
    os.replace(tmp_path, out_path)
    return rows


def itertext(element: ET.Element | None) -> str:
    if element is None:
        return ""
    return clean_text(" ".join(element.itertext()))


def first_text(root: ET.Element, path: str) -> str:
    element = root.find(path)
    return itertext(element)


def find_all_text(root: ET.Element, path: str) -> list[str]:
    return [itertext(element) for element in root.findall(path) if itertext(element)]


def pubdate(article: ET.Element) -> str:
    pub_date = article.find(".//JournalIssue/PubDate")
    if pub_date is None:
        return ""
    year = first_text(pub_date, "Year")
    month = first_text(pub_date, "Month")
    day = first_text(pub_date, "Day")
    medline = first_text(pub_date, "MedlineDate")
    return clean_text(" ".join(part for part in [year, month, day, medline] if part))


def article_ids(article: ET.Element) -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    for item in article.findall(".//PubmedData/ArticleIdList/ArticleId"):
        value = itertext(item)
        if not value:
            continue
            out.append({"idtype": item.attrib.get("IdType", ""), "value": value})
    return out


def book_article_ids(article: ET.Element) -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    for item in article.findall(".//BookDocument/ArticleIdList/ArticleId") + article.findall(
        ".//PubmedBookData/ArticleIdList/ArticleId"
    ):
        value = itertext(item)
        if not value:
            continue
        out.append({"idtype": item.attrib.get("IdType", ""), "value": value})
    return out


def parse_source_records(efetch_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    records: dict[str, dict[str, Any]] = {}
    for raw in efetch_rows:
        root = ET.fromstring(raw["response_xml"])
        for article in root.findall(".//PubmedArticle"):
            pmid = first_text(article, ".//MedlineCitation/PMID")
            if not pmid:
                continue
            abstract_parts = find_all_text(article, ".//Article/Abstract/AbstractText")
            other_abstract_parts = find_all_text(article, ".//Article/OtherAbstract/AbstractText")
            title = first_text(article, ".//Article/ArticleTitle")
            record = {
                "schema_version": 1,
                "pmid": pmid,
                "source_url": PUBMED_RECORD_URL.format(pmid=pmid),
                "title": title,
                "abstract_text": clean_text(" ".join(abstract_parts)),
                "other_abstract_text": clean_text(" ".join(other_abstract_parts)),
                "journal_title": first_text(article, ".//Journal/Title"),
                "journal_iso_abbreviation": first_text(article, ".//Journal/ISOAbbreviation"),
                "pubdate": pubdate(article),
                "publication_types": find_all_text(article, ".//PublicationTypeList/PublicationType"),
                "mesh_terms": find_all_text(article, ".//MeshHeading/DescriptorName"),
                "article_ids": article_ids(article),
                "source_text_sha256": "",
                "efetch_response_sha256": raw["response_sha256"],
                "efetch_chunk_index": raw["chunk_index"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            source_text = source_text_for_record(record)
            record["source_text_sha256"] = sha256_bytes(source_text.encode("utf-8"))
            record["source_text_bytes"] = len(source_text.encode("utf-8"))
            records[pmid] = record
        for article in root.findall(".//PubmedBookArticle"):
            pmid = first_text(article, ".//BookDocument/PMID")
            if not pmid:
                continue
            abstract_parts = find_all_text(article, ".//BookDocument/Abstract/AbstractText")
            title = first_text(article, ".//BookDocument/ArticleTitle")
            record = {
                "schema_version": 1,
                "pmid": pmid,
                "record_type": "PubmedBookArticle",
                "source_url": PUBMED_RECORD_URL.format(pmid=pmid),
                "title": title,
                "abstract_text": clean_text(" ".join(abstract_parts)),
                "other_abstract_text": "",
                "journal_title": first_text(article, ".//BookDocument/Book/BookTitle"),
                "journal_iso_abbreviation": first_text(article, ".//BookDocument/Book/BookTitle"),
                "pubdate": first_text(article, ".//PubmedBookData/History/PubMedPubDate/Year"),
                "publication_types": ["PubmedBookArticle"],
                "mesh_terms": [],
                "article_ids": book_article_ids(article),
                "source_text_sha256": "",
                "efetch_response_sha256": raw["response_sha256"],
                "efetch_chunk_index": raw["chunk_index"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            source_text = source_text_for_record(record)
            record["source_text_sha256"] = sha256_bytes(source_text.encode("utf-8"))
            record["source_text_bytes"] = len(source_text.encode("utf-8"))
            records[pmid] = record
    return [records[pmid] for pmid in sorted(records, key=int)]


def source_text_for_record(record: dict[str, Any]) -> str:
    return clean_text(
        " ".join(
            [
                record.get("title") or "",
                record.get("abstract_text") or "",
                record.get("other_abstract_text") or "",
            ]
        )
    )


def matched_patterns(patterns: list[str], text: str) -> list[str]:
    lowered = text.lower()
    return [pattern for pattern in patterns if re.search(pattern, lowered)]


def classify_relation(text: str, left_present: bool, right_present: bool) -> tuple[str, list[str]]:
    if not text or not (left_present and right_present):
        return "insufficient_text", []
    counter = matched_patterns(COUNTER_PATTERNS, text)
    if counter:
        return "counter_evidence", counter
    interaction = matched_patterns(INTERACTION_PATTERNS, text)
    if interaction:
        return "asserted_interaction", interaction
    combination = matched_patterns(COMBINATION_PATTERNS, text)
    if combination:
        return "asserted_combination", combination
    outcome = matched_patterns(OUTCOME_PATTERNS, text)
    if outcome:
        return "asserted_outcome", outcome
    return "co_mention_only", []


def validate_evidence_rows(
    evidence_rows: list[dict[str, Any]], source_records: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    records_by_pmid = {row["pmid"]: row for row in source_records}
    validations: list[dict[str, Any]] = []
    for row in evidence_rows:
        pmid = str(row["pmid"])
        record = records_by_pmid.get(pmid)
        source_text = source_text_for_record(record) if record else ""
        left_presence = exact_name_presence(source_text, row["drug_a"])
        right_presence = exact_name_presence(source_text, row["drug_b"])
        relation, patterns = classify_relation(source_text, left_presence["present"], right_presence["present"])
        source_text_status = "both_candidate_names_present" if relation != "insufficient_text" else "insufficient_text"
        validations.append(
            {
                "schema_version": 1,
                "validation_id": f"pubmed-validation:{stable_id(row['evidence_id'], pmid)}",
                "source_evidence_id": row["evidence_id"],
                "pair_key": row["pair_key"],
                "representative_pair_id": row["representative_pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "pmid": pmid,
                "source_url": PUBMED_RECORD_URL.format(pmid=pmid),
                "source_record_present": record is not None,
                "source_text_status": source_text_status,
                "source_text_sha256": record.get("source_text_sha256") if record else None,
                "source_text_bytes": record.get("source_text_bytes") if record else 0,
                "title": record.get("title") if record else "",
                "pubdate": record.get("pubdate") if record else "",
                "journal": record.get("journal_iso_abbreviation") if record else "",
                "publication_types": record.get("publication_types") if record else [],
                "drug_a_presence": left_presence,
                "drug_b_presence": right_presence,
                "relation_class": relation,
                "matched_rule_patterns": patterns,
                "validation_status": (
                    "source_text_validated_still_blocked"
                    if relation != "insufficient_text"
                    else "query_hit_not_validated_by_source_text"
                ),
                "reason_codes": reason_codes_for_relation(relation),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": next_validation_for_relation(relation),
            }
        )
    validations.sort(key=lambda item: (item["pair_key"], item["pmid"], item["source_evidence_id"]))
    return validations


def reason_codes_for_relation(relation: str) -> list[str]:
    base = ["pubmed_query_hit_requires_downstream_gates"]
    if relation == "insufficient_text":
        return base + ["source_text_exact_candidate_names_missing_fail_closed"]
    if relation == "counter_evidence":
        return base + ["counter_evidence_review_required"]
    if relation == "asserted_interaction":
        return base + ["asserted_interaction_not_safety_or_outcome_clearance"]
    if relation == "asserted_combination":
        return base + ["asserted_combination_not_efficacy_or_safety_clearance"]
    if relation == "asserted_outcome":
        return base + ["outcome_language_requires_result_extraction_and_safety_gates"]
    return base + ["co_mention_only_not_asserted_relation"]


def next_validation_for_relation(relation: str) -> str:
    if relation == "insufficient_text":
        return "Do not use this query-level hit until source text or another source validates both candidate names."
    if relation == "counter_evidence":
        return "Route to falsification/safety review before any hypothesis promotion."
    if relation == "asserted_interaction":
        return "Extract structured interaction direction, dose/context, safety, and outcome evidence."
    if relation == "asserted_combination":
        return "Extract structured combination context, then require safety and outcome gates."
    if relation == "asserted_outcome":
        return "Extract outcome endpoint/result and run safety/falsification/human-review gates."
    return "Acquire asserted-relation evidence before promotion beyond co-mention triage."


def candidate_rollups(
    candidate_hits: list[dict[str, Any]], validations: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    by_pair_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in validations:
        by_pair_key[row["pair_key"]].append(row)
    rollups: list[dict[str, Any]] = []
    for candidate in candidate_hits:
        if candidate.get("pubmed_literature_status") == "no_external_hit":
            continue
        rows = by_pair_key.get(candidate["pair_key"], [])
        relation_counts = Counter(row["relation_class"] for row in rows)
        best_relation = best_relation_class(rows)
        validated_rows = [row for row in rows if row["relation_class"] != "insufficient_text"]
        counter_rows = [row for row in rows if row["relation_class"] == "counter_evidence"]
        status = "source_text_validated_still_blocked" if validated_rows else "query_hit_not_validated_by_source_text"
        if counter_rows:
            status = "counter_evidence_review_required_still_blocked"
        rollups.append(
            {
                "schema_version": 1,
                "rollup_id": f"pubmed-pair-rollup:{stable_id(candidate['pair_id'], candidate['pair_key'])}",
                "pair_id": candidate["pair_id"],
                "pair_key": candidate["pair_key"],
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "candidate_pubmed_count": int(candidate.get("pubmed_count") or 0),
                "candidate_pubmed_idlist": candidate.get("pubmed_idlist") or [],
                "validated_evidence_rows": len(rows),
                "source_text_validated_rows": len(validated_rows),
                "insufficient_text_rows": relation_counts.get("insufficient_text", 0),
                "counter_evidence_rows": len(counter_rows),
                "relation_class_counts": dict(sorted(relation_counts.items())),
                "best_relation_class": best_relation,
                "pubmed_validation_status": status,
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "representative_pmids": [row["pmid"] for row in rows[:8]],
                "top_validated_titles": [
                    {"pmid": row["pmid"], "relation_class": row["relation_class"], "title": row["title"]}
                    for row in rows
                    if row["relation_class"] != "insufficient_text"
                ][:5],
                "reason_codes": rollup_reason_codes(status, best_relation),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": rollup_next_validation(status, best_relation),
            }
        )
    rollups.sort(
        key=lambda row: (
            RELATION_RANK.get(row["best_relation_class"], 0) * -1,
            -row["source_text_validated_rows"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return rollups


def best_relation_class(rows: list[dict[str, Any]]) -> str:
    if not rows:
        return "insufficient_text"
    return max((row["relation_class"] for row in rows), key=lambda value: RELATION_RANK.get(value, 0))


def rollup_reason_codes(status: str, best_relation: str) -> list[str]:
    codes = ["pubmed_validation_not_clinical_clearance"]
    if status == "query_hit_not_validated_by_source_text":
        codes.append("all_pubmed_query_hits_missing_exact_source_text_names")
    if status == "counter_evidence_review_required_still_blocked":
        codes.append("counter_evidence_review_required")
    if best_relation == "co_mention_only":
        codes.append("best_relation_is_co_mention_only")
    elif best_relation == "asserted_combination":
        codes.append("asserted_combination_requires_safety_outcome_gates")
    elif best_relation == "asserted_interaction":
        codes.append("asserted_interaction_requires_direction_safety_outcome_gates")
    elif best_relation == "asserted_outcome":
        codes.append("asserted_outcome_language_requires_endpoint_extraction")
    return codes


def rollup_next_validation(status: str, best_relation: str) -> str:
    if status == "query_hit_not_validated_by_source_text":
        return "Acquire another source or full abstract text before using this PubMed hit."
    if status == "counter_evidence_review_required_still_blocked":
        return "Route counter-evidence rows through falsification and safety review."
    if best_relation in {"asserted_combination", "asserted_interaction", "asserted_outcome"}:
        return "Extract structured relation/outcome details, then run safety, falsification, and human-review gates."
    return "Acquire asserted-relation evidence before promotion beyond literature co-mention triage."


def build_bridge_rows(
    validations: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollups:
        text = (
            f"PubMed validation rollup {row['pair_id']}: {row['drug_a']} plus {row['drug_b']} "
            f"has validation status {row['pubmed_validation_status']} with best relation "
            f"{row['best_relation_class']}, {row['source_text_validated_rows']} source-text validated rows, "
            f"{row['insufficient_text_rows']} insufficient-text rows, and remains {row['promotion_status']}."
        )
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row["pubmed_validation_status"],
                row["best_relation_class"],
                *row.get("representative_pmids", [])[:3],
            ]
        )
        rows.append(
            {
                "id": row["rollup_id"],
                "domain": "pubmed_source_text_validation_rollup",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1237_pubmed_source_text_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "pubmed_validation_status": row["pubmed_validation_status"],
                    "best_relation_class": row["best_relation_class"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in validations:
        text = (
            f"PubMed validation evidence {row['source_evidence_id']}: PMID {row['pmid']} for "
            f"{row['drug_a']} plus {row['drug_b']} is classified {row['relation_class']} "
            f"with source-text status {row['source_text_status']} and remains {row['validation_status']}."
        )
        terms = uniq([row["drug_a"], row["drug_b"], row["pmid"], row["relation_class"], row["source_text_status"]])
        rows.append(
            {
                "id": row["validation_id"],
                "domain": "pubmed_source_text_validation_evidence",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1237_pubmed_source_text_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pmid": row["pmid"],
                    "relation_class": row["relation_class"],
                    "validation_status": row["validation_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows[:1000]


def build_metrics(
    evidence_rows: list[dict[str, Any]],
    candidate_hits: list[dict[str, Any]],
    efetch_rows: list[dict[str, Any]],
    source_records: list[dict[str, Any]],
    validations: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
) -> dict[str, Any]:
    relation_counts = Counter(row["relation_class"] for row in validations)
    validation_status_counts = Counter(row["validation_status"] for row in validations)
    rollup_status_counts = Counter(row["pubmed_validation_status"] for row in rollups)
    best_relation_counts = Counter(row["best_relation_class"] for row in rollups)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "input_pubmed_evidence_rows": len(evidence_rows),
        "input_candidate_hit_rows": len(candidate_hits),
        "unique_pmids": len(unique_pmids(evidence_rows)),
        "efetch_response_chunks": len(efetch_rows),
        "source_records": len(source_records),
        "evidence_validation_rows": len(validations),
        "candidate_pair_rollup_rows": len(rollups),
        "source_text_validated_evidence_rows": sum(
            1 for row in validations if row["relation_class"] != "insufficient_text"
        ),
        "insufficient_text_evidence_rows": relation_counts.get("insufficient_text", 0),
        "counter_evidence_rows": relation_counts.get("counter_evidence", 0),
        "relation_class_counts": dict(sorted(relation_counts.items())),
        "validation_status_counts": dict(sorted(validation_status_counts.items())),
        "rollup_status_counts": dict(sorted(rollup_status_counts.items())),
        "best_relation_class_counts": dict(sorted(best_relation_counts.items())),
        "clinical_boundary_rows": sum(1 for row in validations if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_rollups": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "best_relation_class": row["best_relation_class"],
                "pubmed_validation_status": row["pubmed_validation_status"],
                "source_text_validated_rows": row["source_text_validated_rows"],
                "counter_evidence_rows": row["counter_evidence_rows"],
                "pmids": row["representative_pmids"][:5],
            }
            for row in rollups[:25]
        ],
    }


def build_input_manifest(inputs: dict[str, str], evidence_rows: list[dict[str, Any]], candidate_hits: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1237,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1234_pubmed_evidence": artifact(Path(inputs["issue1234_pubmed_evidence"]), jsonl=True),
            "issue1234_candidate_hits": artifact(Path(inputs["issue1234_candidate_hits"]), jsonl=True),
            "issue1234_pubmed_esearch": artifact(Path(inputs["issue1234_pubmed_esearch"]), jsonl=True),
            "issue1234_pubmed_esummary": artifact(Path(inputs["issue1234_pubmed_esummary"]), jsonl=True),
            "issue1234_persisted_readback": artifact(Path(inputs["issue1234_persisted_readback"])),
            "issue1234_calyx_readback": artifact(Path(inputs["issue1234_calyx_readback"])),
            "ncbi_eutilities_intro": {
                **artifact(Path(inputs["ncbi_eutilities_intro"])),
                "source_page_url": NCBI_EUTILITIES_INTRO_URL,
            },
            "nlm_eutilities_guide": {
                **artifact(Path(inputs["nlm_eutilities_guide"])),
                "source_page_url": NLM_EUTILITIES_GUIDE_URL,
            },
            "ncbi_eutilities_parameters": {
                **artifact(Path(inputs["ncbi_eutilities_parameters"])),
                "source_page_url": NCBI_EUTILITIES_PARAMETERS_URL,
            },
        },
        "accepted_sources": [
            {
                "source": "PubMed E-utilities EFetch XML",
                "role": "source text and citation validation for #1234 PubMed co-mention hits",
                "efetch_url": PUBMED_EFETCH_URL,
                "intro_url": NCBI_EUTILITIES_INTRO_URL,
                "parameters_url": NCBI_EUTILITIES_PARAMETERS_URL,
                "guide_url": NLM_EUTILITIES_GUIDE_URL,
                "retmode": "xml",
                "request_sleep_seconds": REQUEST_SLEEP_SECONDS,
            }
        ],
        "query_universe": {
            "pubmed_evidence_rows": len(evidence_rows),
            "candidate_hit_rows": len(candidate_hits),
            "unique_pmids": len(unique_pmids(evidence_rows)),
            "source": "issue1234 pubmed_pair_literature_evidence.jsonl and candidate_external_source_hits.jsonl",
        },
    }


def build_readback(
    out_dir: Path,
    evidence_rows: list[dict[str, Any]],
    candidate_hits: list[dict[str, Any]],
    source_records: list[dict[str, Any]],
    validations: list[dict[str, Any]],
    rollups: list[dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "pubmed_efetch_responses": artifact(out_dir / "pubmed_efetch_responses.jsonl", jsonl=True),
        "pubmed_source_records": artifact(out_dir / "pubmed_source_records.jsonl", jsonl=True),
        "pubmed_evidence_validation": artifact(out_dir / "pubmed_evidence_validation.jsonl", jsonl=True),
        "candidate_pair_pubmed_validation_rollup": artifact(
            out_dir / "candidate_pair_pubmed_validation_rollup.jsonl", jsonl=True
        ),
        "candidate_pair_pubmed_validation_hits": artifact(
            out_dir / "candidate_pair_pubmed_validation_hits.jsonl", jsonl=True
        ),
        "pubmed_validation_bridge_rows": artifact(out_dir / "pubmed_validation_bridge_rows.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    source_pmids = {row["pmid"] for row in source_records}
    input_pmids = set(unique_pmids(evidence_rows))
    return {
        "schema_version": 1,
        "issue": 1237,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "source_records_for_every_unique_pmid": source_pmids == input_pmids,
            "validation_row_for_every_evidence_row": len(validations) == len(evidence_rows),
            "rollup_row_for_every_candidate_pubmed_hit": len(rollups) == len(
                [row for row in candidate_hits if row.get("pubmed_literature_status") != "no_external_hit"]
            ),
            "all_validation_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in validations),
            "all_validation_rows_have_relation_class": all(row["relation_class"] in RELATION_RANK for row in validations),
            "all_validated_rows_have_both_names_present": all(
                row["drug_a_presence"]["present"] and row["drug_b_presence"]["present"]
                for row in validations
                if row["relation_class"] != "insufficient_text"
            ),
            "insufficient_rows_do_not_claim_validation": all(
                row["validation_status"] == "query_hit_not_validated_by_source_text"
                for row in validations
                if row["relation_class"] == "insufficient_text"
            ),
            "bridge_rows_1000_or_less": artifacts["pubmed_validation_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "input_evidence_rows": len(evidence_rows),
            "candidate_hit_rows": len(candidate_hits),
            "source_records": len(source_records),
            "validation_rows": len(validations),
            "rollup_rows": len(rollups),
        },
    }


def run(
    root: Path,
    inputs: dict[str, str],
    *,
    max_pmids: int | None,
    chunk_size: int,
    request_sleep_seconds: float,
    email: str,
) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    evidence_rows = rows_jsonl(Path(inputs["issue1234_pubmed_evidence"]))
    candidate_hits = rows_jsonl(Path(inputs["issue1234_candidate_hits"]))
    pmids = unique_pmids(evidence_rows)
    if max_pmids is not None:
        keep = set(pmids[:max_pmids])
        evidence_rows = [row for row in evidence_rows if str(row["pmid"]) in keep]
        candidate_hits = [row for row in candidate_hits if set(map(str, row.get("pubmed_idlist") or [])) & keep]
        pmids = unique_pmids(evidence_rows)

    write_json(out_dir / "input_manifest.json", build_input_manifest(inputs, evidence_rows, candidate_hits))
    efetch_rows = fetch_pubmed_efetch(
        pmids,
        out_dir / "pubmed_efetch_responses.jsonl",
        chunk_size=chunk_size,
        request_sleep_seconds=request_sleep_seconds,
        email=email,
    )
    source_records = parse_source_records(efetch_rows)
    write_jsonl(out_dir / "pubmed_source_records.jsonl", source_records)
    validations = validate_evidence_rows(evidence_rows, source_records)
    write_jsonl(out_dir / "pubmed_evidence_validation.jsonl", validations)
    rollups = candidate_rollups(candidate_hits, validations)
    write_jsonl(out_dir / "candidate_pair_pubmed_validation_rollup.jsonl", rollups)
    rollup_hits = [row for row in rollups if row["source_text_validated_rows"] > 0 or row["counter_evidence_rows"] > 0]
    write_jsonl(out_dir / "candidate_pair_pubmed_validation_hits.jsonl", rollup_hits)
    source_path = out_dir / "pubmed_evidence_validation.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(validations, rollups, source_path, source_sha)
    write_jsonl(out_dir / "pubmed_validation_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(evidence_rows, candidate_hits, efetch_rows, source_records, validations, rollups)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1237,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "pubmed_efetch_responses": artifact(out_dir / "pubmed_efetch_responses.jsonl", jsonl=True),
            "pubmed_source_records": artifact(out_dir / "pubmed_source_records.jsonl", jsonl=True),
            "pubmed_evidence_validation": artifact(out_dir / "pubmed_evidence_validation.jsonl", jsonl=True),
            "candidate_pair_pubmed_validation_rollup": artifact(
                out_dir / "candidate_pair_pubmed_validation_rollup.jsonl", jsonl=True
            ),
            "candidate_pair_pubmed_validation_hits": artifact(
                out_dir / "candidate_pair_pubmed_validation_hits.jsonl", jsonl=True
            ),
            "pubmed_validation_bridge_rows": artifact(out_dir / "pubmed_validation_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, evidence_rows, candidate_hits, source_records, validations, rollups)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "pubmed_evidence_validation": output_manifest["artifacts"]["pubmed_evidence_validation"],
            "candidate_pair_pubmed_validation_rollup": output_manifest["artifacts"][
                "candidate_pair_pubmed_validation_rollup"
            ],
            "candidate_pair_pubmed_validation_hits": output_manifest["artifacts"][
                "candidate_pair_pubmed_validation_hits"
            ],
            "bridge_rows": output_manifest["artifacts"]["pubmed_validation_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def inputs_for_root(root: Path) -> dict[str, str]:
    inputs = dict(DEFAULT_INPUTS)
    inputs["ncbi_eutilities_parameters"] = str(root / "raw" / "ncbi_eutilities_parameters.html")
    return inputs


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1234-pubmed-evidence")
    parser.add_argument("--issue1234-candidate-hits")
    parser.add_argument("--issue1234-pubmed-esearch")
    parser.add_argument("--issue1234-pubmed-esummary")
    parser.add_argument("--issue1234-persisted-readback")
    parser.add_argument("--issue1234-calyx-readback")
    parser.add_argument("--ncbi-eutilities-intro")
    parser.add_argument("--nlm-eutilities-guide")
    parser.add_argument("--ncbi-eutilities-parameters")
    parser.add_argument("--max-pmids", type=int)
    parser.add_argument("--chunk-size", type=int, default=EFETCH_CHUNK_SIZE)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    parser.add_argument("--email", default=PUBMED_EMAIL)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    inputs = inputs_for_root(root)
    for arg_name, input_name in [
        ("issue1234_pubmed_evidence", "issue1234_pubmed_evidence"),
        ("issue1234_candidate_hits", "issue1234_candidate_hits"),
        ("issue1234_pubmed_esearch", "issue1234_pubmed_esearch"),
        ("issue1234_pubmed_esummary", "issue1234_pubmed_esummary"),
        ("issue1234_persisted_readback", "issue1234_persisted_readback"),
        ("issue1234_calyx_readback", "issue1234_calyx_readback"),
        ("ncbi_eutilities_intro", "ncbi_eutilities_intro"),
        ("nlm_eutilities_guide", "nlm_eutilities_guide"),
        ("ncbi_eutilities_parameters", "ncbi_eutilities_parameters"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(
        root,
        inputs,
        max_pmids=args.max_pmids,
        chunk_size=args.chunk_size,
        request_sleep_seconds=args.request_sleep_seconds,
        email=args.email,
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
