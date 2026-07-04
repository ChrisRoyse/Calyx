#!/usr/bin/env python3
"""#1234 FDA/PubMed source mining for remaining combo no-hit rows.

This stage rechecks the #1232 remaining no-hit candidate-pair universe against
current FDA Orange Book product data, current FDA NDC product-listing data, and
PubMed title/abstract co-mention search. The output is source-attributed
research triage evidence only. It is not efficacy, safety, treatment guidance,
dosing, recommendation, clinical actionability, or cure evidence.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import itertools
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import Counter
from pathlib import Path
from typing import Any
from zipfile import ZipFile


CLINICAL_BOUNDARY = (
    "External FDA/PubMed evidence is source-attributed product or literature "
    "documentation only; not efficacy, safety, clinical actionability, "
    "treatment guidance, dosing, recommendation, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "fda_product_or_pubmed_title_abstract_comention_not_combination_outcome_or_safety_clearance"
)

PUBMED_EVIDENCE_KIND = (
    "pubmed_title_abstract_phrase_query_comention_not_combination_outcome_or_safety_clearance"
)

FDA_PRODUCT_EVIDENCE_KIND = (
    "fda_product_ingredient_cooccurrence_not_combination_outcome_or_safety_clearance"
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z"

DEFAULT_INPUTS = {
    "issue1232_clinicaltrials_pair_status": (
        "/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-"
        "20260704T153000Z/out/clinicaltrials_pair_status.jsonl"
    ),
    "fda_orangebook_zip": f"{DEFAULT_ROOT}/raw/fda_orangebook_current.zip",
    "fda_orangebook_page": f"{DEFAULT_ROOT}/raw/fda_orangebook_data_files.html",
    "fda_ndc_zip": f"{DEFAULT_ROOT}/raw/fda_ndc_current_text.zip",
    "fda_ndc_page": f"{DEFAULT_ROOT}/raw/fda_ndc_directory.html",
    "ncbi_eutilities_intro": f"{DEFAULT_ROOT}/raw/ncbi_eutilities_intro.html",
    "nlm_eutilities_guide": f"{DEFAULT_ROOT}/raw/nlm_eutilities_guide.html",
}

FDA_ORANGEBOOK_PAGE_URL = "https://www.fda.gov/drugs/drug-approvals-and-databases/orange-book-data-files"
FDA_ORANGEBOOK_DOWNLOAD_URL = "https://www.fda.gov/media/76860/download?attachment"
FDA_NDC_PAGE_URL = "https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory"
FDA_NDC_DOWNLOAD_URL = "https://www.accessdata.fda.gov/cder/ndctext.zip"
NCBI_EUTILITIES_INTRO_URL = "https://www.ncbi.nlm.nih.gov/books/NBK25497/"
NLM_EUTILITIES_GUIDE_URL = "https://www.nlm.nih.gov/dataguide/eutilities/utilities.html"
PUBMED_ESEARCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi"
PUBMED_ESUMMARY_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi"

PUBMED_TOOL = "calyx"
PUBMED_EMAIL = "opensource@example.com"
PUBMED_RETTYPE_MAX = 20
PUBMED_EVIDENCE_PER_PAIR = 8
REQUEST_SLEEP_SECONDS = 0.40
USER_AGENT = "calyx-discovery/issue1234"


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


def norm_name(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def pair_key(left: object, right: object) -> str:
    a, b = sorted([norm_name(left), norm_name(right)])
    return f"{a}||{b}"


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
                    "message": "required source artifacts are missing",
                    "missing": missing,
                    "remediation": "download and persist the source pages/data files before running #1234",
                },
                indent=2,
            )
        )


def remaining_no_hit_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = [row for row in rows if row.get("clinicaltrials_current_status") == "no_external_hit"]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    return out


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
    return " ".join(text.split())


def valid_norm(value: str) -> bool:
    return bool(value and value not in {"na", "nan", "none", "none given", "unknown", "1"})


def split_components(value: object) -> list[str]:
    text = clean_text(value).strip(" ;,/+")
    if not text:
        return []
    raw_parts = re.split(r"\s*(?:;|\+|/|\bAND\b)\s*", text, flags=re.IGNORECASE)
    parts: list[str] = []
    seen: set[str] = set()
    for part in raw_parts:
        cleaned = clean_text(part).strip(" ;,/+")
        norm = norm_name(cleaned)
        if not cleaned or not valid_norm(norm) or norm in seen:
            continue
        seen.add(norm)
        parts.append(cleaned)
    return parts


def zip_entry_by_basename(zip_file: ZipFile, basename: str) -> str:
    target = basename.lower()
    for info in zip_file.infolist():
        if Path(info.filename).name.lower() == target:
            return info.filename
    raise KeyError(f"zip entry not found: {basename}")


def zip_text_schema(zip_path: Path, delimiter: str, encoding: str) -> dict[str, dict[str, Any]]:
    schemas: dict[str, dict[str, Any]] = {}
    with ZipFile(zip_path) as zip_file:
        for info in zip_file.infolist():
            if info.is_dir():
                continue
            with zip_file.open(info.filename) as handle:
                reader = csv.reader(
                    io.TextIOWrapper(handle, encoding=encoding, errors="replace"),
                    delimiter=delimiter,
                )
                header = next(reader)
                rows = sum(1 for _ in reader)
            schemas[Path(info.filename).name.lower()] = {
                "rows": rows,
                "columns": len(header),
                "header": header,
                "header_sha256": sha256_bytes(json.dumps(header).encode("utf-8")),
                "zip_entry_bytes": info.file_size,
                "zip_entry_crc": f"{info.CRC:08x}",
            }
    return schemas


def append_pair_hit(pair_index: dict[str, dict[str, Any]], key: str, example: dict[str, Any]) -> None:
    entry = pair_index.setdefault(
        key,
        {
            "pair_key": key,
            "source_record_count": 0,
            "two_component_record_count": 0,
            "multi_component_record_count": 0,
            "type_counts": Counter(),
            "examples": [],
        },
    )
    entry["source_record_count"] += 1
    if example["source_combination_size"] == 2:
        entry["two_component_record_count"] += 1
    else:
        entry["multi_component_record_count"] += 1
    entry["type_counts"].update([example.get("source_type") or "unspecified"])
    if len(entry["examples"]) < 8:
        entry["examples"].append(example)


def pair_rows_from_index(
    pair_index: dict[str, dict[str, Any]], source_name: str, pair_id_prefix: str
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for key, entry in pair_index.items():
        left_norm, right_norm = key.split("||", 1)
        examples = entry["examples"]
        source_counts = dict(sorted(entry["type_counts"].items()))
        left_display = examples[0].get("left_component") if examples else left_norm
        right_display = examples[0].get("right_component") if examples else right_norm
        rows.append(
            {
                "schema_version": 1,
                "source": source_name,
                "pair_id": f"{pair_id_prefix}:{stable_id(key)}",
                "pair_key": key,
                "drug_a_norm": left_norm,
                "drug_b_norm": right_norm,
                "display_drug_a": left_display,
                "display_drug_b": right_display,
                "source_record_count": entry["source_record_count"],
                "source_type_counts": source_counts,
                "source_types": sorted(source_counts),
                "two_component_record_count": entry["two_component_record_count"],
                "multi_component_record_count": entry["multi_component_record_count"],
                "examples": examples,
                "evidence_kind": FDA_PRODUCT_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (-row["source_record_count"], row["pair_key"]))
    return rows


def build_orangebook_index(zip_path: Path) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]], dict[str, Any]]:
    schemas = zip_text_schema(zip_path, "~", "latin-1")
    source_rows: list[dict[str, Any]] = []
    pair_index: dict[str, dict[str, Any]] = {}
    with ZipFile(zip_path) as zip_file:
        entry_name = zip_entry_by_basename(zip_file, "products.txt")
        with zip_file.open(entry_name) as handle:
            reader = csv.DictReader(
                io.TextIOWrapper(handle, encoding="latin-1", errors="replace"),
                delimiter="~",
            )
            for raw_index, row in enumerate(reader, start=1):
                components = split_components(row.get("Ingredient"))
                if len(components) < 2:
                    continue
                product_id = f"orangebook-product:{stable_id(row.get('Appl_Type'), row.get('Appl_No'), row.get('Product_No'), raw_index)}"
                source_row = {
                    "schema_version": 1,
                    "source": "FDA Orange Book",
                    "product_id": product_id,
                    "source_row": raw_index,
                    "source_combination_size": len(components),
                    "components": components,
                    "trade_name": clean_text(row.get("Trade_Name")),
                    "applicant": clean_text(row.get("Applicant")),
                    "application_type": clean_text(row.get("Appl_Type")),
                    "application_number": clean_text(row.get("Appl_No")),
                    "product_number": clean_text(row.get("Product_No")),
                    "approval_date": clean_text(row.get("Approval_Date")),
                    "type": clean_text(row.get("Type")),
                    "evidence_kind": FDA_PRODUCT_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
                source_rows.append(source_row)
                for left, right in itertools.combinations(components, 2):
                    append_pair_hit(
                        pair_index,
                        pair_key(left, right),
                        {
                            "source": "FDA Orange Book",
                            "product_id": product_id,
                            "source_type": clean_text(row.get("Type")) or "unspecified",
                            "source_combination_size": len(components),
                            "left_component": left,
                            "right_component": right,
                            "all_components": components[:20],
                            "trade_name": source_row["trade_name"],
                            "application_number": source_row["application_number"],
                            "product_number": source_row["product_number"],
                            "approval_date": source_row["approval_date"],
                        },
                    )
    pair_rows = pair_rows_from_index(pair_index, "FDA Orange Book", "fda-orangebook")
    return source_rows, {row["pair_key"]: row for row in pair_rows}, {
        "schema_version": 1,
        "source": "FDA Orange Book",
        "schemas": schemas,
        "schema_fingerprint_sha256": sha256_bytes(json.dumps(schemas, sort_keys=True).encode("utf-8")),
        "source_multi_ingredient_product_rows": len(source_rows),
        "source_pair_key_rows": len(pair_rows),
        "pair_rows": pair_rows,
    }


def build_ndc_index(zip_path: Path) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]], dict[str, Any]]:
    schemas = zip_text_schema(zip_path, "\t", "utf-8-sig")
    source_rows: list[dict[str, Any]] = []
    pair_index: dict[str, dict[str, Any]] = {}
    with ZipFile(zip_path) as zip_file:
        entry_name = zip_entry_by_basename(zip_file, "product.txt")
        with zip_file.open(entry_name) as handle:
            reader = csv.DictReader(
                io.TextIOWrapper(handle, encoding="utf-8-sig", errors="replace"),
                delimiter="\t",
            )
            for raw_index, row in enumerate(reader, start=1):
                components = split_components(row.get("SUBSTANCENAME"))
                if len(components) < 2:
                    components = split_components(row.get("NONPROPRIETARYNAME"))
                if len(components) < 2:
                    continue
                product_id = f"ndc-product:{stable_id(row.get('PRODUCTID'), row.get('PRODUCTNDC'), raw_index)}"
                source_row = {
                    "schema_version": 1,
                    "source": "FDA NDC Directory",
                    "product_id": product_id,
                    "source_row": raw_index,
                    "source_combination_size": len(components),
                    "components": components,
                    "product_ndc": clean_text(row.get("PRODUCTNDC")),
                    "product_type": clean_text(row.get("PRODUCTTYPENAME")),
                    "proprietary_name": clean_text(row.get("PROPRIETARYNAME")),
                    "nonproprietary_name": clean_text(row.get("NONPROPRIETARYNAME")),
                    "dosage_form": clean_text(row.get("DOSAGEFORMNAME")),
                    "route": clean_text(row.get("ROUTENAME")),
                    "marketing_category": clean_text(row.get("MARKETINGCATEGORYNAME")),
                    "application_number": clean_text(row.get("APPLICATIONNUMBER")),
                    "labeler": clean_text(row.get("LABELERNAME")),
                    "ndc_exclude_flag": clean_text(row.get("NDC_EXCLUDE_FLAG")),
                    "evidence_kind": FDA_PRODUCT_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
                source_rows.append(source_row)
                for left, right in itertools.combinations(components, 2):
                    append_pair_hit(
                        pair_index,
                        pair_key(left, right),
                        {
                            "source": "FDA NDC Directory",
                            "product_id": product_id,
                            "source_type": source_row["product_type"] or "unspecified",
                            "source_combination_size": len(components),
                            "left_component": left,
                            "right_component": right,
                            "all_components": components[:20],
                            "product_ndc": source_row["product_ndc"],
                            "product_type": source_row["product_type"],
                            "proprietary_name": source_row["proprietary_name"],
                            "marketing_category": source_row["marketing_category"],
                            "application_number": source_row["application_number"],
                        },
                    )
    pair_rows = pair_rows_from_index(pair_index, "FDA NDC Directory", "fda-ndc")
    return source_rows, {row["pair_key"]: row for row in pair_rows}, {
        "schema_version": 1,
        "source": "FDA NDC Directory",
        "schemas": schemas,
        "schema_fingerprint_sha256": sha256_bytes(json.dumps(schemas, sort_keys=True).encode("utf-8")),
        "source_multi_ingredient_product_rows": len(source_rows),
        "source_pair_key_rows": len(pair_rows),
        "pair_rows": pair_rows,
    }


def candidate_exact_match(candidate: dict[str, Any], source: dict[str, Any] | None) -> bool:
    if not source:
        return False
    candidate_names = {
        clean_text(candidate.get("drug_a")).lower(),
        clean_text(candidate.get("drug_b")).lower(),
    }
    for example in source.get("examples") or []:
        if example.get("source_combination_size") != 2:
            continue
        source_names = {
            clean_text(example.get("left_component")).lower(),
            clean_text(example.get("right_component")).lower(),
        }
        if source_names == candidate_names:
            return True
    return False


def fda_status(candidate: dict[str, Any], source: dict[str, Any] | None) -> str:
    if candidate_exact_match(candidate, source):
        return "exact_hit"
    if source:
        return "normalized_hit"
    return "no_external_hit"


def pubmed_query_term(row: dict[str, Any]) -> str:
    left = query_name(row.get("drug_a"))
    right = query_name(row.get("drug_b"))
    if not left or not right:
        return ""
    return f'("{left}"[Title/Abstract]) AND ("{right}"[Title/Abstract])'


def pubmed_query_url(base_url: str, params: dict[str, str]) -> str:
    return f"{base_url}?{urllib.parse.urlencode(params)}"


def fetch_bytes(url: str, retries: int = 5) -> bytes:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read()
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
            last_error = error
            if isinstance(error, urllib.error.HTTPError) and error.code in {400, 404}:
                raise
            time.sleep(min(10.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"fetch failed after {retries} attempts: {last_error}")


def unique_pubmed_queries(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    out: list[dict[str, Any]] = []
    for row in rows:
        key = row["pair_key"]
        if key in seen:
            continue
        seen.add(key)
        term = pubmed_query_term(row)
        out.append(
            {
                "pair_key": key,
                "representative_pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "query_term": term,
            }
        )
    out.sort(key=lambda row: row["pair_key"])
    return out


def cached_jsonl_exact(path: Path, expected_keys: set[str], key_field: str) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    rows = rows_jsonl(path)
    observed = {row.get(key_field) for row in rows}
    if observed == expected_keys:
        return rows
    return None


def fetch_pubmed_esearch(
    query_rows: list[dict[str, Any]],
    out_path: Path,
    *,
    retmax: int,
    request_sleep_seconds: float,
    email: str,
) -> list[dict[str, Any]]:
    expected_keys = {row["pair_key"] for row in query_rows}
    cached = cached_jsonl_exact(out_path, expected_keys, "pair_key")
    if cached is not None:
        return cached
    tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
    rows: list[dict[str, Any]] = []
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with tmp_path.open("w", encoding="utf-8") as handle:
        for index, row in enumerate(query_rows, start=1):
            if not row["query_term"]:
                data = {"esearchresult": {"count": "0", "idlist": []}, "query_error": "empty query term"}
                payload = json.dumps(data, sort_keys=True).encode("utf-8")
                url = ""
            else:
                params = {
                    "db": "pubmed",
                    "retmode": "json",
                    "retmax": str(retmax),
                    "sort": "relevance",
                    "term": row["query_term"],
                    "tool": PUBMED_TOOL,
                    "email": email,
                }
                url = pubmed_query_url(PUBMED_ESEARCH_URL, params)
                payload = fetch_bytes(url)
                data = json.loads(payload)
            result = data.get("esearchresult") or {}
            out = {
                "schema_version": 1,
                "pair_key": row["pair_key"],
                "representative_pair_id": row["representative_pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "query_term": row["query_term"],
                "query_url": url,
                "api_base": PUBMED_ESEARCH_URL,
                "retmax": retmax,
                "count": int(result.get("count") or 0),
                "idlist": [str(pmid) for pmid in result.get("idlist") or []],
                "response_bytes": len(payload),
                "response_sha256": sha256_bytes(payload),
                "response": data,
                "evidence_kind": PUBMED_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(out)
            handle.write(json.dumps(out, sort_keys=True) + "\n")
            handle.flush()
            if index % 100 == 0:
                print(f"fetched {index}/{len(query_rows)} PubMed ESearch responses", file=sys.stderr)
            time.sleep(request_sleep_seconds)
    os.replace(tmp_path, out_path)
    return rows


def chunked(values: list[str], size: int) -> list[list[str]]:
    return [values[index : index + size] for index in range(0, len(values), size)]


def fetch_pubmed_esummary(
    esearch_rows: list[dict[str, Any]],
    out_path: Path,
    *,
    request_sleep_seconds: float,
    email: str,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    pmids = sorted({pmid for row in esearch_rows for pmid in row.get("idlist", [])[:PUBMED_EVIDENCE_PER_PAIR]})
    if not pmids:
        write_jsonl(out_path, [])
        return [], {}
    expected_chunk_keys = {",".join(chunk) for chunk in chunked(pmids, 150)}
    cached = cached_jsonl_exact(out_path, expected_chunk_keys, "pmid_chunk_key") if expected_chunk_keys else []
    if cached is None:
        rows: list[dict[str, Any]] = []
        tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
        out_path.parent.mkdir(parents=True, exist_ok=True)
        with tmp_path.open("w", encoding="utf-8") as handle:
            for chunk_index, ids in enumerate(chunked(pmids, 150), start=1):
                params = {
                    "db": "pubmed",
                    "retmode": "json",
                    "id": ",".join(ids),
                    "tool": PUBMED_TOOL,
                    "email": email,
                }
                url = pubmed_query_url(PUBMED_ESUMMARY_URL, params)
                payload = fetch_bytes(url)
                data = json.loads(payload)
                out = {
                    "schema_version": 1,
                    "chunk_index": chunk_index,
                    "pmid_chunk_key": ",".join(ids),
                    "pmids": ids,
                    "query_url": url,
                    "api_base": PUBMED_ESUMMARY_URL,
                    "response_bytes": len(payload),
                    "response_sha256": sha256_bytes(payload),
                    "response": data,
                    "evidence_kind": PUBMED_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
                rows.append(out)
                handle.write(json.dumps(out, sort_keys=True) + "\n")
                handle.flush()
                print(f"fetched PubMed ESummary chunk {chunk_index}/{len(chunked(pmids, 150))}", file=sys.stderr)
                time.sleep(request_sleep_seconds)
        os.replace(tmp_path, out_path)
    else:
        rows = cached
    summaries: dict[str, dict[str, Any]] = {}
    for row in rows:
        result = (row.get("response") or {}).get("result") or {}
        for uid in result.get("uids") or []:
            item = result.get(str(uid)) or {}
            summaries[str(uid)] = {
                "pmid": str(uid),
                "title": clean_text(item.get("title")),
                "pubdate": clean_text(item.get("pubdate")),
                "source": clean_text(item.get("source")),
                "authors": [
                    clean_text(author.get("name"))
                    for author in (item.get("authors") or [])[:8]
                    if clean_text(author.get("name"))
                ],
                "articleids": item.get("articleids") or [],
            }
    return rows, summaries


def pubmed_evidence_rows(
    esearch_rows: list[dict[str, Any]], summaries: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for raw in esearch_rows:
        if int(raw.get("count") or 0) <= 0:
            continue
        for pmid in raw.get("idlist", [])[:PUBMED_EVIDENCE_PER_PAIR]:
            summary = summaries.get(str(pmid), {"pmid": str(pmid)})
            rows.append(
                {
                    "schema_version": 1,
                    "evidence_id": f"pubmed-pair:{stable_id(raw['pair_key'], pmid)}",
                    "pair_key": raw["pair_key"],
                    "representative_pair_id": raw["representative_pair_id"],
                    "drug_a": raw["drug_a"],
                    "drug_b": raw["drug_b"],
                    "status": "normalized_hit",
                    "source": "PubMed",
                    "source_url": f"https://pubmed.ncbi.nlm.nih.gov/{pmid}/",
                    "pmid": str(pmid),
                    "summary": summary,
                    "query_term": raw["query_term"],
                    "query_response_sha256": raw["response_sha256"],
                    "evidence_kind": PUBMED_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    rows.sort(key=lambda row: (row["pair_key"], row["pmid"]))
    return rows


def merge_status(*statuses: str) -> str:
    if "exact_hit" in statuses:
        return "exact_hit"
    if "normalized_hit" in statuses:
        return "normalized_hit"
    return "no_external_hit"


def source_status_rows(
    candidates: list[dict[str, Any]],
    orangebook_index: dict[str, dict[str, Any]],
    ndc_index: dict[str, dict[str, Any]],
    pubmed_by_pair_key: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    joined: list[dict[str, Any]] = []
    for candidate in candidates:
        key = candidate["pair_key"]
        orangebook = orangebook_index.get(key)
        ndc = ndc_index.get(key)
        pubmed = pubmed_by_pair_key.get(key) or {}
        orangebook_status = fda_status(candidate, orangebook)
        ndc_status = fda_status(candidate, ndc)
        pubmed_status = "normalized_hit" if int(pubmed.get("count") or 0) > 0 else "no_external_hit"
        overall = merge_status(orangebook_status, ndc_status, pubmed_status)
        source_types: list[str] = []
        if orangebook_status != "no_external_hit":
            source_types.append("FDA Orange Book")
        if ndc_status != "no_external_hit":
            source_types.append("FDA NDC Directory")
        if pubmed_status != "no_external_hit":
            source_types.append("PubMed")
        reason_codes = list(candidate.get("reason_codes") or [])
        if overall != "no_external_hit":
            reason_codes.append("external_fda_or_pubmed_hit_not_clearance")
            combination_status = "external_fda_or_pubmed_documented_still_blocked"
            next_validation = (
                "Inspect source rows, then require independent mechanism, synergy, safety, "
                "outcome, and human-review gates before any promotion."
            )
        else:
            reason_codes.append("external_fda_or_pubmed_missing_fail_closed")
            combination_status = "blocked_no_external_fda_or_pubmed_evidence_after_issue1234"
            next_validation = "Acquire another open source or normalize identifiers before promotion."
        joined.append(
            {
                "schema_version": 1,
                "evidence_id": f"issue1234-source:{stable_id(candidate['pair_id'], key)}",
                "pair_id": candidate["pair_id"],
                "pair_key": key,
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "previous_clinicaltrials_current_status": candidate.get("clinicaltrials_current_status"),
                "orangebook_status": orangebook_status,
                "orangebook_match": orangebook_status != "no_external_hit",
                "orangebook_summary": orangebook,
                "ndc_status": ndc_status,
                "ndc_match": ndc_status != "no_external_hit",
                "ndc_summary": ndc,
                "pubmed_literature_status": pubmed_status,
                "pubmed_match": pubmed_status != "no_external_hit",
                "pubmed_count": int(pubmed.get("count") or 0),
                "pubmed_idlist": list(pubmed.get("idlist") or [])[:PUBMED_EVIDENCE_PER_PAIR],
                "pubmed_query_term": pubmed.get("query_term"),
                "pubmed_response_sha256": pubmed.get("response_sha256"),
                "overall_external_source_status": overall,
                "source_types": source_types,
                "combination_status": combination_status,
                "reason_codes": uniq(reason_codes),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": next_validation,
            }
        )
    joined.sort(
        key=lambda row: (
            0 if row["overall_external_source_status"] != "no_external_hit" else 1,
            -row["pubmed_count"],
            row["pair_key"],
            row["pair_id"],
        )
    )
    return joined


def build_bridge_rows(rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows[:1000]:
        pmids = ", ".join(row.get("pubmed_idlist") or []) or "none"
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row["overall_external_source_status"],
                row["orangebook_status"],
                row["ndc_status"],
                row["pubmed_literature_status"],
                row["combination_status"],
                *row.get("source_types", []),
                *row.get("pubmed_idlist", [])[:3],
            ]
        )
        text = (
            f"External FDA/PubMed source recheck {row['pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} has overall source status {row['overall_external_source_status']}; "
            f"Orange Book {row['orangebook_status']}, NDC {row['ndc_status']}, PubMed "
            f"{row['pubmed_literature_status']} with {row['pubmed_count']} title/abstract "
            f"query hits (PMIDs {pmids}); the pair remains {row['combination_status']}."
        )
        out.append(
            {
                "id": row["pair_id"],
                "domain": "external_drug_combination_fda_pubmed_recheck",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1234_fda_pubmed_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "overall_external_source_status": row["overall_external_source_status"],
                    "orangebook_status": row["orangebook_status"],
                    "ndc_status": row["ndc_status"],
                    "pubmed_literature_status": row["pubmed_literature_status"],
                    "combination_status": row["combination_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def build_metrics(
    candidates: list[dict[str, Any]],
    joined: list[dict[str, Any]],
    orangebook_source_rows: list[dict[str, Any]],
    orangebook_index_rows: list[dict[str, Any]],
    ndc_source_rows: list[dict[str, Any]],
    ndc_index_rows: list[dict[str, Any]],
    esearch_rows: list[dict[str, Any]],
    esummary_rows: list[dict[str, Any]],
    pubmed_evidence: list[dict[str, Any]],
) -> dict[str, Any]:
    overall_counts = Counter(row["overall_external_source_status"] for row in joined)
    orangebook_counts = Counter(row["orangebook_status"] for row in joined)
    ndc_counts = Counter(row["ndc_status"] for row in joined)
    pubmed_counts = Counter(row["pubmed_literature_status"] for row in joined)
    reason_counts = Counter(reason for row in joined for reason in row["reason_codes"])
    source_type_counts = Counter(source for row in joined for source in row["source_types"])
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rows_from_issue1232_no_hit": len(candidates),
        "joined_status_rows": len(joined),
        "orangebook_multi_ingredient_product_rows": len(orangebook_source_rows),
        "orangebook_unique_pair_keys": len(orangebook_index_rows),
        "orangebook_candidate_hits": sum(1 for row in joined if row["orangebook_status"] != "no_external_hit"),
        "ndc_multi_ingredient_product_rows": len(ndc_source_rows),
        "ndc_unique_pair_keys": len(ndc_index_rows),
        "ndc_candidate_hits": sum(1 for row in joined if row["ndc_status"] != "no_external_hit"),
        "pubmed_unique_pair_queries": len(esearch_rows),
        "pubmed_unique_pair_queries_with_hits": sum(1 for row in esearch_rows if int(row.get("count") or 0) > 0),
        "pubmed_candidate_rows_with_hits": sum(1 for row in joined if row["pubmed_literature_status"] != "no_external_hit"),
        "pubmed_evidence_rows": len(pubmed_evidence),
        "pubmed_esummary_response_chunks": len(esummary_rows),
        "pubmed_unique_pmids": len({row["pmid"] for row in pubmed_evidence}),
        "candidate_rows_with_any_issue1234_hit": sum(
            1 for row in joined if row["overall_external_source_status"] != "no_external_hit"
        ),
        "remaining_no_hit_after_issue1234": sum(
            1 for row in joined if row["overall_external_source_status"] == "no_external_hit"
        ),
        "overall_external_source_status_counts": dict(overall_counts),
        "orangebook_status_counts": dict(orangebook_counts),
        "ndc_status_counts": dict(ndc_counts),
        "pubmed_literature_status_counts": dict(pubmed_counts),
        "source_type_counts": dict(sorted(source_type_counts.items())),
        "reason_code_counts": dict(reason_counts),
        "clinical_boundary_rows": sum(1 for row in joined if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_pubmed_hits": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "overall_external_source_status": row["overall_external_source_status"],
                "pubmed_count": row["pubmed_count"],
                "pmids": row["pubmed_idlist"][:5],
            }
            for row in joined
            if row["pubmed_literature_status"] != "no_external_hit"
        ][:25],
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidates: list[dict[str, Any]],
    orangebook_schema: dict[str, Any],
    ndc_schema: dict[str, Any],
    *,
    request_sleep_seconds: float,
    pubmed_retmax: int,
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1234,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1232_clinicaltrials_pair_status": artifact(
                Path(inputs["issue1232_clinicaltrials_pair_status"]), jsonl=True
            ),
            "fda_orangebook_zip": {
                **artifact(Path(inputs["fda_orangebook_zip"])),
                "source_page_url": FDA_ORANGEBOOK_PAGE_URL,
                "download_url": FDA_ORANGEBOOK_DOWNLOAD_URL,
                "license_or_terms_observation": "FDA public data download page; no separate license file observed in zip.",
            },
            "fda_orangebook_page": {
                **artifact(Path(inputs["fda_orangebook_page"])),
                "source_page_url": FDA_ORANGEBOOK_PAGE_URL,
            },
            "fda_ndc_zip": {
                **artifact(Path(inputs["fda_ndc_zip"])),
                "source_page_url": FDA_NDC_PAGE_URL,
                "download_url": FDA_NDC_DOWNLOAD_URL,
                "license_or_terms_observation": (
                    "FDA public NDC text download; directory inclusion is not FDA approval."
                ),
            },
            "fda_ndc_page": {
                **artifact(Path(inputs["fda_ndc_page"])),
                "source_page_url": FDA_NDC_PAGE_URL,
            },
            "ncbi_eutilities_intro": {
                **artifact(Path(inputs["ncbi_eutilities_intro"])),
                "source_page_url": NCBI_EUTILITIES_INTRO_URL,
            },
            "nlm_eutilities_guide": {
                **artifact(Path(inputs["nlm_eutilities_guide"])),
                "source_page_url": NLM_EUTILITIES_GUIDE_URL,
            },
        },
        "accepted_sources": [
            {
                "source": "FDA Orange Book current downloadable data files",
                "role": "current FDA product ingredient co-occurrence recheck",
                "source_page_url": FDA_ORANGEBOOK_PAGE_URL,
                "download_url": FDA_ORANGEBOOK_DOWNLOAD_URL,
                "source_schema_fingerprint_sha256": orangebook_schema["schema_fingerprint_sha256"],
                "license_or_terms_observation": "FDA public data download page; no separate license file observed in zip.",
            },
            {
                "source": "FDA National Drug Code Directory current text data",
                "role": "current FDA product-listing active-ingredient co-occurrence recheck",
                "source_page_url": FDA_NDC_PAGE_URL,
                "download_url": FDA_NDC_DOWNLOAD_URL,
                "source_schema_fingerprint_sha256": ndc_schema["schema_fingerprint_sha256"],
                "license_or_terms_observation": (
                    "FDA public NDC text download; directory inclusion is not FDA approval."
                ),
            },
            {
                "source": "PubMed E-utilities ESearch/ESummary",
                "role": "current literature title/abstract co-mention recheck",
                "source_page_url": NCBI_EUTILITIES_INTRO_URL,
                "guide_url": NLM_EUTILITIES_GUIDE_URL,
                "esearch_url": PUBMED_ESEARCH_URL,
                "esummary_url": PUBMED_ESUMMARY_URL,
                "retmax": pubmed_retmax,
                "request_sleep_seconds": request_sleep_seconds,
                "rate_limit_policy": "No API key; script sleeps at least 0.40 seconds between requests.",
            },
        ],
        "query_universe": {
            "remaining_no_hit_rows": len(candidates),
            "source": "issue1232 clinicaltrials_pair_status rows with clinicaltrials_current_status=no_external_hit",
        },
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    joined: list[dict[str, Any]],
    esearch_rows: list[dict[str, Any]],
    pubmed_evidence: list[dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "fda_orangebook_source_products": artifact(out_dir / "fda_orangebook_source_products.jsonl", jsonl=True),
        "fda_orangebook_pair_index": artifact(out_dir / "fda_orangebook_pair_index.jsonl", jsonl=True),
        "fda_ndc_source_products": artifact(out_dir / "fda_ndc_source_products.jsonl", jsonl=True),
        "fda_ndc_pair_index": artifact(out_dir / "fda_ndc_pair_index.jsonl", jsonl=True),
        "pubmed_esearch_responses": artifact(out_dir / "pubmed_esearch_responses.jsonl", jsonl=True),
        "pubmed_esummary_responses": artifact(out_dir / "pubmed_esummary_responses.jsonl", jsonl=True),
        "pubmed_pair_literature_evidence": artifact(out_dir / "pubmed_pair_literature_evidence.jsonl", jsonl=True),
        "candidate_external_source_status": artifact(out_dir / "candidate_external_source_status.jsonl", jsonl=True),
        "candidate_external_source_hits": artifact(out_dir / "candidate_external_source_hits.jsonl", jsonl=True),
        "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    unique_query_keys = {row["pair_key"] for row in candidates}
    return {
        "schema_version": 1,
        "issue": 1234,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "status_rows_for_every_input_row": len(joined) == len(candidates),
            "deterministic_status_for_every_row": all(
                row["overall_external_source_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                and row["orangebook_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                and row["ndc_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                and row["pubmed_literature_status"] in {"normalized_hit", "no_external_hit"}
                for row in joined
            ),
            "all_joined_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in joined),
            "pubmed_esearch_for_every_unique_pair": {row["pair_key"] for row in esearch_rows} == unique_query_keys,
            "all_pubmed_hit_rows_have_idlist": all(
                bool(row.get("pubmed_idlist"))
                for row in joined
                if row["pubmed_literature_status"] != "no_external_hit"
            ),
            "all_pubmed_evidence_rows_have_pmids": all(bool(row.get("pmid")) for row in pubmed_evidence),
            "bridge_rows_1000_or_less": artifacts["external_combo_bridge_rows"]["rows"] == min(1000, len(joined)),
        },
        "row_counts": {
            "input_candidate_rows": len(candidates),
            "joined_status_rows": len(joined),
            "pubmed_unique_queries": len(esearch_rows),
            "pubmed_evidence_rows": len(pubmed_evidence),
        },
    }


def run(
    root: Path,
    inputs: dict[str, str],
    *,
    max_rows: int | None,
    pubmed_retmax: int,
    request_sleep_seconds: float,
    email: str,
) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    prior_rows = rows_jsonl(Path(inputs["issue1232_clinicaltrials_pair_status"]))
    candidates = remaining_no_hit_rows(prior_rows)
    if max_rows is not None:
        candidates = candidates[:max_rows]

    orangebook_source_rows, orangebook_index, orangebook_schema = build_orangebook_index(
        Path(inputs["fda_orangebook_zip"])
    )
    ndc_source_rows, ndc_index, ndc_schema = build_ndc_index(Path(inputs["fda_ndc_zip"]))
    orangebook_index_rows = orangebook_schema.pop("pair_rows")
    ndc_index_rows = ndc_schema.pop("pair_rows")

    write_jsonl(out_dir / "fda_orangebook_source_products.jsonl", orangebook_source_rows)
    write_jsonl(out_dir / "fda_orangebook_pair_index.jsonl", orangebook_index_rows)
    write_json(out_dir / "fda_orangebook_source_schema.json", orangebook_schema)
    write_jsonl(out_dir / "fda_ndc_source_products.jsonl", ndc_source_rows)
    write_jsonl(out_dir / "fda_ndc_pair_index.jsonl", ndc_index_rows)
    write_json(out_dir / "fda_ndc_source_schema.json", ndc_schema)

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            candidates,
            orangebook_schema,
            ndc_schema,
            request_sleep_seconds=request_sleep_seconds,
            pubmed_retmax=pubmed_retmax,
        ),
    )

    query_rows = unique_pubmed_queries(candidates)
    esearch_rows = fetch_pubmed_esearch(
        query_rows,
        out_dir / "pubmed_esearch_responses.jsonl",
        retmax=pubmed_retmax,
        request_sleep_seconds=request_sleep_seconds,
        email=email,
    )
    esummary_rows, summaries = fetch_pubmed_esummary(
        esearch_rows,
        out_dir / "pubmed_esummary_responses.jsonl",
        request_sleep_seconds=request_sleep_seconds,
        email=email,
    )
    pubmed_evidence = pubmed_evidence_rows(esearch_rows, summaries)
    write_jsonl(out_dir / "pubmed_pair_literature_evidence.jsonl", pubmed_evidence)

    pubmed_by_pair_key = {row["pair_key"]: row for row in esearch_rows}
    joined = source_status_rows(candidates, orangebook_index, ndc_index, pubmed_by_pair_key)
    write_jsonl(out_dir / "candidate_external_source_status.jsonl", joined)
    hits = [row for row in joined if row["overall_external_source_status"] != "no_external_hit"]
    write_jsonl(out_dir / "candidate_external_source_hits.jsonl", hits)
    source_path = out_dir / "candidate_external_source_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(joined, source_path, source_sha)
    write_jsonl(out_dir / "external_combo_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(
        candidates,
        joined,
        orangebook_source_rows,
        orangebook_index_rows,
        ndc_source_rows,
        ndc_index_rows,
        esearch_rows,
        esummary_rows,
        pubmed_evidence,
    )
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1234,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "fda_orangebook_source_schema": artifact(out_dir / "fda_orangebook_source_schema.json"),
            "fda_orangebook_source_products": artifact(out_dir / "fda_orangebook_source_products.jsonl", jsonl=True),
            "fda_orangebook_pair_index": artifact(out_dir / "fda_orangebook_pair_index.jsonl", jsonl=True),
            "fda_ndc_source_schema": artifact(out_dir / "fda_ndc_source_schema.json"),
            "fda_ndc_source_products": artifact(out_dir / "fda_ndc_source_products.jsonl", jsonl=True),
            "fda_ndc_pair_index": artifact(out_dir / "fda_ndc_pair_index.jsonl", jsonl=True),
            "pubmed_esearch_responses": artifact(out_dir / "pubmed_esearch_responses.jsonl", jsonl=True),
            "pubmed_esummary_responses": artifact(out_dir / "pubmed_esummary_responses.jsonl", jsonl=True),
            "pubmed_pair_literature_evidence": artifact(
                out_dir / "pubmed_pair_literature_evidence.jsonl", jsonl=True
            ),
            "candidate_external_source_status": artifact(
                out_dir / "candidate_external_source_status.jsonl", jsonl=True
            ),
            "candidate_external_source_hits": artifact(
                out_dir / "candidate_external_source_hits.jsonl", jsonl=True
            ),
            "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, candidates, joined, esearch_rows, pubmed_evidence)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "candidate_external_source_status": output_manifest["artifacts"]["candidate_external_source_status"],
            "candidate_external_source_hits": output_manifest["artifacts"]["candidate_external_source_hits"],
            "pubmed_pair_literature_evidence": output_manifest["artifacts"]["pubmed_pair_literature_evidence"],
            "bridge_rows": output_manifest["artifacts"]["external_combo_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def inputs_for_root(root: Path) -> dict[str, str]:
    inputs = dict(DEFAULT_INPUTS)
    inputs["fda_orangebook_zip"] = str(root / "raw" / "fda_orangebook_current.zip")
    inputs["fda_orangebook_page"] = str(root / "raw" / "fda_orangebook_data_files.html")
    inputs["fda_ndc_zip"] = str(root / "raw" / "fda_ndc_current_text.zip")
    inputs["fda_ndc_page"] = str(root / "raw" / "fda_ndc_directory.html")
    inputs["ncbi_eutilities_intro"] = str(root / "raw" / "ncbi_eutilities_intro.html")
    inputs["nlm_eutilities_guide"] = str(root / "raw" / "nlm_eutilities_guide.html")
    return inputs


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1232-clinicaltrials-pair-status")
    parser.add_argument("--fda-orangebook-zip")
    parser.add_argument("--fda-orangebook-page")
    parser.add_argument("--fda-ndc-zip")
    parser.add_argument("--fda-ndc-page")
    parser.add_argument("--ncbi-eutilities-intro")
    parser.add_argument("--nlm-eutilities-guide")
    parser.add_argument("--max-rows", type=int)
    parser.add_argument("--pubmed-retmax", type=int, default=PUBMED_RETTYPE_MAX)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    parser.add_argument("--email", default=PUBMED_EMAIL)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    inputs = inputs_for_root(root)
    if args.issue1232_clinicaltrials_pair_status:
        inputs["issue1232_clinicaltrials_pair_status"] = args.issue1232_clinicaltrials_pair_status
    if args.fda_orangebook_zip:
        inputs["fda_orangebook_zip"] = args.fda_orangebook_zip
    if args.fda_orangebook_page:
        inputs["fda_orangebook_page"] = args.fda_orangebook_page
    if args.fda_ndc_zip:
        inputs["fda_ndc_zip"] = args.fda_ndc_zip
    if args.fda_ndc_page:
        inputs["fda_ndc_page"] = args.fda_ndc_page
    if args.ncbi_eutilities_intro:
        inputs["ncbi_eutilities_intro"] = args.ncbi_eutilities_intro
    if args.nlm_eutilities_guide:
        inputs["nlm_eutilities_guide"] = args.nlm_eutilities_guide
    result = run(
        root,
        inputs,
        max_rows=args.max_rows,
        pubmed_retmax=args.pubmed_retmax,
        request_sleep_seconds=args.request_sleep_seconds,
        email=args.email,
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
