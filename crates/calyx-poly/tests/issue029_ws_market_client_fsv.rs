use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::{
    ClobSide, ERR_WS_MARKET_JSON, ERR_WS_MARKET_NO_PAYLOAD_WINDOW, GammaClient, GammaClientConfig,
    GammaMarketsRequest, MARKET_WS_DOCS_URL, MARKET_WS_REPORT_FILE, MarketWsCaptureSession,
    MarketWsClient, MarketWsClientConfig, MarketWsControlMessage, MarketWsFrameRecord,
    MarketWsParsedEvent, MarketWsProofContext, MarketWsSubscription, PolyError, Result,
    parse_market_ws_text, read_market_ws_capture_report, require_market_ws_session_data,
    write_market_ws_capture_report,
};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn issue029_ws_market_parser_and_readback_fsv() -> Result<()> {
    let root = fsv_root().join("deterministic");
    assert_c_drive(&root)?;
    let book_raw = json!([{
        "event_type": "book",
        "asset_id": "123",
        "market": "0xmarket",
        "timestamp": "1720000000000",
        "hash": "0xhash",
        "bids": [{"price": ".48", "size": "100"}],
        "asks": [{"price": "0.52", "size": "75"}]
    }])
    .to_string();
    let book_env = parse_market_ws_text(&book_raw)?;
    assert_eq!(book_env.events.len(), 1);
    let MarketWsParsedEvent::Book(book) = &book_env.events[0] else {
        panic!("book frame parsed as wrong event");
    };
    assert_eq!(book.asset_id, "123");
    assert_eq!(book.bids[0].price, 0.48);
    assert_eq!(book.asks[0].size, 75.0);

    let mixed_raw = json!([
        {
            "event_type": "price_change",
            "market": "0xmarket",
            "timestamp": "1720000000001",
            "price_changes": [{
                "asset_id": "123",
                "side": "BUY",
                "price": "0.49",
                "size": "0",
                "best_bid": "0.48",
                "best_ask": "0.52"
            }]
        },
        {
            "event_type": "last_trade_price",
            "asset_id": "123",
            "market": "0xmarket",
            "price": "0.5",
            "size": "3.25",
            "side": "SELL",
            "transaction_hash": "0xtx"
        },
        {
            "event_type": "market_resolved",
            "market": "0xmarket",
            "condition_id": "0xcondition"
        }
    ])
    .to_string();
    let mixed_env = parse_market_ws_text(&mixed_raw)?;
    assert_eq!(mixed_env.events.len(), 3);
    let MarketWsParsedEvent::PriceChange(change) = &mixed_env.events[0] else {
        panic!("price_change frame parsed as wrong event");
    };
    assert_eq!(change.changes[0].side, ClobSide::Buy);
    assert!(change.changes[0].removes_level);
    assert!(matches!(
        mixed_env.events[1],
        MarketWsParsedEvent::LastTradePrice(_)
    ));
    assert!(matches!(
        mixed_env.events[2],
        MarketWsParsedEvent::Lifecycle(_)
    ));
    let pong = parse_market_ws_text("PONG")?;
    assert_eq!(pong.control, Some(MarketWsControlMessage::Pong));
    let malformed = parse_market_ws_text("{not-json").unwrap_err();
    assert_eq!(malformed.code(), ERR_WS_MARKET_JSON);

    let config = small_config();
    let mut silent = MarketWsCaptureSession::new(98);
    silent.status_code = Some(101);
    silent.handshake_success = true;
    silent.pong_received = true;
    let silent_err = require_market_ws_session_data(&silent, &config).unwrap_err();
    assert_eq!(silent_err.code(), ERR_WS_MARKET_NO_PAYLOAD_WINDOW);

    let subscription = MarketWsSubscription::new(vec!["123".to_string()]);
    let session = synthetic_session(0, vec![book_raw, mixed_raw])?;
    let proof = proof_context(
        "known-truth parser and persisted raw-frame readback",
        "2 JSON text frames: one array book frame and one mixed price/trade/lifecycle array",
        "1 frame would not cover book, price_change removal, last_trade_price, lifecycle, PONG, malformed JSON, and no-payload detection.",
        "More frames would repeat the same parser branches and readback path without adding proof.",
    );
    let report =
        write_market_ws_capture_report(&root, &subscription, &config, vec![session], proof)?;
    assert!(report.readback_passed);
    assert_eq!(report.frame_files.len(), 1);
    let readback = read_market_ws_capture_report(&root)?;
    assert_eq!(readback.proof.proof_claim, report.proof.proof_claim);
    write_blake3sums(&root, &[root.join(MARKET_WS_REPORT_FILE)])?;
    Ok(())
}

