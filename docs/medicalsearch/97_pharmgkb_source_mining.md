# #1256 PharmGKB Source Mining

Status: complete for the PharmGKB/ClinPGx source-mining pass.

This slice continued #1255 after the DrugCentral no-hit result by snapshotting
PharmGKB/ClinPGx source documentation and downloadable TSV archives, then
checking pharmacogenomic annotations, labels, variant annotations, chemical
aliases, drug aliases, and entity relationships for same-row two-term support.
A hit required one source row to contain both pair terms or mapped PharmGKB ids.
Single-term mappings, source-table presence, annotation existence, and label
existence were not treated as pair evidence.

Clinical boundary:

```text
PharmGKB source mining is pharmacogenomic/source triage only; clinical annotation, label, variant, or relationship rows are blockers/review inputs, not safety clearance, efficacy, treatment guidance, dosing guidance, recommendation, clinical actionability, pair-interaction proof, or cure evidence.
```

## Inputs

FSV root:

```text
/home/croyse/calyx/fsv/issue1256-pharmgkb-source-mining-20260704T230500Z
```

Sealed upstream inputs:

```text
/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/candidate_drugcentral_status.jsonl
sha256: 9dce3e78548d959b56b08f3a74f8e69ec936673f0d5b78924c387315c6eca099

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/drugcentral_pair_status.jsonl
sha256: 07caa8201d7a9367864a52af4bae57afd9822e3e52aa69721e89f250cfe2ed81

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/persisted_readback.json
sha256: 661ef64c2daf25e5eb7483eeeb4754aa93fc2f83e223a7b8d114c14ed5fa9478

/home/croyse/calyx/fsv/issue1255-drugcentral-source-mining-20260704T222500Z/out/calyx_bridge_corpus_readback.json
sha256: c2c421392bb43469bbf7dc8f9be106c794b8c6697a39ceaf01c781ad225203bf
```

Persisted source documentation:

| Source doc | URL | Bytes | SHA-256 |
|---|---|---:|---|
| `clinpgx_api_root.html` | `https://api.pharmgkb.org/` | 4,172 | `92993534b52e3d12d02c2b6e66e580749fb862a2ed777ae95915f5f45de0ce64` |
| `clinpgx_downloads.html` | `https://www.clinpgx.org/downloads` | 2,505 | `ff5fe31c3191e57b278d53ede2f4a551102b8a5c5f017162e0f44987740aaddd` |
| `kg_registry_pharmgkb.html` | `https://kghub.org/kg-registry/resource/pharmgkb/pharmgkb.html` | 292,411 | `80cd262108973ee7533112935e9d524b906c75d8f30b1f574d4ff6a16266c408` |

Persisted PharmGKB archives:

| Archive | Bytes | SHA-256 |
|---|---:|---|
| `chemicals.zip` | 810,667 | `17ddecfbbf7be9ea44ecebb17632514ece6e0877f48d9100390e2c61b8007b80` |
| `clinicalAnnotations.zip` | 1,231,768 | `9c6512c54f3c9321effacb11178fb2ae1c45fa3f1710f08c3c365ee7537ced07` |
| `clinicalVariants.zip` | 74,345 | `68b6592a9039e0a6f0bbc0e15feffb691e66f008036aabb306ed734e4a0b7bda` |
| `drugLabels.zip` | 58,722 | `1be944921a8bbca9c1f273717322f5a8ebd5acb00dcc04cecda653592ffabd5b` |
| `drugs.zip` | 677,109 | `54939a6b4526845238d8e2139a1b59ad3382e64932dff21a1fd27c023f43b072` |
| `relationships.zip` | 2,375,103 | `1d4672930e8ef4c420ef840ca330517580886d8a8bfc56b0eef5a02a918c62be` |
| `variantAnnotations.zip` | 4,240,496 | `bfc1df607f95bfc08dd8e5af1ee78a231a08c0bf6061437ed53cf821c2f538ad` |

Source table snapshots:

