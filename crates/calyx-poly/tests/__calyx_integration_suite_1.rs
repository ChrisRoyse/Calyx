//! Generated integration-test harness. Regenerate with
//! `python scripts/consolidate_integration_tests.py`.

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "fsv_support.rs"]
mod __calyx_shared_fsv_support_rs;

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "live_calyx_native_evidence_support.rs"]
mod __calyx_shared_live_calyx_native_evidence_support_rs;

#[path = "synthetic_panels.rs"]
mod __calyx_shared_synthetic_panels_rs;

#[path = "issue012_crypto_capture_scheduler_fsv.rs"]
mod issue012_crypto_capture_scheduler_fsv;
#[path = "issue012_daily_ops_scheduler_fsv.rs"]
mod issue012_daily_ops_scheduler_fsv;
#[path = "issue022_seed_registry_fsv.rs"]
mod issue022_seed_registry_fsv;
#[path = "issue023_readme_fsv.rs"]
mod issue023_readme_fsv;
#[path = "issue024_gamma_client_fsv.rs"]
mod issue024_gamma_client_fsv;
#[path = "issue025_clob_client_fsv.rs"]
mod issue025_clob_client_fsv;
#[path = "issue030_derived_feature_stage_fsv.rs"]
mod issue030_derived_feature_stage_fsv;
#[path = "issue037_feed_outage_fsv.rs"]
mod issue037_feed_outage_fsv;
#[path = "issue038_crypto_ingestor_fsv.rs"]
mod issue038_crypto_ingestor_fsv;
#[path = "issue042_toxicity_lens_fsv.rs"]
mod issue042_toxicity_lens_fsv;
#[path = "issue048_association_fanout_ingest_fsv.rs"]
mod issue048_association_fanout_ingest_fsv;
#[path = "issue050_assay_bits_fsv.rs"]
mod issue050_assay_bits_fsv;
#[path = "issue053_structural_edges_fsv.rs"]
mod issue053_structural_edges_fsv;
#[path = "issue070_entity_graph_edges_fsv.rs"]
mod issue070_entity_graph_edges_fsv;
#[path = "issue072_knn_graph_edges_fsv.rs"]
mod issue072_knn_graph_edges_fsv;
#[path = "issue077_outcome_backfill_scheduler_fsv.rs"]
mod issue077_outcome_backfill_scheduler_fsv;
#[path = "issue080_no_lookahead_fsv.rs"]
mod issue080_no_lookahead_fsv;
#[path = "issue092_honesty_gate_fsv.rs"]
mod issue092_honesty_gate_fsv;
#[path = "issue094_book_liquidity_fsv.rs"]
mod issue094_book_liquidity_fsv;
#[path = "issue097_calibration_backtest_fsv.rs"]
mod issue097_calibration_backtest_fsv;
#[path = "issue104_anneal_integration_fsv.rs"]
mod issue104_anneal_integration_fsv;
#[path = "issue105_parameter_adaptation_fsv.rs"]
mod issue105_parameter_adaptation_fsv;
#[path = "issue108_strategy_learning_fsv.rs"]
mod issue108_strategy_learning_fsv;
#[path = "issue109_regime_detection_fsv.rs"]
mod issue109_regime_detection_fsv;
#[path = "issue112_blend_relearning_fsv.rs"]
mod issue112_blend_relearning_fsv;
#[path = "issue113_meta_learning_ledger_fsv.rs"]
mod issue113_meta_learning_ledger_fsv;
#[path = "issue128_single_workstation_runbook_fsv.rs"]
mod issue128_single_workstation_runbook_fsv;
#[path = "issue1292_live_superiority_evidence.rs"]
mod issue1292_live_superiority_evidence;
#[path = "issue133_e2e_fsv.rs"]
mod issue133_e2e_fsv;
#[path = "issue134_backtest_fsv.rs"]
mod issue134_backtest_fsv;
#[path = "issue135_reproducibility_fsv.rs"]
mod issue135_reproducibility_fsv;
#[path = "issue136_kernel_recall_gate_fsv.rs"]
mod issue136_kernel_recall_gate_fsv;
#[path = "issue137_edge_case_harness_fsv.rs"]
mod issue137_edge_case_harness_fsv;
#[path = "issue1390_score_authority.rs"]
mod issue1390_score_authority;
#[path = "issue140_wash_trade_screen_fsv.rs"]
mod issue140_wash_trade_screen_fsv;
#[path = "issue142_modeling_guards_fsv.rs"]
mod issue142_modeling_guards_fsv;
#[path = "issue143_false_sufficiency_fsv.rs"]
mod issue143_false_sufficiency_fsv;
#[path = "issue144_kill_switch_fsv.rs"]
mod issue144_kill_switch_fsv;
#[path = "issue163_deepseek_infisical_fsv.rs"]
mod issue163_deepseek_infisical_fsv;
#[path = "issue164_agent_artifact_schema_fsv.rs"]
mod issue164_agent_artifact_schema_fsv;
#[path = "issue165_agent_launcher_fsv.rs"]
mod issue165_agent_launcher_fsv;
#[path = "issue166_forecast_scoring_fsv.rs"]
mod issue166_forecast_scoring_fsv;
#[path = "issue19_file_size_lint_fsv.rs"]
mod issue19_file_size_lint_fsv;
#[path = "issue209_trust_lifecycle_fsv.rs"]
mod issue209_trust_lifecycle_fsv;
#[path = "issue21_structured_logging_fsv.rs"]
mod issue21_structured_logging_fsv;
#[path = "issue233_feedback_controller_fsv.rs"]
mod issue233_feedback_controller_fsv;
#[path = "issue235_resolution_provenance_fsv.rs"]
mod issue235_resolution_provenance_fsv;
#[path = "issue237_first_light_audit_fsv.rs"]
mod issue237_first_light_audit_fsv;
#[path = "issue237_live_first_light_fsv.rs"]
mod issue237_live_first_light_fsv;
#[path = "issue238_crypto_capture_harness_fsv.rs"]
mod issue238_crypto_capture_harness_fsv;
#[path = "issue238_live_resolution_recheck_fsv.rs"]
mod issue238_live_resolution_recheck_fsv;
#[path = "issue238_public_search_discovery_fsv.rs"]
mod issue238_public_search_discovery_fsv;
#[path = "issue240_pending_forecast_replay_fsv.rs"]
mod issue240_pending_forecast_replay_fsv;
#[path = "issue241_calyx_native_capture_seam_fsv.rs"]
mod issue241_calyx_native_capture_seam_fsv;
#[path = "lenses_behavior.rs"]
mod lenses_behavior;
#[path = "local_only_guard.rs"]
mod local_only_guard;
