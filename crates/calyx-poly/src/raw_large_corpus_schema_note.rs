use std::fs;
use std::path::{Path, PathBuf};

use crate::raw_large_corpus::{LargeCorpusBoundedIncompleteDataset, LargeCorpusPage};
use crate::raw_large_corpus_onchain_backfill::OnchainBackfillState;
use crate::raw_large_corpus_profile::{LargeCorpusFieldProfile, LargeCorpusJoinProfile};
use crate::raw_large_corpus_trade_history::LargeCorpusTradeHistoryState;
use crate::raw_large_corpus_ws_semantics::LargeCorpusWebSocketRuntimeSemanticsObservation;
use crate::{PolyError, Result};

pub(crate) struct BackfillSchemaStates<'a> {
    pub(crate) trade_history: &'a LargeCorpusTradeHistoryState,
    pub(crate) onchain: &'a OnchainBackfillState,
}

pub(crate) fn write_schema_decision_input(
    root: &Path,
    profiles: &[LargeCorpusFieldProfile],
    join_profile: &LargeCorpusJoinProfile,
    pages: &[LargeCorpusPage],
    websocket_runtime_semantics: &[LargeCorpusWebSocketRuntimeSemanticsObservation],
    backfill_states: BackfillSchemaStates<'_>,
    bounded_incomplete_datasets: &[LargeCorpusBoundedIncompleteDataset],
) -> Result<PathBuf> {
    let path = root.join("schema-decision-input.md");
    let mut text = String::new();
    text.push_str("# Large Corpus Schema Decision Input\n\n");
    text.push_str("Source of truth: live public/read-only Polymarket responses persisted under this artifact root, then read back from disk.\n\n");
    text.push_str("## Capture Summary\n\n");
    text.push_str(&format!("- Datasets: {}\n", profiles.len()));
    text.push_str(&format!("- Pages: {}\n", pages.len()));
    text.push_str(&format!(
        "- Records: {}\n",
        pages.iter().map(|page| page.record_count).sum::<usize>()
    ));
    text.push_str("\n## Backfill Completeness\n\n");
    if bounded_incomplete_datasets.is_empty() {
        text.push_str("- No paginated dataset hit the configured page cap without a terminal page in this run.\n");
    } else {
        text.push_str("- This run is bounded, not exhaustive, for these datasets:\n");
        for dataset in bounded_incomplete_datasets {
            text.push_str(&format!(
                "  - `{}` (`{}`): {} pages, last page {} had {} records; reason `{}`.\n",
                dataset.dataset,
                dataset.source,
                dataset.page_count,
                dataset.last_page_index,
                dataset.last_record_count,
                dataset.reason
            ));
        }
        text.push_str("- Run with exhaustive requirements must fail closed until these datasets reach a terminal page or have explicit source-specific completion evidence.\n");
    }
    text.push_str("\n## Dataset Profiles\n\n");
    for profile in profiles {
        text.push_str(&format!(
            "- `{}` (`{}`): {} records, {} observed top-level fields.\n",
            profile.dataset,
            profile.source,
            profile.record_count,
            profile.fields.len()
        ));
    }
    text.push_str("\n## Join Evidence\n\n");
    for (label, count) in &join_profile.identifier_counts {
        text.push_str(&format!("- `{label}` observed `{count}` times.\n"));
    }
    text.push_str("\n## Schema Rules\n\n");
    text.push_str("- Preserve raw payload path, body SHA256, source, endpoint, query, page index, and capture timestamp before any normalized projection.\n");
    text.push_str("- Model Gamma events and markets as separate raw families because both carry overlapping but non-identical market/event identifiers.\n");
    append_gamma_rules(&mut text, pages);
    append_clob_rules(&mut text, profiles);
    append_websocket_rules(&mut text, profiles);
    append_websocket_runtime_rules(&mut text, websocket_runtime_semantics);
    append_archive_onchain_rules(&mut text, profiles);
    text.push_str("- Treat Data API `/trades` as a bounded activity window, not the complete all-time trade source.\n");
    text.push_str("- Use public Polygon `OrderFilled` logs as the durable trade-history source family, with deployment-to-latest range completion and dedupe proof required before claiming all-trade completeness.\n");
    text.push_str(&format!(
        "- OrderFilled dedupe rule: {}; join-key rule: {}.\n",
        backfill_states
            .trade_history
            .onchain_order_filled_logs
            .dedupe_key_rule,
        backfill_states
            .trade_history
            .onchain_order_filled_logs
            .join_key_rule
    ));
    text.push_str(&format!(
        "- Trade-history source state artifact: `{}`; status `{}`; all_trade_history_complete `{}`.\n",
        root.join("trade-history-source-state.json").display(),
        backfill_states.trade_history.status_code,
        backfill_states.trade_history.all_trade_history_complete
    ));
    text.push_str(&format!(
        "- On-chain backfill state artifact: `{}`; status `{}`; latest_safe_block `{}`; contract count `{}`.\n",
        root.join("onchain-backfill-state.json").display(),
        backfill_states.onchain.status_code,
        backfill_states.onchain.latest_safe_block,
        backfill_states.onchain.contracts.len()
    ));
    text.push_str("- Treat JSON-string fields as first-class decode targets only after retaining the exact original string bytes.\n");
    text.push_str(
        "- Keep RTDS equity payload schema blocked under #179 until real payload bytes exist.\n",
    );
    fs::write(&path, text).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_SCHEMA_NOTE_WRITE_FAILED",
            format!("write schema decision note {}: {err}", path.display()),
        )
    })?;
    Ok(path)
}

