use crate::raw_clob_post_probes::{add_clob_batch_probes, clob_batch_edge_probes};
pub(crate) use crate::raw_source_probe_docs::docs;
use crate::raw_sources::RawJoinMap;
use crate::raw_url::encode_component;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct Probe {
    pub name: String,
    pub source: String,
    pub endpoint: String,
    pub method: String,
    pub url: String,
    pub docs_url: String,
    pub request_body: Option<Value>,
    pub expected_success: bool,
    pub edge_case: bool,
    pub expect_json: bool,
}

pub(crate) fn initial_probes() -> Vec<Probe> {
    vec![
        probe(
            "gamma_markets_active",
            "gamma",
            "markets",
            "https://gamma-api.polymarket.com/markets?active=true&closed=false&limit=25",
            "https://docs.polymarket.com/api-reference/markets/list-markets",
            true,
            false,
        ),
        probe(
            "gamma_markets_closed",
            "gamma",
            "markets",
            "https://gamma-api.polymarket.com/markets?closed=true&limit=25",
            "https://docs.polymarket.com/api-reference/markets/list-markets",
            true,
            false,
        ),
        probe(
            "gamma_events_active",
            "gamma",
            "events",
            "https://gamma-api.polymarket.com/events?active=true&closed=false&limit=25",
            "https://docs.polymarket.com/api-reference/events/list-events",
            true,
            false,
        ),
        probe(
            "gamma_tags",
            "gamma",
            "tags",
            "https://gamma-api.polymarket.com/tags?limit=100",
            "https://docs.polymarket.com/api-reference/tags/list-tags",
            true,
            false,
        ),
        probe(
            "gamma_series",
            "gamma",
            "series",
            "https://gamma-api.polymarket.com/series?limit=25",
            "https://docs.polymarket.com/api-reference/series/list-series",
            true,
            false,
        ),
        probe(
            "gamma_public_search_bitcoin",
            "gamma",
            "public-search",
            "https://gamma-api.polymarket.com/public-search?q=bitcoin&limit_per_type=5",
            "https://docs.polymarket.com/api-reference/search/search-markets-events-and-profiles",
            true,
            false,
        ),
        probe(
            "clob_sampling_markets",
            "clob",
            "sampling-markets",
            "https://clob.polymarket.com/sampling-markets",
            "https://docs.polymarket.com/api-reference/markets/get-sampling-markets",
            true,
            false,
        ),
        probe(
            "clob_simplified_markets",
            "clob",
            "simplified-markets",
            "https://clob.polymarket.com/simplified-markets",
            "https://docs.polymarket.com/api-reference/markets/get-simplified-markets",
            true,
            false,
        ),
        probe(
            "clob_server_time",
            "clob",
            "time",
            "https://clob.polymarket.com/time",
            "https://docs.polymarket.com/api-reference/data/get-server-time",
            true,
            false,
        ),
        probe(
            "data_trades",
            "data-api",
            "trades",
            "https://data-api.polymarket.com/trades?limit=25",
            "https://docs.polymarket.com/api-reference/core/get-trades-for-a-user-or-markets",
            true,
            false,
        ),
        probe(
            "combo_markets_active",
            "combo-markets",
            "v1/rfq/combo-markets",
            "https://combos-rfq-api.polymarket.com/v1/rfq/combo-markets?limit=5",
            "https://docs.polymarket.com/api-reference/combo-markets/get-combo-markets",
            true,
            false,
        ),
        probe(
            "rewards_current_active",
            "rewards",
            "rewards/markets/current",
            "https://clob.polymarket.com/rewards/markets/current?sponsored=false",
            "https://docs.polymarket.com/api-reference/rewards/get-current-active-rewards-configurations",
            true,
            false,
        ),
        probe(
            "rewards_markets_multi",
            "rewards",
            "rewards/markets/multi",
            "https://clob.polymarket.com/rewards/markets/multi?limit=5",
            "https://docs.polymarket.com/api-reference/rewards/get-multiple-markets-with-rewards",
            true,
            false,
        ),
        probe(
            "rebates_current_real_maker",
            "rewards",
            "rebates/current",
            "https://clob.polymarket.com/rebates/current?date=2026-07-03&maker_address=0xfd8e46519d0a8f9c35e5010ef4e7f56f7583aea4",
            "https://docs.polymarket.com/api-reference/rebates/get-current-rebated-fees-for-a-maker",
            true,
            false,
        ),
    ]
}

