#!/usr/bin/env python3
"""#1228 aggregate component-safety and pair-interaction coverage for #1190."""

from __future__ import annotations

import argparse
import importlib.util
import json
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


UTIL_PATH = Path(__file__).with_name("issue1259_rxnorm_twosides_safety_validation.py")
SPEC = importlib.util.spec_from_file_location("issue1259_utils", UTIL_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"Unable to import helpers from {UTIL_PATH}")
UTIL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UTIL)


DEFAULT_ROOT = "/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T070000Z"
MAX_BRIDGE_ROWS = 1000

CLINICAL_BOUNDARY = (
    "Issue1228 coverage rollup is safety and interaction triage only; source "
    "rows, no-hit rows, adverse-event rows, label text, registry context, "
    "literature context, and identity mappings are blockers or review inputs, "
    "not safety clearance, efficacy, treatment guidance, dosing guidance, "
    "recommendation, clinical actionability, pair-interaction proof, or cure evidence."
)

PROMOTION_STATUS = "blocked_fail_closed_requires_independent_safety_pair_interaction_outcome_falsification_and_human_review"


@dataclass(frozen=True)
class SourceSpec:
    path: str
    sha256: str
    rows: int | None


SOURCE_SPECS: dict[str, SourceSpec] = {
    "issue1190_component_inputs": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/combination_candidate_inputs.jsonl", "be973631b5f16b4989db6d0cb8acad2fad0a369fad3c0c2b4932608573aada67", 1023),
    "issue1190_candidate_pairs": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/candidate_pair_inputs.jsonl", "d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654", 1750),
    "issue1190_hypotheses": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/drug_combination_hypotheses.jsonl", "f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484", 1750),
    "issue1190_flags": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/combination_safety_interaction_flags.jsonl", "86b1a07aad0afd7a64bdc009bc7db18c147efe2ac226ea12612ac085acd575ab", 1750),
    "issue1190_component_safety_index": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/drug_component_safety_index.jsonl", "53e296b2c27685c21e5280c7d70b0e7df2f2b66b72b369d95d9a95e2f75385a8", 14),
    "issue1190_drugcomb_pair_matches": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/drugcomb_pair_matches.jsonl", "9b075a80df9b6b252191d548ee68c93202e2ed9dfaa4430e226446aa7ad51a86", 28),
    "issue1190_persisted_readback": SourceSpec("/home/croyse/calyx/fsv/issue1190-drug-combination-miner-20260704T130000Z/out/persisted_readback.json", "9520cbcd24138df535dbfdba7cd3468897f0cd3e2e194a13437da7fa7349fbe2", None),
    "issue1229_candidate_external_synergy_status": SourceSpec("/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/candidate_external_synergy_status.jsonl", "e4dbb332db9c4b656db83452a2f0debac9748c4c0cadb4b98c9280cfbcc6a4b7", 1750),
    "issue1229_candidate_external_synergy_hits": SourceSpec("/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/candidate_external_synergy_hits.jsonl", "b5f6135a8c86b8bb4c7115ff3f2c72d1384fc021f71c6832e4cc212545844a09", 68),
    "issue1231_candidate_external_combo_status": SourceSpec("/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/candidate_external_combo_status.jsonl", "f9d249482ecc8b3898062b58d61df0e4af1ac057dba4976d1ea1b7e193fd45f4", 1750),
    "issue1231_candidate_external_combo_hits": SourceSpec("/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/candidate_external_combo_hits.jsonl", "c340659ea20de331a93d51c5b44d3665f26c734f6895e436d70761bc3034a26a", 173),
    "issue1231_prior_no_hit_recheck_status": SourceSpec("/home/croyse/calyx/fsv/issue1231-external-combo-sources-20260704T150500Z/out/prior_no_hit_recheck_status.jsonl", "1ae98201d1af0a6e6632f6de05eef4f1e0af6b6ff2e533eec35adab990bf27e0", 1682),
    "issue1232_clinicaltrials_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/clinicaltrials_pair_status.jsonl", "ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc", 1546),
    "issue1232_clinicaltrials_pair_hits": SourceSpec("/home/croyse/calyx/fsv/issue1232-clinicaltrials-current-recheck-20260704T153000Z/out/clinicaltrials_pair_hits.jsonl", "026cc1d5698f6b7b2fdcf2cd095cf0a605a804f1ed3f40ed0bd151342c006c38", 204),
    "issue1233_cdcdb_pair_validation_status": SourceSpec("/home/croyse/calyx/fsv/issue1233-cdcdb-gate-validation-20260705T025403Z/out/cdcdb_pair_validation_status.jsonl", "c0d3e03419dc1240260b18a81a9e9bf3601305773cb3ec99a6b217d81825f3b2", 173),
    "issue1233_cdcdb_missing_gate_rows": SourceSpec("/home/croyse/calyx/fsv/issue1233-cdcdb-gate-validation-20260705T025403Z/out/cdcdb_missing_gate_rows.jsonl", "3ce4b35f88a385ca1aaf977690925981ab8c07eeb7d9b4e686b3bb572d53ec35", 692),
    "issue1234_candidate_external_source_status": SourceSpec("/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/candidate_external_source_status.jsonl", "8f61616f01f44695e7b0227a16ff8cd9bceee2c68ad6147b705142a69ce24b0e", 1342),
    "issue1234_candidate_external_source_hits": SourceSpec("/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/candidate_external_source_hits.jsonl", "b60932183462b012f284a26750b0412c6a179224849938d7bb05ee42d473e950", 301),
    "issue1234_pubmed_pair_literature_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/pubmed_pair_literature_evidence.jsonl", "304400ca4bbc7faea42ac9f2dd708865e7e27a225cbde6e806301d4581b7f7ea", 568),
    "issue1235_clinicaltrials_pair_validation_status": SourceSpec("/home/croyse/calyx/fsv/issue1235-clinicaltrials-gate-validation-20260705T023240Z/out/clinicaltrials_pair_validation_status.jsonl", "9596b2e7e39c78714f2740073991880e8a76c6427e48704af9dedc8f53fdf4f0", 204),
    "issue1235_clinicaltrials_missing_gate_rows": SourceSpec("/home/croyse/calyx/fsv/issue1235-clinicaltrials-gate-validation-20260705T023240Z/out/clinicaltrials_missing_gate_rows.jsonl", "ff00f66206a8091716e1bf559812abde986e1cec58c8ab2fd5ad83d08900b09d", 816),
    "issue1236_openfda_label_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/openfda_label_pair_status.jsonl", "9a1680d5243a90ab52af7bb93b76f8e7677c7b7cef338282bf1cd781b5d7524a", 649),
    "issue1236_openfda_label_pair_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1236-openfda-label-source-mining-20260704T170534Z/out/openfda_label_pair_evidence.jsonl", "d2376732cea19e4f17d4f803e5e7c0b07cbd6ef4898fb7b527a6a1509c304ab3", 40),
    "issue1239_candidate_pubmed_gate_status": SourceSpec("/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T012109Z/out/candidate_pubmed_gate_status.jsonl", "67a0ff300092839addc6bd32b4922da954ee1b2469a905e5192895c041b838a7", 301),
    "issue1239_pubmed_structured_gate_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T012109Z/out/pubmed_structured_gate_evidence.jsonl", "ec8137c141f37fcf1282e7bbb752ba5e100a215bfaf75dd77eab3ea7961d5db2", 510),
    "issue1239_pubmed_missing_gate_rows": SourceSpec("/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T012109Z/out/pubmed_missing_gate_rows.jsonl", "6ce1a596517ccbf52fe3cad5a7a8ef1970d95fd03f3266f7b8b295cce707fc02", 1505),
    "issue1240_rxnorm_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1240-rxnorm-combination-products-20260704T172703Z/out/rxnorm_pair_status.jsonl", "68c2d136ff82dc6ded1c141e17040197e9cd264914fa6594ae4f73737e02bfc1", 640),
    "issue1241_label_evidence_gate_rows": SourceSpec("/home/croyse/calyx/fsv/issue1241-openfda-label-gate-validation-20260705T040000Z/out/label_evidence_gate_rows.jsonl", "d3a53039d3a5d866154f2576e95101f7b8e0bd9ac4cb87b6d15b312887ecc390", 40),
    "issue1241_pair_label_gate_rollups": SourceSpec("/home/croyse/calyx/fsv/issue1241-openfda-label-gate-validation-20260705T040000Z/out/pair_label_gate_rollups.jsonl", "0826cca2b76d539fbc6a90facd8bac467f7d0b556f80d37eda0f0fd2ba04d8bf", 9),
    "issue1241_candidate_label_gate_status": SourceSpec("/home/croyse/calyx/fsv/issue1241-openfda-label-gate-validation-20260705T040000Z/out/candidate_label_gate_status.jsonl", "0cfcb6bfb3552c37435c00860e6e6648cf37b2fc7df7fd0ac6478d2e0876f400", 22),
    "issue1242_dailymed_spl_title_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1242-dailymed-spl-title-mining-20260704T174500Z/out/dailymed_spl_title_pair_status.jsonl", "64e1a84ffb4252ebac671f15e94bd3b617a69dcb0294b817852bfbec8c9004fc", 640),
    "issue1243_europepmc_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_status.jsonl", "153b7f222495f25244d6e221b5cc46ca1830335a12759867328b9a26b2384f9d", 640),
    "issue1243_europepmc_pair_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1243-europepmc-pair-search-20260704T181500Z/out/europepmc_pair_evidence.jsonl", "a393992fb1e5b0d8a83f6f99e27e6b1c6aeeb5563615bb2a729b0602075aa609", 598),
    "issue1244_candidate_europepmc_relation_rollup": SourceSpec("/home/croyse/calyx/fsv/issue1244-europepmc-relation-validation-20260704T193000Z/out/candidate_europepmc_relation_rollup.jsonl", "730cd82b585050ad04909c0d29d0e43bd2a891ff6d40cf9f23ac096bb03f37ef", 487),
    "issue1245_pubchem_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1245-pubchem-synonym-source-mining-20260704T210000Z/out/pubchem_pair_status.jsonl", "60f313a4eb57de2845f2f8a6d9f55a203655c1f53eb593dc6d907ec15aa4002e", 353),
    "issue1246_candidate_europepmc_safety_counter_rollup": SourceSpec("/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z/out/candidate_europepmc_safety_counter_rollup.jsonl", "b7fddaf84c0c9a09dcbb8a6e59e505a6fe225fe4dce5f0167cd165d4b7b06255", 363),
    "issue1247_candidate_europepmc_endpoint_outcome_rollup": SourceSpec("/home/croyse/calyx/fsv/issue1247-europepmc-endpoint-outcome-review-20260704T223000Z/out/candidate_europepmc_endpoint_outcome_rollup.jsonl", "d13703c71a7e1dedf595ee4b21fd0fbf5535d33c7487bebfd3717426e9aea90c", 108),
    "issue1248_openfda_independent_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z/out/openfda_independent_pair_status.jsonl", "2ff40c8720c24c68a77f028dd641715d2368b27b307e3652496ef0fcec71268c", 202),
    "issue1249_faers_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1249-openfda-faers-safety-expansion-20260704T203500Z/out/faers_pair_status.jsonl", "b4f75d65e844c062ea97e54fb4b709ac3b87d2a348f7ad398d98e690e0535c16", 202),
    "issue1249_faers_event_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1249-openfda-faers-safety-expansion-20260704T203500Z/out/faers_event_evidence.jsonl", "c7ff1704f9491b27e6f8edd4f5b432e300236a41261d4999293145d3252df58b", 45),
    "issue1250_clinicaltrials_endpoint_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1250-clinicaltrials-endpoint-validation-20260704T230000Z/out/clinicaltrials_endpoint_pair_status.jsonl", "bfd61154e643adb9c852f5c0ee031db018e6986026ec400863784d677f1806c8", 72),
    "issue1251_europepmc_source_local_endpoint_rollup_status": SourceSpec("/home/croyse/calyx/fsv/issue1251-europepmc-source-local-endpoint-expansion-20260704T234000Z/out/europepmc_source_local_endpoint_rollup_status.jsonl", "5af58d0b05c2fb03374ff0d74f3fceb300a8c40def32f455be8478a5d241f7c4", 108),
    "issue1252_effect_result_rollup_status": SourceSpec("/home/croyse/calyx/fsv/issue1252-effect-result-falsification-gate-20260704T235000Z/out/effect_result_rollup_status.jsonl", "43c11b2c653ff0772e6ef382ceafa6b1a84d5a10debba491586fc8107309a8b1", 35),
    "issue1253_independent_effect_rollup_status": SourceSpec("/home/croyse/calyx/fsv/issue1253-independent-effect-result-validation-20260704T202500Z/out/independent_effect_rollup_status.jsonl", "c0b79bed2ec4c0ff765697d094f6156903a87586b138cba252b74eb7ae784b8f", 24),
    "issue1254_chembl_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1254-chembl-source-mining-20260704T213500Z/out/chembl_pair_status.jsonl", "4ea75759362b16db5a3470605d6321d93f2301fa4954b60bd02dc81c5bd2861f", 353),
    "issue1255_drugcentral_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/drugcentral_pair_status.jsonl", "07caa8201d7a9367864a52af4bae57afd9822e3e52aa69721e89f250cfe2ed81", 353),
    "issue1256_pharmgkb_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/pharmgkb_pair_status.jsonl", "58dc7d1c502f385649d41ff3bc469b0db0a373c12d991f2a9f744cecfdfe0d04", 353),
    "issue1257_nsides_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/nsides_pair_status.jsonl", "16626c6da02115bb1cb6a9a71d1f094bdba9ca799c59b69bad7bc62e8715ca2a", 353),
    "issue1257_offsides_single_drug_context": SourceSpec("/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z/out/offsides_single_drug_context.jsonl", "4d5a6991457b0088b04e3aa4cf22f3eac780fe3c627d7d19cf35f8aa765f12dc", 853),
    "issue1258_rxnorm_pair_status": SourceSpec("/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z/out/rxnorm_pair_status.jsonl", "20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0", 353),
    "issue1258_rxnorm_term_status": SourceSpec("/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z/out/rxnorm_term_status.jsonl", "8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7", 152),
    "issue1258_rxnorm_offsides_single_drug_context": SourceSpec("/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z/out/rxnorm_offsides_single_drug_context.jsonl", "cbb8980f79f05010ab4c515a5b1a9908a302cccc4d867d9f84d9e60c1c7bdc30", 1088),
    "issue1258_rxnorm_twosides_pair_evidence": SourceSpec("/home/croyse/calyx/fsv/issue1258-rxnorm-canonicalization-20260705T000500Z/out/rxnorm_twosides_pair_evidence.jsonl", "7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17", 757),
    "issue1259_pair_validation_rollups": SourceSpec("/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z/out/pair_validation_rollups.jsonl", "186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c", 7),
    "issue1259_independent_evidence_rows": SourceSpec("/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z/out/independent_evidence_rows.jsonl", "6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca", 1),
    "issue1260_candidate_case_status": SourceSpec("/home/croyse/calyx/fsv/issue1260-metformin-trametinib-faers-case-20260705T020000Z/out/candidate_case_status.jsonl", "cc01e740f67c88ac8cd9d2da775310a86dbdf2c5c45b84f524b826a7ed5ca656", 4),
    "issue1261_ranker_case_quality_overlay": SourceSpec("/home/croyse/calyx/fsv/issue1261-faers-case-quality-ranker-overlay-20260705T030000Z/out/ranker_case_quality_overlay.jsonl", "d5f31d382afc25584577aab7b58b6c0268a71e5840fa170fde1467b0e8e0cbb8", 5),
}

