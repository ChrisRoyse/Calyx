#!/usr/bin/env python3
"""#1258 RxNorm/RxNav canonicalization for nSIDES no-map remainder."""

from __future__ import annotations

import argparse
import csv
import gzip
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


csv.field_size_limit(sys.maxsize)

CLINICAL_BOUNDARY = (
    "RxNorm/RxNav canonicalization is identity/source-mapping support only; "
    "mapped RxCUIs, ingredients, approximate matches, TwoSIDES RxCUI rows, and "
    "OffSIDES RxCUI rows are blockers/review inputs, not safety clearance, "
    "efficacy, treatment guidance, dosing guidance, recommendation, clinical "
    "actionability, pair-interaction proof, or cure evidence."
)

PROMOTION_STATUS = "blocked_requires_external_identity_safety_outcome_falsification_and_human_review"
RXNORM_EVIDENCE_KIND = "rxnorm_canonical_identity_mapping_not_clinical_actionability"
TWOSIDES_RXCUI_EVIDENCE_KIND = "twosides_rxcui_pair_adverse_effect_source_match_not_clearance"
OFFSIDES_RXCUI_CONTEXT_KIND = "offsides_rxcui_single_drug_adverse_effect_context_not_pair_proof"

ISSUE1257_ROOT = "/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z"

DEFAULT_INPUTS = {
    "issue1257_candidate_status": f"{ISSUE1257_ROOT}/out/candidate_nsides_status.jsonl",
    "issue1257_pair_status": f"{ISSUE1257_ROOT}/out/nsides_pair_status.jsonl",
    "issue1257_offsides_context": f"{ISSUE1257_ROOT}/out/offsides_single_drug_context.jsonl",
    "issue1257_persisted_readback": f"{ISSUE1257_ROOT}/out/persisted_readback.json",
    "issue1257_calyx_readback": f"{ISSUE1257_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1257_output_manifest": f"{ISSUE1257_ROOT}/out/output_manifest.json",
    "issue1257_twosides_archive": f"{ISSUE1257_ROOT}/raw/twosides.csv.gz",
    "issue1257_offsides_archive": f"{ISSUE1257_ROOT}/raw/offsides.csv.gz",
}

USER_AGENT = "calyx-discovery/issue1258"
REQUEST_SLEEP_SECONDS = 0.08
MAX_RELATED_RXCUIS_PER_TERM = 5
MAX_APPROXIMATE_CANDIDATES = 5
MAX_OFFSIDES_CONTEXT_PER_PAIR_SIDE = 5
MAX_BRIDGE_ROWS = 1000
MAX_BRIDGE_EVIDENCE_ROWS = 60

RAW_DOC_URLS = {
    "rxnorm_api_overview": "https://lhncbc.nlm.nih.gov/RxNav/APIs/RxNormAPIs.html",
    "find_rxcui_by_string": "https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.findRxcuiByString.html",
    "approximate_match": "https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getApproximateMatch.html",
    "related_by_type": "https://lhncbc.nlm.nih.gov/RxNav/APIs/api-RxNorm.getRelatedByType.html",
}

PAIR_STATUS_VALUES = {
    "rxnorm_twosides_rxcui_pair_hit_still_blocked",
    "rxnorm_canonical_rxcui_overlap_still_blocked",
    "rxnorm_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked",
    "rxnorm_approximate_only_provisional_still_blocked",
    "rxnorm_unmapped_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "rxnorm_candidate_twosides_rxcui_pair_hit_still_blocked",
    "rxnorm_candidate_canonical_rxcui_overlap_still_blocked",
    "rxnorm_candidate_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked",
    "rxnorm_candidate_approximate_only_provisional_still_blocked",
    "rxnorm_candidate_unmapped_still_blocked",
}

TERM_STATUS_VALUES = {
    "rxnorm_term_trusted_mapping_still_blocked",
    "rxnorm_term_approximate_only_provisional_still_blocked",
    "rxnorm_term_no_mapping_still_blocked",
}

TWOSIDES_HEADER = [
    "drug_1_rxnorn_id",
    "drug_1_concept_name",
    "drug_2_rxnorm_id",
    "drug_2_concept_name",
    "condition_meddra_id",
    "condition_concept_name",
    "A",
    "B",
    "C",
    "D",
    "PRR",
    "PRR_error",
    "mean_reporting_frequency",
]

OFFSIDES_HEADER = [
    "drug_rxnorn_id",
    "drug_concept_name",
    "condition_meddra_id",
    "condition_concept_name",
    "A",
    "B",
    "C",
    "D",
    "PRR",
    "PRR_error",
    "mean_reporting_frequency",
]


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


def norm_name(value: object) -> str:
    text = clean_text(value)
    text = re.sub(r"\[[^\]]*\]", " ", text)
    text = re.sub(r"\([^)]*\)", " ", text)
    text = re.sub(r"[^A-Za-z0-9]+", " ", text)
    return " ".join(text.lower().split())


def read_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
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
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True) + "\n")


