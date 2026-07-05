# #1259 RxNorm-Rescued TwoSIDES Safety Validation

Status: complete for the independent safety/falsification validation pass over
the seven #1258 RxCUI-rescued TwoSIDES pair-hit keys.

This slice read the sealed #1258 RxNorm canonicalization artifacts, verified
their hashes, grouped original pair keys by trusted RxCUI overlap, queried
independent safety/literature/label sources, and materialized the result into
native Calyx. The result is a blocker/review artifact only.

Clinical boundary:

```text
Independent validation of RxNorm-rescued TwoSIDES pair hits is safety, source, outcome, and falsification triage only; adverse-event rows, label co-mentions, literature co-mentions, and registry/source hits are blockers or review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1259-rxnorm-twosides-safety-validation-20260705T013000Z
```

Sealed #1258 inputs:

| Input | Rows | SHA-256 |
|---|---:|---|
| `rxnorm_twosides_pair_evidence.jsonl` | 757 | `7392eee05700b23d3b7ae49a923a4255525fa91b327b4a5967a912194fb49d17` |
| `rxnorm_pair_status.jsonl` | 353 | `20a57894bb796b843d49ddda07224a5a0545f2d28821f0ff0df8885eb4f07df0` |
| `candidate_rxnorm_status.jsonl` | 532 | `cb9ca1d1a1dd8af195e4dd9830e1072c4c1bfe7e2197ef77f9ec26f2baeddba2` |
| `rxnorm_term_status.jsonl` | 152 | `8907e7a9cb319a53a0c0fe7db78c9366209dd2aeb8a830b6ba02ce32917f7dd7` |
| `persisted_readback.json` | - | `0011f528f7af04e18154e83dc193c822a1171e9d753d3c67f5e4d53add04b55d` |
| `calyx_bridge_corpus_readback.json` | - | `521f420f3b8b5f821064c3a341be76dcdd5ef18cc11da8f3235b5ff893913a50` |
| `output_manifest.json` | - | `1c90ac1e495357da85a74a5c405423d59c914482aaae7036e3c6543216a0c48c` |

Independent source documentation snapshots:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `openfda_event_docs` | `https://open.fda.gov/apis/drug/event/` | 128,623 | `8a043cbfa4650d79191f05309b279687ada890e074432e6ea80574dcab0c61f8` |
| `openfda_event_fields` | `https://open.fda.gov/apis/drug/event/searchable-fields/` | 127,160 | `a318fe3288d61ecc00d5ac891de26622bf3e0966f876c6ebed36bad918920203` |
| `openfda_label_docs` | `https://open.fda.gov/apis/drug/label/` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `openfda_query_syntax` | `https://open.fda.gov/apis/query-syntax/` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `dailymed_web_services` | `https://dailymed.nlm.nih.gov/dailymed/app-support-web-services.cfm` | 85,397 | `3fbe63342062c085fcbb85f1431f6c1614bbacc6b2ade5adc21de8eba3c4327e` |
| `dailymed_spls_api` | `https://dailymed.nlm.nih.gov/dailymed/webservices-help/v2/spls_api.cfm` | 91,702 | `727d6a6a7345430e54100f230ee545081f23fcffb120a78e1c047ecfdba27add` |
| `europepmc_rest_docs` | `https://europepmc.org/RestfulWebService` | 64,486 | `e3b45ced2caf9a35496a33ffbe79e745f79a48ba61d2fbc6996cf94d6bce4c97` |
| `ncbi_eutilities_intro` | `https://www.ncbi.nlm.nih.gov/books/NBK25497/` | 68,713 | `ffd2e3b9ae0e5a472cd0179faf8ab4810f6c17707c828cbe22121595699606fa` |
| `ncbi_eutilities_params` | `https://www.ncbi.nlm.nih.gov/books/NBK25499/` | 114,258 | `feeb9656287baeaf06a1a16e4b938c31c9a894f27dbb58311b018a14b2cd9756` |

Source contract:

- Input scope was exactly the 7 #1258 pair keys with
  `rxnorm_twosides_rxcui_pair_hit_still_blocked`.
- Expected #1258 hashes were verified before processing.
- Trusted RxCUI overlap on both pair sides formed canonical duplicate groups,
  while original pair keys and strict identity keys were retained.
- Queried independent sources: openFDA FAERS, openFDA drug labels, DailyMed SPL
  metadata, Europe PMC, and PubMed E-utilities.
- A query hit did not become evidence unless returned source fields contained
  both pair terms under the deterministic presence gate.
- All evidence remains blocked behind safety, outcome, falsification, and human
  review gates.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `source_rows.jsonl` | 9 | 11,489 | `831719fa41863ab691eb49aea9c639bf313043b78b82d45bc409995b39ee5f96` |