#[test]
#[ignore]
fn issue029_ws_market_live_reconnect_fsv() -> Result<()> {
    let root = fsv_root().join("live");
    assert_c_drive(&root)?;
    let gamma = GammaClient::new(GammaClientConfig {
        timeout_secs: 15,
        max_body_bytes: 2 * 1024 * 1024,
        ..GammaClientConfig::default()
    })?;
    let page = gamma.fetch_markets(&GammaMarketsRequest::crypto_active(10))?;
    let market = page
        .markets
        .iter()
        .find(|market| {
            market.enable_order_book.unwrap_or(true) && !market.clob_token_ids.is_empty()
        })
        .ok_or_else(|| {
            PolyError::raw_source(
                "CALYX_POLY_WS_MARKET_LIVE_TARGET_MISSING",
                "Gamma returned no active crypto market with CLOB token ids",
            )
        })?;
    let asset_ids = market
        .clob_token_ids
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    let subscription = MarketWsSubscription::new(asset_ids);
    let config = small_config();
    let client = MarketWsClient::new(config.clone())?;
    let mut sessions = client.capture_reconnect(&subscription, 2)?;
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|session| session.data_event_count >= 1));
    assert!(sessions.iter().all(|session| session.pong_received));

    let edge_config = MarketWsClientConfig {
        timeout_secs: 3,
        max_frames: 8,
        heartbeat_secs: 0,
        require_pong: false,
        ..config.clone()
    };
    let edge_client = MarketWsClient::new(edge_config.clone())?;
    let edge_subscription = MarketWsSubscription::new(vec!["not-a-real-token".to_string()]);
    let edge_session = edge_client.capture_window(&edge_subscription, 2)?;
    let edge_err = require_market_ws_session_data(&edge_session, &edge_config).unwrap_err();
    assert_eq!(edge_err.code(), ERR_WS_MARKET_NO_PAYLOAD_WINDOW);
    sessions.push(edge_session);

    let proof = proof_context(
        "connect, subscribe, decode real market frames, reconnect, and detect silent socket",
        "2 live reconnect sessions against one active crypto market plus 1 invalid-token no-payload edge",
        "1 live session would not prove reconnect/resubscribe. Omitting the invalid token would not prove silent socket detection.",
        "More markets or long streaming windows would only duplicate the same subscription, parser, heartbeat, reconnect, and readback paths.",
    );
    let report = write_market_ws_capture_report(&root, &subscription, &config, sessions, proof)?;
    assert_eq!(report.frame_files.len(), 3);
    let readback = read_market_ws_capture_report(&root)?;
    assert!(readback.readback_passed);
    let report_path = root.join(MARKET_WS_REPORT_FILE);
    write_blake3sums(&root, &[report_path])?;
    Ok(())
}

fn small_config() -> MarketWsClientConfig {
    MarketWsClientConfig {
        timeout_secs: 12,
        max_frames: 24,
        max_body_bytes: 1024 * 1024,
        heartbeat_secs: 10,
        min_data_events: 1,
        require_pong: true,
        ..MarketWsClientConfig::default()
    }
}

