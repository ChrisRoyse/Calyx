#!/usr/bin/env python3
"""#1248 independent openFDA safety-source validation for #1246 rollups.

This stage reads sealed #1246 safety/counter rollups and queries openFDA Human
Drug Label as an independent regulatory-label source. A hit requires both
candidate terms in the same persisted label section. Output is still blocked
research triage: not safety clearance, contraindication guidance, treatment
guidance, dosing guidance, recommendation, clinical actionability, or cure
evidence.
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
    "openFDA independent safety validation is regulatory-label text triage only; "
    "not safety clearance, contraindication guidance, treatment guidance, dosing "
    "guidance, recommendation, clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "openfda_independent_label_safety_validation_not_clinical_clearance"
)

ISSUE1246_ROOT = "/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z"

DEFAULT_INPUTS = {
    "issue1246_rollup": f"{ISSUE1246_ROOT}/out/candidate_europepmc_safety_counter_rollup.jsonl",
    "issue1246_evidence_review": f"{ISSUE1246_ROOT}/out/europepmc_safety_counter_evidence_review.jsonl",
    "issue1246_persisted_readback": f"{ISSUE1246_ROOT}/out/persisted_readback.json",
    "issue1246_calyx_readback": f"{ISSUE1246_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1246_output_manifest": f"{ISSUE1246_ROOT}/out/output_manifest.json",
}

OPENFDA_LABEL_ENDPOINT = "https://api.fda.gov/drug/label.json"
OPENFDA_DOWNLOAD_MANIFEST_URL = "https://api.fda.gov/download.json"
OPENFDA_LABEL_OVERVIEW_URL = "https://open.fda.gov/apis/drug/label/"
OPENFDA_LABEL_HOWTO_URL = "https://open.fda.gov/apis/drug/label/how-to-use-the-endpoint/"
OPENFDA_QUERY_SYNTAX_URL = "https://open.fda.gov/apis/query-syntax/"
OPENFDA_AUTH_URL = "https://open.fda.gov/apis/authentication/"
OPENFDA_LICENSE_URL = "https://open.fda.gov/license/"
OPENFDA_TERMS_URL = "https://open.fda.gov/terms/"

OPENFDA_LIMIT = 5
REQUEST_SLEEP_SECONDS = 0.30
USER_AGENT = "calyx-discovery/issue1248"
PROMOTION_STATUS = "blocked_requires_independent_safety_falsification_and_human_review"

LABEL_SEARCH_FIELDS = [
    "drug_interactions",
    "warnings",
    "warnings_and_cautions",
    "precautions",
    "adverse_reactions",
    "contraindications",
    "boxed_warning",
    "clinical_pharmacology",
    "pharmacodynamics",
    "pharmacokinetics",
    "description",
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
    "dosage_and_administration",
    "indications_and_usage",
    "use_in_specific_populations",
    "description",
    "mechanism_of_action",
    "pharmacodynamics",
    "pharmacokinetics",
    "information_for_patients",
    "patient_medication_information",
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

STATUS_VALUES = {
    "independent_openfda_safety_label_hit_still_blocked",
    "independent_openfda_interaction_label_hit_still_blocked",
    "independent_openfda_label_text_hit_still_blocked",
    "independent_openfda_no_hit_still_blocked",
    "independent_openfda_not_queryable_still_blocked",
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


def norm_name(value: object) -> str:
    text = clean_text(value).lower()
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return " ".join(text.split())


def query_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
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
    if not path.exists():
        raise FileNotFoundError(f"required artifact is missing: {path}")
    item = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
        "rows": len(rows_jsonl(path)) if jsonl else None,
    }
    if source_url:
        item["source_page_url"] = source_url
    return item


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [value for value in inputs.values() if not Path(value).exists()]
    if missing:
        raise SystemExit(
            json.dumps(
                {
                    "code": "CALYX_DISCOVERY_SOURCE_MISSING",
                    "message": "required #1246 source artifacts are missing",
                    "missing": missing,
                    "remediation": "finish #1246 and persist safety/counter artifacts before running #1248",
                },
                indent=2,
            )
        )


def fetch_bytes(url: str, retries: int = 5) -> tuple[int, bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=90) as response:
                return int(response.status), response.read()
        except urllib.error.HTTPError as error:
            payload = error.read()
            if error.code == 404:
                return int(error.code), payload
            last_error = error
        except (urllib.error.URLError, TimeoutError) as error:
            last_error = error
        time.sleep(min(12.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"fetch failed after {retries} attempts for {url}: {last_error}")


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    sources = {
        "openfda_download_manifest": OPENFDA_DOWNLOAD_MANIFEST_URL,
        "openfda_label_overview": OPENFDA_LABEL_OVERVIEW_URL,
        "openfda_label_howto": OPENFDA_LABEL_HOWTO_URL,
        "openfda_query_syntax": OPENFDA_QUERY_SYNTAX_URL,
        "openfda_authentication": OPENFDA_AUTH_URL,
        "openfda_license": OPENFDA_LICENSE_URL,
        "openfda_terms": OPENFDA_TERMS_URL,
    }
    raw_dir.mkdir(parents=True, exist_ok=True)
    artifacts: dict[str, dict[str, Any]] = {}
    for name, url in sources.items():
        suffix = ".json" if url.endswith(".json") else ".html"
        path = raw_dir / f"{name}{suffix}"
        if not path.exists():
            status, payload = fetch_bytes(url)
            if status >= 400:
                raise RuntimeError(f"source fetch failed {status}: {url}")
            path.write_bytes(payload)
            time.sleep(REQUEST_SLEEP_SECONDS)
        artifacts[name] = artifact(path, source_url=url)
    return artifacts


def all_assertions_true(value: dict[str, Any]) -> bool:
    assertions = value.get("assertions", {})
    return bool(assertions) and all(assertions.values())


def representative_pairs(rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rollups:
        grouped[row["pair_key"]].append(row)
    pairs: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = sorted(members, key=lambda row: row["pair_id"])[0]
        left = query_name(first["drug_a"])
        right = query_name(first["drug_b"])
        pairs.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "representative_pair_id": first["pair_id"],
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query_drug_a": left,
                "query_drug_b": right,
                "rollup_row_count": len(members),
                "rollup_review_ids": [row["rollup_review_id"] for row in sorted(members, key=lambda item: item["pair_id"])],
                "queryable": bool(norm_name(left) and norm_name(right)),
            }
        )
    return pairs


def encode_clause(field: str, term: str) -> str:
    return urllib.parse.quote(f'{field}:"{term}"', safe=":")


def openfda_search_url(left: str, right: str) -> str:
    clauses = []
    for field in LABEL_SEARCH_FIELDS:
        left_clause = encode_clause(field, left)
        right_clause = encode_clause(field, right)
        clauses.append(f"({left_clause}+AND+{right_clause})")
    search = "+OR+".join(clauses)
    return f"{OPENFDA_LABEL_ENDPOINT}?search={search}&limit={OPENFDA_LIMIT}"


def cached_query_rows(path: Path, expected_keys: set[str]) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    rows = rows_jsonl(path)
    if {row.get("pair_key") for row in rows} == expected_keys:
        return rows
    return None


def fetch_openfda_queries(
    pairs: list[dict[str, Any]],
    out_path: Path,
    request_sleep_seconds: float,
) -> list[dict[str, Any]]:
    expected_keys = {row["pair_key"] for row in pairs if row["queryable"]}
    cached = cached_query_rows(out_path, expected_keys)
    if cached is not None:
        return cached
    tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    queryable_rows = [row for row in pairs if row["queryable"]]
    with tmp_path.open("w", encoding="utf-8") as handle:
        for index, pair in enumerate(queryable_rows, start=1):
            url = openfda_search_url(pair["query_drug_a"], pair["query_drug_b"])
            status, payload = fetch_bytes(url)
            try:
                response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
            except json.JSONDecodeError:
                response_json = {"raw_decode_error": payload.decode("utf-8", errors="replace")[:1000]}
            total = response_json.get("meta", {}).get("results", {}).get("total", 0)
            results = response_json.get("results") if isinstance(response_json.get("results"), list) else []
            row = {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "query_url": url,
                "api_endpoint": OPENFDA_LABEL_ENDPOINT,
                "http_status": status,
                "response_bytes": len(payload),
                "response_sha256": sha256_bytes(payload),
                "openfda_total": int(total or 0),
                "returned_result_count": len(results),
                "response_json": response_json,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            handle.write(json.dumps(row, sort_keys=True) + "\n")
            handle.flush()
            print(
                f"openFDA independent safety query {index}/{len(queryable_rows)} status={status} total={row['openfda_total']}",
                file=sys.stderr,
            )
            time.sleep(request_sleep_seconds)
    os.replace(tmp_path, out_path)
    return rows


def label_value_text(value: object) -> str:
    if isinstance(value, list):
        return clean_text(" ".join(clean_text(item) for item in value))
    return clean_text(value)


def label_sections(result: dict[str, Any]) -> list[dict[str, str]]:
    sections: list[dict[str, str]] = []
    for field in LABEL_TEXT_FIELDS:
        text = label_value_text(result.get(field))
        if text:
            sections.append({"field": field, "text": text})
    return sections


def snippet(text: str, name_a: str, name_b: str, window: int = 160) -> str:
    lower = text.lower()
    indexes = []
    for name in [name_a, name_b, query_name(name_a), query_name(name_b), norm_name(name_a), norm_name(name_b)]:
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


def section_match(section: dict[str, str], left: str, right: str) -> dict[str, Any] | None:
    text = section["text"]
    left_presence = exact_presence(text, left)
    right_presence = exact_presence(text, right)
    if not (left_presence["present"] and right_presence["present"]):
        return None
    match_kind = "exact_hit" if left_presence["exact"] and right_presence["exact"] else "normalized_hit"
    return {
        "field": section["field"],
        "match_kind": match_kind,
        "left_presence": left_presence,
        "right_presence": right_presence,
        "section_text_sha256": sha256_bytes(text.encode("utf-8")),
        "section_text_bytes": len(text.encode("utf-8")),
        "snippet": snippet(text, left, right),
        "is_safety_section": section["field"] in SAFETY_FIELD_HINTS,
        "is_interaction_section": section["field"] in INTERACTION_FIELD_HINTS,
    }


def result_label_id(result: dict[str, Any]) -> str:
    return clean_text(result.get("id") or result.get("set_id") or stable_id(json.dumps(result, sort_keys=True)))


def openfda_list(result: dict[str, Any], field: str) -> list[str]:
    openfda = result.get("openfda") or {}
    value = openfda.get(field)
    if isinstance(value, list):
        return uniq(value)
    if value:
        return [clean_text(value)]
    return []


def record_url(result: dict[str, Any]) -> str:
    label_id = result_label_id(result)
    return f"{OPENFDA_LABEL_ENDPOINT}?search=id:%22{urllib.parse.quote(label_id)}%22"


def dailymed_url(result: dict[str, Any]) -> str:
    set_id = clean_text(result.get("set_id") or result.get("id"))
    if not set_id:
        return ""
    return f"https://dailymed.nlm.nih.gov/dailymed/drugInfo.cfm?setid={urllib.parse.quote(set_id)}"


def evidence_status(evidence: list[dict[str, Any]]) -> str:
    if any(row["safety_section_match"] for row in evidence):
        return "independent_openfda_safety_label_hit_still_blocked"
    if any(row["interaction_section_match"] for row in evidence):
        return "independent_openfda_interaction_label_hit_still_blocked"
    if evidence:
        return "independent_openfda_label_text_hit_still_blocked"
    return "independent_openfda_no_hit_still_blocked"


def build_evidence_rows(query_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    for query in query_rows:
        results = query.get("response_json", {}).get("results") or []
        for result_index, result in enumerate(results, start=1):
            matches = [
                match
                for section in label_sections(result)
                if (match := section_match(section, query["query_drug_a"], query["query_drug_b"])) is not None
            ]
            if not matches:
                continue
            best_kind = "exact_hit" if any(match["match_kind"] == "exact_hit" for match in matches) else "normalized_hit"
            label_id = result_label_id(result)
            evidence.append(
                {
                    "schema_version": 1,
                    "evidence_id": "openfda-independent-safety-evidence:" + stable_id(query["pair_key"], label_id, result_index),
                    "pair_key": query["pair_key"],
                    "representative_pair_id": query["representative_pair_id"],
                    "drug_a": query["drug_a"],
                    "drug_b": query["drug_b"],
                    "query_drug_a": query["query_drug_a"],
                    "query_drug_b": query["query_drug_b"],
                    "openfda_label_id": label_id,
                    "set_id": clean_text(result.get("set_id")),
                    "effective_time": clean_text(result.get("effective_time")),
                    "version": clean_text(result.get("version")),
                    "openfda_brand_names": openfda_list(result, "brand_name"),
                    "openfda_generic_names": openfda_list(result, "generic_name"),
                    "openfda_substance_names": openfda_list(result, "substance_name"),
                    "openfda_product_ndcs": openfda_list(result, "product_ndc"),
                    "openfda_manufacturer_names": openfda_list(result, "manufacturer_name"),
                    "openfda_product_types": openfda_list(result, "product_type"),
                    "source_url": record_url(result),
                    "dailymed_url": dailymed_url(result),
                    "query_url": query["query_url"],
                    "response_sha256": query["response_sha256"],
                    "match_kind": best_kind,
                    "matched_sections": matches,
                    "matched_section_fields": uniq([match["field"] for match in matches]),
                    "safety_section_match": any(match["is_safety_section"] for match in matches),
                    "interaction_section_match": any(match["is_interaction_section"] for match in matches),
                    "label_result_sha256": sha256_bytes(
                        json.dumps(result, sort_keys=True).encode("utf-8", errors="replace")
                    ),
                    "promotion_status": PROMOTION_STATUS,
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                    "reason_codes": reason_codes_for_evidence(best_kind, matches),
                }
            )
    evidence.sort(key=lambda row: (row["pair_key"], row["openfda_label_id"], row["evidence_id"]))
    return evidence


def reason_codes_for_evidence(match_kind: str, matches: list[dict[str, Any]]) -> list[str]:
    codes = [
        "openfda_independent_label_match_not_clinical_clearance",
        "requires_independent_safety_falsification_and_human_review",
    ]
    codes.append("both_candidate_names_exact_in_label_section" if match_kind == "exact_hit" else "both_candidate_names_normalized_in_label_section")
    if any(match["is_safety_section"] for match in matches):
        codes.append("independent_label_safety_section_match_not_safety_clearance")
    if any(match["is_interaction_section"] for match in matches):
        codes.append("independent_label_interaction_section_match_not_interaction_clearance")
    return codes


def pair_status_rows(pairs: list[dict[str, Any]], query_rows: list[dict[str, Any]], evidence: list[dict[str, Any]]) -> list[dict[str, Any]]:
    query_by_key = {row["pair_key"]: row for row in query_rows}
    evidence_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence:
        evidence_by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for pair in pairs:
        query = query_by_key.get(pair["pair_key"])
        ev = evidence_by_key.get(pair["pair_key"], [])
        if not pair["queryable"]:
            status = "independent_openfda_not_queryable_still_blocked"
        else:
            status = evidence_status(ev)
        reason_codes = ["openfda_independent_safety_validation_not_clinical_clearance"]
        if not pair["queryable"]:
            reason_codes.append("pair_not_queryable_after_name_normalization")
        if query and query["http_status"] == 404:
            reason_codes.append("openfda_query_no_results")
        if query and query["openfda_total"] > query["returned_result_count"]:
            reason_codes.append("openfda_query_result_truncated_to_limit")
        if query and query["openfda_total"] > 0 and not ev:
            reason_codes.append("openfda_query_returned_but_no_verified_section_pair_match")
        if ev:
            reason_codes.append("openfda_label_section_pair_match")
        rows.append(
            {
                "schema_version": 1,
                "pair_status_id": "openfda-independent-safety-pair-status:" + stable_id(pair["pair_key"], status),
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "rollup_review_ids": pair["rollup_review_ids"],
                "rollup_row_count": pair["rollup_row_count"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "queryable": pair["queryable"],
                "independent_openfda_status": status,
                "openfda_total": query["openfda_total"] if query else 0,
                "returned_result_count": query["returned_result_count"] if query else 0,
                "query_http_status": query["http_status"] if query else None,
                "query_response_sha256": query["response_sha256"] if query else None,
                "query_url": query["query_url"] if query else "",
                "label_evidence_rows": len(ev),
                "label_ids": uniq([row["openfda_label_id"] for row in ev])[:20],
                "evidence_ids": [row["evidence_id"] for row in ev[:20]],
                "matched_section_fields": uniq([field for row in ev for field in row["matched_section_fields"]])[:30],
                "safety_section_match": any(row["safety_section_match"] for row in ev),
                "interaction_section_match": any(row["interaction_section_match"] for row in ev),
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": reason_codes,
                "next_validation_experiment": next_validation(status),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda row: (row["independent_openfda_status"], row["pair_key"]))
    return rows


def next_validation(status: str) -> str:
    if status == "independent_openfda_no_hit_still_blocked":
        return "Continue independent source expansion; do not treat no-hit as safety clearance."
    return "Route through human safety/falsification review; do not emit safety clearance or guidance."


def rollup_status_rows(rollups: list[dict[str, Any]], pair_status: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_key = {row["pair_key"]: row for row in pair_status}
    rows: list[dict[str, Any]] = []
    for rollup in sorted(rollups, key=lambda row: (row["pair_key"], row["pair_id"])):
        pair = by_key[rollup["pair_key"]]
        rows.append(
            {
                "schema_version": 1,
                "status_id": "openfda-independent-safety-rollup-status:" + stable_id(rollup["rollup_review_id"], pair["pair_status_id"]),
                "source_issue1246_rollup_review_id": rollup["rollup_review_id"],
                "pair_id": rollup["pair_id"],
                "pair_key": rollup["pair_key"],
                "drug_a": rollup["drug_a"],
                "drug_b": rollup["drug_b"],
                "issue1246_safety_counter_category": rollup["safety_counter_review_category"],
                "independent_openfda_status": pair["independent_openfda_status"],
                "pair_status_id": pair["pair_status_id"],
                "openfda_total": pair["openfda_total"],
                "label_evidence_rows": pair["label_evidence_rows"],
                "label_ids": pair["label_ids"],
                "evidence_ids": pair["evidence_ids"],
                "matched_section_fields": pair["matched_section_fields"],
                "safety_section_match": pair["safety_section_match"],
                "interaction_section_match": pair["interaction_section_match"],
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": pair["reason_codes"],
                "next_validation_experiment": pair["next_validation_experiment"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def build_bridge_rows(rollup_status: list[dict[str, Any]], evidence: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        text = (
            f"openFDA independent safety validation rollup {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['independent_openfda_status']} "
            f"issue1246 category {row['issue1246_safety_counter_category']} label evidence rows "
            f"{row['label_evidence_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["status_id"],
                "domain": "openfda_independent_safety_rollup_status",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["pair_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["independent_openfda_status"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1248_openfda_independent_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_id": row["pair_id"],
                    "pair_key": row["pair_key"],
                    "independent_openfda_status": row["independent_openfda_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    budget = max(0, 1000 - len(rows))
    for row in evidence[:budget]:
        text = (
            f"openFDA independent safety evidence {row['evidence_id']} label {row['openfda_label_id']} "
            f"pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} match {row['match_kind']} "
            f"safety section {row['safety_section_match']} interaction section {row['interaction_section_match']}."
        )
        rows.append(
            {
                "id": row["evidence_id"],
                "domain": "openfda_independent_safety_evidence",
                "text": text,
                "bridge_terms": [
                    term
                    for term in uniq([row["evidence_id"], row["openfda_label_id"], row["pair_key"], row["drug_a"], row["drug_b"], row["match_kind"]])
                    if term and clean_text(term) in text
                ],
                "metadata": {
                    "source_dataset": "issue1248_openfda_independent_safety_validation",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "openfda_label_id": row["openfda_label_id"],
                    "match_kind": row["match_kind"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def response_schema_fingerprint(query_rows: list[dict[str, Any]]) -> dict[str, Any]:
    result_keys: set[str] = set()
    openfda_keys: set[str] = set()
    for row in query_rows:
        for result in row.get("response_json", {}).get("results") or []:
            result_keys.update(result.keys())
            openfda = result.get("openfda") or {}
            if isinstance(openfda, dict):
                openfda_keys.update(openfda.keys())
    payload = {"result_keys": sorted(result_keys), "openfda_keys": sorted(openfda_keys)}
    return {**payload, "schema_fingerprint_sha256": sha256_bytes(json.dumps(payload, sort_keys=True).encode("utf-8"))}


def build_metrics(
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1246_rollup_rows": len(rollups),
        "unique_pair_keys": len(pairs),
        "queryable_pair_keys": sum(1 for row in pairs if row["queryable"]),
        "openfda_query_response_rows": len(query_rows),
        "openfda_label_evidence_rows": len(evidence),
        "pair_status_rows": len(pair_status),
        "rollup_status_rows": len(rollup_status),
        "query_http_status_counts": dict(sorted(Counter(str(row["http_status"]) for row in query_rows).items())),
        "pair_status_counts": dict(sorted(Counter(row["independent_openfda_status"] for row in pair_status).items())),
        "rollup_status_counts": dict(sorted(Counter(row["independent_openfda_status"] for row in rollup_status).items())),
        "issue1246_category_counts": dict(sorted(Counter(row["issue1246_safety_counter_category"] for row in rollup_status).items())),
        "openfda_query_rows_with_total_gt_0": sum(1 for row in query_rows if row["openfda_total"] > 0),
        "openfda_query_rows_with_verified_evidence": len({row["pair_key"] for row in evidence}),
        "evidence_rows_with_safety_section": sum(1 for row in evidence if row["safety_section_match"]),
        "evidence_rows_with_interaction_section": sum(1 for row in evidence if row["interaction_section_match"]),
        "matched_section_field_counts": dict(sorted(Counter(field for row in evidence for field in row["matched_section_fields"]).items())),
        "response_schema": response_schema_fingerprint(query_rows),
        "top_hits": [
            {
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "status": row["independent_openfda_status"],
                "label_evidence_rows": row["label_evidence_rows"],
                "openfda_total": row["openfda_total"],
                "matched_section_fields": row["matched_section_fields"][:8],
            }
            for row in pair_status
            if row["label_evidence_rows"] > 0
        ][:50],
    }


def build_input_manifest(
    inputs: dict[str, str],
    raw_artifacts: dict[str, dict[str, Any]],
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    issue1246_persisted_readback: dict[str, Any],
    issue1246_calyx_readback: dict[str, Any],
    download_manifest: dict[str, Any],
) -> dict[str, Any]:
    label_info = download_manifest.get("results", {}).get("drug", {}).get("label", {})
    return {
        "schema_version": 1,
        "issue": 1248,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1246_rollup": artifact(Path(inputs["issue1246_rollup"]), jsonl=True),
            "issue1246_evidence_review": artifact(Path(inputs["issue1246_evidence_review"]), jsonl=True),
            "issue1246_persisted_readback": artifact(Path(inputs["issue1246_persisted_readback"])),
            "issue1246_calyx_readback": artifact(Path(inputs["issue1246_calyx_readback"])),
            "issue1246_output_manifest": artifact(Path(inputs["issue1246_output_manifest"])),
            **raw_artifacts,
        },
        "source_contract": {
            "issue1246_persisted_assertions_all_true": all_assertions_true(issue1246_persisted_readback),
            "issue1246_calyx_assertions_all_true": all_assertions_true(issue1246_calyx_readback),
            "issue1246_rollup_rows": len(rollups),
            "unique_pair_keys": len(pairs),
            "openfda_label_export_date": label_info.get("export_date"),
            "openfda_label_total_records": label_info.get("total_records"),
            "openfda_label_partitions": len(label_info.get("partitions") or []),
        },
    }


def build_readback(
    out_dir: Path,
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    issue1246_persisted_readback: dict[str, Any],
    issue1246_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "openfda_independent_query_responses": artifact(out_dir / "openfda_independent_query_responses.jsonl", jsonl=True),
        "openfda_independent_label_evidence": artifact(out_dir / "openfda_independent_label_evidence.jsonl", jsonl=True),
        "openfda_independent_pair_status": artifact(out_dir / "openfda_independent_pair_status.jsonl", jsonl=True),
        "candidate_openfda_independent_status": artifact(out_dir / "candidate_openfda_independent_status.jsonl", jsonl=True),
        "openfda_independent_bridge_rows": artifact(out_dir / "openfda_independent_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    queryable_keys = {row["pair_key"] for row in pairs if row["queryable"]}
    query_keys = {row["pair_key"] for row in query_rows}
    pair_keys = {row["pair_key"] for row in pairs}
    status_keys = {row["pair_key"] for row in pair_status}
    rollup_ids = {row["rollup_review_id"] for row in rollups}
    status_rollup_ids = {row["source_issue1246_rollup_review_id"] for row in rollup_status}
    evidence_keys = {row["pair_key"] for row in evidence}
    hit_keys = {row["pair_key"] for row in pair_status if row["label_evidence_rows"] > 0}
    return {
        "schema_version": 1,
        "issue": 1248,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1246_persisted_readback_all_true": all_assertions_true(issue1246_persisted_readback),
            "issue1246_calyx_readback_all_true": all_assertions_true(issue1246_calyx_readback),
            "query_response_for_every_queryable_pair_key": query_keys == queryable_keys,
            "pair_status_for_every_pair_key": status_keys == pair_keys,
            "rollup_status_for_every_issue1246_rollup": status_rollup_ids == rollup_ids,
            "all_pair_status_values_allowed": all(row["independent_openfda_status"] in STATUS_VALUES for row in pair_status),
            "all_rollup_status_values_allowed": all(row["independent_openfda_status"] in STATUS_VALUES for row in rollup_status),
            "all_hits_have_evidence": hit_keys <= evidence_keys,
            "all_evidence_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence),
            "all_status_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in pair_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status),
            "all_evidence_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in evidence),
            "all_rollup_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status),
            "bridge_rows_1000_or_less": artifacts["openfda_independent_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "issue1246_rollups": len(rollups),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence),
            "pair_status_rows": len(pair_status),
            "rollup_status_rows": len(rollup_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_pairs: int | None, request_sleep_seconds: float) -> dict[str, Any]:
    require_inputs(inputs)
    raw_dir = root / "raw"
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_artifacts = fetch_raw_sources(raw_dir)
    download_manifest = read_json(raw_dir / "openfda_download_manifest.json")
    rollups = rows_jsonl(Path(inputs["issue1246_rollup"]))
    pairs = representative_pairs(rollups)
    if max_pairs is not None:
        keep = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in keep]
        rollups = [row for row in rollups if row["pair_key"] in keep]
    issue1246_persisted_readback = read_json(Path(inputs["issue1246_persisted_readback"]))
    issue1246_calyx_readback = read_json(Path(inputs["issue1246_calyx_readback"]))
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            raw_artifacts,
            rollups,
            pairs,
            issue1246_persisted_readback,
            issue1246_calyx_readback,
            download_manifest,
        ),
    )
    query_rows = fetch_openfda_queries(
        pairs,
        out_dir / "openfda_independent_query_responses.jsonl",
        request_sleep_seconds,
    )
    evidence = build_evidence_rows(query_rows)
    write_jsonl(out_dir / "openfda_independent_label_evidence.jsonl", evidence)
    pair_status = pair_status_rows(pairs, query_rows, evidence)
    write_jsonl(out_dir / "openfda_independent_pair_status.jsonl", pair_status)
    rollup_status = rollup_status_rows(rollups, pair_status)
    write_jsonl(out_dir / "candidate_openfda_independent_status.jsonl", rollup_status)
    source_path = out_dir / "candidate_openfda_independent_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, evidence, source_path, source_sha)
    write_jsonl(out_dir / "openfda_independent_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(rollups, pairs, query_rows, evidence, pair_status, rollup_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1248,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "openfda_independent_query_responses": artifact(out_dir / "openfda_independent_query_responses.jsonl", jsonl=True),
            "openfda_independent_label_evidence": artifact(out_dir / "openfda_independent_label_evidence.jsonl", jsonl=True),
            "openfda_independent_pair_status": artifact(out_dir / "openfda_independent_pair_status.jsonl", jsonl=True),
            "candidate_openfda_independent_status": artifact(out_dir / "candidate_openfda_independent_status.jsonl", jsonl=True),
            "openfda_independent_bridge_rows": artifact(out_dir / "openfda_independent_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        rollups,
        pairs,
        query_rows,
        evidence,
        pair_status,
        rollup_status,
        issue1246_persisted_readback,
        issue1246_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "candidate_openfda_independent_status": output_manifest["artifacts"]["candidate_openfda_independent_status"],
            "openfda_independent_label_evidence": output_manifest["artifacts"]["openfda_independent_label_evidence"],
            "bridge_rows": output_manifest["artifacts"]["openfda_independent_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1246-rollup")
    parser.add_argument("--issue1246-evidence-review")
    parser.add_argument("--issue1246-persisted-readback")
    parser.add_argument("--issue1246-calyx-readback")
    parser.add_argument("--issue1246-output-manifest")
    parser.add_argument("--max-pairs", type=int)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1246_rollup", "issue1246_rollup"),
        ("issue1246_evidence_review", "issue1246_evidence_review"),
        ("issue1246_persisted_readback", "issue1246_persisted_readback"),
        ("issue1246_calyx_readback", "issue1246_calyx_readback"),
        ("issue1246_output_manifest", "issue1246_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_pairs=args.max_pairs, request_sleep_seconds=args.request_sleep_seconds)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
