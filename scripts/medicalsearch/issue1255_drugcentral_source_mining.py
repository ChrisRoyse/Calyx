#!/usr/bin/env python3
"""#1255 DrugCentral source mining after ChEMBL no-hit.

This stage reads sealed #1254 blocked candidate rows, snapshots relevant
DrugCentral source tables, and checks structured drug-drug interaction rows plus
same-structure equivalence mappings. Hits remain review/blocking inputs only.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "DrugCentral source mining is drug/source triage only; interaction rows are "
    "blockers/review inputs, not safety clearance, efficacy, treatment guidance, "
    "dosing guidance, recommendation, clinical actionability, pair-interaction "
    "proof, or cure evidence."
)

SOURCE_EVIDENCE_KIND = (
    "drugcentral_structured_source_match_not_safety_clearance_efficacy_or_cure"
)

ISSUE1254_ROOT = "/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z"
DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z"

DEFAULT_INPUTS = {
    "issue1254_candidate_status": f"{ISSUE1254_ROOT}/out/candidate_chembl_status.jsonl",
    "issue1254_pair_status": f"{ISSUE1254_ROOT}/out/chembl_pair_status.jsonl",
    "issue1254_persisted_readback": f"{ISSUE1254_ROOT}/out/persisted_readback.json",
    "issue1254_calyx_readback": f"{ISSUE1254_ROOT}/out/calyx_bridge_corpus_readback.json",
    "issue1254_output_manifest": f"{ISSUE1254_ROOT}/out/output_manifest.json",
}

DRUGCENTRAL_DOWNLOAD_URL = "https://drugcentral.org/download"
DRUGCENTRAL_ACTIVE_DOWNLOAD_URL = "https://drugcentral.org/ActiveDownload"
DRUGCENTRAL_API_DOCS_URL = "https://uxn2ycvimg.us-east-2.awsapprunner.com/docs"
DRUGCENTRAL_OPENAPI_URL = "https://uxn2ycvimg.us-east-2.awsapprunner.com/openapi.json"
DRUGCENTRAL_INTERACTION_TSV_URL = "https://unmtid-dbs.net/download/DrugCentral/2021_09_01/drug.target.interaction.tsv.gz"

USER_AGENT = "calyx-discovery/issue1255"
REQUEST_SLEEP_SECONDS = 0.15
PROMOTION_STATUS = "blocked_requires_external_source_safety_outcome_falsification_and_human_review"

SNAPSHOT_TABLES = {
    "ddi": """
        SELECT id, drug_class1, drug_class2, ddi_ref_id, ddi_risk, description, source_id
        FROM public.ddi
        ORDER BY id
    """,
    "ddi_risk": """
        SELECT id, risk, ddi_ref_id
        FROM public.ddi_risk
        ORDER BY id
    """,
    "structures": """
        SELECT id, name, cas_reg_no, status, inchikey, smiles
        FROM public.structures
        ORDER BY id
    """,
    "synonyms": """
        SELECT syn_id, id, name, preferred_name, parent_id, lname
        FROM public.synonyms
        ORDER BY syn_id
    """,
    "identifier": """
        SELECT id, identifier, id_type, struct_id, parent_match
        FROM public.identifier
        ORDER BY id
    """,
    "approval": """
        SELECT id, struct_id, approval, type, applicant, orphan
        FROM public.approval
        ORDER BY id
    """,
    "omop_relationship": """
        SELECT id, struct_id, concept_id, relationship_name, concept_name, umls_cui,
               snomed_full_name, cui_semantic_type, snomed_conceptid
        FROM public.omop_relationship
        ORDER BY id
    """,
    "act_table_full": """
        SELECT act_id, struct_id, target_id, target_name, target_class, accession, gene,
               swissprot, act_value, act_unit, act_type, act_comment, act_source,
               relation, moa, moa_source, act_source_url, moa_source_url, action_type,
               first_in_class, tdl, act_ref_id, moa_ref_id, organism
        FROM public.act_table_full
        ORDER BY act_id
    """,
}

PAIR_STATUS_VALUES = {
    "drugcentral_ddi_structured_hit_still_blocked",
    "drugcentral_same_structure_equivalence_hit_still_blocked",
    "drugcentral_single_term_mappings_without_pair_match_still_blocked",
    "drugcentral_no_term_mapping_still_blocked",
}

CANDIDATE_STATUS_VALUES = {
    "drugcentral_candidate_ddi_structured_hit_still_blocked",
    "drugcentral_candidate_same_structure_equivalence_hit_still_blocked",
    "drugcentral_candidate_single_term_mappings_without_pair_match_still_blocked",
    "drugcentral_candidate_no_term_mapping_still_blocked",
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


def artifact(path: Path, *, jsonl: bool = False, csv_rows: bool = False, source_url: str | None = None) -> dict[str, Any]:
    value = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
    if jsonl:
        with path.open("r", encoding="utf-8") as handle:
            value["rows"] = sum(1 for line in handle if line.strip())
    if csv_rows:
        with path.open("r", encoding="utf-8", newline="") as handle:
            value["rows"] = max(0, sum(1 for _ in handle) - 1)
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
        with urllib.request.urlopen(request, timeout=90) as response:
            payload = response.read()
            status = int(response.status)
        path.write_bytes(payload)
        (raw_dir / f"{filename}.status").write_text(str(status) + "\n", encoding="utf-8")
        time.sleep(REQUEST_SLEEP_SECONDS)
    return artifact(path, source_url=url)


def fetch_raw_sources(raw_dir: Path) -> dict[str, dict[str, Any]]:
    return {
        "drugcentral_download_page": fetch_url(raw_dir, "download", DRUGCENTRAL_DOWNLOAD_URL, "drugcentral_download.html"),
        "drugcentral_active_download_page": fetch_url(
            raw_dir, "active_download", DRUGCENTRAL_ACTIVE_DOWNLOAD_URL, "drugcentral_active_download.html"
        ),
        "drugcentral_api_docs": fetch_url(raw_dir, "api_docs", DRUGCENTRAL_API_DOCS_URL, "drugcentral_api_docs.html"),
        "drugcentral_openapi": fetch_url(raw_dir, "openapi", DRUGCENTRAL_OPENAPI_URL, "drugcentral_openapi.json"),
    }


def db_env() -> dict[str, str]:
    env = os.environ.copy()
    password = env.get("DRUGCENTRAL_PGPASSWORD") or env.get("PGPASSWORD")
    if not password:
        raise RuntimeError(
            "Missing DRUGCENTRAL_PGPASSWORD/PGPASSWORD. The script requires runtime credentials for the public "
            "DrugCentral PostgreSQL source and does not store them in repo artifacts."
        )
    env["PGPASSWORD"] = password
    return env


def psql_base_command() -> list[str]:
    return [
        "psql",
        "-X",
        "-v",
        "ON_ERROR_STOP=1",
        "-h",
        os.environ.get("DRUGCENTRAL_PGHOST", "unmtid-dbs.net"),
        "-p",
        os.environ.get("DRUGCENTRAL_PGPORT", "5433"),
        "-U",
        os.environ.get("DRUGCENTRAL_PGUSER", "drugman"),
        "-d",
        os.environ.get("DRUGCENTRAL_PGDATABASE", "drugcentral"),
    ]


def psql_capture(sql: str) -> bytes:
    result = subprocess.run(
        psql_base_command() + ["-c", sql],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=db_env(),
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"psql failed rc={result.returncode}; stderr={result.stderr.decode('utf-8', 'replace')[:2000]}"
        )
    return result.stdout


def export_csv(raw_dir: Path, name: str, sql: str) -> dict[str, Any]:
    path = raw_dir / f"drugcentral_{name}.csv"
    if not path.exists():
        copy_sql = f"COPY ({sql}) TO STDOUT WITH (FORMAT csv, HEADER true)"
        path.write_bytes(psql_capture(copy_sql))
    return artifact(path, csv_rows=True)


def snapshot_source_tables(raw_dir: Path) -> dict[str, dict[str, Any]]:
    raw_dir.mkdir(parents=True, exist_ok=True)
    artifacts: dict[str, dict[str, Any]] = {}
    schema_sql = """
        SELECT table_name, column_name, data_type, ordinal_position
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name IN ('ddi','ddi_risk','structures','synonyms','identifier',
                             'approval','omop_relationship','act_table_full')
        ORDER BY table_name, ordinal_position
    """
    counts_sql = """
        SELECT 'ddi' AS table_name, count(*)::bigint AS row_count FROM public.ddi
        UNION ALL SELECT 'ddi_risk', count(*)::bigint FROM public.ddi_risk
        UNION ALL SELECT 'structures', count(*)::bigint FROM public.structures
        UNION ALL SELECT 'synonyms', count(*)::bigint FROM public.synonyms
        UNION ALL SELECT 'identifier', count(*)::bigint FROM public.identifier
        UNION ALL SELECT 'approval', count(*)::bigint FROM public.approval
        UNION ALL SELECT 'omop_relationship', count(*)::bigint FROM public.omop_relationship
        UNION ALL SELECT 'act_table_full', count(*)::bigint FROM public.act_table_full
        ORDER BY table_name
    """
    artifacts["schema"] = export_csv(raw_dir, "schema", schema_sql)
    artifacts["counts"] = export_csv(raw_dir, "counts", counts_sql)
    for table_name, sql in SNAPSHOT_TABLES.items():
        artifacts[table_name] = export_csv(raw_dir, table_name, sql)
    return artifacts


def csv_rows(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return [dict(row) for row in csv.DictReader(handle)]


def load_candidates(rows: list[dict[str, Any]], max_pairs: int | None = None) -> list[dict[str, Any]]:
    blocked_statuses = {
        "chembl_candidate_record_without_pair_match_still_blocked",
        "chembl_candidate_no_result_still_blocked",
        "chembl_candidate_query_failed_still_blocked",
    }
    out = [row for row in rows if row.get("chembl_candidate_status") in blocked_statuses]
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
                "source_chembl_candidate_status_ids": sorted(
                    {
                        row.get("chembl_candidate_status_id")
                        for row in members
                        if row.get("chembl_candidate_status_id")
                    }
                ),
                "source_chembl_pair_status_ids": sorted(
                    {row.get("chembl_pair_status_id") for row in members if row.get("chembl_pair_status_id")}
                ),
                "candidate_count": len(members),
            }
        )
    return out


def add_name(name_index: dict[str, set[str]], struct_names: dict[str, set[str]], struct_id: str, name: object) -> None:
    text = clean_text(name)
    norm = norm_name(text)
    if not struct_id or not norm:
        return
    name_index[norm].add(struct_id)
    struct_names[struct_id].add(text)


def build_name_index(raw_dir: Path) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    name_index: dict[str, set[str]] = defaultdict(set)
    struct_names: dict[str, set[str]] = defaultdict(set)
    for row in csv_rows(raw_dir / "drugcentral_structures.csv"):
        struct_id = clean_text(row.get("id"))
        add_name(name_index, struct_names, struct_id, row.get("name"))
        add_name(name_index, struct_names, struct_id, row.get("cas_reg_no"))
        add_name(name_index, struct_names, struct_id, row.get("inchikey"))
    for row in csv_rows(raw_dir / "drugcentral_synonyms.csv"):
        struct_id = clean_text(row.get("id"))
        add_name(name_index, struct_names, struct_id, row.get("name"))
        add_name(name_index, struct_names, struct_id, row.get("lname"))
    for row in csv_rows(raw_dir / "drugcentral_identifier.csv"):
        struct_id = clean_text(row.get("struct_id"))
        add_name(name_index, struct_names, struct_id, row.get("identifier"))
    return name_index, struct_names


def mapped_structs(name_index: dict[str, set[str]], term: str) -> set[str]:
    return set(name_index.get(norm_name(term), set()))


def participant_match(
    participant: str,
    term: str,
    name_index: dict[str, set[str]],
) -> dict[str, Any]:
    presence = exact_presence(participant, term)
    participant_structs = mapped_structs(name_index, participant)
    term_structs = mapped_structs(name_index, term)
    shared = sorted(participant_structs & term_structs)
    return {
        "participant": participant,
        "term": term,
        "presence": presence,
        "participant_struct_ids": sorted(participant_structs),
        "term_struct_ids": sorted(term_structs),
        "shared_struct_ids": shared,
        "matched": presence["present"] or bool(shared),
    }


def ddi_match_sides(row: dict[str, str], pair: dict[str, Any], name_index: dict[str, set[str]]) -> list[dict[str, Any]]:
    p1 = clean_text(row.get("drug_class1"))
    p2 = clean_text(row.get("drug_class2"))
    left_1 = participant_match(p1, pair["drug_a"], name_index)
    right_2 = participant_match(p2, pair["drug_b"], name_index)
    left_2 = participant_match(p2, pair["drug_a"], name_index)
    right_1 = participant_match(p1, pair["drug_b"], name_index)
    matches: list[dict[str, Any]] = []
    if left_1["matched"] and right_2["matched"]:
        matches.append({"orientation": "drug_a_to_drug_class1_drug_b_to_drug_class2", "drug_a_match": left_1, "drug_b_match": right_2})
    if left_2["matched"] and right_1["matched"]:
        matches.append({"orientation": "drug_a_to_drug_class2_drug_b_to_drug_class1", "drug_a_match": left_2, "drug_b_match": right_1})
    return matches


def build_source_inventory(raw_dir: Path) -> dict[str, Any]:
    inventory: dict[str, Any] = {}
    for table_name in SNAPSHOT_TABLES:
        path = raw_dir / f"drugcentral_{table_name}.csv"
        inventory[table_name] = artifact(path, csv_rows=True)
    inventory["schema"] = artifact(raw_dir / "drugcentral_schema.csv", csv_rows=True)
    inventory["counts"] = artifact(raw_dir / "drugcentral_counts.csv", csv_rows=True)
    return inventory


def build_drugcentral_source_rows(raw_dir: Path, source_inventory: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for table_name, info in source_inventory.items():
        text = (
            f"DrugCentral source snapshot table {table_name} rows {info.get('rows', 0)} "
            f"bytes {info['bytes']} sha256 {info['sha256']}."
        )
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": f"drugcentral-source:{table_name}",
                "table_name": table_name,
                "rows": info.get("rows"),
                "bytes": info["bytes"],
                "sha256": info["sha256"],
                "path": info["path"],
                "text": text,
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def mine_pair_evidence(
    pairs: list[dict[str, Any]],
    raw_dir: Path,
    name_index: dict[str, set[str]],
    struct_names: dict[str, set[str]],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    ddi_rows = csv_rows(raw_dir / "drugcentral_ddi.csv")
    ddi_path = raw_dir / "drugcentral_ddi.csv"
    ddi_sha = sha256_path(ddi_path)
    ddi_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    pair_by_key = {row["pair_key"]: row for row in pairs}

    for pair in pairs:
        left_structs = mapped_structs(name_index, pair["drug_a"])
        right_structs = mapped_structs(name_index, pair["drug_b"])
        pair["_drugcentral_left_struct_ids"] = sorted(left_structs)
        pair["_drugcentral_right_struct_ids"] = sorted(right_structs)
        pair["_drugcentral_shared_struct_ids"] = sorted(left_structs & right_structs)

    for ddi_row in ddi_rows:
        for pair in pairs:
            side_matches = ddi_match_sides(ddi_row, pair, name_index)
            for match in side_matches:
                evidence_id = f"drugcentral-evidence:{stable_id(pair['pair_key'], 'ddi', ddi_row.get('id'), match['orientation'])}"
                ddi_by_pair[pair["pair_key"]].append(
                    {
                        "schema_version": 1,
                        "drugcentral_evidence_id": evidence_id,
                        "source_issue": 1255,
                        "pair_key": pair["pair_key"],
                        "drug_a": pair["drug_a"],
                        "drug_b": pair["drug_b"],
                        "source_table": "ddi",
                        "source_id": f"DrugCentral-DDI:{ddi_row.get('id')}",
                        "source_row_id": clean_text(ddi_row.get("id")),
                        "source_path": str(ddi_path),
                        "source_sha256": ddi_sha,
                        "drug_class1": clean_text(ddi_row.get("drug_class1")),
                        "drug_class2": clean_text(ddi_row.get("drug_class2")),
                        "ddi_risk": clean_text(ddi_row.get("ddi_risk")),
                        "ddi_ref_id": clean_text(ddi_row.get("ddi_ref_id")),
                        "description": clean_text(ddi_row.get("description")),
                        "source_id_value": clean_text(ddi_row.get("source_id")),
                        "match": match,
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "promotion_status": PROMOTION_STATUS,
                        "clinical_boundary": CLINICAL_BOUNDARY,
                        "reason_codes": [
                            "drugcentral_structured_ddi_row_matches_pair_participants",
                            "interaction_row_is_review_blocker_not_safety_clearance",
                            "requires_safety_outcome_falsification_and_human_review",
                        ],
                    }
                )

    evidence_rows: list[dict[str, Any]] = []
    pair_context: dict[str, dict[str, Any]] = {}
    for pair in pairs:
        left_structs = set(pair["_drugcentral_left_struct_ids"])
        right_structs = set(pair["_drugcentral_right_struct_ids"])
        shared_structs = sorted(left_structs & right_structs)
        pair_evidence = list(ddi_by_pair.get(pair["pair_key"], []))
        if shared_structs:
            for struct_id in shared_structs:
                evidence_id = f"drugcentral-evidence:{stable_id(pair['pair_key'], 'same-structure', struct_id)}"
                pair_evidence.append(
                    {
                        "schema_version": 1,
                        "drugcentral_evidence_id": evidence_id,
                        "source_issue": 1255,
                        "pair_key": pair["pair_key"],
                        "drug_a": pair["drug_a"],
                        "drug_b": pair["drug_b"],
                        "source_table": "structures_synonyms_identifier",
                        "source_id": f"DrugCentral-STRUCT:{struct_id}",
                        "source_row_id": struct_id,
                        "source_path": str(raw_dir),
                        "source_sha256": sha256_path(raw_dir / "drugcentral_synonyms.csv"),
                        "shared_struct_id": struct_id,
                        "shared_struct_names": sorted(struct_names.get(struct_id, set()))[:50],
                        "match": {
                            "drug_a_struct_ids": sorted(left_structs),
                            "drug_b_struct_ids": sorted(right_structs),
                            "shared_struct_ids": shared_structs,
                        },
                        "evidence_kind": SOURCE_EVIDENCE_KIND,
                        "promotion_status": PROMOTION_STATUS,
                        "clinical_boundary": CLINICAL_BOUNDARY,
                        "reason_codes": [
                            "drugcentral_terms_resolve_to_same_structure",
                            "same_structure_mapping_is_identity_triage_not_clinical_actionability",
                            "requires_safety_outcome_falsification_and_human_review",
                        ],
                    }
                )
        pair_context[pair["pair_key"]] = {
            "drug_a_struct_ids": sorted(left_structs),
            "drug_b_struct_ids": sorted(right_structs),
            "shared_struct_ids": shared_structs,
            "evidence_rows": len(pair_evidence),
            "ddi_evidence_rows": sum(1 for row in pair_evidence if row["source_table"] == "ddi"),
            "same_structure_evidence_rows": sum(
                1 for row in pair_evidence if row["source_table"] == "structures_synonyms_identifier"
            ),
        }
        evidence_rows.extend(pair_evidence)
    evidence_rows.sort(key=lambda row: (row["pair_key"], row["source_table"], row["source_id"], row["drugcentral_evidence_id"]))
    return evidence_rows, pair_context


def build_pair_status(pair: dict[str, Any], context: dict[str, Any], evidence_rows: list[dict[str, Any]]) -> dict[str, Any]:
    evidence_ids = [row["drugcentral_evidence_id"] for row in evidence_rows]
    ddi_count = context["ddi_evidence_rows"]
    same_struct_count = context["same_structure_evidence_rows"]
    if ddi_count:
        status = "drugcentral_ddi_structured_hit_still_blocked"
        reason = "drugcentral_structured_ddi_row_matches_pair"
    elif same_struct_count:
        status = "drugcentral_same_structure_equivalence_hit_still_blocked"
        reason = "drugcentral_terms_resolve_to_same_structure"
    elif context["drug_a_struct_ids"] or context["drug_b_struct_ids"]:
        status = "drugcentral_single_term_mappings_without_pair_match_still_blocked"
        reason = "drugcentral_single_term_mappings_without_pair_match"
    else:
        status = "drugcentral_no_term_mapping_still_blocked"
        reason = "drugcentral_no_term_mapping_for_pair"

    return {
        "schema_version": 1,
        "drugcentral_pair_status_id": f"drugcentral-pair-status:{stable_id(pair['pair_key'], status)}",
        "pair_key": pair["pair_key"],
        "drug_a": pair["drug_a"],
        "drug_b": pair["drug_b"],
        "representative_pair_id": pair.get("representative_pair_id"),
        "source_pair_ids": pair.get("source_pair_ids", []),
        "source_chembl_candidate_status_ids": pair.get("source_chembl_candidate_status_ids", []),
        "source_chembl_pair_status_ids": pair.get("source_chembl_pair_status_ids", []),
        "drugcentral_pair_status": status,
        "drug_a_struct_ids": context["drug_a_struct_ids"],
        "drug_b_struct_ids": context["drug_b_struct_ids"],
        "shared_struct_ids": context["shared_struct_ids"],
        "drugcentral_evidence_rows": len(evidence_ids),
        "drugcentral_ddi_evidence_rows": ddi_count,
        "drugcentral_same_structure_evidence_rows": same_struct_count,
        "evidence_ids": evidence_ids,
        "evidence_kind": SOURCE_EVIDENCE_KIND,
        "promotion_status": PROMOTION_STATUS,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "reason_codes": [
            "drugcentral_source_mining_not_clinical_actionability",
            "requires_safety_outcome_falsification_and_human_review",
            reason,
        ],
    }


def build_candidate_status(candidate: dict[str, Any], pair_status: dict[str, Any]) -> dict[str, Any]:
    pair_to_candidate = {
        "drugcentral_ddi_structured_hit_still_blocked": "drugcentral_candidate_ddi_structured_hit_still_blocked",
        "drugcentral_same_structure_equivalence_hit_still_blocked": "drugcentral_candidate_same_structure_equivalence_hit_still_blocked",
        "drugcentral_single_term_mappings_without_pair_match_still_blocked": (
            "drugcentral_candidate_single_term_mappings_without_pair_match_still_blocked"
        ),
        "drugcentral_no_term_mapping_still_blocked": "drugcentral_candidate_no_term_mapping_still_blocked",
    }
    status = pair_to_candidate[pair_status["drugcentral_pair_status"]]
    return {
        "schema_version": 1,
        "drugcentral_candidate_status_id": f"drugcentral-candidate-status:{stable_id(candidate['pair_id'], pair_status['drugcentral_pair_status'])}",
        "pair_id": candidate["pair_id"],
        "pair_key": candidate["pair_key"],
        "drug_a": candidate["drug_a"],
        "drug_b": candidate["drug_b"],
        "source_chembl_candidate_status_id": candidate.get("chembl_candidate_status_id"),
        "source_chembl_pair_status_id": candidate.get("chembl_pair_status_id"),
        "source_chembl_candidate_status": candidate.get("chembl_candidate_status"),
        "drugcentral_pair_status_id": pair_status["drugcentral_pair_status_id"],
        "drugcentral_candidate_status": status,
        "drugcentral_pair_status": pair_status["drugcentral_pair_status"],
        "drug_a_struct_ids": pair_status["drug_a_struct_ids"],
        "drug_b_struct_ids": pair_status["drug_b_struct_ids"],
        "shared_struct_ids": pair_status["shared_struct_ids"],
        "drugcentral_evidence_rows": pair_status["drugcentral_evidence_rows"],
        "drugcentral_ddi_evidence_rows": pair_status["drugcentral_ddi_evidence_rows"],
        "drugcentral_same_structure_evidence_rows": pair_status["drugcentral_same_structure_evidence_rows"],
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
        text = row["text"]
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "drugcentral_source_snapshot",
                "text": text,
                "bridge_terms": uniq(["DrugCentral", row["table_name"], str(row.get("rows", "")), row["sha256"]]),
                "metadata": {
                    "source_dataset": "issue1255_drugcentral_source_mining",
                    "source_path": row["path"],
                    "source_sha256": row["sha256"],
                    "table_name": row["table_name"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in candidate_status:
        text = (
            f"DrugCentral candidate status {row['pair_id']} pair {row['pair_key']} "
            f"{row['drug_a']} plus {row['drug_b']} status {row['drugcentral_candidate_status']} "
            f"evidence rows {row['drugcentral_evidence_rows']} promotion {row['promotion_status']}."
        )
        rows.append(
            {
                "id": row["drugcentral_candidate_status_id"],
                "domain": "drugcentral_candidate_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["drugcentral_candidate_status"]]),
                "metadata": {
                    "source_dataset": "issue1255_drugcentral_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "drugcentral_candidate_status": row["drugcentral_candidate_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    for row in pair_status:
        text = (
            f"DrugCentral pair status {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"status {row['drugcentral_pair_status']} evidence rows {row['drugcentral_evidence_rows']} "
            f"DDI rows {row['drugcentral_ddi_evidence_rows']} same structure rows {row['drugcentral_same_structure_evidence_rows']}."
        )
        rows.append(
            {
                "id": row["drugcentral_pair_status_id"],
                "domain": "drugcentral_pair_status",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["drugcentral_pair_status"]]),
                "metadata": {
                    "source_dataset": "issue1255_drugcentral_source_mining",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "pair_key": row["pair_key"],
                    "drugcentral_pair_status": row["drugcentral_pair_status"],
                    "promotion_status": row["promotion_status"],
                    "evidence_kind": SOURCE_EVIDENCE_KIND,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    remaining = max(0, 1000 - len(rows))
    for row in evidence_rows[:remaining]:
        text = (
            f"DrugCentral evidence {row['source_id']} pair {row['pair_key']} {row['drug_a']} plus {row['drug_b']} "
            f"table {row['source_table']}."
        )
        rows.append(
            {
                "id": row["drugcentral_evidence_id"],
                "domain": "drugcentral_pair_evidence",
                "text": text,
                "bridge_terms": uniq([row["pair_key"], row["drug_a"], row["drug_b"], row["source_id"], row["source_table"]]),
                "metadata": {
                    "source_dataset": "issue1255_drugcentral_source_mining",
                    "source_path": row["source_path"],
                    "source_sha256": row["source_sha256"],
                    "pair_key": row["pair_key"],
                    "source_id": row["source_id"],
                    "source_table": row["source_table"],
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
    table_rows = {name: info.get("rows", 0) for name, info in source_inventory.items()}
    return {
        "schema_version": 1,
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "candidate_rows": len(candidates),
        "unique_pair_keys": len(pairs),
        "source_table_rows": table_rows,
        "drugcentral_evidence_rows": len(evidence_rows),
        "drugcentral_ddi_evidence_rows": sum(1 for row in evidence_rows if row["source_table"] == "ddi"),
        "drugcentral_same_structure_evidence_rows": sum(
            1 for row in evidence_rows if row["source_table"] == "structures_synonyms_identifier"
        ),
        "pair_status_rows": len(pair_status),
        "candidate_status_rows": len(candidate_status),
        "bridge_rows": len(bridge_rows),
        "bridge_evidence_rows_materialized": sum(1 for row in bridge_rows if row["domain"] == "drugcentral_pair_evidence"),
        "pair_status_counts": dict(sorted(Counter(row["drugcentral_pair_status"] for row in pair_status).items())),
        "candidate_status_counts": dict(sorted(Counter(row["drugcentral_candidate_status"] for row in candidate_status).items())),
        "all_rows_blocked": True,
    }


def build_input_manifest(
    inputs: dict[str, str],
    candidates: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    raw_sources: dict[str, dict[str, Any]],
    source_inventory: dict[str, dict[str, Any]],
    issue1254_persisted_readback: dict[str, Any],
    issue1254_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "issue": 1255,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": {
            "issue1254_candidate_status": artifact(Path(inputs["issue1254_candidate_status"]), jsonl=True),
            "issue1254_pair_status": artifact(Path(inputs["issue1254_pair_status"]), jsonl=True),
            "issue1254_persisted_readback": artifact(Path(inputs["issue1254_persisted_readback"])),
            "issue1254_calyx_readback": artifact(Path(inputs["issue1254_calyx_readback"])),
            "issue1254_output_manifest": artifact(Path(inputs["issue1254_output_manifest"])),
        },
        "raw_source_docs": raw_sources,
        "drugcentral_source_inventory": source_inventory,
        "source_contract": {
            "issue1254_persisted_assertions_all_true": all_assertions_true(issue1254_persisted_readback),
            "issue1254_calyx_assertions_all_true": all_assertions_true(issue1254_calyx_readback),
            "candidate_rows": len(candidates),
            "unique_pair_keys": len(pairs),
            "input_filter": "chembl_candidate_status in blocked ChEMBL no-hit/no-pair-match statuses",
            "drugcentral_ddi_hit_requires_structured_participant_match": True,
            "drugcentral_same_structure_hit_requires_shared_struct_id": True,
            "drugcentral_interaction_rows_are_blockers_not_safety_clearance": True,
            "credentials_persisted": False,
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
    issue1254_persisted_readback: dict[str, Any],
    issue1254_calyx_readback: dict[str, Any],
) -> dict[str, Any]:
    artifacts = {
        "drugcentral_source_rows": artifact(out_dir / "drugcentral_source_rows.jsonl", jsonl=True),
        "drugcentral_pair_evidence": artifact(out_dir / "drugcentral_pair_evidence.jsonl", jsonl=True),
        "drugcentral_pair_status": artifact(out_dir / "drugcentral_pair_status.jsonl", jsonl=True),
        "candidate_drugcentral_status": artifact(out_dir / "candidate_drugcentral_status.jsonl", jsonl=True),
        "drugcentral_bridge_rows": artifact(out_dir / "drugcentral_bridge_rows.jsonl", jsonl=True),
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
        "issue1254_persisted_readback_all_true": all_assertions_true(issue1254_persisted_readback),
        "issue1254_calyx_readback_all_true": all_assertions_true(issue1254_calyx_readback),
        "source_rows_present": len(source_rows) >= len(SNAPSHOT_TABLES),
        "pair_status_for_every_pair_key": pair_status_keys == candidate_pair_keys,
        "candidate_status_for_every_candidate": status_candidate_ids == candidate_ids,
        "all_hits_have_evidence": all(
            row["drugcentral_pair_status"]
            not in {
                "drugcentral_ddi_structured_hit_still_blocked",
                "drugcentral_same_structure_equivalence_hit_still_blocked",
            }
            or evidence_by_pair[row["pair_key"]] > 0
            for row in pair_status
        ),
        "all_ddi_evidence_rows_have_participant_match": all(
            row["source_table"] != "ddi"
            or (
                row.get("match", {}).get("drug_a_match", {}).get("matched")
                and row.get("match", {}).get("drug_b_match", {}).get("matched")
            )
            for row in evidence_rows
        ),
        "all_same_structure_evidence_rows_have_shared_struct": all(
            row["source_table"] != "structures_synonyms_identifier" or bool(row.get("shared_struct_id"))
            for row in evidence_rows
        ),
        "all_evidence_rows_have_source_hash": all(row.get("source_sha256") for row in evidence_rows),
        "all_pair_status_values_allowed": all(row["drugcentral_pair_status"] in PAIR_STATUS_VALUES for row in pair_status),
        "all_candidate_status_values_allowed": all(
            row["drugcentral_candidate_status"] in CANDIDATE_STATUS_VALUES for row in candidate_status
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
        "issue": 1255,
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

    issue1254_persisted_readback = read_json(Path(inputs["issue1254_persisted_readback"]))
    issue1254_calyx_readback = read_json(Path(inputs["issue1254_calyx_readback"]))
    source_candidates = rows_jsonl(Path(inputs["issue1254_candidate_status"]))
    candidates = load_candidates(source_candidates, max_pairs=max_pairs)
    pairs = pair_rows(candidates)

    raw_sources = fetch_raw_sources(raw_dir)
    source_inventory = snapshot_source_tables(raw_dir)
    source_rows = build_drugcentral_source_rows(raw_dir, source_inventory)
    write_jsonl(out_dir / "drugcentral_source_rows.jsonl", source_rows)

    name_index, struct_names = build_name_index(raw_dir)
    evidence_rows, pair_context = mine_pair_evidence(pairs, raw_dir, name_index, struct_names)
    evidence_by_pair: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in evidence_rows:
        evidence_by_pair[row["pair_key"]].append(row)

    pair_status_rows = [
        build_pair_status(pair, pair_context[pair["pair_key"]], evidence_by_pair[pair["pair_key"]])
        for pair in pairs
    ]
    pair_status_by_key = {row["pair_key"]: row for row in pair_status_rows}
    candidate_status_rows = [build_candidate_status(row, pair_status_by_key[row["pair_key"]]) for row in candidates]

    write_jsonl(out_dir / "drugcentral_pair_evidence.jsonl", evidence_rows)
    write_jsonl(out_dir / "drugcentral_pair_status.jsonl", pair_status_rows)
    write_jsonl(out_dir / "candidate_drugcentral_status.jsonl", candidate_status_rows)

    bridge_rows = build_bridge_rows(
        source_rows,
        candidate_status_rows,
        pair_status_rows,
        evidence_rows,
        out_dir / "candidate_drugcentral_status.jsonl",
        sha256_path(out_dir / "candidate_drugcentral_status.jsonl"),
    )
    write_jsonl(out_dir / "drugcentral_bridge_rows.jsonl", bridge_rows)

    input_manifest = build_input_manifest(
        inputs,
        candidates,
        pairs,
        raw_sources,
        source_inventory,
        issue1254_persisted_readback,
        issue1254_calyx_readback,
    )
    write_json(out_dir / "input_manifest.json", input_manifest)

    metrics = build_metrics(candidates, pairs, source_inventory, evidence_rows, pair_status_rows, candidate_status_rows, bridge_rows)
    write_json(out_dir / "validation_metrics.json", metrics)

    output_manifest = {
        "schema_version": 1,
        "issue": 1255,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "drugcentral_source_rows": artifact(out_dir / "drugcentral_source_rows.jsonl", jsonl=True),
            "drugcentral_pair_evidence": artifact(out_dir / "drugcentral_pair_evidence.jsonl", jsonl=True),
            "drugcentral_pair_status": artifact(out_dir / "drugcentral_pair_status.jsonl", jsonl=True),
            "candidate_drugcentral_status": artifact(out_dir / "candidate_drugcentral_status.jsonl", jsonl=True),
            "drugcentral_bridge_rows": artifact(out_dir / "drugcentral_bridge_rows.jsonl", jsonl=True),
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
        issue1254_persisted_readback,
        issue1254_calyx_readback,
    )
    write_json(out_dir / "persisted_readback.json", persisted_readback)

    final = {
        "status": persisted_readback["status"],
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "source_rows": artifact(out_dir / "drugcentral_source_rows.jsonl", jsonl=True),
            "pair_evidence": artifact(out_dir / "drugcentral_pair_evidence.jsonl", jsonl=True),
            "pair_status": artifact(out_dir / "drugcentral_pair_status.jsonl", jsonl=True),
            "candidate_status": artifact(out_dir / "candidate_drugcentral_status.jsonl", jsonl=True),
            "bridge_rows": artifact(out_dir / "drugcentral_bridge_rows.jsonl", jsonl=True),
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
