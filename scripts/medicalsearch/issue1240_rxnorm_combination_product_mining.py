#!/usr/bin/env python3
"""#1240 RxNorm combination-product source mining for remaining no-hit rows.

This stage reads the sealed #1236 openFDA status rows, filters to candidates
still lacking external support, and queries current NLM RxNorm product concepts
for deterministic component co-occurrence. The output is source-attributed
research triage only: not efficacy, safety, interaction clearance, dosing,
treatment guidance, clinical actionability, recommendation, or cure evidence.
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
    "RxNorm combination-product concept evidence is source-attributed "
    "vocabulary/product-concept mining only; not efficacy, safety, pair "
    "interaction clearance, treatment guidance, dosing guidance, recommendation, "
    "clinical actionability, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "rxnorm_combination_product_concept_comention_not_safety_efficacy_or_interaction_clearance"
)

ISSUE1236_ROOT = "/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z"

DEFAULT_INPUTS = {
    "issue1236_candidate_status": f"{ISSUE1236_ROOT}/out/candidate_openfda_label_status.jsonl",
    "issue1236_persisted_readback": f"{ISSUE1236_ROOT}/out/persisted_readback.json",
    "issue1236_calyx_readback": f"{ISSUE1236_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1236_output_manifest": f"{ISSUE1236_ROOT}/out/output_manifest.json",
}

RXNORM_DRUGS_ENDPOINT = "https://rxnav.nlm.nih.gov/REST/drugs.json"
RXNORM_VERSION_URL = "https://rxnav.nlm.nih.gov/REST/version.json"
RXNORM_API_DOCS_URL = "https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html"
RXNAV_TERMS_URL = "https://lhncbc.nlm.nih.gov/RxNav/TermsofService.html"
RXNAV_FAQ_URL = "https://lhncbc.nlm.nih.gov/RxNav/information/FAQs.html"
RXNAV_OVERVIEW_URL = "https://lhncbc.nlm.nih.gov/RxNav/"

REQUEST_SLEEP_SECONDS = 0.07
USER_AGENT = "calyx-discovery/issue1240"
STATUS_VALUES = {"exact_hit", "normalized_hit", "no_external_hit"}


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
    text = clean_text(value).lower()
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
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
                    "message": "required #1236 source artifacts are missing",
                    "missing": missing,
                    "remediation": "finish #1236 and persist its source/readback artifacts before running #1240",
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
            if error.code in {400, 404}:
                return int(error.code), payload
            last_error = error
        except (urllib.error.URLError, TimeoutError) as error:
            last_error = error
        time.sleep(min(12.0, 1.5 * (attempt + 1)))
    raise RuntimeError(f"fetch failed after {retries} attempts for {url}: {last_error}")


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    sources = {
        "rxnorm_api_docs": RXNORM_API_DOCS_URL,
        "rxnav_terms": RXNAV_TERMS_URL,
        "rxnav_faq": RXNAV_FAQ_URL,
        "rxnav_overview": RXNAV_OVERVIEW_URL,
        "rxnorm_version": RXNORM_VERSION_URL,
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
        artifacts[name] = {**artifact(path), "source_page_url": url}
    return artifacts


def all_assertions_true(readback: dict[str, Any]) -> bool:
    assertions = readback.get("assertions") or {}
    return bool(assertions) and all(value is True for value in assertions.values())


def remaining_no_hit_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = [
        row
        for row in rows
        if row.get("overall_external_source_status_after_issue1236") == "no_external_hit"
        or row.get("openfda_label_status") == "no_external_hit"
    ]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    return out


def representative_pairs(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        by_key[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for key, items in by_key.items():
        first = sorted(items, key=lambda row: row["pair_id"])[0]
        left = query_name(first["drug_a"])
        right = query_name(first["drug_b"])
        out.append(
            {
                "pair_key": key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "query_drug_a": left,
                "query_drug_b": right,
                "representative_pair_id": first["pair_id"],
                "candidate_row_count": len(items),
                "candidate_pair_ids": [row["pair_id"] for row in sorted(items, key=lambda row: row["pair_id"])],
                "queryable": bool(norm_name(left) and norm_name(right)),
            }
        )
    out.sort(key=lambda row: (row["pair_key"], row["representative_pair_id"]))
    return out


def rxnorm_drugs_url(query: str) -> str:
    return f"{RXNORM_DRUGS_ENDPOINT}?{urllib.parse.urlencode({'name': query})}"


def cached_query_rows(path: Path, expected_keys: set[str]) -> list[dict[str, Any]] | None:
    if not path.exists():
        return None
    rows = rows_jsonl(path)
    if {row.get("pair_key") for row in rows} == expected_keys:
        return rows
    return None


def fetch_rxnorm_queries(pair_rows: list[dict[str, Any]], out_path: Path, request_sleep_seconds: float) -> list[dict[str, Any]]:
    expected_keys = {row["pair_key"] for row in pair_rows if row["queryable"]}
    cached = cached_query_rows(out_path, expected_keys)
    if cached is not None:
        return cached
    tmp_path = out_path.with_suffix(out_path.suffix + ".tmp")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    queryable_rows = [row for row in pair_rows if row["queryable"]]
    with tmp_path.open("w", encoding="utf-8") as handle:
        for index, pair in enumerate(queryable_rows, start=1):
            directions = [
                {
                    "direction": "drug_a_slash_drug_b",
                    "query": f"{pair['query_drug_a']} / {pair['query_drug_b']}",
                },
                {
                    "direction": "drug_b_slash_drug_a",
                    "query": f"{pair['query_drug_b']} / {pair['query_drug_a']}",
                },
            ]
            responses = []
            for direction in directions:
                url = rxnorm_drugs_url(direction["query"])
                status, payload = fetch_bytes(url)
                try:
                    response_json = json.loads(payload.decode("utf-8", errors="replace")) if payload else {}
                except json.JSONDecodeError:
                    response_json = {"raw_decode_error": payload.decode("utf-8", errors="replace")[:1000]}
                concepts = rxnorm_concepts(response_json)
                responses.append(
                    {
                        "direction": direction["direction"],
                        "query": direction["query"],
                        "query_url": url,
                        "http_status": status,
                        "response_bytes": len(payload),
                        "response_sha256": sha256_bytes(payload),
                        "concept_count": len(concepts),
                        "response_json": response_json,
                    }
                )
                time.sleep(request_sleep_seconds)
            row = {
                "schema_version": 1,
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "api_endpoint": RXNORM_DRUGS_ENDPOINT,
                "query_responses": responses,
                "total_concept_count": sum(item["concept_count"] for item in responses),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
            rows.append(row)
            handle.write(json.dumps(row, sort_keys=True) + "\n")
            handle.flush()
            print(
                f"RxNorm combination query {index}/{len(queryable_rows)} concepts={row['total_concept_count']}",
                file=sys.stderr,
            )
    os.replace(tmp_path, out_path)
    return rows


def rxnorm_concepts(response_json: dict[str, Any]) -> list[dict[str, Any]]:
    concepts: list[dict[str, Any]] = []
    groups = response_json.get("drugGroup", {}).get("conceptGroup") or []
    for group in groups:
        tty = clean_text(group.get("tty"))
        for concept in group.get("conceptProperties") or []:
            item = dict(concept)
            item["group_tty"] = tty
            concepts.append(item)
    return concepts


def concept_match_text(concept: dict[str, Any]) -> str:
    return clean_text(" ".join([concept.get("name") or "", concept.get("synonym") or ""]))


def match_concept(concept: dict[str, Any], left: str, right: str) -> dict[str, Any] | None:
    text = concept_match_text(concept)
    left_presence = exact_presence(text, left)
    right_presence = exact_presence(text, right)
    if not (left_presence["present"] and right_presence["present"]):
        return None
    match_kind = "exact_hit" if left_presence["exact"] and right_presence["exact"] else "normalized_hit"
    return {
        "match_kind": match_kind,
        "left_presence": left_presence,
        "right_presence": right_presence,
        "concept_text": text,
        "concept_text_sha256": sha256_bytes(text.encode("utf-8")),
    }


def build_evidence_rows(query_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for row in query_rows:
        for response in row["query_responses"]:
            for concept in rxnorm_concepts(response["response_json"]):
                match = match_concept(concept, row["query_drug_a"], row["query_drug_b"])
                if not match:
                    continue
                rxcui = clean_text(concept.get("rxcui"))
                key = (row["pair_key"], rxcui)
                if key in seen:
                    continue
                seen.add(key)
                evidence.append(
                    {
                        "schema_version": 1,
                        "evidence_id": f"rxnorm-combo-evidence:{stable_id(row['pair_key'], rxcui)}",
                        "pair_key": row["pair_key"],
                        "representative_pair_id": row["representative_pair_id"],
                        "drug_a": row["drug_a"],
                        "drug_b": row["drug_b"],
                        "query_drug_a": row["query_drug_a"],
                        "query_drug_b": row["query_drug_b"],
                        "rxcui": rxcui,
                        "rxnorm_name": clean_text(concept.get("name")),
                        "rxnorm_synonym": clean_text(concept.get("synonym")),
                        "tty": clean_text(concept.get("tty") or concept.get("group_tty")),
                        "language": clean_text(concept.get("language")),
                        "suppress": clean_text(concept.get("suppress")),
                        "umlscui": clean_text(concept.get("umlscui")),
                        "source_url": f"https://rxnav.nlm.nih.gov/REST/rxcui/{urllib.parse.quote(rxcui)}/properties.json",
                        "query_url": response["query_url"],
                        "query_direction": response["direction"],
                        "query_response_sha256": response["response_sha256"],
                        "match_kind": match["match_kind"],
                        "left_presence": match["left_presence"],
                        "right_presence": match["right_presence"],
                        "concept_text_sha256": match["concept_text_sha256"],
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "clinical_boundary": CLINICAL_BOUNDARY,
                        "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                        "reason_codes": reason_codes_for_evidence(match["match_kind"]),
                    }
                )
    evidence.sort(key=lambda item: (item["pair_key"], item["rxcui"]))
    return evidence


def reason_codes_for_evidence(match_kind: str) -> list[str]:
    codes = [
        "rxnorm_combination_product_concept_not_clinical_clearance",
        "requires_safety_outcome_falsification_and_human_review_gates",
    ]
    if match_kind == "exact_hit":
        codes.append("both_candidate_names_exact_in_rxnorm_concept")
    else:
        codes.append("both_candidate_names_normalized_in_rxnorm_concept")
    return codes


def status_rank(status: str) -> int:
    return {"exact_hit": 2, "normalized_hit": 1, "no_external_hit": 0}.get(status, 0)


def pair_status_rows(pair_rows: list[dict[str, Any]], query_rows: list[dict[str, Any]], evidence: list[dict[str, Any]]) -> list[dict[str, Any]]:
    query_by_key = {row["pair_key"]: row for row in query_rows}
    evidence_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence:
        evidence_by_key[row["pair_key"]].append(row)
    rows: list[dict[str, Any]] = []
    for pair in pair_rows:
        query = query_by_key.get(pair["pair_key"])
        ev = evidence_by_key.get(pair["pair_key"], [])
        if ev:
            status = "exact_hit" if any(row["match_kind"] == "exact_hit" for row in ev) else "normalized_hit"
        else:
            status = "no_external_hit"
        reason_codes = ["rxnorm_product_concept_not_clinical_clearance"]
        if not pair["queryable"]:
            reason_codes.append("pair_not_queryable_after_name_normalization")
        if query and query["total_concept_count"] > 0 and not ev:
            reason_codes.append("rxnorm_query_returned_but_no_verified_pair_text_match")
        if ev:
            reason_codes.append("rxnorm_combination_product_concept_match")
        rows.append(
            {
                "schema_version": 1,
                "pair_status_id": f"rxnorm-combo-pair-status:{stable_id(pair['pair_key'])}",
                "pair_key": pair["pair_key"],
                "representative_pair_id": pair["representative_pair_id"],
                "candidate_pair_ids": pair["candidate_pair_ids"],
                "candidate_row_count": pair["candidate_row_count"],
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "query_drug_a": pair["query_drug_a"],
                "query_drug_b": pair["query_drug_b"],
                "queryable": pair["queryable"],
                "rxnorm_status": status,
                "total_concept_count": query["total_concept_count"] if query else 0,
                "query_response_sha256": stable_id(*(r["response_sha256"] for r in query["query_responses"])) if query else None,
                "concept_evidence_rows": len(ev),
                "rxcuis": uniq([row["rxcui"] for row in ev])[:50],
                "evidence_ids": [row["evidence_id"] for row in ev[:50]],
                "ttys": uniq([row["tty"] for row in ev])[:20],
                "overall_external_source_status_after_issue1240": status,
                "promotion_status": "blocked_requires_safety_outcome_falsification_and_human_review",
                "reason_codes": reason_codes,
                "next_validation_experiment": next_validation(status),
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda item: (-status_rank(item["rxnorm_status"]), item["pair_key"], item["representative_pair_id"]))
    return rows


def next_validation(status: str) -> str:
    if status == "no_external_hit":
        return "Continue source expansion or richer synonym normalization; do not promote this pair."
    return (
        "Validate RxNorm product-concept context, then run independent component safety, "
        "pair-interaction, outcome, falsification, and human-review gates with physical readback."
    )


def candidate_status_rows(candidates: list[dict[str, Any]], pair_status: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_key = {row["pair_key"]: row for row in pair_status}
    rows: list[dict[str, Any]] = []
    for prior in candidates:
        pair = by_key[prior["pair_key"]]
        rows.append(
            {
                "schema_version": 1,
                "status_id": f"rxnorm-combo-candidate-status:{stable_id(prior['status_id'], prior['pair_id'])}",
                "source_issue1236_status_id": prior["status_id"],
                "pair_id": prior["pair_id"],
                "pair_key": prior["pair_key"],
                "drug_a": prior["drug_a"],
                "drug_b": prior["drug_b"],
                "previous_overall_external_source_status": prior["overall_external_source_status_after_issue1236"],
                "rxnorm_status": pair["rxnorm_status"],
                "overall_external_source_status_after_issue1240": pair["rxnorm_status"],
                "concept_evidence_rows": pair["concept_evidence_rows"],
                "rxcuis": pair["rxcuis"],
                "pair_status_id": pair["pair_status_id"],
                "promotion_status": pair["promotion_status"],
                "reason_codes": pair["reason_codes"],
                "next_validation_experiment": pair["next_validation_experiment"],
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    rows.sort(key=lambda item: (-status_rank(item["rxnorm_status"]), item["pair_key"], item["pair_id"]))
    return rows


def build_bridge_rows(pair_status: list[dict[str, Any]], evidence: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in pair_status:
        text = (
            f"RxNorm combination-product pair status {row['representative_pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} has RxNorm status {row['rxnorm_status']} with {row['concept_evidence_rows']} "
            f"verified concept evidence rows and remains {row['promotion_status']}."
        )
        terms = uniq([row["drug_a"], row["drug_b"], row["rxnorm_status"], *row["rxcuis"][:5]])
        rows.append(
            {
                "id": row["pair_status_id"],
                "domain": "rxnorm_combination_product_pair_status",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1240_rxnorm_combination_product_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "rxnorm_status": row["rxnorm_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in evidence:
        text = (
            f"RxNorm combination-product evidence {row['evidence_id']}: {row['drug_a']} plus {row['drug_b']} "
            f"matches RxCUI {row['rxcui']} as {row['match_kind']} in term type {row['tty']} and remains "
            f"{row['promotion_status']}."
        )
        terms = uniq([row["drug_a"], row["drug_b"], row["rxcui"], row["match_kind"], row["tty"]])
        rows.append(
            {
                "id": row["evidence_id"],
                "domain": "rxnorm_combination_product_evidence",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1240_rxnorm_combination_product_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "rxcui": row["rxcui"],
                    "match_kind": row["match_kind"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows[:1000]


def response_schema_fingerprint(query_rows: list[dict[str, Any]]) -> dict[str, Any]:
    concept_keys: set[str] = set()
    group_keys: set[str] = set()
    for row in query_rows:
        for response in row["query_responses"]:
            groups = response.get("response_json", {}).get("drugGroup", {}).get("conceptGroup") or []
            for group in groups:
                group_keys.update(group.keys())
                for concept in group.get("conceptProperties") or []:
                    concept_keys.update(concept.keys())
    payload = {"concept_keys": sorted(concept_keys), "group_keys": sorted(group_keys)}
    return {**payload, "schema_fingerprint_sha256": sha256_bytes(json.dumps(payload, sort_keys=True).encode("utf-8"))}


def build_metrics(
    candidates: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
) -> dict[str, Any]:
    pair_counts = Counter(row["rxnorm_status"] for row in pair_status)
    candidate_counts = Counter(row["rxnorm_status"] for row in candidate_status)
    tty_counts = Counter(row["tty"] for row in evidence)
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "issue1236_remaining_no_hit_rows": len(candidates),
        "unique_pair_keys": len(pair_rows),
        "queryable_pair_keys": sum(1 for row in pair_rows if row["queryable"]),
        "rxnorm_query_response_rows": len(query_rows),
        "rxnorm_concept_evidence_rows": len(evidence),
        "rxnorm_pair_status_rows": len(pair_status),
        "candidate_rxnorm_status_rows": len(candidate_status),
        "pair_status_counts": dict(sorted(pair_counts.items())),
        "candidate_status_counts": dict(sorted(candidate_counts.items())),
        "candidate_rows_with_issue1240_hit": sum(1 for row in candidate_status if row["rxnorm_status"] != "no_external_hit"),
        "remaining_no_hit_after_issue1240": sum(1 for row in candidate_status if row["rxnorm_status"] == "no_external_hit"),
        "query_rows_with_concepts": sum(1 for row in query_rows if row["total_concept_count"] > 0),
        "query_rows_with_verified_evidence": len({row["pair_key"] for row in evidence}),
        "tty_counts": dict(sorted(tty_counts.items())),
        "response_schema": response_schema_fingerprint(query_rows),
        "top_hits": [
            {
                "pair_key": row["pair_key"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "rxnorm_status": row["rxnorm_status"],
                "concept_evidence_rows": row["concept_evidence_rows"],
                "rxcuis": row["rxcuis"][:8],
                "ttys": row["ttys"],
            }
            for row in pair_status
            if row["rxnorm_status"] != "no_external_hit"
        ][:50],
    }


def build_input_manifest(
    inputs: dict[str, str],
    raw_artifacts: dict[str, dict[str, Any]],
    candidates: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    issue1236_persisted_readback: dict[str, Any],
    issue1236_calyx_readback: dict[str, Any],
    rxnorm_version: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1240,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1236_candidate_status": artifact(Path(inputs["issue1236_candidate_status"]), jsonl=True),
            "issue1236_persisted_readback": artifact(Path(inputs["issue1236_persisted_readback"])),
            "issue1236_calyx_readback": artifact(Path(inputs["issue1236_calyx_readback"])),
            "issue1236_output_manifest": artifact(Path(inputs["issue1236_output_manifest"])),
            **raw_artifacts,
        },
        "source_contract": {
            "issue1236_persisted_readback_status": issue1236_persisted_readback.get("status"),
            "issue1236_persisted_assertions_all_true": all_assertions_true(issue1236_persisted_readback),
            "issue1236_calyx_readback_status": issue1236_calyx_readback.get("status"),
            "issue1236_calyx_assertions_all_true": all_assertions_true(issue1236_calyx_readback),
            "remaining_no_hit_rows": len(candidates),
            "unique_pair_keys": len(pair_rows),
            "rxnorm_version": rxnorm_version,
            "rxnav_rate_limit_requests_per_second": 20,
        },
        "accepted_sources": [
            {
                "source": "NLM RxNorm API",
                "role": "current RxNorm combination-product concept source mining",
                "api_endpoint": RXNORM_DRUGS_ENDPOINT,
                "docs_url": RXNORM_API_DOCS_URL,
                "terms_url": RXNAV_TERMS_URL,
                "license_observation": "RxNorm API docs/terms state RxNorm API content is non-proprietary NLM vocabulary and no license is needed except the proprietary endpoint, which is not used.",
                "excluded_source": "RxNav drug/drug interaction API is discontinued and not used.",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        ],
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    query_rows: list[dict[str, Any]],
    evidence: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    issue1236_persisted_readback: dict[str, Any],
    issue1236_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "rxnorm_combination_query_responses": artifact(out_dir / "rxnorm_combination_query_responses.jsonl", jsonl=True),
        "rxnorm_combination_evidence": artifact(out_dir / "rxnorm_combination_evidence.jsonl", jsonl=True),
        "rxnorm_pair_status": artifact(out_dir / "rxnorm_pair_status.jsonl", jsonl=True),
        "candidate_rxnorm_status": artifact(out_dir / "candidate_rxnorm_status.jsonl", jsonl=True),
        "rxnorm_combination_bridge_rows": artifact(out_dir / "rxnorm_combination_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    queryable_keys = {row["pair_key"] for row in pair_rows if row["queryable"]}
    query_response_keys = {row["pair_key"] for row in query_rows}
    evidence_keys = {row["pair_key"] for row in evidence}
    pair_hit_keys = {row["pair_key"] for row in pair_status if row["rxnorm_status"] != "no_external_hit"}
    return {
        "schema_version": 1,
        "issue": 1240,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "issue1236_persisted_readback_all_true": all_assertions_true(issue1236_persisted_readback),
            "issue1236_calyx_readback_all_true": all_assertions_true(issue1236_calyx_readback),
            "candidate_status_rows_for_every_remaining_no_hit": len(candidate_status) == len(candidates),
            "pair_status_rows_for_every_unique_pair_key": len(pair_status) == len(pair_rows),
            "query_response_for_every_queryable_pair_key": query_response_keys == queryable_keys,
            "all_pair_status_values_allowed": all(row["rxnorm_status"] in STATUS_VALUES for row in pair_status),
            "all_candidate_status_values_allowed": all(row["rxnorm_status"] in STATUS_VALUES for row in candidate_status),
            "all_evidence_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in evidence),
            "all_status_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in pair_status)
            and all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in candidate_status),
            "all_hits_have_evidence": pair_hit_keys.issubset(evidence_keys),
            "all_evidence_rows_have_rxcui": all(bool(row["rxcui"]) for row in evidence),
            "all_candidate_rows_remain_blocked": all(
                row["promotion_status"] == "blocked_requires_safety_outcome_falsification_and_human_review"
                for row in candidate_status
            ),
            "bridge_rows_1000_or_less": artifacts["rxnorm_combination_bridge_rows"]["rows"] <= 1000,
        },
        "row_counts": {
            "remaining_no_hit_candidates": len(candidates),
            "unique_pair_keys": len(pair_rows),
            "query_response_rows": len(query_rows),
            "evidence_rows": len(evidence),
            "pair_status_rows": len(pair_status),
            "candidate_status_rows": len(candidate_status),
        },
    }


def run(root: Path, inputs: dict[str, str], *, max_pairs: int | None, request_sleep_seconds: float) -> dict[str, Any]:
    require_inputs(inputs)
    raw_dir = root / "raw"
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_artifacts = fetch_raw_sources(raw_dir)
    rxnorm_version = read_json(raw_dir / "rxnorm_version.json")
    all_rows = rows_jsonl(Path(inputs["issue1236_candidate_status"]))
    candidates = remaining_no_hit_rows(all_rows)
    pair_rows = representative_pairs(candidates)
    if max_pairs is not None:
        keep = {row["pair_key"] for row in pair_rows[:max_pairs]}
        pair_rows = [row for row in pair_rows if row["pair_key"] in keep]
        candidates = [row for row in candidates if row["pair_key"] in keep]
    issue1236_persisted_readback = read_json(Path(inputs["issue1236_persisted_readback"]))
    issue1236_calyx_readback = read_json(Path(inputs["issue1236_calyx_readback"]))

    write_json(
        out_dir / "input_manifest.json",
        build_input_manifest(
            inputs,
            raw_artifacts,
            candidates,
            pair_rows,
            issue1236_persisted_readback,
            issue1236_calyx_readback,
            rxnorm_version,
        ),
    )
    query_rows = fetch_rxnorm_queries(pair_rows, out_dir / "rxnorm_combination_query_responses.jsonl", request_sleep_seconds)
    evidence = build_evidence_rows(query_rows)
    write_jsonl(out_dir / "rxnorm_combination_evidence.jsonl", evidence)
    pair_status = pair_status_rows(pair_rows, query_rows, evidence)
    write_jsonl(out_dir / "rxnorm_pair_status.jsonl", pair_status)
    candidate_status = candidate_status_rows(candidates, pair_status)
    write_jsonl(out_dir / "candidate_rxnorm_status.jsonl", candidate_status)
    source_path = out_dir / "rxnorm_pair_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(pair_status, evidence, source_path, source_sha)
    write_jsonl(out_dir / "rxnorm_combination_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(candidates, pair_rows, query_rows, evidence, pair_status, candidate_status)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1240,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "rxnorm_combination_query_responses": artifact(
                out_dir / "rxnorm_combination_query_responses.jsonl", jsonl=True
            ),
            "rxnorm_combination_evidence": artifact(out_dir / "rxnorm_combination_evidence.jsonl", jsonl=True),
            "rxnorm_pair_status": artifact(out_dir / "rxnorm_pair_status.jsonl", jsonl=True),
            "candidate_rxnorm_status": artifact(out_dir / "candidate_rxnorm_status.jsonl", jsonl=True),
            "rxnorm_combination_bridge_rows": artifact(out_dir / "rxnorm_combination_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(
        out_dir,
        candidates,
        pair_rows,
        query_rows,
        evidence,
        pair_status,
        candidate_status,
        issue1236_persisted_readback,
        issue1236_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "rxnorm_pair_status": output_manifest["artifacts"]["rxnorm_pair_status"],
            "candidate_rxnorm_status": output_manifest["artifacts"]["candidate_rxnorm_status"],
            "rxnorm_combination_evidence": output_manifest["artifacts"]["rxnorm_combination_evidence"],
            "bridge_rows": output_manifest["artifacts"]["rxnorm_combination_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--issue1236-candidate-status")
    parser.add_argument("--issue1236-persisted-readback")
    parser.add_argument("--issue1236-calyx-readback")
    parser.add_argument("--issue1236-output-manifest")
    parser.add_argument("--max-pairs", type=int)
    parser.add_argument("--request-sleep-seconds", type=float, default=REQUEST_SLEEP_SECONDS)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    inputs = dict(DEFAULT_INPUTS)
    for arg_name, input_name in [
        ("issue1236_candidate_status", "issue1236_candidate_status"),
        ("issue1236_persisted_readback", "issue1236_persisted_readback"),
        ("issue1236_calyx_readback", "issue1236_calyx_readback"),
        ("issue1236_output_manifest", "issue1236_output_manifest"),
    ]:
        value = getattr(args, arg_name)
        if value:
            inputs[input_name] = value
    result = run(Path(args.root), inputs, max_pairs=args.max_pairs, request_sleep_seconds=args.request_sleep_seconds)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
