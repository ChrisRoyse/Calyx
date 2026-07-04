#!/usr/bin/env python3
"""#1229 NCI ALMANAC external drug-combination evidence ingest.

This stage turns the CellMiner/NCI ALMANAC combo-score workbook into atomic
preclinical evidence rows, joins them to #1190 combination candidates, and
keeps all joined candidates fail-closed unless downstream safety/outcome gates
are present. It is not treatment guidance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import statistics
import time
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path
from typing import Any
from zipfile import ZipFile


CLINICAL_BOUNDARY = (
    "External drug-combination evidence is preclinical/research triage only; "
    "not efficacy, safety, clinical actionability, treatment guidance, dosing, "
    "recommendation, or cure evidence."
)

DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T132000Z"

DEFAULT_INPUTS = {
    "almanac_zip": f"{DEFAULT_ROOT}/raw/DTP_NCI60_ALMANAC_COMBO_SCORE.zip",
    "almanac_xlsx": f"{DEFAULT_ROOT}/raw/almanac_unzipped/output/DTP_NCI60_ALMANAC_COMBO_SCORE.xlsx",
    "candidate_pairs": "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/candidate_pair_inputs.jsonl",
    "combination_hypotheses": "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/drug_combination_hypotheses.jsonl",
    "drugcomb_pair_matches": "/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/drugcomb_pair_matches.jsonl",
}

ALMANAC_DOWNLOAD_URL = (
    "https://discover.nci.nih.gov/cellminer/download/processeddataset/"
    "DTP_NCI60_ALMANAC_COMBO_SCORE.zip"
)
ALMANAC_SOURCE_PAGE = "https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html"
ALMANAC_METADATA_PAGE = "https://discover.nci.nih.gov/cellminer/datasets.do"

XLSX_NS = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}


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


def numeric(value: object) -> float | None:
    try:
        result = float(value)
    except (TypeError, ValueError):
        return None
    return result if math.isfinite(result) else None


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


def col_index(cell_ref: str) -> int:
    letters = "".join(ch for ch in cell_ref if ch.isalpha())
    idx = 0
    for ch in letters:
        idx = idx * 26 + ord(ch.upper()) - 64
    return idx - 1


def shared_strings(zip_file: ZipFile) -> list[str]:
    if "xl/sharedStrings.xml" not in zip_file.namelist():
        return []
    root = ET.fromstring(zip_file.read("xl/sharedStrings.xml"))
    strings: list[str] = []
    for si in root.findall("m:si", XLSX_NS):
        strings.append("".join(t.text or "" for t in si.findall(".//m:t", XLSX_NS)))
    return strings


def cell_value(cell: ET.Element, strings: list[str]) -> str:
    value = cell.find("m:v", XLSX_NS)
    if value is None:
        return ""
    raw = value.text or ""
    if cell.attrib.get("t") == "s":
        return strings[int(raw)]
    if cell.attrib.get("t") == "str":
        return raw
    return raw


def read_sheet_rows(zip_file: ZipFile, sheet_path: str) -> list[tuple[int, dict[int, str]]]:
    strings = shared_strings(zip_file)
    root = ET.fromstring(zip_file.read(sheet_path))
    rows: list[tuple[int, dict[int, str]]] = []
    for row in root.findall("m:sheetData/m:row", XLSX_NS):
        cells: dict[int, str] = {}
        for cell in row.findall("m:c", XLSX_NS):
            cells[col_index(cell.attrib["r"])] = cell_value(cell, strings)
        rows.append((int(row.attrib["r"]), cells))
    return rows


def workbook_rows(path: Path) -> tuple[dict[str, str], list[str], list[dict[str, str]]]:
    with ZipFile(path) as zip_file:
        rows = read_sheet_rows(zip_file, "xl/worksheets/sheet1.xml")
    metadata: dict[str, str] = {}
    for row_num, cells in rows:
        if row_num > 5:
            break
        key = clean_text(cells.get(0))
        value = clean_text(cells.get(1))
        if key:
            metadata[key.rstrip(":")] = value
    header_row = next(
        cells
        for _, cells in rows
        if clean_text(cells.get(0)).startswith("NSC #1") and clean_text(cells.get(1)) == "Drug name"
    )
    headers = [clean_text(header_row.get(i)) for i in range(max(header_row) + 1)]
    data_rows: list[dict[str, str]] = []
    for row_num, cells in rows:
        if row_num <= 9 or not clean_text(cells.get(1)):
            continue
        row = {f"_col_{i}": clean_text(cells.get(i)) for i in range(len(headers))}
        for i, header in enumerate(headers):
            if i >= 8:
                row[header] = clean_text(cells.get(i))
        row["_source_row"] = str(row_num)
        data_rows.append(row)
    return metadata, headers, data_rows


def tissue_code(cell_line: str) -> str:
    return cell_line.split(":", 1)[0] if ":" in cell_line else ""


def score_class(score: float) -> str:
    if score > 0:
        return "positive_combo_score"
    if score < 0:
        return "negative_combo_score"
    return "zero_combo_score"


def summarize_pair(row: dict[str, str], score_headers: list[str]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    drug_a = clean_text(row.get("Drug name"))
    drug_b = clean_text(row.get("Drug name__2") or row.get("Drug name.2"))
    # The duplicate workbook headers are normalized by position below.
    drug_a = clean_text(row["_drug_a"])
    drug_b = clean_text(row["_drug_b"])
    pair = pair_key(drug_a, drug_b)
    pair_id = f"almanac:{stable_id(pair, row.get('_source_row'))}"
    score_rows: list[dict[str, Any]] = []
    scores: list[float] = []
    for cell in score_headers:
        score = numeric(row.get(cell))
        if score is None:
            continue
        scores.append(score)
        score_rows.append(
            {
                "schema_version": 1,
                "score_id": f"almanac-score:{stable_id(pair_id, cell)}",
                "pair_id": pair_id,
                "pair_key": pair,
                "drug_a": drug_a,
                "drug_b": drug_b,
                "drug_a_norm": norm_name(drug_a),
                "drug_b_norm": norm_name(drug_b),
                "cell_line": cell,
                "tissue_code": tissue_code(cell),
                "combo_score": score,
                "score_class": score_class(score),
                "source": "NCI_ALMANAC_CellMiner_combo_score",
                "source_row": int(row["_source_row"]),
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    positives = [score for score in scores if score > 0]
    top_positive = sorted(score_rows, key=lambda item: item["combo_score"], reverse=True)[:5]
    top_negative = sorted(score_rows, key=lambda item: item["combo_score"])[:3]
    summary = {
        "schema_version": 1,
        "pair_id": pair_id,
        "pair_key": pair,
        "source": "NCI_ALMANAC_CellMiner_combo_score",
        "source_row": int(row["_source_row"]),
        "nsc_a": row["_nsc_a"],
        "nsc_b": row["_nsc_b"],
        "drug_a": drug_a,
        "drug_b": drug_b,
        "drug_a_norm": norm_name(drug_a),
        "drug_b_norm": norm_name(drug_b),
        "fda_status_a": row["_fda_status_a"],
        "fda_status_b": row["_fda_status_b"],
        "mechanism_a": row["_mechanism_a"],
        "mechanism_b": row["_mechanism_b"],
        "observed_cell_line_count": len(scores),
        "positive_cell_line_count": len(positives),
        "max_combo_score": max(scores) if scores else None,
        "min_combo_score": min(scores) if scores else None,
        "mean_combo_score": round(statistics.fmean(scores), 6) if scores else None,
        "median_combo_score": round(statistics.median(scores), 6) if scores else None,
        "top_positive_examples": [
            {"cell_line": item["cell_line"], "combo_score": item["combo_score"]}
            for item in top_positive
        ],
        "top_negative_examples": [
            {"cell_line": item["cell_line"], "combo_score": item["combo_score"]}
            for item in top_negative
        ],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    return summary, score_rows


def normalized_almanac_rows(raw_rows: list[dict[str, str]], headers: list[str]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    score_headers = headers[8:]
    pair_rows: list[dict[str, Any]] = []
    score_rows: list[dict[str, Any]] = []
    for raw in raw_rows:
        positional = [raw.get(f"_col_{i}", "") for i in range(8)]
        raw["_nsc_a"] = positional[0]
        raw["_drug_a"] = positional[1]
        raw["_fda_status_a"] = positional[2]
        raw["_mechanism_a"] = positional[3]
        raw["_nsc_b"] = positional[4]
        raw["_drug_b"] = positional[5]
        raw["_fda_status_b"] = positional[6]
        raw["_mechanism_b"] = positional[7]
        summary, scores = summarize_pair(raw, score_headers)
        pair_rows.append(summary)
        score_rows.extend(scores)
    pair_rows.sort(key=lambda row: (-int(row["positive_cell_line_count"]), -(row["max_combo_score"] or -999999), row["pair_key"]))
    score_rows.sort(key=lambda row: (row["pair_key"], row["cell_line"]))
    return pair_rows, score_rows


def raw_names_match(candidate: dict[str, Any], source: dict[str, Any]) -> bool:
    candidate_names = {clean_text(candidate.get("drug_a")).lower(), clean_text(candidate.get("drug_b")).lower()}
    source_names = {clean_text(source.get("drug_a")).lower(), clean_text(source.get("drug_b")).lower()}
    return candidate_names == source_names


def drugcomb_raw_names_match(candidate: dict[str, Any], source: dict[str, Any]) -> bool:
    candidate_names = {clean_text(candidate.get("drug_a")).lower(), clean_text(candidate.get("drug_b")).lower()}
    for example in source.get("examples") or []:
        source_names = {clean_text(example.get("drug_row")).lower(), clean_text(example.get("drug_col")).lower()}
        if candidate_names == source_names:
            return True
    return False


def source_status(candidate: dict[str, Any], almanac: dict[str, Any] | None, drugcomb: dict[str, Any] | None) -> str:
    if almanac and raw_names_match(candidate, almanac):
        return "exact_hit"
    if drugcomb and drugcomb_raw_names_match(candidate, drugcomb):
        return "exact_hit"
    if almanac or drugcomb:
        return "normalized_hit"
    return "no_external_hit"


def index_drugcomb(rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for row in rows:
        key = clean_text(row.get("pair_key"))
        if not key:
            continue
        examples = row.get("examples") or []
        out[key] = row
    return out


def best_by_pair(pair_rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    best: dict[str, dict[str, Any]] = {}
    for row in pair_rows:
        current = best.get(row["pair_key"])
        if current is None:
            best[row["pair_key"]] = row
            continue
        if (row["positive_cell_line_count"], row["max_combo_score"] or -999999) > (
            current["positive_cell_line_count"],
            current["max_combo_score"] or -999999,
        ):
            best[row["pair_key"]] = row
    return best


def join_candidates(
    candidates: list[dict[str, Any]],
    hypotheses: list[dict[str, Any]],
    almanac_index: dict[str, dict[str, Any]],
    drugcomb_index: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    hypothesis_by_id = {row["pair_id"]: row for row in hypotheses}
    joined: list[dict[str, Any]] = []
    for candidate in candidates:
        key = candidate["pair_key"]
        almanac = almanac_index.get(key)
        drugcomb = drugcomb_index.get(key)
        old = hypothesis_by_id.get(candidate["pair_id"], {})
        status = source_status(candidate, almanac, drugcomb)
        reason_codes = list(old.get("reason_codes") or [])
        if status == "no_external_hit" and "external_synergy_evidence_missing_fail_closed" not in reason_codes:
            reason_codes.append("external_synergy_evidence_missing_fail_closed")
        if status != "no_external_hit" and "external_synergy_evidence_missing_fail_closed" in reason_codes:
            reason_codes = [code for code in reason_codes if code != "external_synergy_evidence_missing_fail_closed"]
            reason_codes.append("external_synergy_recheck_required_not_a_pass")
        combination_status = (
            "external_preclinical_hit_still_blocked"
            if status != "no_external_hit"
            else "blocked_no_external_synergy_evidence"
        )
        joined.append(
            {
                "schema_version": 1,
                "pair_id": candidate["pair_id"],
                "pair_key": key,
                "drug_a": candidate["drug_a"],
                "drug_b": candidate["drug_b"],
                "disease": candidate.get("disease"),
                "disease_area": candidate.get("disease_area"),
                "external_synergy_status": status,
                "combination_status": combination_status,
                "reason_codes": reason_codes,
                "almanac_match": bool(almanac),
                "drugcomb_match": bool(drugcomb),
                "almanac_summary": almanac,
                "drugcomb_summary": drugcomb,
                "prior_issue1190_status": old.get("combination_status"),
                "prior_issue1190_rank": old.get("rank"),
                "clinical_boundary": CLINICAL_BOUNDARY,
                "next_validation_experiment": next_validation(status, bool(almanac), bool(drugcomb), reason_codes),
            }
        )
    joined.sort(
        key=lambda row: (
            0 if row["external_synergy_status"] != "no_external_hit" else 1,
            row["disease_area"] or "",
            row["pair_key"],
        )
    )
    return joined


def next_validation(status: str, has_almanac: bool, has_drugcomb: bool, reasons: list[str]) -> str:
    if status == "no_external_hit":
        return "Acquire external pair-level synergy/model evidence before any combination promotion."
    if "component_safety_missing_fail_closed" in reasons:
        return "Complete component safety evidence; external preclinical hits do not clear safety."
    if "pair_interaction_evidence_missing_fail_closed" in reasons:
        return "Acquire exact pair interaction/pharmacology evidence before review."
    if has_almanac and has_drugcomb:
        return "Human reviewer may inspect concordant preclinical sources, then define outcome/safety gates."
    return "Human reviewer may inspect the single preclinical source and require independent replication."


def build_bridge_rows(rows: list[dict[str, Any]], source_path: Path, source_sha: str) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in rows[:1000]:
        terms = uniq([
            row["drug_a"],
            row["drug_b"],
            row.get("disease"),
            row.get("disease_area"),
            row["external_synergy_status"],
            row["combination_status"],
        ])
        text = (
            f"External combination evidence {row['pair_id']}: {row['drug_a']} plus "
            f"{row['drug_b']} for {row.get('disease')} has {row['external_synergy_status']} "
            f"from ALMANAC={row['almanac_match']} DrugComb={row['drugcomb_match']} and remains "
            f"{row['combination_status']}."
        )
        out.append(
            {
                "id": row["pair_id"],
                "domain": "external_drug_combination_evidence",
                "text": text,
                "bridge_terms": [term for term in terms if term and clean_text(term) in text],
                "metadata": {
                    "source_dataset": "issue1229_nci_almanac_external_synergy",
                    "source_path": str(source_path),
                    "source_sha256": source_sha,
                    "external_synergy_status": row["external_synergy_status"],
                    "combination_status": row["combination_status"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return out


def build_metrics(
    pair_rows: list[dict[str, Any]],
    score_rows: list[dict[str, Any]],
    candidates: list[dict[str, Any]],
    joined: list[dict[str, Any]],
) -> dict[str, Any]:
    status_counts = Counter(row["external_synergy_status"] for row in joined)
    reason_counts = Counter(reason for row in joined for reason in row["reason_codes"])
    area_counts = Counter(row.get("disease_area") for row in joined)
    return {
        "schema_version": 1,
        "status": "ok",
        "almanac_pair_rows": len(pair_rows),
        "almanac_cellline_score_rows": len(score_rows),
        "almanac_unique_pair_keys": len({row["pair_key"] for row in pair_rows}),
        "candidate_pair_rows": len(candidates),
        "joined_candidate_rows": len(joined),
        "candidate_rows_with_almanac_hit": sum(1 for row in joined if row["almanac_match"]),
        "candidate_rows_with_drugcomb_hit": sum(1 for row in joined if row["drugcomb_match"]),
        "candidate_rows_with_any_external_hit": sum(1 for row in joined if row["external_synergy_status"] != "no_external_hit"),
        "candidate_rows_with_both_sources": sum(1 for row in joined if row["almanac_match"] and row["drugcomb_match"]),
        "positive_almanac_pair_rows": sum(1 for row in pair_rows if row["positive_cell_line_count"] > 0),
        "status_counts": dict(status_counts),
        "reason_code_counts": dict(reason_counts),
        "disease_area_counts": dict(area_counts),
        "clinical_boundary_rows": sum(1 for row in joined if row["clinical_boundary"] == CLINICAL_BOUNDARY),
        "top_joined_rows": [
            {
                "pair_id": row["pair_id"],
                "drug_a": row["drug_a"],
                "drug_b": row["drug_b"],
                "disease": row.get("disease"),
                "external_synergy_status": row["external_synergy_status"],
                "almanac_match": row["almanac_match"],
                "drugcomb_match": row["drugcomb_match"],
                "combination_status": row["combination_status"],
            }
            for row in joined[:20]
        ],
    }


def input_manifest(inputs: dict[str, str], metadata: dict[str, str], headers: list[str]) -> dict[str, Any]:
    artifacts: dict[str, Any] = {}
    for name, raw in inputs.items():
        path = Path(raw)
        entry = artifact(path, jsonl=path.suffix == ".jsonl")
        if name == "almanac_zip":
            entry["md5"] = md5_path(path)
            entry["source_url"] = ALMANAC_DOWNLOAD_URL
            entry["source_page"] = ALMANAC_SOURCE_PAGE
        if name == "almanac_xlsx":
            entry["metadata_page"] = ALMANAC_METADATA_PAGE
            entry["workbook_metadata"] = metadata
            entry["header_count"] = len(headers)
            entry["score_column_count"] = max(0, len(headers) - 8)
        artifacts[name] = entry
    return {
        "schema_version": 1,
        "issue": 1229,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "inputs": artifacts,
        "source_notes": [
            {
                "source": "CellMiner/NCI ALMANAC DTP Almanac Combo Score",
                "role": "preclinical NCI-60 cell-line combination score evidence",
                "source_page": ALMANAC_SOURCE_PAGE,
                "download_url": ALMANAC_DOWNLOAD_URL,
                "metadata_page": ALMANAC_METADATA_PAGE,
            }
        ],
    }


def build_readback(
    out_dir: Path,
    pair_rows: list[dict[str, Any]],
    score_rows: list[dict[str, Any]],
    candidates: list[dict[str, Any]],
    joined: list[dict[str, Any]],
) -> dict[str, Any]:
    artifacts = {
        "almanac_pair_scores": artifact(out_dir / "almanac_pair_scores.jsonl", jsonl=True),
        "almanac_cellline_combo_scores": artifact(out_dir / "almanac_cellline_combo_scores.jsonl", jsonl=True),
        "candidate_external_synergy_status": artifact(out_dir / "candidate_external_synergy_status.jsonl", jsonl=True),
        "candidate_external_synergy_hits": artifact(out_dir / "candidate_external_synergy_hits.jsonl", jsonl=True),
        "external_synergy_bridge_rows": artifact(out_dir / "external_synergy_bridge_rows.jsonl", jsonl=True),
        "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        "output_manifest": artifact(out_dir / "output_manifest.json"),
    }
    return {
        "schema_version": 1,
        "issue": 1229,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "ok",
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": artifacts,
        "assertions": {
            "almanac_pairs_present": len(pair_rows) > 0,
            "almanac_scores_present": len(score_rows) > 0,
            "score_rows_cover_pairs": len({row["pair_key"] for row in score_rows}) == len({row["pair_key"] for row in pair_rows}),
            "joined_rows_match_candidates": len(joined) == len(candidates),
            "deterministic_status_for_every_candidate": all(row["external_synergy_status"] in {"exact_hit", "normalized_hit", "no_external_hit"} for row in joined),
            "all_joined_rows_have_boundary": all(row["clinical_boundary"] == CLINICAL_BOUNDARY for row in joined),
            "no_clinical_claim_rows": all("clinical" not in row["combination_status"] for row in joined),
            "bridge_rows_1000_or_less": artifacts["external_synergy_bridge_rows"]["rows"] == min(1000, len(joined)),
        },
        "row_counts": {
            "almanac_pair_rows": len(pair_rows),
            "almanac_cellline_score_rows": len(score_rows),
            "candidate_rows": len(candidates),
            "joined_rows": len(joined),
        },
    }


def run(root: Path, inputs: dict[str, str]) -> dict[str, Any]:
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)
    metadata, headers, raw_rows = workbook_rows(Path(inputs["almanac_xlsx"]))
    manifest = input_manifest(inputs, metadata, headers)
    write_json(out_dir / "input_manifest.json", manifest)
    pair_rows, score_rows = normalized_almanac_rows(raw_rows, headers)
    write_jsonl(out_dir / "almanac_pair_scores.jsonl", pair_rows)
    write_jsonl(out_dir / "almanac_cellline_combo_scores.jsonl", score_rows)

    candidates = rows_jsonl(Path(inputs["candidate_pairs"]))
    hypotheses = rows_jsonl(Path(inputs["combination_hypotheses"]))
    drugcomb_index = index_drugcomb(rows_jsonl(Path(inputs["drugcomb_pair_matches"])))
    joined = join_candidates(candidates, hypotheses, best_by_pair(pair_rows), drugcomb_index)
    hits = [row for row in joined if row["external_synergy_status"] != "no_external_hit"]
    write_jsonl(out_dir / "candidate_external_synergy_status.jsonl", joined)
    write_jsonl(out_dir / "candidate_external_synergy_hits.jsonl", hits)
    coverage = {
        "schema_version": 1,
        "issue": 1229,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "metrics": build_metrics(pair_rows, score_rows, candidates, joined),
    }
    write_json(out_dir / "external_source_coverage.json", coverage)
    source_path = out_dir / "candidate_external_synergy_status.jsonl"
    source_sha = sha256_path(source_path)
    bridge_rows = build_bridge_rows(joined, source_path, source_sha)
    write_jsonl(out_dir / "external_synergy_bridge_rows.jsonl", bridge_rows)
    metrics = build_metrics(pair_rows, score_rows, candidates, joined)
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "issue": 1229,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "input_manifest": artifact(out_dir / "input_manifest.json"),
            "almanac_pair_scores": artifact(out_dir / "almanac_pair_scores.jsonl", jsonl=True),
            "almanac_cellline_combo_scores": artifact(out_dir / "almanac_cellline_combo_scores.jsonl", jsonl=True),
            "candidate_external_synergy_status": artifact(out_dir / "candidate_external_synergy_status.jsonl", jsonl=True),
            "candidate_external_synergy_hits": artifact(out_dir / "candidate_external_synergy_hits.jsonl", jsonl=True),
            "external_source_coverage": artifact(out_dir / "external_source_coverage.json"),
            "external_synergy_bridge_rows": artifact(out_dir / "external_synergy_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, pair_rows, score_rows, candidates, joined)
    write_json(out_dir / "persisted_readback.json", readback)
    return {
        "status": "ok",
        "root": str(root),
        "metrics": metrics,
        "artifacts": {
            "joined_status": output_manifest["artifacts"]["candidate_external_synergy_status"],
            "hits": output_manifest["artifacts"]["candidate_external_synergy_hits"],
            "bridge_rows": output_manifest["artifacts"]["external_synergy_bridge_rows"],
            "persisted_readback": artifact(out_dir / "persisted_readback.json"),
        },
    }


def inputs_for_root(root: Path) -> dict[str, str]:
    inputs = dict(DEFAULT_INPUTS)
    inputs["almanac_zip"] = str(root / "raw" / "DTP_NCI60_ALMANAC_COMBO_SCORE.zip")
    inputs["almanac_xlsx"] = str(root / "raw" / "almanac_unzipped" / "output" / "DTP_NCI60_ALMANAC_COMBO_SCORE.xlsx")
    return inputs


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    root = Path(args.root)
    result = run(root, inputs_for_root(root))
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
