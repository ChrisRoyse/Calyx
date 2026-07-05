# #1228 Safety/Interaction Coverage Rollup

## Scope

#1228 aggregates the sealed #1190 drug-combination worklist and all follow-on
safety/interaction source-mining outputs into one coverage table. The goal is
accounting: every #1190 component drug and candidate pair receives either
source-backed context or an explicit fail-closed no-hit/not-cleared row.

This is safety and interaction triage only. Source rows, no-hit rows,
adverse-event rows, label text, registry context, literature context, and
identity mappings are blockers or review inputs, not safety clearance,
efficacy, treatment guidance, dosing guidance, recommendation, clinical
actionability, pair-interaction proof, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1228_safety_interaction_coverage_rollup.py
```

The script:

- verifies 57 sealed input artifacts by row count and SHA-256;
- reads the original #1190 component and pair universes;
- aggregates component safety context from initial safety rows, openFDA label
  gates, Europe PMC safety/counter review, openFDA FAERS, OffSIDES, and RxNorm
  mappings;
- aggregates pair source context from DrugComb, NCI ALMANAC, CDCDB,
  ClinicalTrials.gov, FDA/Orange Book/NDC/PubMed, openFDA labels, RxNorm,
  DailyMed, Europe PMC, FAERS, PubChem, ChEMBL, DrugCentral, PharmGKB, nSIDES,
  and case-quality overlays;
- emits explicit fail-closed gap rows for every #1190 candidate pair;
- writes a bounded bridge-corpus slice for native Calyx materialization.

## Real FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z/out/persisted_readback.json
sha256: 3ec89175f6cf47b144238a99dd0bb454527e7aa7723799edba6ae2e71b6774d4

/home/croyse/calyx/fsv/issue1228-safety-interaction-coverage-rollup-20260705T032413Z/out/calyx_bridge_corpus_readback.json
sha256: f2da81177e9e18fbde45d0cdcb22efbbb7cc4afd9d9ffda12eb7ff4017b80efb
```

Native Calyx materialization:

```text
name: issue1228-safety-interaction-coverage-rollup-20260705t032413z
vault_id: 01KWR4XV6G2AT5QXCPCEW3S2Q6
vault_dir: /home/croyse/calyx/vaults/01KWR4XV6G2AT5QXCPCEW3S2Q6
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 1,421 |
| Graph nodes | 2,421 |
| Graph edges | 7,446 |
| Graph SST files | 9,870 |
| Graph bytes | 9,322,240 |
| CSR persisted | true |
| Active vault index contains name | true |
| Active vault index vault id matches | true |
| Vault `CURRENT` / `MANIFEST` present | true |

## Output Artifacts

| Artifact | Rows | SHA-256 |
|---|---:|---|
| `source_coverage_rows.jsonl` | 57 | `fd22d3800e5360b85c5c33dadec3838cd8b440524aaf9e40e0e0152d5e4eb18d` |
| `component_safety_coverage_rows.jsonl` | 277 | `b11d5d384c315e86666620fa02c1cdae219a6133e7ac410cd28e9e580cc0f6ae` |
| `component_safety_no_hit_rows.jsonl` | 113 | `b5f1558a9b14c1c438c2d0863fd0981702b31665d511243cb9632940efc681df` |
| `pair_interaction_coverage_rows.jsonl` | 1,750 | `cd36320b038f8af781f7a1a3c6c5a864c16352e24212841a83a2451589e0d614` |
| `pair_interaction_gap_rows.jsonl` | 1,750 | `15e65084a54e90b9e6d161b3f054437f17d6320289c41fdf12d74a9352eb89e0` |
| `issue1228_bridge_rows.jsonl` | 1,000 | `59897db9fde850c2353a5f093ae67f00416f6a5aa0615980975994d02ef91df7` |
| `validation_metrics.json` | - | `71a2665e773348e2d8c29605ec4ffeecd912f0c4c55b10f4a4b2614ae81096da` |
| `output_manifest.json` | - | `ae2c2df386a9ba96e7101a42aede7b0b468d8a30d06f0a55e9c3a1208ce6ab24` |

## Metrics

| Metric | Count |
|---|---:|
| Sealed source input files | 57 |
| #1190 component input rows | 1,023 |
| Unique #1190 component drugs | 277 |
| Component safety-context present, still blocked | 164 |
| Explicit component safety no-hit rows | 113 |
| #1190 candidate pair rows | 1,750 |
| Pair coverage rows | 1,750 |
| Pair gap rows | 1,750 |
| Pairs with source context, not clearance | 1,234 |
| Explicit pair source no-hit rows | 516 |
| Bridge rows materialized | 1,000 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Source hashes and row counts match expected sealed values | true |
| Component input rows = 1,023 | true |
| Candidate pair rows = 1,750 | true |
| Component coverage rows match unique drugs | true |
| Pair coverage rows = 1,750 | true |
| Pair gap rows = 1,750 | true |
| All component and pair rows remain blocked | true |
| Bridge rows are bounded to 1,000 | true |
| Bridge terms appear in bridge text | true |
| Materializer status is `ok` | true |
| Bridge-row SHA matches materializer stdout | true |
| Vault directory and graph SST files exist | true |
| Graph node/edge counts match materializer readback | true |

## Findings

- #1190's broad component universe is now explicitly accounted for: 277 unique
  drug terms from 1,023 component rows have coverage rows, and 113 have explicit
  no-hit fail-closed rows.
- #1190's candidate-pair universe is now explicitly accounted for: all 1,750
  candidate rows have pair coverage rows and gap rows.
- Source context is present for 1,234 candidate pairs, but every such row remains
  a review blocker rather than a clearance or pair-interaction proof.
- 516 candidate pairs have explicit no-hit rows for pair source context.
- The aggregate evidence is materialized in native Calyx vault
  `01KWR4XV6G2AT5QXCPCEW3S2Q6`.

## Conclusion

#1228 is satisfied as coverage accounting: every #1190 component and pair is
covered by source-backed context or explicit fail-closed missing/not-cleared
rows, with source hashes, native Calyx materialization, and readback evidence.

No treatment claim, efficacy claim, safety-clearance claim, pair-interaction
proof, clinical-actionability claim, recommendation, dosing guidance, or cure
claim is made.
