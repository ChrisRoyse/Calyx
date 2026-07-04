# 52 - #1206 Falsification asserted-relation gate

## Scope

#1206 hardens `calyx hypothesis-falsification-sweep` so support/counter
evidence can only attach to a hypothesis when both endpoints are present in the
same structured asserted-relation fields. Whole-row substring co-mention is no
longer a validation or falsification gate.

This remains hypothesis triage only. It is not efficacy, safety, actionability,
or cure evidence.

## Code Change

Commits:

```text
42868abc Harden falsification evidence matching
6aed862a Allow concept ids to match asserted endpoint labels
```

Matcher behavior after this change:

- PubTator rows match only `left_id/left_term/left` and
  `right_id/right_term/right`.
- ClinicalTrials seed summaries match only `intervention` and `condition`.
- ClinicalTrials trial rows match only `query_intervention/intervention` and
  `query_condition/condition`.
- DGIdb rows match only `source_overlay_id/drug/drug_name` and
  `target_overlay_id/gene/gene_name`.
- Open Targets rows match only `overlay_target_concepts/target_id/target_name`
  and `overlay_disease_concepts/disease_id/disease_name`.
- Rows with classifiable support/counter polarity but missing asserted endpoint
  fields are persisted to `skipped_evidence.jsonl` with
  `CALYX_FALSIFY_UNSTRUCTURED_ROW`; they are not counted.
- External identifiers such as CHEMBL/HGNC must match endpoint identifiers when
  endpoint identifiers are present. Calyx internal `concept:*` ids may still
  match exact structured endpoint labels such as `@GENE_CD4`.

The persisted schema is now version 2 and includes:

- `skipped_evidence_count`
- `skipped_evidence`
- `skipped_evidence.jsonl` plus hash in CLI summary output

## Local Gates

Passed:

```bash
cargo fmt -p calyx-cli
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Focused regression coverage:

- stopped trial co-mentions a target disease in free text but asserts a
  different intervention-condition pair: no counter-evidence is counted;
- CHEMBL/HGNC exact-id match accepts only the row with the matching endpoint id,
  not a different CHEMBL id sharing a namespace or numeric-looking suffix;
- `concept:ncbi_gene:*` hypotheses match PubTator `@GENE_*` structured endpoint
  labels by exact endpoint label;
- unstructured classifiable row is skipped with
  `CALYX_FALSIFY_UNSTRUCTURED_ROW` and counts stay unchanged.

Synthetic FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1206-falsification-asserted-relation-20260703T225431Z
```

Synthetic readback:

```json
{
  "schema_version": 2,
  "support_evidence_count": 3,
  "counter_evidence_count": 1,
  "skipped_evidence_count": 1,
  "kidney_counter_count": 0,
  "chembl_support_source_row_index": 2,
  "skipped_reason_code": "CALYX_FALSIFY_UNSTRUCTURED_ROW"
}
```

## Real-Data FSV

Release build and full persisted-source sweep were run on `aiwonder` after
fast-forwarding to `6aed862a`.

FSV root:

```text
/home/croyse/calyx/fsv/issue1206-falsification-asserted-relation-20260704T035957Z
```

Inputs:

| Input | Path |
|---|---|
| #1183 broad report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/broad/typed_association_miner_report.json` |
| #1183 chemical/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/chemical_disease/typed_association_miner_report.json` |
| #1183 gene/disease report | `/home/croyse/calyx/fsv/issue1183-typed-association-miner-20260704T021809Z/gene_disease/typed_association_miner_report.json` |
| PubTator/PubMed | `/home/croyse/calyx/fsv/issue1176-pubtator-pubmed-relations-20260703T170855Z` |
| ClinicalTrials.gov | `/home/croyse/calyx/fsv/issue1177-clinicaltrials-validation-20260703T172800Z` |
| DGIdb | `/home/croyse/calyx/fsv/issue1178-dgidb-drug-gene-20260703T174000Z` |
| Open Targets | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z` |

Readback summary:

| Field | Value |
|---|---:|
| Input hypothesis rows | 301 |
| Deduped hypotheses | 280 |
| Support evidence rows retained | 8 |
| Counter-evidence rows retained | 0 |
| Skipped evidence rows | 0 |
| Evidence rows relation-readback verified | 8 |
| Invalid relation evidence rows | 0 |
| Hypotheses flagged with counter-evidence | 0 |

Artifact hashes:

| Artifact | SHA-256 |
|---|---|
| `falsification_sweep_report.json` | `5220ad982a2af59ca9e7fe82c1801f9d6f77aa55023ea4f1df0b5696f918b25a` |
| `support_evidence.jsonl` | `f082960a20d5cd8b4c294fab093ed24e1347ebb8ac17cb3c185a3264ff3dabaa` |
| `counter_evidence.jsonl` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `skipped_evidence.jsonl` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `hypothesis_flags.jsonl` | `fef7ba1531d4bc7ca26873221ea7a4acf2d8c7560ae021ea0a1a4c86208b4b83` |
| `raw_query_manifest.jsonl` | `bd2c94ae621a089cea8cac7826284ba3d00d5065d3b04928f5b07ca5ceef3bbf` |

Separate readback reopened every cited `source_row_index` from the persisted
source rows and verified both hypothesis endpoints against the same asserted
relation record. Example retained evidence:

```text
typed-assoc:concept:ncbi_gene:22925::concept:ncbi_mesh:D011507
source_name=PLA2R1 target_name=Proteinuria
source_row left=@GENE_PLA2R1 right=@DISEASE_Proteinuria
```

The prior #1184 sweep counted 59 support rows and 1 counter row under whole-row
co-mention matching. The stricter asserted-relation gate retained 8 support rows
and removed the counter row. The removed counter was Open Targets row 705:

```json
{
  "target_name": "PLA2R1",
  "disease_name": "diabetic kidney disease",
  "overlay_target_concepts": ["concept:ncbi_gene:22925"],
  "overlay_disease_concepts": [],
  "score": 0.03701863799150296
}
```

That row does not assert the previous hypothesis endpoint `Kidney Diseases` by
exact disease endpoint or overlay disease concept, so it is no longer valid
counter-evidence for PLA2R1 / Kidney Diseases.

## Conclusion

#1206 closes the false-validation/false-falsification hole in the sweep. The
remaining retained evidence rows are relation-field-backed; broad co-mentions
are excluded unless a source parser exposes a structured relation endpoint pair.

