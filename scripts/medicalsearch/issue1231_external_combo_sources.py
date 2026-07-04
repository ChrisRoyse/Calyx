#!/usr/bin/env python3
"""#1231 external drug-combination source expansion.

This stage ingests an additional open drug-combination evidence source, CDCDB,
and re-evaluates the #1190/#1229 candidate-pair universe. The output is
association evidence for research triage only. It is not efficacy, safety,
clinical actionability, treatment guidance, dosing, recommendation, or cure
evidence.
"""

from __future__ import annotations

import argparse
import ast
import csv
import hashlib
import io
import itertools
import json
import re
import time
from collections import Counter
from pathlib import Path
from typing import Any
from zipfile import ZipFile


CLINICAL_BOUNDARY = (
    "External combination-source evidence is research triage only; not "
    "efficacy, safety, clinical actionability, treatment guidance, dosing, "
    "recommendation, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "source_attributed_combination_documentation_not_synergy_or_safety_clearance"
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z"

DEFAULT_INPUTS = {
    "cdcdb_zip": f"{DEFAULT_ROOT}/raw/cdcdb_2022_04_12.zip",
    "figshare_article": f"{DEFAULT_ROOT}/raw/figshare_article.json",
    "candidate_pairs": "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/candidate_pair_inputs.jsonl",
    "prior_external_status": "/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/candidate_external_synergy_status.jsonl",
}

CDCDB_FIGSHARE_PAGE = (
    "https://springernature.figshare.com/articles/dataset/"
    "CSV_version_of_CDCDB_from_12_4_2022/19582069"
)
CDCDB_FIGSHARE_API = "https://api.figshare.com/v2/articles/19582069"
CDCDB_DOWNLOAD_URL = "https://ndownloader.figshare.com/files/34785670"
CDCDB_DATA_DESCRIPTOR = "https://www.nature.com/articles/s41597-023-02303-8"
CDCDB_LICENSE = "CC0"
CDCDB_EXPECTED_MD5 = "2af17e658987b6c32b3d95f3a7c5ed7e"


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def md5_path(path: Path) -> str:
    h = hashlib.md5()
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


def parse_literal_list(value: object) -> list[Any]:
    text = clean_text(value)
    if not text:
        return []
    try:
        parsed = ast.literal_eval(text)
    except (SyntaxError, ValueError):
        return []
    return parsed if isinstance(parsed, list) else []


def valid_norm(value: str) -> bool:
    return bool(value and value not in {"na", "nan", "none", "none given", "unknown", "1"})


def alias_names(group: object) -> list[str]:
    values: list[object] = []
    if isinstance(group, (list, tuple)):
        for item in group:
            if isinstance(item, (list, tuple)):
                values.extend(item)
            else:
                values.append(item)
    else:
        values.append(group)
    out: list[str] = []
    seen: set[str] = set()
    for value in values:
        text = clean_text(value).strip(" ;,")
        norm = norm_name(text)
        if not text or not valid_norm(norm) or norm in seen:
            continue
        seen.add(norm)
        out.append(text)
    return out


def identifiers_for_group(raw: object, index: int) -> list[str]:
    values = parse_literal_list(raw)
    if index >= len(values):
        return []
    item = values[index]
    if isinstance(item, (list, tuple)):
        raw_values = list(item)
    else:
        raw_values = [item]
    return uniq([value for value in raw_values if valid_norm(norm_name(value))])


def source_groups(row: dict[str, str]) -> list[dict[str, Any]]:
    groups: list[dict[str, Any]] = []
    for index, group in enumerate(parse_literal_list(row.get("drugs"))):
        aliases = alias_names(group)
        if not aliases:
            continue
        groups.append(
            {
                "primary": aliases[0],
                "aliases": aliases[:12],
                "norm_aliases": [norm_name(alias) for alias in aliases[:12]],
                "drugbank_ids": identifiers_for_group(row.get("drugbank_identifiers"), index),
                "pubchem_ids": identifiers_for_group(row.get("pubchem_identifiers"), index),
            }
        )
    return groups


def csv_schemas(zip_file: ZipFile) -> dict[str, dict[str, Any]]:
    schemas: dict[str, dict[str, Any]] = {}
    for info in zip_file.infolist():
        if not info.filename.endswith(".csv"):
            continue
        with zip_file.open(info.filename) as handle:
            reader = csv.reader(io.TextIOWrapper(handle, encoding="utf-8-sig", errors="replace"))
            header = next(reader)
            rows = sum(1 for _ in reader)
        schemas[info.filename] = {
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
            "source_type_counts": Counter(),
            "two_drug_record_count": 0,
            "multi_drug_context_count": 0,
            "examples": [],
        },
    )
    entry["source_record_count"] += 1
    entry["source_type_counts"].update([example["source"]])
    if example["source_combination_size"] == 2:
        entry["two_drug_record_count"] += 1
    else:
        entry["multi_drug_context_count"] += 1
    if len(entry["examples"]) < 8:
        entry["examples"].append(example)


