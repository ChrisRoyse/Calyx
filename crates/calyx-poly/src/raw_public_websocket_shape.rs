use serde_json::Value;

pub(crate) fn public_json_shape(value: &Value) -> (Option<String>, Vec<String>) {
    match value {
        Value::Object(map) => {
            let event_type = if let (Some(topic), Some(kind)) = (
                map.get("topic").and_then(Value::as_str),
                map.get("type").and_then(Value::as_str),
            ) {
                Some(format!("{topic}:{kind}"))
            } else if sports_result_shape(map) {
                Some("sport_result".to_string())
            } else if map.contains_key("statusCode") {
                Some("rtds_error".to_string())
            } else {
                None
            };
            (event_type, map.keys().cloned().collect())
        }
        Value::Array(items) => items
            .first()
            .and_then(Value::as_object)
            .map(|map| (None, map.keys().cloned().collect()))
            .unwrap_or((None, Vec::new())),
        _ => (None, Vec::new()),
    }
}

fn sports_result_shape(map: &serde_json::Map<String, Value>) -> bool {
    if map.contains_key("gameId") || map.contains_key("slug") {
        return true;
    }
    if !map.contains_key("metadataGameId") {
        return false;
    }
    let state_fields = [
        "leagueAbbreviation",
        "homeTeam",
        "awayTeam",
        "status",
        "score",
        "period",
        "elapsed",
        "live",
        "ended",
        "finishedTimestamp",
        "finished_timestamp",
        "eventState",
        "turn",
        "last_update",
    ];
    state_fields
        .iter()
        .filter(|field| map.contains_key(**field))
        .count()
        >= 2
}

#[cfg(test)]
mod tests {
    use super::public_json_shape;

    fn assert_shape(case: &str, payload: serde_json::Value, expected: Option<&str>) -> Vec<String> {
        let before = serde_json::json!({
            "case": case,
            "payload": payload,
            "expected_event_type": expected
        });
        let (event_type, fields) = public_json_shape(&before["payload"]);
        println!(
            "{}",
            serde_json::json!({
                "case": case,
                "before": before,
                "after": {
                    "event_type": event_type,
                    "fields": fields,
                }
            })
        );
        assert_eq!(event_type.as_deref(), expected);
        fields
    }

    #[test]
    fn observed_metadata_game_id_payload_is_sports_result() {
        let fields = assert_shape(
            "observed_metadata_game_id_payload",
            serde_json::json!({
                "metadataGameId": "id2703659172410984",
                "leagueAbbreviation": "cricket",
                "score": "54-143",
                "period": "FT",
                "live": false,
                "ended": true,
                "finishedTimestamp": "2026-07-04T05:32:56.71547577Z"
            }),
            Some("sport_result"),
        );
        assert!(fields.iter().any(|field| field == "metadataGameId"));
    }

    #[test]
    fn generic_metadata_json_is_not_a_sports_result() {
        assert_shape(
            "generic_metadata_game_id_payload",
            serde_json::json!({
                "metadataGameId": "id2703659172410984",
                "message": "not a sports score"
            }),
            None,
        );
    }

    #[test]
    fn rtds_status_code_payload_remains_error() {
        assert_shape(
            "rtds_status_code_error_payload",
            serde_json::json!({
                "statusCode": 400,
                "message": "subscription not available"
            }),
            Some("rtds_error"),
        );
    }

    #[test]
    fn topic_type_payload_keeps_topic_kind_event_type() {
        assert_shape(
            "topic_type_payload",
            serde_json::json!({
                "topic": "rtds",
                "type": "equity_prices",
                "ticker": "AAPL"
            }),
            Some("rtds:equity_prices"),
        );
    }
}
