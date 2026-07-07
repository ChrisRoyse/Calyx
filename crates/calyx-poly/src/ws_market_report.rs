use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::raw_source_support::sha256_hex;
use crate::ws_market_client::MarketWsCaptureSession;
use crate::ws_market_types::{
    ERR_WS_MARKET_READBACK_MISMATCH, ERR_WS_MARKET_REQUEST_INVALID, MARKET_WS_ARTIFACT_KIND,
    MARKET_WS_DOCS_URL, MARKET_WS_REPORT_FILE, MARKET_WS_SCHEMA_VERSION, MarketWsClientConfig,
    MarketWsSubscription, ws_market_error,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketWsProofContext {
    pub proof_claim: String,
    pub selected_corpus: String,
    pub why_smaller_insufficient: String,
    pub why_larger_wasteful: String,
    pub source_docs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketWsFileReadback {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub readback_sha256: String,
    pub readback_match: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsCaptureReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub url: String,
    pub docs_url: String,
    pub proof: MarketWsProofContext,
    pub subscription: MarketWsSubscription,
    pub config: MarketWsClientConfig,
    pub sessions: Vec<MarketWsCaptureSession>,
    pub request_file: MarketWsFileReadback,
    pub frame_files: Vec<MarketWsFileReadback>,
    pub readback_passed: bool,
}

#[derive(Serialize)]
struct MarketWsRequestRecord<'a> {
    schema_version: &'static str,
    url: &'a str,
    subscription: &'a MarketWsSubscription,
    config: &'a MarketWsClientConfig,
}

pub fn write_market_ws_capture_report(
    root: &Path,
    subscription: &MarketWsSubscription,
    config: &MarketWsClientConfig,
    sessions: Vec<MarketWsCaptureSession>,
    proof: MarketWsProofContext,
) -> Result<MarketWsCaptureReport> {
    reject_d_drive(root)?;
    fs::create_dir_all(root).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("create market WebSocket FSV root {}: {err}", root.display()),
        )
    })?;
    let request_record = MarketWsRequestRecord {
        schema_version: MARKET_WS_SCHEMA_VERSION,
        url: &config.url,
        subscription,
        config,
    };
    let request_file = write_json_readback(&root.join("request.json"), &request_record)?;
    let frame_dir = root.join("raw-frames");
    fs::create_dir_all(&frame_dir).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("create frame dir {}: {err}", frame_dir.display()),
        )
    })?;
    let mut frame_files = Vec::with_capacity(sessions.len());
    for session in &sessions {
        let path = frame_dir.join(format!("session-{:03}.json", session.session_index));
        frame_files.push(write_json_readback(&path, &session.frames)?);
    }
    let report = MarketWsCaptureReport {
        schema_version: MARKET_WS_SCHEMA_VERSION.to_string(),
        artifact_kind: MARKET_WS_ARTIFACT_KIND.to_string(),
        url: config.url.clone(),
        docs_url: MARKET_WS_DOCS_URL.to_string(),
        proof,
        subscription: subscription.clone(),
        config: config.clone(),
        sessions,
        request_file,
        frame_files,
        readback_passed: true,
    };
    write_json_readback(&root.join(MARKET_WS_REPORT_FILE), &report)?;
    Ok(report)
}

pub fn read_market_ws_capture_report(root: &Path) -> Result<MarketWsCaptureReport> {
    let path = root.join(MARKET_WS_REPORT_FILE);
    let bytes = fs::read(&path).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("read market WebSocket report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("decode market WebSocket report {}: {err}", path.display()),
        )
    })
}

fn write_json_readback<T: Serialize>(path: &Path, value: &T) -> Result<MarketWsFileReadback> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("encode JSON {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("write JSON {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("read JSON {}: {err}", path.display()),
        )
    })?;
    let sha = sha256_hex(&bytes);
    let readback_sha = sha256_hex(&readback);
    if readback != bytes {
        return Err(ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("JSON readback mismatch at {}", path.display()),
        ));
    }
    Ok(MarketWsFileReadback {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: sha,
        readback_sha256: readback_sha,
        readback_match: true,
    })
}

fn reject_d_drive(path: &Path) -> Result<()> {
    let text = display_path(path);
    if text.to_ascii_lowercase().starts_with("d:\\") {
        return Err(ws_market_error(
            ERR_WS_MARKET_REQUEST_INVALID,
            format!("D: drive is forbidden for Poly WebSocket evidence: {text}"),
        ));
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    PathBuf::from(path).display().to_string().replace('/', "\\")
}