| Table | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `chemicals.tsv` | 5,283 | 2,682,260 | `0518a95c92b8f9a5aa0d406492fa9e0324b2704a04c67d7f80ae2fd902f9cd22` |
| `clinicalVariants.tsv` | 5,190 | 362,165 | `5c98338a4c23c52590537776c9630650db7cf844c39b94ed5bc23300ea187414` |
| `clinical_ann_evidence.tsv` | 15,431 | 4,013,649 | `5fc2157933e78a02d0604b899e7db997e6f9d9f924c23f31dace59489803d118` |
| `clinical_annotations.tsv` | 5,186 | 852,045 | `f548f25c7b5930ca250e7640f5a3531cfcc6e89048b916a7f115ae390fcc66f4` |
| `drugLabels.tsv` | 1,401 | 206,852 | `9de09e002a31befd3bd342dc6af7660fc307aad318c5ccb6371a25693038746e` |
| `drugLabels.byGene.tsv` | 237 | 154,060 | `c90037d1468ceb9c7e47080f6e0464dc80cdc049010683a0b94d536a3c38cea4` |
| `drugs.tsv` | 3,756 | 2,222,607 | `6e7a8740fbad58347fabc0f4f5036221b0856f07fc805348f0019ffd5c2baba0` |
| `relationships.tsv` | 127,768 | 15,461,292 | `4bba8db8b80e2acdbfeddf5eba922052b0c7e5da968a1cf1cc4c9879598f74a1` |
| `var_drug_ann.tsv` | 12,963 | 7,093,584 | `742c08ac6d201e65e1aeed47193503bd7a48a4dcb4b32f683649b5a47cee6613` |
| `var_fa_ann.tsv` | 2,149 | 1,100,855 | `a674673b770239d46012eddc91a88c60f4f05cb4e91539fccb0c68c9fb06b8ed` |
| `var_pheno_ann.tsv` | 14,468 | 8,747,853 | `1345f02e1a12bf937a57da724a34332c6fbf200ac3b5eeb24bc8254aff2da43e` |

Source contract:

- Input scope was the 532 #1255 candidate rows still blocked after
  DrugCentral.
- Those candidates represented 353 unique pair keys.
- Alias mapping came from PharmGKB `drugs.tsv` and `chemicals.tsv` names,
  generic names, trade names, brand mixtures, cross-references, RxNorm ids,
  PubChem ids, and ATC ids.
- Same-row source evidence required both pair terms or their mapped PharmGKB ids
  to appear in one scanned TSV row.
- All rows remained blocked behind the clinical boundary.

Runtime notes:

- The live `https://api.pharmgkb.org/` root, ClinPGx downloads page, KG-Registry
  page, and concrete versioned data downloads were persisted.
- The stale `/v1/swagger-ui/index.html` API documentation path returned 404 at
  preflight time and was not included in the FSV contract.
- The TSV parser raised Python's CSV field-size limit because PharmGKB source
  rows exceed the default parser cap.
- The row scan was optimized to build normalized pair-term and PharmGKB-id
  membership once per source row, then produce detailed match objects only for
  row/pair candidates that satisfied both sides of the gate.

## Output Artifacts

| Artifact | Rows | Bytes | SHA-256 |
|---|---:|---:|---|
| `pharmgkb_source_rows.jsonl` | 21 | 17,580 | `e5ea879e444c5172edcf4a9b8d16368835aabdaba8a703e76b2958954f609626` |
| `pharmgkb_pair_evidence.jsonl` | 0 | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `pharmgkb_pair_status.jsonl` | 353 | 510,805 | `58dc7d1c502f385649d41ff3bc469b0db0a373c12d991f2a9f744cecfdfe0d04` |
| `candidate_pharmgkb_status.jsonl` | 532 | 854,883 | `cc2b60f9d692b4dc74cf8c9e0742ae8d3e2f7cf7f707ca91d45e07bbc374ed0c` |
| `pharmgkb_bridge_rows.jsonl` | 906 | 1,299,927 | `da3d922c645989f5b9b639f945c4b9fdc652d1652e5e41ac87de9e2b65d79e99` |
| `input_manifest.json` | - | 9,114 | `e753058c8a3de94d8025532327fb89e9a5fa796b44ee45e985443cedcaa047b8` |
| `validation_metrics.json` | - | 1,305 | `19bf531047eae9a0e2e6210af4dcdcf363bc35cf8ef621085ad3fa59aff7d88f` |
| `output_manifest.json` | - | 2,316 | `8e5957fc9820728a996909d4aeb773fea82a2e320f5d8fb40099b11d35d43ed1` |
| `persisted_readback.json` | - | 3,474 | `6bb5b355ded3f1adf05b10e3186b1d1186612abe6d3556d1294b18850ad25cdf` |
| `calyx_bridge_corpus_stdout.json` | - | 713 | `8aabfe2bc0d577b8dc3ca7eea8e5db338e5a1ca1b146f817e9365e13cfccb805` |
| `calyx_bridge_corpus_readback.json` | - | 5,812 | `f70dae3dca15954d556a9d2ed1ecc7b2f54b0c77c01048fe9e586e7a630dd67d` |

