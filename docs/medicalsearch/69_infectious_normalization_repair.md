# #1225 Infectious / Immunology Normalization Repair

## Scope

#1225 repairs high-value unresolved concept normalization rows from the #1188
infectious/immunology/inflammation domain slice. The repair is conservative:
exact, unambiguous biomedical terms are mapped to stable MeSH or NCBI Gene IDs,
while narrative or ambiguous phrases remain in the unresolved artifact.

This is normalization and association-coverage repair only. It does not assert
clinical actionability, treatment guidance, safety, efficacy, or cure evidence.

## Implementation

Script:

```text
scripts/medicalsearch/issue1225_infectious_normalization_repair.py
```

Final script commit:

```text
69f91cb9343234f4294e9cd5e8e63ec8b43f6eac
```

Local gates:

```text
python -m py_compile scripts/medicalsearch/issue1225_infectious_normalization_repair.py
git diff --check
```

Both passed.

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z
```

Summary:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z/issue1225_fsv_summary.json
```

Inputs:

| Input | Path |
|---|---|
| Source expansion rows | `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z/complete_cxid_source_expansion.jsonl` |
| #1188 normalized domain annotations | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_normalized_annotations.jsonl` |
| #1188 unresolved domain terms | `/home/croyse/calyx/fsv/issue1188-infectious-immunology-hunt-20260704T103019Z/out/infectious_immunology_unresolved_terms.jsonl` |

Raw lookup responses were persisted under:

```text
/home/croyse/calyx/fsv/issue1225-infectious-normalization-repair-20260704T111004Z/raw
```

## Before / After

| Metric | Before | After |
|---|---:|---:|
| Normalized annotation rows | 475 | 1,027 |
| Normalized source CxIds | 265 | 440 |
| Unique normalized concepts | 14 | 27 |
| Unresolved terms | 151 | 122 |
| Resolved unresolved terms | - | 29 |
| Added annotation rows | - | 586 |
| New co-mention association coverage rows | - | 30 |

Remaining unresolved reason counts:

| Reason | Rows |
|---|---:|
| `api_error` | 25 |
| `not_queried_bounded_api_budget` | 97 |

## Deterministic Mappings

| Normalized name | DB | ID | Lookup SHA-256 | Terms |
|---|---|---|---|---|
| Asthma | `ncbi_mesh` | `D001249` | `2a8f55540e7a51a1f738b5a116df840c5e4c5048d8b27a997cca457b302bc682` | asthma; bronchial asthma; acute asthma |
| Tuberculosis | `ncbi_mesh` | `D014376` | `b8b8c563725bb79c317abd6510ec48076fe57c530c03513f23dcb66d391c343c` | Tuberculosis; Myco tuberculosis |
| Sepsis | `ncbi_mesh` | `D018805` | `75c56a2fc0ee0e9408e9bf7741f830682eefe342eaccd721092f8064c6939e18` | Sepsis; Septicemia |
| Malaria | `ncbi_mesh` | `D008288` | `e6055a4606f1ce7f63faca66a449c602b051f66daaa535215fa310cc57748fa3` | Malaria |
| Influenza Vaccines | `ncbi_mesh` | `D007252` | `7ac523184b4fd571d67f5a354272c176b7a4713d6d8de7608c75901027265e28` | Influenza vaccine |
| Lupus Vulgaris | `ncbi_mesh` | `D008177` | `d05507453dcf7a4a414ca29c9b2b8bf895546d6253e560c7ccfd728658a7271e` | Lupus vulgaris |
| Lupus Erythematosus, Systemic | `ncbi_mesh` | `D008180` | `89d37327f96ebde8bbacb7c9fcf84b5fbe242d26eab47825254c3206cbaeac3f` | Systemic lupus erythematosus |
| Arthritis, Rheumatoid | `ncbi_mesh` | `D001172` | `2009efb73ca42e76596c4c86e7bb3a78e475ea77fbb4ae67f68d25d8076a0e96` | Rheumatoid arthritis |
| Yellow Fever | `ncbi_mesh` | `D015004` | `32997d3d54aa634205a1643710ae27a8adf2c483e9fa45e8b2604bb885b4f6cc` | yellow fever |
| Leukotrienes | `ncbi_mesh` | `D015289` | `cd4cfd7864bf271e7d0cf00a3dc407ce71ed4f151b5f00fc005aa3fe2c087400` | Leukotriene |
| Leukotriene Antagonists | `ncbi_mesh` | `D020024` | `e6a26662533eb81f52eefaccff89e79be41ed22a8c0b62128c96552179396bd7` | Leukotriene antagonist; Leukotriene antagonists; Leukotriene receptor antagonist |
| HLA-B | `ncbi_gene` | `3106` | `c67f01175dc94dda80fb6d67acf96145ac41881d26fe8a6efd507654bcbc13ca` | HLA-B27 |
| CD40 | `ncbi_gene` | `958` | `10274bc066be6102bbc04cc91d67bdf8c5a8ecfa0fd7f02c71bfc80a0a1dfd8f` | CD40 |
| CD40LG | `ncbi_gene` | `959` | `bff64d7f016f2f112e2adc15aafeeb4744f7d9a00e698b0c00170c54fbb9ab0a` | CD40L |
| IL2 | `ncbi_gene` | `3558` | `b995e1c0ea2a1495416f5a368bdea399270afd01e3f2a39aa302454b95c45a0c` | Interleukin-2 |

