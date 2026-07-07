pub use crate::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, REFUSE_MISSING_TRUST_EVIDENCE,
    REFUSE_PROVISIONAL_ONLY, admit_forecast, evaluate_admission, refuse_if_provisional_only,
};
pub use crate::agent_artifacts::{
    AgentForecastArtifactRequest, AgentForecastManifest, AgentParsedForecast, AgentPromptArtifact,
    AgentResponseArtifact, AgentSourceSnapshotRef, write_agent_forecast_artifacts,
};
pub use crate::agent_launcher::{
    AGENT_LAUNCH_SCHEMA_VERSION, AgentEvidenceSnapshot, AgentLaunchReceipt, AgentLauncherRequest,
    DeepSeekUsage, launch_deepseek_forecast_agent,
};
pub use crate::agent_secrets::{
    DeepSeekRuntimeSecrets, DeepSeekSecretMetadata, InfisicalDeepSeekSource,
};
pub use crate::anchor_floor::{
    ANCHOR_FLOOR_REPORT_FILE, ANCHOR_FLOOR_SCHEMA_VERSION, AnchorFloorReport, AnchorFloorRequest,
    AnchorFloorRow, AnchorFloorRowAudit, AnchorFloorRun, ERR_ANCHOR_FLOOR_INVALID_REQUEST,
    ERR_ANCHOR_FLOOR_INVALID_ROW, ERR_ANCHOR_FLOOR_READBACK_MISMATCH, MIN_RESOLVED_ANCHOR_FLOOR,
    evaluate_anchor_floor_from_files, read_anchor_floor_report, run_anchor_floor_tracker,
    write_anchor_floor_report,
};
pub use crate::anneal_integration::{
    ANNEAL_INTEGRATION_ARTIFACT_KIND, ANNEAL_INTEGRATION_LEDGER_FILE,
    ANNEAL_INTEGRATION_REPORT_FILE, ANNEAL_INTEGRATION_SCHEMA_VERSION,
    AnnealIntegrationArtifactRef, AnnealIntegrationLedgerEntry, AnnealIntegrationMetricProfile,
    AnnealIntegrationMetricRow, AnnealIntegrationParamSet, AnnealIntegrationReport,
    AnnealIntegrationRequest, AnnealIntegrationRun, AnnealIntegrationStatus,
    AnnealIntegrationTripwireBounds, ERR_ANNEAL_INTEGRATION_INVALID_REQUEST,
    ERR_ANNEAL_INTEGRATION_LEDGER_DECODE, ERR_ANNEAL_INTEGRATION_LEDGER_IO,
    ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT, ERR_ANNEAL_INTEGRATION_READBACK_MISMATCH,
    read_anneal_integration_ledger_entries, read_anneal_integration_report,
    run_anneal_integration_report, write_anneal_integration_report,
};
pub use crate::assay_bits::{
    ASSAY_BITS_ARTIFACT_KIND, ASSAY_BITS_SCHEMA_VERSION, AssayBitsReport, AssayBitsRequest,
    AssayBitsRun, DEFAULT_ASSAY_BITS_K, ERR_ASSAY_BITS_ANCHOR_KIND_MISMATCH,
    ERR_ASSAY_BITS_DEGENERATE_OUTCOME, ERR_ASSAY_BITS_INVALID_REQUEST,
    ERR_ASSAY_BITS_NON_BOOL_ANCHOR, ERR_ASSAY_BITS_READBACK_MISMATCH, RedundancyPair,
    SlotAssayBits, read_assay_bits_report, run_assay_bits_to_vault, write_assay_bits_report,
};
pub use crate::backtest::{
    BacktestMetrics, BacktestObservation, BacktestReport, read_backtest_report, run_backtest,
    write_backtest_report,
};
pub use crate::blend_relearning::{
    BLEND_RELEARNING_ARTIFACT_KIND, BLEND_RELEARNING_REPORT_FILE, BLEND_RELEARNING_SCHEMA_VERSION,
    BlendRelearningReport, BlendRelearningRequest, BlendRelearningRun, BlendWeightObservation,
    BlendWeightRow, ERR_BLEND_RELEARNING_EMPTY, ERR_BLEND_RELEARNING_INSUFFICIENT,
    ERR_BLEND_RELEARNING_INVALID, ERR_BLEND_RELEARNING_NO_SKILL,
    ERR_BLEND_RELEARNING_READBACK_MISMATCH, compute_blend_relearning_report,
    read_blend_relearning_report, run_blend_relearning,
};
pub use crate::book_liquidity::{
    BOOK_LIQUIDITY_FEATURE_ARTIFACT_KIND, BOOK_LIQUIDITY_SCHEMA_VERSION,
    BookLiquidityFeatureRequest, BookLiquidityFeatureRow, BookLiquidityRun, BookLiquidityStatus,
    ERR_BOOK_LIQUIDITY_CROSSED, ERR_BOOK_LIQUIDITY_INVALID_LEVEL,
    ERR_BOOK_LIQUIDITY_INVALID_REQUEST, ERR_BOOK_LIQUIDITY_READBACK_MISMATCH,
    PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND, PublicBookLevel, PublicBookSnapshot,
    compute_book_liquidity_features, read_book_liquidity_features, read_public_book_snapshot,
    run_book_liquidity_feature_extraction, write_book_liquidity_features,
    write_public_book_snapshot,
};
pub use crate::book_shape_lens::{
    BOOK_SHAPE_LEVELS, BOOK_SHAPE_VECTOR_DIM, BookShapeLens, ERR_BOOK_SHAPE_CROSSED,
    ERR_BOOK_SHAPE_INVALID, compute_book_shape_vector,
};
pub use crate::calibration_backtest::{
    CALIBRATION_BACKTEST_ARTIFACT_KIND, CALIBRATION_BACKTEST_REPORT_FILE,
    CALIBRATION_BACKTEST_SCHEMA_VERSION, CalibrationBacktestBin, CalibrationBacktestObservation,
    CalibrationBacktestReport, CalibrationBacktestRequest, CalibrationBacktestRun,
    DomainHorizonCoverage, ERR_CALIBRATION_BACKTEST_EMPTY_HOLDOUT,
    ERR_CALIBRATION_BACKTEST_FUTURE_OUTCOME, ERR_CALIBRATION_BACKTEST_INSUFFICIENT_HOLDOUT,
    ERR_CALIBRATION_BACKTEST_INVALID_REQUEST, ERR_CALIBRATION_BACKTEST_INVALID_ROW,
    ERR_CALIBRATION_BACKTEST_LEAKAGE, ERR_CALIBRATION_BACKTEST_MISSING_ANCHOR,
    ERR_CALIBRATION_BACKTEST_READBACK_MISMATCH, compute_calibration_backtest_report,
    read_calibration_backtest_report, run_calibration_backtest_report,
    write_calibration_backtest_report,
};
pub use crate::calibration_refit::{
    CALIBRATION_REFIT_ARTIFACT_KIND, CALIBRATION_REFIT_REPORT_FILE,
    CALIBRATION_REFIT_SCHEMA_VERSION, CalibrationRefitObservation, CalibrationRefitReport,
    CalibrationRefitRequest, CalibrationRefitRun, ERR_CALIBRATION_REFIT_FUTURE_OBSERVATION,
    ERR_CALIBRATION_REFIT_INVALID, ERR_CALIBRATION_REFIT_NO_IMPROVEMENT,
    ERR_CALIBRATION_REFIT_READBACK_MISMATCH, compute_calibration_refit_report,
    read_calibration_refit_report, run_calibration_refit,
};
pub use crate::capability_gate::{
    ERR_CAPABILITY_GATE_INVALID_REQUEST, ERR_CAPABILITY_GATE_READBACK_MISMATCH,
    POLY_CAPABILITY_GATE_ARTIFACT_KIND, POLY_CAPABILITY_GATE_REPORT_FILE,
    POLY_CAPABILITY_GATE_SCHEMA_VERSION, POLY_CAPABILITY_MAX_PAIRWISE_CORR,
    POLY_CAPABILITY_MIN_SIGNAL_BITS, PolyCapabilityGateDecisionRow, PolyCapabilityGateMeasurement,
    PolyCapabilityGateReport, PolyCapabilityGateRequest, PolyCapabilityGateRun,
    compute_poly_capability_gate_report, read_poly_capability_gate_report,
    run_poly_capability_gate_report, write_poly_capability_gate_report,
};
pub use crate::clob_client::{
    CLOB_BASE_URL, ClobBatchBooksPage, ClobBatchHistoryPage, ClobBatchPricesPage,
    ClobBatchScalarsPage, ClobBookPage, ClobBookStatus, ClobClient, ClobClientConfig,
    ClobHistoryPage, ClobHistoryPoint, ClobJsonPage, ClobLastTrade, ClobLastTradesPage,
    ClobOrderBook, ClobPriceBatchRequest, ClobPriceHistory, ClobScalarKind, ClobScalarPage,
    ClobScalarQuote, ClobSide, ClobTokenPrices, ERR_CLOB_BODY_READ, ERR_CLOB_BOOK_CROSSED,
    ERR_CLOB_BOOK_INVALID, ERR_CLOB_HTTP, ERR_CLOB_JSON, ERR_CLOB_REQUEST_INVALID,
    ERR_CLOB_SCALAR_INVALID, parse_clob_batch_history_value, parse_clob_books_value,
    parse_clob_history_value, parse_clob_last_trades_value, parse_clob_order_book,
    parse_clob_price_map_value, parse_clob_scalar_map_value, parse_clob_scalar_value,
};
pub use crate::config::{POLY_CONFIG_LOADED, PolyConfig};
mod crypto;
pub use crate::data_api_client::{
    DATA_API_BASE_URL, DATA_API_TRADES_OFFSET_CAP, DataApiActivityPage, DataApiActivityRecord,
    DataApiBoundedWindowPage, DataApiClient, DataApiClientConfig, DataApiConcentrationInputs,
    DataApiEvidenceStatus, DataApiHolderGroup, DataApiHolderRecord, DataApiHoldersPage,
    DataApiJsonPage, DataApiMarketPositionGroup, DataApiMarketPositionsPage,
    DataApiOpenInterestPage, DataApiOpenInterestRecord, DataApiPositionRecord,
    DataApiPositionsPage, DataApiTradeRecord, DataApiTradeSide, DataApiTradesPage,
    ERR_DATA_API_BODY_READ, ERR_DATA_API_BOUNDED_WINDOW, ERR_DATA_API_HTTP, ERR_DATA_API_JSON,
    ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE, ERR_DATA_API_REQUEST_INVALID,
    ERR_DATA_API_ROW_INVALID, build_data_api_concentration_inputs, parse_data_api_activity_value,
    parse_data_api_holders_value, parse_data_api_market_positions_value,
    parse_data_api_open_interest_value, parse_data_api_positions_value,
    parse_data_api_trades_value,
};
pub use crate::domain::Domain;
pub use crate::drift_recalibration::{
    AdmissionConfigSnapshot, DRIFT_RECALIBRATION_ARTIFACT_KIND, DRIFT_RECALIBRATION_REPORT_FILE,
    DRIFT_RECALIBRATION_SCHEMA_VERSION, DriftMetricWindow, DriftRecalibrationReport,
    DriftRecalibrationRequest, DriftRecalibrationRun, DriftRecalibrationStatus,
    DriftRecalibrationThresholds, ERR_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS,
    ERR_DRIFT_RECALIBRATION_INVALID_REQUEST, ERR_DRIFT_RECALIBRATION_READBACK_MISMATCH,
    compute_drift_recalibration_report, read_drift_recalibration_report,
    run_drift_recalibration_report, write_drift_recalibration_report,
};
pub use crate::error::{PolyError, PolyErrorDiagnostic, Result};
pub use crate::fanout_selection::{
    ERR_FANOUT_SELECTION_DUPLICATE_PAIR, ERR_FANOUT_SELECTION_EMPTY,
    ERR_FANOUT_SELECTION_INVALID_CANDIDATE, ERR_FANOUT_SELECTION_INVALID_REQUEST,
    ERR_FANOUT_SELECTION_READBACK_MISMATCH, ExpensiveAssociationEstimator,
    FANOUT_SELECTION_ARTIFACT_KIND, FANOUT_SELECTION_SCHEMA_VERSION, FanoutCandidate,
    FanoutDecision, FanoutDecisionKind, FanoutDropReason, FanoutSelectionReport,
    FanoutSelectionRequest, FanoutSelectionRun, FanoutThresholds, compute_fanout_selection_report,
    read_fanout_selection_report, run_fanout_selection_report, write_fanout_selection_report,
};
pub use crate::feedback_controller::{
    ERR_FEEDBACK_INVALID_REQUEST, ERR_FEEDBACK_MISSING_SCORE, ERR_FEEDBACK_READBACK_MISMATCH,
    ERR_FEEDBACK_RESOLUTION_NOT_FINAL, ERR_FEEDBACK_SCORE_MISMATCH,
    FEEDBACK_CONTROLLER_ARTIFACT_KIND, FEEDBACK_CONTROLLER_REPORT_FILE,
    FEEDBACK_CONTROLLER_SCHEMA_VERSION, FeedbackBackfillInput, FeedbackBackfillResult,
    FeedbackControllerCycleRequest, FeedbackControllerReport, FeedbackControllerRun,
    FeedbackLearningRequest, FeedbackLearningResult, FeedbackMetaLearningRequest,
    FeedbackResolutionInput, read_feedback_controller_report, run_feedback_controller_cycle,
};
pub use crate::file_size_lint::{
    DEFAULT_FILE_SIZE_LINE_LIMIT, FILE_SIZE_LINT_PASSED, FILE_SIZE_LINT_SCHEMA_VERSION,
    FileSizeLintFailure, FileSizeLintRecord, FileSizeLintReport, FileSizeLintRequest,
    FileSizeLintRootState, evaluate_file_size_lint, read_file_size_lint_report,
    require_file_size_lint_passed, write_file_size_lint_report,
};
pub use crate::forecast::{ComponentKind, ForecastComponent};
pub use crate::forecast_observability::{
    ERR_OBSERVABILITY_INVALID_REQUEST, ERR_OBSERVABILITY_READBACK_MISMATCH,
    ERR_OBSERVABILITY_STALE_INGEST, FORECAST_OBSERVABILITY_ARTIFACT_KIND,
    FORECAST_OBSERVABILITY_METRICS_FILE, FORECAST_OBSERVABILITY_REPORT_FILE,
    FORECAST_OBSERVABILITY_SCHEMA_VERSION, ForecastObservabilityReport,
    ForecastObservabilityRequest, ForecastObservabilityRun, ForecastObservabilityStatus,
    ForecastObservabilityThresholds, ForecastQualityMetrics, ForecastRefusalMetric,
    METRIC_ASSOCIATION_COVERAGE_RATIO, METRIC_DEEPSEEK_AGENT_FAILURES_TOTAL,
    METRIC_FORECAST_BRIER_MEAN, METRIC_FORECAST_CALIBRATION_ABS_ERROR_MEAN,
    METRIC_FORECAST_DIRECTION_ACCURACY, METRIC_FORECAST_REFUSALS_TOTAL,
    METRIC_FORECAST_SCORED_TOTAL, METRIC_INGEST_FRESHNESS_SECONDS,
    compute_forecast_observability_report, read_forecast_observability_metrics,
    read_forecast_observability_report, require_forecast_observability_healthy,
    run_forecast_observability_report, write_forecast_observability_report,
};
pub use crate::gamma_client::{
    ERR_GAMMA_BODY_READ, ERR_GAMMA_HTTP, ERR_GAMMA_JSON, ERR_GAMMA_MARKET_INVALID,
    ERR_GAMMA_REQUEST_INVALID, GAMMA_BASE_URL, GAMMA_CRYPTO_TAG_ID, GammaClient, GammaClientConfig,
    GammaJoinKey, GammaMarketRecord, GammaMarketsPage, GammaMarketsRequest, GammaOutcomeShape,
    parse_gamma_market, parse_gamma_markets_value,
};
pub use crate::gamma_metadata::{
    ERR_GAMMA_METADATA_INVALID, GammaEventRecord, GammaEventsPage, GammaEventsRequest,
    GammaSeriesPage, GammaSeriesRecord, GammaSeriesRequest, GammaTagRecord, GammaTagsPage,
    GammaTagsRequest, parse_gamma_events_value, parse_gamma_series_value, parse_gamma_tags_value,
};
pub use crate::grounding::{
    ERR_BACKFILL_CONTRADICTION, ERR_BACKFILL_NOT_RESOLVED, ERR_GAMMA_DERIVED_AS_UMA,
    ERR_GAMMA_SUPERSESSION_KIND, GAMMA_CLOSED_DERIVED_SOURCE_PREFIX, GroundingKind,
    PROXY_SOURCE_PREFIX, ProxyKind, RESOLVED_SOURCE_PREFIX, ResolutionSupersession,
    ResolutionSupersessionKind, TrustTransition, grounding_kind_of, promote_on_resolution,
    proxy_anchor, rollup_trust, supersede_gamma_closed_resolution,
};
pub use crate::kernel_recall::{
    DomainKernelRecallInput, DomainKernelRecallReport, KernelRecallVerificationReport,
    POLY_KERNEL_RECALL_GATE_PASSED, POLY_KERNEL_RECALL_MIN_RATIO, measure_kernel_recall_per_domain,
    read_kernel_recall_report, verify_kernel_recall_per_domain, write_kernel_recall_report,
};
pub use crate::kernel_recall_admission::{
    COMPUTED_KERNEL_RECALL_ARTIFACT_KIND, COMPUTED_KERNEL_RECALL_SCHEMA_VERSION,
    ComputedKernelRecall, ComputedKernelRecallRequest, apply_measured_kernel_recall,
    measure_computed_kernel_recall, produce_calyx_native_forecast_with_measured_kernel_recall,
    read_computed_kernel_recall, write_computed_kernel_recall,
};
pub use crate::lens_autobuild::{
    BuiltLensSpec, ERR_LENS_AUTOBUILD_INVALID_REQUEST, ERR_LENS_AUTOBUILD_NO_ADMISSIBLE,
    ERR_LENS_AUTOBUILD_NO_CANDIDATES, ERR_LENS_AUTOBUILD_NO_DEFICIT,
    ERR_LENS_AUTOBUILD_READBACK_MISMATCH, LENS_AUTOBUILD_ARTIFACT_KIND,
    LENS_AUTOBUILD_MIN_GAIN_BITS, LENS_AUTOBUILD_REPORT_FILE, LENS_AUTOBUILD_SCHEMA_VERSION,
    LensAutobuildReport, LensAutobuildRequest, LensAutobuildRun, LensAutobuildStatus,
    LensCandidateMeasurement, LensCandidateRejection, LensDeficit, compute_lens_autobuild_report,
    read_lens_autobuild_report, require_lens_autobuild_admitted, run_lens_autobuild_report,
    write_lens_autobuild_report,
};
pub use crate::lenses::{PolyPanel, SignalLens};
pub use crate::logging::{
    POLY_LOG_EVENT_RECORDED, POLY_LOG_MAX_CONTEXT_FIELDS, POLY_LOG_MAX_CONTEXT_VALUE_BYTES,
    POLY_STRUCTURED_LOG_SCHEMA_VERSION, PolyLogEvent, PolyLogLevel, PolyResultLogExt,
    StructuredLogSink, log_context, read_structured_log_events,
};
pub use crate::meta_learning_ledger::{
    ERR_META_LEARNING_INVALID_REQUEST, ERR_META_LEARNING_LEDGER_DECODE,
    ERR_META_LEARNING_LEDGER_IO, ERR_META_LEARNING_MISSING_GUARDRAIL,
    ERR_META_LEARNING_MISSING_ROLLBACK, ERR_META_LEARNING_READBACK_MISMATCH,
    META_LEARNING_LEDGER_FILE, META_LEARNING_LEDGER_SCHEMA_VERSION, MetaLearningEffect,
    MetaLearningLedgerEntry, MetaLearningLedgerRequest, MetaLearningLedgerRun,
    append_meta_learning_ledger_entry, read_meta_learning_ledger_entries,
};
pub use crate::metrics_scrape::{
    ERR_METRICS_ALERT, ERR_METRICS_INVALID_REQUEST, ERR_METRICS_MALFORMED,
    ERR_METRICS_MISSING_REQUIRED, ERR_METRICS_MISSING_SINK, ERR_METRICS_READBACK_MISMATCH,
    ERR_METRICS_STALE, METRIC_CHAIN_VERIFY_PASSED, METRIC_GUARD_FAR, METRIC_KERNEL_RECALL,
    METRIC_PANEL_N_EFF, METRICS_SCRAPE_ARTIFACT_KIND, METRICS_SCRAPE_REPORT_FILE,
    METRICS_SCRAPE_SCHEMA_VERSION, MetricCheck, MetricSample, MetricsScrapeReport,
    MetricsScrapeRequest, MetricsScrapeRun, MetricsStatus, MetricsThresholds,
    compute_metrics_scrape_report, parse_prometheus_samples, read_metrics_scrape_report,
    read_prometheus_samples, require_metrics_scrape_healthy, run_metrics_scrape_report,
    write_metrics_scrape_report,
};
pub use crate::mistake_closure::{
    ERR_MISTAKE_CLOSURE_FORBIDDEN_SEMANTIC, ERR_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE,
    ERR_MISTAKE_CLOSURE_INVALID_REQUEST, ERR_MISTAKE_CLOSURE_LOOKAHEAD,
    ERR_MISTAKE_CLOSURE_MISSING_ARTIFACT, ERR_MISTAKE_CLOSURE_MISSING_OUTCOME,
    ERR_MISTAKE_CLOSURE_NO_PROPOSAL, ERR_MISTAKE_CLOSURE_READBACK_MISMATCH,
    MISTAKE_CLOSURE_ARTIFACT_KIND, MISTAKE_CLOSURE_MIN_ROWS, MISTAKE_CLOSURE_REPORT_FILE,
    MISTAKE_CLOSURE_SCHEMA_VERSION, MistakeClosureArtifactRef, MistakeClosureEffect,
    MistakeClosureEvidenceLink, MistakeClosureHeadKind, MistakeClosureProposal,
    MistakeClosureReport, MistakeClosureRequest, MistakeClosureRun, MistakeClosureScoreRow,
    MistakeClosureStatus, MistakeClosureThresholds, compute_mistake_closure_report,
    read_mistake_closure_report, require_mistake_closure_proposed, run_mistake_closure_report,
    write_mistake_closure_report,
};
pub use crate::model::{
    Book, CounterpartyVolume, HolderShare, Level, MakerShare, MakerShareEvidenceSource,
    MarketSnapshot, OnchainFill, OnchainFillSide, OracleRiskEvidence, Resolution,
};
pub use crate::no_lookahead::{
    ERR_LOOKAHEAD_ANCHOR_AFTER_BACKFILL, ERR_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION,
    ERR_LOOKAHEAD_ANCHOR_NOT_AFTER_SNAPSHOT, ERR_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION,
    ERR_LOOKAHEAD_EMPTY_ANCHORS, ERR_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT,
    ERR_LOOKAHEAD_READBACK_MISMATCH, ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT,
    NO_LOOKAHEAD_REPORT_FILE, NO_LOOKAHEAD_SCHEMA_VERSION, NoLookaheadAnchorAudit,
    NoLookaheadReport, NoLookaheadRun, NoLookaheadTiming, compute_no_lookahead_report,
    read_no_lookahead_report, run_no_lookahead_report, validate_no_lookahead_timing,
    validate_resolution_anchor_timing, validate_snapshot_before_resolution,
    write_no_lookahead_report,
};
pub use crate::oracle::{OracleRiskParams, OracleRiskScreen, screen_oracle_risk};
pub use crate::oracle_forecast::{
    ORACLE_FORECAST_ARTIFACT_KIND, ORACLE_FORECAST_SCHEMA_VERSION, OracleForecast,
    produce_oracle_forecast_component, read_oracle_forecast, write_oracle_forecast,
};
pub use crate::outcome_backfill::{
    ERR_BACKFILL_CORPUS_MISMATCH, ERR_BACKFILL_EMPTY, ERR_BACKFILL_INVALID_JOB,
    ERR_BACKFILL_NOT_PROVISIONAL, ERR_BACKFILL_NOT_TRUSTED, ERR_BACKFILL_READBACK_MISMATCH,
    OUTCOME_BACKFILL_REPORT_FILE, OUTCOME_BACKFILL_SCHEMA_VERSION, OutcomeBackfillJob,
    OutcomeBackfillJobReport, OutcomeBackfillReport, OutcomeBackfillRun,
    read_outcome_backfill_report, run_outcome_backfill_schedule, write_outcome_backfill_report,
};
pub use crate::pair_gain_gate::{
    MATERIALIZATION_PLAN_SCHEMA_VERSION, PAIR_GAIN_BITS_THRESHOLD, PairGainMaterializationRecord,
    PairGainMeasurement, compute_pair_gain_plan, read_pair_gain_plan, write_pair_gain_plan,
};
pub use crate::panel_diagnostics::{
    ESTIMATOR_KSG, PANEL_DIAGNOSTICS_ARTIFACT_KIND, PANEL_DIAGNOSTICS_SCHEMA_VERSION,
    PanelDiagnostics, PanelDiagnosticsConfig, PanelMatrix, TripleDiagnostic,
    compute_panel_diagnostics, read_panel_diagnostics, write_panel_diagnostics,
};
pub use crate::panel_sufficiency::{
    ERR_PANEL_SUFFICIENCY_INVALID_REQUEST, ERR_PANEL_SUFFICIENCY_READBACK_MISMATCH,
    POLY_PANEL_SUFFICIENCY_ARTIFACT_KIND, POLY_PANEL_SUFFICIENCY_SCHEMA_VERSION,
    PolyPanelSufficiencyReport, PolyPanelSufficiencyRequest, PolyPanelSufficiencyRun,
    compute_panel_sufficiency_report, read_panel_sufficiency_report, run_panel_sufficiency_report,
    write_panel_sufficiency_report,
};
pub use crate::parameter_adaptation::{
    ERR_PARAMETER_ADAPTATION_DEGENERATE, ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA,
    ERR_PARAMETER_ADAPTATION_INVALID_REQUEST, ERR_PARAMETER_ADAPTATION_LEDGER_DECODE,
    ERR_PARAMETER_ADAPTATION_LEDGER_IO, ERR_PARAMETER_ADAPTATION_LOOKAHEAD,
    ERR_PARAMETER_ADAPTATION_MALFORMED_ROW, ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT,
    ERR_PARAMETER_ADAPTATION_READBACK_MISMATCH, PARAMETER_ADAPTATION_ARTIFACT_KIND,
    PARAMETER_ADAPTATION_LEDGER_FILE, PARAMETER_ADAPTATION_MIN_ROWS,
    PARAMETER_ADAPTATION_REPORT_FILE, PARAMETER_ADAPTATION_SCHEMA_VERSION,
    ParameterAdaptationArtifactRef, ParameterAdaptationLedgerEntry, ParameterAdaptationMetrics,
    ParameterAdaptationReport, ParameterAdaptationRequest, ParameterAdaptationRun,
    ParameterAdaptationSchedule, ParameterAdaptationStatus, ParameterObservation,
    ParameterSetSnapshot, compute_parameter_adaptation_report,
    read_parameter_adaptation_ledger_entries, read_parameter_adaptation_report,
    run_parameter_adaptation_report, write_parameter_adaptation_report,
};
pub use crate::pending_forecast_register::{
    ERR_PENDING_FORECAST_INVALID, ERR_PENDING_FORECAST_LEDGER_APPEND, ERR_PENDING_FORECAST_PAYLOAD,
    PENDING_FORECAST_REGISTERED_EVENT, PENDING_FORECAST_RESOLUTION_JOIN_EVENT,
    PENDING_FORECAST_SCHEMA_VERSION, PendingForecastEntry, PendingForecastObservability,
    PendingForecastRegister, PendingForecastStatus, PendingForecastWorkItem, ResolutionJoinResult,
    join_resolution_to_pending_forecasts, observe_pending_forecasts, record_pending_forecast,
};
pub use crate::policy::{LocalOnlyPolicy, PolicyDecision, PolyAction};
pub use crate::policy_audit::{
    POLICY_AUDIT_SCHEMA_VERSION, PolicyEnforcement, policy_config_snapshot, record_policy_decision,
    require_policy_allowed, write_policy_config_snapshot, write_policy_guarded_artifact,
};
pub use crate::poly_panel_registry::{
    ERR_PANEL_REGISTRY_INVALID, ERR_PANEL_REGISTRY_READBACK_MISMATCH,
    POLY_PANEL_REGISTRY_ARTIFACT_KIND, POLY_PANEL_REGISTRY_FILE,
    POLY_PANEL_REGISTRY_SCHEMA_VERSION, PolyPanelRegistryMaterialization, PolyPanelRegistrySlot,
    PolyPanelRegistrySnapshot, materialize_poly_v1_panel_registry, measure_registered_poly_panel,
    read_poly_panel_registry_snapshot, validate_poly_panel_registry_snapshot,
    write_poly_panel_registry_snapshot,
};
pub use crate::question_bm25_lens::{
    QUESTION_BM25_DIM, QUESTION_BM25_KEY, QuestionBm25Lens, compute_question_bm25_vector,
    question_bm25_text,
};
pub use crate::raw_large_corpus::{
    LARGE_CORPUS_CAPTURE_PASSED, LARGE_CORPUS_SCHEMA_VERSION, LargeCorpusBoundedIncompleteDataset,
    LargeCorpusEdgeCase, LargeCorpusFailure, LargeCorpusManifest, LargeCorpusPage,
    LargeCorpusReadbackReport, LargeCorpusRequest, read_large_corpus_manifest,
    readback_large_corpus, readback_large_corpus_with_exhaustive, require_large_corpus_passed,
    run_large_corpus_capture,
};
pub use crate::raw_large_corpus_ws_semantics::{
    LargeCorpusWebSocketRuntimeSemanticsObservation, LargeCorpusWebSocketRuntimeSemanticsReport,
};
pub use crate::raw_onchain_backfill_readback_scope::OnchainBackfillReadbackScope;
pub use crate::raw_onchain_backfill_runner::{
    OnchainBackfillRunOutcome, run_onchain_backfill_once, run_onchain_backfill_once_with_readback,
    run_onchain_backfill_once_with_readback_scope,
};
pub use crate::raw_onchain_backfill_runner_readback::{
    ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_FILE,
    ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PROGRESS_FILE, ONCHAIN_BACKFILL_READBACK_FILE,
    ONCHAIN_BACKFILL_READBACK_PROGRESS_FILE, OnchainBackfillReadbackReport,
    readback_onchain_backfill_run, readback_onchain_backfill_run_scoped,
    require_onchain_backfill_readback_passed,
};
pub use crate::raw_onchain_backfill_runner_types::{
    ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE, ONCHAIN_BACKFILL_RUN_PASSED,
    ONCHAIN_BACKFILL_RUN_REPORT_FILE, ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION,
    OnchainBackfillContractRun, OnchainBackfillRunReport, OnchainBackfillRunRequest,
};
pub use crate::raw_source_readback::{
    ERR_RAW_SOURCE_READBACK_FAILED, RAW_SOURCE_READBACK_FILE, RAW_SOURCE_READBACK_PASSED,
    RAW_SOURCE_READBACK_SCHEMA_VERSION, RawSourceReadbackFile, RawSourceReadbackReport,
    readback_raw_source_inventory, require_raw_source_readback_passed,
};
pub use crate::raw_sources::{
    RAW_SOURCE_INVENTORY_SCHEMA_VERSION, RAW_SOURCE_SAMPLE_PASSED, RawDocsCoverageRow,
    RawDocsIndexCoverage, RawDocsIndexSnapshot, RawEndpointSample, RawFileState, RawJoinMap,
    RawRuntimeSemanticsObservation, RawSchemaObservation, RawSourceCoverage, RawSourceFailure,
    RawSourceInventory, RawSourceSamplingRequest, RawWebSocketFrameState,
    read_raw_source_inventory, require_raw_source_sampling_passed,
    run_polymarket_raw_source_sampling,
};
pub use crate::regime_detection::{
    ERR_REGIME_DETECTION_INVALID_REQUEST, ERR_REGIME_DETECTION_PROVISIONAL_SOURCE,
    ERR_REGIME_DETECTION_READBACK_MISMATCH, ForecastQualityParameters, MarketRegime,
    REGIME_DETECTION_ARTIFACT_KIND, REGIME_DETECTION_REPORT_FILE, REGIME_DETECTION_SCHEMA_VERSION,
    RegimeDetectionReport, RegimeDetectionRequest, RegimeDetectionRun, RegimeDetectorStatus,
    RegimeObservation, RegimeParameterSet, compute_regime_detection_report,
    read_regime_detection_report, run_regime_detection_report, write_regime_detection_report,
};
pub use crate::region_vocab::{
    ERR_REGION_VOCAB_INVALID_REQUEST, ERR_REGION_VOCAB_READBACK_MISMATCH,
    REGION_VOCAB_ARTIFACT_KIND, REGION_VOCAB_FILE, REGION_VOCAB_SCHEMA_VERSION,
    RegionRejectedRecord, RegionTextRecord, RegionVocabEntry, RegionVocabReport,
    RegionVocabRequest, RegionVocabRun, build_region_vocab_report, read_region_vocab_report,
    region_vocab_for_domain, run_region_vocab_report, write_region_vocab_report,
};
pub use crate::reproducibility::{
    AGENT_REPRODUCTION_BIT_FOR_BIT, AGENT_REPRODUCTION_SCHEMA_VERSION,
    AgentForecastReproductionReport, AgentForecastReproductionRequest, FileReproductionComparison,
    read_agent_reproduction_report, reproduce_agent_forecast_artifacts,
    write_agent_reproduction_report,
};
pub use crate::risk::{MarketIntegrityParams, MarketIntegrityScreen, screen_market_integrity};
pub use crate::schema_derivation::{
    SCHEMA_DERIVATION_PASSED, SCHEMA_DERIVATION_SCHEMA_VERSION, SchemaArtifactFileState,
    SchemaBlockedRuntimeSource, SchemaContract, SchemaDatasetContract, SchemaDerivationFailure,
    SchemaDerivationReport, SchemaDerivationRequest, SchemaEdgeAudit, SchemaEdgeCheck,
    SchemaFieldContract, SchemaJoinContract, read_schema_derivation_report,
    require_schema_derivation_passed, run_schema_derivation,
};
pub use crate::score::{
    CalibrationBin, FORECAST_SCORE_SCHEMA_VERSION, ForecastScoreManifest, ForecastScoreMetrics,
    ForecastScoreRequest, ForecastSource, ResolvedOutcome, write_forecast_score_artifacts,
};
pub use crate::seed_registry::{
    ARB_RESIDUAL_RFF, DISTANCE_FROM_50_RFF, ERR_SEED_REGISTRY_COLLISION, ERR_SEED_REGISTRY_INVALID,
    ERR_SEED_REGISTRY_MISSING, ERR_SEED_REGISTRY_READBACK_MISMATCH, FROZEN_RFF_SEED_SPECS,
    FrozenEncoderKind, FrozenRffSeedSpec, MOMENTUM_RFF, OFI_VEC_RFF, PRICE_RFF,
    REQUIRED_RFF_LENS_KEYS, SEED_REGISTRY_ARTIFACT_KIND, SEED_REGISTRY_FILE,
    SEED_REGISTRY_SCHEMA_VERSION, SEED_REGISTRY_VERSION, SPREAD_RFF, SeedRegistryArtifact,
    SeedRegistryEntry, SeedRegistryRun, SeedRegistryValidation, default_seed_registry_artifact,
    read_seed_registry, run_seed_registry_readback, seed_spec_for_lens,
    validate_seed_registry_artifact, write_seed_registry,
};
pub use crate::self_evolution_guardrails::{
    ERR_SELF_EVOLUTION_INVALID_REQUEST, ERR_SELF_EVOLUTION_MISSING_REPRODUCTION,
    ERR_SELF_EVOLUTION_MISSING_ROLLBACK, ERR_SELF_EVOLUTION_READBACK_MISMATCH,
    ERR_SELF_EVOLUTION_TRIPWIRE, SELF_EVOLUTION_GUARDRAIL_ARTIFACT_KIND,
    SELF_EVOLUTION_GUARDRAIL_REPORT_FILE, SELF_EVOLUTION_GUARDRAIL_SCHEMA_VERSION,
    SelfEvolutionGuardrailReport, SelfEvolutionGuardrailRequest, SelfEvolutionGuardrailRun,
    SelfEvolutionMetrics, SelfEvolutionStatus, SelfEvolutionTripwireCheck, SelfEvolutionTripwires,
    compute_self_evolution_guardrail, read_self_evolution_guardrail_report,
    require_self_evolution_approved, run_self_evolution_guardrail,
};
pub use crate::service_scaffold::{
    ERR_SERVICE_SCAFFOLD_FORBIDDEN, ERR_SERVICE_SCAFFOLD_MALFORMED,
    ERR_SERVICE_SCAFFOLD_MISSING_CONFIG, ERR_SERVICE_SCAFFOLD_READBACK_MISMATCH,
    LocalServiceConfig, LocalServiceKind, LocalServiceState, SERVICE_SCAFFOLD_ARTIFACT_KIND,
    SERVICE_SCAFFOLD_MANIFEST_FILE, SERVICE_SCAFFOLD_SCHEMA_VERSION, SERVICE_SCHEDULER_STATE_FILE,
    SchedulerJobState, SchedulerState, ServiceScaffoldManifest, ServiceScaffoldRequest,
    ServiceScaffoldRun, default_local_service_configs, read_service_scaffold_manifest,
    run_service_scaffold,
};
pub use crate::strategy_learning::{
    ERR_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE, ERR_STRATEGY_LEARNING_INVALID_REQUEST,
    ERR_STRATEGY_LEARNING_LOOKAHEAD, ERR_STRATEGY_LEARNING_NO_PROMOTION,
    ERR_STRATEGY_LEARNING_READBACK_MISMATCH, STRATEGY_LEARNING_ARTIFACT_KIND,
    STRATEGY_LEARNING_MIN_HELDOUT_ROWS, STRATEGY_LEARNING_REPORT_FILE,
    STRATEGY_LEARNING_SCHEMA_VERSION, StrategyCandidateArtifact, StrategyChangeKind,
    StrategyComponentChange, StrategyLearningReport, StrategyLearningRequest, StrategyLearningRun,
    StrategyLearningStatus, StrategyMetricDelta, StrategyScoreRow,
    compute_strategy_learning_report, read_strategy_learning_report,
    require_strategy_learning_promoted, run_strategy_learning_report,
    write_strategy_learning_report,
};
pub use crate::temporal_lens::{
    E2_RECENCY_KEY, E3_PERIODIC_KEY, E4_POSITIONAL_KEY, ERR_TEMPORAL_INVALID, PolyTemporalLens,
    TemporalLensKind, compute_temporal_vector, is_temporal_lens_key, temporal_shape,
};
pub use crate::toxicity_lens::{
    ERR_TOXICITY_INVALID_FILL, ERR_TOXICITY_LOOKAHEAD, TOXICITY_TARGET_BUCKET_COUNT,
    TOXICITY_VECTOR_DIM, ToxicityBucket, ToxicityLens, ToxicityMetrics, compute_toxicity_metrics,
    compute_toxicity_vector,
};
pub use crate::uma_resolution::{
    ERR_UMA_RESOLUTION_INVALID, ERR_UMA_RESOLUTION_LOG_INVALID, ERR_UMA_RESOLUTION_NOT_FINAL,
    ERR_UMA_RESOLUTION_READBACK_MISMATCH, UMA_ONCHAIN_SOURCE, UMA_RESOLUTION_WATCHER_REPORT_FILE,
    UMA_RESOLUTION_WATCHER_SCHEMA_VERSION, UmaFinalityState, UmaResolutionDecision,
    UmaResolutionObservation, UmaResolutionWatcherReport, UmaResolutionWatcherRequest,
    UmaResolutionWatcherRun, compute_uma_resolution_watcher_report,
    decode_condition_resolution_data, evaluate_uma_resolution,
    parse_condition_resolution_log_value, read_uma_resolution_watcher_report,
    require_groundable_uma_resolution, run_uma_resolution_watcher,
    write_uma_resolution_watcher_report,
};
pub use crate::ward_calibration::{
    ERR_WARD_CALIBRATION_INSUFFICIENT_ANCHORS, ERR_WARD_CALIBRATION_INVALID_REQUEST,
    ERR_WARD_CALIBRATION_MALFORMED_RESIDUAL, ERR_WARD_CALIBRATION_READBACK_MISMATCH,
    ERR_WARD_CALIBRATION_STALE, POLY_WARD_ADMISSION_LEDGER_SCHEMA_VERSION,
    POLY_WARD_CALIBRATION_ARTIFACT_KIND, POLY_WARD_CALIBRATION_SCHEMA_VERSION, WardAdmissionLedger,
    WardCalibrationMetaSummary, WardCalibrationReport, WardCalibrationRequest,
    WardCalibrationResidual, WardCalibrationRun, WardResidualClass,
    apply_ward_calibration_to_admission, compute_ward_calibration_report,
    read_ward_calibration_report, run_ward_calibration_report, write_ward_calibration_report,
};
pub use crate::wash::{WashTradeParams, WashTradeScreen, screen_wash_trading};
pub use crate::ws_market_client::{
    MarketWsCaptureSession, MarketWsClient, MarketWsFrameRecord, require_market_ws_session_data,
};
pub use crate::ws_market_parse::parse_market_ws_text;
pub use crate::ws_market_report::{
    MarketWsCaptureReport, MarketWsFileReadback, MarketWsProofContext,
    read_market_ws_capture_report, write_market_ws_capture_report,
};
pub use crate::ws_market_types::{
    ERR_WS_MARKET_BODY_LIMIT, ERR_WS_MARKET_CONNECT, ERR_WS_MARKET_EVENT_INVALID,
    ERR_WS_MARKET_JSON, ERR_WS_MARKET_NO_PAYLOAD_WINDOW, ERR_WS_MARKET_READ,
    ERR_WS_MARKET_READBACK_MISMATCH, ERR_WS_MARKET_REQUEST_INVALID, ERR_WS_MARKET_SEND,
    ERR_WS_MARKET_SESSION_INCOMPLETE, MARKET_WS_ARTIFACT_KIND, MARKET_WS_DOCS_URL,
    MARKET_WS_REPORT_FILE, MARKET_WS_SCHEMA_VERSION, MARKET_WS_URL, MarketWsBestBidAsk,
    MarketWsBook, MarketWsClientConfig, MarketWsControlMessage, MarketWsLastTradePrice,
    MarketWsLifecycleEvent, MarketWsParsedEvent, MarketWsPriceChange, MarketWsPriceChangeLevel,
    MarketWsSubscription, MarketWsTextEnvelope, MarketWsTickSizeChange, MarketWsUnknownEvent,
    validate_market_ws_config, validate_subscription,
};
pub use crypto::*;
