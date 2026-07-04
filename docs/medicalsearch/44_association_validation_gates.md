# #1182 Association Validation Gates

## Scope

#1182 adds a power-proven validation instrument for biomedical association
mining before broad all-pair hunts are accepted. The gate measures association
scoring against persisted known-positive rows, known-negative/no-hit controls,
and a time-split later-evidence benchmark.

This is not a cure, treatment recommendation, efficacy proof, safety proof,
causality proof, or clinical-actionability claim. It is a source-backed
instrument for accepting or rejecting association-mining runs.

The issue originally named `docs/medicalsearch/33_association_validation_gates.md`;
that number was already occupied by later append-only work. This file keeps the
append-only numbering and cross-links #1182.

## Code Change

Commit:

```text
2bf61244 Add biomedical association validation gates
```

New CLI:

```text
calyx association-validation-gates \
  --typed-root <dir> \
  --open-targets-root <dir> \
  --pubtator-root <dir> \
  --clinicaltrials-root <dir> \
  --dgidb-root <dir> \
  --out-dir <dir> \
  [--cutoff-year <yyyy>] \
  [--score-threshold <0..1>] \
  [--min-auroc <0..1>] \
  [--min-positive-recall <0..1>] \
  [--min-negative-suppression <0..1>]
```

Artifacts written and read back:

- `benchmark_source_rows.jsonl`
- `train_test_split.jsonl`
- `scored_outputs.jsonl`
- `metrics.json`
- `association_validation_report.json`

The command fails closed on missing source roots/files, empty class sets,
one-class AUROC, failed threshold gates, or artifact readback mismatch. Existing
different output files are not overwritten.

## Source Inputs

The final real run used these persisted source roots:

| Source | Root |
|---|---|
| Typed overlay graph | `/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |

These sources are also materialized into Calyx/Aster Graph CF through #1196
(`biomed_evidence_substrate_v3`). #1182 adds the acceptance instrument and
readback artifacts used by downstream miners.

## Local Gates

Passed:

```bash
cargo test -p calyx-cli association_validation -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

## Real FSV: Strict Threshold Failure

The first real run intentionally used the default `--score-threshold 0.5`.
It failed closed and persisted artifacts:

```text
/home/croyse/calyx/fsv/issue1182-association-validation-gates-20260704T020316Z
```

Readback:

| Field | Value |
|---|---:|
| Exit code | 2 |
| Known positives | 57 |
| Known negatives | 2 |
| Time-split rows | 13 |
| Scored outputs | 72 |
| Known AUROC | 1.000 |
| Known positive recall at 0.5 | 0.544 |
| Known negative suppression at 0.5 | 1.000 |
| Time-split AUROC | 0.864 |

Failure reason:

```text
known-positive recall 0.544 below 0.750
```

Interpretation: the ranking separated positives from no-hit controls, but the
fixed 0.5 threshold suppressed low-score Open Targets positives. This is a
calibration finding, not a system success.

## Real FSV: Passing Source-Inclusive Gate

The accepted gate run used an explicit `--score-threshold 0.05`, reflecting the
lowest admitted external-source score for this bounded validation instrument.

FSV root:

```text
/home/croyse/calyx/fsv/issue1182-association-validation-gates-pass-20260704T020404Z
```

Command shape:

```bash
./target/release/calyx association-validation-gates \
  --typed-root /home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z \
  --open-targets-root /home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z \
  --pubtator-root /home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z \
  --clinicaltrials-root /home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z \
  --dgidb-root /home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z \
  --out-dir "$ROOT/out" \
  --cutoff-year 2016 \
  --score-threshold 0.05
```

Readback:

| Field | Value |
|---|---:|
| Exit code | 0 |
| Gate passed | true |
| Known positives | 57 |
| Known negatives | 2 |
| Benchmark source rows | 59 |
| Time-split rows | 13 |
| Scored outputs | 72 |
| Wall time | 0:00.09 |
| Max RSS | 92,388 KB |

Known-positive/negative metrics:

| Metric | Value | CI |
|---|---:|---|
| AUROC | 1.000 | 1.000 - 1.000 |
| Precision | 1.000 | 0.937 - 1.000 |
| Positive recall | 1.000 | 0.937 - 1.000 |
| Negative suppression | 1.000 | 0.342 - 1.000 |

Time-split metrics:

| Metric | Value | CI |
|---|---:|---|
| AUROC | 0.864 | 0.864 - 0.864 |
| Precision | 0.846 | 0.578 - 0.957 |
| Positive recall | 1.000 | 0.741 - 1.000 |
| Negative suppression | 0.000 | 0.000 - 0.658 |

The time-split benchmark is useful but small: 11 later-positive and 2
later-negative ClinicalTrials.gov seed rows. The gate currently uses time-split
AUROC as the acceptance criterion and reports threshold confusion separately.

## Artifact Hashes

Passing run:

| Artifact | SHA256 |
|---|---|
| `association_validation_report.json` | `7fb0aad1c7f66bea4c86c5d6d99084f1c2203769a494c6f6d58ff0071d0bf2c3` |
| `benchmark_source_rows.jsonl` | `beaadecf0b1a3efea2f7468aa03a8516bb34c6fc5731c2537b1ea698260595ea` |
| `train_test_split.jsonl` | `17efa29009b75fecef6cfc72ffe0111694893e799f8c4ed9e2b0d8fc8b869770` |
| `scored_outputs.jsonl` | `0f305c7c6805af068fe715f617610dc600e285e347be016a0dca37e676a7c4b3` |
| `metrics.json` | `017b5ccd44a10d8bc0533ca787d1bc0ff4b6d273b3a626598ac76a7948554bc7` |
| `stdout.json` | `ac7e1f600f924d000f0f160fa694fd04ab50e3227e15b4ba636bf6697886c3f3` |
| `stderr.log` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

`readback_summary.json` and `sha256sums.txt` are in the FSV root.

## Conclusion

#1182 is satisfied:

- real source roots are read and hashed;
- benchmark source rows are persisted separately;
- train/test split rows are persisted separately;
- scored outputs are persisted separately;
- metrics include recall, precision, AUROC, and confidence intervals;
- the strict-threshold failure is preserved as calibration evidence;
- the accepted thresholded gate passes against real persisted evidence.

Downstream broad miners must require a passing `association-validation-gates`
artifact before accepting mined hypotheses. Passing this gate still only admits
association hypotheses for ranking and falsification; it does not create a
clinical claim.
