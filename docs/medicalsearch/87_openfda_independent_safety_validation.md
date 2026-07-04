# #1248 openFDA Independent Safety-Source Validation

Status: complete.

This slice checked the #1246 Europe PMC safety/counter review rollups against
openFDA Human Drug Label as an independent regulatory-label source. It queried
openFDA label sections for each unique pair key and required both candidate
terms in the same label section before marking an independent source hit.

Clinical boundary:

```text
openFDA independent safety validation is regulatory-label text triage only; not safety clearance, contraindication guidance, treatment guidance, dosing guidance, recommendation, clinical actionability, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1248-openfda-independent-safety-validation-20260704T211500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z/out/candidate_europepmc_safety_counter_rollup.jsonl
sha256: b7fddaf84c0c9a09dcbb8a6e59e505a6fe225fe4dce5f0167cd165d4b7b06255

/home/croyse/calyx/fsv/issue1246-europepmc-safety-counter-review-20260704T203000Z/out/europepmc_safety_counter_evidence_review.jsonl
sha256: 2906d957d06b38ce9ef1f33dc2b42f9984adf9055a1295d9d598ce7fb4a65089
```

Source contract:

- Input rollups: 363.
- Unique pair keys queried: 202.
- Source: openFDA Human Drug Label API.
- Query mode: paired exact-label-section search across safety, interaction,
  pharmacology, and description sections.
- A hit required both pair terms in the same returned label section.
- A no-hit is not safety clearance; it only means the targeted openFDA query did
  not return a same-section pair match.

## Source Artifacts

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `raw/openfda_download_manifest.json` | 582,744 | `546c94d92983a0b0c5f7ebe10bddfa2159f23aef9324db22d1ee3074b39c26d1` |
| `raw/openfda_label_overview.html` | 126,680 | `e538ce31106786b2f1e6432bf815804abda79889b74ee061ea34cea2d11df0be` |
| `raw/openfda_label_howto.html` | 117,945 | `532743e382cccafd6bf567148d9034ee1e8960aaa2c8287172ebac5ddccc1400` |
| `raw/openfda_query_syntax.html` | 124,308 | `b4fb28bf0791b6ebe7ccf8b7acd7218d7644bba97c0b70146cba33caf0f66c84` |
| `raw/openfda_authentication.html` | 116,494 | `d46c961be22f7eeb4e09f5c209eb81fee8a0119d5a242ac6baca8aac76bb898a` |
| `raw/openfda_license.html` | 120,790 | `9e906a722f7c4116154441bac21df15606fd2fe4335fb1a7b415b1acaf16da97` |
| `raw/openfda_terms.html` | 128,668 | `1a9217ccc118017674dc72ebce4e811706f2d895a82352ab38ffc2165ded0019` |

Source URLs:

- https://api.fda.gov/download.json
- https://open.fda.gov/apis/drug/label/
- https://open.fda.gov/apis/drug/label/how-to-use-the-endpoint/
- https://open.fda.gov/apis/query-syntax/
- https://open.fda.gov/apis/authentication/
- https://open.fda.gov/license/
- https://open.fda.gov/terms/

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `openfda_independent_query_responses.jsonl` | 202 | 374,817 | `855ed0237185d46474e03ab5ddb81fb6e69b45ffe747b66290dbdb374af2b57c` |
| `openfda_independent_label_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `openfda_independent_pair_status.jsonl` | 202 | 507,497 | `2ff40c8720c24c68a77f028dd641715d2368b27b307e3652496ef0fcec71268c` |
| `candidate_openfda_independent_status.jsonl` | 363 | 510,549 | `075ba8265ad5160d8f5ae89c99e6c66cf61c8c4733cfdc3d2046e171ae3773a1` |
| `openfda_independent_bridge_rows.jsonl` | 363 | 567,487 | `ed16f2252d9190a1958036e65480a8ea16d316a9fe35787140cb979f7510f4f7` |
| `input_manifest.json` | - | 4,688 | `1ae1912e9854c8ff42b6b70640fa20d8af3be2720558bb583066170feac89815` |
| `validation_metrics.json` | - | 1,463 | `81502aaf32ee1b0ab5ca933d568e19c974dd3a05eef34e4ca895d9f142d8e337` |
| `output_manifest.json` | - | 2,503 | `941f8c80437f9c060c28495fafc181312d5ee8776282d1f9687c216aaf81390e` |
| `persisted_readback.json` | - | 3,662 | `5a88dcf76c2092345cd14ba83dd5a7f8547d8a121c3aa38650052362d0fcfcda` |
| `calyx_bridge_corpus_stdout.json` | - | 711 | `30ccc25b1b367371f48f946a92f6861d1181c4e5a35be71f5a877c7ddb13404c` |
| `calyx_bridge_corpus_readback.json` | - | 5,116 | `92ff7fb1f974e8d01c06511fb54264a2f479e2c592da3154e9905fdc0f27584c` |

## Metrics

| Metric | Count |
|---|---:|
| #1246 rollup rows checked | 363 |
| Unique pair keys queried | 202 |
| Queryable pair keys | 202 |
| openFDA query response rows | 202 |
| HTTP 404 no-result responses | 202 |
| openFDA label evidence rows | 0 |
| Pair statuses with independent openFDA no-hit | 202 |
| Rollup statuses with independent openFDA no-hit | 363 |

#1246 category counts carried through:

| #1246 category | Rollups |
|---|---:|
| `contraindication_or_avoidance_language_review` | 40 |
| `counter_negative_and_safety_language_review` | 82 |
| `counter_negative_language_review` | 23 |
| `fatality_or_mortality_language_review` | 125 |
| `safety_adverse_language_review` | 15 |
| `toxicity_language_review` | 78 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1246 persisted readback all true | true |
| #1246 Calyx readback all true | true |
| Query response for every queryable pair key | true |
| Pair status for every pair key | true |
| Rollup status for every #1246 rollup | true |
| All pair status values allowed | true |
| All rollup status values allowed | true |
| All hits have evidence | true |
| All status rows carry the clinical boundary | true |
| All rollup rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1248-openfda-independent-safety-validation-20260704t211500z
vault_id: 01KWQ9APFXM4Z4QWB7D626YGKT
vault_dir: /home/croyse/calyx/vaults/01KWQ9APFXM4Z4QWB7D626YGKT
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 363 |
| Bridge terms | 707 |
| Graph nodes | 1,070 |
| Graph edges | 3,630 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1248 produced an independent openFDA no-hit accounting layer for all 363 #1246
safety/counter rollups. No openFDA label section contained both terms for any
of the 202 queried pair keys under the targeted search contract.

This does not clear safety. It only records that this independent regulatory
label query found no same-section pair evidence. Every row remains blocked
pending broader source expansion, independent review, and human validation.

No safety clearance, contraindication guidance, treatment guidance, dosing
guidance, recommendation, clinical actionability, or cure claim is made.
