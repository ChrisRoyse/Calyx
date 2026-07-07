use super::super::super::biomedical_blindspot_audit;

pub(super) fn biomedical_blindspot_audit_tokens(
    args: &biomedical_blindspot_audit::BiomedicalBlindspotAuditArgs,
) -> Vec<String> {
    let mut out = vec!["biomedical-blindspot-audit".to_string()];
    for path in &args.hypotheses_reports {
        out.extend([
            "--hypotheses-report".to_string(),
            path.to_string_lossy().into_owned(),
        ]);
    }
    out.extend([
        "--literature-audit".to_string(),
        args.literature_audit.to_string_lossy().into_owned(),
        "--stability-audit".to_string(),
        args.stability_audit.to_string_lossy().into_owned(),
        "--drug-lifecycle".to_string(),
        args.drug_lifecycle.to_string_lossy().into_owned(),
        "--transcriptomic-audit".to_string(),
        args.transcriptomic_audit.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        args.out_dir.to_string_lossy().into_owned(),
        "--known-literature-threshold".to_string(),
        args.known_literature_threshold.to_string(),
        "--min-stability-frequency".to_string(),
        args.min_stability_frequency.to_string(),
        "--max-transcriptomic-class-breadth".to_string(),
        args.max_transcriptomic_class_breadth.to_string(),
    ]);
    out
}
