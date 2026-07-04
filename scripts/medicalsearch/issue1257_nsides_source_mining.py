#!/usr/bin/env python3
"""#1257 nSIDES TwoSIDES/OffSIDES source mining after PharmGKB no-hit."""

from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import json
import re
import sys
import time
import urllib.request
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


csv.field_size_limit(sys.maxsize)

CLINICAL_BOUNDARY = (
    "nSIDES TwoSIDES/OffSIDES source mining is adverse-effect/source triage only; "
    "pair adverse-effect rows and single-drug adverse-effect rows are blockers/review "
    "inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, pair-interaction proof, or cure evidence."
)

SOURCE_EVIDENCE_KIND = "nsides_twosides_drug_pair_adverse_effect_source_match_not_clearance"
OFFSIDES_CONTEXT_KIND = "nsides_offsides_single_drug_adverse_effect_context_not_pair_proof"
PROMOTION_STATUS = "blocked_requires_external_safety_outcome_falsification_and_human_review"

ISSUE1256_ROOT = "/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z"

DEFAULT_INPUTS = {
    "issue1256_candidate_status": f"{ISSUE1256_ROOT}/out/candidate_pharmgkb_status.jsonl",
    "issue1256_pair_status": f"{ISSUE1256_ROOT}/out/pharmgkb_pair_status.jsonl",
    "issue1256_persisted_readback": f"{ISSUE1256_ROOT}/out/persisted_readback.json",
    "issue1256_calyx_readback": f"{ISSUE1256_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1256_output_manifest": f"{ISSUE1256_ROOT}/out/output_manifest.json",
}

USER_AGENT = "calyx-discovery/issue1257"
REQUEST_SLEEP_SECONDS = 0.15
MAX_BRIDGE_ROWS = 1000
MAX_BRIDGE_EVIDENCE_ROWS = 80
MAX_OFFSIDES_CONTEXT_PER_PAIR_SIDE = 5

RAW_DOC_URLS = {
    "nsides_home": "https://nsides.io/",
    "tatonetti_stm": "https://tatonettilab.org/resources/tatonetti-stm.html",
}

ARCHIVE_URLS = {
    "twosides": "https://tatonettilab-resources.s3.us-west-1.amazonaws.com/nsides/TWOSIDES.csv.gz",
    "offsides": "https://tatonettilab-resources.s3.us-west-1.amazonaws.com/nsides/OFFSIDES.csv.gz",
}

EXPECTED_HEADERS = {
    "twosides": [
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
    ],
    "offsides": [
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
    ],
}

PAIR_STATUS_VALUES = {
    "nsides_twosides_pair_adverse_effect_hit_still_blocked",
    "nsides_offsides_single_drug_context_without_pair_hit_still_blocked",
    "nsides_no_source_name_mapping_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "nsides_candidate_twosides_pair_adverse_effect_hit_still_blocked",
    "nsides_candidate_offsides_single_drug_context_without_pair_hit_still_blocked",
    "nsides_candidate_no_source_name_mapping_still_blocked",
}

