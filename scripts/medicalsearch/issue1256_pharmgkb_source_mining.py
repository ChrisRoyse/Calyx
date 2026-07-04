#!/usr/bin/env python3
"""#1256 PharmGKB/ClinPGx source mining after DrugCentral no-hit."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import sys
import time
import urllib.request
import zipfile
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


csv.field_size_limit(sys.maxsize)

CLINICAL_BOUNDARY = (
    "PharmGKB source mining is pharmacogenomic/source triage only; clinical "
    "annotation, label, variant, or relationship rows are blockers/review inputs, "
    "not safety clearance, efficacy, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, pair-interaction proof, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "pharmgkb_same_row_two_term_source_match_not_actionability_efficacy_or_cure"
)

ISSUE1255_ROOT = "/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z"

DEFAULT_INPUTS = {
    "issue1255_candidate_status": f"{ISSUE1255_ROOT}/out/candidate_drugcentral_status.jsonl",
    "issue1255_pair_status": f"{ISSUE1255_ROOT}/out/drugcentral_pair_status.jsonl",
    "issue1255_persisted_readback": f"{ISSUE1255_ROOT}/out/persisted_readback.json",
    "issue1255_calyx_readback": f"{ISSUE1255_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1255_output_manifest": f"{ISSUE1255_ROOT}/out/output_manifest.json",
}

USER_AGENT = "calyx-discovery/issue1256"
REQUEST_SLEEP_SECONDS = 0.15
PROMOTION_STATUS = "blocked_requires_external_source_safety_outcome_falsification_and_human_review"

RAW_DOC_URLS = {
    "clinpgx_downloads": "https://www.clinpgx.org/downloads",
    "clinpgx_api_root": "https://api.pharmgkb.org/",
    "kg_registry_pharmgkb": "https://kghub.org/kg-registry/resource/pharmgkb/pharmgkb.html",
}

ARCHIVE_URLS = {
    "clinicalAnnotations": "https://api.pharmgkb.org/v1/download/file/data/clinicalAnnotations.zip",
    "relationships": "https://api.pharmgkb.org/v1/download/file/data/relationships.zip",
    "drugLabels": "https://api.pharmgkb.org/v1/download/file/data/drugLabels.zip",
    "clinicalVariants": "https://api.pharmgkb.org/v1/download/file/data/clinicalVariants.zip",
    "variantAnnotations": "https://api.pharmgkb.org/v1/download/file/data/variantAnnotations.zip",
    "drugs": "https://api.pharmgkb.org/v1/download/file/data/drugs.zip",
    "chemicals": "https://api.pharmgkb.org/v1/download/file/data/chemicals.zip",
}

SCAN_FILES = {
    "clinical_annotations": ("clinicalAnnotations", "clinical_annotations.tsv"),
    "clinical_ann_evidence": ("clinicalAnnotations", "clinical_ann_evidence.tsv"),
    "clinicalVariants": ("clinicalVariants", "clinicalVariants.tsv"),
    "drugLabels": ("drugLabels", "drugLabels.tsv"),
    "drugLabels_byGene": ("drugLabels", "drugLabels.byGene.tsv"),
    "relationships": ("relationships", "relationships.tsv"),
    "var_drug_ann": ("variantAnnotations", "var_drug_ann.tsv"),
    "var_pheno_ann": ("variantAnnotations", "var_pheno_ann.tsv"),
    "var_fa_ann": ("variantAnnotations", "var_fa_ann.tsv"),
}

ALIAS_FILES = {
    "drugs": ("drugs", "drugs.tsv"),
    "chemicals": ("chemicals", "chemicals.tsv"),
}

PAIR_STATUS_VALUES = {
    "pharmgkb_same_row_two_term_hit_still_blocked",
    "pharmgkb_single_term_mappings_without_pair_match_still_blocked",
    "pharmgkb_no_term_mapping_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "pharmgkb_candidate_same_row_two_term_hit_still_blocked",
    "pharmgkb_candidate_single_term_mappings_without_pair_match_still_blocked",
    "pharmgkb_candidate_no_term_mapping_still_blocked",
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


def split_alias_field(value: object) -> list[str]:
    text = clean_text(value).strip()
    if not text:
        return []
    parts = re.split(r'\s*[,;|]\s*|"\s*,\s*"', text)
    out: list[str] = []
    for part in parts:
        part = part.strip().strip('"').strip("'")
        if part:
            out.append(part)
    return out


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


def count_tsv(path: Path) -> int:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return max(0, sum(1 for _ in handle) - 1)


def artifact(path: Path, *, jsonl: bool = False, tsv_rows: bool = False, source_url: str | None = None) -> dict[str, Any]:
    value = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            value["rows"] = sum(1 for line in handle if line.strip())
    if tsv_rows:
        value["rows"] = count_tsv(path)
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


def fetch_url(raw_dir: Path, key: str, url: str, filename: str) -> dict[str, Any]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    path = raw_dir / filename
    if not path.exists():
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        with urllib.request.urlopen(request, timeout=180) as response:
            payload = response.read()
            status = int(response.status)
        path.write_bytes(payload)
        (raw_dir / f"{filename}.status").write_text(str(status) + "\n", encoding="utf-8")
        time.sleep(REQUEST_SLEEP_SECONDS)
    return artifact(path, source_url=url)


def fetch_raw_docs(raw_dir: Path) -> dict[str, dict[str, Any]]:
    return {
        key: fetch_url(raw_dir, key, url, f"{key}.html" if not url.endswith(".json") else f"{key}.json")
        for key, url in RAW_DOC_URLS.items()
    }


def fetch_archives(raw_dir: Path) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for key, url in ARCHIVE_URLS.items():
        out[key] = fetch_url(raw_dir, key, url, f"{key}.zip")
    return out


def extract_archives(raw_dir: Path) -> dict[str, Path]:
    extracted: dict[str, Path] = {}
    for key in ARCHIVE_URLS:
        archive = raw_dir / f"{key}.zip"
        out_dir = raw_dir / f"extracted_{key}"
        if not out_dir.exists():
            out_dir.mkdir(parents=True, exist_ok=True)
            with zipfile.ZipFile(archive) as zip_handle:
                zip_handle.extractall(out_dir)
        extracted[key] = out_dir
    return extracted


def tsv_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return [dict(row) for row in csv.DictReader(handle, delimiter="\t")]


def load_candidates(rows: list[dict[str, Any]], max_pairs: int | None = None) -> list[dict[str, Any]]:
    blocked_statuses = {
        "drugcentral_candidate_single_term_mappings_without_pair_match_still_blocked",
        "drugcentral_candidate_no_term_mapping_still_blocked",
    }
    out = [row for row in rows if row.get("drugcentral_candidate_status") in blocked_statuses]
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
        out.append(
            {
                "schema_version": 1,
                "pair_key": pair_key,
                "drug_a": first["drug_a"],
                "drug_b": first["drug_b"],
                "representative_pair_id": first.get("pair_id"),
                "source_pair_ids": sorted({row.get("pair_id") for row in members if row.get("pair_id")}),
                "source_drugcentral_candidate_status_ids": sorted(
                    {
                        row.get("drugcentral_candidate_status_id")
                        for row in members
                        if row.get("drugcentral_candidate_status_id")
                    }
                ),
                "source_drugcentral_pair_status_ids": sorted(
                    {
                        row.get("drugcentral_pair_status_id")
                        for row in members
                        if row.get("drugcentral_pair_status_id")
                    }
                ),
                "candidate_count": len(members),
            }
        )
    return out


def add_alias(
    name_index: dict[str, set[str]],
    id_names: dict[str, set[str]],
    entity_id: str,
    name: object,
) -> None:
    entity_id = clean_text(entity_id)
    text = clean_text(name)
    norm = norm_name(text)
    if not entity_id or not norm:
        return
    name_index[norm].add(entity_id)
    id_names[entity_id].add(text)


def build_alias_index(extracted: dict[str, Path]) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    name_index: dict[str, set[str]] = defaultdict(set)
    id_names: dict[str, set[str]] = defaultdict(set)
    for archive_key, filename in ALIAS_FILES.values():
        path = extracted[archive_key] / filename
        for row in tsv_rows(path):
            entity_id = row.get("PharmGKB Accession Id") or row.get("PharmGKB ID") or ""
            add_alias(name_index, id_names, entity_id, row.get("Name"))
            for field in ["Generic Names", "Trade Names", "Brand Mixtures", "Cross-references", "RxNorm Identifiers", "PubChem Compound Identifiers", "ATC Identifiers"]:
                for value in split_alias_field(row.get(field)):
                    add_alias(name_index, id_names, entity_id, value)
    return name_index, id_names


def mapped_ids(name_index: dict[str, set[str]], term: str) -> set[str]:
    return set(name_index.get(norm_name(term), set()))


def exact_presence_precomputed(row_lower: str, row_norm: str, name: str) -> dict[str, Any]:
    raw = clean_text(name)
    query = query_name(raw)
    norm = norm_name(raw)
    raw_present = bool(raw and raw.lower() in row_lower)
    query_present = bool(query and query.lower() in row_lower)
    norm_present = bool(norm and f" {norm} " in row_norm)
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


def row_match(row_text: str, pair: dict[str, Any], name_index: dict[str, set[str]]) -> dict[str, Any]:
    row_lower = row_text.lower()
    row_norm = normalized_blob(row_text)
    left_presence = exact_presence_precomputed(row_lower, row_norm, pair["drug_a"])
    right_presence = exact_presence_precomputed(row_lower, row_norm, pair["drug_b"])
    left_ids = mapped_ids(name_index, pair["drug_a"])
    right_ids = mapped_ids(name_index, pair["drug_b"])
    left_id_present = sorted(entity_id for entity_id in left_ids if entity_id and f" {norm_name(entity_id)} " in row_norm)
    right_id_present = sorted(entity_id for entity_id in right_ids if entity_id and f" {norm_name(entity_id)} " in row_norm)
    return {
        "match_drug_a": left_presence,
        "match_drug_b": right_presence,
        "drug_a_pharmgkb_ids": sorted(left_ids),
        "drug_b_pharmgkb_ids": sorted(right_ids),
        "drug_a_ids_present_in_row": left_id_present,
        "drug_b_ids_present_in_row": right_id_present,
        "drug_a_matched": left_presence["present"] or bool(left_id_present),
        "drug_b_matched": right_presence["present"] or bool(right_id_present),
    }


def prepare_pair_scan(
    pairs: list[dict[str, Any]],
    name_index: dict[str, set[str]],
) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]], dict[str, list[tuple[str, str]]]]:
    pair_context: dict[str, dict[str, Any]] = {}
    prepared: dict[str, dict[str, Any]] = {}
    term_index: dict[str, list[tuple[str, str]]] = defaultdict(list)
    seen_terms: set[tuple[str, str, str]] = set()

    for pair in pairs:
        pair_key = pair["pair_key"]
        left_ids = sorted(mapped_ids(name_index, pair["drug_a"]))
        right_ids = sorted(mapped_ids(name_index, pair["drug_b"]))
        left_id_norms = sorted({norm_name(entity_id) for entity_id in left_ids if norm_name(entity_id)})
        right_id_norms = sorted({norm_name(entity_id) for entity_id in right_ids if norm_name(entity_id)})
        prepared[pair_key] = {
            "pair": pair,
            "drug_a_pharmgkb_ids": left_ids,
            "drug_b_pharmgkb_ids": right_ids,
            "drug_a_id_norms": left_id_norms,
            "drug_b_id_norms": right_id_norms,
        }
        pair_context[pair_key] = {
            "drug_a_pharmgkb_ids": left_ids,
            "drug_b_pharmgkb_ids": right_ids,
            "evidence_rows": 0,
        }

        for role, values in [
            ("a", [pair["drug_a"], *left_id_norms]),
            ("b", [pair["drug_b"], *right_id_norms]),
        ]:
            for value in values:
                term = norm_name(value)
                if not term:
                    continue
                marker = (term, pair_key, role)
                if marker in seen_terms:
                    continue
                seen_terms.add(marker)
                term_index[term].append((pair_key, role))

    return pair_context, prepared, term_index


def build_term_lookup(term_index: dict[str, list[tuple[str, str]]]) -> dict[str, list[tuple[tuple[str, ...], str]]]:
    terms_by_first: dict[str, list[tuple[tuple[str, ...], str]]] = defaultdict(list)
    for term in term_index:
        tokens = tuple(term.split())
        if not tokens:
            continue
        terms_by_first[tokens[0]].append((tokens, term))
    for first_token in list(terms_by_first):
        terms_by_first[first_token].sort(key=lambda item: (-len(item[0]), item[1]))
    return terms_by_first


def present_norm_terms(row_norm: str, terms_by_first: dict[str, list[tuple[tuple[str, ...], str]]]) -> set[str]:
    tokens = tuple(row_norm.split())
    present: set[str] = set()
    for idx, token in enumerate(tokens):
        for term_tokens, term in terms_by_first.get(token, []):
            end = idx + len(term_tokens)
            if end <= len(tokens) and tokens[idx:end] == term_tokens:
                present.add(term)
    return present


def row_match_prepared(row_text: str, row_lower: str, row_norm: str, prepared_pair: dict[str, Any]) -> dict[str, Any]:
    pair = prepared_pair["pair"]
    left_presence = exact_presence_precomputed(row_lower, row_norm, pair["drug_a"])
    right_presence = exact_presence_precomputed(row_lower, row_norm, pair["drug_b"])
    left_ids = prepared_pair["drug_a_pharmgkb_ids"]
    right_ids = prepared_pair["drug_b_pharmgkb_ids"]
    left_id_present = sorted(
        entity_id
        for entity_id in left_ids
        if entity_id and f" {norm_name(entity_id)} " in row_norm
    )
    right_id_present = sorted(
        entity_id
        for entity_id in right_ids
        if entity_id and f" {norm_name(entity_id)} " in row_norm
    )
    return {
        "match_drug_a": left_presence,
        "match_drug_b": right_presence,
        "drug_a_pharmgkb_ids": left_ids,
        "drug_b_pharmgkb_ids": right_ids,
        "drug_a_ids_present_in_row": left_id_present,
        "drug_b_ids_present_in_row": right_id_present,
        "drug_a_matched": left_presence["present"] or bool(left_id_present),
        "drug_b_matched": right_presence["present"] or bool(right_id_present),
    }


def scan_source_files(
    pairs: list[dict[str, Any]],
    extracted: dict[str, Path],
    name_index: dict[str, set[str]],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    evidence_rows: list[dict[str, Any]] = []
    pair_context, prepared_pairs, term_index = prepare_pair_scan(pairs, name_index)
    terms_by_first = build_term_lookup(term_index)

    for source_name, (archive_key, filename) in SCAN_FILES.items():
        path = extracted[archive_key] / filename
        source_sha = sha256_path(path)
        rows = tsv_rows(path)
        for row_index, row in enumerate(rows, start=1):
            row_text = clean_text(row)
            row_lower = row_text.lower()
            row_norm = normalized_blob(row_text)
            matched_roles: dict[str, set[str]] = defaultdict(set)
            for term in present_norm_terms(row_norm, terms_by_first):
                for pair_key, role in term_index[term]:
                    matched_roles[pair_key].add(role)
            for pair_key, roles in matched_roles.items():
                if not {"a", "b"}.issubset(roles):
                    continue
                prepared_pair = prepared_pairs[pair_key]
                pair = prepared_pair["pair"]
                match = row_match_prepared(row_text, row_lower, row_norm, prepared_pair)
                if not (match["drug_a_matched"] and match["drug_b_matched"]):
                    continue
                row_id = (
                    row.get("Clinical Annotation ID")
                    or row.get("Variant Annotation ID")
                    or row.get("PharmGKB ID")
                    or row.get("Entity1_id")
                    or f"row-{row_index}"
                )
                evidence_id = f"pharmgkb-evidence:{stable_id(pair['pair_key'], source_name, row_id, row_index)}"
                evidence = {
                    "schema_version": 1,
                    "pharmgkb_evidence_id": evidence_id,
                    "source_issue": 1256,
                    "pair_key": pair["pair_key"],
                    "drug_a": pair["drug_a"],
                    "drug_b": pair["drug_b"],
                    "source_archive": archive_key,
                    "source_table": source_name,
                    "source_file": filename,
                    "source_path": str(path),
                    "source_sha256": source_sha,
                    "source_row_index": row_index,
                    "source_row_id": clean_text(row_id),
                    "source_row_sha256": sha256_bytes(json.dumps(row, sort_keys=True).encode("utf-8")),
                    "source_text_sha256": sha256_bytes(row_text.encode("utf-8")),
                    "source_text_sample": row_text[:2000],
                    "match": match,
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "promotion_status": PROMOTION_STATUS,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                    "reason_codes": [
                        "pharmgkb_source_row_contains_both_pair_terms_or_ids",
                        "pharmgkb_source_mining_not_clinical_actionability",
                        "requires_safety_outcome_falsification_and_human_review",
                    ],
                }
                evidence_rows.append(evidence)
                pair_context[pair["pair_key"]]["evidence_rows"] += 1
    evidence_rows.sort(key=lambda row: (row["pair_key"], row["source_table"], row["source_row_id"], row["pharmgkb_evidence_id"]))
    return evidence_rows, pair_context


def build_source_inventory(raw_docs: dict[str, Any], archives: dict[str, Any], extracted: dict[str, Path]) -> dict[str, Any]:
    inventory: dict[str, Any] = {"raw_docs": raw_docs, "archives": archives, "tables": {}}
    for source_name, (archive_key, filename) in {**SCAN_FILES, **ALIAS_FILES}.items():
        path = extracted[archive_key] / filename
        inventory["tables"][source_name] = artifact(path, tsv_rows=True)
    return inventory


def build_source_rows(source_inventory: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for group_name in ["raw_docs", "archives", "tables"]:
        for name, info in sorted(source_inventory[group_name].items()):
            text = (
                f"PharmGKB source {group_name} {name} rows {info.get('rows', '')} "
                f"bytes {info['bytes']} sha256 {info['sha256']}."
            )
            rows.append(
                {
                    "schema_version": 1,
                    "source_row_id": f"pharmgkb-source:{group_name}:{name}",
                    "source_group": group_name,
                    "source_name": name,
                    "rows": info.get("rows"),
                    "bytes": info["bytes"],
                    "sha256": info["sha256"],
                    "path": info["path"],
                    "text": text,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return rows


def build_pair_status(pair: dict[str, Any], context: dict[str, Any], evidence_rows: list[dict[str, Any]]) -> dict[str, Any]:
    evidence_ids = [row["pharmgkb_evidence_id"] for row in evidence_rows]
    if evidence_ids:
        status = "pharmgkb_same_row_two_term_hit_still_blocked"
        reason = "pharmgkb_source_row_contains_both_pair_terms_or_ids"
    elif context["drug_a_pharmgkb_ids"] or context["drug_b_pharmgkb_ids"]:
        status = "pharmgkb_single_term_mappings_without_pair_match_still_blocked"
        reason = "pharmgkb_single_term_mappings_without_pair_match"
    else:
        status = "pharmgkb_no_term_mapping_still_blocked"
        reason = "pharmgkb_no_term_mapping_for_pair"
    return {
        "schema_version": 1,
        "pharmgkb_pair_status_id": f"pharmgkb-pair-status:{stable_id(pair['pair_key'], status)}",
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "representative_pair_id": pair.get("representative_pair_id"),
        "source_pair_ids": pair.get("source_pair_ids", []),
        "source_drugcentral_candidate_status_ids": pair.get("source_drugcentral_candidate_status_ids", []),
        "source_drugcentral_pair_status_ids": pair.get("source_drugcentral_pair_status_ids", []),
        "pharmgkb_pair_status": status,
        "drug_a_pharmgkb_ids": context["drug_a_pharmgkb_ids"],
        "drug_b_pharmgkb_ids": context["drug_b_pharmgkb_ids"],
        "pharmgkb_evidence_rows": len(evidence_ids),
        "evidence_ids": evidence_ids,
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "pharmgkb_source_mining_not_clinical_actionability",
            "requires_safety_outcome_falsification_and_human_review",
            reason,
        ],
    }


def build_candidate_status(candidate: dict[str, Any], pair_status: dict[str, Any]) -> dict[str, Any]:
    pair_to_candidate = {
        "pharmgkb_same_row_two_term_hit_still_blocked": "pharmgkb_candidate_same_row_two_term_hit_still_blocked",
        "pharmgkb_single_term_mappings_without_pair_match_still_blocked": (
            "pharmgkb_candidate_single_term_mappings_without_pair_match_still_blocked"
        ),
        "pharmgkb_no_term_mapping_still_blocked": "pharmgkb_candidate_no_term_mapping_still_blocked",
    }
    status = pair_to_candidate[pair_status["pharmgkb_pair_status"]]
    return {
        "schema_version": 1,
        "pharmgkb_candidate_status_id": f"pharmgkb-candidate-status:{stable_id(candidate['pair_id'], pair_status['pharmgkb_pair_status'])}",
        "pair_id": candidate["pair_id"],
        "pair_key": candidate["pair_key"],
        "drug_a": candidate["drug_a"],
        "drug_b": candidate["drug_b"],
        "source_drugcentral_candidate_status_id": candidate.get("drugcentral_candidate_status_id"),
        "source_drugcentral_pair_status_id": candidate.get("drugcentral_pair_status_id"),
        "source_drugcentral_candidate_status": candidate.get("drugcentral_candidate_status"),
        "pharmgkb_pair_status_id": pair_status["pharmgkb_pair_status_id"],
        "pharmgkb_candidate_status": status,
        "pharmgkb_pair_status": pair_status["pharmgkb_pair_status"],
        "drug_a_pharmgkb_ids": pair_status["drug_a_pharmgkb_ids"],
        "drug_b_pharmgkb_ids": pair_status["drug_b_pharmgkb_ids"],
        "pharmgkb_evidence_rows": pair_status["pharmgkb_evidence_rows"],
        "evidence_ids": pair_status["evidence_ids"],
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": pair_status["reason_codes"],
    }


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in source_rows:
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "pharmgkb_source_snapshot",
                "text": row["text"],
                "bridge_terms": uniq(["PharmGKB", row["source_group"], row["source_name"], row["sha256"]]),
                "metadata": {
                    "source_dataset": "issue1256_pharmgkb_source_mining",
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
            f"PharmGKB candidate status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['pharmgkb_candidate_status']} "
            f"evidence rows {row['pharmgkb_evidence_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["pharmgkb_candidate_status_id"],
                "domain": "pharmgkb_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["pharmgkb_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1256_pharmgkb_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "pharmgkb_candidate_status": row["pharmgkb_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        text = (
            f"PharmGKB pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['pharmgkb_pair_status']} evidence rows {row['pharmgkb_evidence_rows']}."
        )
        rows.append(
            {
                "id": row["pharmgkb_pair_status_id"],
                "domain": "pharmgkb_pair_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["pharmgkb_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1256_pharmgkb_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "pharmgkb_pair_status": row["pharmgkb_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = max(0, 1000 - len(rows))
    for row in evidence_rows[:remaining]:
        text = (
            f"PharmGKB evidence {row['source_table']} row {row['source_row_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']}."
        )
        rows.append(
            {
                "id": row["pharmgkb_evidence_id"],
                "domain": "pharmgkb_pair_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_table"], row["source_row_id"]]),
                "metadata": {
                    "source_dataset": "issue1256_pharmgkb_source_mining",
                    "source_path": row["source_path"],
                    "source_sha256": row["source_sha256"],
                    "pair_key": row["pair_key"],
                    "source_table": row["source_table"],
                    "source_row_id": row["source_row_id"],
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
    source_inventory: dict[str, Any],
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    table_rows = {name: info.get("rows", 0) for name, info in source_inventory["tables"].items()}
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "source_table_rows": table_rows,
        "pharmgkb_evidence_rows": len(evidence_rows),
        "pair_status_rows": len(pair_status),
        "candidate_status_rows": len(candidate_status),
        "bridge_rows": len(bridge_rows),
        "bridge_evidence_rows_materialized": sum(1 for row in bridge_rows if row["domain"] == "pharmgkb_pair_evidence"),
        "pair_status_counts": dict(sorted(Counter(row["pharmgkb_pair_status"] for row in pair_status).items())),
        "candidate_status_counts": dict(sorted(Counter(row["pharmgkb_candidate_status"] for row in candidate_status).items())),
        "evidence_source_counts": dict(sorted(Counter(row["source_table"] for row in evidence_rows).items())),
        "all_rows_blocked": True,
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    source_inventory: dict[str, Any],
    issue1255_persisted_readback: dict[str, Any],
    issue1255_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1256,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1255_candidate_status": artifact(Path(inputs["issue1255_candidate_status"]), jsonl=True),
            "issue1255_pair_status": artifact(Path(inputs["issue1255_pair_status"]), jsonl=True),
            "issue1255_persisted_readback": artifact(Path(inputs["issue1255_persisted_readback"])),
            "issue1255_calyx_readback": artifact(Path(inputs["issue1255_calyx_readback"])),
            "issue1255_output_manifest": artifact(Path(inputs["issue1255_output_manifest"])),
        },
        "source_inventory": source_inventory,
        "source_contract": {
            "issue1255_persisted_assertions_all_true": all_assertions_true(issue1255_persisted_readback),
            "issue1255_calyx_assertions_all_true": all_assertions_true(issue1255_calyx_readback),
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "input_filter": "drugcentral_candidate_status in blocked DrugCentral no-hit/no-pair-match statuses",
            "pharmgkb_hit_requires_same_source_row_two_term_presence_or_ids": True,
            "clinical_rows_are_blockers_not_actionability": True,
        },
    }


def build_readback(
    out_dir: Path,
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    source_rows: list[dict[str, Any]],
    evidence_rows: list[dict[str, Any]],
    pair_status: list[dict[str, Any]],
    candidate_status: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
    issue1255_persisted_readback: dict[str, Any],
    issue1255_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "pharmgkb_source_rows": artifact(out_dir / "pharmgkb_source_rows.jsonl", jsonl=True),
        "pharmgkb_pair_evidence": artifact(out_dir / "pharmgkb_pair_evidence.jsonl", jsonl=True),
        "pharmgkb_pair_status": artifact(out_dir / "pharmgkb_pair_status.jsonl", jsonl=True),
        "candidate_pharmgkb_status": artifact(out_dir / "candidate_pharmgkb_status.jsonl", jsonl=True),
        "pharmgkb_bridge_rows": artifact(out_dir / "pharmgkb_bridge_rows.jsonl", jsonl=True),
        "input_manifest": artifact(out_dir / "input_manifest.json"),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    candidate_pair_keys = {row["pair_key"] for row in candidates}
    pair_status_keys = {row["pair_key"] for row in pair_status}
    status_candidate_ids = {row["pair_id"] for row in candidate_status}
    candidate_ids = {row["pair_id"] for row in candidates}
    evidence_by_pair = Counter(row["pair_key"] for row in evidence_rows)
    assertions = {
        "issue1255_persisted_readback_all_true": all_assertions_true(issue1255_persisted_readback),
        "issue1255_calyx_readback_all_true": all_assertions_true(issue1255_calyx_readback),
        "source_rows_present": len(source_rows) >= len(RAW_DOC_URLS) + len(ARCHIVE_URLS) + len(SCAN_FILES),
        "pair_status_for_every_pair_key": pair_status_keys == candidate_pair_keys,
        "candidate_status_for_every_candidate": status_candidate_ids == candidate_ids,
        "all_hits_have_evidence": all(
            row["pharmgkb_pair_status"] != "pharmgkb_same_row_two_term_hit_still_blocked"
            or evidence_by_pair[row["pair_key"]] > 0
            for row in pair_status
        ),
        "all_evidence_rows_have_both_matches": all(
            row.get("match", {}).get("drug_a_matched") and row.get("match", {}).get("drug_b_matched")
            for row in evidence_rows
        ),
        "all_evidence_rows_have_source_hash": all(row.get("source_sha256") for row in evidence_rows),
        "all_pair_status_values_allowed": all(row["pharmgkb_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
        "all_candidate_status_values_allowed": all(
            row["pharmgkb_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
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
        "issue": 1256,
        "status": "ok" if all(assertions.values()) else "failed",
        "created_utc": now_utc(),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "row_counts": {
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "source_rows": len(source_rows),
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

    issue1255_persisted_readback = read_json(Path(inputs["issue1255_persisted_readback"]))
    issue1255_calyx_readback = read_json(Path(inputs["issue1255_calyx_readback"]))
    source_candidates = rows_jsonl(Path(inputs["issue1255_candidate_status"]))
    candidates = load_candidates(source_candidates, max_pairs=max_pairs)
    pairs = pair_rows(candidates)

    raw_docs = fetch_raw_docs(raw_dir)
    archives = fetch_archives(raw_dir)
    extracted = extract_archives(raw_dir)
    source_inventory = build_source_inventory(raw_docs, archives, extracted)
    source_rows = build_source_rows(source_inventory)
    write_jsonl(out_dir / "pharmgkb_source_rows.jsonl", source_rows)

    name_index, _id_names = build_alias_index(extracted)
    evidence_rows, pair_context = scan_source_files(pairs, extracted, name_index)
    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_pair[row["pair_key"]].append(row)

    pair_status_rows = [
        build_pair_status(pair, pair_context[pair["pair_key"]], evidence_by_pair[pair["pair_key"]])
        for pair in pairs
    ]
    pair_status_by_key = {row["pair_key"]: row for row in pair_status_rows}
    candidate_status_rows = [build_candidate_status(row, pair_status_by_key[row["pair_key"]]) for row in candidates]

    write_jsonl(out_dir / "pharmgkb_pair_evidence.jsonl", evidence_rows)
    write_jsonl(out_dir / "pharmgkb_pair_status.jsonl", pair_status_rows)
    write_jsonl(out_dir / "candidate_pharmgkb_status.jsonl", candidate_status_rows)

    bridge_rows = build_bridge_rows(
        source_rows,
        candidate_status_rows,
        pair_status_rows,
        evidence_rows,
        out_dir / "candidate_pharmgkb_status.jsonl",
        sha256_path(out_dir / "candidate_pharmgkb_status.jsonl"),
    )
    write_jsonl(out_dir / "pharmgkb_bridge_rows.jsonl", bridge_rows)

    input_manifest = build_input_manifest(
        inputs,
        candidates,
        pairs,
        source_inventory,
        issue1255_persisted_readback,
        issue1255_calyx_readback,
    )
    write_json(out_dir / "input_manifest.json", input_manifest)

    metrics = build_metrics(candidates, pairs, source_inventory, evidence_rows, pair_status_rows, candidate_status_rows, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1256,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "pharmgkb_source_rows": artifact(out_dir / "pharmgkb_source_rows.jsonl", jsonl=True),
            "pharmgkb_pair_evidence": artifact(out_dir / "pharmgkb_pair_evidence.jsonl", jsonl=True),
            "pharmgkb_pair_status": artifact(out_dir / "pharmgkb_pair_status.jsonl", jsonl=True),
            "candidate_pharmgkb_status": artifact(out_dir / "candidate_pharmgkb_status.jsonl", jsonl=True),
            "pharmgkb_bridge_rows": artifact(out_dir / "pharmgkb_bridge_rows.jsonl", jsonl=True),
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)

    persisted_readback = build_readback(
        out_dir,
        candidates,
        pairs,
        source_rows,
        evidence_rows,
        pair_status_rows,
        candidate_status_rows,
        bridge_rows,
        issue1255_persisted_readback,
        issue1255_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", persisted_readback)

    final = {
        "status": persisted_readback["status"],
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "source_rows": artifact(out_dir / "pharmgkb_source_rows.jsonl", jsonl=True),
            "pair_evidence": artifact(out_dir / "pharmgkb_pair_evidence.jsonl", jsonl=True),
            "pair_status": artifact(out_dir / "pharmgkb_pair_status.jsonl", jsonl=True),
            "candidate_status": artifact(out_dir / "candidate_pharmgkb_status.jsonl", jsonl=True),
            "bridge_rows": artifact(out_dir / "pharmgkb_bridge_rows.jsonl", jsonl=True),
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