## Required Row-Level Examples

| Required term | Added rows | Normalized to | Example source CxId | Source SHA-256 |
|---|---:|---|---|---|
| HLA-B27 | 2 | HLA-B / `ncbi_gene:3106` | `9aa017515951a9c89b41f0e759e23b87` | `30db19bd56e3a270beb82a59f74b56ff4cbf56cce51d1ab0648f22ef97e200df` |
| bronchial asthma | 97 | Asthma / `ncbi_mesh:D001249` | `01585cc6743dc8f752d91b3aa7b2356e` | `b1abeaa5dfdcee173d64fa4d9ce3168a06ebcc04db164c544647bf86a3e36445` |
| acute asthma | 30 | Asthma / `ncbi_mesh:D001249` | `0b142e0d099343c4d86d1dccb22990c3` | `879ebaf24b7789e479fadcb86433cf995cb4ac9f1bf3b7f987e9263ad9f85099` |
| Tuberculosis | 39 | Tuberculosis / `ncbi_mesh:D014376` | `0db83cbc1729a537fa916de8627d4ff8` | `c4e9a5f2a79c67f1cd8695c503f116a43c34479e04262423c2cd84f874ffc1ae` |
| CD40 | 6 | CD40 / `ncbi_gene:958` | `188af6caf2f56d36b6b41c263316d248` | `b717c588a69788d6b28642d038d480adbf0a9e4dc2d65e006cd3866ad1a34a1b` |
| Sepsis | 20 | Sepsis / `ncbi_mesh:D018805` | `082120387ae55f348be72d71736dff4e` | `b5c2b88b966cb0ea67430f661ff8fad2894478d028fee420db9706114148149c` |
| Leukotriene | 49 | Leukotrienes / `ncbi_mesh:D015289` | `01585cc6743dc8f752d91b3aa7b2356e` | `b1abeaa5dfdcee173d64fa4d9ce3168a06ebcc04db164c544647bf86a3e36445` |
| Malaria | 20 | Malaria / `ncbi_mesh:D008288` | `1242654995b8889a1198b02f0b63bcd4` | `d04dae7a2f23564e21bce187b2a73b2460408cc53dd941f0cf38846bb03ef264` |
| Influenza vaccine | 2 | Influenza Vaccines / `ncbi_mesh:D007252` | `1b0c5eb9c22a9db049c73a0b2c0bcbd7` | `200f859603013ac2f84e023c7d7a84ad4407f921949e18a63c11df5ffa5dd615` |

## Artifact Readback

| Artifact | SHA-256 |
|---|---|
| `input_scope.json` | `02983f061ab757a6eb9d86ecee504f6660d9afab993aa0d31305cd9d91315755` |
| `deterministic_mapping_table.json` | `84c26bd756156ac9137f6ccb0ebfaaa10ebe3c020dce7f647ea20e7698f6c8ec` |
| `repaired_normalized_annotations_added.jsonl` | `8c99dbaa3b8683906691b83775c5905ba4f51f17a2d45c8bec62730184b9ffcf` |
| `infectious_immunology_normalized_annotations.repaired.jsonl` | `1d31e9959a282c5a7054f70c1164a7d9abc37b91add8a1ff4c3f8b2be12a4f41` |
| `infectious_immunology_unresolved_terms.remaining.jsonl` | `11b35cce7c0a2afec3616b6985346d26df3e96bdcee5c882eeaefabc4fd21a41` |
| `resolved_terms_from_unresolved.jsonl` | `317e72a36ec5725a7783deaa668aa7e4b29fbed9811af40349473c3dcbe970db` |
| `new_co_mention_association_coverage.jsonl` | `719aa0ce97a8f40d4b37dedad0f714bb1fbacd717aa10e2f606d71dca3a4a021` |
| `before_after_coverage.json` | `425cfff636982d51a0d2f7b8ae8e7a1ef3c3a95c8996f2f8cf2ea5de5e7cdcdd` |
| `required_term_examples.json` | `b28cbf7222be6dec6e6cc3dfc419a60ed7e903bf1e6f8b6b4ca03a5509f44354` |

Readback assertions:

| Assertion | Value |
|---|---:|
| Before unresolved terms | 151 |
| After unresolved terms | 122 |
| Unresolved decreased | true |
| Added rows read back | 586 |
| Added rows match metrics | true |
| Repaired rows read back | 1,027 |
| Required terms all present | true |
| New co-mention pair rows | 30 |

## Conclusion

#1225 is complete for the focused infectious/immunology normalization repair:

- 29 previously unresolved terms were deterministically resolved;
- all required terms from the issue body have row-level source-hash examples;
- unresolved accounting remains explicit for 122 ambiguous or still-unqueried rows;
- repaired annotations increased source coverage from 265 to 440 CxIds;
- 30 new same-source association coverage rows were persisted for downstream
  review.

No clinical recommendation, treatment claim, safety claim, efficacy claim,
actionability claim, or cure claim is made.
