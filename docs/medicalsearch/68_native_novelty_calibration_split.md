# #1227 Native Novelty / Calibration Splitter Stage

## Scope

#1227 turns the bounded #1226 split artifact into a native Calyx CLI stage:

```text
calyx novelty-calibration-split --atlas <issue>|<domain>|<jsonl> ... --out-dir <dir> [--top-k <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
```

The stage preserves every input atlas row, routes explicit known-positive /
calibration rows into a proof view, and emits a novelty-prioritized research
lead view for rows without those calibration markers.

This is still research triage only. It does not assert clinical novelty,
efficacy, safety, actionability, treatment guidance, or cure evidence.

## Implementation

Commit:

```text
0c44f4f3a4ca31af93ed23b2bfa557f6f59a9407
```

Main code paths:

- `crates/calyx-cli/src/cmd/novelty_split/mod.rs`
- `crates/calyx-cli/src/cmd/novelty_split/scoring.rs`
- `crates/calyx-cli/src/cmd/novelty_split/persist.rs`
- `crates/calyx-cli/src/cmd/novelty_split/tests.rs`

The command runs the shared discovery-run manifest preflight before it writes
split artifacts. A stale manifest therefore fails before `combined_original_ranked.jsonl`
or any downstream split view is written.

## Local Gates

```text
cargo test -p calyx-cli novelty_split -- --nocapture
cargo check -p calyx-cli
cargo fmt --check
bash scripts/linecount.sh
git diff --check
```

Local results:

- splitter tests: 3 passed;
- stale manifest unit test returned `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`
  and proved no combined output was written;
- `cargo check -p calyx-cli`: passed;
- `cargo fmt --check`: passed;
- line-count gate: all `.rs` files <= 500 lines;
- `git diff --check`: passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z
```

Real inputs:

| Source issue | Domain | Source artifact |
|---|---|---|
| #1185 | oncology | `/home/croyse/calyx/fsv/issue1185-oncology-deep-hunt-20260704T024819Z/out/oncology_hypothesis_atlas.jsonl` |
| #1186 | metabolic/cardiovascular | `/home/croyse/calyx/fsv/issue1186-metabolic-cardiovascular-hunt-20260704T030654Z/out/metabolic_cardiovascular_hypotheses.jsonl` |
| #1187 | neuro | `/home/croyse/calyx/fsv/issue1187-neuro-hunt-20260704T101459Z/out/neuro_hypotheses.jsonl` |
| #1188 | infectious/immunology | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_hypotheses.jsonl` |

Manifest preflight:

| Field | Value |
|---|---|
| Stage id | `novelty-calibration-split` |
| Expected input SHA-256 | `0d7c186d98f96eeaa3d575394c64412979e2ba9f91f2dc0d62a60ae18de2a88f` |
| Observed input SHA-256 | `0d7c186d98f96eeaa3d575394c64412979e2ba9f91f2dc0d62a60ae18de2a88f` |
| Match | true |
| Manifest | `/home/croyse/calyx/fsv/issue1227-native-novelty-split-20260704T105920Z/manifest.json` |
| Manifest SHA-256 | `e1dc6a3c08752d4f2cf2f95b3c6c46c84bd939b97c9f8b4b0304be90458f9421` |

## Output Readback

| Artifact | SHA-256 |
|---|---|
| `native_stdout.json` | `9b882686bebab18eb6db0c67a511967420d975b08d7debfc0a308469b7dfd22a` |
| `out/persisted_readback.json` | `1a31874ca49d7a24446dd90c334cc4f8563b9ca003d390b3427de92d85917bc1` |
| `out/validation_metrics.json` | `f18fb88d82ce0d98e7b89668986850c622662fb36f8cf986e7bcf74c87a2204c` |
| `out/combined_original_ranked.jsonl` | `b283207096c22bd710b5002d15f49ed00662c47300fa9f7ab3f5949136462497` |
| `out/calibration_known_positive_rows.jsonl` | `44be636712d297fdd775b750615860255523ee25076e191c1e02802d73da0741` |
| `out/novelty_prioritized_research_leads.jsonl` | `731dea0569f99fc7afa3760a663c885a94c1d89bd68f384c0fb02ea3ffdb3815` |

Readback assertions:

| Assertion | Value |
|---|---:|
| Combined rows read back | 340 |
| Calibration rows read back | 169 |
| Novelty rows read back | 171 |
| Split rows sum to total | true |
| Total rows match | true |
| Top after row is not calibration | true |
| Calibration rows available | true |
| Novelty rows available | true |

## Parity With #1226

| View | #1226 rows | Native #1227 rows | Parity |
|---|---:|---:|---|
| Combined original ranked | 340 | 340 | true |
| Calibration / known-positive | 169 | 169 | true |
| Novelty-prioritized | 171 | 171 | true |

Top-row parity:

| View | Candidate |
|---|---|
| Top original | `oncology-civic:11176` |
| Top calibration | `oncology-civic:11176` |
| Top novelty | `typed-assoc:concept:ncbi_gene:24835::concept:ncbi_mesh:D011507` |

The native top novelty candidate matched the #1226 reference top novelty
candidate.

## Fail-Closed Proof

The FSV run also executed the same command with a deliberately stale manifest:

| Check | Value |
|---|---|
| Exit status | 2 |
| Failed | true |
| Error code | `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN` |
| `stale_out/combined_original_ranked.jsonl` written | false |

This proves the native stage checks sealed input identity before writing the
split artifacts.

## Conclusion

#1227 is complete for the native stage slice:

- the CLI command is implemented and discoverable in usage;
- the command consumes one or more sealed atlas JSONL inputs;
- persisted outputs include input scope, combined original ranking,
  calibration rows, novelty-prioritized rows, before/after top-k,
  spot checks, metrics, output manifest, and persisted readback;
- real aiwonder FSV matched the #1226 row counts and top novelty lead;
- stale input manifests fail closed before output.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.
