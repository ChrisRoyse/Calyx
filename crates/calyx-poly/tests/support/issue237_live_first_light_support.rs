use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_poly::Resolution;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GammaResolutionReadback {
    pub market_id: String,
    pub condition_id: String,
    pub question: Option<String>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub end_date: Option<String>,
    pub updated_at: Option<String>,
    pub outcomes: Vec<String>,
    pub outcome_prices: Vec<f64>,
    pub winner: Option<usize>,
    pub resolution: Option<Resolution>,
}

pub fn get_market_body(market_id: &str) -> Vec<u8> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .get(&format!(
            "https://gamma-api.polymarket.com/markets/{market_id}"
        ))
        .header("Accept", "application/json")
        .call()
        .expect("GET exact Gamma market");
    assert!(
        response.status().is_success(),
        "Gamma market read failed with status {}",
        response.status()
    );
    response.body_mut().read_to_vec().expect("read Gamma body")
}

pub fn gamma_resolution_readback(
    value: &Value,
    market_id: &str,
    condition_id: &str,
) -> Result<GammaResolutionReadback, String> {
    let outcomes = parse_string_array(value, "outcomes")?;
    let outcome_prices = parse_price_array(value, "outcomePrices")?;
    let closed = value.get("closed").and_then(Value::as_bool);
    let winner = closed
        .filter(|is_closed| *is_closed)
        .and_then(|_| clean_winner(&outcome_prices));
    let resolution = winner.map(|idx| Resolution {
        condition_id: text(value, "conditionId").unwrap_or_else(|| condition_id.to_string()),
        winning_outcome_index: idx as u32,
        winning_label: outcomes
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("outcome_{idx}")),
        resolved_ts: resolution_ts(value).expect("parse resolution ts"),
        source: "gamma-closed-derived".to_string(),
        disputed: false,
    });
    Ok(GammaResolutionReadback {
        market_id: text(value, "id").unwrap_or_else(|| market_id.to_string()),
        condition_id: text(value, "conditionId").unwrap_or_else(|| condition_id.to_string()),
        question: text(value, "question"),
        active: value.get("active").and_then(Value::as_bool),
        closed,
        archived: value.get("archived").and_then(Value::as_bool),
        end_date: text(value, "endDate"),
        updated_at: text(value, "updatedAt"),
        outcomes,
        outcome_prices,
        winner,
        resolution,
    })
}

#[allow(dead_code)]
pub fn source_capture_root(root_name: &str) -> PathBuf {
    std::env::var_os("POLY_ISSUE237_LIVE_CAPTURE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target/fsv").join(root_name))
}

pub fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create copy root");
    for entry in fs::read_dir(src).expect("read source tree") {
        let entry = entry.expect("source entry");
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy source file");
        }
    }
}

pub fn blake3_hex(path: &Path) -> String {
    blake3::hash(&fs::read(path).expect("read blake3 file"))
        .to_hex()
        .to_string()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02X}")).collect()
}

pub fn assert_c_drive(path: &Path) {
    #[cfg(windows)]
    assert!(
        path.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:"),
        "{} must stay on C:",
        path.display()
    );
}

#[allow(dead_code)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn clean_winner(prices: &[f64]) -> Option<usize> {
    let winners = prices
        .iter()
        .enumerate()
        .filter(|(_, price)| **price >= 0.99)
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    if winners.len() == 1
        && prices
            .iter()
            .enumerate()
            .all(|(idx, price)| idx == winners[0] || *price <= 0.01)
    {
        Some(winners[0])
    } else {
        None
    }
}

fn parse_string_array(value: &Value, field: &str) -> Result<Vec<String>, String> {
    let raw = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string array field {field}"))?;
    serde_json::from_str::<Vec<String>>(raw).map_err(|err| format!("decode {field}: {err}"))
}

fn parse_price_array(value: &Value, field: &str) -> Result<Vec<f64>, String> {
    parse_string_array(value, field)?
        .into_iter()
        .map(|text| {
            text.parse::<f64>()
                .map_err(|err| format!("parse {field} price {text}: {err}"))
        })
        .collect()
}

fn text(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn resolution_ts(value: &Value) -> Option<u64> {
    for field in ["updatedAt", "endDate", "endDateIso"] {
        if let Some(ts) = text(value, field).and_then(|text| iso8601_to_unix(&text)) {
            return Some(ts);
        }
    }
    None
}

fn iso8601_to_unix(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
    {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (h, mi, se) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + se).ok()
}
