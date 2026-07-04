# #1257 nSIDES TwoSIDES/OffSIDES Source Mining

Status: complete for the nSIDES TwoSIDES/OffSIDES source-mining pass.

This slice continued #1256 after the PharmGKB no-hit result by snapshotting
nSIDES/Tatonetti documentation and the current TwoSIDES and OffSIDES flat-file
archives. TwoSIDES was treated as the only pair-level adverse-effect source in
this pass. OffSIDES was treated as single-drug adverse-effect context only, not
pair proof.

Clinical boundary:

```text
nSIDES TwoSIDES/OffSIDES source mining is adverse-effect/source triage only; pair adverse-effect rows and single-drug adverse-effect rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1257-nsides-source-mining-20260704T235500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/candidate_pharmgkb_status.jsonl
sha256: cc2b60f9d692b4dc74cf8c9e0742ae8d3e2f7cf7f707ca91d45e07bbc374ed0c

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/pharmgkb_pair_status.jsonl
sha256: 58dc7d1c502f385649d41ff3bc469b0db0a373c12d991f2a9f744cecfdfe0d04

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/persisted_readback.json
sha256: 6bb5b355ded3f1adf05b10e3186b1d1186612abe6d3556d1294b18850ad25cdf

/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z/out/calyx_bridge_corpus_readback.json
sha256: f70dae3dca15954d556a9d2ed1ecc7b2f54b0c77c01048fe9e586e7a630dd67d
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `nsides_home.html` | `https://nsides.io/` | 23,263 | `ecef6a0b1360726bc1a877715166388c8a849f1a6fea71194a84cb793db7e2a4` |
| `tatonetti_stm.html` | `https://tatonettilab.org/resources/tatonetti-stm.html` | 3,115 | `1cf7afa8da126f311fe1e24c289a87f5cbe6cfa1a962a838e928615cc679af52` |

Persisted archives:

| Archive | Bytes | SHA-256 |
|---|---:|---|
| `twosides.csv.gz` | 738,463,578 | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `offsides.csv.gz` | 68,762,346 | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Parsed source tables:

| Table | Rows | Header SHA-256 | Archive SHA-256 |
|---|---:|---|---|
| `TWOSIDES` | 42,920,391 | `d4aaaba48caff9cc697d80d858f9b43f48c11ef7ef0095de80f801c693eb47d2` | `59e5654a2b4cee2ebad1d37ec7840405c11eed3746dab337d836f73e63aea700` |
| `OFFSIDES` | 3,206,558 | `8d1c9e60327fc016bfd81ce74c5f1619290c469f638ef1a43450f9b1d290cb9c` | `0b5d2bd93ed44b95c22d8f9f053acbef4f59280027ae54d48dfe40d4fb9d60b3` |

Source contract:

- Input scope was the 532 #1256 candidate rows still blocked after PharmGKB.
- Those candidates represented 353 unique pair keys.
- TwoSIDES pair evidence required one TwoSIDES row where both pair drugs matched
  the row's two drug concept names by normalized exact or salt-stripped name
  key, in either order.
- OffSIDES context required one OffSIDES row matching one pair-side drug name by
  the same normalized/salt-stripped key. These rows were marked context only and
  never pair proof.
- All rows remained blocked behind the clinical boundary.

Runtime note:

- The first full run exposed an avoidable repeated source-hash read during
  OffSIDES matches. The miner was patched to cache source archive hashes once
  per scan and to reuse already-downloaded archives before the successful run.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `nsides_source_rows.jsonl` | 6 | 6,273 | `a75100db75fad772a7cfc48c2cc4d04dd8a0606a692c9be9595e82c73ab08ae5` |
| `twosides_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `offsides_single_drug_context.jsonl` | 853 | 1,556,599 | `4d5a6991457b0088b04e3aa4cf22f3eac780fe3c627d7d19cf35f8aa765f12dc` |
| `nsides_pair_status.jsonl` | 353 | 549,534 | `16626c6da02115bb1cb6a9a71d1f094bdba9ca799c59b69bad7bc62e8715ca2a` |
| `candidate_nsides_status.jsonl` | 532 | 908,364 | `70631c484ca28d8a20d08550ca0d15fabaa2eebfa3ab766ef2b43d4d8ccc4282` |
| `nsides_bridge_rows.jsonl` | 1,000 | 1,368,122 | `5c9b651d424ffec1e89e3857b4b226febbed1bbb93486590257ac3fc9547dfa1` |
| `input_manifest.json` | - | 5,194 | `f1300abbd4908118cbb345b4acb7045c4c9ed203ca68ac806459c82cdf2e09ee` |
| `validation_metrics.json` | - | 1,232 | `5467438ae31ebe6810a10f1b21d6c1e9e2023c6deccb30d7580ffdc722745d09` |
| `output_manifest.json` | - | 2,601 | `644e1095034650d89686ccf58164ea7e9d6e2d2776be2bff268e063164a7bc5d` |
| `persisted_readback.json` | - | 3,453 | `b0cbb727d64e4e3bab246d64e16d565bdbffa7e7a99a06dcbad0a8f163964fe4` |
| `calyx_bridge_corpus_stdout.json` | - | 731 | `bbf8c4d001aa88a0ac065d489b730543aae97a07db15257dcea8a78c757762b6` |
| `calyx_bridge_corpus_readback.json` | - | 5,872 | `f92138819ab44501f9a1a00e9fc22c940551af31716562a04659ec54be231fb6` |

## Metrics

| Metric | Count |
|---|---:|
| #1256 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| TwoSIDES source rows parsed | 42,920,391 |
| OffSIDES source rows parsed | 3,206,558 |
| TwoSIDES pair evidence rows | 0 |
| OffSIDES single-drug context sample rows | 853 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `nsides_offsides_single_drug_context_without_pair_hit_still_blocked` | 157 |
| `nsides_no_source_name_mapping_still_blocked` | 196 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `nsides_candidate_offsides_single_drug_context_without_pair_hit_still_blocked` | 241 |
| `nsides_candidate_no_source_name_mapping_still_blocked` | 291 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1256 persisted readback all true | true |
| #1256 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All TwoSIDES hits have evidence | true |
| Evidence rows have both matches | true |
| Evidence rows have source hashes | true |
| OffSIDES context rows are single-drug only | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1257-nsides-source-mining-20260704t235500z
vault_id: 01KWQPR6GMSZ5DD4FFSTDCV1WM
vault_dir: /home/croyse/calyx/vaults/01KWQPR6GMSZ5DD4FFSTDCV1WM
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 606 |
| Graph nodes | 1,606 |
| Graph edges | 7,782 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1257 expanded external source coverage for the #1256 blocked remainder by
checking 42,920,391 TwoSIDES drug-drug-effect rows and 3,206,558 OffSIDES
single-drug adverse-effect rows. No candidate pair satisfied the TwoSIDES
pair-level source gate, so there were zero TwoSIDES pair evidence rows. 157
pair keys had OffSIDES single-drug adverse-effect context for at least one
pair-side drug, and those rows are retained only as safety blocker/review
context.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.
