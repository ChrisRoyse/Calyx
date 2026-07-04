#!/usr/bin/env python3
"""#1224 neuropsychiatric target druggability evidence expansion.

This is a bounded evidence join, not a clinical claim generator. It builds a
target list from the repaired #1187 neuro hypothesis output, enriches it from
persisted DGIdb/Open Targets/molecular artifacts, optionally performs live
DGIdb/ChEMBL lookups, scans local BindingDB rows by UniProt accession, and
emits explicit no-hit/unavailable rows for every target/source gap.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


DEFAULT_NEURO_HYPOTHESES = (
    "/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/"
    "rerun_1187/out/neuro_hypotheses.jsonl"
)
DEFAULT_DGIDB_ROOT = "/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z"
DEFAULT_OPEN_TARGETS = (
    "/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/"
    "open_targets_association_rows.jsonl"
)
DEFAULT_MOLECULAR_ROOT = "/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z"
DEFAULT_BINDINGDB_ZIP = "/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDB_All_202606_tsv.zip"
DEFAULT_BINDINGDB_FASTA = "/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDBTargetSequences.fasta"

CLINICAL_BOUNDARY = (
    "Drug-target-disease mapping evidence only; not treatment, efficacy, safety, "
    "clinical actionability, dosing, recommendation, or cure evidence."
)

ISSUE_NAMED_TARGETS = [
    "DRD2",
    "DRD3",
    "DRD4",
    "HTR1A",
    "HTR2A",
    "HTR4",
    "MTHFR",
    "SHANK3",
    "RTN4R",
    "CACNA1I",
    "OPRM1",
    "OPRK1",
    "OPRD1",
]

USER_AGENT = "Calyx-Dev issue1224 bounded druggability expansion"


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def stable_id(*parts: object) -> str:
    payload = "\x1f".join(str(p) for p in parts)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:24]


def clean_symbol(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    text = text.replace("UNIPROT:", "").replace("HGNC:", "")
    text = re.sub(r"[^A-Za-z0-9_-]+", "", text)
    if not text:
        return None
    return text.upper()


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=False) + "\n")


def line_count(path: Path) -> int:
    with path.open(encoding="utf-8", errors="replace") as handle:
        return sum(1 for line in handle if line.strip())


def require_file(path: Path, label: str) -> None:
    if not path.exists() or not path.is_file():
        raise SystemExit(f"CALYX_ISSUE1224_MISSING_INPUT: {label}: {path}")


def source_hashes(paths: dict[str, Path]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for label, path in paths.items():
        if path.exists() and path.is_file():
            out[label] = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
        else:
            out[label] = {"path": str(path), "missing": True}
    return out


def http_json(url: str, raw_path: Path, data: dict[str, Any] | None = None) -> dict[str, Any]:
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    headers = {"User-Agent": USER_AGENT}
    body = None
    if data is not None:
        body = json.dumps(data, sort_keys=True).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=body, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            payload = response.read()
            status = getattr(response, "status", 200)
    except (urllib.error.URLError, TimeoutError) as exc:
        payload = json.dumps({"error": type(exc).__name__, "message": str(exc)}, sort_keys=True).encode("utf-8")
        status = 0
    raw_path.write_bytes(payload)
    try:
        obj = json.loads(payload.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        obj = {"error": "json_decode_failed", "raw_prefix": payload[:200].decode("utf-8", errors="replace")}
    obj["_issue1224_fetch"] = {
        "url": url,
        "status": status,
        "raw_path": str(raw_path),
        "raw_sha256": sha256_bytes(payload),
        "raw_bytes": len(payload),
        "fetched_at_unix": int(time.time()),
    }
    return obj


def add_target(
    targets: dict[str, dict[str, Any]],
    symbol: str | None,
    reason: str,
    row: dict[str, Any] | None = None,
    target_id: str | None = None,
) -> None:
    symbol = clean_symbol(symbol)
    if not symbol:
        return
    entry = targets.setdefault(
        symbol,
        {
            "schema_version": 1,
            "target_symbol": symbol,
            "reasons": [],
            "target_ids": [],
            "neuro_hypothesis_ids": [],
            "neuro_rank_min": None,
            "neuro_context_count": 0,
            "source_classes": [],
            "clinical_boundary": CLINICAL_BOUNDARY,
        },
    )
    if reason not in entry["reasons"]:
        entry["reasons"].append(reason)
    if target_id and target_id not in entry["target_ids"]:
        entry["target_ids"].append(target_id)
    if row is not None:
        hyp = row.get("hypothesis_id")
        if hyp and hyp not in entry["neuro_hypothesis_ids"]:
            entry["neuro_hypothesis_ids"].append(hyp)
        rank = row.get("rank")
        if isinstance(rank, int):
            entry["neuro_rank_min"] = rank if entry["neuro_rank_min"] is None else min(entry["neuro_rank_min"], rank)
        source_class = row.get("source_class")
        if source_class and source_class not in entry["source_classes"]:
            entry["source_classes"].append(source_class)
        entry["neuro_context_count"] += 1


def build_target_list(neuro_rows: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    targets: dict[str, dict[str, Any]] = {}
    disease_contexts: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for symbol in ISSUE_NAMED_TARGETS:
        add_target(targets, symbol, "issue1224_named_target")
    for row in neuro_rows:
        for endpoint_key in ("source", "target"):
            endpoint = row.get(endpoint_key)
            if not isinstance(endpoint, dict):
                continue
            endpoint_type = str(endpoint.get("type") or "")
            endpoint_name = endpoint.get("name")
            endpoint_id = str(endpoint.get("id") or "")
            if endpoint_type in {"gene", "gene_or_target", "protein", "gene_protein"} or endpoint_id.startswith("ENSG"):
                add_target(targets, endpoint_name, f"neuro_hypothesis_{endpoint_key}", row, endpoint_id)
        ext = row.get("external_validation") or {}
        for ot_row in ext.get("open_targets") or []:
            symbol = ot_row.get("query_target_symbol") or ot_row.get("target_name")
            add_target(targets, symbol, "neuro_open_targets_context", row, ot_row.get("target_id"))
            sym = clean_symbol(symbol)
            if sym:
                disease_contexts[sym].append(open_targets_context(ot_row, row.get("hypothesis_id")))
        for dgidb_row in ext.get("dgidb") or []:
            add_target(targets, dgidb_row.get("gene"), "neuro_dgidb_context", row, dgidb_row.get("gene_concept_id"))
    for target in targets.values():
        target["reasons"].sort()
        target["target_ids"].sort()
        target["source_classes"].sort()
        target["neuro_hypothesis_ids"] = sorted(target["neuro_hypothesis_ids"])[:100]
    return sorted(targets.values(), key=lambda r: (r["neuro_rank_min"] is None, r["neuro_rank_min"] or 10**9, r["target_symbol"])), disease_contexts


def open_targets_context(ot_row: dict[str, Any], hypothesis_id: str | None = None) -> dict[str, Any]:
    return {
        "source": "open_targets",
        "hypothesis_id": hypothesis_id,
        "target_symbol": ot_row.get("target_name") or ot_row.get("query_target_symbol"),
        "target_id": ot_row.get("target_id") or ot_row.get("query_target_id"),
        "disease_id": ot_row.get("disease_id"),
        "disease_name": ot_row.get("disease_name"),
        "score": ot_row.get("score"),
        "rank": ot_row.get("rank"),
        "source_query_type": ot_row.get("source_query_type"),
        "raw_sha256": ot_row.get("api_response_sha256"),
        "open_targets_data_version": ot_row.get("open_targets_data_version"),
    }


def load_open_targets_contexts(path: Path, target_rows: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    wanted = {row["target_symbol"] for row in target_rows}
    contexts: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in load_jsonl(path):
        symbol = clean_symbol(row.get("target_name") or row.get("query_target_symbol"))
        if symbol in wanted:
            contexts[symbol].append(open_targets_context(row))
    for symbol in contexts:
        contexts[symbol].sort(key=lambda r: (-(float(r.get("score") or 0.0)), int(r.get("rank") or 9999)))
    return contexts


def parse_dgidb_local(root: Path, target_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    wanted = {row["target_symbol"] for row in target_rows}
    parsed_root = root / "parsed"
    rows: list[dict[str, Any]] = []
    for filename in [
        "broad_graphql_interactions.jsonl",
        "seed_pair_graphql_interactions.jsonl",
        "dgidb_graph_edges.jsonl",
        "relevant_tsv_interactions.jsonl",
        "seed_pair_tsv_interactions.jsonl",
    ]:
        path = parsed_root / filename
        if not path.exists():
            continue
        for row in load_jsonl(path):
            gene = clean_symbol(row.get("gene") or row.get("gene_name"))
            if gene not in wanted:
                continue
            rows.append(normalize_dgidb_row(row, gene, "dgidb_persisted_artifact", filename))
    return rows


def normalize_dgidb_row(row: dict[str, Any], gene: str, source: str, filename: str) -> dict[str, Any]:
    interaction_types = row.get("interaction_types")
    if interaction_types is None and row.get("interaction_type"):
        interaction_types = [row.get("interaction_type")]
    drug_obj = row.get("drug") if isinstance(row.get("drug"), dict) else {}
    gene_obj = row.get("gene") if isinstance(row.get("gene"), dict) else {}
    source_dbs = row.get("source_dbs") or []
    if not source_dbs and row.get("interaction_source_db_name"):
        source_dbs = [row.get("interaction_source_db_name")]
    return {
        "schema_version": 1,
        "source": source,
        "source_file": filename,
        "target_symbol": gene,
        "target_concept_id": row.get("gene_concept_id") or gene_obj.get("conceptId"),
        "drug": row.get("drug_name") or row.get("drug") or drug_obj.get("name"),
        "drug_concept_id": row.get("drug_concept_id") or drug_obj.get("conceptId"),
        "interaction_id": row.get("dgidb_interaction_id") or row.get("id"),
        "interaction_score": float_or_none(row.get("interaction_score") or row.get("interactionScore")),
        "evidence_score": float_or_none(row.get("evidence_score") or row.get("evidenceScore")),
        "interaction_types": interaction_types or [],
        "source_dbs": source_dbs,
        "publication_pmids": row.get("publication_pmids") or [],
        "approved": parse_bool(row.get("approved") if "approved" in row else drug_obj.get("approved")),
        "anti_neoplastic": parse_bool(row.get("anti_neoplastic") if "anti_neoplastic" in row else drug_obj.get("antiNeoplastic")),
        "immunotherapy": parse_bool(row.get("immunotherapy") if "immunotherapy" in row else drug_obj.get("immunotherapy")),
        "raw_path": row.get("raw_path"),
        "raw_sha256": row.get("raw_sha256"),
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def parse_bool(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if value is None:
        return None
    text = str(value).strip().lower()
    if text in {"true", "1", "yes"}:
        return True
    if text in {"false", "0", "no"}:
        return False
    return None


def float_or_none(value: Any) -> float | None:
    if value in (None, "", "NULL"):
        return None
    try:
        out = float(str(value).strip())
    except ValueError:
        return None
    if not math.isfinite(out):
        return None
    return out


DGIDB_QUERY = """query($geneNames:[String!],$first:Int,$after:String){
  interactions(geneNames:$geneNames, first:$first, after:$after) {
    totalCount
    pageInfo { hasNextPage endCursor }
    nodes {
      id interactionScore evidenceScore
      interactionTypes { type directionality }
      gene { name conceptId }
      drug { name conceptId approved immunotherapy antiNeoplastic }
      sources { sourceDbName sourceDbVersion }
      publications { pmid }
    }
  }
}"""


def query_dgidb_live(target_rows: list[dict[str, Any]], out_dir: Path, page_size: int, max_records: int) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    rows: list[dict[str, Any]] = []
    request_records: list[dict[str, Any]] = []
    endpoint = "https://dgidb.org/api/graphql"
    for target in target_rows:
        symbol = target["target_symbol"]
        after = None
        seen = 0
        page = 0
        total = None
        while True:
            page += 1
            variables = {"geneNames": [symbol], "first": min(page_size, max_records - seen), "after": after}
            raw_path = out_dir / "raw" / "dgidb" / f"{symbol.lower()}_page{page:03}.json"
            obj = http_json(endpoint, raw_path, {"query": DGIDB_QUERY, "variables": variables})
            fetch = obj["_issue1224_fetch"]
            interactions = ((obj.get("data") or {}).get("interactions") or {})
            total = interactions.get("totalCount", total)
            nodes = interactions.get("nodes") or []
            request_records.append(
                {
                    "source": "dgidb_live_graphql",
                    "target_symbol": symbol,
                    "page": page,
                    "total_count": total,
                    "node_count": len(nodes),
                    "query_variables": variables,
                    **fetch,
                }
            )
            for node in nodes:
                flat = normalize_dgidb_live_node(node, symbol, fetch)
                rows.append(flat)
            seen += len(nodes)
            page_info = interactions.get("pageInfo") or {}
            if not page_info.get("hasNextPage") or not page_info.get("endCursor") or seen >= max_records or not nodes:
                if total is not None and seen < int(total):
                    request_records[-1]["truncated"] = True
                    request_records[-1]["truncated_at"] = seen
                break
            after = page_info.get("endCursor")
            time.sleep(0.1)
    return rows, request_records


def normalize_dgidb_live_node(node: dict[str, Any], symbol: str, fetch: dict[str, Any]) -> dict[str, Any]:
    sources = []
    for src in node.get("sources") or []:
        if src.get("sourceDbVersion"):
            sources.append(f"{src.get('sourceDbName')}:{src.get('sourceDbVersion')}")
        else:
            sources.append(src.get("sourceDbName"))
    return {
        "schema_version": 1,
        "source": "dgidb_live_graphql",
        "target_symbol": symbol,
        "target_concept_id": ((node.get("gene") or {}).get("conceptId")),
        "drug": ((node.get("drug") or {}).get("name")),
        "drug_concept_id": ((node.get("drug") or {}).get("conceptId")),
        "interaction_id": node.get("id"),
        "interaction_score": float_or_none(node.get("interactionScore")),
        "evidence_score": float_or_none(node.get("evidenceScore")),
        "interaction_types": [t.get("type") for t in node.get("interactionTypes") or [] if t.get("type")],
        "directionalities": [t.get("directionality") for t in node.get("interactionTypes") or [] if t.get("directionality")],
        "source_dbs": sources,
        "publication_pmids": [p.get("pmid") for p in node.get("publications") or [] if p.get("pmid")],
        "approved": parse_bool((node.get("drug") or {}).get("approved")),
        "anti_neoplastic": parse_bool((node.get("drug") or {}).get("antiNeoplastic")),
        "immunotherapy": parse_bool((node.get("drug") or {}).get("immunotherapy")),
        "raw_path": fetch["raw_path"],
        "raw_sha256": fetch["raw_sha256"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def query_chembl(target_rows: list[dict[str, Any]], out_dir: Path, mechanism_limit: int, activity_limit: int) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    target_hits: list[dict[str, Any]] = []
    mechanism_rows: list[dict[str, Any]] = []
    activity_rows: list[dict[str, Any]] = []
    request_records: list[dict[str, Any]] = []
    for target in target_rows:
        symbol = target["target_symbol"]
        query = urllib.parse.quote(symbol)
        raw_path = out_dir / "raw" / "chembl" / f"{symbol.lower()}_target_search.json"
        url = f"https://www.ebi.ac.uk/chembl/api/data/target/search.json?q={query}&limit=10"
        search = http_json(url, raw_path)
        request_records.append({"source": "chembl_target_search", "target_symbol": symbol, **search["_issue1224_fetch"]})
        exact_targets = []
        for candidate in search.get("targets") or []:
            hit = normalize_chembl_target(symbol, candidate, search["_issue1224_fetch"])
            target_hits.append(hit)
            if hit["is_human_single_protein"] and symbol in hit["gene_symbols"]:
                exact_targets.append(hit)
        for hit in exact_targets[:3]:
            target_id = hit["target_chembl_id"]
            mech_url = f"https://www.ebi.ac.uk/chembl/api/data/mechanism.json?target_chembl_id={target_id}&limit={mechanism_limit}"
            mech_raw = out_dir / "raw" / "chembl" / f"{symbol.lower()}_{target_id.lower()}_mechanism.json"
            mech = http_json(mech_url, mech_raw)
            request_records.append({"source": "chembl_mechanism", "target_symbol": symbol, "target_chembl_id": target_id, **mech["_issue1224_fetch"]})
            for row in mech.get("mechanisms") or []:
                mechanism_rows.append(normalize_chembl_mechanism(symbol, target_id, row, mech["_issue1224_fetch"]))
            act_url = (
                "https://www.ebi.ac.uk/chembl/api/data/activity.json?"
                f"target_chembl_id={target_id}&limit={activity_limit}&standard_units=nM"
            )
            act_raw = out_dir / "raw" / "chembl" / f"{symbol.lower()}_{target_id.lower()}_activity.json"
            act = http_json(act_url, act_raw)
            request_records.append({"source": "chembl_activity", "target_symbol": symbol, "target_chembl_id": target_id, **act["_issue1224_fetch"]})
            for row in act.get("activities") or []:
                activity_rows.append(normalize_chembl_activity(symbol, target_id, row, act["_issue1224_fetch"]))
            time.sleep(0.1)
    return target_hits, mechanism_rows, activity_rows, request_records


def normalize_chembl_target(symbol: str, row: dict[str, Any], fetch: dict[str, Any]) -> dict[str, Any]:
    gene_symbols = set()
    accessions = set()
    for comp in row.get("target_components") or []:
        if comp.get("accession"):
            accessions.add(comp["accession"])
        for syn in comp.get("target_component_synonyms") or []:
            if syn.get("syn_type") == "GENE_SYMBOL" and syn.get("component_synonym"):
                gene_symbols.add(clean_symbol(syn["component_synonym"]) or syn["component_synonym"])
    target_type = row.get("target_type")
    return {
        "schema_version": 1,
        "source": "chembl_target_search",
        "target_symbol": symbol,
        "target_chembl_id": row.get("target_chembl_id"),
        "pref_name": row.get("pref_name"),
        "target_type": target_type,
        "organism": row.get("organism"),
        "score": row.get("score"),
        "gene_symbols": sorted(gene_symbols),
        "uniprot_accessions": sorted(accessions),
        "is_human_single_protein": row.get("organism") == "Homo sapiens" and target_type == "SINGLE PROTEIN",
        "raw_path": fetch["raw_path"],
        "raw_sha256": fetch["raw_sha256"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def normalize_chembl_mechanism(symbol: str, target_id: str, row: dict[str, Any], fetch: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "source": "chembl_mechanism",
        "target_symbol": symbol,
        "target_chembl_id": target_id,
        "molecule_chembl_id": row.get("molecule_chembl_id"),
        "drug": row.get("molecule_pref_name"),
        "action_type": row.get("action_type"),
        "mechanism_of_action": row.get("mechanism_of_action"),
        "max_phase": row.get("max_phase"),
        "disease_efficacy": row.get("disease_efficacy"),
        "direct_interaction": row.get("direct_interaction"),
        "raw_path": fetch["raw_path"],
        "raw_sha256": fetch["raw_sha256"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def normalize_chembl_activity(symbol: str, target_id: str, row: dict[str, Any], fetch: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "source": "chembl_activity",
        "target_symbol": symbol,
        "target_chembl_id": target_id,
        "molecule_chembl_id": row.get("molecule_chembl_id"),
        "drug": row.get("molecule_pref_name"),
        "assay_chembl_id": row.get("assay_chembl_id"),
        "activity_id": row.get("activity_id"),
        "standard_type": row.get("standard_type"),
        "standard_relation": row.get("standard_relation"),
        "standard_value": float_or_none(row.get("standard_value")),
        "standard_units": row.get("standard_units"),
        "pchembl_value": float_or_none(row.get("pchembl_value")),
        "document_chembl_id": row.get("document_chembl_id"),
        "raw_path": fetch["raw_path"],
        "raw_sha256": fetch["raw_sha256"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def load_local_molecular(root: Path, wanted: set[str]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    bindingdb_rows: list[dict[str, Any]] = []
    chembl_rows: list[dict[str, Any]] = []
    activity_path = root / "bindingdb_activity_candidates.jsonl"
    if activity_path.exists():
        for row in load_jsonl(activity_path):
            label = clean_symbol(row.get("label"))
            if label in wanted:
                bindingdb_rows.append(normalize_bindingdb_row(label, row, "bindingdb_persisted_artifact", str(activity_path), sha256_path(activity_path)))
    specific = root / "bindingdb_specific_bridge_rows.json"
    if specific.exists():
        for row in json.loads(specific.read_text(encoding="utf-8", errors="replace")):
            symbol = "DPP4" if "Dipeptidyl peptidase 4" in str(row.get("Target Name")) else clean_symbol(row.get("label"))
            if symbol in wanted:
                bindingdb_rows.append(normalize_bindingdb_row(symbol, row, "bindingdb_specific_bridge_artifact", str(specific), sha256_path(specific)))
    for csv_name in ["chembl_activity_candidates.csv", "chembl_target_activity_counts.csv", "chembl_target_sequences.csv"]:
        path = root / csv_name
        if not path.exists():
            continue
        with path.open(newline="", encoding="utf-8", errors="replace") as handle:
            for row in csv.DictReader(handle):
                label = clean_symbol(row.get("label"))
                if label in wanted:
                    out = dict(row)
                    out.update(
                        {
                            "schema_version": 1,
                            "source": f"chembl_persisted_{csv_name}",
                            "target_symbol": label,
                            "raw_path": str(path),
                            "raw_sha256": sha256_path(path),
                            "clinical_boundary": CLINICAL_BOUNDARY,
                        }
                    )
                    chembl_rows.append(out)
    return bindingdb_rows, chembl_rows


def normalize_bindingdb_row(symbol: str | None, row: dict[str, Any], source: str, raw_path: str, raw_sha: str) -> dict[str, Any]:
    value, value_type = best_affinity(row)
    return {
        "schema_version": 1,
        "source": source,
        "target_symbol": symbol,
        "target_name": row.get("Target Name"),
        "uniprot_accession": row.get("UniProt (SwissProt) Primary ID of Target Chain 1"),
        "drug": row.get("BindingDB Ligand Name"),
        "ligand_chembl_id": row.get("ChEMBL ID of Ligand"),
        "pubchem_cid": row.get("PubChem CID"),
        "ligand_smiles": row.get("Ligand SMILES"),
        "bindingdb_reactant_set_id": row.get("BindingDB Reactant_set_id"),
        "row_index": row.get("row_index"),
        "affinity_nm": value,
        "affinity_type": value_type,
        "publication_pmids": compact_list([row.get("PMID")]),
        "publication_dois": compact_list([row.get("Article DOI")]),
        "raw_path": raw_path,
        "raw_sha256": raw_sha,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def compact_list(values: list[Any]) -> list[Any]:
    return [v for v in values if v not in (None, "", " ")]


def best_affinity(row: dict[str, Any]) -> tuple[float | None, str | None]:
    candidates = []
    for key in ["Kd (nM)", "Ki (nM)", "IC50 (nM)", "EC50 (nM)", "affinity_nm"]:
        value = float_or_none(row.get(key))
        if value is not None and value >= 0:
            candidates.append((value, key.replace(" (nM)", "")))
    if not candidates:
        return None, None
    return sorted(candidates, key=lambda item: item[0])[0]


def scan_bindingdb_zip(zip_path: Path, target_hits: list[dict[str, Any]], max_rows_per_target: int) -> list[dict[str, Any]]:
    accession_to_symbol: dict[str, str] = {}
    for hit in target_hits:
        if hit.get("is_human_single_protein"):
            for accession in hit.get("uniprot_accessions") or []:
                accession_to_symbol[accession] = hit["target_symbol"]
    if not accession_to_symbol:
        return []
    source_sha = sha256_path(zip_path)
    per_target: dict[str, list[dict[str, Any]]] = defaultdict(list)
    with zipfile.ZipFile(zip_path) as zf:
        tsv_name = next((name for name in zf.namelist() if name.lower().endswith(".tsv")), None)
        if tsv_name is None:
            return []
        with zf.open(tsv_name) as raw:
            text = (line.decode("utf-8", errors="replace") for line in raw)
            reader = csv.DictReader(text, delimiter="\t")
            for row_index, row in enumerate(reader, start=2):
                accessions = {
                    str(row.get(col) or "").strip()
                    for col in row.keys()
                    if col.startswith("UniProt (SwissProt) Primary ID of Target Chain")
                }
                for accession in accessions.intersection(accession_to_symbol):
                    symbol = accession_to_symbol[accession]
                    row["row_index"] = row_index
                    normalized = normalize_bindingdb_row(symbol, row, "bindingdb_tsv_accession_scan", str(zip_path), source_sha)
                    per_target[symbol].append(normalized)
                    if len(per_target[symbol]) > max_rows_per_target * 4:
                        per_target[symbol] = keep_best_binding_rows(per_target[symbol], max_rows_per_target)
    rows: list[dict[str, Any]] = []
    for symbol, vals in per_target.items():
        rows.extend(keep_best_binding_rows(vals, max_rows_per_target))
    return sorted(rows, key=lambda r: (r["target_symbol"] or "", r["affinity_nm"] is None, r["affinity_nm"] or 10**18, str(r.get("drug") or "")))


def keep_best_binding_rows(rows: list[dict[str, Any]], max_rows: int) -> list[dict[str, Any]]:
    rows = sorted(rows, key=lambda r: (r["affinity_nm"] is None, r["affinity_nm"] or 10**18, str(r.get("drug") or "")))
    return rows[:max_rows]


def build_approved_rows(dgidb_rows: list[dict[str, Any]], chembl_mechanism_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in dgidb_rows:
        if row.get("approved") is True:
            rows.append(
                {
                    "schema_version": 1,
                    "source": row["source"],
                    "target_symbol": row["target_symbol"],
                    "drug": row.get("drug"),
                    "drug_id": row.get("drug_concept_id"),
                    "approved_signal": "dgidb_drug_approved_true",
                    "interaction_type": row.get("interaction_types"),
                    "raw_path": row.get("raw_path"),
                    "raw_sha256": row.get("raw_sha256"),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    for row in chembl_mechanism_rows:
        if row.get("max_phase") == 4:
            rows.append(
                {
                    "schema_version": 1,
                    "source": "chembl_mechanism",
                    "target_symbol": row["target_symbol"],
                    "drug": row.get("drug"),
                    "drug_id": row.get("molecule_chembl_id"),
                    "approved_signal": "chembl_max_phase_4",
                    "interaction_type": row.get("action_type"),
                    "mechanism_of_action": row.get("mechanism_of_action"),
                    "raw_path": row.get("raw_path"),
                    "raw_sha256": row.get("raw_sha256"),
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    seen = set()
    unique = []
    for row in rows:
        key = (row["source"], row["target_symbol"], row.get("drug_id"), row.get("drug"))
        if key not in seen:
            seen.add(key)
            unique.append(row)
    return unique


def disease_context_map(*maps: dict[str, list[dict[str, Any]]]) -> dict[str, list[dict[str, Any]]]:
    merged: dict[str, list[dict[str, Any]]] = defaultdict(list)
    seen: set[tuple[str, str, str]] = set()
    for mp in maps:
        for symbol, rows in mp.items():
            for row in rows:
                key = (symbol, str(row.get("disease_id")), str(row.get("disease_name")))
                if key not in seen:
                    seen.add(key)
                    merged[symbol].append(row)
    for symbol in merged:
        merged[symbol].sort(key=lambda r: (-(float(r.get("score") or 0.0)), int(r.get("rank") or 9999), str(r.get("disease_name"))))
    return merged


def build_bridges(
    target_rows: list[dict[str, Any]],
    contexts: dict[str, list[dict[str, Any]]],
    dgidb_rows: list[dict[str, Any]],
    chembl_mechanisms: list[dict[str, Any]],
    bindingdb_rows: list[dict[str, Any]],
    max_per_target: int,
) -> list[dict[str, Any]]:
    evidence_by_target: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in dgidb_rows:
        if row.get("drug"):
            evidence_by_target[row["target_symbol"]].append(drug_evidence_from_dgidb(row))
    for row in chembl_mechanisms:
        if row.get("drug"):
            evidence_by_target[row["target_symbol"]].append(drug_evidence_from_chembl(row))
    for row in bindingdb_rows:
        if row.get("drug"):
            evidence_by_target[row["target_symbol"]].append(drug_evidence_from_bindingdb(row))
    bridges: list[dict[str, Any]] = []
    target_by_symbol = {row["target_symbol"]: row for row in target_rows}
    for symbol, evidence_rows in evidence_by_target.items():
        if not contexts.get(symbol):
            continue
        emitted = 0
        for disease in contexts[symbol][:10]:
            for evidence in sorted(evidence_rows, key=lambda r: -r["evidence_rank_score"]):
                bridge = {
                    "schema_version": 1,
                    "bridge_id": "issue1224:" + stable_id(symbol, disease.get("disease_id"), evidence.get("drug_id"), evidence.get("source")),
                    "target_symbol": symbol,
                    "target_ids": target_by_symbol.get(symbol, {}).get("target_ids", []),
                    "disease_id": disease.get("disease_id"),
                    "disease_name": disease.get("disease_name"),
                    "open_targets_score": disease.get("score"),
                    **evidence,
                    "rank_score": bridge_score(disease, evidence),
                    "derived_status": "drug_target_mapping_not_treatment_claim",
                    "clinical_boundary": CLINICAL_BOUNDARY,
                    "ambiguity": [
                        "target-disease association is not evidence that modulating the target treats the disease",
                        "drug-target mapping is not efficacy, safety, dosing, or clinical actionability evidence",
                    ],
                }
                bridges.append(bridge)
                emitted += 1
                if emitted >= max_per_target:
                    break
            if emitted >= max_per_target:
                break
    bridges.sort(key=lambda r: (-float(r["rank_score"]), r["target_symbol"], str(r.get("drug"))))
    return bridges


def drug_evidence_from_dgidb(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "drug_source": row["source"],
        "source": row["source"],
        "drug": row.get("drug"),
        "drug_id": row.get("drug_concept_id"),
        "interaction_type": row.get("interaction_types"),
        "publication_pmids": row.get("publication_pmids") or [],
        "source_hashes": compact_list([row.get("raw_sha256")]),
        "source_paths": compact_list([row.get("raw_path")]),
        "approved": row.get("approved"),
        "evidence_rank_score": (row.get("interaction_score") or 0.0) + 0.2 * (row.get("evidence_score") or 0.0) + (0.5 if row.get("approved") else 0.0),
    }


def drug_evidence_from_chembl(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "drug_source": "chembl_mechanism",
        "source": "chembl_mechanism",
        "drug": row.get("drug"),
        "drug_id": row.get("molecule_chembl_id"),
        "interaction_type": row.get("action_type"),
        "mechanism_of_action": row.get("mechanism_of_action"),
        "publication_pmids": [],
        "source_hashes": compact_list([row.get("raw_sha256")]),
        "source_paths": compact_list([row.get("raw_path")]),
        "approved": row.get("max_phase") == 4,
        "evidence_rank_score": 0.3 + (0.7 if row.get("max_phase") == 4 else 0.0) + (0.2 if row.get("direct_interaction") else 0.0),
    }


def drug_evidence_from_bindingdb(row: dict[str, Any]) -> dict[str, Any]:
    affinity = row.get("affinity_nm")
    affinity_score = 0.0 if affinity is None else max(0.0, min(2.0, -math.log10(max(float(affinity), 1e-12) / 1000.0)))
    return {
        "drug_source": row["source"],
        "source": row["source"],
        "drug": row.get("drug"),
        "drug_id": row.get("ligand_chembl_id") or row.get("pubchem_cid"),
        "interaction_type": row.get("affinity_type"),
        "binding_affinity_nm": affinity,
        "publication_pmids": row.get("publication_pmids") or [],
        "publication_dois": row.get("publication_dois") or [],
        "source_hashes": compact_list([row.get("raw_sha256")]),
        "source_paths": compact_list([row.get("raw_path")]),
        "approved": None,
        "evidence_rank_score": affinity_score,
    }


def bridge_score(disease: dict[str, Any], evidence: dict[str, Any]) -> float:
    disease_score = float(disease.get("score") or 0.0)
    return round(disease_score + float(evidence.get("evidence_rank_score") or 0.0), 6)


def no_hit_rows(
    target_rows: list[dict[str, Any]],
    contexts: dict[str, list[dict[str, Any]]],
    dgidb_rows: list[dict[str, Any]],
    chembl_mechanisms: list[dict[str, Any]],
    bindingdb_rows: list[dict[str, Any]],
    bindingdb_scanned: bool,
) -> list[dict[str, Any]]:
    counters = {
        "open_targets_context": Counter(row["target_symbol"] for symbol, rows in contexts.items() for row in rows for _ in [symbol]),
        "dgidb_interactions": Counter(row["target_symbol"] for row in dgidb_rows),
        "chembl_mechanisms": Counter(row["target_symbol"] for row in chembl_mechanisms),
        "bindingdb_rows": Counter(row["target_symbol"] for row in bindingdb_rows),
    }
    rows = []
    for target in target_rows:
        symbol = target["target_symbol"]
        for source, counter in counters.items():
            if counter[symbol]:
                continue
            status = "no_hit"
            if source == "bindingdb_rows" and not bindingdb_scanned:
                status = "unavailable_not_scanned"
            rows.append(
                {
                    "schema_version": 1,
                    "target_symbol": symbol,
                    "source": source,
                    "status": status,
                    "reason": "no source rows matched this target in the bounded #1224 run",
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return rows


def readback_artifacts(out_dir: Path, files: dict[str, Path], metrics: dict[str, Any]) -> dict[str, Any]:
    artifacts = {}
    for name, path in files.items():
        artifacts[name] = {
            "path": str(path),
            "exists": path.exists(),
            "bytes": path.stat().st_size if path.exists() else 0,
            "sha256": sha256_path(path) if path.exists() else None,
            "line_count": line_count(path) if path.suffix == ".jsonl" and path.exists() else None,
        }
    return {
        "schema_version": 1,
        "issue": 1224,
        "out_dir": str(out_dir),
        "metrics": metrics,
        "artifact_readback": artifacts,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--neuro-hypotheses", default=DEFAULT_NEURO_HYPOTHESES)
    parser.add_argument("--dgidb-root", default=DEFAULT_DGIDB_ROOT)
    parser.add_argument("--open-targets", default=DEFAULT_OPEN_TARGETS)
    parser.add_argument("--molecular-root", default=DEFAULT_MOLECULAR_ROOT)
    parser.add_argument("--bindingdb-zip", default=DEFAULT_BINDINGDB_ZIP)
    parser.add_argument("--bindingdb-fasta", default=DEFAULT_BINDINGDB_FASTA)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--dgidb-page-size", type=int, default=100)
    parser.add_argument("--dgidb-max-records-per-target", type=int, default=500)
    parser.add_argument("--chembl-mechanism-limit", type=int, default=100)
    parser.add_argument("--chembl-activity-limit", type=int, default=25)
    parser.add_argument("--bindingdb-max-rows-per-target", type=int, default=50)
    parser.add_argument("--skip-live", action="store_true")
    parser.add_argument("--skip-bindingdb-scan", action="store_true")
    args = parser.parse_args()

    neuro_path = Path(args.neuro_hypotheses)
    dgidb_root = Path(args.dgidb_root)
    open_targets_path = Path(args.open_targets)
    molecular_root = Path(args.molecular_root)
    bindingdb_zip = Path(args.bindingdb_zip)
    bindingdb_fasta = Path(args.bindingdb_fasta)
    out_dir = Path(args.out_dir)

    for label, path in {
        "neuro_hypotheses": neuro_path,
        "open_targets": open_targets_path,
        "bindingdb_zip": bindingdb_zip,
        "bindingdb_fasta": bindingdb_fasta,
    }.items():
        if label.startswith("bindingdb") and args.skip_bindingdb_scan:
            continue
        require_file(path, label)
    if not dgidb_root.exists():
        raise SystemExit(f"CALYX_ISSUE1224_MISSING_INPUT: dgidb_root: {dgidb_root}")
    if not molecular_root.exists():
        raise SystemExit(f"CALYX_ISSUE1224_MISSING_INPUT: molecular_root: {molecular_root}")

    out_dir.mkdir(parents=True, exist_ok=True)
    neuro_rows = load_jsonl(neuro_path)
    target_rows, neuro_contexts = build_target_list(neuro_rows)
    target_symbols = {row["target_symbol"] for row in target_rows}
    open_contexts = load_open_targets_contexts(open_targets_path, target_rows)
    contexts = disease_context_map(neuro_contexts, open_contexts)

    local_dgidb_rows = parse_dgidb_local(dgidb_root, target_rows)
    local_bindingdb_rows, local_chembl_rows = load_local_molecular(molecular_root, target_symbols)

    live_dgidb_rows: list[dict[str, Any]] = []
    dgidb_requests: list[dict[str, Any]] = []
    chembl_targets: list[dict[str, Any]] = []
    chembl_mechanisms: list[dict[str, Any]] = []
    chembl_activities: list[dict[str, Any]] = []
    chembl_requests: list[dict[str, Any]] = []
    if not args.skip_live:
        live_dgidb_rows, dgidb_requests = query_dgidb_live(target_rows, out_dir, args.dgidb_page_size, args.dgidb_max_records_per_target)
        chembl_targets, chembl_mechanisms, chembl_activities, chembl_requests = query_chembl(
            target_rows,
            out_dir,
            args.chembl_mechanism_limit,
            args.chembl_activity_limit,
        )

    scanned_bindingdb_rows: list[dict[str, Any]] = []
    if not args.skip_bindingdb_scan:
        scanned_bindingdb_rows = scan_bindingdb_zip(bindingdb_zip, chembl_targets, args.bindingdb_max_rows_per_target)

    all_dgidb_rows = local_dgidb_rows + live_dgidb_rows
    all_bindingdb_rows = local_bindingdb_rows + scanned_bindingdb_rows
    all_chembl_rows = local_chembl_rows + chembl_targets + chembl_mechanisms + chembl_activities
    approved_rows = build_approved_rows(all_dgidb_rows, chembl_mechanisms)
    bridge_rows = build_bridges(
        target_rows,
        contexts,
        all_dgidb_rows,
        chembl_mechanisms,
        all_bindingdb_rows,
        max_per_target=100,
    )
    no_hits = no_hit_rows(target_rows, contexts, all_dgidb_rows, chembl_mechanisms, all_bindingdb_rows, not args.skip_bindingdb_scan)

    files = {
        "target_input_list": out_dir / "target_input_list.jsonl",
        "open_targets_context_rows": out_dir / "open_targets_context_rows.jsonl",
        "dgidb_target_interactions": out_dir / "dgidb_target_interactions.jsonl",
        "dgidb_request_records": out_dir / "dgidb_request_records.jsonl",
        "chembl_target_rows": out_dir / "chembl_target_rows.jsonl",
        "chembl_mechanism_rows": out_dir / "chembl_mechanism_rows.jsonl",
        "chembl_activity_rows": out_dir / "chembl_activity_rows.jsonl",
        "chembl_request_records": out_dir / "chembl_request_records.jsonl",
        "molecular_source_hits": out_dir / "molecular_source_hits.jsonl",
        "approved_drug_mappings": out_dir / "approved_drug_mappings.jsonl",
        "drug_target_disease_bridge_candidates": out_dir / "drug_target_disease_bridge_candidates.jsonl",
        "no_hit_or_unavailable_targets": out_dir / "no_hit_or_unavailable_targets.jsonl",
    }

    context_rows = [row for rows in contexts.values() for row in rows]
    write_jsonl(files["target_input_list"], target_rows)
    write_jsonl(files["open_targets_context_rows"], context_rows)
    write_jsonl(files["dgidb_target_interactions"], all_dgidb_rows)
    write_jsonl(files["dgidb_request_records"], dgidb_requests)
    write_jsonl(files["chembl_target_rows"], chembl_targets)
    write_jsonl(files["chembl_mechanism_rows"], chembl_mechanisms)
    write_jsonl(files["chembl_activity_rows"], chembl_activities)
    write_jsonl(files["chembl_request_records"], chembl_requests)
    write_jsonl(files["molecular_source_hits"], all_bindingdb_rows + all_chembl_rows)
    write_jsonl(files["approved_drug_mappings"], approved_rows)
    write_jsonl(files["drug_target_disease_bridge_candidates"], bridge_rows)
    write_jsonl(files["no_hit_or_unavailable_targets"], no_hits)

    source_paths = {
        "neuro_hypotheses": neuro_path,
        "open_targets_rows": open_targets_path,
        "dgidb_broad_graphql": dgidb_root / "parsed" / "broad_graphql_interactions.jsonl",
        "dgidb_druggability": dgidb_root / "parsed" / "gene_druggability_rows.jsonl",
        "molecular_scaleout_rows": molecular_root / "molecular_scaleout_rows.jsonl",
        "bindingdb_zip": bindingdb_zip,
        "bindingdb_target_fasta": bindingdb_fasta,
    }
    raw_hashes = {
        str(path.relative_to(out_dir)): {"bytes": path.stat().st_size, "sha256": sha256_path(path)}
        for path in sorted((out_dir / "raw").rglob("*.json"))
    } if (out_dir / "raw").exists() else {}
    source_hash_path = out_dir / "source_hashes.json"
    write_json(source_hash_path, {"input_sources": source_hashes(source_paths), "live_raw_sources": raw_hashes})

    per_target_counts = {}
    for target in target_rows:
        symbol = target["target_symbol"]
        per_target_counts[symbol] = {
            "open_targets_context_rows": sum(1 for row in context_rows if clean_symbol(row.get("target_symbol")) == symbol),
            "dgidb_rows": sum(1 for row in all_dgidb_rows if row["target_symbol"] == symbol),
            "chembl_mechanism_rows": sum(1 for row in chembl_mechanisms if row["target_symbol"] == symbol),
            "bindingdb_rows": sum(1 for row in all_bindingdb_rows if row["target_symbol"] == symbol),
            "bridge_rows": sum(1 for row in bridge_rows if row["target_symbol"] == symbol),
            "no_hit_rows": sum(1 for row in no_hits if row["target_symbol"] == symbol),
        }
    metrics = {
        "schema_version": 1,
        "issue": 1224,
        "target_count": len(target_rows),
        "neuro_input_rows": len(neuro_rows),
        "open_targets_context_rows": len(context_rows),
        "local_dgidb_rows": len(local_dgidb_rows),
        "live_dgidb_rows": len(live_dgidb_rows),
        "chembl_target_rows": len(chembl_targets),
        "chembl_mechanism_rows": len(chembl_mechanisms),
        "chembl_activity_rows": len(chembl_activities),
        "local_bindingdb_rows": len(local_bindingdb_rows),
        "scanned_bindingdb_rows": len(scanned_bindingdb_rows),
        "approved_drug_mapping_rows": len(approved_rows),
        "bridge_candidate_rows": len(bridge_rows),
        "no_hit_or_unavailable_rows": len(no_hits),
        "bindingdb_scan_enabled": not args.skip_bindingdb_scan,
        "live_query_enabled": not args.skip_live,
        "per_target_counts": per_target_counts,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    metrics_path = out_dir / "validation_metrics.json"
    write_json(metrics_path, metrics)
    readback_path = out_dir / "persisted_readback.json"
    all_files = {**files, "source_hashes": source_hash_path, "validation_metrics": metrics_path}
    write_json(readback_path, readback_artifacts(out_dir, all_files, metrics))

    print(json.dumps({"summary": str(readback_path), "metrics": metrics}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
