use super::super::super::*;

pub(super) fn association_validation_tokens(
    args: &association_validation::AssociationValidationArgs,
) -> Vec<String> {
    let mut out = vec![
        "association-validation-gates".to_string(),
        "--typed-root".to_string(),
        args.typed_root.to_string_lossy().into_owned(),
        "--open-targets-root".to_string(),
        args.open_targets_root.to_string_lossy().into_owned(),
        "--pubtator-root".to_string(),
        args.pubtator_root.to_string_lossy().into_owned(),
        "--clinicaltrials-root".to_string(),
        args.clinicaltrials_root.to_string_lossy().into_owned(),
        "--dgidb-root".to_string(),
        args.dgidb_root.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        args.out_dir.to_string_lossy().into_owned(),
        "--cutoff-year".to_string(),
        args.cutoff_year.to_string(),
        "--score-threshold".to_string(),
        args.score_threshold.to_string(),
        "--min-auroc".to_string(),
        args.min_auroc.to_string(),
        "--min-positive-recall".to_string(),
        args.min_positive_recall.to_string(),
        "--min-negative-suppression".to_string(),
        args.min_negative_suppression.to_string(),
    ];
    if let Some(manifest) = &args.preflight.manifest {
        out.extend([
            "--run-manifest".to_string(),
            manifest.to_string_lossy().into_owned(),
        ]);
    }
    if let Some(stage_id) = &args.preflight.stage_id {
        out.extend(["--run-stage-id".to_string(), stage_id.clone()]);
    }
    out
}

#[test]
fn association_validation_round_trips_through_tokens() {
    let command =
        Subcommand::AssociationValidationGates(association_validation::AssociationValidationArgs {
            typed_root: "/fsv/typed".into(),
            open_targets_root: "/fsv/open-targets".into(),
            pubtator_root: "/fsv/pubtator".into(),
            clinicaltrials_root: "/fsv/clinical".into(),
            dgidb_root: "/fsv/dgidb".into(),
            out_dir: "/fsv/out".into(),
            cutoff_year: 2017,
            score_threshold: 0.45,
            min_auroc: 0.65,
            min_positive_recall: 0.70,
            min_negative_suppression: 0.80,
            preflight: Default::default(),
        });
    let tokens = super::subcommand_tokens(&command);
    assert_eq!(parse(&tokens).unwrap(), command);
}