## Metrics

| Metric | Count |
|---|---:|
| #1255 blocked candidate rows checked | 532 |
| Unique pair keys checked | 353 |
| PharmGKB/ClinPGx source inventory rows | 21 |
| PharmGKB TSV tables scanned | 9 |
| Alias TSV tables scanned | 2 |
| PharmGKB pair evidence rows | 0 |
| Pair status rows | 353 |
| Candidate status rows | 532 |

Scanned TSV row counts:

| Table | Rows |
|---|---:|
| `clinical_annotations` | 5,186 |
| `clinical_ann_evidence` | 15,431 |
| `clinicalVariants` | 5,190 |
| `drugLabels` | 1,401 |
| `drugLabels_byGene` | 237 |
| `relationships` | 127,768 |
| `var_drug_ann` | 12,963 |
| `var_pheno_ann` | 14,468 |
| `var_fa_ann` | 2,149 |

Alias TSV row counts:

| Table | Rows |
|---|---:|
| `drugs` | 3,756 |
| `chemicals` | 5,283 |

Pair status counts:

| Status | Pair rows |
|---|---:|
| `pharmgkb_single_term_mappings_without_pair_match_still_blocked` | 161 |
| `pharmgkb_no_term_mapping_still_blocked` | 192 |

Candidate status counts:

| Status | Candidate rows |
|---|---:|
| `pharmgkb_candidate_single_term_mappings_without_pair_match_still_blocked` | 258 |
| `pharmgkb_candidate_no_term_mapping_still_blocked` | 274 |

Validation assertions:

| Assertion | Result |
|---|---|
| #1255 persisted readback all true | true |
| #1255 Calyx readback all true | true |
| Source rows present | true |
| Pair status for every pair key | true |
| Candidate status for every candidate | true |
| All PharmGKB hits have evidence rows | true |
| Evidence rows have both matches | true |
| Evidence rows have source hashes | true |
| Pair/candidate status values allowed | true |
| Rows carry the clinical boundary | true |
| Rows remain blocked | true |
| Bridge rows <= 1,000 | true |

## Native Calyx Materialization

```text
name: issue1256-pharmgkb-source-mining-20260704t230500z
vault_id: 01KWQMCNSKQFEM30T5CKV72CNV
vault_dir: /home/croyse/calyx/vaults/01KWQMCNSKQFEM30T5CKV72CNV
```

Materialization readback:

| Item | Count / value |
|---|---:|
| Bridge rows | 906 |
| Bridge terms | 550 |
| Graph nodes | 1,456 |
| Graph edges | 7,248 |
| CSR persisted | true |
| Active vault index contains final name exactly once | true |
| Graph nodes match readback | true |
| Graph edges match readback | true |
| Vault manifest/CURRENT files present | true |
| Graph files present | true |

## Result

#1256 expanded external source coverage for the #1255 blocked remainder by
checking PharmGKB/ClinPGx drug, chemical, relationship, label, clinical
annotation, clinical variant, variant-drug annotation, variant-phenotype
annotation, and variant-functional annotation sources. No candidate pair
satisfied the same-row two-term or mapped-id source gate, so there were zero
PharmGKB pair evidence rows. The 532 carried candidate rows remain blocked and
now have explicit PharmGKB no-term-mapping or single-term-only status rows.

No efficacy, safety clearance, treatment guidance, dosing guidance, clinical
recommendation, clinical actionability, pair-interaction proof, or cure claim is
made.