SALT_WORDS = {
    "acetate",
    "adbm",
    "anhydrous",
    "calcium",
    "chloride",
    "dihydrate",
    "disodium",
    "fumarate",
    "hcl",
    "hydrochloride",
    "hydrate",
    "hydrobromide",
    "maleate",
    "mesylate",
    "phosphate",
    "potassium",
    "sodium",
    "succinate",
    "sulfate",
    "tartrate",
    "trihydrate",
    "usp",
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


def stripped_norm(value: object) -> str:
    tokens = norm_name(value).split()
    while len(tokens) > 1 and tokens[-1] in SALT_WORDS:
        tokens.pop()
    return " ".join(tokens)


def name_keys(value: object) -> list[str]:
    keys = [norm_name(value), stripped_norm(value)]
    return sorted({key for key in keys if key})


def source_name_match(candidate: str, source: str) -> dict[str, Any]:
    candidate_norm = norm_name(candidate)
    source_norm = norm_name(source)
    candidate_stripped = stripped_norm(candidate)
    source_stripped = stripped_norm(source)
    exact = bool(candidate_norm and candidate_norm == source_norm)
    stripped_exact = bool(candidate_stripped and candidate_stripped == source_stripped)
    return {
        "candidate_name": clean_text(candidate),
        "source_name": clean_text(source),
        "candidate_norm": candidate_norm,
        "source_norm": source_norm,
        "candidate_stripped_norm": candidate_stripped,
        "source_stripped_norm": source_stripped,
        "exact_norm": exact,
        "stripped_norm": stripped_exact,
        "matched": exact or stripped_exact,
    }


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


def load_candidates(rows: list[dict[str, Any]], max_pairs: int | None = None) -> list[dict[str, Any]]:
    blocked_statuses = {
        "pharmgkb_candidate_single_term_mappings_without_pair_match_still_blocked",
        "pharmgkb_candidate_no_term_mapping_still_blocked",
    }
    out = [row for row in rows if row.get("pharmgkb_candidate_status") in blocked_statuses]
    out.sort(key=lambda row: (row.get("pair_key") or "", row.get("pair_id") or ""))
    if max_pairs is not None:
        selected_pair_keys = sorted({row["pair_key"] for row in out})[:max_pairs]
        out = [row for row in out if row["pair_key"] in selected_pair_keys]
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
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "representative_pair_id": first.get("pair_id"),
                "source_pair_ids": sorted({row.get("pair_id") for row in members if row.get("pair_id")}),
                "source_pharmgkb_candidate_status_ids": sorted(
                    {
                        row.get("pharmgkb_candidate_status_id")
                        for row in members
                        if row.get("pharmgkb_candidate_status_id")
                    }
                ),
                "source_pharmgkb_pair_status_ids": sorted(
                    {
                        row.get("pharmgkb_pair_status_id")
                        for row in members
                        if row.get("pharmgkb_pair_status_id")
                    }
                ),
                "candidate_count": len(members),
            }
        )
    return out


def fetch_url(raw_dir: Path, key: str, url: str, filename: str) -> dict[str, Any]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    path = raw_dir / filename
    if not path.exists() or path.stat().st_size == 0:
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


def fetch_archives(raw_dir: Path) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for key, url in ARCHIVE_URLS.items():
        out[key] = fetch_url(raw_dir, key, url, f"{key}.csv.gz")
    return out


def csv_gzip_rows(path: Path):
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            yield row


def csv_gzip_header(path: Path) -> list[str]:
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        reader = csv.reader(handle)
        return next(reader)


def count_csv_gzip_rows(path: Path) -> int:
    with gzip.open(path, "rt", encoding="utf-8", newline="") as handle:
        reader = csv.reader(handle)
        next(reader)
        return sum(1 for _ in reader)


def header_fingerprint(header: list[str]) -> str:
    return sha256_bytes(json.dumps(header, sort_keys=True).encode("utf-8"))


def build_term_index(pairs: list[dict[str, Any]]) -> dict[str, list[tuple[str, str]]]:
    index: dict[str, list[tuple[str, str]]] = defaultdict(list)
    seen: set[tuple[str, str, str]] = set()
    for pair in pairs:
        for role, drug_name in [("a", pair["drug_a"]), ("b", pair["drug_b"])]:
            for key in name_keys(drug_name):
                marker = (key, pair["pair_key"], role)
                if marker in seen:
                    continue
                seen.add(marker)
                index[key].append((pair["pair_key"], role))
    return index


def source_drug_entries(term_index: dict[str, list[tuple[str, str]]], source_name: str) -> list[tuple[str, str, str]]:
    entries: list[tuple[str, str, str]] = []
    seen: set[tuple[str, str, str]] = set()
    for key in name_keys(source_name):
        for pair_key, role in term_index.get(key, []):
            marker = (pair_key, role, key)
            if marker in seen:
                continue
            seen.add(marker)
            entries.append(marker)
    return entries


def numeric(value: object) -> float | None:
    try:
        return float(clean_text(value))
    except ValueError:
        return None


def top_context_insert(items: list[dict[str, Any]], row: dict[str, Any]) -> None:
    items.append(row)
    items.sort(key=lambda item: (-(numeric(item.get("PRR")) or 0.0), int(item.get("_source_row_index", 0))))
    del items[MAX_OFFSIDES_CONTEXT_PER_PAIR_SIDE:]


def row_sha(row: dict[str, Any]) -> str:
    payload = {k: v for k, v in row.items() if not k.startswith("_")}
    return sha256_bytes(json.dumps(payload, sort_keys=True).encode("utf-8"))


