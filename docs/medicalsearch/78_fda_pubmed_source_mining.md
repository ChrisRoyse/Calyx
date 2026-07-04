# #1234 FDA/PubMed Source Mining

## Scope

#1234 continues the external source-mining chain over the 1,342 #1232 rows
that still had no current ClinicalTrials.gov hit after DrugComb, NCI ALMANAC,
CDCDB, and the #1232 registry recheck.

This pass checks three current sources:

- FDA Orange Book downloadable data files;
- FDA National Drug Code Directory text download;
- PubMed E-utilities ESearch/ESummary title/abstract co-mention search.

The output is source-attributed research triage only. FDA product ingredient
co-occurrence and PubMed title/abstract co-mention are not efficacy proof,
safety proof, dosing guidance, treatment guidance, recommendation, clinical
actionability, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1234_fda_pubmed_source_mining.py
```

The script:

- reads the #1232 `clinicaltrials_pair_status.jsonl` rows;
- filters to the 1,342 rows still `no_external_hit`;
- parses current FDA Orange Book `products.txt` ingredient sets;
- parses current FDA NDC `product.txt` substance sets;
- builds deterministic source pair indexes;
- queries PubMed ESearch for 790 unique pair keys using title/abstract phrase
  terms, under the no-key E-utilities rate limit;
- batches PubMed ESummary reads for returned PMIDs;
- emits one exact/normalized/no-hit status row for every input row;
- writes a 1,000-row bridge-corpus slice for native Calyx materialization.

Accepted sources:

- FDA Orange Book Data Files: <https://www.fda.gov/drugs/drug-approvals-and-databases/orange-book-data-files>
- FDA Orange Book download: <https://www.fda.gov/media/76860/download?attachment>
- FDA NDC Directory: <https://www.fda.gov/drugs/drug-approvals-and-databases/national-drug-code-directory>
- FDA NDC text download: <https://www.accessdata.fda.gov/cder/ndctext.zip>
- NCBI E-utilities intro/rate policy: <https://www.ncbi.nlm.nih.gov/books/NBK25497/>
- NLM E-utilities guide: <https://www.nlm.nih.gov/dataguide/eutilities/utilities.html>

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z
```

Primary readbacks:

```text
/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/persisted_readback.json
sha256: a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16

/home/croyse/calyx/fsv/issue1234-fda-orangebook-current-20260704T160500Z/out/calyx_bridge_corpus_readback.json
sha256: 21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265
```

Native Calyx materialization:

