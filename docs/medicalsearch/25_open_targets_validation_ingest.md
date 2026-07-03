# 25 - Open Targets validation ingest

- **Issue:** #1174
- **Date (UTC):** 2026-07-03
- **Status:** Complete bounded Open Targets validation ingest.
- **FSV root:** `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z`

## Bottom line

The #1173 typed overlay now has a bounded external Open Targets validation layer for mapped target and disease concepts.

- Open Targets release: `26.06`.
- API endpoint: `https://api.platform.opentargets.org/api/v4/graphql`.
- API version from live metadata: `26.6.0`.
- `44` overlay concept mapping attempts.
- `8` target concept mappings, representing `7` unique Open Targets targets.
- `26` disease concept mappings.
- `10` concepts persisted as unmapped/ambiguous.
- `77` Open Targets API responses persisted with request/response hashes.
- `1,422` association rows persisted.
- `1,420` validation edges persisted.
- `23` validation edges map both sides back to #1173 overlay concepts.
- `0` API errors.

This is validation and triage evidence only. Open Targets association scores are not verdicts, treatment recommendations, or proof that a target should be modulated for a disease.

## Source and release

Official documentation used:

- GraphQL API: `https://platform-docs.opentargets.org/data-access/graphql-api`
- Dataset downloads: `https://platform-docs.opentargets.org/data-access/datasets`

The live Open Targets `meta` query reported:

```json
{
  "apiVersion": {
    "x": "26",
    "y": "6",
    "z": "0"
  },
  "dataPrefix": "platform2606",
  "dataVersion": {
    "year": "26",
    "month": "06",
    "iteration": null
  }
}
```

The full Croissant metadata manifest is stored in:

```text
open_targets_metadata.json
```

SHA256:

```text
caf886b66a5f786bf71a63b9c8d141be4024c2dca872e39fdd1fac2b8089ec64
```

Download locations from the metadata:

| Location | URL |
|---|---|
| FTP | `http://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.06/output/` |
| GCP | `gs://open-targets-data-releases/26.06/output/` |
| AWS | `s3://open-targets-public-data-releases/platform/26.06/output/` |

The metadata download manifest string has SHA256:

```text
4775a7b7db016563f57219c3d96bbeb8cf489b50c91b5df83bcda3fa55be2faf
```

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `open_targets_api_responses.jsonl` | Every GraphQL request/response, including raw response JSON and request/response hashes | `39924964718433cd5f50d44448825a43afb6490e0b985152900b0b9b1897a119` |
| `open_targets_concept_mappings.jsonl` | Overlay concept to Open Targets target/disease mapping attempts | `788c845163e77876e01da635dccbc4d81bdc1d2a4c00ccff7014bfab7f966e77` |
| `open_targets_unmapped_concepts.jsonl` | Concepts not mapped because they had no hit or ambiguous top hit | `2a81eae7e998df2c7b6570fb88939c33f3bac6187a60ad92875f175ff1663af3` |
| `open_targets_association_rows.jsonl` | Top-50 target-associated diseases and disease-associated targets for mapped concepts | `6ceb368538c52b87cbb1fb661c661b7009a3b03f81be5f1b39f42f470e5b0ba6` |
| `open_targets_validation_edges.jsonl` | External validation edges with score, data source scores, data type scores, and overlay concept links where possible | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| `open_targets_metadata.json` | Full Open Targets metadata response, including release/download manifest | `caf886b66a5f786bf71a63b9c8d141be4024c2dca872e39fdd1fac2b8089ec64` |
| `readback_summary.json` | Compact counts, source paths, release, and hashes | `7e0837840e3ce341bfc5e59a03fe6e824f7888cc82d27068bc8545c1e3bc1c6f` |
| `persisted_readback.json` | Separate readback from persisted artifacts | `9fbc71f5dac11342cf262409d15a5088916be921e3c5fac7df89a57e09a8793a` |
| `api_errors.json` | Empty API error list | `37517e5f3dc66819f61f5a7bb8ace1921282415f10551d2defa5c3eb0985b570` |