pub(crate) fn dynamic_probes(join: &RawJoinMap) -> Vec<Probe> {
    let mut probes = Vec::new();
    if let Some(token) = &join.token_id {
        let encoded_token = encode_component(token);
        probes.push(probe(
            "clob_book_by_token",
            "clob",
            "book",
            format!("https://clob.polymarket.com/book?token_id={encoded_token}"),
            "https://docs.polymarket.com/api-reference/market-data/get-order-book",
            true,
            false,
        ));
        probes.push(probe(
            "clob_price_buy_by_token",
            "clob",
            "price",
            format!("https://clob.polymarket.com/price?token_id={encoded_token}&side=BUY"),
            "https://docs.polymarket.com/api-reference/market-data/get-market-price",
            true,
            false,
        ));
        probes.push(probe(
            "clob_price_sell_by_token",
            "clob",
            "price",
            format!("https://clob.polymarket.com/price?token_id={encoded_token}&side=SELL"),
            "https://docs.polymarket.com/api-reference/market-data/get-market-price",
            true,
            false,
        ));
        probes.push(probe(
            "clob_midpoint_by_token",
            "clob",
            "midpoint",
            format!("https://clob.polymarket.com/midpoint?token_id={encoded_token}"),
            "https://docs.polymarket.com/api-reference/data/get-midpoint-price",
            true,
            false,
        ));
        probes.push(probe(
            "clob_spread_by_token",
            "clob",
            "spread",
            format!("https://clob.polymarket.com/spread?token_id={encoded_token}"),
            "https://docs.polymarket.com/api-reference/market-data/get-spread",
            true,
            false,
        ));
        probes.push(probe(
            "clob_last_trade_by_token",
            "clob",
            "last-trade-price",
            format!("https://clob.polymarket.com/last-trade-price?token_id={encoded_token}"),
            "https://docs.polymarket.com/api-reference/market-data/get-last-trade-price",
            true,
            false,
        ));
        probes.push(probe(
            "clob_tick_size_by_token",
            "clob",
            "tick-size",
            format!("https://clob.polymarket.com/tick-size?token_id={encoded_token}"),
            "https://docs.polymarket.com/api-reference/market-data/get-tick-size",
            true,
            false,
        ));
        probes.push(probe("clob_prices_history_by_token", "clob", "prices-history", format!("https://clob.polymarket.com/prices-history?market={encoded_token}&interval=1d&fidelity=1440"), "https://docs.polymarket.com/api-reference/markets/get-prices-history", true, false));
        let mut tokens = vec![token.clone()];
        if let Some(opposite) = &join.opposite_token_id {
            tokens.push(opposite.clone());
        }
        add_clob_batch_probes(&mut probes, &tokens);
    }
    if let Some(condition) = &join.condition_id {
        let condition = encode_component(condition);
        probes.push(probe(
            "clob_market_info_by_condition",
            "clob",
            "clob-markets/{condition_id}",
            format!("https://clob.polymarket.com/clob-markets/{condition}"),
            "https://docs.polymarket.com/api-reference/markets/get-clob-market-info",
            true,
            false,
        ));
        probes.push(probe(
            "data_holders_by_market",
            "data-api",
            "holders",
            format!("https://data-api.polymarket.com/holders?market={condition}&limit=5"),
            "https://docs.polymarket.com/api-reference/core/get-top-holders-for-markets",
            true,
            false,
        ));
        probes.push(probe(
            "data_oi_by_market",
            "data-api",
            "oi",
            format!("https://data-api.polymarket.com/oi?market={condition}"),
            "https://docs.polymarket.com/api-reference/misc/get-open-interest",
            true,
            false,
        ));
        probes.push(probe(
            "rewards_raw_by_condition",
            "rewards",
            "rewards/markets/{condition_id}",
            format!("https://clob.polymarket.com/rewards/markets/{condition}"),
            "https://docs.polymarket.com/api-reference/rewards/get-raw-rewards-for-a-specific-market",
            true,
            false,
        ));
    }
    if let Some(user) = &join.trade_user_address {
        let user = encode_component(user);
        probes.push(probe(
            "data_positions_by_user",
            "data-api",
            "positions",
            format!("https://data-api.polymarket.com/positions?user={user}&limit=25"),
            "https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user",
            true,
            false,
        ));
        probes.push(probe(
            "data_activity_by_user",
            "data-api",
            "activity",
            format!("https://data-api.polymarket.com/activity?user={user}&limit=25"),
            "https://docs.polymarket.com/api-reference/core/get-user-activity",
            true,
            false,
        ));
        probes.push(probe(
            "data_total_traded_by_user",
            "data-api",
            "traded",
            format!("https://data-api.polymarket.com/traded?user={user}"),
            "https://docs.polymarket.com/api-reference/misc/get-total-markets-a-user-has-traded",
            true,
            false,
        ));
        probes.push(binary_probe(
            "data_accounting_snapshot_by_user",
            "data-api",
            "v1/accounting/snapshot",
            format!("https://data-api.polymarket.com/v1/accounting/snapshot?user={user}"),
            "https://docs.polymarket.com/api-reference/misc/download-an-accounting-snapshot-zip-of-csvs",
            true,
            false,
        ));
    }
    if let Some(event_id) = &join.event_id {
        let event_id = encode_component(event_id);
        probes.push(probe("gamma_comments_by_event", "gamma", "comments", format!("https://gamma-api.polymarket.com/comments?limit=25&parent_entity_type=Event&parent_entity_id={event_id}"), "https://docs.polymarket.com/api-reference/comments/list-comments", true, false));
        probes.push(probe(
            "data_live_volume_by_event",
            "data-api",
            "live-volume",
            format!("https://data-api.polymarket.com/live-volume?id={event_id}"),
            "https://docs.polymarket.com/api-reference/misc/get-live-volume-for-an-event",
            true,
            false,
        ));
    }
    probes
}