```text
name: issue1234-fda-pubmed-source-mining-20260704t171500z
vault_id: 01KWPYD9HZWK974Z6Z8G839ZYG
vault_dir: /home/croyse/calyx/vaults/01KWPYD9HZWK974Z6Z8G839ZYG
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 1,000 |
| Bridge terms | 507 |
| Graph nodes | 1,507 |
| Graph edges | 10,580 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Inputs

| Input | Rows/bytes | SHA-256 |
|---|---:|---|
| #1232 ClinicalTrials.gov pair status | 1,546 rows | `ba87af2046b16a5857e61c6a8a8d14dbb8c29c7b01db8e9de1e073aacbf07bdc` |
| FDA Orange Book source page | 37,044 bytes | `944322f9bc0e153600a4f38057fde15881b955fd9e9a16522a2e13c4297dddc2` |
| FDA Orange Book zip | 1,087,144 bytes | `a9c4f73aef2770b655a4af077053511fb8b8f97259be658980bcff3b64ca9e31` |
| FDA NDC source page | 36,115 bytes | `d7b3f16c960d97cae95c002f36bdfab5f54724094b404f8559c1800b70088c09` |
| FDA NDC text zip | 10,712,849 bytes | `5c481b512cdd5d545f27eae736bab48d0209f4716f85d7de256bfc47fbb84de4` |
| NCBI E-utilities intro | 68,713 bytes | `204e634142073f071ade91b62c83e5034fe57c1ca1b1642a4d468aece020c65c` |
| NLM E-utilities guide | 72,237 bytes | `08c0e23ecdec38e4fe8c6cdc0d655e87186662573a6d98de93a22a950fa1e081` |

Source schema fingerprints:

| Source | Fingerprint |
|---|---|
| FDA Orange Book parsed schemas | `557bf0e59d9105c0e230c44811c71b81bef1d1135c727a0528a11ca9037a4c05` |
| FDA NDC parsed schemas | `923929bd837503e6d7be9e56675cd2c0f369edbec454fb164d2a29564207ca80` |

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `fda_orangebook_source_products.jsonl` | 5,755 | 4,409,934 | `7520e6d167a2491c8edc5da9bbb72d23e8662d56ce07e26aeac4f9263f7b5684` |
| `fda_orangebook_pair_index.jsonl` | 1,036 | 3,065,980 | `7236e0f526e5cc3d90d621801ae0007381bb065649f85a8c8bf3e7a72c887f56` |
| `fda_ndc_source_products.jsonl` | 24,119 | 23,861,429 | `08106a075c5f46415fe585efa748b0235f704444c823526ff6dc1138f0a8c5b3` |
| `fda_ndc_pair_index.jsonl` | 163,832 | 507,538,433 | `21c6048e20897325a685f7df29a2c9d0a2d31ef3a4f9ca90eb93bf5c20b654f4` |
| `pubmed_esearch_responses.jsonl` | 790 | 1,183,322 | `feb6390a6f23eabe273c64710ab9ce5b42c8e8481f101c440389cfabdf3a2339` |
| `pubmed_esummary_responses.jsonl` | 4 | 1,045,478 | `b7a871fd3b687d49b63d07e9050db23e1eb97e1b068722d2dcba5e173f5629e3` |
| `pubmed_pair_literature_evidence.jsonl` | 568 | 766,538 | `304400ca4bbc7faea42ac9f2dd708865e7e27a225cbde6e806301d4581b7f7ea` |
| `candidate_external_source_status.jsonl` | 1,342 | 2,017,898 | `8f61616f01f44695e7b0227a16ff8cd9bceee2c68ad6147b705142a69ce24b0e` |
| `candidate_external_source_hits.jsonl` | 301 | 473,300 | `b60932183462b012f284a26750b0412c6a179224849938d7bb05ee42d473e950` |
| `external_combo_bridge_rows.jsonl` | 1,000 | 1,491,061 | `ec0f457e185648d31bb29a6a58abb8e6337a65283a42fec46f303697ac8e3107` |
| `validation_metrics.json` | - | 9,996 | `70b9146d373ea4c7eda910da5d48f5c46f98c0a59c6e33c051aec75b0f58f39e` |
| `output_manifest.json` | - | 4,392 | `26c2938c8f45d070828d812b04decb59f2f2168ee537ca2fd83d9af13bf84cd1` |
| `persisted_readback.json` | - | 4,368 | `a344a962768ad6d9b4759945e5fa76c95c0ac0de8afeec58e6628fa9a57aba16` |
| `calyx_bridge_corpus_stdout.json` | - | 687 | `6687467d950190dee4128aa03270b52d67dce7d47c3733f6d7856c0546cd186d` |
| `calyx_bridge_corpus_readback.json` | - | 3,649 | `21562a1baf00309e8c95a8311cd59dec84ed3df5e9a949cd75e6b7e799cca265` |

## Metrics

| Metric | Count |
|---|---:|
| #1232 no-hit rows rechecked | 1,342 |
| FDA Orange Book candidate hits | 0 |
| FDA NDC candidate hits | 0 |
| PubMed unique pair queries | 790 |
| PubMed unique pair queries with hits | 141 |
| PubMed candidate rows with hits | 301 |
| PubMed evidence rows | 568 |
| PubMed unique PMIDs | 523 |
| Candidate rows with any #1234 hit | 301 |
| Remaining no-hit rows after #1234 | 1,041 |

Status counts:

| Status | Count |
|---|---:|
| `normalized_hit` | 301 |
| `no_external_hit` | 1,041 |

Source hit counts:

| Source | Candidate hits |
|---|---:|
| FDA Orange Book | 0 |
| FDA NDC Directory | 0 |
| PubMed | 301 |

## Readback Assertions

| Assertion | Value |
|---|---:|
| Status row for every input row | true |
| Deterministic exact/normalized/no-hit status for every row | true |
| PubMed ESearch response for every unique pair key | true |
| PubMed hit rows have PMID lists | true |
| PubMed evidence rows have PMIDs | true |
| All joined rows carry the clinical boundary | true |
| Bridge rows <= 1,000 | true |
| Bridge row count matches materializer stdout | true |
| Bridge SHA matches materializer stdout | true |
| Active vault index contains exactly one final name | true |
| Active vault id matches materializer stdout | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Findings

- Current FDA Orange Book and FDA NDC product ingredient co-occurrence did not
  match any of the 1,342 remaining #1232 no-hit candidate rows.
- PubMed ESearch/ESummary added 301 normalized literature co-mention hits,
  across 141 unique pair queries and 523 unique PMIDs.
- The remaining external no-hit set drops from 1,342 to 1,041.
- The PubMed rows are deliberately `normalized_hit`: they are query-level
  title/abstract co-mention evidence, not abstract-body extraction and not
  asserted combination efficacy, safety, or clinical outcome evidence.
- The bounded bridge corpus is materialized into native Calyx vault
  `01KWPYD9HZWK974Z6Z8G839ZYG`.

## Conclusion

#1234 is complete for the current FDA/PubMed source-mining slice: 1,342
remaining candidate rows were rechecked, 301 source-attributed literature
co-mention hits were found, and the bridge corpus was materialized into Calyx
with separate persisted readback.

No treatment claim, efficacy claim, safety claim, clinical-actionability claim,
recommendation, dosing guidance, or cure claim is made.
