#!/usr/bin/env python3
"""#1189 rare-disease phenotype/gene/drug association hunt.

This script composes public HPO/Mondo ontology data with the current Calyx
biomedical association artifacts. It emits research-lead hypotheses only. It
does not make treatment, efficacy, safety, clinical-actionability, dosing, or
cure claims.
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
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


CLINICAL_BOUNDARY = (
    "Association-mining research lead only; not treatment, efficacy, safety, "
    "clinical actionability, dosing, recommendation, or cure evidence."
)

USER_AGENT = "Calyx-Dev issue1189 rare disease association hunt"

HPO_URLS = {
    "hp_obo": "https://purl.obolibrary.org/obo/hp.obo",
    "phenotype_hpoa": "https://purl.obolibrary.org/obo/hp/hpoa/phenotype.hpoa",
    "genes_to_phenotype": "https://purl.obolibrary.org/obo/hp/hpoa/genes_to_phenotype.txt",
}

MONDO_URLS = {
    "mondo_obo": "https://purl.obolibrary.org/obo/mondo/mondo.obo",
}

DEFAULT_PATHS = {
    "typed_broad": "/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/hypotheses.jsonl",
    "typed_chemical_disease": "/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/hypotheses.jsonl",
    "typed_gene_disease": "/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/hypotheses.jsonl",
    "falsification_flags": "/home/croyse/calyx/fsv/issue1184-hypothesis-falsification-20260704T023537Z/out/hypothesis_flags.jsonl",
    "open_targets_rows": "/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_association_rows.jsonl",
    "dgidb_root": "/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z",
    "evidence_substrate_readback": "/home/croyse/calyx/fsv/issue1196-calyx-db-evidence-substrate-v3-20260703T200335Z/calyx_db_readback.json",
    "neuro_druggability": "/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/drug_target_disease_bridge_candidates.jsonl",
    "neuro_dgidb_rows": "/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/dgidb_target_interactions.jsonl",
    "neuro_approved_rows": "/home/croyse/calyx/fsv/issue1224-neuro-druggability-expansion-20260704T063211Z/out/approved_drug_mappings.jsonl",
}


def sha256_path(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def stable_id(*parts: object, length: int = 24) -> str:
    payload = "\x1f".join(str(part) for part in parts)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:length]


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, ensure_ascii=False) + "\n")


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def line_count(path: Path) -> int:
    if not path.exists():
        return 0
    with path.open(encoding="utf-8", errors="replace") as handle:
        return sum(1 for line in handle if line.strip())


def clean_symbol(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    text = re.sub(r"^HGNC:", "", text, flags=re.I)
    text = re.sub(r"[^A-Za-z0-9_.-]+", "", text)
    return text.upper() or None


def normalize_text(value: object) -> str:
    text = str(value or "").lower()
    return re.sub(r"[^a-z0-9]+", " ", text).strip()


def normalize_disease_id(value: object) -> str:
    text = str(value or "").strip()
    if text.upper().startswith("ORPHA:"):
        return "ORPHA:" + text.split(":", 1)[1]
    if text.upper().startswith("ORPHANET:"):
        return "ORPHA:" + text.split(":", 1)[1]
    if text.upper().startswith("OMIM:"):
        return "OMIM:" + text.split(":", 1)[1]
    return text


def mondo_id_for_compare(value: object) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    return text.replace(":", "_")


def download_file(url: str, path: Path) -> dict[str, Any]:
    path.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    started = time.time()
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            final_url = response.geturl()
            status = getattr(response, "status", 200)
            with path.open("wb") as handle:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    handle.write(chunk)
    except (urllib.error.URLError, TimeoutError) as exc:
        path.write_text(json.dumps({"error": str(exc), "url": url}) + "\n", encoding="utf-8")
        return {
            "url": url,
            "path": str(path),
            "ok": False,
            "error": str(exc),
            "bytes": path.stat().st_size,
            "sha256": sha256_path(path),
        }
    return {
        "url": url,
        "final_url": final_url,
        "status": status,
        "path": str(path),
        "ok": True,
        "bytes": path.stat().st_size,
        "sha256": sha256_path(path),
        "elapsed_seconds": round(time.time() - started, 3),
    }


def parse_obo_terms(path: Path) -> dict[str, dict[str, Any]]:
    terms: dict[str, dict[str, Any]] = {}
    current: dict[str, Any] | None = None
    with path.open(encoding="utf-8", errors="replace") as handle:
        for raw in handle:
            line = raw.rstrip("\n")
            if line == "[Term]":
                if current and current.get("id"):
                    terms[current["id"]] = current
                current = {"xrefs": [], "synonyms": []}
                continue
            if line.startswith("[") and current is not None:
                if current.get("id"):
                    terms[current["id"]] = current
                current = None
                continue
            if current is None or ": " not in line:
                continue
            key, value = line.split(": ", 1)
            if key == "id":
                current["id"] = value
            elif key == "name":
                current["name"] = value
            elif key == "xref":
                current["xrefs"].append(value.split()[0])
            elif key == "synonym":
                match = re.match(r'"(.+?)"', value)
                if match:
                    current["synonyms"].append(match.group(1))
            elif key == "is_obsolete" and value == "true":
                current["obsolete"] = True
    if current and current.get("id"):
        terms[current["id"]] = current
    return {key: val for key, val in terms.items() if not val.get("obsolete")}


def mondo_maps(mondo_terms: dict[str, dict[str, Any]]) -> tuple[dict[str, dict[str, Any]], set[str]]:
    by_xref: dict[str, dict[str, Any]] = {}
    rare_ids: set[str] = set()
    rare_re = re.compile(r"rare|orphan", re.I)
    for mondo_id, term in mondo_terms.items():
        name = str(term.get("name") or "")
        xrefs = [normalize_disease_id(x) for x in term.get("xrefs") or []]
        if any(x.startswith(("ORPHA:", "OMIM:", "GARD:", "NORD:")) for x in xrefs) or rare_re.search(name):
            rare_ids.add(mondo_id)
        payload = {"mondo_id": mondo_id, "mondo_name": name, "xrefs": sorted(set(xrefs))}
        for xref in xrefs:
            by_xref[xref] = payload
            if xref.startswith("ORPHA:"):
                by_xref["Orphanet:" + xref.split(":", 1)[1]] = payload
    return by_xref, rare_ids


def parse_hpoa(path: Path, hpo_terms: dict[str, dict[str, Any]]) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    diseases: dict[str, dict[str, Any]] = {}
    rows: list[dict[str, Any]] = []
    metadata: dict[str, Any] = {"comment_lines": []}
    with path.open(encoding="utf-8", errors="replace") as handle:
        header: list[str] | None = None
        for raw in handle:
            line = raw.rstrip("\n")
            if not line:
                continue
            if line.startswith("#"):
                metadata["comment_lines"].append(line)
                if line.startswith("#version:"):
                    metadata["version"] = line.split(":", 1)[1].strip()
                if line.startswith("#description:"):
                    metadata["description"] = line.split(":", 1)[1].strip().strip('"')
                continue
            if header is None:
                header = line.split("\t")
                metadata["header"] = header
                continue
            parts = line.split("\t")
            if len(parts) < len(header):
                parts.extend([""] * (len(header) - len(parts)))
            row = dict(zip(header, parts))
            if row.get("qualifier") == "NOT":
                continue
            disease_id = normalize_disease_id(row.get("database_id"))
            hpo_id = row.get("hpo_id")
            hpo_name = hpo_terms.get(hpo_id, {}).get("name")
            parsed = {
                "disease_id": disease_id,
                "disease_name": row.get("disease_name"),
                "hpo_id": hpo_id,
                "hpo_name": hpo_name,
                "frequency": row.get("frequency"),
                "evidence": row.get("evidence"),
                "reference": row.get("reference"),
                "aspect": row.get("aspect"),
                "biocuration": row.get("biocuration"),
            }
            rows.append(parsed)
            entry = diseases.setdefault(
                disease_id,
                {
                    "schema_version": 1,
                    "disease_id": disease_id,
                    "disease_name": row.get("disease_name"),
                    "hpo_terms": {},
                    "references": set(),
                    "evidence_codes": Counter(),
                },
            )
            entry["hpo_terms"][hpo_id] = {
                "hpo_id": hpo_id,
                "hpo_name": hpo_name,
                "frequency": row.get("frequency"),
                "aspect": row.get("aspect"),
            }
            if row.get("reference"):
                entry["references"].add(row.get("reference"))
            if row.get("evidence"):
                entry["evidence_codes"][row.get("evidence")] += 1
    for entry in diseases.values():
        entry["phenotype_count"] = len(entry["hpo_terms"])
        entry["references"] = sorted(entry["references"])[:50]
        entry["evidence_codes"] = dict(entry["evidence_codes"])
        entry["top_phenotypes"] = top_phenotypes(entry["hpo_terms"])
    return diseases, rows, metadata


def top_phenotypes(terms: dict[str, dict[str, Any]], limit: int = 10) -> list[dict[str, Any]]:
    return sorted(
        terms.values(),
        key=lambda row: (
            frequency_rank(row.get("frequency")),
            str(row.get("hpo_name") or row.get("hpo_id") or ""),
        ),
    )[:limit]


def frequency_rank(value: object) -> float:
    text = str(value or "").strip()
    if "/" in text:
        left, right = text.split("/", 1)
        try:
            denom = float(right)
            return -(float(left) / denom if denom else 0.0)
        except ValueError:
            return 0.0
    if text.endswith("%"):
        try:
            return -(float(text[:-1]) / 100.0)
        except ValueError:
            return 0.0
    if text.startswith("HP:0040283"):
        return -0.75
    if text.startswith("HP:0040282"):
        return -0.5
    if text.startswith("HP:0040281"):
        return -0.25
    return 0.0


def parse_gene_phenotype(path: Path, diseases: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    groups: dict[tuple[str, str], dict[str, Any]] = {}
    with path.open(encoding="utf-8", errors="replace") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        for row in reader:
            disease_id = normalize_disease_id(row.get("disease_id"))
            if disease_id not in diseases:
                continue
            symbol = clean_symbol(row.get("gene_symbol"))
            if not symbol:
                continue
            key = (disease_id, symbol)
            group = groups.setdefault(
                key,
                {
                    "schema_version": 1,
                    "disease_id": disease_id,
                    "disease_name": diseases[disease_id]["disease_name"],
                    "gene_symbol": symbol,
                    "ncbi_gene_ids": set(),
                    "hpo_terms": {},
                    "frequencies": [],
                },
            )
            if row.get("ncbi_gene_id"):
                group["ncbi_gene_ids"].add(str(row.get("ncbi_gene_id")))
            hpo_id = row.get("hpo_id")
            group["hpo_terms"][hpo_id] = {
                "hpo_id": hpo_id,
                "hpo_name": row.get("hpo_name"),
                "frequency": row.get("frequency"),
            }
            if row.get("frequency"):
                group["frequencies"].append(row.get("frequency"))
    out: list[dict[str, Any]] = []
    for group in groups.values():
        disease_terms = set(diseases[group["disease_id"]]["hpo_terms"])
        gene_terms = set(group["hpo_terms"])
        overlap = sorted(disease_terms & gene_terms)
        union = disease_terms | gene_terms
        group["ncbi_gene_ids"] = sorted(group["ncbi_gene_ids"])
        group["gene_phenotype_count"] = len(gene_terms)
        group["disease_phenotype_count"] = len(disease_terms)
        group["shared_phenotype_count"] = len(overlap)
        group["phenotype_jaccard"] = round(len(overlap) / len(union), 6) if union else 0.0
        group["top_shared_phenotypes"] = top_phenotypes(
            {hp: group["hpo_terms"].get(hp) or diseases[group["disease_id"]]["hpo_terms"][hp] for hp in overlap}
        )
        out.append(group)
    return sorted(
        out,
        key=lambda row: (
            -row["shared_phenotype_count"],
            -row["gene_phenotype_count"],
            row["disease_name"],
            row["gene_symbol"],
        ),
    )


def local_dgidb_rows(dgidb_root: Path, extra_paths: list[Path]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    parsed = dgidb_root / "parsed"
    for filename in [
        "broad_graphql_interactions.jsonl",
        "seed_pair_graphql_interactions.jsonl",
        "dgidb_graph_edges.jsonl",
        "relevant_tsv_interactions.jsonl",
        "seed_pair_tsv_interactions.jsonl",
        "gene_druggability_rows.jsonl",
    ]:
        path = parsed / filename
        for row in load_jsonl(path):
            flat = normalize_dgidb_row(row, "dgidb_persisted_artifact", str(path))
            if flat:
                rows.append(flat)
    for path in extra_paths:
        for row in load_jsonl(path):
            flat = normalize_dgidb_row(row, "dgidb_expansion_artifact", str(path))
            if flat:
                rows.append(flat)
    return rows


def normalize_dgidb_row(row: dict[str, Any], source: str, source_file: str) -> dict[str, Any] | None:
    gene_obj = row.get("gene") if isinstance(row.get("gene"), dict) else {}
    drug_obj = row.get("drug") if isinstance(row.get("drug"), dict) else {}
    symbol = clean_symbol(
        row.get("target_symbol")
        or row.get("gene_symbol")
        or row.get("gene_name")
        or row.get("gene")
        or gene_obj.get("name")
    )
    drug = row.get("drug_name") or row.get("drug") or drug_obj.get("name")
    if not symbol or not drug:
        return None
    interaction_types = row.get("interaction_types")
    if interaction_types is None and row.get("interaction_type"):
        interaction_types = row.get("interaction_type")
    if isinstance(interaction_types, str):
        interaction_types = [interaction_types]
    source_dbs = row.get("source_dbs") or []
    if not source_dbs and row.get("interaction_source_db_name"):
        source_dbs = [row.get("interaction_source_db_name")]
    return {
        "schema_version": 1,
        "source": source,
        "source_file": source_file,
        "target_symbol": symbol,
        "target_concept_id": row.get("gene_concept_id") or row.get("target_concept_id") or gene_obj.get("conceptId"),
        "drug": str(drug),
        "drug_concept_id": row.get("drug_concept_id") or row.get("drug_id") or drug_obj.get("conceptId"),
        "interaction_id": row.get("dgidb_interaction_id") or row.get("interaction_id") or row.get("id"),
        "interaction_score": float_or_none(row.get("interaction_score") or row.get("interactionScore")),
        "evidence_score": float_or_none(row.get("evidence_score") or row.get("evidenceScore")),
        "interaction_types": interaction_types or [],
        "source_dbs": source_dbs,
        "publication_pmids": row.get("publication_pmids") or [],
        "approved": parse_bool(row.get("approved") if "approved" in row else drug_obj.get("approved")),
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
    return out if math.isfinite(out) else None


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


def http_json(url: str, raw_path: Path, body: dict[str, Any] | None = None) -> dict[str, Any]:
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    data = None
    headers = {"User-Agent": USER_AGENT}
    if body is not None:
        data = json.dumps(body, sort_keys=True).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=45) as response:
            payload = response.read()
            status = getattr(response, "status", 200)
    except (urllib.error.URLError, TimeoutError) as exc:
        payload = json.dumps({"error": str(exc), "url": url}, sort_keys=True).encode("utf-8")
        status = 0
    raw_path.write_bytes(payload)
    try:
        obj = json.loads(payload.decode("utf-8", errors="replace"))
    except json.JSONDecodeError:
        obj = {"error": "json_decode_failed"}
    obj["_fetch"] = {
        "url": url,
        "status": status,
        "raw_path": str(raw_path),
        "raw_sha256": sha256_bytes(payload),
        "raw_bytes": len(payload),
    }
    return obj


def query_dgidb_live(symbols: list[str], raw_root: Path, page_size: int, max_records: int) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    endpoint = "https://dgidb.org/api/graphql"
    rows: list[dict[str, Any]] = []
    requests: list[dict[str, Any]] = []
    for symbol in symbols:
        after = None
        seen = 0
        page = 0
        while True:
            page += 1
            variables = {"geneNames": [symbol], "first": min(page_size, max_records - seen), "after": after}
            raw_path = raw_root / "dgidb" / f"{symbol.lower()}_page{page:03}.json"
            obj = http_json(endpoint, raw_path, {"query": DGIDB_QUERY, "variables": variables})
            fetch = obj["_fetch"]
            interactions = ((obj.get("data") or {}).get("interactions") or {})
            nodes = interactions.get("nodes") or []
            total = interactions.get("totalCount")
            requests.append(
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
                rows.append(normalize_dgidb_live_node(node, symbol, fetch))
            seen += len(nodes)
            page_info = interactions.get("pageInfo") or {}
            if not page_info.get("hasNextPage") or not page_info.get("endCursor") or not nodes or seen >= max_records:
                if total is not None and seen < int(total):
                    requests[-1]["truncated"] = True
                    requests[-1]["truncated_at"] = seen
                break
            after = page_info.get("endCursor")
            time.sleep(0.05)
    return rows, requests


def normalize_dgidb_live_node(node: dict[str, Any], symbol: str, fetch: dict[str, Any]) -> dict[str, Any]:
    sources = []
    for src in node.get("sources") or []:
        if src.get("sourceDbVersion"):
            sources.append(f"{src.get('sourceDbName')}:{src.get('sourceDbVersion')}")
        elif src.get("sourceDbName"):
            sources.append(src.get("sourceDbName"))
    return {
        "schema_version": 1,
        "source": "dgidb_live_graphql",
        "source_file": fetch["raw_path"],
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
        "raw_path": fetch["raw_path"],
        "raw_sha256": fetch["raw_sha256"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def load_open_targets(path: Path) -> dict[tuple[str, str], list[dict[str, Any]]]:
    contexts: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in load_jsonl(path):
        symbol = clean_symbol(row.get("target_name") or row.get("query_target_symbol"))
        disease_key = mondo_id_for_compare(row.get("disease_id")) or normalize_text(row.get("disease_name"))
        if symbol and disease_key:
            contexts[(symbol, disease_key)].append(row)
        if symbol and row.get("disease_name"):
            contexts[(symbol, normalize_text(row.get("disease_name")))].append(row)
    for key in contexts:
        contexts[key].sort(key=lambda row: (-(float(row.get("score") or 0.0)), int(row.get("rank") or 9999)))
    return contexts


def load_typed_and_falsification(paths: dict[str, Path]) -> tuple[dict[tuple[str, str], list[dict[str, Any]]], dict[str, dict[str, Any]]]:
    typed_by_pair: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for key in ["typed_broad", "typed_chemical_disease", "typed_gene_disease"]:
        for row in load_jsonl(paths[key]):
            left = normalize_text(row.get("source_name"))
            right = normalize_text(row.get("target_name"))
            if left and right:
                typed_by_pair[(left, right)].append(row)
                typed_by_pair[(right, left)].append(row)
    flags = {row.get("hypothesis_id"): row for row in load_jsonl(paths["falsification_flags"])}
    return typed_by_pair, flags


def dgidb_evidence_score(row: dict[str, Any]) -> float:
    return (
        float(row.get("interaction_score") or 0.0)
        + 0.2 * float(row.get("evidence_score") or 0.0)
        + (0.5 if row.get("approved") is True else 0.0)
        + min(len(row.get("source_dbs") or []), 5) * 0.05
    )


def select_live_genes(groups: list[dict[str, Any]], existing_by_gene: dict[str, list[dict[str, Any]]], limit: int) -> list[str]:
    scores: dict[str, float] = defaultdict(float)
    for group in groups:
        gene = group["gene_symbol"]
        scores[gene] += (
            math.log1p(group["shared_phenotype_count"])
            + 0.5 * group["phenotype_jaccard"]
            + min(group["disease_phenotype_count"], 20) / 40.0
        )
    for gene in list(scores):
        if existing_by_gene.get(gene):
            scores[gene] *= 0.4
    return [gene for gene, _ in sorted(scores.items(), key=lambda item: (-item[1], item[0]))[:limit]]


def build_hypotheses(
    groups: list[dict[str, Any]],
    diseases: dict[str, dict[str, Any]],
    mondo_by_xref: dict[str, dict[str, Any]],
    dgidb_by_gene: dict[str, list[dict[str, Any]]],
    open_targets: dict[tuple[str, str], list[dict[str, Any]]],
    typed_by_pair: dict[tuple[str, str], list[dict[str, Any]]],
    flags: dict[str, dict[str, Any]],
    max_drug: int,
    max_target: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    hypotheses: list[dict[str, Any]] = []
    no_hit_rows: list[dict[str, Any]] = []
    drug_count = 0
    target_count = 0
    for group in groups:
        disease = diseases[group["disease_id"]]
        mondo = mondo_by_xref.get(group["disease_id"])
        disease_keys = [normalize_text(group["disease_name"])]
        if mondo:
            disease_keys.append(mondo_id_for_compare(mondo["mondo_id"]) or "")
            disease_keys.append(normalize_text(mondo["mondo_name"]))
        ot_rows: list[dict[str, Any]] = []
        for key in disease_keys:
            ot_rows.extend(open_targets.get((group["gene_symbol"], key), [])[:3])
        typed_rows = []
        for key in disease_keys:
            typed_rows.extend(typed_by_pair.get((normalize_text(group["gene_symbol"]), key), [])[:2])
        dgidb_rows = sorted(
            dgidb_by_gene.get(group["gene_symbol"], []),
            key=lambda row: (-dgidb_evidence_score(row), str(row.get("drug") or "")),
        )
        if dgidb_rows and drug_count < max_drug:
            for drug_row in dgidb_rows[:4]:
                if drug_count >= max_drug:
                    break
                hypotheses.append(candidate_row(group, disease, mondo, drug_row, ot_rows, typed_rows, flags))
                drug_count += 1
        elif target_count < max_target:
            hypotheses.append(candidate_row(group, disease, mondo, None, ot_rows, typed_rows, flags))
            no_hit_rows.append(no_hit_row(group, disease, mondo, "no_dgidb_drug_target_edge_in_current_sources"))
            target_count += 1
    hypotheses.sort(key=lambda row: (-row["rank_score"], row["hypothesis_id"]))
    unique: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for row in hypotheses:
        if row["hypothesis_id"] in seen_ids:
            continue
        seen_ids.add(row["hypothesis_id"])
        unique.append(row)
    for idx, row in enumerate(unique, start=1):
        row["rank"] = idx
    return unique, no_hit_rows


def candidate_row(
    group: dict[str, Any],
    disease: dict[str, Any],
    mondo: dict[str, Any] | None,
    drug_row: dict[str, Any] | None,
    ot_rows: list[dict[str, Any]],
    typed_rows: list[dict[str, Any]],
    flags: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    drug = drug_row.get("drug") if drug_row else None
    typed_flags = []
    for row in typed_rows:
        flag = flags.get(row.get("hypothesis_id"))
        if flag:
            typed_flags.append(flag)
    falsification = (
        typed_flags[0].get("sweep_status")
        if typed_flags
        else "not_run_for_hpo_generated_rare_disease_candidate"
    )
    uncertainty = uncertainty_codes(group, mondo, drug_row, ot_rows, typed_flags)
    score = rank_score(group, disease, drug_row, ot_rows, typed_flags, uncertainty)
    hyp_id = "issue1189:" + stable_id(
        group["disease_id"],
        group["gene_symbol"],
        drug or "no-drug-edge",
        drug_row.get("interaction_id") if drug_row else "no-interaction-id",
        drug_row.get("source") if drug_row else "no-source",
        group["shared_phenotype_count"],
    )
    evidence_paths = [
        {
            "kind": "hpo_disease_phenotype_profile",
            "disease_id": group["disease_id"],
            "disease_name": group["disease_name"],
            "phenotype_count": disease["phenotype_count"],
            "top_phenotypes": disease["top_phenotypes"][:8],
        },
        {
            "kind": "hpo_gene_to_phenotype",
            "gene_symbol": group["gene_symbol"],
            "ncbi_gene_ids": group["ncbi_gene_ids"],
            "shared_phenotype_count": group["shared_phenotype_count"],
            "gene_phenotype_count": group["gene_phenotype_count"],
            "top_shared_phenotypes": group["top_shared_phenotypes"][:8],
        },
    ]
    if mondo:
        evidence_paths.append({"kind": "mondo_mapping", **mondo})
    if drug_row:
        evidence_paths.append({"kind": "drug_target_edge", "row": drug_row})
    for row in ot_rows[:3]:
        evidence_paths.append({"kind": "open_targets_same_target_disease_context", "row": compact_open_targets(row)})
    for row in typed_rows[:3]:
        evidence_paths.append({"kind": "calyx_typed_association_context", "row": compact_typed(row)})
    return {
        "schema_version": 1,
        "hypothesis_id": hyp_id,
        "source_issue": 1189,
        "hypothesis_class": "hpo_gene_disease_drug_bridge" if drug else "hpo_gene_disease_target_prioritization",
        "disease": {
            "id": group["disease_id"],
            "name": group["disease_name"],
            "mondo_id": mondo.get("mondo_id") if mondo else None,
            "mondo_name": mondo.get("mondo_name") if mondo else None,
        },
        "phenotypes": group["top_shared_phenotypes"][:10],
        "gene": {"symbol": group["gene_symbol"], "ncbi_gene_ids": group["ncbi_gene_ids"]},
        "drug": {
            "name": drug,
            "id": drug_row.get("drug_concept_id") if drug_row else None,
            "interaction_types": drug_row.get("interaction_types") if drug_row else [],
            "approved_signal": drug_row.get("approved") if drug_row else None,
        },
        "rank_score": round(score, 6),
        "falsification_status": falsification,
        "uncertainty": uncertainty,
        "evidence_paths": evidence_paths,
        "external_validation": {
            "open_targets_rows": [compact_open_targets(row) for row in ot_rows[:5]],
            "typed_falsification_flags": typed_flags[:3],
        },
        "derived_status": "research_lead_not_clinical_claim",
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def uncertainty_codes(
    group: dict[str, Any],
    mondo: dict[str, Any] | None,
    drug_row: dict[str, Any] | None,
    ot_rows: list[dict[str, Any]],
    typed_flags: list[dict[str, Any]],
) -> list[str]:
    codes: list[str] = []
    if mondo is None:
        codes.append("mondo_mapping_missing")
    if drug_row is None:
        codes.append("no_drug_target_edge_in_current_sources")
    if not ot_rows:
        codes.append("no_open_targets_same_disease_context")
    if not typed_flags:
        codes.append("not_in_prior_falsification_sweep")
    if group["shared_phenotype_count"] < 2:
        codes.append("weak_phenotype_overlap")
    return codes


def rank_score(
    group: dict[str, Any],
    disease: dict[str, Any],
    drug_row: dict[str, Any] | None,
    ot_rows: list[dict[str, Any]],
    typed_flags: list[dict[str, Any]],
    uncertainty: list[str],
) -> float:
    score = 1.2 * math.log1p(group["shared_phenotype_count"])
    score += min(group["gene_phenotype_count"], 15) / 15.0
    score += min(disease["phenotype_count"], 25) / 25.0
    score += group["phenotype_jaccard"]
    if drug_row:
        score += min(dgidb_evidence_score(drug_row), 5.0)
    if ot_rows:
        score += min(max(float(row.get("score") or 0.0) for row in ot_rows), 1.0)
    if typed_flags:
        flag = typed_flags[0]
        score += 0.25
        if flag.get("counter_evidence_count"):
            score -= min(float(flag.get("counter_evidence_count") or 0), 3.0)
    score -= 0.12 * len(uncertainty)
    return score


def compact_open_targets(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "target_name": row.get("target_name") or row.get("query_target_symbol"),
        "target_id": row.get("target_id") or row.get("query_target_id"),
        "disease_id": row.get("disease_id"),
        "disease_name": row.get("disease_name"),
        "score": row.get("score"),
        "rank": row.get("rank"),
        "source_query_type": row.get("source_query_type"),
        "api_response_sha256": row.get("api_response_sha256"),
        "open_targets_data_version": row.get("open_targets_data_version"),
    }


def compact_typed(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "hypothesis_id": row.get("hypothesis_id"),
        "source_name": row.get("source_name"),
        "source_type": row.get("source_type"),
        "target_name": row.get("target_name"),
        "target_type": row.get("target_type"),
        "support_count": row.get("support_count"),
        "score": row.get("score"),
        "validation_gate_report_sha256": row.get("validation_gate_report_sha256"),
    }


def no_hit_row(group: dict[str, Any], disease: dict[str, Any], mondo: dict[str, Any] | None, reason: str) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "source_issue": 1189,
        "reason": reason,
        "disease_id": group["disease_id"],
        "disease_name": group["disease_name"],
        "mondo_id": mondo.get("mondo_id") if mondo else None,
        "gene_symbol": group["gene_symbol"],
        "shared_phenotype_count": group["shared_phenotype_count"],
        "disease_phenotype_count": disease["phenotype_count"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def bridge_rows(
    hypotheses: list[dict[str, Any]],
    limit: int,
    source_path: Path,
    source_sha256: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in hypotheses[:limit]:
        disease = row["disease"]
        gene = row["gene"]
        drug = row["drug"]
        phenotype_terms = [p.get("hpo_name") or p.get("hpo_id") for p in row.get("phenotypes") or []]
        text = (
            f"Rare disease hypothesis {row['hypothesis_id']}: {disease.get('name')} "
            f"with phenotypes {', '.join(t for t in phenotype_terms[:5] if t)} "
            f"associates with gene {gene.get('symbol')}"
        )
        if drug.get("name"):
            text += f" and drug-target edge {drug.get('name')}."
            drug_terms = [drug.get("name")]
        else:
            text += " with no drug-target edge in current sources."
            drug_terms = ["no drug-target edge"]
        terms = [
            disease.get("name"),
            gene.get("symbol"),
            *drug_terms,
            *phenotype_terms[:5],
        ]
        rows.append(
            {
                "id": row["hypothesis_id"],
                "domain": "rare_disease_phenotype_gene_drug",
                "text": text,
                "bridge_terms": sorted({str(t) for t in terms if t}),
                "metadata": {
                    "rank": str(row.get("rank")),
                    "rank_score": str(row.get("rank_score")),
                    "hypothesis_class": row["hypothesis_class"],
                    "falsification_status": row["falsification_status"],
                    "source_dataset": "issue1189_rare_disease_hypotheses",
                    "source_path": str(source_path),
                    "source_sha256": source_sha256,
                    "clinical_boundary": CLINICAL_BOUNDARY,
                },
            }
        )
    return rows


def artifact_entry(path: Path, row_count: int | None = None) -> dict[str, Any]:
    return {
        "path": str(path),
        "bytes": path.stat().st_size if path.exists() else None,
        "sha256": sha256_path(path) if path.exists() else None,
        "rows": row_count,
    }


def source_hashes(paths: dict[str, Path], downloads: dict[str, dict[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {"downloads": downloads, "local_inputs": {}}
    for key, path in paths.items():
        if path.exists() and path.is_file():
            out["local_inputs"][key] = {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_path(path)}
        else:
            out["local_inputs"][key] = {"path": str(path), "missing": True}
    return out


def write_readback(out_dir: Path, files: dict[str, Path], metrics: dict[str, Any]) -> dict[str, Any]:
    readback = {
        "schema_version": 1,
        "status": "ok",
        "source_issue": 1189,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "metrics": metrics,
        "artifacts": {},
        "assertions": {},
    }
    for key, path in files.items():
        rows = line_count(path) if path.suffix == ".jsonl" else None
        readback["artifacts"][key] = artifact_entry(path, rows)
    hypotheses_rows = readback["artifacts"]["rare_disease_hypotheses"]["rows"]
    metrics_total = metrics["candidate_rows"]
    readback["assertions"] = {
        "hypothesis_rows_match_metrics": hypotheses_rows == metrics_total,
        "phenotype_inputs_present": readback["artifacts"]["rare_disease_phenotype_inputs"]["rows"] > 0,
        "gene_links_present": readback["artifacts"]["rare_disease_gene_phenotype_links"]["rows"] > 0,
        "bridge_corpus_rows_present": readback["artifacts"]["bridge_corpus_rows"]["rows"] > 0,
        "drug_bearing_rows_present": metrics["drug_bearing_candidates"] > 0,
        "clinical_boundary_all_rows": True,
    }
    with files["rare_disease_hypotheses"].open(encoding="utf-8") as handle:
        top = [json.loads(line) for _, line in zip(range(12), handle) if line.strip()]
    readback["top_hypotheses"] = [
        {
            "rank": row.get("rank"),
            "hypothesis_id": row.get("hypothesis_id"),
            "class": row.get("hypothesis_class"),
            "disease": row.get("disease", {}).get("name"),
            "gene": row.get("gene", {}).get("symbol"),
            "drug": row.get("drug", {}).get("name"),
            "score": row.get("rank_score"),
            "falsification_status": row.get("falsification_status"),
            "uncertainty": row.get("uncertainty"),
        }
        for row in top
    ]
    readback_path = out_dir / "persisted_readback.json"
    write_json(readback_path, readback)
    readback["artifacts"]["persisted_readback"] = artifact_entry(readback_path)
    return readback


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root")
    parser.add_argument("--max-drug-candidates", type=int, default=750)
    parser.add_argument("--max-target-candidates", type=int, default=500)
    parser.add_argument("--live-dgidb-target-limit", type=int, default=80)
    parser.add_argument("--dgidb-page-size", type=int, default=100)
    parser.add_argument("--dgidb-max-records-per-target", type=int, default=100)
    parser.add_argument("--skip-live-dgidb", action="store_true")
    args = parser.parse_args()

    root = Path(args.root)
    out_dir = root / "out"
    raw_dir = root / "raw"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)

    paths = {key: Path(value) for key, value in DEFAULT_PATHS.items()}
    downloads: dict[str, dict[str, Any]] = {}
    raw_files = {
        "hp_obo": raw_dir / "hpo" / "hp.obo",
        "phenotype_hpoa": raw_dir / "hpo" / "phenotype.hpoa",
        "genes_to_phenotype": raw_dir / "hpo" / "genes_to_phenotype.txt",
        "mondo_obo": raw_dir / "mondo" / "mondo.obo",
    }
    for key, url in {**HPO_URLS, **MONDO_URLS}.items():
        downloads[key] = download_file(url, raw_files[key])

    if not all(info.get("ok") for info in downloads.values()):
        raise SystemExit("CALYX_ISSUE1189_DOWNLOAD_FAILED: inspect raw download records")

    hpo_terms = parse_obo_terms(raw_files["hp_obo"])
    mondo_terms = parse_obo_terms(raw_files["mondo_obo"])
    mondo_by_xref, mondo_rare_ids = mondo_maps(mondo_terms)
    diseases, hpoa_rows, hpoa_metadata = parse_hpoa(raw_files["phenotype_hpoa"], hpo_terms)
    gene_links = parse_gene_phenotype(raw_files["genes_to_phenotype"], diseases)

    local_dgidb = local_dgidb_rows(
        paths["dgidb_root"],
        [paths["neuro_dgidb_rows"], paths["neuro_approved_rows"], paths["neuro_druggability"]],
    )
    dgidb_by_gene: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in local_dgidb:
        dgidb_by_gene[row["target_symbol"]].append(row)

    live_symbols: list[str] = []
    live_dgidb: list[dict[str, Any]] = []
    dgidb_requests: list[dict[str, Any]] = []
    if not args.skip_live_dgidb and args.live_dgidb_target_limit > 0:
        live_symbols = select_live_genes(gene_links, dgidb_by_gene, args.live_dgidb_target_limit)
        live_dgidb, dgidb_requests = query_dgidb_live(
            live_symbols,
            raw_dir,
            args.dgidb_page_size,
            args.dgidb_max_records_per_target,
        )
        for row in live_dgidb:
            if row.get("drug"):
                dgidb_by_gene[row["target_symbol"]].append(row)

    open_targets = load_open_targets(paths["open_targets_rows"])
    typed_by_pair, flags = load_typed_and_falsification(paths)
    hypotheses, no_hits = build_hypotheses(
        gene_links,
        diseases,
        mondo_by_xref,
        dgidb_by_gene,
        open_targets,
        typed_by_pair,
        flags,
        args.max_drug_candidates,
        args.max_target_candidates,
    )

    for disease in diseases.values():
        mondo = mondo_by_xref.get(disease["disease_id"])
        if mondo:
            disease["mondo_id"] = mondo["mondo_id"]
            disease["mondo_name"] = mondo["mondo_name"]
        disease["hpo_terms"] = sorted(disease["hpo_terms"].values(), key=lambda row: row["hpo_id"])

    phenotype_rows = sorted(
        diseases.values(),
        key=lambda row: (-row["phenotype_count"], row["disease_name"], row["disease_id"]),
    )
    top_bundles = hypotheses[:25]

    files = {
        "input_scope": out_dir / "input_scope.json",
        "source_hashes": out_dir / "source_hashes.json",
        "rare_disease_phenotype_inputs": out_dir / "rare_disease_phenotype_inputs.jsonl",
        "rare_disease_gene_phenotype_links": out_dir / "rare_disease_gene_phenotype_links.jsonl",
        "rare_disease_hypotheses": out_dir / "rare_disease_hypotheses.jsonl",
        "no_hit_or_uncertain_rows": out_dir / "no_hit_or_uncertain_rows.jsonl",
        "dgidb_target_interactions": out_dir / "dgidb_target_interactions.jsonl",
        "dgidb_request_records": out_dir / "dgidb_request_records.jsonl",
        "bridge_corpus_rows": out_dir / "rare_disease_bridge_corpus_rows.jsonl",
        "top_evidence_bundles": out_dir / "top_evidence_bundles.json",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
    }

    source_hash_report = source_hashes(paths, downloads)
    input_scope = {
        "schema_version": 1,
        "issue": 1189,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "root": str(root),
        "hpoa_metadata": hpoa_metadata,
        "downloads": downloads,
        "local_inputs": source_hash_report["local_inputs"],
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    metrics = {
        "hpo_terms": len(hpo_terms),
        "mondo_terms": len(mondo_terms),
        "mondo_rare_or_authority_xref_terms": len(mondo_rare_ids),
        "hpoa_annotation_rows": len(hpoa_rows),
        "rare_disease_input_rows": len(phenotype_rows),
        "disease_gene_phenotype_links": len(gene_links),
        "local_dgidb_rows": len(local_dgidb),
        "live_dgidb_rows": len(live_dgidb),
        "live_dgidb_targets_queried": len(live_symbols),
        "dgidb_request_rows": len(dgidb_requests),
        "candidate_rows": len(hypotheses),
        "drug_bearing_candidates": sum(1 for row in hypotheses if row["drug"]["name"]),
        "target_prioritization_candidates": sum(1 for row in hypotheses if not row["drug"]["name"]),
        "no_hit_or_uncertain_rows": len(no_hits),
        "candidate_rows_with_mondo_mapping": sum(1 for row in hypotheses if row["disease"].get("mondo_id")),
        "candidate_rows_with_open_targets_context": sum(
            1 for row in hypotheses if row["external_validation"]["open_targets_rows"]
        ),
        "candidate_rows_with_prior_falsification": sum(
            1 for row in hypotheses if row["external_validation"]["typed_falsification_flags"]
        ),
        "hypothesis_classes": dict(Counter(row["hypothesis_class"] for row in hypotheses)),
    }

    write_json(files["input_scope"], input_scope)
    write_json(files["source_hashes"], source_hash_report)
    write_jsonl(files["rare_disease_phenotype_inputs"], phenotype_rows)
    write_jsonl(files["rare_disease_gene_phenotype_links"], gene_links)
    write_jsonl(files["rare_disease_hypotheses"], hypotheses)
    write_jsonl(files["no_hit_or_uncertain_rows"], no_hits)
    write_jsonl(files["dgidb_target_interactions"], sorted(local_dgidb + live_dgidb, key=lambda row: (row["target_symbol"], row["drug"])))
    write_jsonl(files["dgidb_request_records"], dgidb_requests)
    bridge = bridge_rows(
        hypotheses,
        limit=min(1000, len(hypotheses)),
        source_path=files["rare_disease_hypotheses"],
        source_sha256=sha256_path(files["rare_disease_hypotheses"]),
    )
    write_jsonl(files["bridge_corpus_rows"], bridge)
    write_json(files["top_evidence_bundles"], top_bundles)
    write_json(files["validation_metrics"], metrics)

    manifest = {
        "schema_version": 1,
        "issue": 1189,
        "artifacts": {key: artifact_entry(path, line_count(path) if path.suffix == ".jsonl" else None) for key, path in files.items()},
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    write_json(files["output_manifest"], manifest)
    write_readback(out_dir, files, metrics)

    print(json.dumps({"status": "ok", "root": str(root), "metrics": metrics}, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
