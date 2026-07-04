#!/usr/bin/env python3
"""#1245 PubChem synonym/equivalence source mining after Europe PMC no-hit.

This stage reads sealed #1243 remaining no-hit candidate rows and queries a
distinct external source instrument: PubChem PUG-REST synonym records. A hit
requires PubChem returned structured synonym text to physically contain both
candidate terms or accepted normalized equivalents. Every row remains blocked.
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
    "PubChem synonym/equivalence source mining is chemical identity/source "
    "triage only; not efficacy, safety, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, pair-interaction evidence, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "pubchem_synonym_equivalence_not_pair_interaction_safety_efficacy_or_cure"
)

ISSUE1243_ROOT = "/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z"

DEFAULT_INPUTS = {
    "issue1243_candidate_status": f"{ISSUE1243_ROOT}/out/candidate_europepmc_status.jsonl",
    "issue1243_pair_status": f"{ISSUE1243_ROOT}/out/europepmc_pair_status.jsonl",
    "issue1243_persisted_readback": f"{ISSUE1243_ROOT}/out/persisted_readback.json",
    "issue1243_calyx_readback": f"{ISSUE1243_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1243_output_manifest": f"{ISSUE1243_ROOT}/out/output_manifest.json",
}

PUBCHEM_PUG_REST_DOC_URL = "https://pubchem.ncbi.nlm.nih.gov/docs/pug-rest"
PUBCHEM_PUG_VIEW_DOC_URL = "https://pubchem.ncbi.nlm.nih.gov/docs/pug-view"
PUBCHEM_PROGRAMMATIC_ACCESS_URL = "https://pubchem.ncbi.nlm.nih.gov/docs/programmatic-access"
PUBCHEM_SYNONYM_ENDPOINT = "https://pubchem.ncbi.nlm.nih.gov/rest/pug/compound/name/{name}/synonyms/JSON"

REQUEST_SLEEP_SECONDS = 0.20
USER_AGENT = "calyx-discovery/issue1245"
PROMOTION_STATUS = "blocked_requires_external_source_safety_outcome_falsification_and_human_review"

PAIR_STATUS_VALUES = {
    "pubchem_synonym_equivalence_hit_still_blocked",
    "pubchem_synonym_record_without_pair_match_still_blocked",
    "pubchem_synonym_no_result_still_blocked",
    "pubchem_synonym_not_queryable_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "pubchem_synonym_equivalence_hit_still_blocked",
    "pubchem_synonym_record_without_pair_match_still_blocked",
    "pubchem_synonym_no_result_still_blocked",
    "pubchem_synonym_not_queryable_still_blocked",
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
    if isinstance(value, dict):
        value = " ".join(f"{key} {clean_text(val)}" for key, val in value.items())
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
        "pubchem_pug_rest_docs": (PUBCHEM_PUG_REST_DOC_URL, "pubchem_pug_rest_docs.html"),
        "pubchem_pug_view_docs": (PUBCHEM_PUG_VIEW_DOC_URL, "pubchem_pug_view_docs.html"),
        "pubchem_programmatic_access": (PUBCHEM_PROGRAMMATIC_ACCESS_URL, "pubchem_programmatic_access.html"),
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


def load_no_hit_candidates(rows: list[dict[str, Any]], max_candidates: int | None = None) -> list[dict[str, Any]]:
    out = [
        row
        for row in rows
        if row.get("overall_external_source_status_after_issue1243") == "no_external_hit"
        or row.get("europepmc_status") == "no_external_hit"
    ]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_candidates is not None:
        out = out[:max_candidates]
    return out


def pair_rows(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        grouped[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        out.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query_drug_a": query_name(first["drug_a"]),
                "query_drug_b": query_name(first["drug_b"]),
                "representative_pair_id": first["pair_id"],
                "source_candidate_status_ids": [row["status_id"] for row in members],
                "source_pair_status_ids": uniq([row.get("pair_status_id") for row in members]),
                "source_pair_ids": [row["pair_id"] for row in members],
                "queryable": bool(query_name(first["drug_a"]) and query_name(first["drug_b"])),
            }
        )
    return out


def unique_terms(pairs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_norm: dict[str, dict[str, Any]] = {}
    for pair in pairs:
        for field in ["drug_a", "drug_b"]:
            raw = pair[field]
            query = query_name(raw)
            norm = norm_name(raw)
            if not norm:
                continue
            by_norm.setdefault(
                norm,
                {
                    "schema_version": 1,
                    "term": raw,
                    "query_name": query,
                    "normalized_name": norm,
                    "pair_keys": [],
                },
            )
            by_norm[norm]["pair_keys"].append(pair["pair_key"])
    return sorted(by_norm.values(), key=lambda row: row["normalized_name"])


def pubchem_synonym_url(term: str) -> str:
    return PUBCHEM_SYNONYM_ENDPOINT.format(name=urllib.parse.quote(term, safe=""))


def query_pubchem_synonyms(terms: list[dict[str, Any]], raw_dir: Path) -> list[dict[str, Any]]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    for index, term in enumerate(terms, start=1):
        url = pubchem_synonym_url(term["query_name"])
        status, payload = fetch_bytes(url)
        raw_path = raw_dir / f"pubchem_synonyms_{stable_id(term['normalized_name'], term['query_name'])}.json"
        raw_path.write_bytes(payload)
        try:
            response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
        except json.JSONDecodeError:
            response_json = {"decode_error": payload.decode("utf-8", errors="replace")[:1000]}
        cid_values: list[int] = []
        synonyms: list[str] = []
        if isinstance(response_json, dict):
            info_rows = (response_json.get("InformationList") or {}).get("Information") or []
            for info in info_rows if isinstance(info_rows, list) else []:
                if not isinstance(info, dict):
                    continue
                cid = info.get("CID")
                if isinstance(cid, int):
                    cid_values.append(cid)
                synonyms.extend(clean_text(value) for value in info.get("Synonym", []) if clean_text(value))
        row = {
            "schema_version": 1,
            "term": term["term"],
            "query_name": term["query_name"],
            "normalized_name": term["normalized_name"],
            "query_url": url,
            "http_status": status,
            "raw_response_path": str(raw_path),
            "raw_response_bytes": len(payload),
            "raw_response_sha256": sha256_bytes(payload),
            "pubchem_cids": sorted(set(cid_values)),
            "synonym_count": len(uniq(synonyms)),
            "synonyms": uniq(synonyms)[:500],
            "pair_keys": sorted(set(term["pair_keys"])),
            "evidence_kind": SOURCE_EVIDENCE_KIND,
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        rows.append(row)
        print(
            f"#1245 PubChem synonym query {index}/{len(terms)} term={term['query_name']} "
            f"status={status} cids={len(row['pubchem_cids'])} synonyms={row['synonym_count']}",
            file=sys.stderr,
        )
        time.sleep(REQUEST_SLEEP_SECONDS)
    return rows


def build_term_lookup(query_rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {row["normalized_name"]: row for row in query_rows}


def evidence_from_source(source: dict[str, Any], pair: dict[str, Any], side: str) -> dict[str, Any] | None:
    synonyms_text = " ".join(source.get("synonyms", []))
    left = exact_presence(synonyms_text, pair["drug_a"])
    right = exact_presence(synonyms_text, pair["drug_b"])
    if not (left["present"] and right["present"]):
        return None
    source_id = f"CID:{source['pubchem_cids'][0]}" if source.get("pubchem_cids") else f"PUBCHEM-NAME:{source['normalized_name']}"
    structured = {
        "source_id": source_id,
        "query_term": source["term"],
        "pubchem_cids": source.get("pubchem_cids", []),
        "synonyms": source.get("synonyms", [])[:80],
    }
    structured_text = clean_text(structured)
    return {
        "schema_version": 1,
        "pubchem_evidence_id": "pubchem-synonym-evidence:" + stable_id(pair["pair_key"], side, source_id),
        "pair_key": pair["pair_key"],
        "representative_pair_id": pair["representative_pair_id"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "source_side": side,
        "source": "PubChem PUG-REST synonym record",
        "source_id": source_id,
        "source_url": source["query_url"],
        "source_structured_field": "InformationList.Information.Synonym",
        "source_structured_text": structured_text,
        "source_structured_sha256": hashlib.sha256(structured_text.encode("utf-8")).hexdigest(),
        "raw_response_path": source["raw_response_path"],
        "raw_response_sha256": source["raw_response_sha256"],
        "pubchem_cids": source.get("pubchem_cids", []),
        "pair_term_presence": {
            "left": left,
            "right": right,
            "both_present_in_pubchem_synonyms": left["present"] and right["present"],
            "both_exact_in_pubchem_synonyms": left["exact"] and right["exact"],
        },
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "pubchem_synonym_equivalence_not_pair_interaction_evidence",
            "requires_safety_outcome_falsification_and_human_review",
        ],
    }


def build_evidence_rows(pairs: list[dict[str, Any]], term_lookup: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for pair in pairs:
        for side, term in [("left", pair["drug_a"]), ("right", pair["drug_b"])]:
            source = term_lookup.get(norm_name(term))
            if not source:
                continue
            evidence = evidence_from_source(source, pair, side)
            if not evidence:
                continue
            key = (evidence["pair_key"], evidence["source_id"])
            if key in seen:
                continue
            seen.add(key)
            rows.append(evidence)
    rows.sort(key=lambda row: (row["pair_key"], row["source_id"]))
    return rows


def pair_status_value(pair: dict[str, Any], term_lookup: dict[str, dict[str, Any]], evidence: list[dict[str, Any]]) -> str:
    if not pair["queryable"]:
        return "pubchem_synonym_not_queryable_still_blocked"
    if evidence:
        return "pubchem_synonym_equivalence_hit_still_blocked"
    left = term_lookup.get(norm_name(pair["drug_a"]))
    right = term_lookup.get(norm_name(pair["drug_b"]))
    if (left and left.get("pubchem_cids")) or (right and right.get("pubchem_cids")):
        return "pubchem_synonym_record_without_pair_match_still_blocked"
    return "pubchem_synonym_no_result_still_blocked"


def build_pair_status(
    pairs: list[dict[str, Any]], query_rows: list[dict[str, Any]], evidence_rows: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    term_lookup = build_term_lookup(query_rows)
    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_pair[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for pair in pairs:
        evidence = evidence_by_pair.get(pair["pair_key"], [])
        left = term_lookup.get(norm_name(pair["drug_a"]))
        right = term_lookup.get(norm_name(pair["drug_b"]))
        status = pair_status_value(pair, term_lookup, evidence)
        rows.append(
            {
                "schema_version": 1,
                "pubchem_pair_status_id": "pubchem-synonym-pair-status:" + stable_id(pair["pair_key"]),
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "pubchem_pair_status": status,
                "left_pubchem_cids": left.get("pubchem_cids", []) if left else [],
                "right_pubchem_cids": right.get("pubchem_cids", []) if right else [],
                "left_http_status": left.get("http_status") if left else None,
                "right_http_status": right.get("http_status") if right else None,
                "evidence_ids": [row["pubchem_evidence_id"] for row in evidence],
                "pubchem_evidence_rows": len(evidence),
                "source_pair_ids": pair["source_pair_ids"],
                "source_pair_status_ids": pair["source_pair_status_ids"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def candidate_reason_codes(status: str) -> list[str]:
    codes = [
        "pubchem_synonym_source_mining_not_clinical_actionability",
        "requires_safety_outcome_falsification_and_human_review",
    ]
    if status == "pubchem_synonym_equivalence_hit_still_blocked":
        codes.append("pubchem_synonym_equivalence_requires_review")
    elif status == "pubchem_synonym_record_without_pair_match_still_blocked":
        codes.append("pubchem_record_exists_without_pair_synonym_match")
    elif status == "pubchem_synonym_not_queryable_still_blocked":
        codes.append("pubchem_pair_not_queryable")
    else:
        codes.append("pubchem_synonym_no_result")
    return codes


def build_candidate_status(candidates: list[dict[str, Any]], pair_status: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_pair = {row["pair_key"]: row for row in pair_status}
    rows: list[dict[str, Any]] = []
    for candidate in candidates:
        pair = by_pair[candidate["pair_key"]]
        status = pair["pubchem_pair_status"]
        rows.append(
            {
                "schema_version": 1,
                "pubchem_candidate_status_id": "pubchem-synonym-candidate-status:"
                + stable_id(candidate["status_id"], candidate["pair_key"]),
                "source_issue1243_status_id": candidate["status_id"],
                "source_issue1243_pair_status_id": candidate["pair_status_id"],
                "pair_id": candidate["pair_id"],
                "pair_key": candidate["pair_key"],
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "source_issue1243_status": candidate.get("overall_external_source_status_after_issue1243"),
                "pubchem_candidate_status": status,
                "pubchem_pair_status_id": pair["pubchem_pair_status_id"],
                "evidence_ids": pair["evidence_ids"],
                "pubchem_evidence_rows": pair["pubchem_evidence_rows"],
                "left_pubchem_cids": pair["left_pubchem_cids"],
                "right_pubchem_cids": pair["right_pubchem_cids"],
                "reason_codes": candidate_reason_codes(status),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


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
            f"PubChem synonym candidate status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['pubchem_candidate_status']} "
            f"evidence rows {row['pubchem_evidence_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["pubchem_candidate_status_id"],
                "domain": "pubchem_synonym_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["pubchem_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1245_pubchem_synonym_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "pubchem_candidate_status": row["pubchem_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        text = (
            f"PubChem synonym pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['pubchem_pair_status']} evidence rows {row['pubchem_evidence_rows']}."
        )
        rows.append(
            {
                "id": row["pubchem_pair_status_id"],
                "domain": "pubchem_synonym_pair_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["pubchem_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1245_pubchem_synonym_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "pubchem_pair_status": row["pubchem_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in evidence_rows:
        text = (
            f"PubChem synonym evidence {row['source_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} source side {row['source_side']}."
        )
        rows.append(
            {
                "id": row["pubchem_evidence_id"],
                "domain": "pubchem_synonym_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_id"]]),
                "metadata": {
                    "source_dataset": "issue1245_pubchem_synonym_source_mining",
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
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "unique_query_terms": len(query_rows),
        "query_http_status_counts": dict(sorted(Counter(str(row["http_status"]) for row in query_rows).items())),
        "pubchem_terms_with_cid": sum(1 for row in query_rows if row.get("pubchem_cids")),
        "pubchem_evidence_rows": len(evidence_rows),
        "pair_status_rows": len(pair_status),
        "candidate_status_rows": len(candidate_status),
        "pair_status_counts": dict(sorted(Counter(row["pubchem_pair_status"] for row in pair_status).items())),
        "candidate_status_counts": dict(sorted(Counter(row["pubchem_candidate_status"] for row in candidate_status).items())),
        "all_rows_blocked": True,
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    terms: list[dict[str, Any]],
    raw_sources: dict[str, dict[str, Any]],
    issue1243_persisted_readback: dict[str, Any],
    issue1243_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1245,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1243_candidate_status": artifact(Path(inputs["issue1243_candidate_status"]), jsonl=True),
            "issue1243_pair_status": artifact(Path(inputs["issue1243_pair_status"]), jsonl=True),
            "issue1243_persisted_readback": artifact(Path(inputs["issue1243_persisted_readback"])),
            "issue1243_calyx_readback": artifact(Path(inputs["issue1243_calyx_readback"])),
            "issue1243_output_manifest": artifact(Path(inputs["issue1243_output_manifest"])),
        },
        "raw_source_docs": raw_sources,
        "source_contract": {
            "issue1243_persisted_assertions_all_true": all_assertions_true(issue1243_persisted_readback),
            "issue1243_calyx_assertions_all_true": all_assertions_true(issue1243_calyx_readback),
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "unique_query_terms": len(terms),
            "input_filter": "overall_external_source_status_after_issue1243 == no_external_hit or europepmc_status == no_external_hit",
            "pubchem_synonym_equivalence_is_not_pair_interaction_or_clinical_actionability": True,
        },
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    terms: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    issue1243_persisted_readback: dict[str, Any],
    issue1243_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "pubchem_synonym_query_responses": artifact(out_dir / "pubchem_synonym_query_responses.jsonl", jsonl=True),
        "pubchem_pair_evidence": artifact(out_dir / "pubchem_pair_evidence.jsonl", jsonl=True),
        "pubchem_pair_status": artifact(out_dir / "pubchem_pair_status.jsonl", jsonl=True),
        "candidate_pubchem_status": artifact(out_dir / "candidate_pubchem_status.jsonl", jsonl=True),
        "pubchem_bridge_rows": artifact(out_dir / "pubchem_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    expected_query_terms = {row["normalized_name"] for row in terms}
    observed_query_terms = {row["normalized_name"] for row in query_rows}
    expected_pair_keys = {row["pair_key"] for row in pairs}
    pair_status_keys = {row["pair_key"] for row in pair_status}
    expected_candidate_ids = {row["status_id"] for row in candidates}
    candidate_status_ids = {row["source_issue1243_status_id"] for row in candidate_status}
    hit_pair_keys = {
        row["pair_key"]
        for row in pair_status
        if row["pubchem_pair_status"] == "pubchem_synonym_equivalence_hit_still_blocked"
    }
    evidence_pair_keys = {row["pair_key"] for row in evidence_rows}
    return {
        "schema_version": 1,
        "issue": 1245,
        "status": "ok",
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1243_persisted_readback_all_true": all_assertions_true(issue1243_persisted_readback),
            "issue1243_calyx_readback_all_true": all_assertions_true(issue1243_calyx_readback),
            "query_response_for_every_unique_term": observed_query_terms == expected_query_terms,
            "pair_status_for_every_pair_key": pair_status_keys == expected_pair_keys,
            "candidate_status_for_every_no_hit_candidate": candidate_status_ids == expected_candidate_ids,
            "all_hits_have_evidence": hit_pair_keys <= evidence_pair_keys,
            "all_evidence_rows_have_source_hash": all(
                row["source_id"] and row["source_structured_sha256"] and row["raw_response_sha256"] for row in evidence_rows
            ),
            "all_evidence_rows_have_pair_terms": all(
                row["pair_term_presence"]["both_present_in_pubchem_synonyms"] for row in evidence_rows
            ),
            "all_pair_status_values_allowed": all(row["pubchem_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
            "all_candidate_status_values_allowed": all(
                row["pubchem_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
            ),
            "all_status_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in pair_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in candidate_status),
            "all_evidence_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence_rows),
            "all_rows_remain_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in pair_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in candidate_status)
            and all(row["promotion_status"] == PROMOTION_STATUS for row in evidence_rows),
            "bridge_rows_1000_or_less": artifacts["pubchem_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "unique_query_terms": len(terms),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence_rows),
            "pair_status_rows": len(pair_status),
            "candidate_status_rows": len(candidate_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_candidates: int | None = None, max_pairs: int | None = None) -> dict[str, Any]:
    require_inputs(inputs)
    out_dir = root / "out"
    raw_dir = root / "raw"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    all_candidates = rows_jsonl(Path(inputs["issue1243_candidate_status"]))
    issue1243_persisted_readback = read_json(Path(inputs["issue1243_persisted_readback"]))
    issue1243_calyx_readback = read_json(Path(inputs["issue1243_calyx_readback"]))
    candidates = load_no_hit_candidates(all_candidates, max_candidates=max_candidates)
    pairs = pair_rows(candidates)
    if max_pairs is not None:
        selected = {row["pair_key"] for row in pairs[:max_pairs]}
        pairs = [row for row in pairs if row["pair_key"] in selected]
        candidates = [row for row in candidates if row["pair_key"] in selected]
    terms = unique_terms(pairs)
    raw_sources = fetch_raw_sources(raw_dir)
    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            candidates,
            pairs,
            terms,
            raw_sources,
            issue1243_persisted_readback,
            issue1243_calyx_readback,
        ),
    )
    query_rows = query_pubchem_synonyms(terms, raw_dir)
    write_jsonl(out_dir / "pubchem_synonym_query_responses.jsonl", query_rows)
    evidence_rows = build_evidence_rows(pairs, build_term_lookup(query_rows))
    write_jsonl(out_dir / "pubchem_pair_evidence.jsonl", evidence_rows)
    pair_status = build_pair_status(pairs, query_rows, evidence_rows)
    write_jsonl(out_dir / "pubchem_pair_status.jsonl", pair_status)
    candidate_status = build_candidate_status(candidates, pair_status)
    write_jsonl(out_dir / "candidate_pubchem_status.jsonl", candidate_status)
    source_path = out_dir / "candidate_pubchem_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(candidate_status, pair_status, evidence_rows, source_path, source_sha)
    write_jsonl(out_dir / "pubchem_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(candidates, pairs, query_rows, evidence_rows, pair_status, candidate_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1245,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "pubchem_synonym_query_responses": artifact(out_dir / "pubchem_synonym_query_responses.jsonl", jsonl=True),
            "pubchem_pair_evidence": artifact(out_dir / "pubchem_pair_evidence.jsonl", jsonl=True),
            "pubchem_pair_status": artifact(out_dir / "pubchem_pair_status.jsonl", jsonl=True),
            "candidate_pubchem_status": artifact(out_dir / "candidate_pubchem_status.jsonl", jsonl=True),
            "pubchem_bridge_rows": artifact(out_dir / "pubchem_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        candidates,
        pairs,
        terms,
        query_rows,
        evidence_rows,
        pair_status,
        candidate_status,
        issue1243_persisted_readback,
        issue1243_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    if not all(readback["assertions"].values()):
        raise AssertionError(f"Persisted readback assertions failed: {readback['assertions']}")
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "query_responses": output_manifest["artifacts"]["pubchem_synonym_query_responses"],
            "pair_evidence": output_manifest["artifacts"]["pubchem_pair_evidence"],
            "pair_status": output_manifest["artifacts"]["pubchem_pair_status"],
            "candidate_status": output_manifest["artifacts"]["candidate_pubchem_status"],
            "bridge_rows": output_manifest["artifacts"]["pubchem_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1243-candidate-status")
    parser.add_argument("--issue1243-pair-status")
    parser.add_argument("--issue1243-persisted-readback")
    parser.add_argument("--issue1243-calyx-readback")
    parser.add_argument("--issue1243-output-manifest")
    parser.add_argument("--max-candidates", type=int)
    parser.add_argument("--max-pairs", type=int)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1243_candidate_status", "issue1243_candidate_status"),
        ("issue1243_pair_status", "issue1243_pair_status"),
        ("issue1243_persisted_readback", "issue1243_persisted_readback"),
        ("issue1243_calyx_readback", "issue1243_calyx_readback"),
        ("issue1243_output_manifest", "issue1243_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_candidates=args.max_candidates, max_pairs=args.max_pairs)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
