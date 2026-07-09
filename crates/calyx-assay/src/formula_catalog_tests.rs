use super::*;

#[test]
fn formula_coverage_catalog_has_all_prd22_rows() {
    let artifact = formula_coverage_artifact("/tmp/fsv-639", 1);
    let summary = validate_formula_coverage(&artifact).unwrap();
    assert_eq!(
        (
            summary.total_rows,
            summary.covered_rows,
            summary.missing_rows
        ),
        (38, 38, 0)
    );
}

#[test]
fn incomplete_formula_spec_is_missing_not_self_attested() {
    let row = FormulaRowSpec {
        formula: "unwired",
        prd_ref: "22 test",
        engine: "assay",
        callable: "",
        tunable_params: NONE,
        test: "",
    }
    .row("/tmp/fsv-missing");

    assert_eq!(row.status, FormulaCoverageStatus::Missing);
}
