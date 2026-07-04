# #1183 All-Pair Typed Association Miner

## Scope

#1183 adds a bounded typed association miner over the #1173 biomedical overlay
graph. It scans persisted `typed_edges.jsonl`, deduplicates repeated or reversed
`associated_with` edges into typed concept-pair hypotheses, requires a passing
#1182 validation report, and writes readback-verifiable artifacts.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. Every emitted row is an
association hypothesis with explicit counter-evidence hooks for #1184 and
safety triage before any downstream claim can be trusted.

## Code Change

Commit:

```text
0d64a449 Add typed association miner
```

New CLI:

```text
calyx typed-association-miner \
  --typed-root <dir> \
  --validation-report <json> \
  --out-dir <dir> \
  [--source-type <concept-type>] \
  [--target-type <concept-type>] \
  [--name-contains <text>] \
  [--source-issue <n>] \
  [--min-support <n>] \
  [--max-pairs <n>] \
  [--max-input-edges <n>] \
  [--max-paths-per-pair <n>]
```

Artifacts written and read back:

- `typed_association_miner_report.json`
- `hypotheses.jsonl`
- `score_summary.json`

The command fails closed if the #1182 validation report did not pass, if typed
nodes are missing, if no candidates remain after filters, or if persisted
artifact bytes differ on readback. Existing different output files are not
overwritten.

## Behavior

- Full typed-edge JSONL is streamed, not loaded wholesale.
- Filters are orientation-aware: a disease-to-chemical source edge can still
  emit a chemical-to-disease hypothesis when `--source-type chemical
  --target-type disease` is requested.
- Repeated/reversed edges for the same typed pair are deduplicated into one
  hypothesis, with support summed and paths capped by `--max-paths-per-pair`.
- Each hypothesis carries:
  - validation report SHA-256
  - source/target ids, names, and concept types
  - support count, path count, score, novelty score
  - source hashes/support CxIds when present
  - `requires_1184_falsification_sweep`
  - `requires_safety_triage_for_drug_or_intervention_claims`
  - clinical boundary text

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli typed_association_miner -- --nocapture
cargo test -p calyx-cli typed_association_miner_round_trips_through_tokens -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused unit evidence:

- parser accepts source/target/name/source issue/support filters
- failed #1182 validation report is refused
- artifacts persist with report readback
- reversed chemical/disease edges deduplicate into one filtered orientation
- token round-trip covers the new command

## Remote Build

`aiwonder` release build passed after fast-forwarding to `0d64a449`:

```bash
ssh aiwonder 'cd /home/croyse/calyx/repo && git pull --ff-only && cargo build --release -p calyx-cli'
```

Result:

```text
Finished `release` profile [optimized] target(s) in 35.48s
```

## Real FSV

Root:

```text
/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z
```

Inputs:

| Input | Path / SHA |
|---|---|
| Typed overlay | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z` |
| #1182 validation report | `/home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json` |
| Validation report SHA-256 | `7fb0aad1c7f66bea4c86c5d6d99084f1c2203769a494c6f6d58ff0071d0bf2c3` |

Commands:

```bash
./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad \
  --min-support 1 \
  --max-pairs 250 \
  --max-input-edges 200000

./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease \
  --source-type chemical \
  --target-type disease \
  --min-support 1 \
  --max-pairs 100 \
  --max-input-edges 200000

./target/release/calyx typed-association-miner \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --validation-report /home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z/out/association_validation_report.json \
  --out-dir /home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease \
  --source-type gene \
  --target-type disease \
  --min-support 1 \
  --max-pairs 100 \
  --max-input-edges 200000
```

Readback summary from persisted report bytes:

| Run | Nodes | Edges scanned | Limit hit | Candidate pairs | Emitted | Report SHA-256 |
|---|---:|---:|---|---:|---:|---|
| Broad | 85 | 116,753 | false | 267 | 250 | `973d939cfd8f2aec8ac1ef218233078f59c5524de865284c64bc3e1e490c1c8c` |
| Chemical/disease | 85 | 116,753 | false | 42 | 42 | `5614f6fc1594e6eb7ad318364637d73d19bc7d8107b7ebe0891864f001bdc03f` |
| Gene/disease | 85 | 116,753 | false | 9 | 9 | `3532e0bc3e03b0d45469e5cf371f6dddc46e97799d2182cfa0024b03ee658bf3` |

Top readback rows:

| Run | Hypothesis | Source | Target | Support | Score |
|---|---|---|---|---:|---:|
| Broad | `typed-assoc:concept:ncbi_mesh:C062735::concept:ncbi_mesh:C093875` | zafirlukast / chemical | montelukast / chemical | 28 | 1.0 |
| Chemical/disease | `typed-assoc:concept:ncbi_mesh:D016595::concept:ncbi_mesh:D010437` | Misoprostol / chemical | Peptic Ulcer / disease | 8 | 1.0 |
| Gene/disease | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` | Tnf / gene | Proteinuria / disease | 9 | 1.0 |

Artifact hashes:

| Run | `hypotheses.jsonl` SHA-256 | `score_summary.json` SHA-256 |
|---|---|---|
| Broad | `99394214a3147d34828dc2830b622b90ae434cadc407c89a57e3c57ae9144d15` | `b1ae7814ef57ef4f17a57dafeb66f27cbaf1ff2bc698264dc82ad1dbdfb542bb` |
| Chemical/disease | `ba1d310bdb38d2cefc654b0faab45252ce7223c9f6b9e68e743b33b78492828b` | `f0ee70b18e0fbbc7a0cb3426fd462955bb6c43866476076da4068b68aa63dc91` |
| Gene/disease | `845a2609eec7a392a3ccde10d8756a549eb91a71c9eaa96cee77874afff04518` | `d42c51dcf3150235543bc7fd6ed848429f4e607f208a111d659c7af661dea970` |

## Conclusion

#1183 is complete for the current typed overlay: the miner scans the persisted
overlay, requires the #1182 validation gate, deduplicates typed pairs, emits
bounded scored hypotheses, and proves the output by reading back artifact bytes.

The output is intentionally still hypothesis-only. The next required atomic
step is #1184 counter-evidence/falsification sweep before any ranked association
can be promoted beyond a traceable lead.
