//! # calyx-poly — local Polymarket forecasting on Calyx
//!
//! `calyx-poly` is the domain crate that turns Calyx (the association-native database) into a
//! local-only Polymarket intelligence and forecasting engine. It is the in-workspace home of the
//! design in `docs/prd/` at the repository root.
//!
//! The intelligence lives in the database. This crate provides the *thin, embedder-free*
//! representation and orchestration layer around it:
//!
//! - [`encode`] — deterministic numeric→vector encoders (Random Fourier Features, quantile /
//!   piecewise-linear, one-hot, signed-log). These make cosine meaningful over prices, spreads,
//!   and volumes so Loom's cross-terms and Sextant's kNN operate on numbers, no embedder required.
//! - [`seed_registry`] — frozen RFF lens seed specifications and persisted registry readback.
//! - [`features`] — derived market signals (order-flow imbalance, holder concentration, arbitrage
//!   residuals, distance-from-50, realized volatility).
//! - [`model`] — the Polymarket record types (`MarketSnapshot`, `Resolution`, …) the ingestor
//!   produces from the feeds.
//! - [`lenses`] — the panel of [`lenses::SignalLens`] instances that map a snapshot to typed
//!   slot-vectors, plus the default v1 panel.
//! - [`lens_autobuild`] — converts persisted `propose_lens` deficits into admitted local lens specs.
//! - [`constellation`] — maps a snapshot to a real [`calyx_core::Constellation`] (slots + scalars +
//!   metadata + anchors) and builds resolution anchors.
//! - [`pipeline`] — ingest a snapshot into a [`calyx_core::VaultStore`] and ground it on resolution.
//! - [`admission`] — forecast admission math for local, reproducible forecast artifacts.
//! - [`anneal_integration`] — reversible index/fusion/tau self-tuning through Calyx Anneal shadow gates.
//! - [`policy`] — the local-only runtime boundary that refuses every trading action.
//! - [`raw_sources`] — raw read-only Polymarket source sampling before database schema design.
//! - [`risk`] — thin/manipulable-market screens over holder and maker concentration evidence.
//! - [`wash`] — wash-trade screens over distinct on-chain counterparty volume.
//! - [`kernel_recall`] — per-domain kernel recall gates before predictions are trusted.
//! - [`metrics_scrape`] — local `/metrics` scrape readback and alert evaluation.
//! - [`forecast_observability`] — local forecast-quality metrics and alert reports.
//! - [`historical_backfill_loader`] — terminal/reference historical dump loading without
//!   pre-resolution eligibility.
//! - [`external_kalshi_feed`] — read-only Kalshi external feed capture, encoding, and signal admission.
//! - [`domain_graph_build_job`] — per-domain Loom, Graph CF, CSR, kernel, and recall build job.
//! - [`knn_graph_edges`] — resolved-neighbor kNN edges persisted into Graph CF on ingest.
//! - [`temporal_graph_edges`] — lead/lag, transfer-entropy, periodicity, and hazard edges persisted into Graph CF.
//! - [`service_scaffold`] — local service/scheduler state contract.
//! - [`daily_ops_scheduler`] — local daily scheduler tick for graph/kernel and calibration jobs.
//! - [`self_evolution_guardrails`] — tripwire and rollback gates for self-tuning.
//! - [`meta_learning_ledger`] — append-only audit rows for self-evolution effects.
//! - [`blend_relearning`] — held-out Brier to component reliability weights.
//! - [`calibration_refit`] — versioned domain×horizon calibration slope refits.
//! - [`drift_recalibration`] — drift windows that trigger versioned calibration/admission updates.
//! - [`entity_graph_edges`] — shared holder/maker/counterparty entity edges into Graph CF.
//! - [`mistake_closure`] — resolved forecast-error heads for local corrective proposals.
//! - [`parameter_adaptation`] — scheduled encoder/lag/kNN parameter refits from local data.
//! - [`regime_detection`] — MMD/CUSUM regime detection with per-regime forecast parameters.
//! - [`strategy_learning`] — held-out local forecast-score deltas for strategy promotion.
//! - [`structural_edges`] — deterministic structural/arbitrage edges persisted into Graph CF.
//! - [`domain`] — the domain roster and the **data-density-first** selection strategy (crypto first).
//! - [`config`] — the engine configuration.
//! - [`error`] — the crate error type.
//!
//! ## Principles (inherited from Calyx doctrine)
//! Grounding is mandatory, slots stay un-flattened, and the system fails closed. Every ambiguity
//! resolves to local forecast refusal, never trading.

#![forbid(unsafe_code)]