## Input hashes

| Input | SHA256 |
|---|---|
| #1173 `typed_nodes.jsonl` | `749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab` |
| #1173 `typed_edges.jsonl` | `1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85` |

## Readback

Separate persisted readback:

```json
{
  "api_responses_jsonl_readback": 77,
  "association_rows_jsonl_readback": 1422,
  "concept_mappings_jsonl_readback": 44,
  "counts_match_summary": true,
  "open_targets_metadata_json_readback": true,
  "unmapped_concepts_jsonl_readback": 10,
  "validation_edges_jsonl_readback": 1420
}
```

## Mapping summary

Targets:

| Overlay concept | Open Targets target | Status |
|---|---|---|
| DPP4 / NCBI Gene `1803` | `ENSG00000197635` / DPP4 | mapped |
| DPP4 / UniProtKB `P27487` | `ENSG00000197635` / DPP4 | mapped |
| CD4 / NCBI Gene `920` | `ENSG00000010610` / CD4 | mapped |
| CD8A / NCBI Gene `925` | `ENSG00000153563` / CD8A | mapped |
| TNF / NCBI Gene `24835` | `ENSG00000232810` / TNF | mapped |
| PLA2R1 / NCBI Gene `22925` | `ENSG00000153246` / PLA2R1 | mapped |
| LTC4S / NCBI Gene `4056` | `ENSG00000213316` / LTC4S | mapped |
| NF1 / NCBI Gene `4763` | `ENSG00000196712` / NF1 | mapped |
| Lt1 / NCBI Gene `16991` | none | unmapped |
| SV40gp3 / NCBI Gene `29031016` | none | unmapped |

Disease mappings were conservative exact/token-equivalent matches. Examples include asthma, sarcoidosis, psoriasis, bacterial meningitis, proteinuria, hypertension, schizophrenia, silicosis, and Salmonella infections. Ambiguous or missing disease mappings are persisted in `open_targets_unmapped_concepts.jsonl`.

## Overlay-mapped associations

Top overlay-mapped validation edges:

| Target | Disease | Score | Evidence categories |
|---|---|---:|---|
| TNF | psoriasis | `0.6372348524495187` | literature, animal_model, clinical |
| TNF | sarcoidosis | `0.39272075294445863` | literature, animal_model, clinical |
| DPP4 | asthma | `0.3880634550607382` | literature, genetic_association |
| TNF | asthma | `0.37194382582424823` | literature, clinical |
| DPP4 | Proteinuria | `0.3549124108545795` | literature, clinical |
| DPP4 | Hypertension | `0.2807492875353422` | literature, clinical |
| DPP4 | schizophrenia | `0.19218936351670968` | literature, genetic_association, clinical |
| CD4 | asthma | `0.14213649903074943` | literature, rna_expression, clinical |
| CD4 | sarcoidosis | `0.11693800758175663` | literature |
| CD8A | psoriasis | `0.11503779807105693` | literature, rna_expression |

These are not new discoveries. They are external support/triage rows showing where the internal overlay intersects Open Targets target-disease evidence.

## Scope and limitations

- This is a bounded GraphQL ingest over the #1173 overlay concepts, not a full Open Targets mirror.
- The official docs recommend datasets/downloads or BigQuery for broad systematic pulls; the API was used here because the slice is concept-bounded and every response is persisted with a hash.
- Only top-50 association pages were pulled for each mapped target and disease.
- Conservative mapping intentionally leaves some plausible terms unmapped rather than guessing.
- The outputs are validation/triage features for later scoring, not verdicts.

## Closeout

#1174 acceptance is met:

- Exact Open Targets release, API endpoint, metadata, and download/source locations are persisted and hash-backed.
- Target and disease concepts from the overlay are mapped where possible and unmapped otherwise.
- Association scores, data source scores, and data type scores are persisted.
- Separate readback validates downloaded/API response rows, parsed rows, mappings, unmapped concepts, and validation graph rows.

The next dependency is #1175: scale ChEMBL/BindingDB molecular vault materialization beyond the narrow proof slice.
