use super::super::super::*;

pub(super) fn hypothesis_falsification_tokens(
    args: &hypothesis_falsification::HypothesisFalsificationArgs,
) -> Vec<String> {
    let mut out = vec!["hypothesis-falsification-sweep".to_string()];
    for report in &args.hypotheses_reports {
        out.extend([
            "--hypotheses-report".to_string(),
            report.to_string_lossy().into_owned(),
        ]);
    }
    out.extend([
        "--pubtator-root".to_string(),
        args.pubtator_root.to_string_lossy().into_owned(),
        "--clinicaltrials-root".to_string(),
        args.clinicaltrials_root.to_string_lossy().into_owned(),
        "--dgidb-root".to_string(),
        args.dgidb_root.to_string_lossy().into_owned(),
        "--open-targets-root".to_string(),
        args.open_targets_root.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        args.out_dir.to_string_lossy().into_owned(),
        "--max-hypotheses".to_string(),
        args.max_hypotheses.to_string(),
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
fn hypothesis_falsification_round_trips_through_tokens() {
    let command = Subcommand::HypothesisFalsificationSweep(
        hypothesis_falsification::HypothesisFalsificationArgs {
            hypotheses_reports: vec![
                "/fsv/broad/report.json".into(),
                "/fsv/scoped/report.json".into(),
            ],
            pubtator_root: "/fsv/pubtator".into(),
            clinicaltrials_root: "/fsv/clinicaltrials".into(),
            dgidb_root: "/fsv/dgidb".into(),
            open_targets_root: "/fsv/open-targets".into(),
            out_dir: "/fsv/falsification".into(),
            max_hypotheses: 300,
            preflight: Default::default(),
        },
    );
    let tokens = super::subcommand_tokens(&command);
    assert_eq!(parse(&tokens).unwrap(), command);
}
