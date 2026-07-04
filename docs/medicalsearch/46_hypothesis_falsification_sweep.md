# #1184 Hypothesis Falsification Sweep

## Scope

#1184 adds a persisted-source falsification sweep for retained typed
association hypotheses. It reads #1183 miner reports, scans persisted
PubTator/PubMed, ClinicalTrials.gov, DGIdb, and Open Targets evidence roots,
then writes separate support evidence, counter-evidence, raw source manifest,
and final per-hypothesis flags.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. The sweep is a demotion and
triage instrument: it makes counter-evidence visible before atlas/human-review
publication.

## Code Change

Commits:

```text
04155875 Add hypothesis falsification sweep
2d1ffbd7 Constrain falsification source matching
```

The second commit corrected source applicability after the first real readback
showed Open Targets target-disease rows could otherwise create false counters
for gene-gene hypotheses. Final FSV below uses the corrected release build.

New CLI:

```text
calyx hypothesis-falsification-sweep \
  --hypotheses-report <json> \
  [--hypotheses-report <json> ...] \
  --pubtator-root <dir> \
  --clinicaltrials-root <dir> \
  --dgidb-root <dir> \
  --open-targets-root <dir> \
  --out-dir <dir> \
  [--max-hypotheses <n>]
```

Artifacts written and read back:

- `falsification_sweep_report.json`
- `support_evidence.jsonl`
- `counter_evidence.jsonl`
- `hypothesis_flags.jsonl`
- `raw_query_manifest.jsonl`

The command fails closed on missing reports, missing source roots/files, missing
hypothesis arrays, over-budget hypothesis count, parse failures, or readback
mismatch. Existing different output files are not overwritten.

## Evidence Classes

Support evidence currently includes:

- PubTator supporting literature rows.
- ClinicalTrials.gov registry hits and completed/results trial rows.
- DGIdb exact drug-gene interactions.
- Open Targets target-disease association scores when type-applicable.

Counter-evidence currently includes:

- PubTator negative text-signal rows.
- ClinicalTrials.gov stopped/withdrawn/suspended status rows.
- DGIdb exact-pair no-hit rows.
- Open Targets low-score target-disease rows when type-applicable.

Source applicability is enforced before text matching:

| Source | Applicable hypothesis types |
|---|---|
| ClinicalTrials.gov | chemical/disease |
| DGIdb | chemical/gene or chemical/gene_protein |
| Open Targets | gene/disease or gene_protein/disease |
| PubTator | all typed pairs |

Drug/chemical hypotheses also carry
`safety_toxicity_triage_pending_issue_1181` until the separate safety triage
issue is complete.

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
cargo test -p calyx-cli hypothesis_falsification_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused unit evidence:

- parser accepts repeated `--hypotheses-report` inputs and required roots;
- persisted source fixture produces support and counter-evidence rows;
- final report readback decodes the persisted flags;
- token round-trip covers the new command.

## Remote Build

`aiwonder` release build passed after fast-forwarding to `2d1ffbd7`:

```bash
ssh aiwonder 'cd /home/croyse/calyx/repo && git pull --ff-only && cargo build --release -p calyx-cli'
```

Result:

```text
Finished `release` profile [optimized] target(s) in 36.26s
```

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1184-hypothesis-falsification-20260704T023537Z
```

Inputs:

| Input | Path |
|---|---|
| #1183 broad report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json` |
| #1183 chemical/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json` |
| #1183 gene/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |

Command:

```bash
./target/release/calyx hypothesis-falsification-sweep \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json \
  --hypotheses-report /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json \
  --pubtator-root /home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z \
  --clinicaltrials-root /home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z \
  --dgidb-root /home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z \
  --open-targets-root /home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z \
  --out-dir /home/croyse/calyx/fsv/issue1184-hypothesis-falsification-20260704T023537Z/out \
  --max-hypotheses 1000
```

Readback counts:

| Field | Value |
|---|---:|
| Input hypothesis rows | 301 |
| Deduped hypotheses | 280 |
| Raw source manifest rows | 7 |
| Support evidence rows | 59 |
| Counter-evidence rows | 1 |
| Hypotheses flagged with counter-evidence | 1 |
| Hypothesis flags read back | 280 |

Sweep status distribution:

| Status | Count |
|---|---:|
| `complete_no_counterevidence_found_in_current_sources` | 279 |
| `complete_counterevidence_found` | 1 |

Counter-evidence row:

| Hypothesis | Source | Reason | Weight | Summary |
|---|---|---|---:|---|
| `typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D007674` | Open Targets | `open_targets_low_score_exact_pair` | 0.5 | Open Targets low score `0.03701863799150296` |

Highest falsification-score flag:

| Hypothesis | Pair | Counter | Support | Score | Reasons |
|---|---|---:|---:|---:|---|
| `typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D007674` | PLA2R1 / Kidney Diseases | 1 | 4 | 0.092 | `open_targets_low_score_exact_pair` |

## Artifact Hashes

| Artifact | SHA-256 |
|---|---|
| `falsification_sweep_report.json` | `4c3c8bd121a45df9fcf5ab1d3a05b72204539f893cf3b9b8d821399550e2ef5c` |
| `support_evidence.jsonl` | `01d66f0fb6a1644dda9ba3a4c57eaa6dbb595665ce424514e80422cb895e23d0` |
| `counter_evidence.jsonl` | `f0e4f62da5c719c48579c832cf24e5adae607245ff6848aaa4d27c82e84f3637` |
| `hypothesis_flags.jsonl` | `9d80c503a5173e8a3056101c132b1b299905e801a634d87aabf5bcab862e3e77` |
| `raw_query_manifest.jsonl` | `bd2c94ae621a089cea8cac7826284ba3d00d5065d3b04928f5b07ca5ceef3bbf` |
| `stdout.json` | `d52d151ac8b7a76356b529ff045a65c6a0d0f5e917029ebb3036f6f1115d1aef` |

Raw source manifest hashes:

| Source | Role | Bytes | SHA-256 |
|---|---|---:|---|
| PubTator | supporting_literature | 532,523 | `bf473c33e99f596411116b8fb4a165ca1dd893a73399d552efa8979689ad9cb0` |
| PubTator | negative_literature | 2,982 | `2ded353b125e85436a6fad4d431c61f760bb4aa2de2112ac7a68acec4002dd08` |
| ClinicalTrials.gov | seed_summaries | 10,951 | `00d7be7f73876ade7158350c1ff08b0d377a67bd8ef8e98e035095276caca2e3` |
| ClinicalTrials.gov | trial_rows | 424,800 | `ee845c959cb4d9144b4ab38d37e76411a7c8e6c7899c688b3d038fb987533663` |
| DGIdb | seed_pair_interactions | 63,220 | `1ab68b172f383c93b3d4308b143e0028322b800551ab39daad4e8984bee36994` |
| DGIdb | unmapped_no_hit_rows | 971 | `228bd8df335a75045a3ceb596da5d17a0041501807a0ceaa29afdd63c92ced50` |
| Open Targets | validation_edges | 1,641,000 | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |

## Conclusion

#1184 is complete for the retained #1183 hypotheses: every deduped hypothesis
now has a persisted falsification sweep status before atlas publication. The
single counter-evidence flag demotes PLA2R1 / Kidney Diseases for low Open
Targets association score in the current source set.

The absence of counter-evidence in these bounded sources is not proof of truth,
safety, efficacy, actionability, or cure. Drug/chemical hypotheses still require
#1181 safety/adverse-event triage before any promotion beyond traceable lead.
