# #1222 Neuro Concept Normalization Repair

## Scope

#1222 repairs unresolved or ambiguous neuro terms surfaced by #1187. Accepted
terms are mapped with deterministic, source-backed MeSH descriptors. Ambiguous
or narrative phrases remain unresolved.

This is normalization and coverage repair only. It does not assert clinical
actionability, treatment guidance, safety, efficacy, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1222_neuro_normalization_repair.py
```

Script commit:

```text
47b70c9684e9e5b136beaa9b7a6423aa8325a77c
```

Local gates:

```text
python -m py_compile scripts/medicalsearch/issue1222_neuro_normalization_repair.py
git diff --check
```

Both passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z
```

Repair summary:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/issue1222_fsv_summary.json
```

#1187 rerun summary:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187_summary.json
```

Raw MeSH lookup responses were persisted under:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/raw
```

## Repair Coverage

| Metric | Before | After |
|---|---:|---:|
| Neuro normalized annotation rows | 222 | 721 |
| Neuro normalized source CxIds | 129 | 297 |
| Unique neuro concepts | 10 | 44 |
| Neuro unresolved terms | 70 | 22 |
| Accepted unresolved term rows | - | 48 |
| Added annotation rows | - | 499 |
| New co-mention association rows | - | 30 |
| Overlay delta nodes | - | 260 |
| Overlay delta edges | - | 499 |
| Overlay untyped accepted edges | - | 0 |

Remaining unresolved reason counts:

| Reason | Rows |
|---|---:|
| `api_error` | 9 |
| `not_queried_bounded_api_budget` | 13 |

## Required Examples

| Required term | Added rows | Normalized to | Example source CxId | Source SHA-256 |
|---|---:|---|---|---|
| Parkinsonism | 18 | Parkinsonian Disorders / `ncbi_mesh:D020734` | `0298683a4e9e41041bbb08458f045d6e` | `0e6b92a0c40c009400dda75854ee7f55d867200bc07932fe55ec9c02e066dbef` |
| Parkinson's disease | 21 | Parkinson Disease / `ncbi_mesh:D010300` | `0298683a4e9e41041bbb08458f045d6e` | `0e6b92a0c40c009400dda75854ee7f55d867200bc07932fe55ec9c02e066dbef` |
| Seizures | 103 | Seizures / `ncbi_mesh:D012640` | `00f89b05ed956703fbe7bce73c41cce4` | `33e89a56397bccccfcfc7e750c3fb5494bd85ec70fb520cc242ea406e8876bb4` |
| Vascular dementia | 13 | Dementia, Vascular / `ncbi_mesh:D015140` | `0c74148f46bc73b2b85566f7908fecd8` | `f40cb7f9b1bf9444dc41a6167bd7420e5e06e08049af044a3cf5eeb3fb3c7333` |
| Ischemic stroke | 4 | Ischemic Stroke / `ncbi_mesh:D000083242` | `2748324e759eedd47fdcd1d61e8e075a` | `8c027bc6323b2168a5e873abab45edd3508b83f6b4888cd7eaa10ceab95ffea4` |
| Optic glioma | 4 | Optic Nerve Glioma / `ncbi_mesh:D020339` | `11d8c83de6d5efe0eb06137302d8104f` | `dba4e97ff8d669c4f30ccb3bbc581b92209ce7843fddf2fd058da96e54a023a5` |
| Spinocerebellar ataxia | 5 | Spinocerebellar Ataxias / `ncbi_mesh:D020754` | `175408106c68d1c2bd92fcf7badd7f63` | `5f3a2e017bf212ad5506b2ff58809f1087ceca37b12b2cdc79fd42d9c4a209d9` |
| Multiple sclerosis | 52 | Multiple Sclerosis / `ncbi_mesh:D009103` | `17b6aed7f8e8952cf10f11b516630698` | `a9df4ce25eea4d974079b39a48237e83a0fac193b53aab1538ea015368d605d9` |
| Migraine | 27 | Migraine Disorders / `ncbi_mesh:D008881` | `099f9f22fa1d9f29c77596fac0b55b8f` | `7a1f1f766ce14ec9f6520a48fd765483a36c90441ce3a682bc9fc8ecab9031b9` |
| Paranoid schizophrenia | 1 | Schizophrenia, Paranoid / `ncbi_mesh:D012563` | `8b70e13560f718e6d3b08c30badac20a` | `3a515cfa8f99ee90e8b7dba0af6368d8623135f0e4bc109ab1b9c4a47afca47c` |

## Rerun Deltas

The #1187 hunt script was rerun with the full repaired normalization inputs:

```text
/home/croyse/calyx/fsv/issue1222-neuro-normalization-repair-20260704T111804Z/rerun_1187
```

| Metric | Original #1187 | Rerun | Delta |
|---|---:|---:|---:|
| Neuro annotations | 222 | 721 | +499 |
| Neuro unresolved terms | 70 | 22 | -48 |
| Neuro hypotheses | 77 | 131 | +54 |

Top hypothesis changed from:

```text
issue1187:opentargets_target_neuro:03088f07d62fb884ac77
```

to:

```text
issue1187:normalized_comention:3ab84708ef24c3b7bcfd
```

The new top is a normalized co-mention candidate and remains a research lead,
not a treatment or actionability claim.

Rerun readback assertions:

| Assertion | Value |
|---|---:|
| Hypothesis rows read back | 131 |
| Metrics hypothesis total | 131 |
| Row count matches metrics | true |
| Neuro annotation rows | 721 |
| Neuro unresolved rows | 22 |
| Raw query manifest rows | 42 |
| Safety/trial flag rows | 14 |
| Top bundle count read back | 20 |

## Artifact Readback

| Artifact | SHA-256 |
|---|---|
| `out/deterministic_mapping_table.json` | `939eaf1d61a8c97556414e40c5f2028ca60a86ef836a15cd9faca5525267d267` |
| `out/neuro_normalized_annotations_added.jsonl` | `5571dc491a1901582d6dd3565c51a33fc3c8f78569aa1c8ec88cd19a7cb5220a` |
| `out/neuro_normalized_annotations.repaired.jsonl` | `849fe6b9bd000cb720f2d219ef9ba8db4954a3b62bb4c87847aa7ad6dce2e43c` |
| `out/neuro_unresolved_terms.remaining.jsonl` | `bf558199ba3d857da7f9784e59a13d9f071d04655e91fc0608992769a90eeec6` |
| `out/full_normalized_concept_annotations.repaired.jsonl` | `ee457a8b420260025ae43e97fe17d6dbf245faa58fea1c95ec94d4838ea1ad7c` |
| `out/full_unresolved_or_ambiguous_concepts.repaired.jsonl` | `429bbf36c55d81f9e89ffa562084b6edfbb8841f7ccaf4e70014169aa78c3b45` |
| `out/typed_overlay_delta_nodes.jsonl` | `92769f1f4b0b670f9350b8deb9d2317829ac2df8c4c15ceeee022d2c27f85550` |
| `out/typed_overlay_delta_edges.jsonl` | `699f8c0d82dead5bb8fad98de3d0a326583cc824f60ed69b4953c456b6a6ac15` |
| `out/new_co_mention_association_coverage.jsonl` | `56bf029fa5f988f575441d4013a66c6deb8b1573d8f3aa9a7655fc123437fc34` |
| `rerun_1187/out/neuro_hypotheses.jsonl` | `9c23eb5e6ed726c5a3003fdbbd6edbb24b315ac63e919bd8a8f5c3c9b00e5bf6` |
| `rerun_1187/out/persisted_readback.json` | `7d0a3a76f04fde55b4fd0fc2897135cc78f3cd4b0e003e9930cde707d8e5aca3` |

## Conclusion

#1222 is complete for the targeted neuro normalization repair:

- 48 unresolved neuro rows were accepted into deterministic mappings;
- 22 unresolved rows remain explicit;
- all required neuro term families have row-level source-hash examples;
- the overlay delta has 499 typed mention edges and zero untyped accepted edges;
- the #1187 rerun using repaired inputs persisted 131 hypotheses with readback
  counts matching metrics.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.