pub mod admission;
mod admission_checks;
pub mod agent_artifacts;
mod agent_deepseek;
pub mod agent_launcher;
pub mod agent_secrets;
pub mod anchor_floor;
pub mod anneal_integration;
mod anneal_integration_types;
pub mod assay_bits;
pub mod backtest;
pub mod bits_vote;
pub mod blend_relearning;
pub mod book_liquidity;
pub mod book_shape_lens;
pub mod calibration_backtest;
pub mod calibration_refit;
pub mod calyx_native;
pub mod capability_gate;
pub mod clob_client;
mod clob_parse;
mod clob_types;
pub mod config;
pub mod constellation;
pub mod crypto_capture_harness;
mod crypto_capture_harness_types;
pub mod crypto_capture_scheduler;
pub mod crypto_forecast_registration;
pub mod crypto_ingestor;
mod crypto_ingestor_live;
pub mod daily_ops_scheduler;
pub mod data_api_client;
mod data_api_parse;
mod data_api_types;
pub mod derived_feature_stage;
pub mod diagnostics_store;
pub mod domain;
pub mod domain_graph_build_job;
mod domain_graph_build_job_types;
pub mod drift_recalibration;
pub mod edge_audit;
pub mod encode;
pub mod entity_graph_edges;
pub mod error;
pub mod external_kalshi_feed;
mod external_kalshi_feed_parse;
mod external_kalshi_feed_types;
pub mod fanout_selection;
pub mod features;
pub mod feed_outage;
pub mod feedback_controller;
pub mod file_size_lint;
pub mod first_light_audit;
pub mod flow_price_transfer_entropy;
pub mod forecast;
pub mod forecast_blend;
pub mod forecast_calibration;
pub mod forecast_ceiling;
pub mod forecast_observability;
pub mod gamma_client;
pub mod gamma_metadata;
pub mod gamma_public_search;
mod gamma_time;
pub mod grounding;
pub mod historical_backfill_loader;
pub mod historical_point_in_time_replay;
pub mod kernel_forecast;
pub mod kernel_recall;
pub mod kernel_recall_admission;
pub mod knn_base_rate;
pub mod knn_graph_edges;
pub mod lens_autobuild;
pub mod lenses;
pub mod logging;
pub mod loom_shape_weave;
mod loom_shape_weave_types;
pub mod loom_weave;
pub mod meta_learning_ledger;
pub mod metrics_scrape;
pub mod mispricing;
pub mod mistake_closure;
mod mistake_closure_types;
pub mod model;
pub mod no_lookahead;
pub mod onchain_backfill_lease;
pub mod oracle;
pub mod oracle_forecast;
pub mod outcome_backfill;
pub mod pair_gain_gate;
pub mod panel_diagnostics;
pub mod panel_sufficiency;
pub mod parameter_adaptation;
mod parameter_adaptation_math;
mod parameter_adaptation_types;
mod pending_forecast_payload;
pub mod pending_forecast_register;
pub mod pending_forecast_replay;
pub mod pipeline;
pub mod policy;
pub mod policy_audit;
pub mod poly_panel_registry;
pub mod question_bm25_lens;
pub mod rate_limit_governor;
mod raw_clob_post_probes;
mod raw_docs_coverage;
mod raw_docs_coverage_classify;
mod raw_historical;
mod raw_historical_support;
mod raw_http;
pub mod raw_large_corpus;
mod raw_large_corpus_clob;
mod raw_large_corpus_clob_plan;
mod raw_large_corpus_failure;
mod raw_large_corpus_get_request;
mod raw_large_corpus_historical;
mod raw_large_corpus_onchain;
mod raw_large_corpus_onchain_backfill;
mod raw_large_corpus_onchain_backfill_readback;
mod raw_large_corpus_onchain_chunks;
mod raw_large_corpus_onchain_chunks_http;
mod raw_large_corpus_onchain_plans;
mod raw_large_corpus_onchain_specs;
mod raw_large_corpus_pagination_readback;
mod raw_large_corpus_profile;
mod raw_large_corpus_range;
mod raw_large_corpus_readback;
mod raw_large_corpus_readback_range;
mod raw_large_corpus_request;
mod raw_large_corpus_schema_note;
mod raw_large_corpus_support;
mod raw_large_corpus_trade_history;
mod raw_large_corpus_types;
mod raw_large_corpus_websocket;
mod raw_large_corpus_websocket_plan;
mod raw_large_corpus_websocket_support;
mod raw_large_corpus_ws_semantics;
mod raw_onchain;
mod raw_onchain_backfill_checkpoint_validate;
mod raw_onchain_backfill_readback_checks;
mod raw_onchain_backfill_readback_context;
mod raw_onchain_backfill_readback_scope;
mod raw_onchain_backfill_runner;
mod raw_onchain_backfill_runner_readback;
mod raw_onchain_backfill_runner_types;
mod raw_public_websocket;
mod raw_public_websocket_probes;
mod raw_public_websocket_shape;
mod raw_runtime_semantics;
mod raw_source_probes;
pub mod raw_source_readback;
mod raw_source_readme;
mod raw_source_support;
pub mod raw_sources;
mod raw_websocket;
mod raw_websocket_support;
pub mod regime_detection;
pub mod region_vocab;
pub mod reproducibility;
pub mod resolved_market_corpus;
pub mod resolved_market_gamma_loader;
pub mod risk;
pub mod schema_derivation;
mod schema_derivation_classify;
mod schema_derivation_types;
pub mod score;
pub mod seed_registry;
pub mod self_evolution_guardrails;
pub mod service_scaffold;
pub mod strategy_learning;
pub mod structural_edges;
mod structural_edges_types;
pub mod superiority;
pub mod temporal_graph_edges;
mod temporal_graph_edges_types;
pub mod temporal_lens;
pub mod toxicity_lens;
pub mod uma_resolution;
pub mod ward_calibration;
pub mod wash;
pub mod ws_market_client;
mod ws_market_parse;
mod ws_market_report;
mod ws_market_types;

mod exports;
pub use exports::*;