def jsonl_count(path: Path) -> int:
    if path.stat().st_size == 0:
        return 0
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def artifact(path: Path, jsonl: bool = False) -> dict[str, Any]:
    item: dict[str, Any] = {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    if jsonl:
        item["rows"] = jsonl_count(path)
    return item


def all_assertions_true(payload: dict[str, Any]) -> bool:
    assertions = payload.get("assertions")
    return isinstance(assertions, dict) and bool(assertions) and all(assertions.values())


def require_inputs(inputs: dict[str, str]) -> None:
    missing = [key for key, path in inputs.items() if not Path(path).exists()]
    if missing:
        raise SystemExit(f"missing required input artifacts: {missing}")


def unique_terms_from_pairs(pairs: list[dict[str, Any]]) -> list[str]:
    terms = {clean_text(row["drug_a"]) for row in pairs}
    terms.update(clean_text(row["drug_b"]) for row in pairs)
    return sorted(term for term in terms if term)


def load_candidates(rows: list[dict[str, Any]], max_pairs: int | None = None) -> list[dict[str, Any]]:
    allowed = {
        "nsides_candidate_no_source_name_mapping_still_blocked",
        "nsides_candidate_offsides_single_drug_context_without_pair_hit_still_blocked",
    }
    out = [row for row in rows if row.get("nsides_candidate_status") in allowed]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_pairs is not None:
        selected_pair_keys = sorted({row["pair_key"] for row in out})[:max_pairs]
        out = [row for row in out if row["pair_key"] in selected_pair_keys]
    return out


def pair_rows(candidates: list[dict[str, Any]], upstream_pair_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    upstream_by_key = {row["pair_key"]: row for row in upstream_pair_rows}
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        grouped[row["pair_key"]].append(row)
    out: list[dict[str, Any]] = []
    for pair_key, members in sorted(grouped.items()):
        first = members[0]
        upstream = upstream_by_key.get(pair_key, {})
        out.append(
            {
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "representative_pair_id": first.get("pair_id") or upstream.get("representative_pair_id"),
                "source_pair_ids": sorted({row.get("pair_id") for row in members if row.get("pair_id")}),
                "source_nsides_candidate_status_ids": sorted(
                    {
                        row.get("nsides_candidate_status_id")
                        for row in members
                        if row.get("nsides_candidate_status_id")
                    }
                ),
                "source_nsides_pair_status_id": first.get("nsides_pair_status_id") or upstream.get("nsides_pair_status_id"),
                "source_nsides_pair_status": first.get("nsides_pair_status") or upstream.get("nsides_pair_status"),
                "candidate_count": len(members),
            }
        )
    return out


def fetch_url(raw_dir: Path, key: str, url: str, filename: str) -> dict[str, Any]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    path = raw_dir / filename
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=120) as response, path.open("wb") as handle:
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            handle.write(chunk)
    time.sleep(REQUEST_SLEEP_SECONDS)
    status = {
        "key": key,
        "url": url,
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
    }
    write_json(path.with_suffix(path.suffix + ".status"), status)
    return status


def fetch_raw_docs(raw_dir: Path) -> dict[str, dict[str, Any]]:
    return {
        key: fetch_url(raw_dir, key, url, f"{key}.html")
        for key, url in RAW_DOC_URLS.items()
    }


def fetch_api_json(
    response_dir: Path,
    request_rows: list[dict[str, Any]],
    kind: str,
    subject: str,
    url: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    response_dir.mkdir(parents=True, exist_ok=True)
    response_id = f"rxnav-response:{stable_id(kind, subject, url)}"
    filename = f"{stable_id(kind, subject, url, length=32)}.json"
    path = response_dir / filename
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    status_code = 0
    error = ""
    try:
        with urllib.request.urlopen(req, timeout=120) as response:
            status_code = response.status
            body = response.read()
    except urllib.error.HTTPError as exc:
        status_code = exc.code
        body = exc.read()
        error = str(exc)
    except Exception as exc:
        body = json.dumps({"error": type(exc).__name__, "message": str(exc)}).encode("utf-8")
        error = str(exc)
    path.write_bytes(body)
    sha = sha256_path(path)
    row = {
        "schema_version": 1,
        "rxnav_response_id": response_id,
        "request_kind": kind,
        "subject": subject,
        "url": url,
        "http_status": status_code,
        "error": error,
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha,
        "retrieved_utc": now_utc(),
    }
    request_rows.append(row)
    time.sleep(REQUEST_SLEEP_SECONDS)
    payload: dict[str, Any]
    try:
        payload = json.loads(body.decode("utf-8"))
    except Exception:
        payload = {"parse_error": True, "raw_sha256": sha}
    return payload, row


def rxnav_url(path: str, params: dict[str, str]) -> str:
    return f"https://rxnav.nlm.nih.gov/REST/{path}?{urllib.parse.urlencode(params)}"


def listify(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, list):
        return value
    return [value]


def extract_rxnorm_ids(payload: dict[str, Any]) -> list[str]:
    ids = payload.get("idGroup", {}).get("rxnormId")
    return sorted({clean_text(item) for item in listify(ids) if clean_text(item)})


def extract_approximate_candidates(payload: dict[str, Any]) -> list[dict[str, Any]]:
    candidates = payload.get("approximateGroup", {}).get("candidate")
    out: list[dict[str, Any]] = []
    for index, item in enumerate(listify(candidates), start=1):
        if not isinstance(item, dict):
            continue
        out.append(
            {
                "rank_order": index,
                "rxcui": clean_text(item.get("rxcui")),
                "rxaui": clean_text(item.get("rxaui")),
                "score": clean_text(item.get("score")),
                "rank": clean_text(item.get("rank")),
                "name": clean_text(item.get("name")),
                "source": clean_text(item.get("source")),
                "provisional": True,
                "trusted_for_pair_matching": False,
                "manual_review_required": True,
            }
        )
    return out


def extract_related_concepts(payload: dict[str, Any]) -> list[dict[str, Any]]:
    related_group = payload.get("relatedGroup", {})
    concepts: list[dict[str, Any]] = []
    for group in listify(related_group.get("conceptGroup")):
        if not isinstance(group, dict):
            continue
        tty = clean_text(group.get("tty"))
        for concept in listify(group.get("conceptProperties")):
            if not isinstance(concept, dict):
                continue
            concepts.append(
                {
                    "rxcui": clean_text(concept.get("rxcui")),
                    "name": clean_text(concept.get("name")),
                    "synonym": clean_text(concept.get("synonym")),
                    "tty": clean_text(concept.get("tty")) or tty,
                    "suppress": clean_text(concept.get("suppress")),
                    "psn": clean_text(concept.get("psn")),
                }
            )
    dedup: dict[tuple[str, str], dict[str, Any]] = {}
    for concept in concepts:
        if concept["rxcui"]:
            dedup[(concept["rxcui"], concept["tty"])] = concept
    return [dedup[key] for key in sorted(dedup)]


def canonicalize_terms(
    terms: list[str],
    raw_dir: Path,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    response_dir = raw_dir / "rxnav_responses"
    request_rows: list[dict[str, Any]] = []
    statuses: list[dict[str, Any]] = []
    for term in terms:
        exact_url = rxnav_url("rxcui.json", {"name": term, "search": "2"})
        exact_payload, exact_response = fetch_api_json(response_dir, request_rows, "find_rxcui_exact_or_normalized", term, exact_url)
        exact_rxcuis = extract_rxnorm_ids(exact_payload)

        approx_url = rxnav_url("approximateTerm.json", {"term": term, "maxEntries": str(MAX_APPROXIMATE_CANDIDATES), "option": "1"})
        approx_payload, approx_response = fetch_api_json(response_dir, request_rows, "approximate_term", term, approx_url)
        approximate_candidates = extract_approximate_candidates(approx_payload)

        related_concepts: list[dict[str, Any]] = []
        related_response_ids: list[str] = []
        for rxcui in exact_rxcuis[:MAX_RELATED_RXCUIS_PER_TERM]:
            related_url = rxnav_url(f"rxcui/{rxcui}/related.json", {"tty": "IN PIN MIN", "expand": "psn"})
            related_payload, related_response = fetch_api_json(response_dir, request_rows, "related_ingredients", rxcui, related_url)
            related_response_ids.append(related_response["rxnav_response_id"])
            related_concepts.extend(extract_related_concepts(related_payload))

        trusted_rxcuis = set(exact_rxcuis)
        trusted_rxcuis.update(concept["rxcui"] for concept in related_concepts if concept.get("rxcui"))
        if trusted_rxcuis:
            status = "rxnorm_term_trusted_mapping_still_blocked"
            reason = "exact_or_normalized_rxnorm_mapping"
        elif approximate_candidates:
            status = "rxnorm_term_approximate_only_provisional_still_blocked"
            reason = "approximate_mapping_provisional_manual_review_required"
        else:
            status = "rxnorm_term_no_mapping_still_blocked"
            reason = "no_rxnorm_mapping"
        statuses.append(
            {
                "schema_version": 1,
                "rxnorm_term_status_id": f"rxnorm-term-status:{stable_id(term, status)}",
                "source_issue": 1258,
                "term": term,
                "term_norm": norm_name(term),
                "rxnorm_term_status": status,
                "exact_or_normalized_rxcuis": exact_rxcuis,
                "trusted_rxcuis": sorted(trusted_rxcuis),
                "related_ingredient_concepts": sorted(related_concepts, key=lambda item: (item.get("rxcui", ""), item.get("tty", ""))),
                "approximate_candidates": approximate_candidates,
                "approximate_candidates_provisional": bool(approximate_candidates),
                "manual_review_required_for_approximate": bool(approximate_candidates),
                "exact_response_id": exact_response["rxnav_response_id"],
                "approximate_response_id": approx_response["rxnav_response_id"],
                "related_response_ids": related_response_ids,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "evidence_kind": RXNORM_EVIDENCE_KIND,
                "reason_codes": [
                    "rxnorm_identity_mapping_not_clinical_actionability",
                    "requires_external_identity_safety_outcome_falsification_and_human_review",
                    reason,
                ],
            }
        )
    return statuses, request_rows


def csv_gzip_header(path: Path) -> list[str]:
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        reader = csv.reader(handle)
        return next(reader)


def csv_gzip_rows(path: Path):
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            yield row


def row_sha(row: dict[str, Any]) -> str:
    payload = {k: v for k, v in row.items() if not k.startswith("_")}
    return sha256_bytes(json.dumps(payload, sort_keys=True).encode("utf-8"))


def numeric(value: object) -> float:
    try:
        return float(clean_text(value))
    except ValueError:
        return 0.0


def pair_rxcui_indexes(
    pairs: list[dict[str, Any]],
    term_by_name: dict[str, dict[str, Any]],
) -> tuple[dict[str, set[str]], dict[str, set[str]], dict[str, set[str]], dict[str, set[str]]]:
    a_index: dict[str, set[str]] = defaultdict(set)
    b_index: dict[str, set[str]] = defaultdict(set)
    trusted_by_pair_a: dict[str, set[str]] = {}
    trusted_by_pair_b: dict[str, set[str]] = {}
    for pair in pairs:
        left = set(term_by_name[pair["drug_a"]]["trusted_rxcuis"])
        right = set(term_by_name[pair["drug_b"]]["trusted_rxcuis"])
        trusted_by_pair_a[pair["pair_key"]] = left
        trusted_by_pair_b[pair["pair_key"]] = right
        for rxcui in left:
            a_index[rxcui].add(pair["pair_key"])
        for rxcui in right:
            b_index[rxcui].add(pair["pair_key"])
    return a_index, b_index, trusted_by_pair_a, trusted_by_pair_b


def scan_twosides_by_rxcui(
    path: Path,
    pairs_by_key: dict[str, dict[str, Any]],
    term_by_name: dict[str, dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    header = csv_gzip_header(path)
    if header != TWOSIDES_HEADER:
        raise SystemExit(f"unexpected TwoSIDES header: {header}")
    source_sha = sha256_path(path)
    a_index, b_index, _trusted_a, _trusted_b = pair_rxcui_indexes(list(pairs_by_key.values()), term_by_name)
    evidence_rows: list[dict[str, Any]] = []
    row_count = 0
    for row_index, row in enumerate(csv_gzip_rows(path), start=1):
        row_count = row_index
        rxcui1 = clean_text(row.get("drug_1_rxnorn_id"))
        rxcui2 = clean_text(row.get("drug_2_rxnorm_id"))
        hit_keys = (a_index.get(rxcui1, set()) & b_index.get(rxcui2, set())) | (
            a_index.get(rxcui2, set()) & b_index.get(rxcui1, set())
        )
        for pair_key in sorted(hit_keys):
            pair = pairs_by_key[pair_key]
            evidence_rows.append(
                {
                    "schema_version": 1,
                    "rxnorm_twosides_evidence_id": f"rxnorm-twosides-evidence:{stable_id(pair_key, row_index, row.get('condition_meddra_id'))}",
                    "source_issue": 1258,
                    "pair_key": pair_key,
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source_table": "TWOSIDES",
                    "source_path": str(path),
                    "source_sha256": source_sha,
                    "source_row_index": row_index,
                    "source_row_sha256": row_sha(row),
                    "source_drug_1": {
                        "rxnorm_id": rxcui1,
                        "concept_name": clean_text(row.get("drug_1_concept_name")),
                    },
                    "source_drug_2": {
                        "rxnorm_id": rxcui2,
                        "concept_name": clean_text(row.get("drug_2_concept_name")),
                    },
                    "drug_a_trusted_rxcuis": sorted(term_by_name[pair["drug_a"]]["trusted_rxcuis"]),
                    "drug_b_trusted_rxcuis": sorted(term_by_name[pair["drug_b"]]["trusted_rxcuis"]),
                    "condition_meddra_id": clean_text(row.get("condition_meddra_id")),
                    "condition_concept_name": clean_text(row.get("condition_concept_name")),
                    "A": clean_text(row.get("A")),
                    "B": clean_text(row.get("B")),
                    "C": clean_text(row.get("C")),
                    "D": clean_text(row.get("D")),
                    "PRR": clean_text(row.get("PRR")),
                    "PRR_error": clean_text(row.get("PRR_error")),
                    "mean_reporting_frequency": clean_text(row.get("mean_reporting_frequency")),
                    "evidence_kind": TWOSIDES_RXCUI_EVIDENCE_KIND,
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                    "reason_codes": [
                        "twosides_row_contains_both_pair_rxnorm_cuis",
                        "adverse_effect_source_mining_not_safety_clearance",
                        "requires_external_safety_outcome_falsification_and_human_review",
                    ],
                }
            )
    evidence_rows.sort(key=lambda row: (row["pair_key"], row["condition_meddra_id"], row["source_row_index"]))
    return evidence_rows, {
        "rows": row_count,
        "header": header,
        "header_sha256": sha256_bytes(json.dumps(header, sort_keys=True).encode("utf-8")),
        "sha256": source_sha,
        "bytes": path.stat().st_size,
        "path": str(path),
    }


def top_context_insert(items: list[dict[str, Any]], row: dict[str, Any]) -> None:
    items.append(row)
    items.sort(key=lambda item: (-numeric(item.get("PRR")), int(item.get("source_row_index", 0))))
    del items[MAX_OFFSIDES_CONTEXT_PER_PAIR_SIDE:]


def scan_offsides_by_rxcui(
    path: Path,
    pairs_by_key: dict[str, dict[str, Any]],
    term_by_name: dict[str, dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, int]], dict[str, Any]]:
    header = csv_gzip_header(path)
    if header != OFFSIDES_HEADER:
        raise SystemExit(f"unexpected OffSIDES header: {header}")
    source_sha = sha256_path(path)
    a_index, b_index, _trusted_a, _trusted_b = pair_rxcui_indexes(list(pairs_by_key.values()), term_by_name)
    context_counts: dict[str, dict[str, int]] = defaultdict(lambda: {"a": 0, "b": 0})
    top_by_pair_role: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    row_count = 0
    for row_index, row in enumerate(csv_gzip_rows(path), start=1):
        row_count = row_index
        rxcui = clean_text(row.get("drug_rxnorn_id"))
        hits: list[tuple[str, str]] = []
        hits.extend((pair_key, "a") for pair_key in sorted(a_index.get(rxcui, set())))
        hits.extend((pair_key, "b") for pair_key in sorted(b_index.get(rxcui, set())))
        for pair_key, role in hits:
            pair = pairs_by_key[pair_key]
            context_counts[pair_key][role] += 1
            context = {
                "schema_version": 1,
                "rxnorm_offsides_context_id": f"rxnorm-offsides-context:{stable_id(pair_key, role, row_index, row.get('condition_meddra_id'))}",
                "source_issue": 1258,
                "pair_key": pair_key,
                "role": role,
                "candidate_drug_name": pair["drug_a"] if role == "a" else pair["drug_b"],
                "source_table": "OFFSIDES",
                "source_path": str(path),
                "source_sha256": source_sha,
                "source_row_index": row_index,
                "source_row_sha256": row_sha(row),
                "source_drug": {
                    "rxnorm_id": rxcui,
                    "concept_name": clean_text(row.get("drug_concept_name")),
                },
                "candidate_trusted_rxcuis": sorted(
                    term_by_name[pair["drug_a" if role == "a" else "drug_b"]]["trusted_rxcuis"]
                ),
                "condition_meddra_id": clean_text(row.get("condition_meddra_id")),
                "condition_concept_name": clean_text(row.get("condition_concept_name")),
                "A": clean_text(row.get("A")),
                "B": clean_text(row.get("B")),
                "C": clean_text(row.get("C")),
                "D": clean_text(row.get("D")),
                "PRR": clean_text(row.get("PRR")),
                "PRR_error": clean_text(row.get("PRR_error")),
                "mean_reporting_frequency": clean_text(row.get("mean_reporting_frequency")),
                "evidence_kind": OFFSIDES_RXCUI_CONTEXT_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "offsides_single_drug_rxcui_context_only",
                    "not_pair_interaction_proof",
                    "requires_external_safety_outcome_falsification_and_human_review",
                ],
            }
            top_context_insert(top_by_pair_role[(pair_key, role)], context)
    context_rows = [
        row
        for key in sorted(top_by_pair_role)
        for row in sorted(top_by_pair_role[key], key=lambda item: (item["pair_key"], item["role"], item["condition_meddra_id"], item["source_row_index"]))
    ]
    return context_rows, {key: dict(value) for key, value in context_counts.items()}, {
        "rows": row_count,
        "header": header,
        "header_sha256": sha256_bytes(json.dumps(header, sort_keys=True).encode("utf-8")),
        "sha256": source_sha,
        "bytes": path.stat().st_size,
        "path": str(path),
    }


def source_inventory(
    raw_docs: dict[str, Any],
    api_request_rows: list[dict[str, Any]],
    twosides_stats: dict[str, Any],
    offsides_stats: dict[str, Any],
) -> dict[str, Any]:
    return {
        "raw_docs": raw_docs,
        "api_responses": {
            "rows": len(api_request_rows),
            "bytes": sum(row["bytes"] for row in api_request_rows),
            "response_sha256": sha256_bytes(json.dumps(api_request_rows, sort_keys=True).encode("utf-8")),
        },
        "tables": {
            "TWOSIDES": twosides_stats,
            "OFFSIDES": offsides_stats,
        },
    }


def source_rows_from_inventory(inventory: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for name, info in sorted(inventory["raw_docs"].items()):
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": f"rxnorm-source:raw_docs:{name}",
                "source_issue": 1258,
                "source_group": "raw_docs",
                "source_name": name,
                "rows": None,
                "bytes": info["bytes"],
                "sha256": info["sha256"],
                "path": info["path"],
                "url": info.get("url"),
                "text": f"RxNorm/RxNav source doc {name} bytes {info['bytes']} sha256 {info['sha256']} url {info.get('url')}",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    api_info = inventory["api_responses"]
    rows.append(
        {
            "schema_version": 1,
            "source_row_id": "rxnorm-source:api_responses:rxnav",
            "source_issue": 1258,
            "source_group": "api_responses",
            "source_name": "rxnav",
            "rows": api_info["rows"],
            "bytes": api_info["bytes"],
            "sha256": api_info["response_sha256"],
            "path": "rxnav_responses/",
            "text": f"RxNav API response corpus rows {api_info['rows']} bytes {api_info['bytes']} sha256 {api_info['response_sha256']}",
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
    )
    for name, info in sorted(inventory["tables"].items()):
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": f"rxnorm-source:tables:{name}",
                "source_issue": 1258,
                "source_group": "tables",
                "source_name": name,
                "rows": info["rows"],
                "bytes": info["bytes"],
                "sha256": info["sha256"],
                "path": info["path"],
                "header": info["header"],
                "header_sha256": info["header_sha256"],
                "text": f"RxNorm remap source table {name} rows {info['rows']} bytes {info['bytes']} sha256 {info['sha256']}",
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def build_pair_status(
    pair: dict[str, Any],
    term_by_name: dict[str, dict[str, Any]],
    twosides_rows: list[dict[str, Any]],
    offsides_counts: dict[str, int],
) -> dict[str, Any]:
    left_status = term_by_name[pair["drug_a"]]
    right_status = term_by_name[pair["drug_b"]]
    left_trusted = set(left_status["trusted_rxcuis"])
    right_trusted = set(right_status["trusted_rxcuis"])
    overlap = sorted(left_trusted & right_trusted)
    evidence_ids = [row["rxnorm_twosides_evidence_id"] for row in twosides_rows]
    if evidence_ids:
        status = "rxnorm_twosides_rxcui_pair_hit_still_blocked"
        reason = "twosides_rxcui_pair_hit"
    elif overlap:
        status = "rxnorm_canonical_rxcui_overlap_still_blocked"
        reason = "same_canonical_rxcui_overlap_not_clinical_claim"
    elif left_trusted and right_trusted:
        status = "rxnorm_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked"
        reason = "both_terms_trusted_mapped_without_pair_source_hit"
    elif left_status["rxnorm_term_status"] == "rxnorm_term_approximate_only_provisional_still_blocked" or right_status["rxnorm_term_status"] == "rxnorm_term_approximate_only_provisional_still_blocked":
        status = "rxnorm_approximate_only_provisional_still_blocked"
        reason = "one_or_both_terms_approximate_only_provisional"
    else:
        status = "rxnorm_unmapped_still_blocked"
        reason = "one_or_both_terms_unmapped"
    return {
        "schema_version": 1,
        "rxnorm_pair_status_id": f"rxnorm-pair-status:{stable_id(pair['pair_key'], status)}",
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "representative_pair_id": pair.get("representative_pair_id"),
        "source_pair_ids": pair.get("source_pair_ids", []),
        "source_nsides_pair_status_id": pair.get("source_nsides_pair_status_id"),
        "source_nsides_pair_status": pair.get("source_nsides_pair_status"),
        "rxnorm_pair_status": status,
        "drug_a_term_status_id": left_status["rxnorm_term_status_id"],
        "drug_b_term_status_id": right_status["rxnorm_term_status_id"],
        "drug_a_trusted_rxcuis": sorted(left_trusted),
        "drug_b_trusted_rxcuis": sorted(right_trusted),
        "canonical_rxcui_overlap": overlap,
        "twosides_rxcui_evidence_rows": len(evidence_ids),
        "twosides_rxcui_evidence_ids": evidence_ids,
        "offsides_drug_a_rxcui_context_rows": offsides_counts.get("a", 0),
        "offsides_drug_b_rxcui_context_rows": offsides_counts.get("b", 0),
        "evidence_kind": RXNORM_EVIDENCE_KIND,
        "twosides_evidence_kind": TWOSIDES_RXCUI_EVIDENCE_KIND,
        "offsides_context_kind": OFFSIDES_RXCUI_CONTEXT_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "rxnorm_identity_mapping_not_clinical_actionability",
            "requires_external_identity_safety_outcome_falsification_and_human_review",
            reason,
        ],
    }


def build_candidate_status(candidate: dict[str, Any], pair_status: dict[str, Any]) -> dict[str, Any]:
    pair_to_candidate = {
        "rxnorm_twosides_rxcui_pair_hit_still_blocked": "rxnorm_candidate_twosides_rxcui_pair_hit_still_blocked",
        "rxnorm_canonical_rxcui_overlap_still_blocked": "rxnorm_candidate_canonical_rxcui_overlap_still_blocked",
        "rxnorm_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked": (
            "rxnorm_candidate_both_terms_trusted_mapped_without_twosides_pair_hit_still_blocked"
        ),
        "rxnorm_approximate_only_provisional_still_blocked": "rxnorm_candidate_approximate_only_provisional_still_blocked",
        "rxnorm_unmapped_still_blocked": "rxnorm_candidate_unmapped_still_blocked",
    }
    status = pair_to_candidate[pair_status["rxnorm_pair_status"]]
    return {
        "schema_version": 1,
        "rxnorm_candidate_status_id": f"rxnorm-candidate-status:{stable_id(candidate['pair_id'], pair_status['rxnorm_pair_status'])}",
        "pair_id": candidate["pair_id"],
        "pair_key": candidate["pair_key"],
        "drug_a": candidate["drug_a"],
        "drug_b": candidate["drug_b"],
        "source_nsides_candidate_status_id": candidate.get("nsides_candidate_status_id"),
        "source_nsides_pair_status_id": candidate.get("nsides_pair_status_id"),
        "source_nsides_candidate_status": candidate.get("nsides_candidate_status"),
        "rxnorm_pair_status_id": pair_status["rxnorm_pair_status_id"],
        "rxnorm_candidate_status": status,
        "rxnorm_pair_status": pair_status["rxnorm_pair_status"],
        "drug_a_trusted_rxcuis": pair_status["drug_a_trusted_rxcuis"],
        "drug_b_trusted_rxcuis": pair_status["drug_b_trusted_rxcuis"],
        "canonical_rxcui_overlap": pair_status["canonical_rxcui_overlap"],
        "twosides_rxcui_evidence_rows": pair_status["twosides_rxcui_evidence_rows"],
        "twosides_rxcui_evidence_ids": pair_status["twosides_rxcui_evidence_ids"],
        "offsides_drug_a_rxcui_context_rows": pair_status["offsides_drug_a_rxcui_context_rows"],
        "offsides_drug_b_rxcui_context_rows": pair_status["offsides_drug_b_rxcui_context_rows"],
        "evidence_kind": RXNORM_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": pair_status["reason_codes"],
    }


def uniq(values: list[object]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for value in values:
        text = clean_text(value)
        if text and text not in seen:
            seen.add(text)
            out.append(text)
    return out


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    term_status: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in source_rows:
        source_terms = uniq(["RxNorm", "RxNav", row["source_group"], row["source_name"], row["sha256"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "rxnorm_source_snapshot",
                "text": f"{row['text']} bridge terms {' '.join(source_terms)}.",
                "bridge_terms": source_terms,
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": row["path"],
                    "source_sha256": row["sha256"],
                    "source_group": row["source_group"],
                    "source_name": row["source_name"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        pair_terms = uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["rxnorm_pair_status"], *row["drug_a_trusted_rxcuis"], *row["drug_b_trusted_rxcuis"]])
        text = (
            f"RxNorm pair remap status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['rxnorm_pair_status']} TwoSIDES RxCUI evidence rows {row['twosides_rxcui_evidence_rows']} "
            f"canonical overlap {','.join(row['canonical_rxcui_overlap']) or 'none'} bridge terms {' '.join(pair_terms)}."
        )
        rows.append(
            {
                "id": row["rxnorm_pair_status_id"],
                "domain": "rxnorm_pair_status",
                "text": text,
                "bridge_terms": pair_terms,
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "rxnorm_pair_status": row["rxnorm_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in candidate_status:
        text = (
            f"RxNorm candidate remap status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['rxnorm_candidate_status']} "
            f"TwoSIDES RxCUI evidence rows {row['twosides_rxcui_evidence_rows']}."
        )
        rows.append(
            {
                "id": row["rxnorm_candidate_status_id"],
                "domain": "rxnorm_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["rxnorm_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "rxnorm_candidate_status": row["rxnorm_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in twosides_evidence[:MAX_BRIDGE_EVIDENCE_ROWS]:
        text = (
            f"TwoSIDES RxCUI evidence {row['rxnorm_twosides_evidence_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} condition {row['condition_concept_name']} PRR {row['PRR']} still blocked."
        )
        rows.append(
            {
                "id": row["rxnorm_twosides_evidence_id"],
                "domain": "rxnorm_twosides_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["condition_concept_name"]]),
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": row["source_path"],
                    "source_sha256": row["source_sha256"],
                    "pair_key": row["pair_key"],
                    "condition_meddra_id": row["condition_meddra_id"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = MAX_BRIDGE_ROWS - len(rows)
    for row in term_status[: max(0, remaining)]:
        text = (
            f"RxNorm term status {row['term']} status {row['rxnorm_term_status']} "
            f"trusted RxCUIs {','.join(row['trusted_rxcuis']) or 'none'} approximate candidates {len(row['approximate_candidates'])}."
        )
        rows.append(
            {
                "id": row["rxnorm_term_status_id"],
                "domain": "rxnorm_term_status",
                "text": text,
                "bridge_terms": uniq([row["term"], row["rxnorm_term_status"], *row["trusted_rxcuis"]]),
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "term": row["term"],
                    "rxnorm_term_status": row["rxnorm_term_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = MAX_BRIDGE_ROWS - len(rows)
    for row in offsides_context[: max(0, remaining)]:
        text = (
            f"OffSIDES RxCUI context {row['rxnorm_offsides_context_id']} pair {row['pair_key']} "
            f"role {row['role']} drug {row['candidate_drug_name']} condition {row['condition_concept_name']} context only."
        )
        rows.append(
            {
                "id": row["rxnorm_offsides_context_id"],
                "domain": "rxnorm_offsides_context",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["candidate_drug_name"], row["condition_concept_name"]]),
                "metadata": {
                    "source_dataset": "issue1258_rxnorm_canonicalization",
                    "source_path": row["source_path"],
                    "source_sha256": row["source_sha256"],
                    "pair_key": row["pair_key"],
                    "role": row["role"],
                    "condition_meddra_id": row["condition_meddra_id"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows[:MAX_BRIDGE_ROWS]


def build_input_manifest(
    inputs: dict[str, str],
    issue1257_persisted_readback: dict[str, Any],
    issue1257_calyx_readback: dict[str, Any],
    source_inventory_payload: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1258,
        "created_utc": now_utc(),
        "inputs": {
            key: artifact(Path(path), jsonl=path.endswith(".jsonl"))
            for key, path in inputs.items()
        },
        "source_inventory": source_inventory_payload,
        "source_contract": {
            "issue1257_persisted_assertions_all_true": all_assertions_true(issue1257_persisted_readback),
            "issue1257_calyx_assertions_all_true": all_assertions_true(issue1257_calyx_readback),
            "clinical_boundary": CLINICAL_BOUNDARY,
            "rxnav_exact_normalized_gate": "findRxcuiByString search=2 trusted for identity remap only",
            "rxnav_approximate_gate": "approximateTerm maxEntries=5 option=1 retained as provisional/manual-review only",
            "twosides_rxcui_gate": "TwoSIDES row has both pair-side trusted RxCUIs in drug_1/drug_2 RxNorm fields",
            "promotion_policy": PROMOTION_STATUS,
        },
    }


def build_metrics(
    terms: list[str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    inventory: dict[str, Any],
    term_status: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    api_request_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "unique_terms": len(terms),
        "candidate_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "api_response_rows": len(api_request_rows),
        "api_response_status_counts": dict(Counter(str(row["http_status"]) for row in api_request_rows)),
        "source_table_rows": {
            "TWOSIDES": inventory["tables"]["TWOSIDES"]["rows"],
            "OFFSIDES": inventory["tables"]["OFFSIDES"]["rows"],
        },
        "term_status_rows": len(term_status),
        "term_status_counts": dict(Counter(row["rxnorm_term_status"] for row in term_status)),
        "twosides_rxcui_evidence_rows": len(twosides_evidence),
        "offsides_rxcui_context_sample_rows": len(offsides_context),
        "pair_status_rows": len(pair_status),
        "pair_status_counts": dict(Counter(row["rxnorm_pair_status"] for row in pair_status)),
        "candidate_status_rows": len(candidate_status),
        "candidate_status_counts": dict(Counter(row["rxnorm_candidate_status"] for row in candidate_status)),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
        "all_rows_blocked": all(
            row.get("promotion_status") == PROMOTION_STATUS
            for row in term_status + twosides_evidence + offsides_context + pair_status + candidate_status
        ),
    }


def build_persisted_readback(
    out_dir: Path,
    terms: list[str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    api_request_rows: list[dict[str, Any]],
    term_status: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1257_persisted_readback: dict[str, Any],
    issue1257_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "rxnorm_source_rows": artifact(out_dir / "rxnorm_source_rows.jsonl", jsonl=True),
        "rxnav_api_response_rows": artifact(out_dir / "rxnav_api_response_rows.jsonl", jsonl=True),
        "rxnorm_term_status": artifact(out_dir / "rxnorm_term_status.jsonl", jsonl=True),
        "rxnorm_twosides_pair_evidence": artifact(out_dir / "rxnorm_twosides_pair_evidence.jsonl", jsonl=True),
        "rxnorm_offsides_single_drug_context": artifact(out_dir / "rxnorm_offsides_single_drug_context.jsonl", jsonl=True),
        "rxnorm_pair_status": artifact(out_dir / "rxnorm_pair_status.jsonl", jsonl=True),
        "candidate_rxnorm_status": artifact(out_dir / "candidate_rxnorm_status.jsonl", jsonl=True),
        "rxnorm_bridge_rows": artifact(out_dir / "rxnorm_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    candidate_pair_keys = {row["pair_key"] for row in candidates}
    pair_status_keys = {row["pair_key"] for row in pair_status}
    status_candidate_ids = {row["pair_id"] for row in candidate_status}
    candidate_ids = {row["pair_id"] for row in candidates}
    status_terms = {row["term"] for row in term_status}
    evidence_by_pair = Counter(row["pair_key"] for row in twosides_evidence)
    response_paths_ok = all(Path(row["path"]).exists() and sha256_path(Path(row["path"])) == row["sha256"] for row in api_request_rows)
    approximate_candidates = [
        candidate
        for row in term_status
        for candidate in row.get("approximate_candidates", [])
    ]
    assertions = {
        "issue1257_persisted_readback_all_true": all_assertions_true(issue1257_persisted_readback),
        "issue1257_calyx_readback_all_true": all_assertions_true(issue1257_calyx_readback),
        "source_rows_present": len(source_rows) >= len(RAW_DOC_URLS) + 3,
        "api_response_rows_present": len(api_request_rows) >= len(terms) * 2,
        "api_response_paths_hash_match": response_paths_ok,
        "term_status_for_every_unique_term": status_terms == set(terms),
        "pair_status_for_every_pair_key": pair_status_keys == candidate_pair_keys,
        "candidate_status_for_every_candidate": status_candidate_ids == candidate_ids,
        "all_twosides_hits_have_evidence": all(
            row["rxnorm_pair_status"] != "rxnorm_twosides_rxcui_pair_hit_still_blocked"
            or evidence_by_pair[row["pair_key"]] > 0
            for row in pair_status
        ),
        "all_evidence_rows_have_both_rxcui_sides": all(
            row.get("drug_a_trusted_rxcuis") and row.get("drug_b_trusted_rxcuis")
            for row in twosides_evidence
        ),
        "all_evidence_rows_have_source_hash": all(row.get("source_sha256") for row in twosides_evidence + offsides_context),
        "all_approximate_candidates_provisional": all(
            item.get("provisional") is True and item.get("trusted_for_pair_matching") is False and item.get("manual_review_required") is True
            for item in approximate_candidates
        ),
        "all_term_status_values_allowed": all(row["rxnorm_term_status"] in TERM_STATUS_VALUES for row in term_status),
        "all_pair_status_values_allowed": all(row["rxnorm_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
        "all_candidate_status_values_allowed": all(
            row["rxnorm_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
        ),
        "all_status_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in term_status + pair_status + candidate_status),
        "all_evidence_rows_have_boundary": all(
            row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in twosides_evidence + offsides_context
        ),
        "all_rows_remain_blocked": all(
            row.get("promotion_status") == PROMOTION_STATUS
            for row in term_status + pair_status + candidate_status + twosides_evidence + offsides_context
        ),
        "bridge_rows_1000_or_less": len(bridge_rows) <= MAX_BRIDGE_ROWS,
    }
    return {
        "schema_version": 1,
        "issue": 1258,
        "status": "ok" if all(assertions.values()) else "failed",
        "created_utc": now_utc(),
        "counts": {
            "unique_terms": len(terms),
            "source_rows": len(source_rows),
            "api_response_rows": len(api_request_rows),
            "term_status_rows": len(term_status),
            "twosides_evidence_rows": len(twosides_evidence),
            "offsides_context_rows": len(offsides_context),
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

    issue1257_persisted_readback = read_json(Path(inputs["issue1257_persisted_readback"]))
    issue1257_calyx_readback = read_json(Path(inputs["issue1257_calyx_readback"]))
    candidate_input_rows = read_jsonl(Path(inputs["issue1257_candidate_status"]))
    pair_input_rows = read_jsonl(Path(inputs["issue1257_pair_status"]))
    candidates = load_candidates(candidate_input_rows, max_pairs=max_pairs)
    pairs = pair_rows(candidates, pair_input_rows)
    pairs_by_key = {row["pair_key"]: row for row in pairs}
    terms = unique_terms_from_pairs(pairs)

    raw_docs = fetch_raw_docs(raw_dir)
    term_status_rows, api_request_rows = canonicalize_terms(terms, raw_dir)
    term_by_name = {row["term"]: row for row in term_status_rows}

    twosides_evidence, twosides_stats = scan_twosides_by_rxcui(Path(inputs["issue1257_twosides_archive"]), pairs_by_key, term_by_name)
    offsides_context, offsides_context_counts, offsides_stats = scan_offsides_by_rxcui(
        Path(inputs["issue1257_offsides_archive"]),
        pairs_by_key,
        term_by_name,
    )

    inventory = source_inventory(raw_docs, api_request_rows, twosides_stats, offsides_stats)
    source_rows = source_rows_from_inventory(inventory)
    write_jsonl(out_dir / "rxnorm_source_rows.jsonl", source_rows)
    write_jsonl(out_dir / "rxnav_api_response_rows.jsonl", api_request_rows)
    write_jsonl(out_dir / "rxnorm_term_status.jsonl", term_status_rows)
    write_jsonl(out_dir / "rxnorm_twosides_pair_evidence.jsonl", twosides_evidence)
    write_jsonl(out_dir / "rxnorm_offsides_single_drug_context.jsonl", offsides_context)

    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in twosides_evidence:
        evidence_by_pair[row["pair_key"]].append(row)

    pair_status_rows = [
        build_pair_status(
            pair,
            term_by_name,
            evidence_by_pair[pair["pair_key"]],
            offsides_context_counts.get(pair["pair_key"], {"a": 0, "b": 0}),
        )
        for pair in pairs
    ]
    pair_status_by_key = {row["pair_key"]: row for row in pair_status_rows}
    candidate_status_rows = [build_candidate_status(row, pair_status_by_key[row["pair_key"]]) for row in candidates]

    write_jsonl(out_dir / "rxnorm_pair_status.jsonl", pair_status_rows)
    write_jsonl(out_dir / "candidate_rxnorm_status.jsonl", candidate_status_rows)

    bridge_rows = build_bridge_rows(
        source_rows,
        term_status_rows,
        pair_status_rows,
        candidate_status_rows,
        twosides_evidence,
        offsides_context,
        out_dir / "candidate_rxnorm_status.jsonl",
        sha256_path(out_dir / "candidate_rxnorm_status.jsonl"),
    )
    write_jsonl(out_dir / "rxnorm_bridge_rows.jsonl", bridge_rows)

    input_manifest = build_input_manifest(inputs, issue1257_persisted_readback, issue1257_calyx_readback, inventory)
    write_json(out_dir / "input_manifest.json", input_manifest)

    metrics = build_metrics(
        terms,
        candidates,
        pairs,
        inventory,
        term_status_rows,
        twosides_evidence,
        offsides_context,
        pair_status_rows,
        candidate_status_rows,
        api_request_rows,
        bridge_rows,
    )
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1258,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "rxnorm_source_rows": artifact(out_dir / "rxnorm_source_rows.jsonl", jsonl=True),
            "rxnav_api_response_rows": artifact(out_dir / "rxnav_api_response_rows.jsonl", jsonl=True),
            "rxnorm_term_status": artifact(out_dir / "rxnorm_term_status.jsonl", jsonl=True),
            "rxnorm_twosides_pair_evidence": artifact(out_dir / "rxnorm_twosides_pair_evidence.jsonl", jsonl=True),
            "rxnorm_offsides_single_drug_context": artifact(out_dir / "rxnorm_offsides_single_drug_context.jsonl", jsonl=True),
            "rxnorm_pair_status": artifact(out_dir / "rxnorm_pair_status.jsonl", jsonl=True),
            "candidate_rxnorm_status": artifact(out_dir / "candidate_rxnorm_status.jsonl", jsonl=True),
            "rxnorm_bridge_rows": artifact(out_dir / "rxnorm_bridge_rows.jsonl", jsonl=True),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)

    persisted_readback = build_persisted_readback(
        out_dir,
        terms,
        candidates,
        pairs,
        source_rows,
        api_request_rows,
        term_status_rows,
        twosides_evidence,
        offsides_context,
        pair_status_rows,
        candidate_status_rows,
        bridge_rows,
        issue1257_persisted_readback,
        issue1257_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", persisted_readback)

    final = {
        "status": persisted_readback["status"],
        "root": str(root),
        "artifacts": {
            "source_rows": artifact(out_dir / "rxnorm_source_rows.jsonl", jsonl=True),
            "api_responses": artifact(out_dir / "rxnav_api_response_rows.jsonl", jsonl=True),
            "term_status": artifact(out_dir / "rxnorm_term_status.jsonl", jsonl=True),
            "twosides_pair_evidence": artifact(out_dir / "rxnorm_twosides_pair_evidence.jsonl", jsonl=True),
            "offsides_context": artifact(out_dir / "rxnorm_offsides_single_drug_context.jsonl", jsonl=True),
            "pair_status": artifact(out_dir / "rxnorm_pair_status.jsonl", jsonl=True),
            "candidate_status": artifact(out_dir / "candidate_rxnorm_status.jsonl", jsonl=True),
            "bridge_rows": artifact(out_dir / "rxnorm_bridge_rows.jsonl", jsonl=True),
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
        "metrics": metrics,
    }
    print(json.dumps(final, indent=2, sort_keys=True))
    if persisted_readback["status"] != "ok":
        raise SystemExit(1)
    return final


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--max-pairs", type=int, default=None)
    for key, default in DEFAULT_INPUTS.items():
        parser.add_argument(f"--{key.replace('_', '-')}", default=default)
    args = parser.parse_args()
    inputs = {
        key: getattr(args, key)
        for key in DEFAULT_INPUTS
    }
    run(Path(args.root), inputs, max_pairs=args.max_pairs)


if __name__ == "__main__":
    main()
