#!/usr/bin/env python3
"""#1254 ChEMBL source mining after PubChem no-hit.

This stage reads sealed #1245 blocked candidate rows and queries a distinct
external source instrument: ChEMBL molecule search records. A hit requires a
returned ChEMBL structured record to physically contain both pair terms or
accepted normalized equivalents. Every row remains blocked.
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
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "ChEMBL source mining is molecule/source triage only; not efficacy, safety, "
    "treatment guidance, dosing guidance, recommendation, clinical actionability, "
    "pair-interaction evidence, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "chembl_molecule_record_two_term_match_not_pair_interaction_safety_efficacy_or_cure"
)

ISSUE1245_ROOT = "/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z"

DEFAULT_INPUTS = {
    "issue1245_candidate_status": f"{ISSUE1245_ROOT}/out/candidate_pubchem_status.jsonl",
    "issue1245_pair_status": f"{ISSUE1245_ROOT}/out/pubchem_pair_status.jsonl",
    "issue1245_persisted_readback": f"{ISSUE1245_ROOT}/out/persisted_readback.json",
    "issue1245_calyx_readback": f"{ISSUE1245_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1245_output_manifest": f"{ISSUE1245_ROOT}/out/output_manifest.json",
}

CHEMBL_DOC_URL = "https://www.ebi.ac.uk/chembl/api/data/docs"
CHEMBL_MOLECULE_SCHEMA_URL = "https://www.ebi.ac.uk/chembl/api/data/molecule.json?limit=1"
CHEMBL_MOLECULE_SEARCH_ENDPOINT = "https://www.ebi.ac.uk/chembl/api/data/molecule/search.json?q={query}&limit=20"

REQUEST_SLEEP_SECONDS = 0.15
USER_AGENT = "calyx-discovery/issue1254"
PROMOTION_STATUS = "blocked_requires_external_source_safety_outcome_falsification_and_human_review"

PAIR_STATUS_VALUES = {
    "chembl_pair_record_two_term_hit_still_blocked",
    "chembl_pair_record_without_pair_match_still_blocked",
    "chembl_pair_no_result_still_blocked",
    "chembl_pair_query_failed_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "chembl_candidate_record_two_term_hit_still_blocked",
    "chembl_candidate_record_without_pair_match_still_blocked",
    "chembl_candidate_no_result_still_blocked",
    "chembl_candidate_query_failed_still_blocked",
}


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
    text = re.sub(r"\bchembl[: ]?chembl", "chembl", text, flags=re.IGNORECASE)
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
        "chembl_rest_docs": (CHEMBL_DOC_URL, "chembl_rest_docs.html"),
        "chembl_molecule_schema_sample": (CHEMBL_MOLECULE_SCHEMA_URL, "chembl_molecule_schema_sample.json"),
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


def load_candidates(rows: list[dict[str, Any]], max_pairs: int | None = None) -> list[dict[str, Any]]:
    blocked_statuses = {
        "pubchem_synonym_record_without_pair_match_still_blocked",
        "pubchem_synonym_no_result_still_blocked",
        "pubchem_synonym_not_queryable_still_blocked",
    }
    out = [row for row in rows if row.get("pubchem_candidate_status") in blocked_statuses]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_pairs is not None:
        selected_pair_keys = sorted({row["pair_key"] for row in out})[:max_pairs]
        selected = set(selected_pair_keys)
        out = [row for row in out if row["pair_key"] in selected]
    return out


def pair_rows(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        grouped[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        query = " ".join([query_name(first["drug_a"]), query_name(first["drug_b"])]).strip()
        out.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query": query,
                "representative_pair_id": first.get("pair_id"),
                "source_pair_ids": sorted({row.get("pair_id") for row in members if row.get("pair_id")}),
                "source_pubchem_candidate_status_ids": sorted(
                    {
                        row.get("pubchem_candidate_status_id")
                        for row in members
                        if row.get("pubchem_candidate_status_id")
                    }
                ),
                "source_pubchem_pair_status_ids": sorted(
                    {row.get("pubchem_pair_status_id") for row in members if row.get("pubchem_pair_status_id")}
                ),
                "candidate_count": len(members),
            }
        )
    return out


def parse_json_payload(payload: bytes) -> dict[str, Any]:
    try:
        return json.loads(payload.decode("utf-8"))
    except Exception as error:  # noqa: BLE001 - persisted row carries parse failure.
        return {"parse_error": str(error)}


def molecule_records(payload_json: dict[str, Any]) -> list[dict[str, Any]]:
    molecules = payload_json.get("molecules", [])
    if isinstance(molecules, list):
        return [row for row in molecules if isinstance(row, dict)]
    return []


def total_count(payload_json: dict[str, Any]) -> int | None:
    page_meta = payload_json.get("page_meta", {})
    if isinstance(page_meta, dict) and isinstance(page_meta.get("total_count"), int):
        return page_meta["total_count"]
    return None


def record_source_id(record: dict[str, Any], fallback: str) -> str:
    chembl_id = clean_text(record.get("molecule_chembl_id"))
    pref_name = clean_text(record.get("pref_name"))
    if chembl_id:
        return f"CHEMBL:{chembl_id}"
    if pref_name:
        return f"CHEMBL-PREF:{pref_name}"
    return fallback


def fetch_pair_query(pair: dict[str, Any], raw_dir: Path) -> dict[str, Any]:
    encoded = urllib.parse.quote(pair["query"])
    url = CHEMBL_MOLECULE_SEARCH_ENDPOINT.format(query=encoded)
    raw_path = raw_dir / f"chembl_molecule_search_{stable_id(pair['pair_key'], pair['query'])}.json"
    if raw_path.exists():
        payload = raw_path.read_bytes()
        status_path = raw_dir / f"{raw_path.name}.status"
        status = int(status_path.read_text(encoding="utf-8").strip()) if status_path.exists() else 200
    else:
        status, payload = fetch_bytes(url)
        raw_path.write_bytes(payload)
        (raw_dir / f"{raw_path.name}.status").write_text(str(status) + "\n", encoding="utf-8")
        time.sleep(REQUEST_SLEEP_SECONDS)

    payload_json = parse_json_payload(payload)
    records = molecule_records(payload_json)
    record_ids = [record_source_id(record, f"CHEMBL-SEARCH:{pair['pair_key']}") for record in records]
    flattened = [clean_text(record) for record in records]
    contains_a = [exact_presence(text, pair["drug_a"]) for text in flattened]
    contains_b = [exact_presence(text, pair["drug_b"]) for text in flattened]
    both_indices = [idx for idx, (a, b) in enumerate(zip(contains_a, contains_b)) if a["present"] and b["present"]]
    return {
        "schema_version": 1,
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "query": pair["query"],
        "url": url,
        "http_status": status,
        "raw_response_path": str(raw_path),
        "raw_response_bytes": len(payload),
        "raw_response_sha256": sha256_bytes(payload),
        "total_count": total_count(payload_json),
        "records_returned": len(records),
        "record_ids": record_ids,
        "contains_drug_a_any_record": any(item["present"] for item in contains_a),
        "contains_drug_b_any_record": any(item["present"] for item in contains_b),
        "contains_both_in_same_record": bool(both_indices),
        "both_match_record_indices": both_indices,
    }


def mine_evidence(pair: dict[str, Any], query_row: dict[str, Any]) -> list[dict[str, Any]]:
    raw_path = Path(query_row["raw_response_path"])
    payload = raw_path.read_bytes()
    payload_json = parse_json_payload(payload)
    records = molecule_records(payload_json)
    out: list[dict[str, Any]] = []
    for idx, record in enumerate(records):
        text = clean_text(record)
        left = exact_presence(text, pair["drug_a"])
        right = exact_presence(text, pair["drug_b"])
        if not (left["present"] and right["present"]):
            continue
        source_id = record_source_id(record, f"CHEMBL-SEARCH:{pair['pair_key']}:{idx}")
        evidence_id = f"chembl-evidence:{stable_id(pair['pair_key'], source_id, idx)}"
        out.append(
            {
                "schema_version": 1,
                "chembl_evidence_id": evidence_id,
                "source_issue": 1254,
                "pair_key": pair["pair_key"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "source_id": source_id,
                "molecule_chembl_id": clean_text(record.get("molecule_chembl_id")),
                "pref_name": clean_text(record.get("pref_name")),
                "raw_response_path": str(raw_path),
                "raw_response_sha256": query_row["raw_response_sha256"],
                "source_record_index": idx,
                "source_record_sha256": sha256_bytes(json.dumps(record, sort_keys=True).encode("utf-8")),
                "source_text_sha256": sha256_bytes(text.encode("utf-8")),
                "source_text_sample": text[:2000],
                "match_drug_a": left,
                "match_drug_b": right,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "chembl_molecule_record_contains_both_pair_terms",
                    "chembl_source_mining_not_clinical_actionability",
                    "requires_safety_outcome_falsification_and_human_review",
                ],
            }
        )
    return out


def build_pair_status(pair: dict[str, Any], query_row: dict[str, Any], evidence_rows: list[dict[str, Any]]) -> dict[str, Any]:
    evidence_ids = [row["chembl_evidence_id"] for row in evidence_rows]
    if evidence_ids:
        status = "chembl_pair_record_two_term_hit_still_blocked"
        reason = "chembl_record_contains_both_pair_terms"
    elif int(query_row["http_status"]) != 200:
        status = "chembl_pair_query_failed_still_blocked"
        reason = "chembl_query_failed_or_non_200"
    elif not query_row.get("records_returned"):
        status = "chembl_pair_no_result_still_blocked"
        reason = "chembl_query_returned_no_records"
    else:
        status = "chembl_pair_record_without_pair_match_still_blocked"
        reason = "chembl_records_returned_without_same_record_pair_match"

    return {
        "schema_version": 1,
        "chembl_pair_status_id": f"chembl-pair-status:{stable_id(pair['pair_key'], status)}",
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "query": pair["query"],
        "representative_pair_id": pair.get("representative_pair_id"),
        "source_pair_ids": pair.get("source_pair_ids", []),
        "source_pubchem_candidate_status_ids": pair.get("source_pubchem_candidate_status_ids", []),
        "source_pubchem_pair_status_ids": pair.get("source_pubchem_pair_status_ids", []),
        "chembl_pair_status": status,
        "http_status": query_row["http_status"],
        "total_count": query_row.get("total_count"),
        "records_returned": query_row.get("records_returned"),
        "record_ids": query_row.get("record_ids", []),
        "chembl_evidence_rows": len(evidence_ids),
        "evidence_ids": evidence_ids,
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "chembl_source_mining_not_clinical_actionability",
            "requires_safety_outcome_falsification_and_human_review",
            reason,
        ],
    }


def build_candidate_status(candidate: dict[str, Any], pair_status: dict[str, Any]) -> dict[str, Any]:
    pair_to_candidate = {
        "chembl_pair_record_two_term_hit_still_blocked": "chembl_candidate_record_two_term_hit_still_blocked",
        "chembl_pair_record_without_pair_match_still_blocked": "chembl_candidate_record_without_pair_match_still_blocked",
        "chembl_pair_no_result_still_blocked": "chembl_candidate_no_result_still_blocked",
        "chembl_pair_query_failed_still_blocked": "chembl_candidate_query_failed_still_blocked",
    }
    status = pair_to_candidate[pair_status["chembl_pair_status"]]
    return {
        "schema_version": 1,
        "chembl_candidate_status_id": f"chembl-candidate-status:{stable_id(candidate['pair_id'], pair_status['chembl_pair_status'])}",
        "pair_id": candidate["pair_id"],
        "pair_key": candidate["pair_key"],
        "drug_a": candidate["drug_a"],
        "drug_b": candidate["drug_b"],
        "source_pubchem_candidate_status_id": candidate.get("pubchem_candidate_status_id"),
        "source_pubchem_pair_status_id": candidate.get("pubchem_pair_status_id"),
        "source_pubchem_candidate_status": candidate.get("pubchem_candidate_status"),
        "chembl_pair_status_id": pair_status["chembl_pair_status_id"],
        "chembl_candidate_status": status,
        "chembl_pair_status": pair_status["chembl_pair_status"],
        "chembl_evidence_rows": pair_status["chembl_evidence_rows"],
        "evidence_ids": pair_status["evidence_ids"],
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": pair_status["reason_codes"],
    }


def build_bridge_rows(
    candidate_status: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in candidate_status:
        text = (
            f"ChEMBL candidate status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['chembl_candidate_status']} "
            f"evidence rows {row['chembl_evidence_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["chembl_candidate_status_id"],
                "domain": "chembl_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["chembl_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1254_chembl_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "chembl_candidate_status": row["chembl_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        text = (
            f"ChEMBL pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['chembl_pair_status']} records returned {row['records_returned']} "
            f"evidence rows {row['chembl_evidence_rows']}."
        )
        rows.append(
            {
                "id": row["chembl_pair_status_id"],
                "domain": "chembl_pair_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["chembl_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1254_chembl_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "chembl_pair_status": row["chembl_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = max(0, 1000 - len(rows))
    for row in evidence_rows[:remaining]:
        text = (
            f"ChEMBL evidence {row['source_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} molecule {row.get('molecule_chembl_id', '')}."
        )
        rows.append(
            {
                "id": row["chembl_evidence_id"],
                "domain": "chembl_pair_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_id"]]),
                "metadata": {
                    "source_dataset": "issue1254_chembl_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "source_id": row["source_id"],
                    "promotion_status": row["promotion_status"],
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
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "chembl_pair_queries": len(query_rows),
        "query_http_status_counts": dict(sorted(Counter(str(row["http_status"]) for row in query_rows).items())),
        "chembl_queries_with_records": sum(1 for row in query_rows if row.get("records_returned")),
        "chembl_total_records_returned": sum(int(row.get("records_returned") or 0) for row in query_rows),
        "chembl_pair_evidence_rows": len(evidence_rows),
        "pair_status_rows": len(pair_status),
        "candidate_status_rows": len(candidate_status),
        "bridge_rows": len(bridge_rows),
        "bridge_evidence_rows_materialized": sum(1 for row in bridge_rows if row["domain"] == "chembl_pair_evidence"),
        "pair_status_counts": dict(sorted(Counter(row["chembl_pair_status"] for row in pair_status).items())),
        "candidate_status_counts": dict(sorted(Counter(row["chembl_candidate_status"] for row in candidate_status).items())),
        "all_rows_blocked": True,
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    raw_sources: dict[str, dict[str, Any]],
    issue1245_persisted_readback: dict[str, Any],
    issue1245_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1254,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1245_candidate_status": artifact(Path(inputs["issue1245_candidate_status"]), jsonl=True),
            "issue1245_pair_status": artifact(Path(inputs["issue1245_pair_status"]), jsonl=True),
            "issue1245_persisted_readback": artifact(Path(inputs["issue1245_persisted_readback"])),
            "issue1245_calyx_readback": artifact(Path(inputs["issue1245_calyx_readback"])),
            "issue1245_output_manifest": artifact(Path(inputs["issue1245_output_manifest"])),
        },
        "raw_source_docs": raw_sources,
        "source_contract": {
            "issue1245_persisted_assertions_all_true": all_assertions_true(issue1245_persisted_readback),
            "issue1245_calyx_assertions_all_true": all_assertions_true(issue1245_calyx_readback),
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "input_filter": "pubchem_candidate_status in blocked PubChem no-hit/no-pair-match statuses",
            "chembl_search_hit_requires_same_record_two_term_presence": True,
            "chembl_molecule_record_is_not_pair_interaction_or_clinical_actionability": True,
        },
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1245_persisted_readback: dict[str, Any],
    issue1245_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "chembl_molecule_query_responses": artifact(out_dir / "chembl_molecule_query_responses.jsonl", jsonl=True),
        "chembl_pair_evidence": artifact(out_dir / "chembl_pair_evidence.jsonl", jsonl=True),
        "chembl_pair_status": artifact(out_dir / "chembl_pair_status.jsonl", jsonl=True),
        "candidate_chembl_status": artifact(out_dir / "candidate_chembl_status.jsonl", jsonl=True),
        "chembl_bridge_rows": artifact(out_dir / "chembl_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    candidate_pair_keys = {row["pair_key"] for row in candidates}
    pair_status_keys = {row["pair_key"] for row in pair_status}
    query_keys = {row["pair_key"] for row in query_rows}
    status_candidate_ids = {row["pair_id"] for row in candidate_status}
    candidate_ids = {row["pair_id"] for row in candidates}
    evidence_by_pair = Counter(row["pair_key"] for row in evidence_rows)
    assertions = {
        "issue1245_persisted_readback_all_true": all_assertions_true(issue1245_persisted_readback),
        "issue1245_calyx_readback_all_true": all_assertions_true(issue1245_calyx_readback),
        "query_response_for_every_pair_key": query_keys == candidate_pair_keys,
        "pair_status_for_every_pair_key": pair_status_keys == candidate_pair_keys,
        "candidate_status_for_every_candidate": status_candidate_ids == candidate_ids,
        "all_hits_have_evidence": all(
            row["chembl_pair_status"] != "chembl_pair_record_two_term_hit_still_blocked"
            or evidence_by_pair[row["pair_key"]] > 0
            for row in pair_status
        ),
        "all_evidence_rows_have_source_hash": all(row.get("source_record_sha256") and row.get("source_text_sha256") for row in evidence_rows),
        "all_evidence_rows_have_pair_terms": all(
            row.get("match_drug_a", {}).get("present") and row.get("match_drug_b", {}).get("present")
            for row in evidence_rows
        ),
        "all_pair_status_values_allowed": all(row["chembl_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
        "all_candidate_status_values_allowed": all(
            row["chembl_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
        ),
        "all_status_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in pair_status + candidate_status),
        "all_evidence_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in evidence_rows),
        "all_rows_remain_blocked": all(
            row.get("promotion_status") == PROMOTION_STATUS for row in pair_status + candidate_status + evidence_rows
        ),
        "bridge_rows_1000_or_less": len(bridge_rows) <= 1000,
    }
    return {
        "schema_version": 1,
        "issue": 1254,
        "status": "ok" if all(assertions.values()) else "failed",
        "created_utc": now_utc(),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "row_counts": {
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence_rows),
            "pair_status_rows": len(pair_status),
            "candidate_status_rows": len(candidate_status),
            "bridge_rows": len(bridge_rows),
        },
        "artifacts": artifacts,
        "assertions": assertions,
    }


def run(root: Path, inputs: dict[str, str], max_pairs: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    raw_dir = root / "raw"
    out_dir = root / "out"
    raw_dir.mkdir(parents=True, exist_ok=True)
    out_dir.mkdir(parents=True, exist_ok=True)

    issue1245_persisted_readback = read_json(Path(inputs["issue1245_persisted_readback"]))
    issue1245_calyx_readback = read_json(Path(inputs["issue1245_calyx_readback"]))
    source_candidates = rows_jsonl(Path(inputs["issue1245_candidate_status"]))
    candidates = load_candidates(source_candidates, max_pairs=max_pairs)
    pairs = pair_rows(candidates)
    raw_sources = fetch_raw_sources(raw_dir)

    input_manifest = build_input_manifest(
        inputs,
        candidates,
        pairs,
        raw_sources,
        issue1245_persisted_readback,
        issue1245_calyx_readback,
    )
    write_json(out_dir / "input_manifest.json", input_manifest)

    query_rows: list[dict[str, Any]] = []
    evidence_rows: list[dict[str, Any]] = []
    pair_status_rows: list[dict[str, Any]] = []

    for idx, pair in enumerate(pairs, 1):
        query_row = fetch_pair_query(pair, raw_dir)
        query_rows.append(query_row)
        pair_evidence = mine_evidence(pair, query_row)
        evidence_rows.extend(pair_evidence)
        pair_status_rows.append(build_pair_status(pair, query_row, pair_evidence))
        print(
            f"#1254 ChEMBL pair query {idx}/{len(pairs)} pair={pair['pair_key']} "
            f"status={query_row['http_status']} records={query_row['records_returned']} "
            f"evidence={len(pair_evidence)}",
            file=sys.stderr,
        )

    pair_status_by_key = {row["pair_key"]: row for row in pair_status_rows}
    candidate_status_rows = [build_candidate_status(row, pair_status_by_key[row["pair_key"]]) for row in candidates]

    write_jsonl(out_dir / "chembl_molecule_query_responses.jsonl", query_rows)
    write_jsonl(out_dir / "chembl_pair_evidence.jsonl", evidence_rows)
    write_jsonl(out_dir / "chembl_pair_status.jsonl", pair_status_rows)
    write_jsonl(out_dir / "candidate_chembl_status.jsonl", candidate_status_rows)

    bridge_rows = build_bridge_rows(
        candidate_status_rows,
        pair_status_rows,
        evidence_rows,
        out_dir / "candidate_chembl_status.jsonl",
        sha256_path(out_dir / "candidate_chembl_status.jsonl"),
    )
    write_jsonl(out_dir / "chembl_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(candidates, pairs, query_rows, evidence_rows, pair_status_rows, candidate_status_rows, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1254,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "chembl_molecule_query_responses": artifact(out_dir / "chembl_molecule_query_responses.jsonl", jsonl=True),
            "chembl_pair_evidence": artifact(out_dir / "chembl_pair_evidence.jsonl", jsonl=True),
            "chembl_pair_status": artifact(out_dir / "chembl_pair_status.jsonl", jsonl=True),
            "candidate_chembl_status": artifact(out_dir / "candidate_chembl_status.jsonl", jsonl=True),
            "chembl_bridge_rows": artifact(out_dir / "chembl_bridge_rows.jsonl", jsonl=True),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)

    persisted_readback = build_readback(
        out_dir,
        candidates,
        pairs,
        query_rows,
        evidence_rows,
        pair_status_rows,
        candidate_status_rows,
        bridge_rows,
        issue1245_persisted_readback,
        issue1245_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", persisted_readback)

    final = {
        "status": persisted_readback["status"],
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "query_responses": artifact(out_dir / "chembl_molecule_query_responses.jsonl", jsonl=True),
            "pair_evidence": artifact(out_dir / "chembl_pair_evidence.jsonl", jsonl=True),
            "pair_status": artifact(out_dir / "chembl_pair_status.jsonl", jsonl=True),
            "candidate_status": artifact(out_dir / "candidate_chembl_status.jsonl", jsonl=True),
            "bridge_rows": artifact(out_dir / "chembl_bridge_rows.jsonl", jsonl=True),
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }
    print(json.dumps(final, indent=2, sort_keys=True))
    if persisted_readback["status"] != "ok":
        raise SystemExit(1)
    return final


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--max-pairs", type=int, default=None, help="limit pair keys for smoke tests")
    for key, value in DEFAULT_INPUTS.items():
        parser.add_argument(f"--{key.replace('_', '-')}", default=value)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = {key: getattr(args, key) for key in DEFAULT_INPUTS}
    run(Path(args.root), inputs, max_pairs=args.max_pairs)


if __name__ == "__main__":
    main()
