#!/usr/bin/env python3
"""#1259 independent validation for RxNorm-rescued TwoSIDES safety hits."""

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
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Independent validation of RxNorm-rescued TwoSIDES pair hits is safety, "
    "source, outcome, and falsification triage only; adverse-event rows, label "
    "co-mentions, literature co-mentions, and registry/source hits are blockers "
    "or review inputs, not safety clearance, efficacy, treatment guidance, "
    "dosing guidance, recommendation, clinical actionability, pair-interaction "
    "proof, or cure evidence."
)

PROMOTION_STATUS = "blocked_requires_independent_safety_outcome_falsification_and_human_review"
SOURCE_EVIDENCE_KIND = "independent_source_validation_not_clinical_actionability"
TWOSIDES_KIND = "twosides_rxcui_pair_adverse_effect_source_match_not_clearance"

ISSUE1258_ROOT = "/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z"
ISSUE1255_ROOT = "/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z"

DEFAULT_INPUTS = {
    "issue1258_twosides_evidence": f"{ISSUE1258_ROOT}/out/rxnorm_twosides_pair_evidence.jsonl",
    "issue1258_pair_status": f"{ISSUE1258_ROOT}/out/rxnorm_pair_status.jsonl",
    "issue1258_candidate_status": f"{ISSUE1258_ROOT}/out/candidate_rxnorm_status.jsonl",
    "issue1258_term_status": f"{ISSUE1258_ROOT}/out/rxnorm_term_status.jsonl",
    "issue1258_persisted_readback": f"{ISSUE1258_ROOT}/out/persisted_readback.json",
    "issue1258_calyx_readback": f"{ISSUE1258_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1258_output_manifest": f"{ISSUE1258_ROOT}/out/output_manifest.json",
    "issue1255_pair_status": f"{ISSUE1255_ROOT}/out/drugcentral_pair_status.jsonl",
}

EXPECTED_INPUT_SHA256 = {
    "issue1258_twosides_evidence": "7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17",
    "issue1258_pair_status": "20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0",
    "issue1258_candidate_status": "cb9ca1d1a1dd8af195e4dd9830e1072c4c1bfe7e2197ef77f9ec26f2baeddba2",
    "issue1258_term_status": "8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7",
    "issue1258_persisted_readback": "0011f528f7af04e18154e83dc193c822a1171e9d753d3c67f5e4d53add04b55d",
    "issue1258_calyx_readback": "521f420f3b8b5f821064c3a341be76dcdd5ef18cc11da8f3235b5ff893913a50",
    "issue1258_output_manifest": "1c90ac1e495357da85a74a5c405423d59c914482aaae7036e3c6543216a0c48c",
}

OPENFDA_EVENT_ENDPOINT = "https://api.fda.gov/drug/event.json"
OPENFDA_LABEL_ENDPOINT = "https://api.fda.gov/drug/label.json"
DAILYMED_SPLS_ENDPOINT = "https://dailymed.nlm.nih.gov/dailymed/services/v2/spls.json"
DAILYMED_LABEL_URL = "https://dailymed.nlm.nih.gov/dailymed/drugInfo.cfm"
EUROPEPMC_SEARCH_ENDPOINT = "https://www.ebi.ac.uk/europepmc/webservices/rest/search"
PUBMED_ESEARCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi"
PUBMED_EFETCH_URL = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi"
PUBMED_RECORD_URL = "https://pubmed.ncbi.nlm.nih.gov/{pmid}/"

SOURCE_DOCS = {
    "openfda_event_docs": "https://open.fda.gov/apis/drug/event/",
    "openfda_event_fields": "https://open.fda.gov/apis/drug/event/searchable-fields/",
    "openfda_label_docs": "https://open.fda.gov/apis/drug/label/",
    "openfda_query_syntax": "https://open.fda.gov/apis/query-syntax/",
    "dailymed_web_services": "https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm",
    "dailymed_spls_api": "https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm",
    "europepmc_rest_docs": "https://europepmc.org/RestfulWebService",
    "ncbi_eutilities_intro": "https://www.ncbi.nlm.nih.gov/books/NBK25497/",
    "ncbi_eutilities_params": "https://www.ncbi.nlm.nih.gov/books/NBK25499/",
}

LABEL_SEARCH_FIELDS = [
    "drug_interactions",
    "warnings",
    "warnings_and_cautions",
    "precautions",
    "adverse_reactions",
    "contraindications",
    "clinical_pharmacology",
    "boxed_warning",
    "indications_and_usage",
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
    "indications_and_usage",
    "mechanism_of_action",
    "pharmacodynamics",
    "pharmacokinetics",
]

SAFETY_FIELD_HINTS = {
    "boxed_warning",
    "contraindications",
    "warnings",
    "warnings_and_cautions",
    "precautions",
    "adverse_reactions",
    "adverse_reactions_table",
}

INTERACTION_FIELD_HINTS = {
    "drug_interactions",
    "drug_interactions_table",
    "clinical_pharmacology",
    "pharmacodynamics",
    "pharmacokinetics",
}

USER_AGENT = "calyx-discovery/issue1259"
REQUEST_SLEEP_SECONDS = 0.10
OPENFDA_LIMIT = 5
EUROPEPMC_PAGE_SIZE = 5
PUBMED_RETRIEVE_PER_PAIR = 5
MAX_BRIDGE_ROWS = 1000


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


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
        return " ".join(clean_text(item) for item in value)
    if isinstance(value, dict):
        return " ".join(f"{key} {clean_text(val)}" for key, val in value.items())
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


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=True) + "\n")


def read_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def artifact(path: Path, *, jsonl: bool = False) -> dict[str, Any]:
    item = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            item["rows"] = sum(1 for line in handle if line.strip())
    return item


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [name for name, value in inputs.items() if value and not Path(value).exists()]
    if missing:
        raise FileNotFoundError(f"Missing required inputs: {missing}")


