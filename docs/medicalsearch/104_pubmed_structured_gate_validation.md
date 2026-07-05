# #1239 PubMed Structured Gate Validation

Status: complete for the downstream gate-validation pass over the sealed #1238
PubMed structured extraction and candidate rollup artifacts.

This slice reads #1238 as sealed input, preserves counter-evidence fail-closed,
emits deterministic safety/outcome/falsification/human-review preflight rows,
and materializes a bounded validation overlay into native Calyx.

Clinical boundary:

```text
PubMed structured gate validation is safety/outcome/falsification preflight only; structured literature rows, counter-evidence rows, and missing-gate rows are blockers or review inputs, not causality, safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1239-pubmed-structured-gate-validation-20260705T012109Z
```

Sealed #1238 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `pubmed_structured_extraction.jsonl` | 510 | `10bc9ca90b97a089962bd85ee2f7881819668414859fdedb8040c86d78fa992e` |
| `candidate_pair_pubmed_structured_rollup.jsonl` | 301 | `e0c8db57ee492727fa525c9044dfcce85bf03acbfd7a03587030ab2d8393c3e6` |
| `candidate_pair_pubmed_structured_hits.jsonl` | 298 | `cb6ce01e363c6c320231cfaf01e59c9fafb891c6d437e1a9bc2f5ca924874c26` |
| `persisted_readback.json` | - | `14e92d9f8908474385337d0b71d10531d991897e9edf2dcdfea015806eaba413` |
| `calyx_bridge_corpus_readback.json` | - | `5e88bb218b3e06e31ff2eb3313a911b6c58daec4d85e79e428e87be6c2bafdf8` |
| `output_manifest.json` | - | `8fc6728117d993bb74ea3e1090ee94136f449ded4c5c89f733ce19fe0ba9ce74` |

Source contract:

- Verify the #1238 artifact hashes before processing.
- Emit one evidence gate row per #1238 structured extraction row.
- Emit one candidate gate-status row per #1238 candidate rollup row.
- Emit explicit missing/not-cleared rows for component safety,
  pair-interaction, outcome endpoint, falsification, and human review.
- Preserve all counter-evidence rows as falsification-review blockers.
- Keep every row blocked behind independent safety, outcome, falsification,
  and human-review gates.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 6 | 5,946 | `a4c71d4144737832c6522613b2a1a304047654f034f3d5507da8cb55f2f12b02` |
| `pubmed_structured_gate_evidence.jsonl` | 510 | 1,162,137 | `ec8137c141f37fcf1282e7bbb752ba5e100a215bfaf75dd77eab3ea7961d5db2` |
| `candidate_pubmed_gate_status.jsonl` | 301 | 810,904 | `67a0ff300092839addc6bd32b4922da954ee1b2469a905e5192895c041b838a7` |
| `pubmed_missing_gate_rows.jsonl` | 1,505 | 1,545,376 | `6ce1a596517ccbf52fe3cad5a7a8ef1970d95fd03f3266f7b8b295cce707fc02` |
| `gate_summary.jsonl` | 1 | 1,988 | `fbf31aa3ff8d8f3de8ff8d943b74406092ee258b48ae81ec6756c39fe035aa15` |
| `issue1239_bridge_rows.jsonl` | 1,000 | 1,165,572 | `89d23691fd2b0d8f24d4404dae9d30ec0d09ff74af12f3a7405672513609138d` |
| `validation_metrics.json` | - | 2,179 | `e5185c29d42195c8b498c20850b8624e6c15d2574d59dd5ebadcd363bc4ab33f` |
| `output_manifest.json` | - | 6,287 | `3da4ca41406bd38b42ec214ac16424aada832413ba8f54420b743deb1f6f6705` |
| `persisted_readback.json` | - | 3,527 | `c8661ab0546bc128fde69fcf35ff79bf7d290f9662063cddc2711f14e85153ac` |
| `calyx_bridge_corpus_stdout.json` | - | 806 | `1e6e3285098150ca46780263ef0823eb36ec27b014dcd570502de8ace4864a5e` |
| `calyx_bridge_corpus_stderr.txt` | - | 331 | `94dee7b1a64dfae4640f4fe60c6455ec491e3fc4b882d9760540a454c1f3152c` |
| `calyx_bridge_corpus_readback.json` | - | 4,491 | `a8acf5aae36a4fd94325d146933ee0d13c96420e1ab99c155238aac3866ac8a2` |

