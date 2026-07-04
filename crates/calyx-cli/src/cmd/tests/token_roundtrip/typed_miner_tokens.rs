use super::super::super::*;
use super::push_opt;

pub(super) fn typed_association_miner_tokens(
    args: &typed_association_miner::TypedAssociationMinerArgs,
) -> Vec<String> {
    let mut out = vec![
        "typed-association-miner".to_string(),
        "--typed-root".to_string(),
        args.typed_root.to_string_lossy().into_owned(),
        "--validation-report".to_string(),
        args.validation_report.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        args.out_dir.to_string_lossy().into_owned(),
    ];
    push_opt(&mut out, "--source-type", args.source_type.as_deref());
    push_opt(&mut out, "--target-type", args.target_type.as_deref());
    push_opt(&mut out, "--name-contains", args.name_contains.as_deref());
    if let Some(issue) = args.source_issue {
        out.extend(["--source-issue".to_string(), issue.to_string()]);
    }
    out.extend([
        "--min-support".to_string(),
        args.min_support.to_string(),
        "--max-pairs".to_string(),
        args.max_pairs.to_string(),
        "--max-input-edges".to_string(),
        args.max_input_edges.to_string(),
        "--max-paths-per-pair".to_string(),
        args.max_paths_per_pair.to_string(),
    ]);
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
fn typed_association_miner_round_trips_through_tokens() {
    let command =
        Subcommand::TypedAssociationMiner(typed_association_miner::TypedAssociationMinerArgs {
            typed_root: "/fsv/typed".into(),
            validation_report: "/fsv/validation/report.json".into(),
            out_dir: "/fsv/miner".into(),
            source_type: Some("chemical".to_string()),
            target_type: Some("disease".to_string()),
            name_contains: Some("asthma".to_string()),
            source_issue: Some(1173),
            min_support: 2,
            max_pairs: 50,
            max_input_edges: 200_000,
            max_paths_per_pair: 4,
            preflight: Default::default(),
        });
    let tokens = super::subcommand_tokens(&command);
    assert_eq!(parse(&tokens).unwrap(), command);
}
