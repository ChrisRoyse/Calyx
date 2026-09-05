//! Issue #1391 - score requests must match the registered forecast exactly.

#[allow(dead_code, reason = "reuse the feedback controller's durable fixture")]
#[path = "issue233_feedback/support.rs"]
mod issue233_feedback_support;
// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;

use calyx_poly::{
    ERR_FEEDBACK_SCORE_MISMATCH, ForecastScoreRequest, run_feedback_controller_cycle,
};

use issue233_feedback_support::{
    cycle_request, fixture, forecast, input, record, score, score_payloads,
};
use support::{named_fsv_root, reset_dir};

type Mutation = fn(&mut ForecastScoreRequest);

#[test]
fn rewritten_registered_fields_fail_before_score_ledger_append() {
    let (root, keep) = named_fsv_root("POLY_ISSUE1391_TEST_ROOT", "issue1391-score-mismatch");
    reset_dir(&root);
    let cases: [(&str, Mutation); 6] = [
        ("probability", mutate_probability),
        ("confidence", mutate_confidence),
        ("forecast_ts", mutate_forecast_ts),
        ("market_id", mutate_market_id),
        ("outcome_id", mutate_outcome_id),
        ("actual_win", mutate_actual_win),
    ];

    for (field, mutate) in cases {
        let mut fx = fixture(&root, field);
        record(
            &fx.vault,
            &mut fx.register,
            forecast("f1391", "cond1391", 0, 100),
        );
        let mut score_request = score("score1391", "f1391", "cond1391", true);
        mutate(&mut score_request);
        let request = cycle_request(
            &fx.report_dir,
            &fx.score_root,
            "cycle1391",
            vec![input("cond1391", 0, 200, false, false)],
            vec![score_request],
            Vec::new(),
            None,
        );

        let err = run_feedback_controller_cycle(
            &request,
            &fx.vault,
            &mut fx.register,
            &mut fx.score_ledger,
        )
        .expect_err("rewritten score field must fail closed");
        assert_eq!(err.code(), ERR_FEEDBACK_SCORE_MISMATCH);
        assert!(err.to_string().contains(field), "{field}: {err}");
        assert!(score_payloads(&fx.score_ledger_dir).is_empty());
        assert!(!fx.score_root.join("score1391").exists());
    }

    if !keep {
        fs::remove_dir_all(&root).expect("remove mismatch test root");
    }
}

#[test]
fn exact_registered_score_is_ledger_stamped() {
    let (root, keep) = named_fsv_root("POLY_ISSUE1391_MATCH_ROOT", "issue1391-score-match");
    reset_dir(&root);
    {
        let mut fx = fixture(&root, "exact-match");
        record(
            &fx.vault,
            &mut fx.register,
            forecast("f1391exact", "cond1391exact", 0, 100),
        );
        let request = cycle_request(
            &fx.report_dir,
            &fx.score_root,
            "cycle1391exact",
            vec![input("cond1391exact", 0, 200, false, false)],
            vec![score("score1391exact", "f1391exact", "cond1391exact", true)],
            Vec::new(),
            None,
        );

        let run = run_feedback_controller_cycle(
            &request,
            &fx.vault,
            &mut fx.register,
            &mut fx.score_ledger,
        )
        .expect("exact registered score must succeed");
        let payloads = score_payloads(&fx.score_ledger_dir);
        assert_eq!(run.report.score_manifests.len(), 1);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["score_id"], "score1391exact");
    }
    if !keep {
        fs::remove_dir_all(&root).expect("remove exact-match test root");
    }
}

fn mutate_probability(request: &mut ForecastScoreRequest) {
    request.probability = f64::from_bits(request.probability.to_bits() + 1);
}

fn mutate_confidence(request: &mut ForecastScoreRequest) {
    request.confidence = f64::from_bits(request.confidence.to_bits() + 1);
}

fn mutate_forecast_ts(request: &mut ForecastScoreRequest) {
    request.forecast_ts += 1;
}

fn mutate_market_id(request: &mut ForecastScoreRequest) {
    request.market_id.push_str("-rewritten");
}

fn mutate_outcome_id(request: &mut ForecastScoreRequest) {
    request.outcome_id.push_str("-rewritten");
    request.outcome.outcome_id = request.outcome_id.clone();
}

fn mutate_actual_win(request: &mut ForecastScoreRequest) {
    request.outcome.actual_win = !request.outcome.actual_win;
}
