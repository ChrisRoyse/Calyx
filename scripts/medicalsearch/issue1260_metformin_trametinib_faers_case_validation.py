#!/usr/bin/env python3
"""#1260 case-level validation for metformin/trametinib FAERS blocker."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


UTIL_PATH = Path(__file__).with_name("issue1259_rxnorm_twosides_safety_validation.py")
SPEC = importlib.util.spec_from_file_location("issue1259_utils", UTIL_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"Unable to import helpers from {UTIL_PATH}")
UTIL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UTIL)


CLINICAL_BOUNDARY = (
    "FAERS case-level validation is safety/source/falsification triage only; "
    "case reports, label text, RxNorm identity rows, and literature rows are "
    "blockers or review inputs, not causality, safety clearance, efficacy, "
    "treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, pair-interaction proof, or cure evidence."
)

PROMOTION_STATUS = "blocked_requires_case_level_safety_falsification_and_human_review"
PAIR_KEY = "metformin||trametinib dimethyl sulfoxide"
FAERS_SOURCE_ID = "24608768"
SOURCE_DATASET = "issue1260_metformin_trametinib_faers_case_validation"

ISSUE1259_ROOT = "/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1260-metformin-trametinib-faers-case-20260705T020000Z"

DEFAULT_INPUTS = {
    "issue1259_evidence": f"{ISSUE1259_ROOT}/out/independent_evidence_rows.jsonl",
    "issue1259_pair_rollups": f"{ISSUE1259_ROOT}/out/pair_validation_rollups.jsonl",
    "issue1259_candidate_status": f"{ISSUE1259_ROOT}/out/candidate_validation_status.jsonl",
    "issue1259_calyx_readback": f"{ISSUE1259_ROOT}/out/calyx_bridge_corpus_readback.json",
}

EXPECTED_INPUT_SHA256 = {
    "issue1259_evidence": "6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca",
    "issue1259_pair_rollups": "186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c",
    "issue1259_candidate_status": "b6a3eeaa15add12f23b57ae2956c6dfddfadb78ab45788e7a4f4c1e0de843700",
    "issue1259_calyx_readback": "10462d52057fa323443d1e2ae8c0fe752c464b2424a1e726eb3dc1b6a1680ce1",
}

OPENFDA_EVENT_ENDPOINT = "https://api.fda.gov/drug/event.json"
OPENFDA_LABEL_ENDPOINT = "https://api.fda.gov/drug/label.json"
DAILYMED_SPLS_ENDPOINT = "https://dailymed.nlm.nih.gov/dailymed/services/v2/spls.json"
DAILYMED_SPL_XML_BASE = "https://dailymed.nlm.nih.gov/dailymed/services/v2/spls"
DAILYMED_LABEL_URL = "https://dailymed.nlm.nih.gov/dailymed/drugInfo.cfm"
EUROPEPMC_SEARCH_ENDPOINT = "https://www.ebi.ac.uk/europepmc/webservices/rest/search"
PUBMED_ESEARCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi"
PUBMED_EFETCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi"
PUBMED_RECORD_URL = "https://pubmed.ncbi.nlm.nih.gov/{pmid}/"
RXNAV_RXCUI_URL = "https://rxnav.nlm.nih.gov/REST/rxcui.json"
RXNAV_RELATED_URL = "https://rxnav.nlm.nih.gov/REST/rxcui/{rxcui}/related.json"

SOURCE_DOCS = {
    "openfda_event_docs": "https://open.fda.gov/apis/drug/event/",
    "openfda_label_docs": "https://open.fda.gov/apis/drug/label/",
    "dailymed_spls_api": "https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm",
    "europepmc_rest_docs": "https://europepmc.org/RestfulWebService",
    "ncbi_eutilities_intro": "https://www.ncbi.nlm.nih.gov/books/NBK25497/",
    "rxnorm_find_rxcui": "https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.findRxcuiByString.html",
    "rxnorm_related": "https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getRelatedByType.html",
}

LABEL_TERMS = [
    {"term": "METFORMIN", "role": "pair_drug"},
    {"term": "TRAMETINIB", "role": "pair_drug"},
    {"term": "TRAMETINIB DIMETHYL SULFOXIDE", "role": "pair_drug_salt"},
    {"term": "MEKINIST", "role": "trametinib_brand"},
    {"term": "ELIQUIS", "role": "event_primary_suspect_confounder"},
    {"term": "APIXABAN", "role": "event_primary_suspect_confounder"},
]

RXNORM_TERMS = [item["term"] for item in LABEL_TERMS]

LITERATURE_QUERIES = [
    {"query_id": "metformin_trametinib", "left": "metformin", "right": "trametinib", "extra": ""},
    {"query_id": "metformin_mekinist", "left": "metformin", "right": "Mekinist", "extra": ""},
    {
        "query_id": "metformin_trametinib_dimethyl_sulfoxide",
        "left": "metformin",
        "right": "trametinib dimethyl sulfoxide",
        "extra": "",
    },
    {
        "query_id": "metformin_trametinib_bleeding",
        "left": "metformin",
        "right": "trametinib",
        "extra": '(bleeding OR hemorrhage OR haemorrhage OR "gastrointestinal hemorrhage")',
    },
]

LABEL_TEXT_FIELDS = [
    "boxed_warning",
    "contraindications",
    "warnings",
    "warnings_and_cautions",
    "precautions",
    "drug_interactions",
    "drug_interactions_table",
    "adverse_reactions",
    "adverse_reactions_table",
    "clinical_pharmacology",
    "clinical_studies",
    "use_in_specific_populations",
    "description",
]

SAFETY_KEYWORDS = [
    "haemorrhage",
    "hemorrhage",
    "bleeding",
    "gastrointestinal",
    "lower gastrointestinal",
    "off label",
    "lactic acidosis",
    "renal",
    "anticoagulant",
]

ANTICOAGULANT_TERMS = {"ELIQUIS", "APIXABAN", "WARFARIN", "RIVAROXABAN", "XARELTO", "DABIGATRAN", "PRADAXA"}
REQUEST_SLEEP_SECONDS = 0.12
USER_AGENT = "calyx-discovery/issue1260"
MAX_BRIDGE_ROWS = 1000


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def fetch_bytes(url: str, retries: int = 4) -> tuple[int, bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=90) as response:
                return int(response.status), response.read()
        except urllib.error.HTTPError as error:
            payload = error.read()
            if int(error.code) in {429, 500, 502, 503, 504} and attempt < retries - 1:
                retry_after = error.headers.get("Retry-After")
                try:
                    sleep_seconds = float(retry_after) if retry_after else 2.0 * (attempt + 1)
                except ValueError:
                    sleep_seconds = 2.0 * (attempt + 1)
                time.sleep(min(20.0, sleep_seconds))
                continue
            return int(error.code), payload
        except (urllib.error.URLError, TimeoutError) as error:
            last_error = error
            time.sleep(min(8.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"Fetch failed after {retries} attempts for {url}: {last_error}")


def decode_json(payload: bytes) -> dict[str, Any]:
    if not payload:
        return {}
    try:
        value = json.loads(payload.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return {"decode_error": payload.decode("utf-8", errors="replace")[:1000]}
    return value if isinstance(value, dict) else {"value": value}


def write_raw(raw_dir: Path, prefix: str, key: str, payload: bytes, suffix: str = ".json") -> Path:
    raw_dir.mkdir(parents=True, exist_ok=True)
    path = raw_dir / f"{prefix}_{UTIL.stable_id(key, UTIL.sha256_bytes(payload))}{suffix}"
    path.write_bytes(payload)
    return path


def verify_inputs(inputs: dict[str, str], skip: bool = False) -> dict[str, dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    missing = [name for name, value in inputs.items() if not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")
    for name, expected in EXPECTED_INPUT_SHA256.items():
        path = Path(inputs[name])
        observed = UTIL.sha256_path(path)
        ok = observed == expected
        if not ok and not skip:
            raise RuntimeError(f"Input hash mismatch for {name}: observed {observed} expected {expected}")
        rows[name] = {"path": str(path), "sha256": observed, "expected_sha256": expected, "match": ok}
    return rows


def fetch_source_docs(raw_dir: Path) -> list[dict[str, Any]]:
    rows = []
    for name, url in SOURCE_DOCS.items():
        status, payload = fetch_bytes(url)
        path = raw_dir / f"{name}.html"
        status_path = raw_dir / f"{name}.html.status"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        status_row = {
            "schema_version": 1,
            "source_name": name,
            "source_url": url,
            "http_status": status,
            "path": str(path),
            "bytes": len(payload),
            "sha256": UTIL.sha256_bytes(payload),
            "retrieved_at": now_utc(),
        }
        UTIL.write_json(status_path, status_row)
        rows.append(
            {
                **status_row,
                "source_row_id": "issue1260-source:" + UTIL.stable_id(name, url, status_row["sha256"]),
                "source_group": "source_documentation",
                "status_path": str(status_path),
                "status_sha256": UTIL.sha256_path(status_path),
                "text": f"Issue1260 source documentation {name} from {url} http status {status}.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def load_scope(inputs: dict[str, str]) -> tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]:
    evidence = [
        row
        for row in UTIL.rows_jsonl(Path(inputs["issue1259_evidence"]))
        if row.get("pair_key") == PAIR_KEY and row.get("source_id") == FAERS_SOURCE_ID
    ]
    if len(evidence) != 1:
        raise RuntimeError(f"Expected exactly one #1259 FAERS evidence row, got {len(evidence)}")
    rollups = [row for row in UTIL.rows_jsonl(Path(inputs["issue1259_pair_rollups"])) if row.get("pair_key") == PAIR_KEY]
    if len(rollups) != 1:
        raise RuntimeError(f"Expected exactly one #1259 pair rollup, got {len(rollups)}")
    candidates = [row for row in UTIL.rows_jsonl(Path(inputs["issue1259_candidate_status"])) if row.get("pair_key") == PAIR_KEY]
    if len(candidates) != 4:
        raise RuntimeError(f"Expected four #1259 candidate rows for pair, got {len(candidates)}")
    return evidence[0], rollups[0], candidates


def fetch_faers_case(raw_dir: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    query = f"safetyreportid:{FAERS_SOURCE_ID}"
    url = f"{OPENFDA_EVENT_ENDPOINT}?{urllib.parse.urlencode({'search': query, 'limit': '1'})}"
    status, payload = fetch_bytes(url)
    raw_path = write_raw(raw_dir, "openfda_event_exact", FAERS_SOURCE_ID, payload)
    data = decode_json(payload)
    results = data.get("results") if isinstance(data.get("results"), list) else []
    if status != 200 or len(results) != 1 or UTIL.clean_text(results[0].get("safetyreportid")) != FAERS_SOURCE_ID:
        raise RuntimeError(f"Exact FAERS event fetch failed status={status} results={len(results)}")
    query_row = {
        "schema_version": 1,
        "query_row_id": "issue1260-query:" + UTIL.stable_id("faers_exact", FAERS_SOURCE_ID, url),
        "source_type": "openfda_faers_exact_case",
        "source": "openFDA Drug Event API",
        "query": query,
        "query_url": url,
        "api_endpoint": OPENFDA_EVENT_ENDPOINT,
        "http_status": status,
        "raw_response_path": str(raw_path),
        "raw_response_bytes": len(payload),
        "raw_response_sha256": UTIL.sha256_bytes(payload),
        "total": int((data.get("meta", {}).get("results", {}) or {}).get("total") or 0),
        "returned_result_count": len(results),
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    return query_row, results[0]


def simplified_drugs(event: dict[str, Any]) -> list[dict[str, Any]]:
    drugs = (event.get("patient") or {}).get("drug") or []
    if isinstance(drugs, dict):
        drugs = [drugs]
    rows = []
    for index, drug in enumerate(drugs if isinstance(drugs, list) else [], start=1):
        if not isinstance(drug, dict):
            continue
        rows.append(
            {
                "index": index,
                "medicinalproduct": UTIL.clean_text(drug.get("medicinalproduct")),
                "drugcharacterization": UTIL.clean_text(drug.get("drugcharacterization")),
                "drugindication": UTIL.clean_text(drug.get("drugindication")),
                "drugdosagetext": UTIL.clean_text(drug.get("drugdosagetext")),
                "actiondrug": UTIL.clean_text(drug.get("actiondrug")),
                "drugstartdate": UTIL.clean_text(drug.get("drugstartdate")),
                "drugenddate": UTIL.clean_text(drug.get("drugenddate")),
            }
        )
    return rows


def reaction_terms(event: dict[str, Any]) -> list[str]:
    reactions = (event.get("patient") or {}).get("reaction") or []
    if isinstance(reactions, dict):
        reactions = [reactions]
    return UTIL.uniq([item.get("reactionmeddrapt") for item in reactions if isinstance(item, dict)])


def seriousness(event: dict[str, Any]) -> dict[str, Any]:
    fields = [
        "serious",
        "seriousnessdeath",
        "seriousnesslifethreatening",
        "seriousnesshospitalization",
        "seriousnessdisabling",
        "seriousnesscongenitalanomali",
        "seriousnessother",
    ]
    flags = {field: UTIL.clean_text(event.get(field)) for field in fields if UTIL.clean_text(event.get(field))}
    return {"flags": flags, "serious": any(value == "1" for value in flags.values()), "death": flags.get("seriousnessdeath") == "1"}


def build_faers_case_row(query_row: dict[str, Any], event: dict[str, Any], source_evidence: dict[str, Any]) -> dict[str, Any]:
    drugs = simplified_drugs(event)
    drug_names = [row["medicinalproduct"] for row in drugs]
    suspected = [row for row in drugs if row["drugcharacterization"] == "1"]
    concomitant = [row for row in drugs if row["drugcharacterization"] == "2"]
    pair_presence = {
        "trametinib": any(UTIL.exact_presence(name, "TRAMETINIB DIMETHYL SULFOXIDE")["present"] or UTIL.exact_presence(name, "TRAMETINIB")["present"] for name in drug_names),
        "metformin": any(UTIL.exact_presence(name, "METFORMIN")["present"] for name in drug_names),
    }
    anticoagulants = [name for name in drug_names if UTIL.norm_name(name).upper() in ANTICOAGULANT_TERMS or name.upper() in ANTICOAGULANT_TERMS]
    event_text = UTIL.clean_text({"drugs": drugs, "reactions": reaction_terms(event), "seriousness": seriousness(event)})
    return {
        "schema_version": 1,
        "faers_case_id": "issue1260-faers-case:" + UTIL.stable_id(FAERS_SOURCE_ID),
        "pair_key": PAIR_KEY,
        "source_id": FAERS_SOURCE_ID,
        "source_issue1259_evidence_id": source_evidence["evidence_id"],
        "source_url": query_row["query_url"],
        "source_response_sha256": query_row["raw_response_sha256"],
        "source_text_sha256": UTIL.sha256_bytes(event_text.encode("utf-8")),
        "receivedate": UTIL.clean_text(event.get("receivedate")),
        "receiptdate": UTIL.clean_text(event.get("receiptdate")),
        "seriousness": seriousness(event),
        "reactions": reaction_terms(event),
        "drug_count": len(drugs),
        "drugs": drugs,
        "suspected_drugs": suspected,
        "concomitant_drugs": concomitant,
        "pair_drug_presence": pair_presence,
        "pair_drugs_are_concomitant": all(
            any(UTIL.exact_presence(row["medicinalproduct"], term)["present"] for row in concomitant)
            for term in ["TRAMETINIB", "METFORMIN"]
        ),
        "anticoagulant_confounders": anticoagulants,
        "primary_suspect_not_pair_drug": bool(suspected)
        and not any(UTIL.exact_presence(row["medicinalproduct"], "TRAMETINIB")["present"] or UTIL.exact_presence(row["medicinalproduct"], "METFORMIN")["present"] for row in suspected),
        "polypharmacy_count": len(drugs),
        "classification": "serious_faers_case_confounded_still_blocked",
        "promotion_status": PROMOTION_STATUS,
        "reason_codes": [
            "case_report_not_causality",
            "pair_drugs_concomitant_not_primary_suspect",
            "anticoagulant_confounder_present" if anticoagulants else "no_anticoagulant_confounder_detected",
            "polypharmacy_case_requires_human_review",
        ],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def rxnav_json_url(term: str) -> str:
    return f"{RXNAV_RXCUI_URL}?{urllib.parse.urlencode({'name': term, 'search': '2'})}"


def rxnav_related_url(rxcui: str) -> str:
    return f"{RXNAV_RELATED_URL.format(rxcui=urllib.parse.quote(rxcui))}?{urllib.parse.urlencode({'tty': 'IN PIN MIN', 'expand': 'psn'})}"


def fetch_rxnorm_rows(raw_dir: Path) -> list[dict[str, Any]]:
    rows = []
    for term in RXNORM_TERMS:
        url = rxnav_json_url(term)
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "rxnav_rxcui", term, payload)
        data = decode_json(payload)
        rxcuis = [UTIL.clean_text(item) for item in ((data.get("idGroup") or {}).get("rxnormId") or [])]
        rows.append(
            {
                "schema_version": 1,
                "identity_row_id": "issue1260-rxnorm:" + UTIL.stable_id(term, "rxcui"),
                "term": term,
                "query_kind": "find_rxcui_exact_or_normalized",
                "query_url": url,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_sha256": UTIL.sha256_bytes(payload),
                "rxcuis": rxcuis,
                "response_json": data,
                "trusted_for_identity_review": bool(rxcuis),
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        for rxcui in rxcuis[:3]:
            related_url = rxnav_related_url(rxcui)
            rel_status, rel_payload = fetch_bytes(related_url)
            rel_raw_path = write_raw(raw_dir, "rxnav_related", f"{term}_{rxcui}", rel_payload)
            rows.append(
                {
                    "schema_version": 1,
                    "identity_row_id": "issue1260-rxnorm:" + UTIL.stable_id(term, rxcui, "related"),
                    "term": term,
                    "query_kind": "related_ingredients",
                    "rxcui": rxcui,
                    "query_url": related_url,
                    "http_status": rel_status,
                    "raw_response_path": str(rel_raw_path),
                    "raw_response_sha256": UTIL.sha256_bytes(rel_payload),
                    "response_json": decode_json(rel_payload),
                    "trusted_for_identity_review": True,
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
            time.sleep(REQUEST_SLEEP_SECONDS)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def openfda_label_url(term: str) -> str:
    clauses = []
    for field in ["openfda.generic_name", "openfda.brand_name", "openfda.substance_name"]:
        clauses.append(urllib.parse.quote(f'{field}:"{term}"', safe=".:"))
    return f"{OPENFDA_LABEL_ENDPOINT}?search={'+OR+'.join(clauses)}&limit=5"


def dailymed_spls_url(term: str) -> str:
    return f"{DAILYMED_SPLS_ENDPOINT}?{urllib.parse.urlencode({'drug_name': term, 'name_type': 'both', 'pagesize': '5', 'page': '1'})}"


def dailymed_spl_xml_url(setid: str) -> str:
    return f"{DAILYMED_SPL_XML_BASE}/{urllib.parse.quote(setid)}.xml"


def fetch_label_query_rows(raw_dir: Path) -> list[dict[str, Any]]:
    rows = []
    for term_row in LABEL_TERMS:
        term = term_row["term"]
        url = openfda_label_url(term)
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "openfda_label", term, payload)
        data = decode_json(payload)
        results = data.get("results") if isinstance(data.get("results"), list) else []
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1260-label-query:" + UTIL.stable_id("openfda", term, url),
                "source_type": "openfda_label",
                "term": term,
                "term_role": term_row["role"],
                "query_url": url,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_sha256": UTIL.sha256_bytes(payload),
                "total": int((data.get("meta", {}).get("results", {}) or {}).get("total") or 0),
                "returned_result_count": len(results),
                "response_json": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        time.sleep(REQUEST_SLEEP_SECONDS)

        daily_url = dailymed_spls_url(term)
        daily_status, daily_payload = fetch_bytes(daily_url)
        daily_raw_path = write_raw(raw_dir, "dailymed_spls", term, daily_payload)
        daily_data = decode_json(daily_payload)
        items = daily_data.get("data") if isinstance(daily_data.get("data"), list) else []
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1260-label-query:" + UTIL.stable_id("dailymed", term, daily_url),
                "source_type": "dailymed_spl_metadata",
                "term": term,
                "term_role": term_row["role"],
                "query_url": daily_url,
                "http_status": daily_status,
                "raw_response_path": str(daily_raw_path),
                "raw_response_sha256": UTIL.sha256_bytes(daily_payload),
                "total": len(items),
                "returned_result_count": len(items),
                "response_json": daily_data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        for item in items[:2]:
            setid = UTIL.clean_text(item.get("setid")) if isinstance(item, dict) else ""
            if not setid:
                continue
            xml_url = dailymed_spl_xml_url(setid)
            xml_status, xml_payload = fetch_bytes(xml_url)
            xml_raw_path = write_raw(raw_dir, "dailymed_spl_xml", f"{term}_{setid}", xml_payload, ".xml")
            rows.append(
                {
                    "schema_version": 1,
                    "query_row_id": "issue1260-label-query:" + UTIL.stable_id("dailymed_xml", term, setid),
                    "source_type": "dailymed_spl_xml",
                    "term": term,
                    "term_role": term_row["role"],
                    "setid": setid,
                    "title": UTIL.clean_text(item.get("title")),
                    "query_url": xml_url,
                    "http_status": xml_status,
                    "raw_response_path": str(xml_raw_path),
                    "raw_response_sha256": UTIL.sha256_bytes(xml_payload),
                    "total": 1 if xml_status == 200 else 0,
                    "returned_result_count": 1 if xml_status == 200 else 0,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
            time.sleep(REQUEST_SLEEP_SECONDS)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def label_text_sections(result: dict[str, Any]) -> list[dict[str, str]]:
    rows = []
    for field in LABEL_TEXT_FIELDS:
        text = UTIL.clean_text(result.get(field))
        if text:
            rows.append({"field": field, "text": text})
    return rows


def xml_text(path: Path) -> str:
    try:
        root = ET.fromstring(path.read_bytes())
    except ET.ParseError:
        return ""
    return UTIL.clean_text(" ".join(root.itertext()))


def keyword_hits(text: str) -> list[str]:
    norm = UTIL.normalized_blob(text)
    hits = []
    for keyword in SAFETY_KEYWORDS:
        if UTIL.norm_name(keyword) in norm:
            hits.append(keyword)
    return UTIL.uniq(hits)


def snippet(text: str, keywords: list[str], window: int = 180) -> str:
    lower = text.lower()
    indexes = [lower.find(keyword.lower()) for keyword in keywords if keyword and lower.find(keyword.lower()) >= 0]
    if not indexes:
        return UTIL.clean_text(text[: window * 2])
    start = max(0, min(indexes) - window)
    end = min(len(text), max(indexes) + window)
    return UTIL.clean_text(text[start:end])


def build_label_evidence(query_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for query in query_rows:
        if query["source_type"] == "openfda_label":
            for result_index, result in enumerate(query.get("response_json", {}).get("results") or [], start=1):
                if not isinstance(result, dict):
                    continue
                label_id = UTIL.clean_text(result.get("id") or result.get("set_id") or UTIL.stable_id(result, length=16))
                for section in label_text_sections(result):
                    hits = keyword_hits(section["text"])
                    if not hits:
                        continue
                    rows.append(
                        {
                            "schema_version": 1,
                            "label_evidence_id": "issue1260-label-evidence:" + UTIL.stable_id("openfda", query["term"], label_id, section["field"]),
                            "source_type": "openfda_label",
                            "term": query["term"],
                            "term_role": query["term_role"],
                            "source_id": label_id,
                            "source_url": f"{OPENFDA_LABEL_ENDPOINT}?search=id:%22{urllib.parse.quote(label_id)}%22",
                            "source_response_sha256": query["raw_response_sha256"],
                            "section": section["field"],
                            "keyword_hits": hits,
                            "snippet": snippet(section["text"], hits),
                            "classification": "label_safety_context",
                            "promotion_status": PROMOTION_STATUS,
                            "clinical_boundary": CLINICAL_BOUNDARY,
                        }
                    )
        elif query["source_type"] == "dailymed_spl_xml" and int(query.get("http_status") or 0) == 200:
            text = xml_text(Path(query["raw_response_path"]))
            hits = keyword_hits(text)
            if not hits:
                continue
            rows.append(
                {
                    "schema_version": 1,
                    "label_evidence_id": "issue1260-label-evidence:" + UTIL.stable_id("dailymed", query["term"], query.get("setid")),
                    "source_type": "dailymed_spl_xml",
                    "term": query["term"],
                    "term_role": query["term_role"],
                    "source_id": query.get("setid"),
                    "source_url": f"{DAILYMED_LABEL_URL}?{urllib.parse.urlencode({'setid': query.get('setid')})}",
                    "source_response_sha256": query["raw_response_sha256"],
                    "section": "spl_xml_text",
                    "keyword_hits": hits,
                    "snippet": snippet(text, hits),
                    "classification": "label_safety_context",
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    rows.sort(key=lambda row: (row["term"], row["source_type"], row["source_id"] or ""))
    return rows


def europepmc_url(query: str) -> str:
    params = {"query": query, "format": "json", "resultType": "core", "pageSize": "5", "cursorMark": "*", "synonym": "false"}
    return f"{EUROPEPMC_SEARCH_ENDPOINT}?{urllib.parse.urlencode(params)}"


def pubmed_esearch_url(query: str, email: str) -> str:
    params = {"db": "pubmed", "retmode": "json", "retmax": "5", "sort": "relevance", "term": query, "tool": "calyx", "email": email}
    return f"{PUBMED_ESEARCH_URL}?{urllib.parse.urlencode(params)}"


def pubmed_efetch_url(pmids: list[str], email: str) -> str:
    params = {"db": "pubmed", "retmode": "xml", "rettype": "abstract", "id": ",".join(pmids), "tool": "calyx", "email": email}
    return f"{PUBMED_EFETCH_URL}?{urllib.parse.urlencode(params)}"


def fetch_literature_rows(raw_dir: Path, email: str) -> list[dict[str, Any]]:
    rows = []
    for spec in LITERATURE_QUERIES:
        pair_query = f'"{spec["left"]}" AND "{spec["right"]}"'
        if spec["extra"]:
            pair_query = f"{pair_query} AND {spec['extra']}"
        epmc_url = europepmc_url(pair_query)
        epmc_status, epmc_payload = fetch_bytes(epmc_url)
        epmc_raw = write_raw(raw_dir, "europepmc", spec["query_id"], epmc_payload)
        epmc_data = decode_json(epmc_payload)
        results = ((epmc_data.get("resultList") or {}).get("result") or []) if isinstance(epmc_data, dict) else []
        rows.append(
            {
                "schema_version": 1,
                "literature_query_id": "issue1260-lit-query:" + UTIL.stable_id("europepmc", spec["query_id"]),
                "source_type": "europepmc",
                "query_spec": spec,
                "query": pair_query,
                "query_url": epmc_url,
                "http_status": epmc_status,
                "raw_response_path": str(epmc_raw),
                "raw_response_sha256": UTIL.sha256_bytes(epmc_payload),
                "total": int(epmc_data.get("hitCount") or 0) if str(epmc_data.get("hitCount") or "0").isdigit() else 0,
                "returned_result_count": len(results) if isinstance(results, list) else 0,
                "response_json": epmc_data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        time.sleep(REQUEST_SLEEP_SECONDS)

        pub_query = f'("{spec["left"]}"[Title/Abstract]) AND ("{spec["right"]}"[Title/Abstract])'
        if spec["extra"]:
            pub_query = f"{pub_query} AND {spec['extra']}"
        p_url = pubmed_esearch_url(pub_query, email)
        p_status, p_payload = fetch_bytes(p_url)
        p_raw = write_raw(raw_dir, "pubmed_esearch", spec["query_id"], p_payload)
        p_data = decode_json(p_payload)
        result = p_data.get("esearchresult") or {}
        idlist = [str(item) for item in result.get("idlist") or []]
        rows.append(
            {
                "schema_version": 1,
                "literature_query_id": "issue1260-lit-query:" + UTIL.stable_id("pubmed_esearch", spec["query_id"]),
                "source_type": "pubmed_esearch",
                "query_spec": spec,
                "query": pub_query,
                "query_url": p_url,
                "http_status": p_status,
                "raw_response_path": str(p_raw),
                "raw_response_sha256": UTIL.sha256_bytes(p_payload),
                "total": int(result.get("count") or 0),
                "returned_result_count": len(idlist),
                "idlist": idlist,
                "response_json": p_data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        if idlist:
            efetch_url = pubmed_efetch_url(idlist, email)
            e_status, e_payload = fetch_bytes(efetch_url)
            e_raw = write_raw(raw_dir, "pubmed_efetch", spec["query_id"], e_payload, ".xml")
            rows.append(
                {
                    "schema_version": 1,
                    "literature_query_id": "issue1260-lit-query:" + UTIL.stable_id("pubmed_efetch", spec["query_id"]),
                    "source_type": "pubmed_efetch",
                    "query_spec": spec,
                    "query": pub_query,
                    "query_url": efetch_url,
                    "http_status": e_status,
                    "raw_response_path": str(e_raw),
                    "raw_response_sha256": UTIL.sha256_bytes(e_payload),
                    "pmids": idlist,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def metadata_text(item: dict[str, Any]) -> str:
    return UTIL.clean_text([item.get("title"), item.get("abstractText"), item.get("keywordList"), item.get("journalInfo")])


def parse_pubmed_records(xml_bytes: bytes) -> list[dict[str, str]]:
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return []
    records = []
    for article in root.findall(".//PubmedArticle"):
        pmid = UTIL.clean_text(article.findtext(".//PMID"))
        title = UTIL.clean_text("".join(article.findtext(".//ArticleTitle") or ""))
        abstracts = [UTIL.clean_text("".join(item.itertext())) for item in article.findall(".//AbstractText")]
        journal = UTIL.clean_text(article.findtext(".//Journal/Title"))
        pub_year = UTIL.clean_text(article.findtext(".//PubDate/Year") or article.findtext(".//PubDate/MedlineDate"))
        records.append({"pmid": pmid, "title": title, "abstract": UTIL.clean_text(abstracts), "journal": journal, "pub_year": pub_year})
    return records


def both_terms_present(text: str, left: str, right: str) -> bool:
    return UTIL.exact_presence(text, left)["present"] and UTIL.exact_presence(text, right)["present"]


def build_literature_evidence(query_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for query in query_rows:
        spec = query.get("query_spec") or {}
        if query["source_type"] == "europepmc":
            result_list = query.get("response_json", {}).get("resultList") or {}
            for index, item in enumerate(result_list.get("result") or [], start=1):
                if not isinstance(item, dict):
                    continue
                text = metadata_text(item)
                if not both_terms_present(text, spec.get("left", ""), spec.get("right", "")):
                    continue
                source_id = UTIL.clean_text(item.get("id") or item.get("pmid") or item.get("pmcid") or UTIL.stable_id(item, length=16))
                rows.append(
                    {
                        "schema_version": 1,
                        "literature_evidence_id": "issue1260-lit-evidence:" + UTIL.stable_id("europepmc", spec.get("query_id"), source_id),
                        "source_type": "europepmc",
                        "query_id": spec.get("query_id"),
                        "source_id": source_id,
                        "source_response_sha256": query["raw_response_sha256"],
                        "title": UTIL.clean_text(item.get("title")),
                        "classification": "literature_comention",
                        "promotion_status": PROMOTION_STATUS,
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
        elif query["source_type"] == "pubmed_efetch":
            spec = query.get("query_spec") or {}
            for record in parse_pubmed_records(Path(query["raw_response_path"]).read_bytes()):
                text = UTIL.clean_text([record["title"], record["abstract"]])
                if not both_terms_present(text, spec.get("left", ""), spec.get("right", "")):
                    continue
                rows.append(
                    {
                        "schema_version": 1,
                        "literature_evidence_id": "issue1260-lit-evidence:" + UTIL.stable_id("pubmed", spec.get("query_id"), record["pmid"]),
                        "source_type": "pubmed",
                        "query_id": spec.get("query_id"),
                        "source_id": record["pmid"],
                        "source_url": PUBMED_RECORD_URL.format(pmid=record["pmid"]),
                        "source_response_sha256": query["raw_response_sha256"],
                        "title": record["title"],
                        "journal": record["journal"],
                        "publication_year": record["pub_year"],
                        "classification": "literature_title_or_abstract_comention",
                        "promotion_status": PROMOTION_STATUS,
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
    rows.sort(key=lambda row: (row["source_type"], row["source_id"]))
    return rows


def build_case_rollup(
    case_row: dict[str, Any],
    label_evidence: list[dict[str, Any]],
    literature_queries: list[dict[str, Any]],
    literature_evidence: list[dict[str, Any]],
    identity_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    label_counts_by_term = Counter(row["term"] for row in label_evidence)
    lit_query_counts = {row.get("query_spec", {}).get("query_id"): row.get("total", 0) for row in literature_queries if row["source_type"] in {"europepmc", "pubmed_esearch"}}
    row = {
        "schema_version": 1,
        "case_rollup_id": "issue1260-case-rollup:" + UTIL.stable_id(FAERS_SOURCE_ID, PAIR_KEY),
        "pair_key": PAIR_KEY,
        "source_id": FAERS_SOURCE_ID,
        "classification": "serious_faers_case_confounded_still_blocked",
        "case_summary": (
            "FAERS report contains trametinib and metformin as concomitant drugs, "
            "but Eliquis appears as primary suspect and an anticoagulant confounder; "
            "polypharmacy prevents pair-causality inference."
        ),
        "serious": case_row["seriousness"]["serious"],
        "reactions": case_row["reactions"],
        "drug_count": case_row["drug_count"],
        "suspected_drug_names": [row["medicinalproduct"] for row in case_row["suspected_drugs"]],
        "anticoagulant_confounders": case_row["anticoagulant_confounders"],
        "pair_drugs_are_concomitant": case_row["pair_drugs_are_concomitant"],
        "primary_suspect_not_pair_drug": case_row["primary_suspect_not_pair_drug"],
        "label_evidence_rows": len(label_evidence),
        "label_evidence_by_term": dict(label_counts_by_term),
        "literature_query_totals": lit_query_counts,
        "literature_evidence_rows": len(literature_evidence),
        "rxnorm_identity_rows": len(identity_rows),
        "promotion_status": PROMOTION_STATUS,
        "reason_codes": [
            "serious_faers_report_found",
            "eliquis_primary_suspect_anticoagulant_confounder_present",
            "trametinib_metformin_concomitant_not_primary_suspect",
            "polypharmacy_case_report_not_pair_causality",
            "requires_human_review",
        ],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    return [row]


def build_candidate_status(candidates: list[dict[str, Any]], case_rollup: dict[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for candidate in candidates:
        rows.append(
            {
                "schema_version": 1,
                "candidate_case_status_id": "issue1260-candidate-status:" + UTIL.stable_id(candidate["candidate_validation_status_id"], case_rollup["classification"]),
                "pair_id": candidate["pair_id"],
                "pair_key": PAIR_KEY,
                "source_issue1259_candidate_status_id": candidate["candidate_validation_status_id"],
                "case_rollup_id": case_rollup["case_rollup_id"],
                "case_validation_status": "candidate_serious_faers_case_confounded_still_blocked",
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": case_rollup["reason_codes"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def bridge_metadata(source_path: str | Path, source_sha: str, **extra: Any) -> dict[str, Any]:
    metadata = {
        "source_dataset": SOURCE_DATASET,
        "source_path": str(source_path),
        "source_sha256": source_sha,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    metadata.update(extra)
    return metadata


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    faers_query_rows: list[dict[str, Any]],
    case_rows: list[dict[str, Any]],
    identity_rows: list[dict[str, Any]],
    label_evidence: list[dict[str, Any]],
    literature_evidence: list[dict[str, Any]],
    case_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows = []
    for row in source_rows:
        terms = UTIL.uniq(["issue1260", row["source_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1260_source_snapshot",
                "text": f"{row['text']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["path"], row["sha256"]),
            }
        )
    for row in faers_query_rows:
        terms = UTIL.uniq([row["source_type"], FAERS_SOURCE_ID, row["raw_response_sha256"]])
        rows.append(
            {
                "id": row["query_row_id"],
                "domain": "issue1260_faers_query",
                "text": (
                    f"Issue1260 exact FAERS query {row['source_type']} source {FAERS_SOURCE_ID} "
                    f"sha256 {row['raw_response_sha256']} bridge terms {' '.join(terms)}."
                ),
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["raw_response_path"], row["raw_response_sha256"], query_url=row["query_url"]),
            }
        )
    for row in case_rows:
        terms = UTIL.uniq([PAIR_KEY, FAERS_SOURCE_ID, "ELIQUIS", "TRAMETINIB", "METFORMIN", row["classification"]])
        text = (
            f"Issue1260 FAERS case {FAERS_SOURCE_ID} pair {PAIR_KEY} classification {row['classification']} "
            f"drugs TRAMETINIB METFORMIN ELIQUIS reactions {' '.join(row['reactions'])} bridge terms {' '.join(terms)}."
        )
        rows.append(
            {
                "id": row["faers_case_id"],
                "domain": "issue1260_faers_case",
                "text": text,
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in case_rollups:
        terms = UTIL.uniq([PAIR_KEY, row["classification"], "ELIQUIS", "TRAMETINIB", "METFORMIN"])
        rows.append(
            {
                "id": row["case_rollup_id"],
                "domain": "issue1260_case_rollup",
                "text": f"Issue1260 case rollup {PAIR_KEY} {row['classification']} ELIQUIS TRAMETINIB METFORMIN bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in candidate_status:
        terms = UTIL.uniq([PAIR_KEY, row["case_validation_status"]])
        rows.append(
            {
                "id": row["candidate_case_status_id"],
                "domain": "issue1260_candidate_status",
                "text": f"Issue1260 candidate status {row['pair_id']} {PAIR_KEY} {row['case_validation_status']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in identity_rows[:80]:
        terms = UTIL.uniq([row["term"], row["query_kind"], *(row.get("rxcuis") or []), row.get("rxcui", "")])
        rows.append(
            {
                "id": row["identity_row_id"],
                "domain": "issue1260_rxnorm_identity",
                "text": f"Issue1260 RxNorm identity {row['term']} {row['query_kind']} {' '.join(row.get('rxcuis') or [row.get('rxcui','')])} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in label_evidence[:200]:
        terms = UTIL.uniq([row["term"], row["source_type"], row["classification"], *row["keyword_hits"]])
        rows.append(
            {
                "id": row["label_evidence_id"],
                "domain": "issue1260_label_evidence",
                "text": f"Issue1260 label evidence {row['term']} {row['source_type']} keywords {' '.join(row['keyword_hits'])} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    for row in literature_evidence[:100]:
        terms = UTIL.uniq([row["source_type"], row["classification"], row["source_id"], row.get("query_id", "")])
        rows.append(
            {
                "id": row["literature_evidence_id"],
                "domain": "issue1260_literature_evidence",
                "text": f"Issue1260 literature evidence {row['source_type']} {row['source_id']} {row['classification']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    return rows[:MAX_BRIDGE_ROWS]


def build_metrics(
    source_rows: list[dict[str, Any]],
    faers_query_rows: list[dict[str, Any]],
    case_rows: list[dict[str, Any]],
    identity_rows: list[dict[str, Any]],
    label_query_rows: list[dict[str, Any]],
    label_evidence: list[dict[str, Any]],
    literature_query_rows: list[dict[str, Any]],
    literature_evidence: list[dict[str, Any]],
    case_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "all_rows_blocked": True,
        "source_rows": len(source_rows),
        "faers_query_rows": len(faers_query_rows),
        "faers_case_rows": len(case_rows),
        "rxnorm_identity_rows": len(identity_rows),
        "label_query_rows": len(label_query_rows),
        "label_evidence_rows": len(label_evidence),
        "label_evidence_by_term": dict(Counter(row["term"] for row in label_evidence)),
        "literature_query_rows": len(literature_query_rows),
        "literature_evidence_rows": len(literature_evidence),
        "case_rollup_rows": len(case_rollups),
        "candidate_status_rows": len(candidate_status),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
        "case_classification_counts": dict(Counter(row["classification"] for row in case_rollups)),
    }


def build_readback(
    out_dir: Path,
    input_hashes: dict[str, dict[str, Any]],
    faers_query_rows: list[dict[str, Any]],
    case_rows: list[dict[str, Any]],
    case_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    artifact_paths = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_rows": out_dir / "source_rows.jsonl",
        "faers_query_rows": out_dir / "faers_query_rows.jsonl",
        "faers_case_rows": out_dir / "faers_case_rows.jsonl",
        "rxnorm_identity_rows": out_dir / "rxnorm_identity_rows.jsonl",
        "label_query_rows": out_dir / "label_query_rows.jsonl",
        "label_evidence_rows": out_dir / "label_evidence_rows.jsonl",
        "literature_query_rows": out_dir / "literature_query_rows.jsonl",
        "literature_evidence_rows": out_dir / "literature_evidence_rows.jsonl",
        "case_rollups": out_dir / "case_rollups.jsonl",
        "candidate_case_status": out_dir / "candidate_case_status.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
        "issue1260_bridge_rows": out_dir / "issue1260_bridge_rows.jsonl",
    }
    artifacts = {name: UTIL.artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifact_paths.items()}
    assertions = {
        "expected_input_hashes_match": all(item["match"] for item in input_hashes.values()),
        "one_faers_query_row": len(faers_query_rows) == 1,
        "faers_exact_query_total_one": len(faers_query_rows) == 1 and faers_query_rows[0]["total"] == 1,
        "faers_exact_query_returned_one": len(faers_query_rows) == 1 and faers_query_rows[0]["returned_result_count"] == 1,
        "faers_exact_raw_response_exists": len(faers_query_rows) == 1 and Path(faers_query_rows[0]["raw_response_path"]).exists(),
        "one_faers_case_row": len(case_rows) == 1,
        "faers_case_source_id_matches": case_rows[0]["source_id"] == FAERS_SOURCE_ID,
        "faers_case_has_pair_drugs": all(case_rows[0]["pair_drug_presence"].values()),
        "faers_case_is_serious": case_rows[0]["seriousness"]["serious"] is True,
        "faers_case_has_anticoagulant_confounder": bool(case_rows[0]["anticoagulant_confounders"]),
        "case_rollup_rows": len(case_rollups) == 1,
        "candidate_status_rows": len(candidate_status) == 4,
        "case_rollups_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in case_rollups),
        "candidate_status_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in candidate_status),
        "bridge_terms_present_in_text": all(
            all(UTIL.clean_text(term).lower() in UTIL.clean_text(row.get("text")).lower() for term in row.get("bridge_terms", []))
            for row in bridge_rows
        ),
        "bridge_rows_1000_or_less": len(bridge_rows) <= MAX_BRIDGE_ROWS,
    }
    return {
        "schema_version": 1,
        "status": "ok" if all(assertions.values()) else "failed",
        "assertions": assertions,
        "artifacts": artifacts,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1259-evidence")
    parser.add_argument("--issue1259-pair-rollups")
    parser.add_argument("--issue1259-candidate-status")
    parser.add_argument("--issue1259-calyx-readback")
    parser.add_argument("--skip-input-sha-check", action="store_true")
    parser.add_argument("--email", default="opensource@example.com")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.root)
    raw_dir = root / "raw"
    raw_responses_dir = raw_dir / "responses"
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    inputs = dict(DEFAULT_INPUTS)
    for key in list(inputs):
        override = getattr(args, key, None)
        if override:
            inputs[key] = override
    input_hashes = verify_inputs(inputs, skip=args.skip_input_sha_check)
    UTIL.write_json(out_dir / "input_manifest.json", {"schema_version": 1, "inputs": inputs, "input_hashes": input_hashes})

    source_evidence, source_rollup, source_candidates = load_scope(inputs)
    source_rows = fetch_source_docs(raw_dir)
    query_row, event = fetch_faers_case(raw_responses_dir)
    faers_query_rows = [query_row]
    faers_case_rows = [build_faers_case_row(query_row, event, source_evidence)]
    rxnorm_identity_rows = fetch_rxnorm_rows(raw_responses_dir)
    label_query_rows = fetch_label_query_rows(raw_responses_dir)
    label_evidence_rows = build_label_evidence(label_query_rows)
    literature_query_rows = fetch_literature_rows(raw_responses_dir, args.email)
    literature_evidence_rows = build_literature_evidence(literature_query_rows)
    case_rollups = build_case_rollup(
        faers_case_rows[0],
        label_evidence_rows,
        literature_query_rows,
        literature_evidence_rows,
        rxnorm_identity_rows,
    )
    candidate_status = build_candidate_status(source_candidates, case_rollups[0])

    UTIL.write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    UTIL.write_jsonl(out_dir / "faers_query_rows.jsonl", faers_query_rows)
    UTIL.write_jsonl(out_dir / "faers_case_rows.jsonl", faers_case_rows)
    UTIL.write_jsonl(out_dir / "rxnorm_identity_rows.jsonl", rxnorm_identity_rows)
    UTIL.write_jsonl(out_dir / "label_query_rows.jsonl", label_query_rows)
    UTIL.write_jsonl(out_dir / "label_evidence_rows.jsonl", label_evidence_rows)
    UTIL.write_jsonl(out_dir / "literature_query_rows.jsonl", literature_query_rows)
    UTIL.write_jsonl(out_dir / "literature_evidence_rows.jsonl", literature_evidence_rows)
    UTIL.write_jsonl(out_dir / "case_rollups.jsonl", case_rollups)
    UTIL.write_jsonl(out_dir / "candidate_case_status.jsonl", candidate_status)

    source_path = out_dir / "case_rollups.jsonl"
    source_sha = UTIL.sha256_path(source_path)
    bridge_rows = build_bridge_rows(
        source_rows,
        faers_query_rows,
        faers_case_rows,
        rxnorm_identity_rows,
        label_evidence_rows,
        literature_evidence_rows,
        case_rollups,
        candidate_status,
        source_path,
        source_sha,
    )
    UTIL.write_jsonl(out_dir / "issue1260_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(
        source_rows,
        faers_query_rows,
        faers_case_rows,
        rxnorm_identity_rows,
        label_query_rows,
        label_evidence_rows,
        literature_query_rows,
        literature_evidence_rows,
        case_rollups,
        candidate_status,
        bridge_rows,
    )
    UTIL.write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1260,
        "created_at": now_utc(),
        "inputs": inputs,
        "input_hashes": input_hashes,
        "artifacts": {
            "source_rows": UTIL.artifact(out_dir / "source_rows.jsonl", jsonl=True),
            "faers_query_rows": UTIL.artifact(out_dir / "faers_query_rows.jsonl", jsonl=True),
            "faers_case_rows": UTIL.artifact(out_dir / "faers_case_rows.jsonl", jsonl=True),
            "rxnorm_identity_rows": UTIL.artifact(out_dir / "rxnorm_identity_rows.jsonl", jsonl=True),
            "label_query_rows": UTIL.artifact(out_dir / "label_query_rows.jsonl", jsonl=True),
            "label_evidence_rows": UTIL.artifact(out_dir / "label_evidence_rows.jsonl", jsonl=True),
            "literature_query_rows": UTIL.artifact(out_dir / "literature_query_rows.jsonl", jsonl=True),
            "literature_evidence_rows": UTIL.artifact(out_dir / "literature_evidence_rows.jsonl", jsonl=True),
            "case_rollups": UTIL.artifact(out_dir / "case_rollups.jsonl", jsonl=True),
            "candidate_case_status": UTIL.artifact(out_dir / "candidate_case_status.jsonl", jsonl=True),
            "issue1260_bridge_rows": UTIL.artifact(out_dir / "issue1260_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": UTIL.artifact(out_dir / "validation_metrics.json"),
        },
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    UTIL.write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, input_hashes, faers_query_rows, faers_case_rows, case_rollups, candidate_status, bridge_rows)
    UTIL.write_json(out_dir / "persisted_readback.json", readback)
    if readback["status"] != "ok":
        raise RuntimeError(f"Persisted readback failed: {readback['assertions']}")

    print(
        json.dumps(
            {
                "status": "ok",
                "root": str(root),
                "metrics": metrics,
                "artifacts": {
                    "faers_query_rows": UTIL.artifact(out_dir / "faers_query_rows.jsonl", jsonl=True),
                    "faers_case_rows": UTIL.artifact(out_dir / "faers_case_rows.jsonl", jsonl=True),
                    "label_evidence_rows": UTIL.artifact(out_dir / "label_evidence_rows.jsonl", jsonl=True),
                    "literature_evidence_rows": UTIL.artifact(out_dir / "literature_evidence_rows.jsonl", jsonl=True),
                    "case_rollups": UTIL.artifact(out_dir / "case_rollups.jsonl", jsonl=True),
                    "candidate_status": UTIL.artifact(out_dir / "candidate_case_status.jsonl", jsonl=True),
                    "bridge_rows": UTIL.artifact(out_dir / "issue1260_bridge_rows.jsonl", jsonl=True),
                    "persisted_readback": UTIL.artifact(out_dir / "persisted_readback.json"),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
