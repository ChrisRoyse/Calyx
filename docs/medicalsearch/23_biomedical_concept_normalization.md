# 23 - Biomedical concept normalization

- **Issue:** #1172
- **Date (UTC):** 2026-07-03
- **Status:** Complete bounded first pass with explicit unresolved accounting.
- **FSV root:** `/home/croyse/calyx/fsv/issue1172-biomedical-concept-normalization-20260703T154840Z`

## Bottom line

The #1171 expanded evidence rows now have a persisted biomedical concept-normalization layer.

- `2,620` evidence rows processed: `2,612` expanded CxId source rows plus `8` molecular bridge rows.
- `3,347` candidate terms extracted.
- `1,200` terms queried against PubTator3 entity autocomplete.
- `79` unique terms normalized to stable biomedical IDs.
- `3,268` terms explicitly unresolved or ambiguous.
- Accounting is complete for the extracted term set: `79 + 3,268 = 3,347`.
- `2,575` exact-span annotation rows persisted.

This is a candidate normalizer, not a biomedical truth set. It uses exact-span lexical matching and PubTator3 autocomplete as a bounded first pass. Short spans and generic biomedical words can map incorrectly, so downstream typed scoring must run context/ontology QC before using these annotations as assertions.

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `candidate_terms.jsonl` | Extracted candidate terms with occurrence counts and source-span examples | `673c16cc0869360fffd2e6f84f679d8b924a6e5267f7f3c6eb5fea84ae0c9cb5` |
| `normalized_concept_annotations.jsonl` | Exact source-span annotations with normalized ID, source DB, confidence, normalizer URL, and response hash | `ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000` |
| `unresolved_or_ambiguous_concepts.jsonl` | Terms not safely normalized, including unqueried terms from the bounded API pass | `a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c` |
| `pubtator_autocomplete_cache.jsonl` | Persisted PubTator3 autocomplete responses for queried terms | `ab61be27a39faa4cd8e387e3e67ce2706864d08295c79366c1c100ee03c8edec` |
| `validation_samples.json` | Known drug/disease/gene-protein/variant validation examples and persisted annotation samples | `23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b` |
| `readback_summary.json` | Counts, input hashes, accounting status, and unresolved reason counts | `e994fc60e53ccd4c6f07d2f911462bd7b3c0db9f18ae88e5bbfd05421d047edc` |
| `persisted_readback.json` | Separate readback from persisted artifacts | `ac84ea014c24bd909a87ee19dd1f9df732d86c8d060213113df02b74d8f12c5c` |
| `postprocess_manifest.json` | Audit record for adding unresolved rows for unqueried terms | `576956e5ca82f394efacdc69d7afc5fdefa280ae35be03e22aaf58bea96a5827` |

The original unresolved artifact before unqueried-term accounting is retained as:

```text
unresolved_or_ambiguous_concepts.before_unqueried_accounting.jsonl
```

SHA256:

```text
fbedc37d1322b73ba3909b8595b8b82dea1b2dc8fddc332868b578bfb152ede2
```

## Input hashes

| Input | SHA256 |
|---|---|
| #1171 source expansion `complete_cxid_source_expansion.jsonl` | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| #1171 molecular rows `molecular_rows_expansion.jsonl` | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| #1170 result pack `association_result_pack.jsonl` | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |

## Readback

Separate persisted readback:

```json
{
  "annotation_rows_jsonl_readback": 2575,
  "annotation_sha256": "ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000",
  "cache_rows_jsonl_readback": 1200,
  "cache_sha256": "ab61be27a39faa4cd8e387e3e67ce2706864d08295c79366c1c100ee03c8edec",
  "candidate_term_accounting_status": "complete_for_extracted_terms",
  "candidate_terms_accounted_for_by_normalized_or_unresolved": 3347,
  "candidate_terms_jsonl_readback": 3347,
  "candidate_terms_sha256": "673c16cc0869360fffd2e6f84f679d8b924a6e5267f7f3c6eb5fea84ae0c9cb5",
  "unresolved_rows_jsonl_readback": 3268,
  "unresolved_sha256": "a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c",
  "validation_samples_json_readback": true,
  "validation_samples_sha256": "23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b"
}
```

## Annotation counts

| Concept type | Annotation rows |
|---|---:|
| chemical | `1,261` |
| disease | `1,185` |
| gene | `115` |
| variant | `14` |

Unique normalized terms by type:

| Concept type | Terms |
|---|---:|
| chemical | `37` |
| disease | `33` |
| gene | `8` |
| variant | `1` |

## Unresolved accounting

| Reason | Rows |
|---|---:|
| `not_queried_bounded_api_budget` | `2,147` |
| `api_error` | `1,077` |
| `no_exact_or_close_name_match` | `23` |
| `no_results` | `21` |

The high `api_error` count came from transient PubTator3 autocomplete `502` responses during the run. Those terms were not guessed; they were written to the unresolved artifact.

## Validation samples

The validation file contains two kinds of evidence:

1. Persisted normalizer examples with exact spans and source IDs:
   - chemical: `Quinidine`, MeSH `D011802`
   - disease: `Sarcoidosis`, MeSH `D012507`
   - gene: `CD4`, NCBI Gene `920`
   - variant candidate: `p.T1027I`, LitVar `#43740568#p.T1027I`

2. Known source-vocabulary checks for terms that were important to the molecular/clinical bridge but hit PubTator autocomplete limits:
   - chemical: `Metformin`, MeSH `D008687`
   - disease: `Asthma`, MeSH `D001249`
   - gene/protein: `DPP4`, NCBI Gene `1803`, UniProtKB `P27487`
   - variant: `Factor V Leiden`, ClinVar `VCV000000642`, dbSNP `rs6025`

Source URL readback returned HTTP `200` for those external validation URLs at artifact creation time.

## Known limitations

- Exact-span lexical extraction is intentionally broad. It produces useful candidates but also false positives.
- Short spans such as `T10`, generic spans such as `Protein`, and ambiguous abbreviations require context checks before downstream use.
- PubTator3 autocomplete is useful for first-pass normalization but is not enough for final relation evidence or variant resolution.
- The unresolved rows are not dead ends; they are the work queue for improved NER, source-specific ontology lookup, PubTator/PubMed relation ingest, and external KG validation.

## Closeout

#1172 acceptance is met for a bounded first pass:

- Normalized annotations persist exact source spans, source CxIds, source dataset/hash metadata, normalizer URL, normalizer response hash, source vocabulary, source ID, confidence, and scope.
- Unresolved rows are explicit for every extracted term that was not normalized.
- Known drug, disease, gene/protein, and variant samples were validated against source vocabularies with persisted readback.
- The output is grounded in real #1171 expanded evidence rows and #1170 result-pack hashes.

The next useful step is #1173: build a typed biomedical association overlay graph from these normalized and unresolved concept artifacts, while preserving the limitation that first-pass annotations are candidates until context/ontology QC confirms them.