PAIR_SOURCE_NAMES = [
    name
    for name in SOURCE_SPECS
    if name not in {"issue1190_component_inputs", "issue1190_component_safety_index", "issue1190_persisted_readback", "issue1258_rxnorm_term_status"}
]

PAIR_HIT_SOURCE_NAMES = {
    name
    for name in SOURCE_SPECS
    if any(token in name for token in ("hits", "matches", "evidence"))
}

COMPONENT_SOURCE_NAMES = {
    "issue1190_component_safety_index",
    "issue1241_label_evidence_gate_rows",
    "issue1246_candidate_europepmc_safety_counter_rollup",
    "issue1248_openfda_independent_pair_status",
    "issue1249_faers_event_evidence",
    "issue1257_offsides_single_drug_context",
    "issue1258_rxnorm_term_status",
    "issue1258_rxnorm_offsides_single_drug_context",
}


def now_utc() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def rows_jsonl(path: str | Path) -> list[dict[str, Any]]:
    return list(UTIL.rows_jsonl(Path(path)))


def write_json(path: Path, value: Any) -> None:
    UTIL.write_json(path, value)


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    UTIL.write_jsonl(path, rows)


def count_jsonl(path: Path) -> int:
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def verify_sources(skip: bool) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for name, spec in SOURCE_SPECS.items():
        path = Path(spec.path)
        if not path.exists():
            raise FileNotFoundError(f"Missing input {name}: {path}")
        observed = UTIL.sha256_path(path)
        hash_match = observed == spec.sha256
        if not hash_match and not skip:
            raise RuntimeError(f"Input hash mismatch for {name}: {observed} != {spec.sha256}")
        row_count = count_jsonl(path) if path.suffix == ".jsonl" else None
        row_count_match = row_count == spec.rows
        if not row_count_match and not skip:
            raise RuntimeError(f"Input row mismatch for {name}: {row_count} != {spec.rows}")
        rows.append(
            {
                "schema_version": 1,
                "source_row_id": "issue1228-source:" + UTIL.stable_id(name, observed),
                "input_name": name,
                "path": str(path),
                "bytes": path.stat().st_size,
                "rows": row_count,
                "expected_rows": spec.rows,
                "sha256": observed,
                "expected_sha256": spec.sha256,
                "hash_match": hash_match,
                "row_count_match": row_count_match,
                "role": source_role(name),
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return rows


def source_role(name: str) -> str:
    if name == "issue1190_component_inputs":
        return "input_component_universe"
    if name == "issue1190_candidate_pairs":
        return "input_pair_universe"
    if "component_safety_index" in name or "offsides" in name:
        return "component_safety_context"
    if "label" in name or "faers" in name or "safety" in name:
        return "safety_interaction_context"
    if "clinicaltrials" in name or "pubmed" in name or "europepmc" in name:
        return "registry_or_literature_context"
    if "rxnorm" in name or "pubchem" in name or "chembl" in name or "drugcentral" in name or "pharmgkb" in name:
        return "identity_or_pharmacology_context"
    return "source_context"


def pair_ids_for_row(row: dict[str, Any], ids_by_key: dict[str, list[str]], pair_by_id: dict[str, dict[str, Any]]) -> list[str]:
    ids: list[str] = []
    for key in ("pair_id", "representative_pair_id", "source_issue1231_pair_id", "source_issue1232_pair_evidence_id"):
        value = row.get(key)
        if isinstance(value, str) and value in pair_by_id:
            ids.append(value)
    for key in ("source_pair_ids", "candidate_pair_ids"):
        values = row.get(key) or []
        if isinstance(values, list):
            ids.extend(value for value in values if isinstance(value, str) and value in pair_by_id)
    pair_key = row.get("pair_key")
    if isinstance(pair_key, str):
        ids.extend(ids_by_key.get(pair_key, []))
    return sorted(set(ids))


def status_values(row: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, value in row.items():
        if not isinstance(value, (str, int, float, bool)):
            continue
        lowered = key.lower()
        if lowered.endswith("status") or "gate" in lowered or "classification" in lowered:
            out[key] = value
    return out


def add_pair_sources(
    pair_rows: dict[str, dict[str, Any]],
    ids_by_key: dict[str, list[str]],
    pair_by_id: dict[str, dict[str, Any]],
    source_name: str,
) -> None:
    source_path = SOURCE_SPECS[source_name].path
    if not source_path.endswith(".jsonl"):
        return
    for row in rows_jsonl(source_path):
        ids = pair_ids_for_row(row, ids_by_key, pair_by_id)
        if not ids:
            continue
        is_hit = source_name in PAIR_HIT_SOURCE_NAMES
        for pair_id in ids:
            target = pair_rows[pair_id]
            target["source_row_counts"][source_name] += 1
            if is_hit:
                target["source_hit_row_counts"][source_name] += 1
            for reason in row.get("reason_codes") or []:
                target["reason_codes"].add(UTIL.clean_text(reason))
            for key, value in status_values(row).items():
                target["source_status_values"][source_name][key].add(str(value))


def component_terms_for_row(source_name: str, row: dict[str, Any]) -> set[str]:
    terms: set[str] = set()
    for key in ("drug_norm", "term_norm", "drug", "drug_term", "term", "candidate_drug_name", "source_drug"):
        value = row.get(key)
        if isinstance(value, str) and value.strip():
            terms.add(UTIL.norm_name(value))
    if source_name in COMPONENT_SOURCE_NAMES:
        for key in ("drug_a", "drug_b"):
            value = row.get(key)
            if isinstance(value, str) and value.strip():
                terms.add(UTIL.norm_name(value))
    return {term for term in terms if term}


def component_context_kind(source_name: str, row: dict[str, Any]) -> str:
    if source_name == "issue1258_rxnorm_term_status":
        return "identity_mapping_context_not_safety_clearance"
    if "offsides" in source_name:
        return "single_drug_adverse_event_context_not_pair_proof"
    if "faers_event" in source_name:
        return "adverse_event_coreport_context_not_causality"
    if "label" in source_name:
        return "label_safety_or_interaction_section_context"
    if "safety_counter" in source_name:
        return "literature_safety_counter_context"
    return UTIL.clean_text(row.get("evidence_kind")) or "component_source_context"


def build_component_rows(components: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    by_norm: dict[str, dict[str, Any]] = {}
    for row in components:
        norm = row.get("drug_norm") or UTIL.norm_name(row.get("drug"))
        if not norm:
            continue
        target = by_norm.setdefault(
            norm,
            {
                "drug_norm": norm,
                "drug_names": set(),
                "component_ids": set(),
                "candidate_ids": set(),
                "disease_areas": set(),
                "source_row_counts": Counter(),
                "context_kinds": Counter(),
                "source_hashes": {},
            },
        )
        target["drug_names"].add(UTIL.clean_text(row.get("drug")))
        target["component_ids"].add(row.get("component_id"))
        target["candidate_ids"].add(row.get("candidate_id"))
        target["disease_areas"].add(UTIL.clean_text(row.get("disease_area")))
    for source_name in COMPONENT_SOURCE_NAMES:
        spec = SOURCE_SPECS[source_name]
        for row in rows_jsonl(spec.path):
            for term in component_terms_for_row(source_name, row):
                if term not in by_norm:
                    continue
                target = by_norm[term]
                target["source_row_counts"][source_name] += 1
                target["context_kinds"][component_context_kind(source_name, row)] += 1
                target["source_hashes"][source_name] = spec.sha256
    coverage_rows: list[dict[str, Any]] = []
    no_hit_rows: list[dict[str, Any]] = []
    for norm, value in sorted(by_norm.items()):
        safety_context_sources = [
            name
            for name, count in value["source_row_counts"].items()
            if count and name != "issue1258_rxnorm_term_status"
        ]
        coverage_status = (
            "component_safety_context_present_still_blocked"
            if safety_context_sources
            else "explicit_no_component_safety_hit_fail_closed"
        )
        row = {
            "schema_version": 1,
            "component_coverage_id": "issue1228-component:" + UTIL.stable_id(norm),
            "drug_norm": norm,
            "drug_names": sorted(name for name in value["drug_names"] if name),
            "component_input_rows": len(value["component_ids"]),
            "candidate_ids_sample": sorted(cid for cid in value["candidate_ids"] if cid)[:12],
            "disease_areas": sorted(area for area in value["disease_areas"] if area),
            "source_row_counts": dict(sorted(value["source_row_counts"].items())),
            "context_kind_counts": dict(sorted(value["context_kinds"].items())),
            "source_hashes": dict(sorted(value["source_hashes"].items())),
            "coverage_status": coverage_status,
            "promotion_status": PROMOTION_STATUS,
            "reason_codes": [
                "component_safety_context_not_clearance" if safety_context_sources else "component_safety_no_source_hit",
                "fail_closed_no_component_safety_clearance",
            ],
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        coverage_rows.append(row)
        if not safety_context_sources:
            no_hit_rows.append(
                {
                    "schema_version": 1,
                    "component_no_hit_id": "issue1228-component-no-hit:" + UTIL.stable_id(norm),
                    "drug_norm": norm,
                    "coverage_status": coverage_status,
                    "missing_gate": "component_safety",
                    "promotion_status": PROMOTION_STATUS,
                    "reason_codes": row["reason_codes"],
                    "clinical_boundary": CLINICAL_BOUNDARY,
                }
            )
    return coverage_rows, no_hit_rows


def build_pair_rows(
    pairs: list[dict[str, Any]],
    hypotheses: list[dict[str, Any]],
    flags: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    pair_by_id = {row["pair_id"]: row for row in pairs}
    ids_by_key: dict[str, list[str]] = defaultdict(list)
    for row in pairs:
        ids_by_key[row["pair_key"]].append(row["pair_id"])
    hypothesis_by_id = {row["pair_id"]: row for row in hypotheses}
    flag_by_id = {row["pair_id"]: row for row in flags}
    raw_rows: dict[str, dict[str, Any]] = {}
    for row in pairs:
        hypothesis = hypothesis_by_id.get(row["pair_id"], {})
        flag = flag_by_id.get(row["pair_id"], {})
        raw_rows[row["pair_id"]] = {
            "pair_id": row["pair_id"],
            "pair_key": row["pair_key"],
            "drug_a": row["drug_a"],
            "drug_b": row["drug_b"],
            "disease": row.get("disease"),
            "disease_area": row.get("disease_area"),
            "original_rank": hypothesis.get("rank"),
            "original_combination_status": hypothesis.get("combination_status") or flag.get("combination_status"),
            "original_reason_codes": hypothesis.get("reason_codes") or flag.get("reason_codes") or [],
            "source_row_counts": Counter(),
            "source_hit_row_counts": Counter(),
            "source_status_values": defaultdict(lambda: defaultdict(set)),
            "reason_codes": set(hypothesis.get("reason_codes") or flag.get("reason_codes") or []),
        }
    for source_name in PAIR_SOURCE_NAMES:
        add_pair_sources(raw_rows, ids_by_key, pair_by_id, source_name)
    coverage_rows: list[dict[str, Any]] = []
    gap_rows: list[dict[str, Any]] = []
    for pair_id, value in sorted(raw_rows.items(), key=lambda item: (item[1].get("original_rank") or 999999, item[0])):
        source_counts = dict(sorted(value["source_row_counts"].items()))
        hit_counts = dict(sorted(value["source_hit_row_counts"].items()))
        hit_total = sum(hit_counts.values())
        has_hit = hit_total > 0
        status_values_by_source = {
            source: {field: sorted(values) for field, values in sorted(fields.items())}
            for source, fields in sorted(value["source_status_values"].items())
        }
        coverage_status = (
            "pair_source_context_present_still_blocked"
            if has_hit
            else "explicit_no_pair_interaction_source_hit_fail_closed"
        )
        row = {
            "schema_version": 1,
            "pair_coverage_id": "issue1228-pair:" + UTIL.stable_id(pair_id, value["pair_key"]),
            "pair_id": pair_id,
            "pair_key": value["pair_key"],
            "drug_a": value["drug_a"],
            "drug_b": value["drug_b"],
            "disease": value.get("disease"),
            "disease_area": value.get("disease_area"),
            "original_rank": value.get("original_rank"),
            "original_combination_status": value.get("original_combination_status"),
            "original_reason_codes": value.get("original_reason_codes"),
            "source_row_counts": source_counts,
            "source_hit_row_counts": hit_counts,
            "source_status_values": status_values_by_source,
            "source_context_row_total": sum(source_counts.values()),
            "source_hit_row_total": hit_total,
            "coverage_status": coverage_status,
            "pair_interaction_clearance_status": "not_cleared_fail_closed",
            "promotion_status": PROMOTION_STATUS,
            "reason_codes": sorted(reason for reason in value["reason_codes"] if reason),
            "clinical_boundary": CLINICAL_BOUNDARY,
        }
        coverage_rows.append(row)
        gap_rows.append(
            {
                "schema_version": 1,
                "pair_gap_id": "issue1228-pair-gap:" + UTIL.stable_id(pair_id, value["pair_key"]),
                "pair_id": pair_id,
                "pair_key": value["pair_key"],
                "drug_a": value["drug_a"],
                "drug_b": value["drug_b"],
                "gap_class": "source_context_not_clearance" if has_hit else "explicit_no_pair_interaction_source_hit",
                "missing_gate": "exact_pair_interaction_clearance",
                "source_hit_row_total": hit_total,
                "coverage_status": coverage_status,
                "promotion_status": PROMOTION_STATUS,
                "reason_codes": [
                    "exact_pair_interaction_not_cleared_fail_closed",
                    "source_context_review_input_only" if has_hit else "pair_interaction_no_source_hit",
                ],
                "clinical_boundary": CLINICAL_BOUNDARY,
            }
        )
    return coverage_rows, gap_rows


def bridge_metadata(source_path: str | Path, source_sha: str, **extra: Any) -> dict[str, Any]:
    metadata = {
        "source_dataset": "issue1228_safety_interaction_coverage_rollup",
        "source_path": str(source_path),
        "source_sha256": source_sha,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }
    metadata.update(extra)
    return metadata


def build_bridge_rows(
    source_rows: list[dict[str, Any]],
    component_rows: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    source_path: Path,
    source_sha: str,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in source_rows:
        terms = UTIL.uniq(["issue1228", row["input_name"], row["sha256"], row["role"]])
        rows.append(
            {
                "id": row["source_row_id"],
                "domain": "issue1228_source_input",
                "text": f"Issue1228 source input {row['input_name']} sha256 {row['sha256']} role {row['role']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(row["path"], row["sha256"], input_name=row["input_name"]),
            }
        )
    for row in component_rows:
        terms = UTIL.uniq(["issue1228", row["drug_norm"], row["coverage_status"]])
        rows.append(
            {
                "id": row["component_coverage_id"],
                "domain": "issue1228_component_safety_coverage",
                "text": f"Issue1228 component safety coverage {row['drug_norm']} {row['coverage_status']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    remaining = max(0, MAX_BRIDGE_ROWS - len(rows))
    ranked_pairs = sorted(pair_rows, key=lambda row: (row["original_rank"] or 999999, row["pair_id"]))[:remaining]
    for row in ranked_pairs:
        terms = UTIL.uniq(["issue1228", row["pair_key"], row["coverage_status"], row["pair_id"]])
        rows.append(
            {
                "id": row["pair_coverage_id"],
                "domain": "issue1228_pair_interaction_coverage",
                "text": f"Issue1228 pair interaction coverage {row['pair_id']} {row['pair_key']} {row['coverage_status']} bridge terms {' '.join(terms)}.",
                "bridge_terms": terms,
                "metadata": bridge_metadata(source_path, source_sha),
            }
        )
    return rows[:MAX_BRIDGE_ROWS]


def build_metrics(
    source_rows: list[dict[str, Any]],
    components: list[dict[str, Any]],
    pairs: list[dict[str, Any]],
    component_rows: list[dict[str, Any]],
    component_no_hit_rows: list[dict[str, Any]],
    pair_rows: list[dict[str, Any]],
    pair_gap_rows: list[dict[str, Any]],
    bridge_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "ok",
        "generated_at": now_utc(),
        "clinical_boundary": CLINICAL_BOUNDARY,
        "source_input_files": len(source_rows),
        "source_hashes_all_match": all(row["hash_match"] and row["row_count_match"] for row in source_rows),
        "component_input_rows": len(components),
        "unique_component_drugs": len(component_rows),
        "component_no_hit_rows": len(component_no_hit_rows),
        "candidate_pair_rows": len(pairs),
        "pair_coverage_rows": len(pair_rows),
        "pair_gap_rows": len(pair_gap_rows),
        "pair_source_context_rows_present": sum(1 for row in pair_rows if row["source_hit_row_total"] > 0),
        "pair_explicit_no_hit_rows": sum(1 for row in pair_rows if row["source_hit_row_total"] == 0),
        "component_status_counts": dict(Counter(row["coverage_status"] for row in component_rows)),
        "pair_status_counts": dict(Counter(row["coverage_status"] for row in pair_rows)),
        "pair_gap_counts": dict(Counter(row["gap_class"] for row in pair_gap_rows)),
        "bridge_rows": len(bridge_rows),
        "bridge_domain_counts": dict(Counter(row["domain"] for row in bridge_rows)),
        "all_component_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in component_rows),
        "all_pair_rows_blocked": all(row["promotion_status"] == PROMOTION_STATUS for row in pair_rows),
    }


def build_readback(out_dir: Path, metrics: dict[str, Any], bridge_rows: list[dict[str, Any]]) -> dict[str, Any]:
    artifacts = {
        "input_manifest": out_dir / "input_manifest.json",
        "source_coverage_rows": out_dir / "source_coverage_rows.jsonl",
        "component_safety_coverage_rows": out_dir / "component_safety_coverage_rows.jsonl",
        "component_safety_no_hit_rows": out_dir / "component_safety_no_hit_rows.jsonl",
        "pair_interaction_coverage_rows": out_dir / "pair_interaction_coverage_rows.jsonl",
        "pair_interaction_gap_rows": out_dir / "pair_interaction_gap_rows.jsonl",
        "issue1228_bridge_rows": out_dir / "issue1228_bridge_rows.jsonl",
        "validation_metrics": out_dir / "validation_metrics.json",
        "output_manifest": out_dir / "output_manifest.json",
    }
    artifact_info = {name: UTIL.artifact(path, jsonl=path.suffix == ".jsonl") for name, path in artifacts.items()}
    assertions = {
        "source_hashes_all_match": metrics["source_hashes_all_match"],
        "component_input_rows_1023": metrics["component_input_rows"] == 1023,
        "candidate_pair_rows_1750": metrics["candidate_pair_rows"] == 1750,
        "component_coverage_rows_match_unique_drugs": artifact_info["component_safety_coverage_rows"]["rows"] == metrics["unique_component_drugs"],
        "pair_coverage_rows_1750": artifact_info["pair_interaction_coverage_rows"]["rows"] == 1750,
        "pair_gap_rows_1750": artifact_info["pair_interaction_gap_rows"]["rows"] == 1750,
        "component_no_hit_rows_materialized": artifact_info["component_safety_no_hit_rows"]["rows"] == metrics["component_no_hit_rows"],
        "bridge_rows_bounded": len(bridge_rows) <= MAX_BRIDGE_ROWS,
        "bridge_terms_present_in_text": all(
            all(UTIL.clean_text(term).lower() in UTIL.clean_text(row.get("text")).lower() for term in row.get("bridge_terms", []))
            for row in bridge_rows
        ),
        "all_rows_blocked": metrics["all_component_rows_blocked"] and metrics["all_pair_rows_blocked"],
    }
    return {
        "schema_version": 1,
        "status": "ok" if all(assertions.values()) else "failed",
        "assertions": assertions,
        "artifacts": artifact_info,
        "metrics": metrics,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=DEFAULT_ROOT)
    parser.add_argument("--skip-input-sha-check", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.root)
    out_dir = root / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    source_rows = verify_sources(args.skip_input_sha_check)
    components = rows_jsonl(SOURCE_SPECS["issue1190_component_inputs"].path)
    pairs = rows_jsonl(SOURCE_SPECS["issue1190_candidate_pairs"].path)
    hypotheses = rows_jsonl(SOURCE_SPECS["issue1190_hypotheses"].path)
    flags = rows_jsonl(SOURCE_SPECS["issue1190_flags"].path)
    component_rows, component_no_hit_rows = build_component_rows(components)
    pair_rows, pair_gap_rows = build_pair_rows(pairs, hypotheses, flags)

    source_sha = UTIL.sha256_path(Path(SOURCE_SPECS["issue1190_candidate_pairs"].path))
    source_path = Path(SOURCE_SPECS["issue1190_candidate_pairs"].path)
    bridge_rows = build_bridge_rows(source_rows, component_rows, pair_rows, source_path, source_sha)

    write_json(out_dir / "input_manifest.json", {"schema_version": 1, "sources": source_rows})
    write_jsonl(out_dir / "source_coverage_rows.jsonl", source_rows)
    write_jsonl(out_dir / "component_safety_coverage_rows.jsonl", component_rows)
    write_jsonl(out_dir / "component_safety_no_hit_rows.jsonl", component_no_hit_rows)
    write_jsonl(out_dir / "pair_interaction_coverage_rows.jsonl", pair_rows)
    write_jsonl(out_dir / "pair_interaction_gap_rows.jsonl", pair_gap_rows)
    write_jsonl(out_dir / "issue1228_bridge_rows.jsonl", bridge_rows)

    metrics = build_metrics(
        source_rows,
        components,
        pairs,
        component_rows,
        component_no_hit_rows,
        pair_rows,
        pair_gap_rows,
        bridge_rows,
    )
    write_json(out_dir / "validation_metrics.json", metrics)
    output_manifest = {
        "schema_version": 1,
        "clinical_boundary": CLINICAL_BOUNDARY,
        "artifacts": {
            "source_coverage_rows": UTIL.artifact(out_dir / "source_coverage_rows.jsonl", jsonl=True),
            "component_safety_coverage_rows": UTIL.artifact(out_dir / "component_safety_coverage_rows.jsonl", jsonl=True),
            "component_safety_no_hit_rows": UTIL.artifact(out_dir / "component_safety_no_hit_rows.jsonl", jsonl=True),
            "pair_interaction_coverage_rows": UTIL.artifact(out_dir / "pair_interaction_coverage_rows.jsonl", jsonl=True),
            "pair_interaction_gap_rows": UTIL.artifact(out_dir / "pair_interaction_gap_rows.jsonl", jsonl=True),
            "issue1228_bridge_rows": UTIL.artifact(out_dir / "issue1228_bridge_rows.jsonl", jsonl=True),
            "validation_metrics": UTIL.artifact(out_dir / "validation_metrics.json"),
        },
    }
    write_json(out_dir / "output_manifest.json", output_manifest)
    readback = build_readback(out_dir, metrics, bridge_rows)
    write_json(out_dir / "persisted_readback.json", readback)
    print(json.dumps({"status": readback["status"], "root": str(root), "metrics": metrics}, sort_keys=True))
    return 0 if readback["status"] == "ok" else 1


if __name__ == "__main__":
    raise SystemExit(main())