def build_cdcdb_indexes(zip_path: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]], dict[str, dict[str, Any]], dict[str, Any]]:
    source_rows: list[dict[str, Any]] = []
    pair_index: dict[str, dict[str, Any]] = {}
    with ZipFile(zip_path) as zip_file:
        schemas = csv_schemas(zip_file)
        with zip_file.open("all_combs_unormalized.csv") as handle:
            reader = csv.DictReader(io.TextIOWrapper(handle, encoding="utf-8-sig", errors="replace"))
            for raw_index, row in enumerate(reader, start=1):
                groups = source_groups(row)
                if len(groups) < 2:
                    continue
                combo_id = f"cdcdb-combo:{stable_id(row.get('source'), row.get('source_id'), raw_index)}"
                source_row = {
                    "schema_version": 1,
                    "combo_id": combo_id,
                    "source_row": raw_index,
                    "source": clean_text(row.get("source")),
                    "source_id": clean_text(row.get("source_id")),
                    "source_combination_size": len(groups),
                    "drug_groups": groups,
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
                source_rows.append(source_row)
                for left, right in itertools.combinations(groups, 2):
                    keys = {
                        pair_key(left_alias, right_alias)
                        for left_alias in left["aliases"]
                        for right_alias in right["aliases"]
                    }
                    example = {
                        "combo_id": combo_id,
                        "source": source_row["source"],
                        "source_id": source_row["source_id"],
                        "source_combination_size": len(groups),
                        "left_primary": left["primary"],
                        "right_primary": right["primary"],
                        "left_aliases": left["aliases"],
                        "right_aliases": right["aliases"],
                        "left_drugbank_ids": left["drugbank_ids"],
                        "right_drugbank_ids": right["drugbank_ids"],
                        "left_pubchem_ids": left["pubchem_ids"],
                        "right_pubchem_ids": right["pubchem_ids"],
                    }
                    for key in keys:
                        append_pair_hit(pair_index, key, example)

    pair_rows: list[dict[str, Any]] = []
    for key, entry in pair_index.items():
        left_norm, right_norm = key.split("||", 1)
        examples = entry["examples"]
        source_counts = dict(sorted(entry["source_type_counts"].items()))
        pair_rows.append(
            {
                "schema_version": 1,
                "pair_id": f"cdcdb:{stable_id(key)}",
                "pair_key": key,
                "drug_a_norm": left_norm,
                "drug_b_norm": right_norm,
                "display_drug_a": examples[0]["left_primary"] if examples else left_norm,
                "display_drug_b": examples[0]["right_primary"] if examples else right_norm,
                "source_record_count": entry["source_record_count"],
                "source_type_counts": source_counts,
                "source_types": sorted(source_counts),
                "two_drug_record_count": entry["two_drug_record_count"],
                "multi_drug_context_count": entry["multi_drug_context_count"],
                "examples": examples,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    pair_rows.sort(key=lambda row: (-row["source_record_count"], row["pair_key"]))
    schema_fingerprint = sha256_bytes(json.dumps(schemas, sort_keys=True).encode("utf-8"))
    return source_rows, pair_rows, {row["pair_key"]: row for row in pair_rows}, {
        "schema_version": 1,
        "source": "CDCDB",
        "schemas": schemas,
        "schema_fingerprint_sha256": schema_fingerprint,
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
        for left in example.get("left_aliases") or []:
            for right in example.get("right_aliases") or []:
                source_names = {clean_text(left).lower(), clean_text(right).lower()}
                if source_names == candidate_names:
                    return True
    return False


def cdcdb_status(candidate: dict[str, Any], source: dict[str, Any] | None) -> str:
    if candidate_exact_match(candidate, source):
        return "exact_hit"
    if source:
        return "normalized_hit"
    return "no_external_hit"


def merge_external_status(prior_status: str, new_status: str) -> str:
    if "exact_hit" in {prior_status, new_status}:
        return "exact_hit"
    if "normalized_hit" in {prior_status, new_status}:
        return "normalized_hit"
    return "no_external_hit"


def next_validation(row: dict[str, Any], status: str) -> str:
    if status == "no_external_hit":
        return "Acquire additional open, source-attributed combination evidence before any promotion."
    if "component_safety_missing_fail_closed" in row.get("reason_codes", []):
        return "Complete component safety evidence; CDCDB source evidence does not clear safety."
    if "pair_interaction_evidence_missing_fail_closed" in row.get("reason_codes", []):
        return "Acquire exact pair interaction/pharmacology evidence; CDCDB does not clear interaction gates."
    return "Human reviewer may inspect CDCDB support, then require independent outcome and safety gates."


def recheck_candidates(prior_rows: list[dict[str, Any]], pair_index: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    joined: list[dict[str, Any]] = []
    for prior in prior_rows:
        source = pair_index.get(prior["pair_key"])
        status = cdcdb_status(prior, source)
        prior_status = clean_text(prior.get("external_synergy_status"))
        overall_status = merge_external_status(prior_status, status)
        reason_codes = list(prior.get("reason_codes") or [])
        if status != "no_external_hit":
            if "external_combo_source_hit_not_clearance" not in reason_codes:
                reason_codes.append("external_combo_source_hit_not_clearance")
        elif prior_status == "no_external_hit":
            if "external_combo_source_missing_fail_closed" not in reason_codes:
                reason_codes.append("external_combo_source_missing_fail_closed")
        if status != "no_external_hit":
            combination_status = "external_combination_documented_still_blocked"
        elif prior_status != "no_external_hit":
            combination_status = clean_text(prior.get("combination_status"))
        else:
            combination_status = "blocked_no_external_combination_evidence"
        joined.append(
            {
                "schema_version": 1,
                "pair_id": prior["pair_id"],
                "pair_key": prior["pair_key"],
                "drug_a": prior["drug_a"],
                "drug_b": prior["drug_b"],
                "disease": prior.get("disease"),
                "disease_area": prior.get("disease_area"),
                "prior_issue1229_external_synergy_status": prior_status,
                "prior_issue1229_almanac_match": bool(prior.get("almanac_match")),
                "prior_issue1229_drugcomb_match": bool(prior.get("drugcomb_match")),
                "cdcdb_external_combo_status": status,
                "cdcdb_match": status != "no_external_hit",
                "cdcdb_summary": source,
                "cdcdb_source_types": source.get("source_types") if source else [],
                "overall_external_evidence_status": overall_status,
                "combination_status": combination_status,
                "reason_codes": reason_codes,
                "evidence_kind": SOURCE_EVIDENCE_KIND,
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": next_validation(prior, status),
            }
        )
    joined.sort(
        key=lambda row: (
            0
            if row["prior_issue1229_external_synergy_status"] == "no_external_hit"
            and row["cdcdb_external_combo_status"] != "no_external_hit"
            else 1,
            0 if row["cdcdb_external_combo_status"] != "no_external_hit" else 1,
            row.get("disease_area") or "",
            row["pair_key"],
        )
    )
    return joined


def build_metrics(
    source_rows: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    prior_rows: list[dict[str, Any]],
    joined: list[dict[str, Any]],
) -> dict[str, Any]:
    prior_no_hit = [row for row in joined if row["prior_issue1229_external_synergy_status"] == "no_external_hit"]
    prior_no_hit_with_cdcdb = [row for row in prior_no_hit if row["cdcdb_external_combo_status"] != "no_external_hit"]
    status_counts = Counter(row["cdcdb_external_combo_status"] for row in joined)
    overall_counts = Counter(row["overall_external_evidence_status"] for row in joined)
    source_combo_types = Counter(row["source"] for row in source_rows)
    pair_key_source_types = Counter(source for row in pair_rows for source in row["source_types"])
    reason_counts = Counter(reason for row in joined for reason in row["reason_codes"])
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "cdcdb_source_combo_rows": len(source_rows),
        "cdcdb_unique_pair_keys": len(pair_rows),
        "cdcdb_source_combo_type_counts": dict(sorted(source_combo_types.items())),
        "cdcdb_pair_key_source_type_counts": dict(sorted(pair_key_source_types.items())),
        "candidate_rows": len(joined),
        "prior_issue1229_no_hit_rows": len(prior_no_hit),
        "candidate_rows_with_cdcdb_hit": sum(1 for row in joined if row["cdcdb_external_combo_status"] != "no_external_hit"),
        "prior_no_hit_rows_with_cdcdb_hit": len(prior_no_hit_with_cdcdb),
        "remaining_prior_no_hit_rows_after_cdcdb": len(prior_no_hit) - len(prior_no_hit_with_cdcdb),
        "candidate_rows_with_any_external_evidence_after_cdcdb": sum(
            1 for row in joined if row["overall_external_evidence_status"] != "no_external_hit"
        ),
        "cdcdb_status_counts": dict(status_counts),
        "overall_external_evidence_status_counts": dict(overall_counts),
        "reason_code_counts": dict(reason_counts),
        "clinical_boundary_rows": sum(1 for row in joined if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_prior_no_hit_cdcdb_hits": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "disease": row.get("disease"),
                "cdcdb_external_combo_status": row["cdcdb_external_combo_status"],
                "source_types": row["cdcdb_source_types"],
                "source_record_count": row["cdcdb_summary"]["source_record_count"] if row["cdcdb_summary"] else 0,
            }
            for row in prior_no_hit_with_cdcdb[:25]
        ],
    }


def build_bridge_rows(rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows[:1000]:
        terms = uniq(
            [
                row["drug_a"],
                row["drug_b"],
                row.get("disease"),
                row.get("disease_area"),
                row["cdcdb_external_combo_status"],
                row["overall_external_evidence_status"],
                *row.get("cdcdb_source_types", []),
            ]
        )
        source_types = ", ".join(row.get("cdcdb_source_types") or ["none"])
        text = (
            f"External combination source recheck {row['pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} for {row.get('disease')} has CDCDB status "
            f"{row['cdcdb_external_combo_status']} from {source_types}; prior "
            f"ALMANAC/DrugComb status was {row['prior_issue1229_external_synergy_status']}; "
            f"overall external evidence status is {row['overall_external_evidence_status']} "
            f"and remains {row['combination_status']}."
        )
        out.append(
            {
                "id": row["pair_id"],
                "domain": "external_drug_combination_source_recheck",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1231_cdcdb_external_combo_sources",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "cdcdb_external_combo_status": row["cdcdb_external_combo_status"],
                    "overall_external_evidence_status": row["overall_external_evidence_status"],
                    "combination_status": row["combination_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def input_manifest(inputs: dict[str, str], source_schema: dict[str, Any]) -> dict[str, Any]:
    cdcdb_zip = Path(inputs["cdcdb_zip"])
    figshare_article = Path(inputs["figshare_article"])
    figshare_data = json.loads(figshare_article.read_text(encoding="utf-8"))
    figshare_file = next(
        (
            file
            for file in figshare_data.get("files", [])
            if str(file.get("id")) == "34785670" or file.get("name") == "12.04.2022.zip"
        ),
        {},
    )
    zip_artifact = artifact(cdcdb_zip)
    zip_artifact["md5"] = md5_path(cdcdb_zip)
    zip_artifact["md5_matches_figshare"] = zip_artifact["md5"] == CDCDB_EXPECTED_MD5
    zip_artifact["figshare_file_id"] = figshare_file.get("id")
    zip_artifact["figshare_file_name"] = figshare_file.get("name")
    zip_artifact["figshare_size"] = figshare_file.get("size")
    zip_artifact["figshare_supplied_md5"] = figshare_file.get("supplied_md5")
    zip_artifact["figshare_computed_md5"] = figshare_file.get("computed_md5")
    zip_artifact["md5_matches_figshare_api"] = zip_artifact["md5"] in {
        figshare_file.get("supplied_md5"),
        figshare_file.get("computed_md5"),
    }
    zip_artifact["bytes_match_figshare_api"] = zip_artifact["bytes"] == figshare_file.get("size")
    zip_artifact["download_url_matches_figshare_api"] = (
        figshare_file.get("download_url") == CDCDB_DOWNLOAD_URL
    )
    zip_artifact["source_page"] = CDCDB_FIGSHARE_PAGE
    zip_artifact["api_url"] = CDCDB_FIGSHARE_API
    zip_artifact["download_url"] = CDCDB_DOWNLOAD_URL
    zip_artifact["license"] = CDCDB_LICENSE
    artifacts = {
        "cdcdb_zip": zip_artifact,
        "figshare_article": artifact(figshare_article),
        "candidate_pairs": artifact(Path(inputs["candidate_pairs"]), jsonl=True),
        "prior_external_status": artifact(Path(inputs["prior_external_status"]), jsonl=True),
    }
    return {
        "schema_version": 1,
        "issue": 1231,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": artifacts,
        "accepted_sources": [
            {
                "source": "CDCDB CSV snapshot 12.04.2022",
                "role": "source-attributed drug-combination documentation",
                "figshare_page": CDCDB_FIGSHARE_PAGE,
                "api_url": CDCDB_FIGSHARE_API,
                "download_url": CDCDB_DOWNLOAD_URL,
                "data_descriptor": CDCDB_DATA_DESCRIPTOR,
                "license": CDCDB_LICENSE,
                "expected_md5": CDCDB_EXPECTED_MD5,
            }
        ],
        "source_schema": source_schema,
    }


def build_readback(
    out_dir: Path,
    source_rows: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    prior_rows: list[dict[str, Any]],
    joined: list[dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "cdcdb_source_combinations": artifact(out_dir / "cdcdb_source_combinations.jsonl", jsonl=True),
        "cdcdb_pair_index": artifact(out_dir / "cdcdb_pair_index.jsonl", jsonl=True),
        "candidate_external_combo_status": artifact(out_dir / "candidate_external_combo_status.jsonl", jsonl=True),
        "prior_no_hit_recheck_status": artifact(out_dir / "prior_no_hit_recheck_status.jsonl", jsonl=True),
        "candidate_external_combo_hits": artifact(out_dir / "candidate_external_combo_hits.jsonl", jsonl=True),
        "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    prior_no_hit_rows = [row for row in joined if row["prior_issue1229_external_synergy_status"] == "no_external_hit"]
    return {
        "schema_version": 1,
        "issue": 1231,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "cdcdb_source_rows_present": len(source_rows) > 0,
            "cdcdb_pair_keys_present": len(pair_rows) > 0,
            "joined_rows_match_prior_status": len(joined) == len(prior_rows),
            "prior_no_hit_recheck_rows_match_prior_no_hits": artifacts["prior_no_hit_recheck_status"]["rows"] == len(prior_no_hit_rows),
            "deterministic_status_for_every_candidate": all(
                row["cdcdb_external_combo_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                for row in joined
            ),
            "deterministic_status_for_every_prior_no_hit": all(
                row["cdcdb_external_combo_status"] in {"exact_hit", "normalized_hit", "no_external_hit"}
                for row in prior_no_hit_rows
            ),
            "all_joined_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in joined),
            "all_cdcdb_hits_have_summary": all(
                bool(row["cdcdb_summary"]) for row in joined if row["cdcdb_external_combo_status"] != "no_external_hit"
            ),
            "bridge_rows_1000_or_less": artifacts["external_combo_bridge_rows"]["rows"] == min(1000, len(joined)),
        },
        "row_counts": {
            "cdcdb_source_combo_rows": len(source_rows),
            "cdcdb_pair_rows": len(pair_rows),
            "prior_status_rows": len(prior_rows),
            "joined_rows": len(joined),
            "prior_no_hit_rows": len(prior_no_hit_rows),
        },
    }


def run(root: Path, inputs: dict[str, str]) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    source_rows, pair_rows, pair_index, source_schema = build_cdcdb_indexes(Path(inputs["cdcdb_zip"]))
    write_jsonl(out_dir / "cdcdb_source_combinations.jsonl", source_rows)
    write_jsonl(out_dir / "cdcdb_pair_index.jsonl", pair_rows)
    write_json(out_dir / "cdcdb_source_schema.json", source_schema)
    manifest = input_manifest(inputs, source_schema)
    write_json(out_dir / "input_manifest.json", manifest)

    prior_rows = rows_jsonl(Path(inputs["prior_external_status"]))
    candidate_pairs = rows_jsonl(Path(inputs["candidate_pairs"]))
    if len(prior_rows) != len(candidate_pairs):
        raise SystemExit("prior status row count does not match candidate pair row count")
    candidate_pair_ids = {row["pair_id"] for row in candidate_pairs}
    prior_pair_ids = {row["pair_id"] for row in prior_rows}
    if candidate_pair_ids != prior_pair_ids:
        raise SystemExit("prior status pair ids do not match #1190 candidate pair ids")

    joined = recheck_candidates(prior_rows, pair_index)
    prior_no_hit = [
        row for row in joined if row["prior_issue1229_external_synergy_status"] == "no_external_hit"
    ]
    hits = [row for row in joined if row["cdcdb_external_combo_status"] != "no_external_hit"]
    write_jsonl(out_dir / "candidate_external_combo_status.jsonl", joined)
    write_jsonl(out_dir / "prior_no_hit_recheck_status.jsonl", prior_no_hit)
    write_jsonl(out_dir / "candidate_external_combo_hits.jsonl", hits)
    source_path = out_dir / "candidate_external_combo_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(joined, source_path, source_sha)
    write_jsonl(out_dir / "external_combo_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(source_rows, pair_rows, prior_rows, joined)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1231,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "cdcdb_source_schema": artifact(out_dir / "cdcdb_source_schema.json"),
            "cdcdb_source_combinations": artifact(out_dir / "cdcdb_source_combinations.jsonl", jsonl=True),
            "cdcdb_pair_index": artifact(out_dir / "cdcdb_pair_index.jsonl", jsonl=True),
            "candidate_external_combo_status": artifact(out_dir / "candidate_external_combo_status.jsonl", jsonl=True),
            "prior_no_hit_recheck_status": artifact(out_dir / "prior_no_hit_recheck_status.jsonl", jsonl=True),
            "candidate_external_combo_hits": artifact(out_dir / "candidate_external_combo_hits.jsonl", jsonl=True),
            "external_combo_bridge_rows": artifact(out_dir / "external_combo_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, source_rows, pair_rows, prior_rows, joined)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "joined_status": output_manifest["artifacts"]["candidate_external_combo_status"],
            "prior_no_hit_recheck": output_manifest["artifacts"]["prior_no_hit_recheck_status"],
            "hits": output_manifest["artifacts"]["candidate_external_combo_hits"],
            "bridge_rows": output_manifest["artifacts"]["external_combo_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def inputs_for_root(root: Path) -> dict[str, str]:
    inputs = dict(DEFAULT_INPUTS)
    inputs["cdcdb_zip"] = str(root / "raw" / "cdcdb_2022_04_12.zip")
    inputs["figshare_article"] = str(root / "raw" / "figshare_article.json")
    return inputs


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--cdcdb-zip")
    parser.add_argument("--figshare-article")
    parser.add_argument("--candidate-pairs")
    parser.add_argument("--prior-external-status")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    inputs = inputs_for_root(root)
    if args.cdcdb_zip:
        inputs["cdcdb_zip"] = args.cdcdb_zip
    if args.figshare_article:
        inputs["figshare_article"] = args.figshare_article
    if args.candidate_pairs:
        inputs["candidate_pairs"] = args.candidate_pairs
    if args.prior_external_status:
        inputs["prior_external_status"] = args.prior_external_status
    result = run(root, inputs)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
