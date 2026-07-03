# 24 - Typed biomedical overlay graph

- **Issue:** #1173
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for the first typed overlay graph.
- **FSV root:** `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z`

## Bottom line

The current association result pack, source expansion, concept-normalization layer, and molecular bridge rows are now joined into a typed graph.

- `7,928` nodes.
- `116,753` typed edges.
- `0` untyped or invalid edges.
- `1,949` association-result rows consumed.
- `2,612` source-expanded CxIds consumed.
- `2,575` normalized annotation rows consumed.
- `3,268` unresolved concept rows consumed.
- `8` molecular bridge rows consumed.

This overlay makes the current evidence queryable by biomedical concept and provenance. It still does not prove treatment efficacy, causality, novelty, or clinical actionability. `associated_with` edges are co-mention candidates until external relation evidence, counter-evidence, safety, and validation gates confirm or reject them.

## Persisted artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `typed_nodes.jsonl` | CxId, association result, concept, unresolved term, evidence row, and sequence nodes | `749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab` |
| `typed_edges.jsonl` | Typed provenance, mention, support, co-mention, molecular, and sequence edges | `1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85` |
| `typed_graph_summary.json` | Input hashes, counts by node/edge/source type, and scope | `def9fb26ccdb3aae11d2df044fc796813060ca2283881f3f1d268af22d776d3f` |
| `persisted_readback.json` | Separate readback from persisted graph artifacts | `d1d61737dd481d6d7730d889526179ae1cd0204cdd9f7e58a8ceeb27388fc76e` |
| `top_associated_concept_pairs.json` | Top concept co-mention candidates by support count | `f4944fa7b2bdc2013a61ac9f65ea55744118cbbc73a690519a6408dbd637ba04` |
| `untyped_or_invalid_edges.jsonl` | Empty fail-closed invalid-edge artifact | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

## Input hashes

| Input | SHA256 |
|---|---|
| #1170 `association_result_pack.jsonl` | `50c9d6902d71302c118599ee1101f1e335e41c23254fd9e2c688d9427a70db37` |
| #1171 `complete_cxid_source_expansion.jsonl` | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| #1171 `molecular_rows_expansion.jsonl` | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| #1172 `normalized_concept_annotations.jsonl` | `ca01a27b65061b22e9af869be89d72e379fdb14e6c0f22b872b1f7289ede4000` |
| #1172 `unresolved_or_ambiguous_concepts.jsonl` | `a5750dfdecda7547cd92e91d1ef8ce3efa7fb50d6cb5b230a079c0a814f86f5c` |
| #1172 `validation_samples.json` | `23cd7180e74fc71d94c1a56f474e2e86ff68ee12371d5fe4a8043440f553283b` |

## Readback

Separate persisted readback:

```json
{
  "edge_count_matches_summary": true,
  "node_count_matches_summary": true,
  "typed_edges_jsonl_readback": 116753,
  "typed_edges_sha256": "1fab25cc87f4b42309589f1ff2efdc340670505d56187ae7a91b4acebc8ffd85",
  "typed_nodes_jsonl_readback": 7928,
  "typed_nodes_sha256": "749f4dbbaecd90c8da0afc2f0ab93dc022a61033e820bf60db29cabbceb5caab",
  "untyped_or_invalid_edge_count": 0,
  "untyped_or_invalid_edges_jsonl_readback": 0
}
```

## Node counts

| Node type | Count |
|---|---:|
| `association_result` | `1,949` |
| `concept` | `85` |
| `cxid` | `2,616` |
| `evidence_row` | `8` |
| `sequence` | `2` |
| `unresolved_term` | `3,268` |

Concept nodes by type:

| Concept type | Count |
|---|---:|
| chemical | `38` |
| disease | `34` |
| gene | `8` |
| gene_protein | `2` |
| variant | `3` |

## Edge counts