def scan_twosides(
    path: Path,
    pairs_by_key: dict[str, dict[str, Any]],
    term_index: dict[str, list[tuple[str, str]]],
) -> tuple[list[dict[str, Any]], dict[str, int], dict[str, Any]]:
    header = csv_gzip_header(path)
    if header != EXPECTED_HEADERS["twosides"]:
        raise SystemExit(f"unexpected TwoSIDES header: {header}")
    source_sha = sha256_path(path)
    evidence_rows: list[dict[str, Any]] = []
    counts_by_pair: Counter[str] = Counter()
    row_count = 0
    for row_index, row in enumerate(csv_gzip_rows(path), start=1):
        row_count = row_index
        name1 = clean_text(row.get("drug_1_concept_name"))
        name2 = clean_text(row.get("drug_2_concept_name"))
        entries1 = source_drug_entries(term_index, name1)
        entries2 = source_drug_entries(term_index, name2)
        if not entries1 or not entries2:
            continue
        hit_pair_keys: set[str] = set()
        for pair_key_1, role1, _key1 in entries1:
            for pair_key_2, role2, _key2 in entries2:
                if pair_key_1 == pair_key_2 and role1 != role2:
                    hit_pair_keys.add(pair_key_1)
        for pair_key in sorted(hit_pair_keys):
            pair = pairs_by_key[pair_key]
            match_a_name = name1 if source_name_match(pair["drug_a"], name1)["matched"] else name2
            match_b_name = name2 if source_name_match(pair["drug_b"], name2)["matched"] else name1
            match = {
                "drug_a": source_name_match(pair["drug_a"], match_a_name),
                "drug_b": source_name_match(pair["drug_b"], match_b_name),
                "source_drug_1": {
                    "rxnorm_id": clean_text(row.get("drug_1_rxnorn_id")),
                    "concept_name": name1,
                },
                "source_drug_2": {
                    "rxnorm_id": clean_text(row.get("drug_2_rxnorm_id")),
                    "concept_name": name2,
                },
            }
            evidence_id = f"nsides-twosides-evidence:{stable_id(pair_key, row_index, row.get('condition_meddra_id'))}"
            evidence = {
                "schema_version": 1,
                "nsides_evidence_id": evidence_id,
                "source_issue": 1257,
                "pair_key": pair_key,
                "drug_a": pair["drug_a"],
                "drug_b": pair["drug_b"],
                "source_table": "TWOSIDES",
                "source_path": str(path),
                "source_sha256": source_sha,
                "source_row_index": row_index,
                "source_row_sha256": row_sha(row),
                "condition_meddra_id": clean_text(row.get("condition_meddra_id")),
                "condition_concept_name": clean_text(row.get("condition_concept_name")),
                "A": clean_text(row.get("A")),
                "B": clean_text(row.get("B")),
                "C": clean_text(row.get("C")),
                "D": clean_text(row.get("D")),
                "PRR": clean_text(row.get("PRR")),
                "PRR_error": clean_text(row.get("PRR_error")),
                "mean_reporting_frequency": clean_text(row.get("mean_reporting_frequency")),
                "match": match,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "twosides_source_row_contains_both_pair_drugs",
                    "twosides_adverse_effect_source_mining_not_safety_clearance",
                    "requires_external_safety_outcome_falsification_and_human_review",
                ],
            }
            evidence_rows.append(evidence)
            counts_by_pair[pair_key] += 1
    stats = {
        "rows": row_count,
        "header": header,
        "header_sha256": header_fingerprint(header),
    }
    evidence_rows.sort(
        key=lambda row: (
            row["pair_key"],
            row["condition_meddra_id"],
            row["source_row_index"],
            row["nsides_evidence_id"],
        )
    )
    return evidence_rows, dict(counts_by_pair), stats


