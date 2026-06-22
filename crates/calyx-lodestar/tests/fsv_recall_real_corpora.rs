#![cfg(feature = "fsv")]

#[path = "support/real_corpora.rs"]
mod real_corpora;

use std::fs;

use serde_json::json;

use real_corpora::{
    STAMP, calyx_home, cora_graph, run_case, run_text_gap_check, scifact_text, write_json,
};

#[test]
#[ignore = "manual FSV: reads real corpora and writes $CALYX_HOME/fsv reports"]
fn fsv_recall_real_corpora_manual() {
    let home = calyx_home();
    let report_dir = home.join("fsv");
    fs::create_dir_all(&report_dir).expect("create fsv dir");

    let cases = vec![
        scifact_text(&home),
        real_corpora::calyx_code(&home),
        cora_graph(&home),
    ];
    let mut reports = Vec::new();
    let mut ratios = Vec::new();

    for case in &cases {
        let report = run_case(case);
        let path = report_dir.join(format!("ph33_recall_{}_{}.json", case.name, STAMP));
        write_json(&path, &report);
        println!(
            "PH33_REAL_RECALL_REPORT={} ratio={} final_members={} rows={}",
            path.display(),
            report.final_recall.ratio,
            report.final_member_count,
            report.row_count
        );
        assert!(
            report.final_recall.ratio >= 0.95,
            "{} ratio {} below gate",
            case.name,
            report.final_recall.ratio
        );
        assert!(
            report.final_recall.warning.is_none(),
            "{} emitted recall warning {:?}",
            case.name,
            report.final_recall.warning
        );
        assert!(
            report.final_member_count < report.row_count,
            "{} final kernel covered the full corpus",
            case.name
        );
        ratios.push((case.name, report.final_recall.ratio));
        reports.push(report);
    }

    let gap_report = run_text_gap_check(&cases[0]);
    let gap_path = report_dir.join(format!(
        "ph33_grounding_gaps_{}_{}.json",
        cases[0].name, STAMP
    ));
    write_json(&gap_path, &gap_report);
    println!("PH33_REAL_GAPS_REPORT={}", gap_path.display());

    let summary_path = report_dir.join(format!("ph33_real_corpora_summary_{STAMP}.json"));
    write_json(
        &summary_path,
        &json!({
            "stamp": STAMP,
            "reports": reports,
            "gap_report_path": gap_path,
            "ratios": ratios,
        }),
    );
    println!("PH33_REAL_SUMMARY={}", summary_path.display());
}