def verify_expected_input_hashes(inputs: dict[str, str], *, skip: bool = False) -> dict[str, dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    for name, expected in EXPECTED_INPUT_SHA256.items():
        path = Path(inputs[name])
        observed = sha256_path(path)
        ok = observed == expected
        if not ok and not skip:
            raise RuntimeError(f"Input hash mismatch for {name}: observed {observed} expected {expected}")
        rows[name] = {"path": str(path), "sha256": observed, "expected_sha256": expected, "match": ok}
    if inputs.get("issue1255_pair_status") and Path(inputs["issue1255_pair_status"]).exists():
        path = Path(inputs["issue1255_pair_status"])
        rows["issue1255_pair_status"] = {"path": str(path), "sha256": sha256_path(path), "match": True}
    return rows


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


def fetch_source_docs(raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    raw_dir.mkdir(parents=True, exist_ok=True)
    for name, url in SOURCE_DOCS.items():
        path = raw_dir / f"{name}.html"
        status_path = raw_dir / f"{name}.html.status"
        status, payload = fetch_bytes(url)
        path.write_bytes(payload)
        status_row = {
            "schema_version": 1,
            "source_name": name,
            "source_url": url,
            "http_status": status,
            "path": str(path),
            "bytes": len(payload),
            "sha256": sha256_bytes(payload),
            "retrieved_at": now_utc(),
        }
        write_json(status_path, status_row)
        rows.append(
            {
                **status_row,
                "source_row_id": f"issue1259-source:{stable_id(name, url, status_row['sha256'])}",
                "source_group": "source_documentation",
                "status_path": str(status_path),
                "status_sha256": sha256_path(status_path),
                "text": f"Source documentation snapshot {name} from {url} http_status {status}.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def float_value(value: object) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def rxnorm_identity_set(term_row: dict[str, Any] | None, trusted: list[str]) -> list[str]:
    if not term_row:
        return sorted(set(clean_text(value) for value in trusted if clean_text(value)))
    ingredient_ids = [
        clean_text(item.get("rxcui"))
        for item in term_row.get("related_ingredient_concepts", [])
        if isinstance(item, dict) and clean_text(item.get("rxcui"))
    ]
    return sorted(set(ingredient_ids or [clean_text(value) for value in trusted if clean_text(value)]))


def canonical_pair_identity(left_ids: list[str], right_ids: list[str]) -> str:
    sides = ["+".join(sorted(left_ids)) or "unmapped-left", "+".join(sorted(right_ids)) or "unmapped-right"]
    return "||".join(sorted(sides))


def pair_identity_overlaps(left_a: list[str], right_a: list[str], left_b: list[str], right_b: list[str]) -> bool:
    left_a_set = set(left_a)
    right_a_set = set(right_a)
    left_b_set = set(left_b)
    right_b_set = set(right_b)
    same_orientation = bool(left_a_set & left_b_set) and bool(right_a_set & right_b_set)
    swapped_orientation = bool(left_a_set & right_b_set) and bool(right_a_set & left_b_set)
    return same_orientation or swapped_orientation


def overlap_components(rows: list[dict[str, Any]]) -> dict[str, list[str]]:
    parent = {row["pair_key"]: row["pair_key"] for row in rows}

    def find(value: str) -> str:
        while parent[value] != value:
            parent[value] = parent[parent[value]]
            value = parent[value]
        return value

    def union(left: str, right: str) -> None:
        root_left = find(left)
        root_right = find(right)
        if root_left != root_right:
            parent[root_right] = root_left

    for index, left in enumerate(rows):
        for right in rows[index + 1 :]:
            if pair_identity_overlaps(
                left["drug_a_identity_rxcuis"],
                left["drug_b_identity_rxcuis"],
                right["drug_a_identity_rxcuis"],
                right["drug_b_identity_rxcuis"],
            ):
                union(left["pair_key"], right["pair_key"])

    components: dict[str, list[str]] = defaultdict(list)
    for row in rows:
        components[find(row["pair_key"])].append(row["pair_key"])
    return {key: sorted(values) for key, values in components.items()}


def top_twosides_conditions(rows: list[dict[str, Any]], limit: int = 10) -> list[dict[str, Any]]:
    ordered = sorted(
        rows,
        key=lambda row: (-float_value(row.get("PRR")), clean_text(row.get("condition_concept_name")), int(row.get("source_row_index") or 0)),
    )
    out = []
    seen: set[str] = set()
    for row in ordered:
        cond = clean_text(row.get("condition_concept_name"))
        if not cond or cond in seen:
            continue
        seen.add(cond)
        out.append(
            {
                "condition_concept_name": cond,
                "condition_meddra_id": clean_text(row.get("condition_meddra_id")),
                "PRR": clean_text(row.get("PRR")),
                "mean_reporting_frequency": clean_text(row.get("mean_reporting_frequency")),
                "source_row_index": int(row.get("source_row_index") or 0),
                "source_row_sha256": clean_text(row.get("source_row_sha256")),
            }
        )
        if len(out) >= limit:
            break
    return out


def build_pair_scope(
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    term_status: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    drugcentral_pair_status: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    term_by_id = {row["rxnorm_term_status_id"]: row for row in term_status}
    ev_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in twosides_evidence:
        ev_by_pair[row["pair_key"]].append(row)
    candidates_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidate_status:
        candidates_by_pair[row["pair_key"]].append(row)
    drugcentral_by_pair = {row.get("pair_key"): row for row in drugcentral_pair_status}
    scoped: list[dict[str, Any]] = []
    for row in pair_status:
        if row.get("rxnorm_pair_status") != "rxnorm_twosides_rxcui_pair_hit_still_blocked":
            continue
        ev = ev_by_pair[row["pair_key"]]
        left_identity = rxnorm_identity_set(term_by_id.get(row.get("drug_a_term_status_id")), row.get("drug_a_trusted_rxcuis", []))
        right_identity = rxnorm_identity_set(term_by_id.get(row.get("drug_b_term_status_id")), row.get("drug_b_trusted_rxcuis", []))
        canonical_identity = canonical_pair_identity(left_identity, right_identity)
        source_candidates = candidates_by_pair.get(row["pair_key"], [])
        scoped.append(
            {
                "schema_version": 1,
                "scope_id": "issue1259-pair-scope:" + stable_id(row["pair_key"]),
                "pair_key": row["pair_key"],
                "representative_pair_id": row["representative_pair_id"],
                "source_pair_ids": row.get("source_pair_ids", []),
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "query_drug_a": query_name(row["drug_a"]),
                "query_drug_b": query_name(row["drug_b"]),
                "drug_a_trusted_rxcuis": row.get("drug_a_trusted_rxcuis", []),
                "drug_b_trusted_rxcuis": row.get("drug_b_trusted_rxcuis", []),
                "drug_a_identity_rxcuis": left_identity,
                "drug_b_identity_rxcuis": right_identity,
                "canonical_pair_identity": canonical_identity,
                "queryable": bool(query_name(row["drug_a"]) and query_name(row["drug_b"])),
                "twosides_rxcui_evidence_rows": len(ev),
                "twosides_max_prr": max([float_value(item.get("PRR")) for item in ev] or [0.0]),
                "twosides_condition_count": len({clean_text(item.get("condition_meddra_id")) for item in ev if clean_text(item.get("condition_meddra_id"))}),
                "twosides_top_conditions": top_twosides_conditions(ev),
                "rxnorm_pair_status_id": row["rxnorm_pair_status_id"],
                "source_nsides_pair_status_id": row.get("source_nsides_pair_status_id"),
                "source_candidate_status_ids": [item["rxnorm_candidate_status_id"] for item in source_candidates],
                "source_candidate_pair_ids": [item["pair_id"] for item in source_candidates],
                "source_candidate_rows": len(source_candidates),
                "drugcentral_prior_pair_status": (drugcentral_by_pair.get(row["pair_key"]) or {}).get("drugcentral_pair_status", "not_present_in_issue1255_pair_status"),
                "drugcentral_prior_pair_status_id": (drugcentral_by_pair.get(row["pair_key"]) or {}).get("drugcentral_pair_status_id"),
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    strict_groups: dict[str, list[str]] = defaultdict(list)
    for row in scoped:
        strict_groups[row["canonical_pair_identity"]].append(row["pair_key"])
    overlap_groups = overlap_components(scoped)
    overlap_by_pair = {pair_key: peers for peers in overlap_groups.values() for pair_key in peers}
    for row in scoped:
        strict_peers = sorted(strict_groups[row["canonical_pair_identity"]])
        peers = overlap_by_pair[row["pair_key"]]
        row["canonical_duplicate_group_pair_keys"] = peers
        row["canonical_duplicate_group_size"] = len(peers)
        row["strict_canonical_identity_group_pair_keys"] = strict_peers
        row["strict_canonical_identity_group_size"] = len(strict_peers)
        row["canonical_duplicate_note"] = (
            "rxnorm_trusted_rxcui_overlap_group_requires_deduplicated_review"
            if len(peers) > 1
            else "single_original_pair_key_for_canonical_identity"
        )
    scoped.sort(key=lambda row: row["pair_key"])
    return scoped


def openfda_event_query_url(left: str, right: str) -> tuple[str, str]:
    query = f'patient.drug.medicinalproduct:"{left}" AND patient.drug.medicinalproduct:"{right}"'
    params = {"search": query, "limit": str(OPENFDA_LIMIT)}
    return query, f"{OPENFDA_EVENT_ENDPOINT}?{urllib.parse.urlencode(params)}"


def openfda_label_search_url(left: str, right: str) -> tuple[str, str]:
    clauses = []
    for field in LABEL_SEARCH_FIELDS:
        left_clause = urllib.parse.quote(f'{field}:"{left}"', safe=":")
        right_clause = urllib.parse.quote(f'{field}:"{right}"', safe=":")
        clauses.append(f"({left_clause}+AND+{right_clause})")
    query = " OR ".join(f'{field}:"{left}" AND {field}:"{right}"' for field in LABEL_SEARCH_FIELDS)
    search = "+OR+".join(clauses)
    return query, f"{OPENFDA_LABEL_ENDPOINT}?search={search}&limit={OPENFDA_LIMIT}"


def dailymed_spls_url(query: str) -> str:
    params = {"drug_name": query, "name_type": "both", "pagesize": "100", "page": "1"}
    return f"{DAILYMED_SPLS_ENDPOINT}?{urllib.parse.urlencode(params)}"


def europepmc_search_url(left: str, right: str) -> tuple[str, str]:
    query = f'"{left}" AND "{right}"'
    params = {
        "query": query,
        "format": "json",
        "resultType": "core",
        "pageSize": str(EUROPEPMC_PAGE_SIZE),
        "cursorMark": "*",
        "synonym": "false",
    }
    return query, f"{EUROPEPMC_SEARCH_ENDPOINT}?{urllib.parse.urlencode(params)}"


def pubmed_query_term(left: str, right: str) -> str:
    if not left or not right:
        return ""
    return f'("{left}"[Title/Abstract]) AND ("{right}"[Title/Abstract])'


def pubmed_url(base_url: str, params: dict[str, str]) -> str:
    return f"{base_url}?{urllib.parse.urlencode(params)}"


def decode_json(payload: bytes) -> dict[str, Any]:
    if not payload:
        return {}
    try:
        value = json.loads(payload.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        return {"decode_error": payload.decode("utf-8", errors="replace")[:1000]}
    return value if isinstance(value, dict) else {"value": value}


def write_raw(raw_dir: Path, prefix: str, pair_key: str, payload: bytes, suffix: str = ".json") -> Path:
    raw_dir.mkdir(parents=True, exist_ok=True)
    path = raw_dir / f"{prefix}_{stable_id(pair_key, sha256_bytes(payload))}{suffix}"
    path.write_bytes(payload)
    return path


def query_openfda_event(pairs: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        if not pair["queryable"]:
            continue
        query, url = openfda_event_query_url(pair["query_drug_a"], pair["query_drug_b"])
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "openfda_event", pair["pair_key"], payload)
        data = decode_json(payload)
        results = data.get("results") if isinstance(data.get("results"), list) else []
        total = int((data.get("meta", {}).get("results", {}) or {}).get("total") or 0) if isinstance(data, dict) else 0
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("openfda_event", pair["pair_key"], url),
                "source": "openFDA Drug Event API",
                "source_type": "openfda_faers",
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query": query,
                "query_url": url,
                "api_endpoint": OPENFDA_EVENT_ENDPOINT,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "total": total,
                "returned_result_count": len(results),
                "response_json": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(f"#1259 openFDA FAERS {index}/{len(pairs)} pair={pair['pair_key']} total={total}", file=sys.stderr)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def query_openfda_label(pairs: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        if not pair["queryable"]:
            continue
        query, url = openfda_label_search_url(pair["query_drug_a"], pair["query_drug_b"])
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "openfda_label", pair["pair_key"], payload)
        data = decode_json(payload)
        results = data.get("results") if isinstance(data.get("results"), list) else []
        total = int((data.get("meta", {}).get("results", {}) or {}).get("total") or 0) if isinstance(data, dict) else 0
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("openfda_label", pair["pair_key"], url),
                "source": "openFDA Drug Label API",
                "source_type": "openfda_label",
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query": query,
                "query_url": url,
                "api_endpoint": OPENFDA_LABEL_ENDPOINT,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "total": total,
                "returned_result_count": len(results),
                "response_json": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(f"#1259 openFDA label {index}/{len(pairs)} pair={pair['pair_key']} total={total}", file=sys.stderr)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def query_dailymed(pairs: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        if not pair["queryable"]:
            continue
        responses = []
        for direction, value in [
            ("drug_a_space_drug_b", f"{pair['query_drug_a']} {pair['query_drug_b']}"),
            ("drug_b_space_drug_a", f"{pair['query_drug_b']} {pair['query_drug_a']}"),
        ]:
            url = dailymed_spls_url(value)
            status, payload = fetch_bytes(url)
            raw_path = write_raw(raw_dir, f"dailymed_{direction}", pair["pair_key"], payload)
            data = decode_json(payload)
            items = data.get("data") if isinstance(data.get("data"), list) else []
            responses.append(
                {
                    "direction": direction,
                    "query": value,
                    "query_url": url,
                    "http_status": status,
                    "raw_response_path": str(raw_path),
                    "raw_response_bytes": len(payload),
                    "raw_response_sha256": sha256_bytes(payload),
                    "spl_count": len(items),
                    "response_json": data,
                }
            )
            time.sleep(REQUEST_SLEEP_SECONDS)
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("dailymed", pair["pair_key"]),
                "source": "NLM DailyMed v2 SPL metadata",
                "source_type": "dailymed_spl_title",
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "api_endpoint": DAILYMED_SPLS_ENDPOINT,
                "query_responses": responses,
                "total": sum(item["spl_count"] for item in responses),
                "returned_result_count": sum(item["spl_count"] for item in responses),
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(f"#1259 DailyMed {index}/{len(pairs)} pair={pair['pair_key']} spls={rows[-1]['total']}", file=sys.stderr)
    return rows


def query_europepmc(pairs: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        if not pair["queryable"]:
            continue
        query, url = europepmc_search_url(pair["query_drug_a"], pair["query_drug_b"])
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "europepmc", pair["pair_key"], payload)
        data = decode_json(payload)
        result_list = data.get("resultList") if isinstance(data.get("resultList"), dict) else {}
        results = result_list.get("result") if isinstance(result_list.get("result"), list) else []
        hit_count = int(data.get("hitCount") or 0) if str(data.get("hitCount") or "0").isdigit() else 0
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("europepmc", pair["pair_key"], url),
                "source": "Europe PMC Articles REST search",
                "source_type": "europepmc",
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query": query,
                "query_url": url,
                "api_endpoint": EUROPEPMC_SEARCH_ENDPOINT,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "total": hit_count,
                "returned_result_count": len(results),
                "response_json": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(f"#1259 Europe PMC {index}/{len(pairs)} pair={pair['pair_key']} hits={hit_count}", file=sys.stderr)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def query_pubmed_esearch(pairs: list[dict[str, Any]], raw_dir: Path, email: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, pair in enumerate(pairs, start=1):
        term = pubmed_query_term(pair["query_drug_a"], pair["query_drug_b"])
        params = {
            "db": "pubmed",
            "retmode": "json",
            "retmax": str(PUBMED_RETRIEVE_PER_PAIR),
            "sort": "relevance",
            "term": term,
            "tool": "calyx",
            "email": email,
        }
        url = pubmed_url(PUBMED_ESEARCH_URL, params)
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "pubmed_esearch", pair["pair_key"], payload)
        data = decode_json(payload)
        result = data.get("esearchresult") or {}
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("pubmed_esearch", pair["pair_key"], url),
                "source": "PubMed E-utilities ESearch",
                "source_type": "pubmed_esearch",
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query": term,
                "query_url": url,
                "api_endpoint": PUBMED_ESEARCH_URL,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "total": int(result.get("count") or 0),
                "returned_result_count": len(result.get("idlist") or []),
                "idlist": [str(item) for item in result.get("idlist") or []],
                "response_json": data,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(f"#1259 PubMed ESearch {index}/{len(pairs)} pair={pair['pair_key']} count={rows[-1]['total']}", file=sys.stderr)
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def query_pubmed_efetch(esearch_rows: list[dict[str, Any]], raw_dir: Path, email: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in esearch_rows:
        pmids = row.get("idlist", [])[:PUBMED_RETRIEVE_PER_PAIR]
        if not pmids:
            continue
        params = {
            "db": "pubmed",
            "retmode": "xml",
            "rettype": "abstract",
            "id": ",".join(pmids),
            "tool": "calyx",
            "email": email,
        }
        url = pubmed_url(PUBMED_EFETCH_URL, params)
        status, payload = fetch_bytes(url)
        raw_path = write_raw(raw_dir, "pubmed_efetch", row["pair_key"], payload, ".xml")
        rows.append(
            {
                "schema_version": 1,
                "query_row_id": "issue1259-query:" + stable_id("pubmed_efetch", row["pair_key"], url),
                "source": "PubMed E-utilities EFetch XML",
                "source_type": "pubmed_efetch",
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "query": row["query"],
                "query_url": url,
                "api_endpoint": PUBMED_EFETCH_URL,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "pmids": pmids,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def label_sections(result: dict[str, Any]) -> list[dict[str, str]]:
    sections = []
    for field in LABEL_TEXT_FIELDS:
        text = clean_text(result.get(field))
        if text:
            sections.append({"field": field, "text": text})
    return sections


def snippet(text: str, left: str, right: str, window: int = 160) -> str:
    lower = text.lower()
    indexes = []
    for name in [left, right, query_name(left), query_name(right), norm_name(left), norm_name(right)]:
        if not name:
            continue
        index = lower.find(name.lower())
        if index >= 0:
            indexes.append(index)
    if not indexes:
        return clean_text(text[: window * 2])
    start = max(0, min(indexes) - window)
    end = min(len(text), max(indexes) + window)
    return clean_text(text[start:end])


def openfda_list(result: dict[str, Any], field: str) -> list[str]:
    openfda = result.get("openfda") or {}
    value = openfda.get(field)
    if isinstance(value, list):
        return uniq(value)
    if value:
        return [clean_text(value)]
    return []


def event_drug_text(event: dict[str, Any]) -> str:
    patient = event.get("patient") or {}
    drugs = patient.get("drug") or []
    if isinstance(drugs, dict):
        drugs = [drugs]
    parts = []
    for drug in drugs if isinstance(drugs, list) else []:
        if not isinstance(drug, dict):
            continue
        for key in ["medicinalproduct", "drugdosagetext", "drugindication"]:
            parts.append(clean_text(drug.get(key)))
        openfda = drug.get("openfda") or {}
        if isinstance(openfda, dict):
            parts.extend(clean_text(value) for value in openfda.values())
    return clean_text(parts)


def event_reactions(event: dict[str, Any]) -> list[str]:
    patient = event.get("patient") or {}
    reactions = patient.get("reaction") or []
    if isinstance(reactions, dict):
        reactions = [reactions]
    out = []
    for item in reactions if isinstance(reactions, list) else []:
        if isinstance(item, dict):
            out.append(clean_text(item.get("reactionmeddrapt")))
    return uniq(out)


def event_seriousness(event: dict[str, Any]) -> dict[str, Any]:
    fields = [
        "serious",
        "seriousnessdeath",
        "seriousnesslifethreatening",
        "seriousnesshospitalization",
        "seriousnessdisabling",
        "seriousnesscongenitalanomali",
        "seriousnessother",
    ]
    flags = {field: clean_text(event.get(field)) for field in fields if clean_text(event.get(field))}
    return {"flags": flags, "serious": any(value == "1" for value in flags.values()), "death": flags.get("seriousnessdeath") == "1"}


def evidence_from_openfda_event(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence = []
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        results = query.get("response_json", {}).get("results") or []
        for index, event in enumerate(results if isinstance(results, list) else [], start=1):
            if not isinstance(event, dict):
                continue
            text = event_drug_text(event)
            left = exact_presence(text, pair["query_drug_a"])
            right = exact_presence(text, pair["query_drug_b"])
            if not (left["present"] and right["present"]):
                continue
            source_id = clean_text(event.get("safetyreportid") or stable_id(event, length=16))
            seriousness = event_seriousness(event)
            reactions = event_reactions(event)
            evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": "issue1259-evidence:" + stable_id("openfda_event", query["pair_key"], source_id, index),
                    "source_type": "openfda_faers",
                    "source": "openFDA Drug Event API",
                    "pair_key": query["pair_key"],
                    "canonical_pair_identity": pair["canonical_pair_identity"],
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source_id": source_id,
                    "source_url": query["query_url"],
                    "source_response_sha256": query["raw_response_sha256"],
                    "source_text_sha256": sha256_bytes(text.encode("utf-8")),
                    "classification": "safety_signal",
                    "subclassification": "faers_coreport_serious" if seriousness["serious"] else "faers_coreport",
                    "seriousness": seriousness,
                    "reaction_terms": reactions,
                    "term_presence": {"left": left, "right": right},
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": ["faers_coreport_not_safety_clearance", "requires_falsification_and_human_review"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return evidence


def evidence_from_openfda_label(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence = []
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        results = query.get("response_json", {}).get("results") or []
        for result_index, result in enumerate(results if isinstance(results, list) else [], start=1):
            if not isinstance(result, dict):
                continue
            for section in label_sections(result):
                left = exact_presence(section["text"], pair["query_drug_a"])
                right = exact_presence(section["text"], pair["query_drug_b"])
                if not (left["present"] and right["present"]):
                    continue
                label_id = clean_text(result.get("id") or result.get("set_id") or stable_id(result, length=16))
                field = section["field"]
                evidence.append(
                    {
                        "schema_version": 1,
                        "evidence_id": "issue1259-evidence:" + stable_id("openfda_label", query["pair_key"], label_id, field, result_index),
                        "source_type": "openfda_label",
                        "source": "openFDA Drug Label API",
                        "pair_key": query["pair_key"],
                        "canonical_pair_identity": pair["canonical_pair_identity"],
                        "drug_a": pair["drug_a"],
                        "drug_b": pair["drug_b"],
                        "source_id": label_id,
                        "set_id": clean_text(result.get("set_id")),
                        "source_url": f"{OPENFDA_LABEL_ENDPOINT}?search=id:%22{urllib.parse.quote(label_id)}%22",
                        "source_response_sha256": query["raw_response_sha256"],
                        "classification": "label_safety_or_interaction" if field in SAFETY_FIELD_HINTS | INTERACTION_FIELD_HINTS else "label_comention",
                        "subclassification": field,
                        "openfda_brand_names": openfda_list(result, "brand_name"),
                        "openfda_generic_names": openfda_list(result, "generic_name"),
                        "section_text_sha256": sha256_bytes(section["text"].encode("utf-8")),
                        "snippet": snippet(section["text"], pair["query_drug_a"], pair["query_drug_b"]),
                        "term_presence": {"left": left, "right": right},
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "promotion_status": PROMOTION_STATUS,
                        "reason_codes": ["label_text_comention_not_safety_clearance", "requires_falsification_and_human_review"],
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
    return evidence


def evidence_from_dailymed(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence = []
    seen: set[tuple[str, str]] = set()
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        for response in query.get("query_responses", []):
            results = response.get("response_json", {}).get("data") or []
            for item in results if isinstance(results, list) else []:
                if not isinstance(item, dict):
                    continue
                title = clean_text(item.get("title"))
                left = exact_presence(title, pair["query_drug_a"])
                right = exact_presence(title, pair["query_drug_b"])
                if not (left["present"] and right["present"]):
                    continue
                setid = clean_text(item.get("setid"))
                if not setid or (query["pair_key"], setid) in seen:
                    continue
                seen.add((query["pair_key"], setid))
                evidence.append(
                    {
                        "schema_version": 1,
                        "evidence_id": "issue1259-evidence:" + stable_id("dailymed", query["pair_key"], setid),
                        "source_type": "dailymed_spl_title",
                        "source": "NLM DailyMed v2 SPL metadata",
                        "pair_key": query["pair_key"],
                        "canonical_pair_identity": pair["canonical_pair_identity"],
                        "drug_a": pair["drug_a"],
                        "drug_b": pair["drug_b"],
                        "source_id": setid,
                        "source_url": f"{DAILYMED_LABEL_URL}?{urllib.parse.urlencode({'setid': setid})}",
                        "source_response_sha256": response["raw_response_sha256"],
                        "classification": "label_metadata_comention",
                        "subclassification": "dailymed_spl_title",
                        "title": title,
                        "term_presence": {"left": left, "right": right},
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "promotion_status": PROMOTION_STATUS,
                        "reason_codes": ["dailymed_title_comention_not_safety_clearance", "requires_falsification_and_human_review"],
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
    return evidence


def strings_from_value(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        return [value] if value.strip() else []
    if isinstance(value, dict):
        out = []
        for item in value.values():
            out.extend(strings_from_value(item))
        return out
    if isinstance(value, list):
        out = []
        for item in value:
            out.extend(strings_from_value(item))
        return out
    return [clean_text(value)]


def europepmc_metadata_text(item: dict[str, Any]) -> str:
    fields = ["title", "abstractText", "authorString", "pubTypeList", "keywordList", "journalInfo", "subsetList"]
    parts = []
    for field in fields:
        parts.extend(strings_from_value(item.get(field)))
    return clean_text(parts)


def evidence_from_europepmc(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence = []
    for query in query_rows:
        pair = pair_lookup[query["pair_key"]]
        result_list = query.get("response_json", {}).get("resultList") or {}
        results = result_list.get("result") or []
        for index, item in enumerate(results if isinstance(results, list) else [], start=1):
            if not isinstance(item, dict):
                continue
            text = europepmc_metadata_text(item)
            left = exact_presence(text, pair["query_drug_a"])
            right = exact_presence(text, pair["query_drug_b"])
            if not (left["present"] and right["present"]):
                continue
            source_id = clean_text(item.get("id") or item.get("pmid") or item.get("pmcid") or stable_id(item, length=16))
            evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": "issue1259-evidence:" + stable_id("europepmc", query["pair_key"], source_id, index),
                    "source_type": "europepmc",
                    "source": "Europe PMC Articles REST search",
                    "pair_key": query["pair_key"],
                    "canonical_pair_identity": pair["canonical_pair_identity"],
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source_id": source_id,
                    "source_url": clean_text(item.get("doi") or item.get("pmid") or item.get("pmcid")),
                    "source_response_sha256": query["raw_response_sha256"],
                    "classification": "literature_comention",
                    "subclassification": clean_text(item.get("pubType")),
                    "title": clean_text(item.get("title")),
                    "journal": clean_text((item.get("journalInfo") or {}).get("journal", {}).get("title")) if isinstance(item.get("journalInfo"), dict) else "",
                    "publication_year": clean_text(item.get("pubYear")),
                    "metadata_text_sha256": sha256_bytes(text.encode("utf-8")),
                    "term_presence": {"left": left, "right": right},
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": ["literature_comention_not_effect_validation", "requires_falsification_and_human_review"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return evidence


def parse_pubmed_records(xml_bytes: bytes) -> list[dict[str, str]]:
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError:
        return []
    records = []
    for article in root.findall(".//PubmedArticle"):
        pmid = clean_text(article.findtext(".//PMID"))
        title = clean_text("".join(article.findtext(".//ArticleTitle") or ""))
        abstracts = [clean_text("".join(item.itertext())) for item in article.findall(".//AbstractText")]
        journal = clean_text(article.findtext(".//Journal/Title"))
        pub_year = clean_text(article.findtext(".//PubDate/Year") or article.findtext(".//PubDate/MedlineDate"))
        records.append({"pmid": pmid, "title": title, "abstract": clean_text(abstracts), "journal": journal, "pub_year": pub_year})
    return records


def evidence_from_pubmed(efetch_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    evidence = []
    for query in efetch_rows:
        pair = pair_lookup[query["pair_key"]]
        payload = Path(query["raw_response_path"]).read_bytes()
        for record in parse_pubmed_records(payload):
            text = clean_text([record["title"], record["abstract"]])
            left = exact_presence(text, pair["query_drug_a"])
            right = exact_presence(text, pair["query_drug_b"])
            if not (left["present"] and right["present"]):
                continue
            pmid = clean_text(record.get("pmid"))
            evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": "issue1259-evidence:" + stable_id("pubmed", query["pair_key"], pmid),
                    "source_type": "pubmed",
                    "source": "PubMed E-utilities EFetch XML",
                    "pair_key": query["pair_key"],
                    "canonical_pair_identity": pair["canonical_pair_identity"],
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source_id": pmid,
                    "source_url": PUBMED_RECORD_URL.format(pmid=pmid),
                    "source_response_sha256": query["raw_response_sha256"],
                    "classification": "literature_title_or_abstract_comention",
                    "subclassification": "pubmed_title_abstract",
                    "title": record["title"],
                    "journal": record["journal"],
                    "publication_year": record["pub_year"],
                    "source_text_sha256": sha256_bytes(text.encode("utf-8")),
                    "term_presence": {"left": left, "right": right},
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": ["pubmed_text_comention_not_effect_validation", "requires_falsification_and_human_review"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return evidence


def evidence_from_drugcentral(pairs: list[dict[str, Any]], drugcentral_pair_status: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_pair = {row.get("pair_key"): row for row in drugcentral_pair_status}
    evidence = []
    for pair in pairs:
        row = by_pair.get(pair["pair_key"])
        if not row:
            continue
        status = clean_text(row.get("drugcentral_pair_status"))
        if status not in {"drugcentral_ddi_structured_hit_still_blocked", "drugcentral_same_structure_equivalence_hit_still_blocked"}:
            continue
        evidence.append(
            {
                "schema_version": 1,
                "evidence_id": "issue1259-evidence:" + stable_id("drugcentral_prior", pair["pair_key"], status),
                "source_type": "drugcentral_prior_source_mining",
                "source": "DrugCentral source mining #1255",
                "pair_key": pair["pair_key"],
                "canonical_pair_identity": pair["canonical_pair_identity"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "source_id": clean_text(row.get("drugcentral_pair_status_id")),
                "classification": "structured_interaction_prior_hit",
                "subclassification": status,
                "source_row": row,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": ["drugcentral_prior_hit_still_not_safety_clearance", "requires_falsification_and_human_review"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return evidence


def source_classification_counts(evidence_rows: list[dict[str, Any]]) -> dict[str, int]:
    return dict(Counter(row["source_type"] for row in evidence_rows))


def rollup_status_for(pair: dict[str, Any], evidence_rows: list[dict[str, Any]]) -> tuple[str, list[str]]:
    reason_codes = [
        "rxnorm_twosides_source_hit_requires_independent_validation",
        "no_clinical_actionability_without_outcome_safety_falsification_human_review",
    ]
    source_types = {row["source_type"] for row in evidence_rows}
    classifications = {row["classification"] for row in evidence_rows}
    if "openfda_faers" in source_types:
        status = "independent_faers_safety_signal_still_blocked"
        reason_codes.append("independent_faers_coreport_found")
    elif "openfda_label" in source_types:
        status = "independent_label_comention_still_blocked"
        reason_codes.append("independent_label_comention_found")
    elif {"pubmed", "europepmc"} & source_types:
        status = "independent_literature_comention_still_blocked"
        reason_codes.append("independent_literature_comention_found")
    elif "dailymed_spl_title" in source_types:
        status = "independent_dailymed_title_comention_still_blocked"
        reason_codes.append("independent_dailymed_title_comention_found")
    elif evidence_rows:
        status = "independent_source_hit_still_blocked"
        reason_codes.append("independent_source_hit_found")
    else:
        status = "twosides_only_no_independent_confirmation_still_blocked"
        reason_codes.append("no_independent_confirmation_found")
    if pair["canonical_duplicate_group_size"] > 1:
        reason_codes.append("rxnorm_canonical_duplicate_group_requires_deduplicated_review")
    if any("safety" in value for value in classifications):
        reason_codes.append("safety_classified_evidence_requires_review")
    return status, reason_codes


def build_pair_rollups(
    pairs: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    ev_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        ev_by_pair[row["pair_key"]].append(row)
    query_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in query_rows:
        query_by_pair[row["pair_key"]].append(row)
    rows = []
    for pair in pairs:
        ev = ev_by_pair.get(pair["pair_key"], [])
        status, reason_codes = rollup_status_for(pair, ev)
        rows.append(
            {
                "schema_version": 1,
                "pair_rollup_id": "issue1259-pair-rollup:" + stable_id(pair["pair_key"]),
                "pair_key": pair["pair_key"],
                "canonical_pair_identity": pair["canonical_pair_identity"],
                "canonical_duplicate_group_pair_keys": pair["canonical_duplicate_group_pair_keys"],
                "canonical_duplicate_group_size": pair["canonical_duplicate_group_size"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "drug_a_identity_rxcuis": pair["drug_a_identity_rxcuis"],
                "drug_b_identity_rxcuis": pair["drug_b_identity_rxcuis"],
                "representative_pair_id": pair["representative_pair_id"],
                "source_pair_ids": pair["source_pair_ids"],
                "source_candidate_status_ids": pair["source_candidate_status_ids"],
                "twosides_rxcui_evidence_rows": pair["twosides_rxcui_evidence_rows"],
                "twosides_condition_count": pair["twosides_condition_count"],
                "twosides_max_prr": pair["twosides_max_prr"],
                "twosides_top_conditions": pair["twosides_top_conditions"],
                "independent_query_rows": len(query_by_pair.get(pair["pair_key"], [])),
                "independent_evidence_rows": len(ev),
                "independent_source_type_counts": source_classification_counts(ev),
                "validation_status": status,
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "drugcentral_prior_pair_status": pair["drugcentral_prior_pair_status"],
                "rxnorm_pair_status_id": pair["rxnorm_pair_status_id"],
                "evidence_ids": [row["evidence_id"] for row in ev],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: row["pair_key"])
    return rows


def build_candidate_status(candidate_rows: list[dict[str, Any]], pair_rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rollup_by_key = {row["pair_key"]: row for row in pair_rollups}
    rows = []
    for row in candidate_rows:
        rollup = rollup_by_key.get(row["pair_key"])
        if not rollup:
            continue
        rows.append(
            {
                "schema_version": 1,
                "candidate_validation_status_id": "issue1259-candidate-status:" + stable_id(row["rxnorm_candidate_status_id"], rollup["validation_status"]),
                "pair_id": row["pair_id"],
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "source_rxnorm_candidate_status_id": row["rxnorm_candidate_status_id"],
                "source_rxnorm_candidate_status": row["rxnorm_candidate_status"],
                "pair_rollup_id": rollup["pair_rollup_id"],
                "validation_status": "candidate_" + rollup["validation_status"],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": rollup["reason_codes"],
                "independent_evidence_rows": rollup["independent_evidence_rows"],
                "twosides_rxcui_evidence_rows": rollup["twosides_rxcui_evidence_rows"],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["pair_id"]))
    return rows


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    pair_scope: list[dict[str, Any]],
    pair_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in source_rows:
        terms = uniq(["issue1259", "source_documentation", row["source_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1259_source_snapshot",
                "text": f"{row['text']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": {
                    "source_dataset": "issue1259_rxnorm_twosides_safety_validation",
                    "source_path": row["path"],
                    "source_sha256": row["sha256"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_rollups:
        terms = uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["validation_status"], row["canonical_pair_identity"]])
        text = (
            f"Issue1259 pair rollup {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['validation_status']} independent evidence rows {row['independent_evidence_rows']} "
            f"TwoSIDES rows {row['twosides_rxcui_evidence_rows']} bridge terms {' '.join(terms)}."
        )
        rows.append(
            {
                "id": row["pair_rollup_id"],
                "domain": "issue1259_pair_rollup",
                "text": text,
                "bridge_terms": terms,
                "metadata": {
                    "source_dataset": "issue1259_rxnorm_twosides_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "validation_status": row["validation_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in candidate_status:
        terms = uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["validation_status"]])
        text = (
            f"Issue1259 candidate validation {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['validation_status']} bridge terms {' '.join(terms)}."
        )
        rows.append(
            {
                "id": row["candidate_validation_status_id"],
                "domain": "issue1259_candidate_status",
                "text": text,
                "bridge_terms": terms,
                "metadata": {
                    "source_dataset": "issue1259_rxnorm_twosides_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "validation_status": row["validation_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = MAX_BRIDGE_ROWS - len(rows)
    for row in evidence_rows[: max(0, remaining)]:
        terms = uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_type"], row["classification"]])
        text = (
            f"Issue1259 independent evidence {row['evidence_id']} source {row['source_type']} "
            f"pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} classification {row['classification']} "
            f"still blocked bridge terms {' '.join(terms)}."
        )
        rows.append(
            {
                "id": row["evidence_id"],
                "domain": "issue1259_independent_evidence",
                "text": text,
                "bridge_terms": terms,
                "metadata": {
                    "source_dataset": "issue1259_rxnorm_twosides_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_type": row["source_type"],
                    "classification": row["classification"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = MAX_BRIDGE_ROWS - len(rows)
    for row in query_rows[: max(0, remaining)]:
        terms = uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_type"], str(row.get("http_status", ""))])
        text = (
            f"Issue1259 query row {row['query_row_id']} source {row['source_type']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} total {row.get('total', 0)} bridge terms {' '.join(terms)}."
        )
        rows.append(
            {
                "id": row["query_row_id"],
                "domain": "issue1259_query_row",
                "text": text,
                "bridge_terms": terms,
                "metadata": {
                    "source_dataset": "issue1259_rxnorm_twosides_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_type": row["source_type"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows[:MAX_BRIDGE_ROWS]


def build_metrics(
    pair_scope: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "all_rows_blocked": True,
        "source_rows": len(source_rows),
        "pair_scope_rows": len(pair_scope),
        "canonical_identity_groups": len({row["canonical_pair_identity"] for row in pair_scope}),
        "canonical_duplicate_groups": len(
            {
                tuple(row["canonical_duplicate_group_pair_keys"])
                for row in pair_scope
                if row["canonical_duplicate_group_size"] > 1
            }
        ),
        "query_rows": len(query_rows),
        "query_rows_by_source_type": dict(Counter(row["source_type"] for row in query_rows)),
        "query_http_status_counts": dict(Counter(str(row.get("http_status", "multi")) for row in query_rows if row.get("source_type") != "dailymed_spl_title")),
        "independent_evidence_rows": len(evidence_rows),
        "independent_evidence_by_source_type": dict(Counter(row["source_type"] for row in evidence_rows)),
        "independent_evidence_by_classification": dict(Counter(row["classification"] for row in evidence_rows)),
        "pair_rollup_rows": len(pair_rollups),
        "pair_rollup_status_counts": dict(Counter(row["validation_status"] for row in pair_rollups)),
        "candidate_status_rows": len(candidate_status),
        "candidate_status_counts": dict(Counter(row["validation_status"] for row in candidate_status)),
        "twosides_rxcui_evidence_rows": sum(row["twosides_rxcui_evidence_rows"] for row in pair_scope),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
    }


def build_manifest(out_dir: Path, inputs: dict[str, str], input_hashes: dict[str, dict[str, Any]]) -> dict[str, Any]:
    artifacts = {
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "source_rows": artifact(out_dir / "source_rows.jsonl", jsonl=True),
        "pair_scope": artifact(out_dir / "pair_scope.jsonl", jsonl=True),
        "independent_query_rows": artifact(out_dir / "independent_query_rows.jsonl", jsonl=True),
        "independent_evidence_rows": artifact(out_dir / "independent_evidence_rows.jsonl", jsonl=True),
        "pair_validation_rollups": artifact(out_dir / "pair_validation_rollups.jsonl", jsonl=True),
        "candidate_validation_status": artifact(out_dir / "candidate_validation_status.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "issue1259_bridge_rows": artifact(out_dir / "issue1259_bridge_rows.jsonl", jsonl=True),
    }
    return {
        "schema_version": 1,
        "issue": 1259,
        "created_at": now_utc(),
        "inputs": inputs,
        "input_hashes": input_hashes,
        "artifacts": artifacts,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def build_readback(
    out_dir: Path,
    input_hashes: dict[str, dict[str, Any]],
    pair_scope: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_rollups: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    expected_pair_scope_rows: int,
    expected_twosides_rows: int,
) -> dict[str, Any]:
    artifact_paths = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_rows": out_dir / "source_rows.jsonl",
        "pair_scope": out_dir / "pair_scope.jsonl",
        "independent_query_rows": out_dir / "independent_query_rows.jsonl",
        "independent_evidence_rows": out_dir / "independent_evidence_rows.jsonl",
        "pair_validation_rollups": out_dir / "pair_validation_rollups.jsonl",
        "candidate_validation_status": out_dir / "candidate_validation_status.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
        "issue1259_bridge_rows": out_dir / "issue1259_bridge_rows.jsonl",
    }
    artifacts = {name: artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifact_paths.items()}
    assertions = {
        "expected_input_hashes_match": all(item.get("match") for item in input_hashes.values() if "expected_sha256" in item),
        "expected_pair_scope_rows": len(pair_scope) == expected_pair_scope_rows,
        "all_pair_scope_queryable": all(row["queryable"] for row in pair_scope),
        "twosides_input_rows_preserved": sum(row["twosides_rxcui_evidence_rows"] for row in pair_scope) == expected_twosides_rows,
        "pair_rollup_for_every_scope_pair": {row["pair_key"] for row in pair_rollups} == {row["pair_key"] for row in pair_scope},
        "candidate_status_sources_are_scoped": {row["pair_key"] for row in candidate_status}.issubset({row["pair_key"] for row in pair_scope}),
        "query_rows_for_every_pair": {row["pair_key"] for row in query_rows} == {row["pair_key"] for row in pair_scope},
        "query_rows_have_response_hashes": all(row.get("raw_response_sha256") or row.get("query_responses") for row in query_rows),
        "evidence_rows_have_sources": all(row.get("source") and row.get("source_type") for row in evidence_rows),
        "evidence_rows_blocked": all(row.get("promotion_status") == PROMOTION_STATUS for row in evidence_rows),
        "pair_rollups_blocked": all(row.get("promotion_status") == PROMOTION_STATUS for row in pair_rollups),
        "candidate_status_blocked": all(row.get("promotion_status") == PROMOTION_STATUS for row in candidate_status),
        "clinical_boundary_present": all(CLINICAL_BOUNDARY == row.get("clinical_boundary") for row in pair_rollups),
        "bridge_rows_1000_or_less": len(bridge_rows) <= MAX_BRIDGE_ROWS,
        "bridge_terms_present_in_text": all(
            all(clean_text(term).lower() in clean_text(row.get("text")).lower() for term in row.get("bridge_terms", []))
            for row in bridge_rows
        ),
    }
    return {
        "schema_version": 1,
        "status": "ok" if all(assertions.values()) else "failed",
        "artifacts": artifacts,
        "assertions": assertions,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1258-twosides-evidence")
    parser.add_argument("--issue1258-pair-status")
    parser.add_argument("--issue1258-candidate-status")
    parser.add_argument("--issue1258-term-status")
    parser.add_argument("--issue1258-persisted-readback")
    parser.add_argument("--issue1258-calyx-readback")
    parser.add_argument("--issue1258-output-manifest")
    parser.add_argument("--issue1255-pair-status")
    parser.add_argument("--skip-input-sha-check", action="store_true")
    parser.add_argument("--email", default="opensource@example.com")
    parser.add_argument("--max-pairs", type=int, default=0, help="Smoke-test limiter over scoped pair rows.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.root)
    raw_dir = root / "raw"
    out_dir = root / "out"
    raw_responses_dir = raw_dir / "responses"
    out_dir.mkdir(parents=True, exist_ok=True)

    inputs = dict(DEFAULT_INPUTS)
    for key in list(inputs):
        attr = key.replace("-", "_")
        override = getattr(args, attr, None)
        if override:
            inputs[key] = override
    require_inputs(inputs)
    input_hashes = verify_expected_input_hashes(inputs, skip=args.skip_input_sha_check)
    write_json(out_dir / "input_manifest.json", {"schema_version": 1, "inputs": inputs, "input_hashes": input_hashes})

    source_rows = fetch_source_docs(raw_dir)
    pair_status = rows_jsonl(Path(inputs["issue1258_pair_status"]))
    candidate_rows_all = rows_jsonl(Path(inputs["issue1258_candidate_status"]))
    term_status = rows_jsonl(Path(inputs["issue1258_term_status"]))
    twosides_evidence = rows_jsonl(Path(inputs["issue1258_twosides_evidence"]))
    drugcentral_rows = rows_jsonl(Path(inputs["issue1255_pair_status"])) if Path(inputs["issue1255_pair_status"]).exists() else []

    pair_scope = build_pair_scope(pair_status, candidate_rows_all, term_status, twosides_evidence, drugcentral_rows)
    full_pair_scope_count = len(pair_scope)
    full_twosides_count = sum(row["twosides_rxcui_evidence_rows"] for row in pair_scope)
    if args.max_pairs:
        pair_scope = pair_scope[: args.max_pairs]
    expected_pair_scope_rows = len(pair_scope) if args.max_pairs else 7
    expected_twosides_rows = (
        sum(row["twosides_rxcui_evidence_rows"] for row in pair_scope)
        if args.max_pairs
        else 757
    )
    if not args.max_pairs and (full_pair_scope_count != 7 or full_twosides_count != 757):
        raise RuntimeError(
            f"Unexpected full #1258 scope: pairs={full_pair_scope_count} twosides_rows={full_twosides_count}"
        )
    scoped_pair_keys = {row["pair_key"] for row in pair_scope}
    candidate_rows = [row for row in candidate_rows_all if row["pair_key"] in scoped_pair_keys]
    pair_lookup = {row["pair_key"]: row for row in pair_scope}

    write_jsonl(out_dir / "source_rows.jsonl", source_rows)
    write_jsonl(out_dir / "pair_scope.jsonl", pair_scope)

    query_rows: list[dict[str, Any]] = []
    query_rows.extend(query_openfda_event(pair_scope, raw_responses_dir))
    query_rows.extend(query_openfda_label(pair_scope, raw_responses_dir))
    query_rows.extend(query_dailymed(pair_scope, raw_responses_dir))
    query_rows.extend(query_europepmc(pair_scope, raw_responses_dir))
    pubmed_esearch = query_pubmed_esearch(pair_scope, raw_responses_dir, args.email)
    query_rows.extend(pubmed_esearch)
    pubmed_efetch = query_pubmed_efetch(pubmed_esearch, raw_responses_dir, args.email)
    query_rows.extend(pubmed_efetch)

    evidence_rows: list[dict[str, Any]] = []
    evidence_rows.extend(evidence_from_openfda_event([row for row in query_rows if row["source_type"] == "openfda_faers"], pair_lookup))
    evidence_rows.extend(evidence_from_openfda_label([row for row in query_rows if row["source_type"] == "openfda_label"], pair_lookup))
    evidence_rows.extend(evidence_from_dailymed([row for row in query_rows if row["source_type"] == "dailymed_spl_title"], pair_lookup))
    evidence_rows.extend(evidence_from_europepmc([row for row in query_rows if row["source_type"] == "europepmc"], pair_lookup))
    evidence_rows.extend(evidence_from_pubmed([row for row in query_rows if row["source_type"] == "pubmed_efetch"], pair_lookup))
    evidence_rows.extend(evidence_from_drugcentral(pair_scope, drugcentral_rows))
    evidence_rows.sort(key=lambda row: (row["pair_key"], row["source_type"], row["evidence_id"]))

    pair_rollups = build_pair_rollups(pair_scope, evidence_rows, query_rows)
    candidate_status = build_candidate_status(candidate_rows, pair_rollups)

    write_jsonl(out_dir / "independent_query_rows.jsonl", query_rows)
    write_jsonl(out_dir / "independent_evidence_rows.jsonl", evidence_rows)
    write_jsonl(out_dir / "pair_validation_rollups.jsonl", pair_rollups)
    write_jsonl(out_dir / "candidate_validation_status.jsonl", candidate_status)

    source_path = out_dir / "pair_validation_rollups.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(source_rows, pair_scope, pair_rollups, candidate_status, evidence_rows, query_rows, source_path, source_sha)
    write_jsonl(out_dir / "issue1259_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(pair_scope, source_rows, query_rows, evidence_rows, pair_rollups, candidate_status, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)
    manifest = build_manifest(out_dir, inputs, input_hashes)
    write_json(out_dir / "output_manifest.json", manifest)
    readback = build_readback(
        out_dir,
        input_hashes,
        pair_scope,
        query_rows,
        evidence_rows,
        pair_rollups,
        candidate_status,
        bridge_rows,
        expected_pair_scope_rows,
        expected_twosides_rows,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    if readback["status"] != "ok":
        raise RuntimeError(f"Persisted readback failed: {readback['assertions']}")

    print(
        json.dumps(
            {
                "status": "ok",
                "root": str(root),
                "metrics": metrics,
                "artifacts": {
                    "pair_scope": artifact(out_dir / "pair_scope.jsonl", jsonl=True),
                    "query_rows": artifact(out_dir / "independent_query_rows.jsonl", jsonl=True),
                    "evidence_rows": artifact(out_dir / "independent_evidence_rows.jsonl", jsonl=True),
                    "pair_rollups": artifact(out_dir / "pair_validation_rollups.jsonl", jsonl=True),
                    "candidate_status": artifact(out_dir / "candidate_validation_status.jsonl", jsonl=True),
                    "bridge_rows": artifact(out_dir / "issue1259_bridge_rows.jsonl", jsonl=True),
                    "persisted_readback": artifact(out_dir / "persisted_readback.json"),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
