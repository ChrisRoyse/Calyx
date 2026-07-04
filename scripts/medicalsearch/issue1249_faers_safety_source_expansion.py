#!/usr/bin/env python3
"""#1249 openFDA FAERS safety-source expansion after label no-hit.

This stage reads sealed #1248 openFDA label no-hit rows and queries the
distinct openFDA FAERS drug event endpoint for pair co-report evidence. FAERS
event co-report rows are adverse-event source triage only; every row remains
blocked pending safety/falsification and human review.
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
    "openFDA FAERS event expansion is adverse-event source triage only; not "
    "safety clearance, contraindication guidance, treatment guidance, dosing "
    "guidance, recommendation, clinical actionability, efficacy, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "openfda_faers_event_coreport_not_safety_clearance_or_clinical_actionability"
)

ISSUE1248_ROOT = "/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1249-openfda-faers-safety-expansion-20260704T203500Z"

DEFAULT_INPUTS = {
    "issue1248_rollup_status": f"{ISSUE1248_ROOT}/out/candidate_openfda_independent_status.jsonl",
    "issue1248_pair_status": f"{ISSUE1248_ROOT}/out/openfda_independent_pair_status.jsonl",
    "issue1248_persisted_readback": f"{ISSUE1248_ROOT}/out/persisted_readback.json",
    "issue1248_calyx_readback": f"{ISSUE1248_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1248_output_manifest": f"{ISSUE1248_ROOT}/out/output_manifest.json",
}

OPENFDA_EVENT_ENDPOINT = "https://api.fda.gov/drug/event.json"
OPENFDA_EVENT_DOC_URL = "https://open.fda.gov/apis/drug/event/"
OPENFDA_EVENT_FIELDS_URL = "https://open.fda.gov/apis/drug/event/searchable-fields/"
OPENFDA_DOWNLOAD_DOC_URL = "https://open.fda.gov/data/downloads/"

REQUEST_SLEEP_SECONDS = 0.08
USER_AGENT = "calyx-discovery/issue1249"
PROMOTION_STATUS = "blocked_requires_independent_safety_falsification_and_human_review"

ROLLUP_INPUT_STATUS = "independent_openfda_no_hit_still_blocked"

PAIR_STATUS_VALUES = {
    "faers_pair_coreport_serious_event_hit_still_blocked",
    "faers_pair_coreport_event_hit_still_blocked",
    "faers_pair_query_no_result_still_blocked",
    "faers_pair_query_result_without_verified_pair_terms_still_blocked",
    "faers_pair_not_queryable_still_blocked",
}

ROLLUP_STATUS_VALUES = {
    "faers_rollup_coreport_serious_event_hit_still_blocked",
    "faers_rollup_coreport_event_hit_still_blocked",
    "faers_rollup_query_no_result_still_blocked",
    "faers_rollup_query_result_without_verified_pair_terms_still_blocked",
    "faers_rollup_not_queryable_still_blocked",
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
    value = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
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


def fetch_bytes(url: str, retries: int = 4) -> tuple[int, bytes]:
    last_error: Exception | None = None
    for attempt in range(retries):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=90) as response:
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
        "openfda_drug_event_api": (OPENFDA_EVENT_DOC_URL, "openfda_drug_event_api.html"),
        "openfda_drug_event_fields": (OPENFDA_EVENT_FIELDS_URL, "openfda_drug_event_fields.html"),
        "openfda_download_docs": (OPENFDA_DOWNLOAD_DOC_URL, "openfda_download_docs.html"),
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


def source_rollups(rows: list[dict[str, Any]], max_rollups: int | None = None) -> list[dict[str, Any]]:
    out = [row for row in rows if row.get("independent_openfda_status") == ROLLUP_INPUT_STATUS]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_rollups is not None:
        out = out[:max_rollups]
    return out


def pair_rows(rollups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rollups:
        grouped[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        query_a = query_name(first["drug_a"])
        query_b = query_name(first["drug_b"])
        out.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query_drug_a": query_a,
                "query_drug_b": query_b,
                "representative_pair_id": first["pair_id"],
                "source_rollup_status_ids": [row["status_id"] for row in members],
                "source_pair_ids": [row["pair_id"] for row in members],
                "source_issue1246_rollup_ids": [row["source_issue1246_rollup_review_id"] for row in members],
                "queryable": bool(query_a and query_b),
            }
        )
    return out


def faers_query_url(left: str, right: str) -> tuple[str, str]:
    query = f'patient.drug.medicinalproduct:"{left}" AND patient.drug.medicinalproduct:"{right}"'
    params = {"search": query, "limit": "1"}
    return query, f"{OPENFDA_EVENT_ENDPOINT}?{urllib.parse.urlencode(params)}"


def query_faers(pairs: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    raw_dir.mkdir(parents=True, exist_ok=True)
    queryable = [row for row in pairs if row["queryable"]]
    for index, pair in enumerate(queryable, start=1):
        query, url = faers_query_url(pair["query_drug_a"], pair["query_drug_b"])
        status, payload = fetch_bytes(url)
        raw_path = raw_dir / f"openfda_faers_{stable_id(pair['pair_key'], query)}.json"
        raw_path.write_bytes(payload)
        try:
            response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
        except json.JSONDecodeError:
            response_json = {"decode_error": payload.decode("utf-8", errors="replace")[:1000]}
        total = 0
        results = []
        if isinstance(response_json, dict):
            total = int((response_json.get("meta", {}).get("results", {}) or {}).get("total") or 0)
            raw_results = response_json.get("results") or []
            results = raw_results if isinstance(raw_results, list) else []
        rows.append(
            {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "api_endpoint": OPENFDA_EVENT_ENDPOINT,
                "query": query,
                "query_url": url,
                "http_status": status,
                "raw_response_path": str(raw_path),
                "raw_response_bytes": len(payload),
                "raw_response_sha256": sha256_bytes(payload),
                "openfda_total": total,
                "returned_result_count": len(results),
                "first_result": results[0] if results else None,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
        print(
            f"#1249 FAERS query {index}/{len(queryable)} pair={pair['pair_key']} status={status} total={total}",
            file=sys.stderr,
        )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def drug_texts(event: dict[str, Any]) -> list[str]:
    out: list[str] = []
    patient = event.get("patient") or {}
    drugs = patient.get("drug") or []
    if isinstance(drugs, dict):
        drugs = [drugs]
    for drug in drugs if isinstance(drugs, list) else []:
        if not isinstance(drug, dict):
            continue
        out.extend(
            clean_text(drug.get(key))
            for key in [
                "medicinalproduct",
                "drugcharacterization",
                "drugauthorizationnumb",
                "drugdosagetext",
                "drugindication",
            ]
        )
        openfda = drug.get("openfda") or {}
        if isinstance(openfda, dict):
            out.extend(clean_text(value) for value in openfda.values())
    return [text for text in out if text]


def reaction_terms(event: dict[str, Any]) -> list[str]:
    patient = event.get("patient") or {}
    reactions = patient.get("reaction") or []
    if isinstance(reactions, dict):
        reactions = [reactions]
    terms: list[str] = []
    for reaction in reactions if isinstance(reactions, list) else []:
        if isinstance(reaction, dict):
            terms.append(clean_text(reaction.get("reactionmeddrapt")))
    return uniq(terms)


def seriousness_flags(event: dict[str, Any]) -> dict[str, Any]:
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
    return {
        "flags": flags,
        "serious": any(value == "1" for value in flags.values()),
        "death": flags.get("seriousnessdeath") == "1",
    }


def event_source_id(event: dict[str, Any]) -> str:
    for key in ["safetyreportid", "safetyreportversion", "receiptdate"]:
        text = clean_text(event.get(key))
        if text:
            return text
    return stable_id(event, length=16)


def build_evidence_rows(query_rows: list[dict[str, Any]], pair_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for query in query_rows:
        event = query.get("first_result")
        if not isinstance(event, dict):
            continue
        pair = pair_lookup[query["pair_key"]]
        source_drug_text = " ".join(drug_texts(event))
        left = exact_presence(source_drug_text, pair["drug_a"])
        right = exact_presence(source_drug_text, pair["drug_b"])
        if not (left["present"] and right["present"]):
            continue
        seriousness = seriousness_flags(event)
        evidence_status = (
            "faers_pair_coreport_serious_event_hit_still_blocked"
            if seriousness["serious"]
            else "faers_pair_coreport_event_hit_still_blocked"
        )
        source_id = event_source_id(event)
        reactions = reaction_terms(event)
        event_structured_text = clean_text(
            {
                "safetyreportid": event.get("safetyreportid"),
                "seriousness": seriousness["flags"],
                "drugs": source_drug_text,
                "reactions": reactions,
            }
        )
        rows.append(
            {
                "schema_version": 1,
                "faers_evidence_id": "faers-event-evidence:" + stable_id(query["pair_key"], source_id),
                "pair_key": query["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "faers_pair_status": evidence_status,
                "source": "openFDA Drug Event API",
                "source_id": source_id,
                "source_url": query["query_url"],
                "source_structured_field": "patient.drug.medicinalproduct",
                "source_structured_text": event_structured_text,
                "source_structured_sha256": hashlib.sha256(event_structured_text.encode("utf-8")).hexdigest(),
                "raw_response_path": query["raw_response_path"],
                "raw_response_sha256": query["raw_response_sha256"],
                "openfda_total": query["openfda_total"],
                "pair_term_presence": {
                    "left": left,
                    "right": right,
                    "both_present_in_event_drug_fields": left["present"] and right["present"],
                    "both_exact_in_event_drug_fields": left["exact"] and right["exact"],
                },
                "seriousness": seriousness,
                "reaction_terms": reactions,
                "receiptdate": clean_text(event.get("receiptdate")),
                "receivedate": clean_text(event.get("receivedate")),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "faers_event_coreport_not_safety_clearance",
                    "requires_independent_safety_falsification_and_human_review",
                ],
            }
        )
    rows.sort(key=lambda row: (row["pair_key"], row["source_id"]))
    return rows


def build_pair_status(pairs: list[dict[str, Any]], query_rows: list[dict[str, Any]], evidence_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    query_by_key = {row["pair_key"]: row for row in query_rows}
    evidence_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for pair in pairs:
        evidence = evidence_by_key.get(pair["pair_key"], [])
        query = query_by_key.get(pair["pair_key"])
        if not pair["queryable"]:
            status = "faers_pair_not_queryable_still_blocked"
        elif evidence and any(row["seriousness"]["serious"] for row in evidence):
            status = "faers_pair_coreport_serious_event_hit_still_blocked"
        elif evidence:
            status = "faers_pair_coreport_event_hit_still_blocked"
        elif query and query.get("openfda_total", 0) > 0:
            status = "faers_pair_query_result_without_verified_pair_terms_still_blocked"
        else:
            status = "faers_pair_query_no_result_still_blocked"
        rows.append(
            {
                "schema_version": 1,
                "faers_pair_status_id": "faers-pair-status:" + stable_id(pair["pair_key"]),
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "faers_pair_status": status,
                "openfda_total": int(query.get("openfda_total", 0)) if query else 0,
                "http_status": int(query.get("http_status", 0)) if query else 0,
                "evidence_ids": [row["faers_evidence_id"] for row in evidence],
                "faers_evidence_rows": len(evidence),
                "source_pair_ids": pair["source_pair_ids"],
                "source_issue1246_rollup_ids": pair["source_issue1246_rollup_ids"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def rollup_status_from_pair(pair_status: str) -> str:
    return {
        "faers_pair_coreport_serious_event_hit_still_blocked": "faers_rollup_coreport_serious_event_hit_still_blocked",
        "faers_pair_coreport_event_hit_still_blocked": "faers_rollup_coreport_event_hit_still_blocked",
        "faers_pair_query_no_result_still_blocked": "faers_rollup_query_no_result_still_blocked",
        "faers_pair_query_result_without_verified_pair_terms_still_blocked": (
            "faers_rollup_query_result_without_verified_pair_terms_still_blocked"
        ),
        "faers_pair_not_queryable_still_blocked": "faers_rollup_not_queryable_still_blocked",
    }[pair_status]


def rollup_reason_codes(status: str) -> list[str]:
    codes = [
        "faers_event_expansion_not_safety_clearance",
        "requires_independent_safety_falsification_and_human_review",
    ]
    if "coreport_serious" in status:
        codes.append("faers_serious_event_coreport_requires_review")
    elif "coreport_event" in status:
        codes.append("faers_event_coreport_requires_review")
    elif "without_verified_pair_terms" in status:
        codes.append("faers_query_returned_result_but_pair_terms_not_verified")
    elif "not_queryable" in status:
        codes.append("faers_pair_not_queryable")
    else:
        codes.append("faers_query_no_result")
    return codes


def build_rollup_status(
    rollups: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    pair_by_key = {row["pair_key"]: row for row in pair_status}
    evidence_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for rollup in rollups:
        pair = pair_by_key[rollup["pair_key"]]
        status = rollup_status_from_pair(pair["faers_pair_status"])
        evidence = evidence_by_key.get(rollup["pair_key"], [])
        rows.append(
            {
                "schema_version": 1,
                "faers_rollup_status_id": "faers-rollup-status:" + stable_id(rollup["status_id"], rollup["pair_key"]),
                "source_issue1248_status_id": rollup["status_id"],
                "source_issue1248_pair_status_id": rollup["pair_status_id"],
                "source_issue1246_rollup_review_id": rollup["source_issue1246_rollup_review_id"],
                "pair_id": rollup["pair_id"],
                "pair_key": rollup["pair_key"],
                "drug_a": rollup["drug_a"],
                "drug_b": rollup["drug_b"],
                "issue1246_safety_counter_category": rollup.get("issue1246_safety_counter_category"),
                "source_issue1248_status": rollup["independent_openfda_status"],
                "faers_rollup_status": status,
                "faers_pair_status": pair["faers_pair_status"],
                "openfda_total": pair["openfda_total"],
                "evidence_ids": [row["faers_evidence_id"] for row in evidence],
                "faers_evidence_rows": len(evidence),
                "reaction_terms": uniq(sum((row.get("reaction_terms", []) for row in evidence), [])),
                "reason_codes": rollup_reason_codes(status),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def build_bridge_rows(
    rollup_status: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in rollup_status:
        rows.append(
            {
                "id": row["faers_rollup_status_id"],
                "domain": "faers_safety_rollup",
                "text": (
                    f"FAERS safety rollup {row['pair_id']} pair {row['pair_key']} "
                    f"{row['drug_a']} plus {row['drug_b']} status {row['faers_rollup_status']} "
                    f"evidence rows {row['faers_evidence_rows']} promotion {row['promotion_status']}."
                ),
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["faers_rollup_status"]]),
                "metadata": {
                    "source_dataset": "issue1249_faers_safety_source_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "faers_rollup_status": row["faers_rollup_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        rows.append(
            {
                "id": row["faers_pair_status_id"],
                "domain": "faers_safety_pair_status",
                "text": (
                    f"FAERS pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
                    f"status {row['faers_pair_status']} total {row['openfda_total']}."
                ),
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["faers_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1249_faers_safety_source_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "faers_pair_status": row["faers_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in evidence_rows:
        reaction_terms = row.get("reaction_terms", [])[:8]
        rows.append(
            {
                "id": row["faers_evidence_id"],
                "domain": "faers_safety_evidence",
                "text": (
                    f"FAERS event evidence {row['source_id']} pair {row['pair_key']} "
                    f"status {row['faers_pair_status']} reactions {'; '.join(reaction_terms)}."
                ),
                "bridge_terms": uniq(
                    [row["pair_key"], row["source_id"], row["faers_pair_status"]]
                    + reaction_terms
                ),
                "metadata": {
                    "source_dataset": "issue1249_faers_safety_source_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_id": row["source_id"],
                    "faers_pair_status": row["faers_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in query_rows:
        rows.append(
            {
                "id": "faers-query:" + stable_id(row["pair_key"]),
                "domain": "faers_safety_query",
                "text": (
                    f"FAERS query for pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
                    f"HTTP {row['http_status']} total {row['openfda_total']}."
                ),
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], str(row["http_status"])]),
                "metadata": {
                    "source_dataset": "issue1249_faers_safety_source_expansion",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "http_status": str(row["http_status"]),
                    "raw_response_sha256": row["raw_response_sha256"],
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
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1248_rollups": len(rollups),
        "unique_pair_keys": len(pairs),
        "query_response_rows": len(query_rows),
        "faers_evidence_rows": len(evidence_rows),
        "pair_status_rows": len(pair_status),
        "rollup_status_rows": len(rollup_status),
        "query_http_status_counts": dict(sorted(Counter(str(row["http_status"]) for row in query_rows).items())),
        "pair_status_counts": dict(sorted(Counter(row["faers_pair_status"] for row in pair_status).items())),
        "rollup_status_counts": dict(sorted(Counter(row["faers_rollup_status"] for row in rollup_status).items())),
        "rollups_with_faers_event_coreport": sum(
            1
            for row in rollup_status
            if row["faers_rollup_status"]
            in {"faers_rollup_coreport_serious_event_hit_still_blocked", "faers_rollup_coreport_event_hit_still_blocked"}
        ),
        "rollups_with_serious_faers_coreport": sum(
            1 for row in rollup_status if row["faers_rollup_status"] == "faers_rollup_coreport_serious_event_hit_still_blocked"
        ),
        "all_rows_blocked": True,
    }


def build_input_manifest(
    inputs: dict[str, str],
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    raw_sources: dict[str, dict[str, Any]],
    issue1248_persisted_readback: dict[str, Any],
    issue1248_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1249,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1248_rollup_status": artifact(Path(inputs["issue1248_rollup_status"]), jsonl=True),
            "issue1248_pair_status": artifact(Path(inputs["issue1248_pair_status"]), jsonl=True),
            "issue1248_persisted_readback": artifact(Path(inputs["issue1248_persisted_readback"])),
            "issue1248_calyx_readback": artifact(Path(inputs["issue1248_calyx_readback"])),
            "issue1248_output_manifest": artifact(Path(inputs["issue1248_output_manifest"])),
        },
        "raw_source_docs": raw_sources,
        "source_contract": {
            "issue1248_persisted_assertions_all_true": all_assertions_true(issue1248_persisted_readback),
            "issue1248_calyx_assertions_all_true": all_assertions_true(issue1248_calyx_readback),
            "issue1248_rollups": len(rollups),
            "unique_pair_keys": len(pairs),
            "input_filter": f"independent_openfda_status == {ROLLUP_INPUT_STATUS}",
            "faers_event_coreport_is_not_safety_clearance": True,
        },
    }


def build_readback(
    out_dir: Path,
    rollups: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    rollup_status: list[dict[str, Any]],
    issue1248_persisted_readback: dict[str, Any],
    issue1248_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "faers_query_responses": artifact(out_dir / "faers_query_responses.jsonl", jsonl=True),
        "faers_event_evidence": artifact(out_dir / "faers_event_evidence.jsonl", jsonl=True),
        "faers_pair_status": artifact(out_dir / "faers_pair_status.jsonl", jsonl=True),
        "faers_rollup_status": artifact(out_dir / "faers_rollup_status.jsonl", jsonl=True),
        "faers_bridge_rows": artifact(out_dir / "faers_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    pair_keys = {row["pair_key"] for row in pairs if row["queryable"]}
    query_keys = {row["pair_key"] for row in query_rows}
    rollup_ids = {row["status_id"] for row in rollups}
    status_ids = {row["source_issue1248_status_id"] for row in rollup_status}
    evidence_pair_keys = {row["pair_key"] for row in evidence_rows}
    hit_pair_keys = {
        row["pair_key"]
        for row in pair_status
        if row["faers_pair_status"]
        in {"faers_pair_coreport_serious_event_hit_still_blocked", "faers_pair_coreport_event_hit_still_blocked"}
    }
    return {
        "schema_version": 1,
        "issue": 1249,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1248_persisted_readback_all_true": all_assertions_true(issue1248_persisted_readback),
            "issue1248_calyx_readback_all_true": all_assertions_true(issue1248_calyx_readback),
            "query_response_for_every_queryable_pair_key": query_keys == pair_keys,
            "pair_status_for_every_pair_key": {row["pair_key"] for row in pair_status} == {row["pair_key"] for row in pairs},
            "rollup_status_for_every_issue1248_rollup": status_ids == rollup_ids,
            "all_hits_have_evidence": hit_pair_keys <= evidence_pair_keys,
            "all_evidence_rows_have_source_hash": all(
                row["source_id"] and row["source_structured_sha256"] and row["raw_response_sha256"] for row in evidence_rows
            ),
            "all_evidence_rows_have_pair_terms": all(
                row["pair_term_presence"]["both_present_in_event_drug_fields"] for row in evidence_rows
            ),
            "all_pair_status_values_allowed": all(row["faers_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
            "all_rollup_status_values_allowed": all(row["faers_rollup_status"] in ROLLUP_STATUS_VALUES for row in rollup_status),
            "all_status_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in pair_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in rollup_status),
            "all_evidence_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence_rows),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in pair_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in rollup_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_rows),
            "bridge_rows_1000_or_less": artifacts["faers_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "issue1248_rollups": len(rollups),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence_rows),
            "pair_status_rows": len(pair_status),
            "rollup_status_rows": len(rollup_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_rollups: int | None = None, max_pairs: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    raw_dir = root / "raw"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    all_rollups = rows_jsonl(Path(inputs["issue1248_rollup_status"]))
    issue1248_persisted_readback = read_json(Path(inputs["issue1248_persisted_readback"]))
    issue1248_calyx_readback = read_json(Path(inputs["issue1248_calyx_readback"]))
    rollups = source_rollups(all_rollups, max_rollups=max_rollups)
    pairs = pair_rows(rollups)
    if max_pairs is not None:
        selected = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in selected]
        rollups = [row for row in rollups if row["pair_key"] in selected]
    pair_lookup = {row["pair_key"]: row for row in pairs}
    raw_sources = fetch_raw_sources(raw_dir)
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(inputs, rollups, pairs, raw_sources, issue1248_persisted_readback, issue1248_calyx_readback),
    )
    query_rows = query_faers(pairs, raw_dir)
    write_jsonl(out_dir / "faers_query_responses.jsonl", query_rows)
    evidence_rows = build_evidence_rows(query_rows, pair_lookup)
    write_jsonl(out_dir / "faers_event_evidence.jsonl", evidence_rows)
    pair_status = build_pair_status(pairs, query_rows, evidence_rows)
    write_jsonl(out_dir / "faers_pair_status.jsonl", pair_status)
    rollup_status = build_rollup_status(rollups, pair_status, evidence_rows)
    write_jsonl(out_dir / "faers_rollup_status.jsonl", rollup_status)
    source_path = out_dir / "faers_rollup_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(rollup_status, pair_status, evidence_rows, query_rows, source_path, source_sha)
    write_jsonl(out_dir / "faers_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(rollups, pairs, query_rows, evidence_rows, pair_status, rollup_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1249,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "faers_query_responses": artifact(out_dir / "faers_query_responses.jsonl", jsonl=True),
            "faers_event_evidence": artifact(out_dir / "faers_event_evidence.jsonl", jsonl=True),
            "faers_pair_status": artifact(out_dir / "faers_pair_status.jsonl", jsonl=True),
            "faers_rollup_status": artifact(out_dir / "faers_rollup_status.jsonl", jsonl=True),
            "faers_bridge_rows": artifact(out_dir / "faers_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        rollups,
        pairs,
        query_rows,
        evidence_rows,
        pair_status,
        rollup_status,
        issue1248_persisted_readback,
        issue1248_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    if not all(readback["assertions"].values()):
        raise AssertionError(f"Persisted readback assertions failed: {readback['assertions']}")
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "query_responses": output_manifest["artifacts"]["faers_query_responses"],
            "event_evidence": output_manifest["artifacts"]["faers_event_evidence"],
            "pair_status": output_manifest["artifacts"]["faers_pair_status"],
            "rollup_status": output_manifest["artifacts"]["faers_rollup_status"],
            "bridge_rows": output_manifest["artifacts"]["faers_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1248-rollup-status")
    parser.add_argument("--issue1248-pair-status")
    parser.add_argument("--issue1248-persisted-readback")
    parser.add_argument("--issue1248-calyx-readback")
    parser.add_argument("--issue1248-output-manifest")
    parser.add_argument("--max-rollups", type=int)
    parser.add_argument("--max-pairs", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1248_rollup_status", "issue1248_rollup_status"),
        ("issue1248_pair_status", "issue1248_pair_status"),
        ("issue1248_persisted_readback", "issue1248_persisted_readback"),
        ("issue1248_calyx_readback", "issue1248_calyx_readback"),
        ("issue1248_output_manifest", "issue1248_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_rollups=args.max_rollups, max_pairs=args.max_pairs)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
