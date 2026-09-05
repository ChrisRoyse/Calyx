use crate::raw_source_probes::{dynamic_probes, edge_probes};
use crate::raw_sources::RawJoinMap;

#[test]
fn issue1394_dynamic_probe_identifiers_cannot_inject_url_parameters_or_paths() {
    let join = RawJoinMap {
        token_id: Some("token&side=SELL /".to_string()),
        opposite_token_id: Some("opposite?raw=true".to_string()),
        condition_id: Some("condition/../?limit=0".to_string()),
        trade_user_address: Some("user&limit=0".to_string()),
        event_id: Some("event#fragment".to_string()),
        ..RawJoinMap::default()
    };
    let probes = dynamic_probes(&join);
    assert_url(
        &probes,
        "clob_book_by_token",
        "https://clob.polymarket.com/book?token_id=token%26side%3DSELL%20%2F",
    );
    assert_url(
        &probes,
        "clob_market_info_by_condition",
        "https://clob.polymarket.com/clob-markets/condition%2F..%2F%3Flimit%3D0",
    );
    assert_url(
        &probes,
        "data_positions_by_user",
        "https://data-api.polymarket.com/positions?user=user%26limit%3D0&limit=25",
    );
    assert_url(
        &probes,
        "gamma_comments_by_event",
        "https://gamma-api.polymarket.com/comments?limit=25&parent_entity_type=Event&parent_entity_id=event%23fragment",
    );
    assert_url(
        &edge_probes(&join),
        "edge_data_holders_zero_limit",
        "https://data-api.polymarket.com/holders?market=condition%2F..%2F%3Flimit%3D0&limit=0",
    );
    let post = probes
        .iter()
        .find(|probe| probe.name == "clob_post_books_by_tokens")
        .unwrap();
    assert_eq!(
        post.request_body.as_ref().unwrap(),
        &serde_json::json!([
            {"token_id": "token&side=SELL /"},
            {"token_id": "opposite?raw=true"}
        ])
    );
    let edge_post = edge_probes(&join)
        .into_iter()
        .find(|probe| probe.name == "edge_clob_post_books_object_payload")
        .unwrap();
    assert_eq!(
        edge_post.request_body.as_ref().unwrap(),
        &serde_json::json!({"token_id": "token&side=SELL /"})
    );
}

fn assert_url(probes: &[crate::raw_source_probes::Probe], name: &str, expected: &str) {
    let probe = probes.iter().find(|probe| probe.name == name).unwrap();
    assert_eq!(probe.url, expected);
}
