# #1179 LINCS/CMap Reversal Screen

Status: complete for the bounded #1179 screen. This is not a treatment, cure,
clinical recommendation, or actionability claim. Transcriptomic reversal is a
lead-generation signal only.

## Sources

- CREEDS disease signatures: `https://maayanlab.cloud/CREEDS/`
- L1000CDS2 reverse perturbation search: `https://maayanlab.cloud/L1000CDS2/`
- FSV root:
  `/home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z`
- Source manifest readback:
  `persisted_readback.json`, 74 files, 48,002,018 bytes,
  aggregate sha256
  `a0dd0b367c1058c446153a9d92b3ce096d59ef0ac1a0433b30a6d07498a40f1c`.

The screen selected 30 real CREEDS disease signatures and submitted 30
reverse-mode L1000CDS2 gene-set queries. All 30 returned HTTP 200 responses.

## Calyx DB Materialization

Accepted collection: `biomed_lincs_cmap_reversal_v4`

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --collection biomed_lincs_cmap_reversal_v4 \
  --report /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z/calyx_db_readback_v4.json \
  --home /home/croyse/calyx
```

Readback report:

- `calyx_db_readback_v4.json`
- bytes: 21,467
- sha256: `9d56ebcf42edb543bb69b8a6b0ec98e932b0a38e59415de88eecb266f444b7e0`
- stdout JSON parsed equal to report JSON: true
- exit file: `materialize_lincs_v4.exit` contained `0`

Physical Calyx readback:

- nodes written: 6,551
- edges written: 13,532
- physical node keys: 6,551
- physical outgoing edge keys: 13,532
- persisted CSR nodes: 6,551
- persisted CSR edges: 13,532
- assoc graph nodes: 6,551
- assoc graph edges: 13,532
- CSR bytes: 1,300,710
- CSR sha256: `0800c6f8cb241d2e00214076d5cb53131fcddf8d07948ff19be4dea8a46e16ed`
- CSR blake3:
  `ac5871bfc4557d80107a03e8b8cb49620a7fd0a678ee4592f6579c39e8bfaca1`
- all node values were physically read back
- all edge keys were physically counted by collection-local range
- 512 deterministic edge values were physically read back

The first attempts are not accepted state:

- `biomed_lincs_cmap_reversal_v1`: killed after pathological full edge-value
  point-readback, exit `143`.
- `biomed_lincs_cmap_reversal_v2` and `v3`: timed out, exit `124`.
- Cleanup/atomic replacement is tracked by #1197.
- A batch edge-value readback API is tracked by #1198.

## Parsed Evidence

Persisted parsed rows:

- disease signature inputs: 30
- L1000CDS2 request records: 30
- LINCS reversal score rows: 1,500
- unsupported/unmapped current-candidate rows: 390

Graph node types include disease signatures, CREEDS source rows, L1000CDS2
queries, reversal score rows, perturbations, LINCS signatures, cell lines,
chemical identifiers, current candidate drugs, unsupported cases, source
artifacts, hashes, GEO series, UMLS concepts, and disease ontology IDs.

Important edge families include:

- `has_lincs_reversal_score`: 1,500
- `scores_perturbation`: 1,500
- `returned_reversal_score`: 1,500
- `measured_in_cell_line`: 1,500
- `has_lincs_signature`: 1,500
- `absent_from_l1000cds2_top50_reverse_results`: 390
- `unsupported_candidate`: 390

## Current Candidate Result

The current Calyx candidate-drug set did not appear in the top-50 reverse
L1000CDS2 results for the 30 selected disease signatures. This is not evidence
that the drugs do not work clinically; it is a bounded negative lead-signal
result for this data source, query mode, and selected signature set.

The unsupported rows cover:

- metformin
- linagliptin
- sitagliptin
- saxagliptin
- alogliptin
- vildagliptin
- adalimumab
- certolizumab
- etanercept
- golimumab
- infliximab
- ibalizumab
- rituximab

## Repeated Reversal Leads

The most repeated perturbation labels in the 1,500 top-50 rows were:

| Perturbation | Pert ID | Rows | Min rank | Max score | Boundary |
|---|---:|---:|---:|---:|---|
| `-666` | multiple BRD IDs | 252 | 1 | 0.1107 | placeholder label; corrected/resolved by #1199 |
| CGP-60474 | `BRD-K79090631` | 76 | 1 | 0.0705 | lead signal only |
| vorinostat | `BRD-K81418486` | 60 | 1 | 0.0936 | lead signal only |
| trichostatin A | `BRD-A19037878` | 44 | 3 | 0.0749 | lead signal only |
| geldanamycin | `BRD-A19500257` | 43 | 3 | 0.0610 | lead signal only |
| mitoxantrone | `BRD-K21680192` | 28 | 1 | 0.0571 | lead signal only |
| PD-0325901 | `BRD-K49865102` | 26 | 5 | 0.0605 | lead signal only |
| alvocidib | `BRD-K87909389` | 25 | 2 | 0.0591 | lead signal only |
| Narciclasine | `BRD-K06792661` | 21 | 2 | 0.1033 | lead signal only |

The `-666` row is a placeholder-label data-quality blocker, not a usable
therapeutic lead. #1199 resolved this by mapping the underlying BRD perturbation
IDs against authoritative LINCS/L1000FWD metadata and preserving unresolved rows.

## Conclusion

#1179 produced a grounded LINCS/CMap association substrate inside Calyx:
CREEDS disease signatures, L1000CDS2 query provenance, response hashes,
reversal-score rows, unsupported current-candidate rows, perturbation nodes,
and physical Graph CF/CSR readback are now in one collection.

This does not unlock a cure claim. It creates a verified association layer that
can feed the next tasks: perturbation metadata mapping (#1199), safety and
counter-evidence gates, broader signature coverage, clinical evidence joins,
and lead-ranking gates that explicitly reject unsupported clinical claims.