## Metrics

| Metric | Count |
|---|---:|
| Source input rows | 6 |
| PubMed structured evidence gate rows | 510 |
| Candidate gate-status rows | 301 |
| Missing/not-cleared gate rows | 1,505 |
| Counter-evidence evidence rows preserved | 224 |
| Bridge rows | 1,000 |

Required-gate accounting:

| Gate | Rows |
|---|---:|
| `component_safety` | 301 |
| `pair_interaction` | 301 |
| `outcome_endpoint` | 301 |
| `falsification` | 301 |
| `human_review` | 301 |

Every #1238 candidate rollup has exactly five required-gate rows. Source
language can mark a gate as review input, but no source-language row clears an
independent gate.

## Validation Assertions

| Assertion | Result |
|---|---|
| Expected #1238 input hashes matched | true |
| Evidence gate rows = 510 | true |
| Candidate gate rows = 301 | true |
| Missing gate rows = 1,505 | true |
| Counter-evidence preserved = 224 | true |
| All evidence rows blocked | true |
| All candidate rows blocked | true |
| All missing-gate rows blocked | true |
| Every candidate has five required-gate rows | true |
| Bridge rows <= 1,000 | true |
| Bridge terms present in text | true |
| Bridge metadata `source_dataset` present | true |

## Native Calyx Materialization

```text
name: issue1239-pubmed-structured-gate-validation-20260705t012109z
vault_id: 01KWQXYT0KB77KMVZJ5A018QH5
vault_dir: /home/croyse/calyx/vaults/01KWQXYT0KB77KMVZJ5A018QH5
rows: 1,000
bridge_terms: 950
graph_nodes_written: 1,950
graph_edges_written: 6,000
csr_persisted: true
```

Materialization domain counts:

| Domain | Rows |
|---|---:|
| `issue1239_pubmed_gate_evidence` | 510 |
| `issue1239_candidate_gate_status` | 301 |
| `issue1239_missing_gate` | 182 |
| `issue1239_source_input` | 6 |
| `issue1239_gate_summary` | 1 |

Calyx readback assertions:

| Assertion | Result |
|---|---|
| Materialize stdout status ok | true |
| Bridge row count matches stdout | true |
| Bridge row SHA matches stdout | true |
| Graph node count matches written count | true |
| Graph edge count matches written count | true |
| CSR persisted | true |
| Index contains exactly one active materialization name | true |
| Index vault id matches stdout | true |
| Vault directory exists | true |
| `CURRENT` and `MANIFEST` exist | true |
| `cf/graph` SST files present | true |
| Vault-tree readback nonempty | true |
| Native manifest version readback ok | true |

The native manifest readback used:

```text
calyx readback vault-manifest --field version --vault /home/croyse/calyx/vaults/01KWQXYT0KB77KMVZJ5A018QH5
```

It returned:

```json
{"major":1,"minor":0}
```

During physical readback, `calyx readback --vault <vault> --show-manifest`
incorrectly routed this native vault through the shadow-manifest parser and
returned `CALYX_MANIFEST_CORRUPT`. That is tracked separately as #1262 and did
not invalidate the native vault-manifest readback above.

## Result

#1239 did not produce any efficacy, safety-clearance, dosing, recommendation,
clinical-actionability, pair-interaction-proof, or cure claim. It produced a
deterministic PubMed structured validation overlay: all 510 structured rows,
301 candidate rollups, and 1,505 required-gate rows remain blocked pending
independent safety, outcome, falsification, and human review.
