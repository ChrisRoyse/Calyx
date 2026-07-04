# #1180 Precision Oncology Validation Sources

## Scope

#1180 adds a precision-oncology validation/triage source layer for cancer
hypotheses. This slice ingests CIViC public monthly dumps, records source
license/API constraints for non-ingested oncology resources, parses cancer
evidence/assertion/gene/variant rows, maps rows against the current #1173 typed
overlay, and persists unresolved accounting.

This is validation and triage evidence only. CIViC evidence levels,
assertions, therapies, and clinical significance fields are not Calyx treatment
recommendations, cure claims, safety claims, or clinical-actionability claims.

The issue originally named `docs/medicalsearch/31_precision_oncology_validation.md`;
that number is already occupied by LINCS/CMap reversal work. This file keeps the
append-only numbering and cross-links #1180.

## Source Decisions

| Source | Decision | Constraint |
|---|---|---|
| CIViC | ingested | AWS Open Data registry lists monthly CIViC dumps with CC0 license |
| OncoKB | not ingested | API requires registration/license token; FAQ states OncoKB cannot be used to train AI/ML models |
| Sanger DepMap/GDSC | not ingested | data/API are public for non-commercial/internal research with commercial/API restrictions |

References used during source selection:

- CIViC API/docs: `https://civic.readthedocs.io/en/latest/api.html`
- CIViC AWS Open Data registry: `https://registry.opendata.aws/civic/`
- OncoKB licensing FAQ: `https://faq.oncokb.org/licensing`
- Sanger DepMap data usage policy: `https://depmap.sanger.ac.uk/documentation/data-usage-policy/`

## Real FSV

Final FSV root:

```text
/home/croyse/calyx/fsv/issue1180-precision-oncology-validation-20260704T024318Z
```

Typed overlay used for mapping:

```text
/home/croyse/calyx/fsv/issue1173-typed-biomedical-overlay-20260703T160109Z
```

The FSV script discovered CIViC dump keys through the public S3 XML listing API
and downloaded the latest available files for each selected category. `aws` was
not installed on aiwonder, so no AWS account or signed request was used.

Selected CIViC files:

| Category | Key | Bytes | SHA-256 |
|---|---|---:|---|
| ClinicalEvidenceSummaries | `ClinicalEvidenceSummaries/date=01-Jul-2026/ClinicalEvidenceSummaries.tsv` | 4,085,714 | `7277812e373f9489f3232f032378b94dc50474307e476c4d07bb891d8e31ce33` |
| AssertionSummaries | `AssertionSummaries/date=01-Jul-2026/AssertionSummaries.tsv` | 182,113 | `889cfc9d16e071d38ef4ca790b6c074f99466bd0763b17402eb348fd55713d77` |
| GeneSummaries | `GeneSummaries/date=01-Jan-2025/GeneSummaries.tsv` | 91,343 | `19f0b79a6d3dc68b663e38287c64e611505e1a1bb756c5b81172a8ab0a228d6f` |
| VariantSummaries | `VariantSummaries/date=01-Jul-2026/VariantSummaries.tsv` | 583,238 | `7d7973e6b0c6deaa78e3d906a3092b38c62c08a38a9fdca6c11a3941ad11ef50` |

Readback counts:

| Field | Count |
|---|---:|
| CIViC clinical evidence rows | 4,870 |
| Mapped rows against current typed overlay | 15 |
| Unresolved rows | 4,855 |
| Assertion rows | 143 |
| Gene rows | 591 |
| Variant rows | 1,984 |
| Distinct disease labels in evidence | 332 |
| Distinct gene labels in evidence | 550 |
| Distinct variant labels in evidence | 1,161 |

Top disease labels:

| Disease | Rows |
|---|---:|
| Von Hippel-Lindau Disease | 625 |
| Lung Non-small Cell Carcinoma | 452 |
| Colorectal Cancer | 351 |
| Chronic Myeloid Leukemia | 320 |
| Cancer | 236 |
| Acute Myeloid Leukemia | 220 |
| Breast Cancer | 199 |
| Melanoma | 139 |
| Lung Adenocarcinoma | 119 |
| Gastrointestinal Stromal Tumor | 89 |

Top genes:

| Gene | Rows |
|---|---:|
| VHL | 659 |
| EGFR | 243 |
| BRAF | 204 |
| TP53 | 192 |
| KRAS | 191 |
| PIK3CA | 157 |
| ERBB2 | 142 |
| KIT | 112 |
| FLT3 | 78 |
| PTEN | 60 |