fn synthetic_session(
    session_index: usize,
    raw_frames: Vec<String>,
) -> Result<MarketWsCaptureSession> {
    let mut session = MarketWsCaptureSession::new(session_index);
    session.status_code = Some(101);
    session.handshake_success = true;
    session.pong_received = true;
    for raw in raw_frames {
        let envelope = parse_market_ws_text(&raw)?;
        session.data_event_count += envelope
            .events
            .iter()
            .filter(|event| event.is_market_data())
            .count();
        session.lifecycle_event_count += envelope
            .events
            .iter()
            .filter(|event| event.is_lifecycle())
            .count();
        session.payload_bytes += raw.len() as u64;
        session.frames.push(MarketWsFrameRecord {
            direction: "inbound".to_string(),
            opcode: "text".to_string(),
            received_at_unix_ms: 1720000000000,
            body_bytes: raw.len() as u64,
            body_sha256: Some(sha256_hex(raw.as_bytes())),
            raw_text: Some(raw),
            json_parse_ok: true,
            control: None,
            event_types: envelope
                .events
                .iter()
                .map(|event| event.event_type().to_string())
                .collect(),
            events: envelope.events,
            error_code: None,
        });
    }
    session.event_types = vec![
        "book".to_string(),
        "last_trade_price".to_string(),
        "market_resolved".to_string(),
        "price_change".to_string(),
    ];
    session.no_payload_window = false;
    let body = serde_json::to_vec(&session.frames).map_err(|err| {
        PolyError::raw_source(
            "CALYX_POLY_WS_MARKET_TEST_ENCODE_FAILED",
            format!("encode synthetic frames: {err}"),
        )
    })?;
    session.body_bytes = body.len() as u64;
    session.body_sha256 = Some(sha256_hex(&body));
    Ok(session)
}

fn proof_context(
    claim: &str,
    corpus: &str,
    why_smaller: &str,
    why_larger: &str,
) -> MarketWsProofContext {
    MarketWsProofContext {
        proof_claim: claim.to_string(),
        selected_corpus: corpus.to_string(),
        why_smaller_insufficient: why_smaller.to_string(),
        why_larger_wasteful: why_larger.to_string(),
        source_docs: vec![MARKET_WS_DOCS_URL.to_string()],
    }
}

fn fsv_root() -> PathBuf {
    env::var("POLY_ISSUE029_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(r"C:\code\poly\target\fsv\issue029_ws_market_client_20260707")
        })
}

fn assert_c_drive(path: &Path) -> Result<()> {
    let text = path.display().to_string().replace('/', "\\");
    if !text.to_ascii_lowercase().starts_with("c:\\") {
        return Err(PolyError::raw_source(
            "CALYX_POLY_WS_MARKET_TEST_ROOT_NOT_C",
            format!("FSV root must stay on C:, got {text}"),
        ));
    }
    Ok(())
}

fn write_blake3sums(root: &Path, extra: &[PathBuf]) -> Result<()> {
    let mut paths = vec![root.join("request.json"), root.join(MARKET_WS_REPORT_FILE)];
    let frame_dir = root.join("raw-frames");
    if frame_dir.exists() {
        for entry in fs::read_dir(&frame_dir).map_err(|err| {
            PolyError::raw_source(
                "CALYX_POLY_WS_MARKET_TEST_FRAME_DIR_READ_FAILED",
                format!("read {}: {err}", frame_dir.display()),
            )
        })? {
            paths.push(
                entry
                    .map_err(|err| {
                        PolyError::raw_source(
                            "CALYX_POLY_WS_MARKET_TEST_FRAME_DIR_ENTRY_FAILED",
                            format!("read frame dir entry: {err}"),
                        )
                    })?
                    .path(),
            );
        }
    }
    paths.extend(extra.iter().cloned());
    paths.sort();
    paths.dedup();
    let mut lines = Vec::new();
    for path in paths {
        if path.exists() {
            let bytes = fs::read(&path).map_err(|err| {
                PolyError::raw_source(
                    "CALYX_POLY_WS_MARKET_TEST_READBACK_FAILED",
                    format!("read {}: {err}", path.display()),
                )
            })?;
            lines.push(format!(
                "{}  {}",
                blake3::hash(&bytes).to_hex(),
                path.display()
            ));
        }
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines.join("\n")).map_err(|err| {
        PolyError::raw_source(
            "CALYX_POLY_WS_MARKET_TEST_SUMS_WRITE_FAILED",
            format!("write BLAKE3SUMS: {err}"),
        )
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
