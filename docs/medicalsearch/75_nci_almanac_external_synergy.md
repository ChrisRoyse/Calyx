# #1229 NCI ALMANAC External Combination Evidence Ingest

## Scope

#1229 ingests the public CellMiner/NCI ALMANAC processed combo-score workbook
and joins it to the #1190 drug-combination candidate pairs. The purpose is to
turn an external preclinical source into typed, persisted evidence rows inside
the Calyx discovery substrate.

No row is efficacy, safety, clinical actionability, treatment guidance, dosing,
recommendation, or cure evidence. ALMANAC and DrugComb evidence are preclinical
or model evidence only; missing safety, pair-interaction, and real outcome gates
remain hard blocks.

## Implementation

Script:

```text
scripts/medicalsearch/issue1229_nci_almanac_synergy_ingest.py
```

The script:

- parses the NCI ALMANAC XLSX workbook with a stdlib ZIP/XML reader;
- emits one pair-level row for every workbook drug pair;
- emits atomic per-cell-line combo-score rows;
- joins ALMANAC and existing #1190 DrugComb evidence to every #1190 candidate
  pair;
- assigns each #1190 candidate a deterministic external evidence status:
  `exact_hit`, `normalized_hit`, or `no_external_hit`;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

External source:

- CellMiner/NCI ALMANAC source page:
  <https://discover.nci.nih.gov/cellminer/html/drug_almanac_combo_score.html>
- Processed dataset download:
  <https://discover.nci.nih.gov/cellminer/download/processeddataset/DTP_NCI60_ALMANAC_COMBO_SCORE.zip>
- Dataset metadata page:
  <https://discover.nci.nih.gov/cellminer/datasets.do>

The workbook metadata read back by the parser:

| Field | Value |
|---|---|
| CellMiner Database Version | `2.15` |
| Human Genome Version | `HG-19` |
| Date | `09-17-2025` |

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/persisted_readback.json
sha256: b7d6ce0ffb7a3843cc0b3be5755a89db6f5ac647b6e99a05c2f24a27e961f3b1

/home/croyse/calyx/fsv/issue1229-nci-almanac-synergy-20260704T140500Z/out/calyx_bridge_corpus_readback.json
sha256: 0f5dcb5efb25428d3738054e16d1bd07107041a8f1d604e2e070e9c076da6987
```

Native Calyx materialization:

```text
name: issue1229-nci-almanac-synergy-20260704t140500z
vault_id: 01KWPPR0HC0Z02P5SDB1QEZWQ2
vault_dir: /home/croyse/calyx/vaults/01KWPPR0HC0Z02P5SDB1QEZWQ2
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 356 |
| Graph nodes | 1,356 |
| Graph edges | 10,000 |
| CSR persisted | true |
| Active vault index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph SST files present | true |
| Time-index SST files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| ALMANAC ZIP | 1,653,406 bytes | `161a0801e5a34c1fd1a3ae5b3c743d0b481d979b1c61fb08c75031b980233a0a` |
| ALMANAC XLSX | 1,627,596 bytes | `f43ca26735aa58152410ce4145ccdeffc0ba78c56c7279b7b48ef372f2b3c52b` |
| #1190 candidate pairs | 1,750 rows | `d61a04e62114d4124fade8a9f5a2a506c500a601d4c35615e56866aa3b697654` |
| #1190 combination hypotheses | 1,750 rows | `f5bf39325a5905770203b7326ded955831cf5e44e1797f1e37139ae1a8bfd484` |
| #1190 DrugComb matches | 28 rows | `9b075a80df9b6b252191d548ee68c93202e2ed9dfaa4430e226446aa7ad51a86` |

