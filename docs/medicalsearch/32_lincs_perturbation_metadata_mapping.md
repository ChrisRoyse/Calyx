# #1199 LINCS Perturbation Metadata Mapping

Status: complete for the bounded #1199 mapping slice. This is not a
treatment, cure, clinical recommendation, or actionability claim. LINCS/CMap
metadata makes perturbation labels interpretable; it does not prove efficacy or
safety.

## Sources

- L1000FWD downloads/API pages:
  `https://maayanlab.cloud/l1000fwd/download_page` and
  `https://maayanlab.cloud/l1000fwd/api_page`
- FSV root:
  `/home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z`
- Source manifest readback:
  `persisted_readback.json`, 9 files, 13,427,781 bytes, aggregate sha256
  `af874468aab777c8f9466f05770bb34c02fcc38f6b9d7fbd2542b0fc4ee598ce`.

Authoritative metadata files downloaded and hashed:

- `raw/Drugs_metadata.csv`: 7,797,325 bytes,
  sha256 `6447c511ac7d4111f2bd5e46cc95f0ca872d98d579d5e4ccaef6594ed7b899e3`
- `raw/CD_signature_metadata.csv`: 5,075,455 bytes,
  sha256 `b4495715d35f9bc17412c5ffa900d802812b25419ecd6291be6dd6fa4f38a557`
- `raw/download_page.html`: 26,703 bytes,
  sha256 `af82fe8d7a2f206aa77deafecfd2b8cf569dfa1dd2f50f53a950fda8c8f82763`
- `raw/api_page.html`: 57,972 bytes,
  sha256 `3c150dc4bc324294269650534ce90f7961d7cdfdee106164fb449bbe77f110b8`

Derived files:

- `parsed/perturbation_id_mappings.jsonl`: 530 rows,
  sha256 `dc2ed13f008bec632c4d0638685d35fcd78e7ba3f0b0e0ff5e0748d139c7580c`
- `parsed/placeholder_pert_desc_cases.jsonl`: 252 rows,
  sha256 `718a16debe90aab6dbb69769ccfa3ef24dd9bfa6478996a825e277d18f434098`
- `parsed/resolved_repeated_leads.jsonl`: 487 rows,
  sha256 `653ffd05a8fe9b8801d08e2018ec44012f05ca4863ae6b4de305b1f623ecb638`
- `mapping_summary.json`: sha256
  `1391ab3d5c7bd5c76bb3c6a8fb2cba308a4f4930951f9f9d9ed897c6c87ce4e0`

## Correction To #1179

The #1179 repeated-lead table grouped by `pert_desc`, so the placeholder label
`-666` was incorrectly easy to read as one perturbation. It is not one
compound. In the #1179 score rows, `-666` spans 252 rows and 148 unique
`pert_id` values.

Authoritative metadata resolves many of those IDs independently. Example:

- `BRD-K84595254` resolves to `strophanthidin`, PubChem CID `6185`,
  LSM ID `LSM-3891`.
- The matching placeholder score row is
  `CPC018_HT29_6H:BRD-K84595254:10.0`, disease label `Parkinson's disease`,
  rank `48`, score `0.0506`.

Therefore downstream ranking must use resolved perturbation IDs/names, not the
raw placeholder label.

## Mapping Results

Across the 1,500 #1179 LINCS/CMap reversal score rows:

- unique perturbation IDs: 530
- resolved to real perturbation names: 461
- structure/identifier rows without a resolved common name: 66
- unmapped perturbation IDs: 3

For the `-666` placeholder-label rows:

- placeholder rows: 252
- unique placeholder perturbation IDs: 148
- unique placeholder IDs resolved to names: 112
- row-level resolved-name cases: 189
- row-level structure/identifier-only cases: 63

Resolved placeholder examples include:

| Pert ID | Resolved name | PubChem CID | Example disease label | Example rank |
|---|---|---:|---|---:|
| `BRD-K84595254` | strophanthidin | 6185 | Parkinson's disease | 48 |
| `BRD-K57080016` | selumetinib | 10127622 | type 2 diabetes mellitus | 11 |
| `BRD-A36630025` | SN-38 | 4014291 | type 2 diabetes mellitus | 23 |
| `BRD-K64606589` | apicidin | NULL | type 2 diabetes mellitus | 29 |
| `BRD-K43389675` | daunorubicin | 30323 | type 2 diabetes mellitus | 34 |
| `BRD-K94441233` | mevastatin | 64715 | type 2 diabetes mellitus | 33 |

## Calyx DB Materialization

Accepted collection: `biomed_lincs_cmap_reversal_v7`

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --metadata-root /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z \
  --collection biomed_lincs_cmap_reversal_v7 \
  --report /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z/calyx_db_readback_v7.json \
  --home /home/croyse/calyx
```

Readback report:

- `calyx_db_readback_v7.json`
- bytes: 23,012
- sha256: `189777e8514c147a709d3f4b704a88752278a302d151209482b2058e26f56186`
- exit file: `materialize_v7.exit` contained `0`

Physical Calyx readback:

- nodes written: 8,993
- edges written: 19,150
- physical node keys: 8,993
- physical outgoing edge keys: 19,150
- persisted CSR nodes: 8,993
- persisted CSR edges: 19,150
- assoc graph nodes: 8,993
- assoc graph edges: 19,150
- CSR bytes: 1,821,310
- CSR sha256: `67933e9ec687cc5ede02e2cfeb4c1109f457f97f538eed1320187e6f19963e64`
- CSR blake3:
  `e791ae34d822a8fa77178c85fe49e969836ee3e61321a4adf9e9f0d8d5605667`
- all node values were physically read back by collection-local range
- all edge keys were physically counted by collection-local range
- 512 deterministic edge values were physically read back

The aborted v5/v6 collections are not accepted state. They were written before
the rebuilt executable used range-based node-value readback. Cleanup/atomic
replacement remains tracked by #1197.

## Graph Additions

The v7 collection extends #1179 with these metadata-specific nodes and edges:

- `perturbation_metadata_row`: 530 nodes
- `placeholder_pert_desc_case_row`: 252 nodes
- `resolved_repeated_lead_row`: 487 nodes
- `resolved_drug_name`: 416 nodes
- `chemical_identifier`: 508 nodes
- `has_perturbation_metadata`: 530 edges
- `resolved_to_drug_name`: 461 edges
- `has_pubchem_id`: 513 edges
- `resolved_placeholder_to_name`: 170 edges
- `placeholder_unresolved_status`: 57 edges
- `summarizes_perturbation`: 530 edges

Representative persisted path:

```text
placeholder_pert_desc_case:CPC006_HCC515_24H:BRD-K57080016:80.0:BRD-K57080016
  -> perturbation:BRD-K57080016
  -> resolved_drug_name:selumetinib
```

## Conclusion

#1199 makes the #1179 LINCS/CMap reversal screen rankable at the perturbation
identifier level. Placeholder-like labels are now preserved as evidence rows,
resolved where authoritative metadata allows, and blocked from being treated as
standalone drug leads.

This still does not support a cure claim. It produces a better association
substrate for the next gates: rank resolved perturbations, join safety and
known-target evidence, require disease-specific counterevidence, and reject any
clinical conclusion that lacks outcome-backed support.