fn append_gamma_rules(text: &mut String, pages: &[LargeCorpusPage]) {
    if pages.iter().any(|page| {
        page.pagination_state
            .as_ref()
            .is_some_and(|state| state.mode == "keyset")
    }) {
        text.push_str("- Preserve Gamma keyset request cursor, response next_cursor, and terminal-cursor state because these fields prove all-data page completion.\n");
    }
}

fn append_clob_rules(text: &mut String, profiles: &[LargeCorpusFieldProfile]) {
    if profiles.iter().any(|profile| profile.source == "clob") {
        text.push_str("- Model CLOB orderbook, single-token price, batch-token price, spread, midpoint, tick-size, market-info, and history payloads as separate raw families before projection.\n");
        text.push_str("- Preserve POST request bodies and request SHA256 values for CLOB batch endpoints because request shape controls response shape.\n");
        text.push_str("- Treat CLOB token-keyed maps as dynamic association inputs, not fixed table columns.\n");
    }
}

fn append_websocket_rules(text: &mut String, profiles: &[LargeCorpusFieldProfile]) {
    if profiles
        .iter()
        .any(|profile| profile.source.starts_with("websocket-"))
    {
        text.push_str("- Model WebSocket captures as windowed frame streams with separate raw-frame retention, parsed JSON values, event types, and no-payload-window semantics.\n");
        text.push_str("- Keep market-channel custom lifecycle events separate from asset-scoped book, price, tick-size, and trade frames.\n");
        text.push_str("- Model RTDS equity payloads only from persisted `equity_prices` bytes; keep docs-only corpora blocked-runtime.\n");
    }
}

fn append_websocket_runtime_rules(
    text: &mut String,
    observations: &[LargeCorpusWebSocketRuntimeSemanticsObservation],
) {
    if observations.is_empty() {
        return;
    }
    text.push_str("\n## WebSocket Runtime Semantics\n\n");
    for observation in observations {
        text.push_str(&format!(
            "- `{}`: {} -> {}; schema implication: {}\n",
            observation.sample_name,
            observation.request_case,
            observation.expected_runtime_semantics,
            observation.schema_implication
        ));
    }
    text.push_str("- Treat WebSocket control semantics as runtime-observed source data; do not infer quiet/no-op behavior from operation names alone.\n");
}

fn append_archive_onchain_rules(text: &mut String, profiles: &[LargeCorpusFieldProfile]) {
    if profiles
        .iter()
        .any(|profile| profile.source == "historical-dump")
    {
        text.push_str("- Preserve historical dataset bytes by format before extraction; JSONL, README text, manifest JSON, and Parquet samples are different source families.\n");
        text.push_str("- Treat public historical datasets as fixed external archives with their own license, coverage period, and known limitations, not as live Polymarket API state.\n");
    }
    if profiles
        .iter()
        .any(|profile| matches!(profile.source.as_str(), "polygon-rpc" | "goldsky-subgraph"))
    {
        text.push_str("- Model Polygon RPC requests and Goldsky GraphQL queries with persisted request bodies and response hashes because query shape controls event coverage.\n");
        text.push_str("- Treat legacy public Goldsky subgraphs as historical/limited after Polymarket's v2 migration; do not infer complete current on-chain coverage from them.\n");
    }
}