def scan_offsides(
    path: Path,
    pairs_by_key: dict[str, dict[str, Any]],
    term_index: dict[str, list[tuple[str, str]]],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, int]], dict[str, Any]]:
    header = csv_gzip_header(path)
    if header != EXPECTED_HEADERS["offsides"]:
        raise SystemExit(f"unexpected OffSIDES header: {header}")
    source_sha = sha256_path(path)
    context_counts: dict[str, dict[str, int]] = defaultdict(lambda: {"a": 0, "b": 0})
    top_by_pair_role: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    row_count = 0
    for row_index, row in enumerate(csv_gzip_rows(path), start=1):
        row_count = row_index
        source_name = clean_text(row.get("drug_concept_name"))
        entries = source_drug_entries(term_index, source_name)
        if not entries:
            continue
        for pair_key, role, _key in entries:
            pair = pairs_by_key[pair_key]
            context_counts[pair_key][role] += 1
            row["_source_row_index"] = str(row_index)
            context = {
                "schema_version": 1,
                "nsides_offsides_context_id": f"nsides-offsides-context:{stable_id(pair_key, role, row_index, row.get('condition_meddra_id'))}",
                "source_issue": 1257,
                "pair_key": pair_key,
                "role": role,
                "candidate_drug_name": pair["drug_a"] if role == "a" else pair["drug_b"],
                "source_table": "OFFSIDES",
                "source_path": str(path),
                "source_sha256": source_sha,
                "source_row_index": row_index,
                "source_row_sha256": row_sha(row),
                "source_drug": {
                    "rxnorm_id": clean_text(row.get("drug_rxnorn_id")),
                    "concept_name": source_name,
                },
                "condition_meddra_id": clean_text(row.get("condition_meddra_id")),
                "condition_concept_name": clean_text(row.get("condition_concept_name")),
                "A": clean_text(row.get("A")),
                "B": clean_text(row.get("B")),
                "C": clean_text(row.get("C")),
                "D": clean_text(row.get("D")),
                "PRR": clean_text(row.get("PRR")),
                "PRR_error": clean_text(row.get("PRR_error")),
                "mean_reporting_frequency": clean_text(row.get("mean_reporting_frequency")),
                "match": source_name_match(pair["drug_a"] if role == "a" else pair["drug_b"], source_name),
                "evidence_kind": OFFSIDES_CONTEXT_KIND,
                "promotion_status": PROMOTION_STATUS,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "reason_codes": [
                    "offsides_single_drug_adverse_effect_context_only",
                    "not_pair_interaction_proof",
                    "requires_external_safety_outcome_falsification_and_human_review",
                ],
            }
            top_context_insert(top_by_pair_role[(pair_key, role)], context)
    context_rows = [
        row
        for key in sorted(top_by_pair_role)
        for row in sorted(
            top_by_pair_role[key],
            key=lambda item: (item["pair_key"], item["role"], item["condition_meddra_id"], item["source_row_index"]),
        )
    ]
    stats = {
        "rows": row_count,
        "header": header,
        "header_sha256": header_fingerprint(header),
    }
    return context_rows, {key: dict(value) for key, value in context_counts.items()}, stats


def source_inventory(
    raw_docs: dict[str, Any],
    archives: dict[str, Any],
    twosides_stats: dict[str, Any],
    offsides_stats: dict[str, Any],
) -> dict[str, Any]:
    return {
        "raw_docs": raw_docs,
        "archives": archives,
        "tables": {
            "TWOSIDES": {
                "path": archives["twosides"]["path"],
                "bytes": archives["twosides"]["bytes"],
                "sha256": archives["twosides"]["sha256"],
                "rows": twosides_stats["rows"],
                "header": twosides_stats["header"],
                "header_sha256": twosides_stats["header_sha256"],
            },
            "OFFSIDES": {
                "path": archives["offsides"]["path"],
                "bytes": archives["offsides"]["bytes"],
                "sha256": archives["offsides"]["sha256"],
                "rows": offsides_stats["rows"],
                "header": offsides_stats["header"],
                "header_sha256": offsides_stats["header_sha256"],
            },
        },
    }