pub(crate) fn edge_probes(join: &RawJoinMap) -> Vec<Probe> {
    let mut probes = vec![
        probe(
            "edge_gamma_comments_missing_parent",
            "gamma",
            "comments",
            "https://gamma-api.polymarket.com/comments?limit=1",
            "https://docs.polymarket.com/api-reference/comments/list-comments",
            false,
            true,
        ),
        probe(
            "edge_data_activity_missing_user",
            "data-api",
            "activity",
            "https://data-api.polymarket.com/activity?limit=1",
            "https://docs.polymarket.com/api-reference/core/get-user-activity",
            false,
            true,
        ),
        probe(
            "edge_clob_book_invalid_token",
            "clob",
            "book",
            "https://clob.polymarket.com/book?token_id=not-a-real-token",
            "https://docs.polymarket.com/api-reference/market-data/get-order-book",
            false,
            true,
        ),
        probe(
            "edge_gamma_markets_zero_limit",
            "gamma",
            "markets",
            "https://gamma-api.polymarket.com/markets?limit=0",
            "https://docs.polymarket.com/api-reference/markets/list-markets",
            true,
            true,
        ),
        probe(
            "edge_combo_markets_limit_zero",
            "combo-markets",
            "v1/rfq/combo-markets",
            "https://combos-rfq-api.polymarket.com/v1/rfq/combo-markets?limit=0",
            "https://docs.polymarket.com/api-reference/combo-markets/get-combo-markets",
            false,
            true,
        ),
        probe(
            "edge_data_live_volume_invalid_id",
            "data-api",
            "live-volume",
            "https://data-api.polymarket.com/live-volume?id=0",
            "https://docs.polymarket.com/api-reference/misc/get-live-volume-for-an-event",
            false,
            true,
        ),
        probe(
            "edge_data_accounting_snapshot_invalid_user",
            "data-api",
            "v1/accounting/snapshot",
            "https://data-api.polymarket.com/v1/accounting/snapshot?user=not-an-address",
            "https://docs.polymarket.com/api-reference/misc/download-an-accounting-snapshot-zip-of-csvs",
            false,
            true,
        ),
        probe(
            "edge_rewards_current_invalid_cursor",
            "rewards",
            "rewards/markets/current",
            "https://clob.polymarket.com/rewards/markets/current?next_cursor=not-a-cursor",
            "https://docs.polymarket.com/api-reference/rewards/get-current-active-rewards-configurations",
            false,
            true,
        ),
        probe(
            "edge_rebates_current_invalid_maker",
            "rewards",
            "rebates/current",
            "https://clob.polymarket.com/rebates/current?date=not-a-date&maker_address=not-an-address",
            "https://docs.polymarket.com/api-reference/rebates/get-current-rebated-fees-for-a-maker",
            false,
            true,
        ),
    ];
    if let Some(condition) = &join.condition_id {
        let condition = encode_component(condition);
        probes.push(probe(
            "edge_data_holders_zero_limit",
            "data-api",
            "holders",
            format!("https://data-api.polymarket.com/holders?market={condition}&limit=0"),
            "https://docs.polymarket.com/api-reference/core/get-top-holders-for-markets",
            true,
            true,
        ));
    }
    if let Some(token) = &join.token_id {
        probes.extend(clob_batch_edge_probes(token));
    }
    probes
}

fn probe(
    name: impl Into<String>,
    source: impl Into<String>,
    endpoint: impl Into<String>,
    url: impl Into<String>,
    docs_url: impl Into<String>,
    expected_success: bool,
    edge_case: bool,
) -> Probe {
    Probe {
        name: name.into(),
        source: source.into(),
        endpoint: endpoint.into(),
        method: "GET".to_string(),
        url: url.into(),
        docs_url: docs_url.into(),
        request_body: None,
        expected_success,
        edge_case,
        expect_json: true,
    }
}

fn binary_probe(
    name: impl Into<String>,
    source: impl Into<String>,
    endpoint: impl Into<String>,
    url: impl Into<String>,
    docs_url: impl Into<String>,
    expected_success: bool,
    edge_case: bool,
) -> Probe {
    Probe {
        name: name.into(),
        source: source.into(),
        endpoint: endpoint.into(),
        method: "GET".to_string(),
        url: url.into(),
        docs_url: docs_url.into(),
        request_body: None,
        expected_success,
        edge_case,
        expect_json: false,
    }
}
