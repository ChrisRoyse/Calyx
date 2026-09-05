//! Generated integration-test harness. Regenerate with
//! `python scripts/consolidate_integration_tests.py`.

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "fsv_support.rs"]
mod __calyx_shared_fsv_support_rs;

#[path = "synthetic_panels.rs"]
mod __calyx_shared_synthetic_panels_rs;

#[allow(
    dead_code,
    reason = "legacy zero-test helper target retained for compile coverage"
)]
#[path = "daily_ops_scheduler_fixture.rs"]
mod daily_ops_scheduler_fixture;
#[path = "issue026_data_api_client_fsv.rs"]
mod issue026_data_api_client_fsv;
#[path = "issue028_uma_resolution_watcher_fsv.rs"]
mod issue028_uma_resolution_watcher_fsv;
#[path = "issue029_ws_market_client_fsv.rs"]
mod issue029_ws_market_client_fsv;
#[path = "issue031_constellation_vault_put_fsv.rs"]
mod issue031_constellation_vault_put_fsv;
#[path = "issue035_historical_backfill_loader_fsv.rs"]
mod issue035_historical_backfill_loader_fsv;
#[path = "issue036_rate_limit_governor_fsv.rs"]
mod issue036_rate_limit_governor_fsv;
#[path = "issue039_poly_panel_registry_fsv.rs"]
mod issue039_poly_panel_registry_fsv;
#[path = "issue041_book_shape_lens_fsv.rs"]
mod issue041_book_shape_lens_fsv;
#[path = "issue043_temporal_lenses_fsv.rs"]
mod issue043_temporal_lenses_fsv;
#[path = "issue044_region_vocab_fsv.rs"]
mod issue044_region_vocab_fsv;
#[path = "issue046_question_bm25_fsv.rs"]
mod issue046_question_bm25_fsv;
#[path = "issue049_loom_weave_fsv.rs"]
mod issue049_loom_weave_fsv;
#[path = "issue073_domain_graph_build_job_fsv.rs"]
mod issue073_domain_graph_build_job_fsv;
#[path = "issue074_fanout_selection_fsv.rs"]
mod issue074_fanout_selection_fsv;
#[path = "issue076_proxy_anchors_fsv.rs"]
mod issue076_proxy_anchors_fsv;
#[path = "issue078_anchor_floor_tracker_fsv.rs"]
mod issue078_anchor_floor_tracker_fsv;
#[path = "issue079_panel_sufficiency_fsv.rs"]
mod issue079_panel_sufficiency_fsv;
#[path = "issue091_ward_calibration_fsv.rs"]
mod issue091_ward_calibration_fsv;
#[path = "issue096_forecast_admission_provenance_fsv.rs"]
mod issue096_forecast_admission_provenance_fsv;
#[path = "issue106_mistake_closure_fsv.rs"]
mod issue106_mistake_closure_fsv;
#[path = "issue107_drift_recalibration_fsv.rs"]
mod issue107_drift_recalibration_fsv;
#[path = "issue110_lens_autobuild_fsv.rs"]
mod issue110_lens_autobuild_fsv;
#[path = "issue111_calibration_refit_fsv.rs"]
mod issue111_calibration_refit_fsv;
#[path = "issue114_self_evolution_guardrails_fsv.rs"]
mod issue114_self_evolution_guardrails_fsv;
#[path = "issue124_service_scaffold_fsv.rs"]
mod issue124_service_scaffold_fsv;
#[path = "issue125_forecast_observability_fsv.rs"]
mod issue125_forecast_observability_fsv;
#[path = "issue126_metrics_scrape_fsv.rs"]
mod issue126_metrics_scrape_fsv;
#[path = "issue129_daily_jobs_runbook_fsv.rs"]
mod issue129_daily_jobs_runbook_fsv;
#[allow(
    dead_code,
    reason = "legacy zero-test helper target retained for compile coverage"
)]
#[path = "issue133_support.rs"]
mod issue133_support;
#[path = "issue1391_score_request_integrity.rs"]
mod issue1391_score_request_integrity;
#[path = "issue139_oracle_risk_screen_fsv.rs"]
mod issue139_oracle_risk_screen_fsv;
#[path = "issue141_thin_market_screen_fsv.rs"]
mod issue141_thin_market_screen_fsv;
#[path = "issue146_provisional_vault_fsv.rs"]
mod issue146_provisional_vault_fsv;
#[path = "issue158_grounding_ledger_fsv.rs"]
mod issue158_grounding_ledger_fsv;
#[path = "issue162_no_trade_policy_fsv.rs"]
mod issue162_no_trade_policy_fsv;
#[path = "issue168_forecast_admission_api_fsv.rs"]
mod issue168_forecast_admission_api_fsv;
#[path = "issue171_snapshot_identity_fsv.rs"]
mod issue171_snapshot_identity_fsv;
#[path = "issue173_raw_source_readback_fsv.rs"]
mod issue173_raw_source_readback_fsv;
#[path = "issue174_schema_derivation_fsv.rs"]
mod issue174_schema_derivation_fsv;
#[path = "issue182_derived_feature_absence_fsv.rs"]
mod issue182_derived_feature_absence_fsv;
#[path = "issue184_confidence_ceiling_fsv.rs"]
mod issue184_confidence_ceiling_fsv;
#[path = "issue207_panel_diagnostics_fsv.rs"]
mod issue207_panel_diagnostics_fsv;
#[path = "issue208_pair_gain_gate_fsv.rs"]
mod issue208_pair_gain_gate_fsv;
#[path = "issue20_poly_config_fsv.rs"]
mod issue20_poly_config_fsv;
#[path = "issue213_onchain_readback_fsv.rs"]
mod issue213_onchain_readback_fsv;
#[path = "issue215_oracle_forecast_component_fsv.rs"]
mod issue215_oracle_forecast_component_fsv;
#[path = "issue216_computed_kernel_recall_admission_fsv.rs"]
mod issue216_computed_kernel_recall_admission_fsv;
#[path = "issue231_shape_aware_loom_weave_fsv.rs"]
mod issue231_shape_aware_loom_weave_fsv;
#[path = "issue234_pending_forecast_register_fsv.rs"]
mod issue234_pending_forecast_register_fsv;
#[path = "issue237_calyx_native_accumulation_fsv.rs"]
mod issue237_calyx_native_accumulation_fsv;
#[path = "issue237_calyx_native_settlement_batch_fsv.rs"]
mod issue237_calyx_native_settlement_batch_fsv;
#[path = "issue238_nearterm_selection_fsv.rs"]
mod issue238_nearterm_selection_fsv;
#[path = "issue238_snapshot_price_fsv.rs"]
mod issue238_snapshot_price_fsv;
#[path = "issue239_maker_concentration_source_fsv.rs"]
mod issue239_maker_concentration_source_fsv;
#[path = "issue243_historical_point_in_time_replay_fsv.rs"]
mod issue243_historical_point_in_time_replay_fsv;
#[path = "issue_calyx_native_forecast_fsv.rs"]
mod issue_calyx_native_forecast_fsv;