ALMANAC ZIP MD5 readback: `de0114d0730986b4d98f1b190189ee24`.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `almanac_pair_scores.jsonl` | 5,355 | 6,869,895 | `aa81aaaffc01b4087f36d34555e66593d412926733ffef7fb56a95764025347f` |
| `almanac_cellline_combo_scores.jsonl` | 306,365 | 199,787,593 | `6a60297dbd7051c4b9be7c14c29b5a9166667dcbda6de1adce569b126f341aae` |
| `candidate_external_synergy_status.jsonl` | 1,750 | 1,990,187 | `e4dbb332db9c4b656db83452a2f0debac9748c4c0cadb4b98c9280cfbcc6a4b7` |
| `candidate_external_synergy_hits.jsonl` | 68 | 151,003 | `b5f6135a8c86b8bb4c7115ff3f2c72d1384fc021f71c6832e4cc212545844a09` |
| `external_synergy_bridge_rows.jsonl` | 1,000 | 1,089,861 | `0f6d2fdfabbae4f536977a35bf5396b9465308910d65dbfcfe8f727742c29ec1` |
| `input_manifest.json` | - | 2,898 | `b6b71ddad5ea33c850d8ae7df208c641e8920c82ecd86ade5100f10cceec3af6` |
| `output_manifest.json` | - | 2,587 | `fd5db8ee0481e3c5138458c4d6982395bb4cb4e7036a9e6191fcacea3a374cef` |
| `validation_metrics.json` | - | 8,414 | `9907dba76b8944c2f62ba6a8079ea27abedc4bc8632a12e261862688c8c14e3c` |
| `persisted_readback.json` | - | 2,854 | `b7d6ce0ffb7a3843cc0b3be5755a89db6f5ac647b6e99a05c2f24a27e961f3b1` |
| `calyx_bridge_corpus_stdout.json` | - | 671 | `9d52898fa61e53e20eb9f7f0759e2d640067c2812f0299ef8e76dee91e8e52d1` |
| `calyx_bridge_corpus_readback.json` | - | 3,188 | `0f5dcb5efb25428d3738054e16d1bd07107041a8f1d604e2e070e9c076da6987` |

## Metrics

| Metric | Count |
|---|---:|
| ALMANAC pair rows | 5,355 |
| ALMANAC unique pair keys | 5,233 |
| ALMANAC cell-line score rows | 306,365 |
| ALMANAC pair rows with positive combo score in at least one cell line | 5,274 |
| #1190 candidate pair rows joined | 1,750 |
| Candidate rows with ALMANAC hit | 31 |
| Candidate rows with DrugComb hit | 47 |
| Candidate rows with any external hit | 68 |
| Candidate rows with both ALMANAC and DrugComb hit | 10 |
| Candidate rows with no external hit | 1,682 |

External evidence status counts:

| Status | Count |
|---|---:|
| `exact_hit` | 63 |
| `normalized_hit` | 5 |
| `no_external_hit` | 1,682 |

Reason-code counts after the external evidence join:

| Reason | Count |
|---|---:|
| `component_blocked_or_demoted_before_combination` | 1,750 |
| `component_safety_missing_fail_closed` | 1,727 |
| `pair_interaction_evidence_missing_fail_closed` | 1,745 |
| `external_synergy_evidence_missing_fail_closed` | 1,682 |
| `external_synergy_recheck_required_not_a_pass` | 21 |
| `overlapping_component_safety_flags_review_required` | 17 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| ALMANAC pairs present | true |
| ALMANAC scores present | true |
| Score rows cover pair keys | true |
| Joined rows match #1190 candidates | true |
| Deterministic status for every candidate | true |
| All joined rows carry boundary | true |
| No clinical-claim rows | true |
| Bridge rows <= 1,000 | true |

## Findings

- NCI ALMANAC adds a second external preclinical combination-evidence source
  beyond DrugComb. It contributes 31 #1190 candidate hits; 10 overlap DrugComb.
- External evidence coverage improved from 47 DrugComb-hit rows to 68 total
  external-hit rows, but 1,682/1,750 candidates still have no external pair
  evidence.
- All 68 external-hit rows remain blocked/provisional. External preclinical
  support does not clear component-safety, pair-interaction, clinical outcome,
  human-review, dosing, or safety gates.
- The corrected bridge corpus is materialized in native Calyx vault
  `01KWPPR0HC0Z02P5SDB1QEZWQ2`.
- A first trial vault from the pre-correction run remains physically present and
  active because `retire-vault` refuses to retire a non-quarantined healthy
  vault. The final source of truth for #1229 is the corrected
  `20260704T140500Z` root and vault above.

## Conclusion

#1229 is complete for the NCI ALMANAC/external-synergy ingest slice: the public
ALMANAC workbook was downloaded, checksummed, parsed into atomic pair and
cell-line evidence rows, joined to all #1190 candidates, and materialized into a
native Calyx bridge-corpus vault.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