| `pair_scope.jsonl` | 7 | 35,421 | `c2958b87c75e45ac14164d25498b5dc01529a498a6b353c30a29ace77cae6aac` |
| `independent_query_rows.jsonl` | 35 | 188,922 | `736cfba3178cc6ae557bef94f95294a898365b1267f07ea3390e793cb00f914c` |
| `independent_evidence_rows.jsonl` | 1 | 2,526 | `6e8643e558790549baba17cef531a6cfc3475c69654ce731ea2ccb519a45b0ca` |
| `pair_validation_rollups.jsonl` | 7 | 33,602 | `186227f7b52aa1433c22c1725a5de2d89e645af1198caa5134196225128b156c` |
| `candidate_validation_status.jsonl` | 16 | 22,476 | `b6a3eeaa15add12f23b57ae2956c6dfddfadb78ab45788e7a4f4c1e0de843700` |
| `issue1259_bridge_rows.jsonl` | 68 | 96,303 | `e3b7552159c041239cff5fff4cbea35195901be42bfa6ba27a7aecd4c7d339e4` |
| `validation_metrics.json` | - | 1,687 | `93622d3c0de17a2760e528b220c76cfde1531cbbe0532200d458ca57166eb16b` |
| `output_manifest.json` | - | 7,058 | `2f48d9d64a8a973f684816f72cb2ddcc15fc20284e2d272c2a4ced3f10112751` |
| `persisted_readback.json` | - | 3,935 | `2a421bfd797355c8b4c777240b88a7c1d02c376005a53d50e86e85025dde92ad` |
| `calyx_bridge_corpus_stdout.json` | - | 793 | `8fd1b2e5e054536cf7370c91cd7cb80755f29ae111ae49373e319793909848ac` |
| `calyx_bridge_corpus_stderr.txt` | - | 324 | `bb3e375644b6b6d3b3ecee83f186063af42ca7b90880eefc7c048bd1a2fc83ff` |
| `calyx_bridge_corpus_readback.json` | - | 3,248 | `10462d52057fa323443d1e2ae8c0fe752c464b2424a1e726eb3dc1b6a1680ce1` |

## Metrics

| Metric | Count |
|---|---:|
| Pair scope rows | 7 |
| Candidate status rows touched | 16 |
| TwoSIDES RxCUI adverse-effect rows preserved | 757 |
| Canonical strict identity groups | 7 |
| Trusted-RxCUI overlap duplicate groups | 1 |
| Independent query rows | 35 |
| Independent evidence rows | 1 |
| Pair rollups | 7 |
| Bridge rows | 68 |

Query rows by source:

| Source | Query rows |
|---|---:|
| openFDA FAERS | 7 |
| openFDA label | 7 |
| DailyMed SPL metadata | 7 |
| Europe PMC | 7 |
| PubMed ESearch | 7 |

HTTP status counts:

| Status | Query rows |
|---|---:|
| 200 | 15 |
| 404 | 13 |

The 404 rows are persisted no-result responses from openFDA. A transient PubMed
429 was observed in an earlier run; fetch retry handling was patched and the
final run had no 429 rows.

Pair rollups:

| Pair key | Status | Independent evidence | TwoSIDES rows | Max PRR |
|---|---|---:|---:|---:|
| `etanercept szzs||ribavirin monophosphate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 68 | 40.0 |
| `etanercept szzs||sitagliptin` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 212 | 30.0 |
| `etanercept||ribavirin monophosphate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 68 | 40.0 |
| `infliximab dyyb||sitagliptin` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 123 | 40.0 |
| `metformin||trametinib dimethyl sulfoxide` | `independent_faers_safety_signal_still_blocked` | 1 | 11 | 40.0 |
| `saxagliptin anhydrous||sitagliptin hydrochloride monohydrate` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 151 | 60.0 |
| `sitagliptin hydrochloride monohydrate||valacyclovir` | `twosides_only_no_independent_confirmation_still_blocked` | 0 | 124 | 40.0 |

Trusted-RxCUI overlap duplicate group:

```text
etanercept szzs||ribavirin monophosphate
etanercept||ribavirin monophosphate
```

Independent evidence row:

| Pair key | Source | Classification | Source ID | Reactions | Serious |
|---|---|---|---|---|---|
| `metformin||trametinib dimethyl sulfoxide` | openFDA FAERS | `safety_signal` / `faers_coreport_serious` | `24608768` | Off label use; Lower gastrointestinal haemorrhage | true |

Europe PMC returned hit counts for two pairs but no returned metadata record
passed the both-term evidence gate:

| Pair key | Hit count | Returned | Evidence rows |
|---|---:|---:|---:|
| `infliximab dyyb||sitagliptin` | 1 | 1 | 0 |
| `metformin||trametinib dimethyl sulfoxide` | 24 | 5 | 0 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `candidate_independent_faers_safety_signal_still_blocked` | 4 |
| `candidate_twosides_only_no_independent_confirmation_still_blocked` | 12 |

Validation assertions:

| Assertion | Result |
|---|---|
| Expected #1258 input hashes matched | true |
| Seven pair-scope rows | true |
| All pair-scope rows queryable | true |
| 757 TwoSIDES rows preserved | true |
| Pair rollup for every scoped pair | true |
| Query rows for every pair | true |
| Query rows have response hashes | true |
| Evidence rows have sources | true |
| Evidence rows blocked | true |
| Pair rollups blocked | true |
| Candidate statuses blocked | true |
| Clinical boundary present | true |
| Bridge rows <= 1,000 | true |
| Bridge terms present in text | true |

## Native Calyx Materialization

```text
name: issue1259-rxnorm-twosides-safety-validation-20260705t013000z-final
vault_id: 01KWQSYVD5WWDGTC0QFDR9651N
vault_dir: /home/croyse/calyx/vaults/01KWQSYVD5WWDGTC0QFDR9651N
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 68 |
| Bridge terms | 56 |
| Graph nodes | 124 |
| Graph edges | 616 |
| CSR persisted | true |
| Materializer index contains final name | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault `CURRENT` and `MANIFEST` files present | true |
| Vault `cf/graph` files present | true |
| Vault `cf/graph` file count | 743 |
| Vault `cf/graph` total bytes | 718,864 |

## Result

#1259 did not produce any treatment, efficacy, safety-clearance, dosing,
recommendation, clinical-actionability, pair-interaction-proof, or cure claim.
It did produce one independent safety blocker: a serious openFDA FAERS co-report
for `metformin||trametinib dimethyl sulfoxide` with reactions `Off label use`
and `Lower gastrointestinal haemorrhage`.

The remaining six pair rollups have TwoSIDES RxCUI adverse-effect source rows
but no independent verified evidence in this pass. All seven pair rollups and
all sixteen candidate statuses remain blocked.