| Edge type | Count | Meaning |
|---|---:|---|
| `uses_cxid` | `85,101` | Association result row links back to original CxIds. |
| `supports` | `23,884` | Association result row supports a normalized concept through its CxIds. |
| `mentions_unresolved` | `4,902` | Source row/example mentions a term that was not safely normalized. |
| `mentions` | `2,575` | Source CxId exact-span mention of a normalized concept. |
| `associated_with` | `267` | Concept co-mention candidate inside verified source rows. |
| `mentions_external_validated` | `15` | External validation sample or molecular row mention. |
| `binds` | `3` | BindingDB molecular binding row edge. |
| `same_as` | `3` | External identifier equivalence from validation or BindingDB aliasing. |
| `has_protein_sequence` | `2` | DPP4 concept to BindingDB target sequence. |
| `has_dna_sequence` | `1` | DPP4 concept to NCBI RefSeq DNA/mRNA sequence. |

Source issue coverage:

| Issue | Edge count |
|---:|---:|
| #1172 | `7,483` |
| #1173 | `282` |
| #875 | `2,428` |
| #876 | `8` |
| #877 | `356` |
| #878 | `105,157` |
| #880 | `354` |
| #881 | `354` |
| #882 | `326` |
| #883 | `2` |
| #884 | `3` |

## Top associated concept candidates

The strongest current co-mention candidates are dominated by asthma-pharmacology teaching clusters, which makes them useful as calibration/known-positive signals rather than novel discoveries.

| Rank | Source concept | Target concept | Support CxIds | Status |
|---:|---|---|---:|---|
| 1 | zafirlukast | montelukast | `28` | co-mention candidate |
| 2 | Ipratropium | Theophylline | `28` | co-mention candidate |
| 3 | Ipratropium | Steroids | `25` | co-mention candidate |
| 4 | Steroids | Theophylline | `25` | co-mention candidate |
| 5 | Tiotropium Bromide | Ipratropium | `18` | co-mention candidate |
| 6 | Prednisolone | Steroids | `18` | co-mention candidate |
| 7 | zileuton | montelukast | `17` | co-mention candidate |
| 8 | montelukast | Steroids | `17` | co-mention candidate |
| 9 | montelukast | Theophylline | `17` | co-mention candidate |
| 10 | Ipratropium | Prednisolone | `17` | co-mention candidate |

Every row in `top_associated_concept_pairs.json` includes source/target concept IDs, names, concept types, support count, and supporting CxIds.

## Molecular bridge edges

The overlay preserves the current molecular bridge evidence without overstating it:

- BindingDB row `50408024` creates a `binds` edge from metformin (`CHEMBL1431`) to DPP4 (`NCBI Gene 1803`), with PMID `18068977` and DOI `10.1016/j.bmcl.2007.11.107`.
- BindingDB row `108468` creates metformin-to-Streptokinase A `binds` edges, but the Streptokinase target remains an unresolved molecular target in this overlay.
- #884 DPP4 protein and DNA rows create `has_protein_sequence` and `has_dna_sequence` edges to BindingDB target sequence `p1234` and NCBI RefSeq `NM_001935.4`.

This corrects the earlier shorthand that treated row `108468` as the DPP4 binding row. The DPP4 binding evidence in this overlay comes from row `50408024`; row `108468` is a separate metformin/Streptokinase A row.

## Failure behavior

The builder failed closed on:

- missing input files,
- input SHA256 mismatches,
- edges without `edge_type`,
- edges without `direction`,
- edges whose endpoints were not persisted nodes.

The invalid-edge artifact is intentionally empty:

```text
untyped_or_invalid_edges.jsonl
```

SHA256:

```text
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```

## Closeout

#1173 acceptance is met:

- Typed nodes and edges are persisted with provenance.
- Links back to original Aster CxIds and association result surfaces are preserved through `uses_cxid` and `supports` edges.
- Edge type, direction, support count, source dataset, source hash, source issue, and extraction method are present.
- Unknown/unresolved material is explicit through `unresolved_term` nodes and `mentions_unresolved` edges.
- Counts by node type, edge type, source issue, and source dataset are persisted and hash-backed.

The next dependency is #1174: ingest Open Targets target-disease evidence so typed graph candidates can be compared against an external target-disease source instead of only internal co-mention/provenance evidence.