Top therapies:

| Therapy | Rows |
|---|---:|
| Imatinib | 169 |
| Cetuximab | 158 |
| Dasatinib | 148 |
| Erlotinib | 126 |
| Vemurafenib | 124 |
| Crizotinib | 120 |
| Gefitinib | 91 |
| Trastuzumab | 80 |
| Trametinib | 78 |
| Imatinib Mesylate | 73 |

Evidence level counts:

| Level | Rows |
|---|---:|
| A | 225 |
| B | 1,625 |
| C | 1,671 |
| D | 1,317 |
| E | 32 |

Evidence type counts:

| Type | Rows |
|---|---:|
| Predictive | 2,849 |
| Predisposing | 681 |
| Prognostic | 536 |
| Diagnostic | 499 |
| Functional | 164 |
| Oncogenic | 141 |

## Persisted Artifacts

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `source_constraints.json` | 561 | `d24e0ea98697bed4da8b356fb41d717dfcb9cf4479f62cd0301735b248e33de5` |
| `civic_download_manifest.json` | 2,386 | `912a14facbcfdea6833f61be80148072829678d6402bb0e35d7b35b198e5698b` |
| `civic_tsv_headers.json` | 2,320 | `4e58f287990c4434d272f5bad1e006536c21d6d5f71b5c660906b80ad3cd31d5` |
| `run_summary.json` | 6,747 | `85baec997eda18c3ca81dcfa864e14b5cf5cdb8f650b141199a33a0a38e52e2e` |
| `persisted_readback.json` | 251 | `833436775e7ad60f1391534e3adf8e0fb22326da9e8a34e610cab52a9af18bf2` |
| `final_file_manifest.json` | 3,271 | `12564fecdf505d9ed22ba57bfa7af56e7809f6df6759e9fe62a2f9e0c43aafe2` |
| `parsed/civic_evidence_rows.jsonl` | 3,506,213 | `bb3b6955275f71cf51af65d3541249bb06f4a7137689c323877af3c2946d2261` |
| `parsed/civic_mapped_rows.jsonl` | 13,589 | `b9891be3230b6dfa18f0fc3fba5ba130c3bbde66448b890517a8097c0779927d` |
| `parsed/civic_unresolved_rows.jsonl` | 1,055,614 | `2e65ec06511c9138517512b77d887631e97ad513ade44cb8c3c7106991abe315` |
| `parsed/civic_assertion_rows.jsonl` | 59,155 | `b682159bf8ff544bbc5cedb74f95b20cea677f48af9783878f21d96ebf8c0639` |
| `parsed/civic_gene_rows.jsonl` | 47,164 | `5c870c8ca16b819197345be4a5df0b29fab3c1e8746a0b937e2317d3c5e6d63a` |
| `parsed/civic_variant_rows.jsonl` | 269,720 | `d1ece325ccaf537c6babc7c9bd96556f0dedee2f4cd6401b676700b95d96d0d7` |

## Mapped Examples

The current typed overlay is small relative to CIViC. Exact mapping therefore
mostly hits concepts already present in the #1173 overlay.

Examples:

| CIViC row | Mapped concept | Evidence |
|---|---|---|
| NF1 mutation / Skin Melanoma / Vemurafenib | `concept:ncbi_gene:4763` NF1 | Predictive resistance, level D/C rows |
| NF1 mutation / Plexiform Neurofibroma / Selumetinib | `concept:ncbi_gene:4763` NF1 | Predictive sensitivity/response, level A |
| Metformin combinations in cancer/breast cancer rows | `concept:chembl:CHEMBL1431`, `concept:mesh:D008687` Metformin | Predictive sensitivity/response, level D/B |
| PTTG1/LEPR expression / Meningioma | `concept:ncbi_mesh:D008579` Meningioma | Prognostic poor-outcome rows |

## Conclusion

#1180 is complete as a source-acquisition and triage layer:

- current CIViC public source bytes are downloaded and hashed;
- source constraints for OncoKB and Sanger DepMap/GDSC are recorded instead of
  silently ingesting restricted data;
- cancer evidence, assertions, genes, variants, mapped rows, and unresolved rows
  are persisted separately;
- actionability/evidence-level fields remain validation features, not hypothesis
  scores or clinical claims.

The immediately useful downstream input for #1185 is:

```text
/home/croyse/calyx/fsv/issue1180-precision-oncology-validation-20260704T024318Z/parsed/civic_mapped_rows.jsonl
```

with unresolved evidence available for future concept-expansion work.