def source_rows_from_inventory(inventory: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for group_name, group in inventory.items():
        if group_name == "tables":
            for name, info in sorted(group.items()):
                rows.append(
                    {
                        "schema_version": 1,
                        "source_row_id": f"nsides-source:{group_name}:{name}",
                        "source_issue": 1257,
                        "source_group": group_name,
                        "source_name": name,
                        "rows": info.get("rows"),
                        "bytes": info["bytes"],
                        "sha256": info["sha256"],
                        "path": info["path"],
                        "header": info.get("header"),
                        "header_sha256": info.get("header_sha256"),
                        "text": (
                            f"nSIDES source {group_name}/{name} rows {info.get('rows')} "
                            f"bytes {info['bytes']} sha256 {info['sha256']} header {info.get('header_sha256')}"
                        ),
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
        else:
            for name, info in sorted(group.items()):
                rows.append(
                    {
                        "schema_version": 1,
                        "source_row_id": f"nsides-source:{group_name}:{name}",
                        "source_issue": 1257,
                        "source_group": group_name,
                        "source_name": name,
                        "rows": None,
                        "bytes": info["bytes"],
                        "sha256": info["sha256"],
                        "path": info["path"],
                        "url": info.get("url"),
                        "text": (
                            f"nSIDES source {group_name}/{name} bytes {info['bytes']} "
                            f"sha256 {info['sha256']} url {info.get('url')}"
                        ),
                        "clinical_boundary": CLINICAL_BOUNDARY,
                    }
                )
    rows.sort(key=lambda row: (row["source_group"], row["source_name"]))
    return rows


def build_pair_status(
    pair: dict[str, Any],
    twosides_rows: list[dict[str, Any]],
    context_counts: dict[str, int],
) -> dict[str, Any]:
    evidence_ids = [row["nsides_evidence_id"] for row in twosides_rows]
    if evidence_ids:
        status = "nsides_twosides_pair_adverse_effect_hit_still_blocked"
        reason = "twosides_source_pair_adverse_effect_hit"
    elif context_counts.get("a", 0) or context_counts.get("b", 0):
        status = "nsides_offsides_single_drug_context_without_pair_hit_still_blocked"
        reason = "offsides_single_drug_context_without_twosides_pair_hit"
    else:
        status = "nsides_no_source_name_mapping_still_blocked"
        reason = "nsides_no_source_name_mapping_for_pair"
    return {
        "schema_version": 1,
        "nsides_pair_status_id": f"nsides-pair-status:{stable_id(pair['pair_key'], status)}",
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "representative_pair_id": pair.get("representative_pair_id"),
        "source_pair_ids": pair.get("source_pair_ids", []),
        "source_pharmgkb_candidate_status_ids": pair.get("source_pharmgkb_candidate_status_ids", []),
        "source_pharmgkb_pair_status_ids": pair.get("source_pharmgkb_pair_status_ids", []),
        "nsides_pair_status": status,
        "twosides_evidence_rows": len(evidence_ids),
        "twosides_evidence_ids": evidence_ids,
        "offsides_drug_a_context_rows": context_counts.get("a", 0),
        "offsides_drug_b_context_rows": context_counts.get("b", 0),
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "offsides_context_kind": OFFSIDES_CONTEXT_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "nsides_source_mining_not_safety_clearance",
            "requires_external_safety_outcome_falsification_and_human_review",
            reason,
        ],
    }


def build_candidate_status(candidate: dict[str, Any], pair_status: dict[str, Any]) -> dict[str, Any]:
    pair_to_candidate = {
        "nsides_twosides_pair_adverse_effect_hit_still_blocked": (
            "nsides_candidate_twosides_pair_adverse_effect_hit_still_blocked"
        ),
        "nsides_offsides_single_drug_context_without_pair_hit_still_blocked": (
            "nsides_candidate_offsides_single_drug_context_without_pair_hit_still_blocked"
        ),
        "nsides_no_source_name_mapping_still_blocked": "nsides_candidate_no_source_name_mapping_still_blocked",
    }
    status = pair_to_candidate[pair_status["nsides_pair_status"]]
    return {
        "schema_version": 1,
        "nsides_candidate_status_id": f"nsides-candidate-status:{stable_id(candidate['pair_id'], pair_status['nsides_pair_status'])}",
        "pair_id": candidate["pair_id"],
        "pair_key": candidate["pair_key"],
        "drug_a": candidate["drug_a"],
        "drug_b": candidate["drug_b"],
        "source_pharmgkb_candidate_status_id": candidate.get("pharmgkb_candidate_status_id"),
        "source_pharmgkb_pair_status_id": candidate.get("pharmgkb_pair_status_id"),
        "source_pharmgkb_candidate_status": candidate.get("pharmgkb_candidate_status"),
        "nsides_pair_status_id": pair_status["nsides_pair_status_id"],
        "nsides_candidate_status": status,
        "nsides_pair_status": pair_status["nsides_pair_status"],
        "twosides_evidence_rows": pair_status["twosides_evidence_rows"],
        "twosides_evidence_ids": pair_status["twosides_evidence_ids"],
        "offsides_drug_a_context_rows": pair_status["offsides_drug_a_context_rows"],
        "offsides_drug_b_context_rows": pair_status["offsides_drug_b_context_rows"],
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "offsides_context_kind": OFFSIDES_CONTEXT_KIND,
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
    candidate_status: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in source_rows:
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "nsides_source_snapshot",
                "text": row["text"],
                "bridge_terms": uniq(["nSIDES", row["source_group"], row["source_name"], row["sha256"]]),
                "metadata": {
                    "source_dataset": "issue1257_nsides_source_mining",
                    "source_path": row["path"],
                    "source_sha256": row["sha256"],
                    "source_group": row["source_group"],
                    "source_name": row["source_name"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in candidate_status:
        text = (
            f"nSIDES candidate status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['nsides_candidate_status']} "
            f"TwoSIDES evidence rows {row['twosides_evidence_rows']} "
            f"OffSIDES context {row['offsides_drug_a_context_rows']} and {row['offsides_drug_b_context_rows']} "
            f"promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["nsides_candidate_status_id"],
                "domain": "nsides_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["nsides_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1257_nsides_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "nsides_candidate_status": row["nsides_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        text = (
            f"nSIDES pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['nsides_pair_status']} TwoSIDES evidence rows {row['twosides_evidence_rows']} "
            f"OffSIDES context rows {row['offsides_drug_a_context_rows']} and {row['offsides_drug_b_context_rows']}."
        )
        rows.append(
            {
                "id": row["nsides_pair_status_id"],
                "domain": "nsides_pair_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["nsides_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1257_nsides_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "nsides_pair_status": row["nsides_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    evidence_sample = sorted(
        twosides_evidence,
        key=lambda row: (row["pair_key"], row["condition_meddra_id"], row["source_row_index"]),
    )[:MAX_BRIDGE_EVIDENCE_ROWS]
    for row in evidence_sample:
        text = (
            f"TwoSIDES source evidence {row['nsides_evidence_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} adverse effect {row['condition_concept_name']} "
            f"PRR {row['PRR']} still blocked."
        )
        rows.append(
            {
                "id": row["nsides_evidence_id"],
                "domain": "nsides_twosides_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["condition_concept_name"]]),
                "metadata": {
                    "source_dataset": "issue1257_nsides_source_mining",
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
    for row in offsides_context[: max(0, remaining)]:
        text = (
            f"OffSIDES single-drug context {row['nsides_offsides_context_id']} pair {row['pair_key']} "
            f"role {row['role']} drug {row['candidate_drug_name']} adverse effect {row['condition_concept_name']} "
            f"PRR {row['PRR']} context only."
        )
        rows.append(
            {
                "id": row["nsides_offsides_context_id"],
                "domain": "nsides_offsides_context",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["candidate_drug_name"], row["condition_concept_name"]]),
                "metadata": {
                    "source_dataset": "issue1257_nsides_source_mining",
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
    if len(rows) > MAX_BRIDGE_ROWS:
        rows = rows[:MAX_BRIDGE_ROWS]
    return rows


def build_input_manifest(
    inputs: dict[str, str],
    issue1256_persisted_readback: dict[str, Any],
    issue1256_calyx_readback: dict[str, Any],
    source_inventory_payload: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1257,
        "created_utc": now_utc(),
        "inputs": {
            key: artifact(Path(path), jsonl=path.endswith(".jsonl"))
            for key, path in inputs.items()
        },
        "source_inventory": source_inventory_payload,
        "source_contract": {
            "issue1256_persisted_assertions_all_true": all_assertions_true(issue1256_persisted_readback),
            "issue1256_calyx_assertions_all_true": all_assertions_true(issue1256_calyx_readback),
            "clinical_boundary": CLINICAL_BOUNDARY,
            "twosides_evidence_gate": "one TwoSIDES row with both pair drugs matched by normalized or salt-stripped name keys",
            "offsides_context_gate": "single-drug adverse-effect context only; never pair proof",
            "promotion_policy": PROMOTION_STATUS,
        },
    }


def build_metrics(
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    inventory: dict[str, Any],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
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
        "source_table_rows": {
            "TWOSIDES": inventory["tables"]["TWOSIDES"]["rows"],
            "OFFSIDES": inventory["tables"]["OFFSIDES"]["rows"],
        },
        "twosides_evidence_rows": len(twosides_evidence),
        "offsides_context_sample_rows": len(offsides_context),
        "pair_status_rows": len(pair_status),
        "candidate_status_rows": len(candidate_status),
        "bridge_rows": len(bridge_rows),
        "bridge_twosides_evidence_rows_materialized": sum(
            1 for row in bridge_rows if row["domain"] == "nsides_twosides_evidence"
        ),
        "bridge_offsides_context_rows_materialized": sum(
            1 for row in bridge_rows if row["domain"] == "nsides_offsides_context"
        ),
        "pair_status_counts": dict(Counter(row["nsides_pair_status"] for row in pair_status)),
        "candidate_status_counts": dict(Counter(row["nsides_candidate_status"] for row in candidate_status)),
        "evidence_condition_counts_top20": dict(
            Counter(row["condition_concept_name"] for row in twosides_evidence).most_common(20)
        ),
        "all_rows_blocked": all(
            row.get("promotion_status") == PROMOTION_STATUS
            for row in pair_status + candidate_status + twosides_evidence + offsides_context
        ),
    }


def build_persisted_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    twosides_evidence: list[dict[str, Any]],
    offsides_context: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1256_persisted_readback: dict[str, Any],
    issue1256_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "nsides_source_rows": artifact(out_dir / "nsides_source_rows.jsonl", jsonl=True),
        "twosides_pair_evidence": artifact(out_dir / "twosides_pair_evidence.jsonl", jsonl=True),
        "offsides_single_drug_context": artifact(out_dir / "offsides_single_drug_context.jsonl", jsonl=True),
        "nsides_pair_status": artifact(out_dir / "nsides_pair_status.jsonl", jsonl=True),
        "candidate_nsides_status": artifact(out_dir / "candidate_nsides_status.jsonl", jsonl=True),
        "nsides_bridge_rows": artifact(out_dir / "nsides_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    candidate_pair_keys = {row["pair_key"] for row in candidates}
    pair_status_keys = {row["pair_key"] for row in pair_status}
    status_candidate_ids = {row["pair_id"] for row in candidate_status}
    candidate_ids = {row["pair_id"] for row in candidates}
    evidence_by_pair = Counter(row["pair_key"] for row in twosides_evidence)
    assertions = {
        "issue1256_persisted_readback_all_true": all_assertions_true(issue1256_persisted_readback),
        "issue1256_calyx_readback_all_true": all_assertions_true(issue1256_calyx_readback),
        "source_rows_present": len(source_rows) >= len(RAW_DOC_URLS) + len(ARCHIVE_URLS) + 2,
        "pair_status_for_every_pair_key": pair_status_keys == candidate_pair_keys,
        "candidate_status_for_every_candidate": status_candidate_ids == candidate_ids,
        "all_twosides_hits_have_evidence": all(
            row["nsides_pair_status"] != "nsides_twosides_pair_adverse_effect_hit_still_blocked"
            or evidence_by_pair[row["pair_key"]] > 0
            for row in pair_status
        ),
        "all_evidence_rows_have_both_matches": all(
            row.get("match", {}).get("drug_a", {}).get("matched")
            and row.get("match", {}).get("drug_b", {}).get("matched")
            for row in twosides_evidence
        ),
        "all_evidence_rows_have_source_hash": all(row.get("source_sha256") for row in twosides_evidence),
        "all_context_rows_single_drug_only": all(
            row.get("evidence_kind") == OFFSIDES_CONTEXT_KIND for row in offsides_context
        ),
        "all_pair_status_values_allowed": all(row["nsides_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
        "all_candidate_status_values_allowed": all(
            row["nsides_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
        ),
        "all_status_rows_have_boundary": all(row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in pair_status + candidate_status),
        "all_evidence_rows_have_boundary": all(
            row.get("clinical_boundary") == CLINICAL_BOUNDARY for row in twosides_evidence + offsides_context
        ),
        "all_rows_remain_blocked": all(
            row.get("promotion_status") == PROMOTION_STATUS
            for row in pair_status + candidate_status + twosides_evidence + offsides_context
        ),
        "bridge_rows_1000_or_less": len(bridge_rows) <= MAX_BRIDGE_ROWS,
    }
    return {
        "schema_version": 1,
        "issue": 1257,
        "status": "ok" if all(assertions.values()) else "failed",
        "created_utc": now_utc(),
        "counts": {
            "source_rows": len(source_rows),
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

    issue1256_persisted_readback = read_json(Path(inputs["issue1256_persisted_readback"]))
    issue1256_calyx_readback = read_json(Path(inputs["issue1256_calyx_readback"]))
    candidate_input_rows = read_jsonl(Path(inputs["issue1256_candidate_status"]))
    candidates = load_candidates(candidate_input_rows, max_pairs=max_pairs)
    pairs = pair_rows(candidates)
    pairs_by_key = {row["pair_key"]: row for row in pairs}
    term_index = build_term_index(pairs)

    raw_docs = fetch_raw_docs(raw_dir)
    archives = fetch_archives(raw_dir)

    twosides_path = Path(archives["twosides"]["path"])
    offsides_path = Path(archives["offsides"]["path"])
    twosides_evidence, twosides_counts_by_pair, twosides_stats = scan_twosides(
        twosides_path,
        pairs_by_key,
        term_index,
    )
    offsides_context, offsides_context_counts, offsides_stats = scan_offsides(
        offsides_path,
        pairs_by_key,
        term_index,
    )

    inventory = source_inventory(raw_docs, archives, twosides_stats, offsides_stats)
    source_rows = source_rows_from_inventory(inventory)
    write_jsonl(out_dir / "nsides_source_rows.jsonl", source_rows)
    write_jsonl(out_dir / "twosides_pair_evidence.jsonl", twosides_evidence)
    write_jsonl(out_dir / "offsides_single_drug_context.jsonl", offsides_context)

    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in twosides_evidence:
        evidence_by_pair[row["pair_key"]].append(row)

    pair_status_rows = [
        build_pair_status(
            pair,
            evidence_by_pair[pair["pair_key"]],
            offsides_context_counts.get(pair["pair_key"], {"a": 0, "b": 0}),
        )
        for pair in pairs
    ]
    pair_status_by_key = {row["pair_key"]: row for row in pair_status_rows}
    candidate_status_rows = [build_candidate_status(row, pair_status_by_key[row["pair_key"]]) for row in candidates]

    write_jsonl(out_dir / "nsides_pair_status.jsonl", pair_status_rows)
    write_jsonl(out_dir / "candidate_nsides_status.jsonl", candidate_status_rows)

    bridge_rows = build_bridge_rows(
        source_rows,
        candidate_status_rows,
        pair_status_rows,
        twosides_evidence,
        offsides_context,
        out_dir / "candidate_nsides_status.jsonl",
        sha256_path(out_dir / "candidate_nsides_status.jsonl"),
    )
    write_jsonl(out_dir / "nsides_bridge_rows.jsonl", bridge_rows)

    input_manifest = build_input_manifest(
        inputs,
        issue1256_persisted_readback,
        issue1256_calyx_readback,
        inventory,
    )
    write_json(out_dir / "input_manifest.json", input_manifest)

    metrics = build_metrics(
        candidates,
        pairs,
        inventory,
        twosides_evidence,
        offsides_context,
        pair_status_rows,
        candidate_status_rows,
        bridge_rows,
    )
    metrics["twosides_pair_hit_counts"] = dict(twosides_counts_by_pair)
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1257,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "nsides_source_rows": artifact(out_dir / "nsides_source_rows.jsonl", jsonl=True),
            "twosides_pair_evidence": artifact(out_dir / "twosides_pair_evidence.jsonl", jsonl=True),
            "offsides_single_drug_context": artifact(out_dir / "offsides_single_drug_context.jsonl", jsonl=True),
            "nsides_pair_status": artifact(out_dir / "nsides_pair_status.jsonl", jsonl=True),
            "candidate_nsides_status": artifact(out_dir / "candidate_nsides_status.jsonl", jsonl=True),
            "nsides_bridge_rows": artifact(out_dir / "nsides_bridge_rows.jsonl", jsonl=True),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)

    persisted_readback = build_persisted_readback(
        out_dir,
        candidates,
        pairs,
        source_rows,
        twosides_evidence,
        offsides_context,
        pair_status_rows,
        candidate_status_rows,
        bridge_rows,
        issue1256_persisted_readback,
        issue1256_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", persisted_readback)

    final = {
        "status": persisted_readback["status"],
        "root": str(root),
        "artifacts": {
            "source_rows": artifact(out_dir / "nsides_source_rows.jsonl", jsonl=True),
            "twosides_pair_evidence": artifact(out_dir / "twosides_pair_evidence.jsonl", jsonl=True),
            "offsides_single_drug_context": artifact(out_dir / "offsides_single_drug_context.jsonl", jsonl=True),
            "pair_status": artifact(out_dir / "nsides_pair_status.jsonl", jsonl=True),
            "candidate_status": artifact(out_dir / "candidate_nsides_status.jsonl", jsonl=True),
            "bridge_rows": artifact(out_dir / "nsides_bridge_rows.jsonl", jsonl=True),
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
