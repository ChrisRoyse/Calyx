//! Issue #238 - targeted live maturity/re-resolution recheck for captured market 2744242.
//!
//! This ignored test reads exactly the market captured by the #238 live harness. It records a
//! not-ready report while the market is still open, and after the source closes with a clean winner
//! it joins the saved pending forecasts to a clearly tagged Gamma-derived resolution in a copied
//! harness root so the original capture evidence remains immutable.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_poly::crypto_capture_harness::{
    CRYPTO_CAPTURE_STATE_FILE, CRYPTO_PRE_RESOLUTION_CORPUS_FILE, join_crypto_capture_resolution,
    read_crypto_capture_state,
};
use calyx_poly::{PendingForecastRegister, Resolution};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const MARKET_ID: &str = "2744242";
const CONDITION_ID: &str = "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20";
const CAPTURE_ROOT_NAME: &str =
    "issue238_live_crypto_capture_harness_public_search_pricefix_20260707";
const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue238-crypto-capture";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecheckDecision {
    SourceStillOpen,
    NoCleanWinner,
    JoinedGammaClosedDerived,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GammaResolutionReadback {
    market_id: String,
    condition_id: String,
    question: Option<String>,
    active: Option<bool>,
    closed: Option<bool>,
    archived: Option<bool>,
    end_date: Option<String>,
    updated_at: Option<String>,
    outcomes: Vec<String>,
    outcome_prices: Vec<f64>,
    winner: Option<usize>,
    resolution: Option<Resolution>,
}

#[test]
#[ignore = "requires live Gamma read and the prior #238 live capture evidence root"]
fn issue238_live_resolution_recheck_target_market_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE238_LIVE_RESOLUTION_RECHECK_ROOT",
        "issue238-live-resolution-recheck",
    );
    assert_c_drive(&root);
    reset_dir(&root);

    let source_root = source_capture_root();
    assert_c_drive(&source_root);
    let harness_root = root.join("harness-copy");
    copy_tree(&source_root, &harness_root);

    let before_state = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
        .expect("read copied harness state before recheck");
    assert_eq!(before_state.captures.len(), 1);
    assert_eq!(before_state.captures[0].market_id, MARKET_ID);
    assert_eq!(before_state.captures[0].condition_id, CONDITION_ID);

    let raw_path = root.join("gamma-market-2744242-body.json");
    let body = get_market_body();
    fs::write(&raw_path, &body).expect("write Gamma body");
    let raw_hash = sha256_hex(&body);
    let value: Value = serde_json::from_slice(&body).expect("decode Gamma body");
    let gamma = gamma_resolution_readback(&value).expect("parse Gamma recheck fields");

    let mut edge_cases = Vec::new();
    let (decision, join_value) = if gamma.closed != Some(true) {
        let after = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
            .expect("read state after open source");
        assert_eq!(after, before_state);
        edge_cases.push(json!({
            "case": "source_still_open",
            "before_matured": before_state.matured_resolutions.len(),
            "after_matured": after.matured_resolutions.len(),
            "state_unchanged": true
        }));
        (RecheckDecision::SourceStillOpen, Value::Null)
    } else if gamma.resolution.is_none() {
        let after = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
            .expect("read state after no winner");
        assert_eq!(after, before_state);
        edge_cases.push(json!({
            "case": "closed_without_clean_winner",
            "outcome_prices": gamma.outcome_prices,
            "state_unchanged": true
        }));
        (RecheckDecision::NoCleanWinner, Value::Null)
    } else {
        let resolution = gamma.resolution.clone().expect("resolution present");
        assert!(resolution.resolved_ts > before_state.captures[0].captured_ts);
        let vault = AsterVault::open(
            harness_root.join("live-capture-vault"),
            VAULT_ID.parse().expect("vault id"),
            VAULT_SALT.to_vec(),
            VaultOptions::default(),
        )
        .expect("open copied live capture vault");
        let mut register = PendingForecastRegister::default();
        let joined = join_crypto_capture_resolution(
            &vault,
            &mut register,
            &harness_root,
            &resolution,
            false,
        )
        .expect("join copied pending forecasts to source-derived resolution");
        let corpus_path = harness_root.join(CRYPTO_PRE_RESOLUTION_CORPUS_FILE);
        let corpus: Value =
            serde_json::from_slice(&fs::read(&corpus_path).expect("read corpus")).unwrap();
        assert_eq!(joined.record.pairs.len(), 2);
        assert_eq!(
            joined
                .record
                .pairs
                .iter()
                .filter(|pair| pair.actual_win)
                .count(),
            1
        );
        edge_cases.push(json!({
            "case": "no_lookahead_timing",
            "capture_ts": before_state.captures[0].captured_ts,
            "resolved_ts": resolution.resolved_ts,
            "resolution_after_capture": true
        }));
        (
            RecheckDecision::JoinedGammaClosedDerived,
            json!({
                "resolution": resolution,
                "join": joined.join,
                "matured_record": joined.record,
                "corpus_path": corpus_path.display().to_string(),
                "corpus": corpus
            }),
        )
    };

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 238,
        "proof_claim": "Determine whether the exact live #238 captured market 2744242 can now mature into pending-to-resolved pre-resolution pairs without broad replay.",
        "minimum_sufficient_proof_corpus": {
            "market_count": 1,
            "market_id": MARKET_ID,
            "condition_id": CONDITION_ID,
            "copied_capture_count": before_state.captures.len(),
            "why_this_is_sufficient": "The prior live capture has exactly one pending market for this closeout path; this one-market Gamma read plus copied harness state is the smallest corpus that can prove whether those pending forecasts can now mature.",
            "why_smaller_is_insufficient": "Zero market reads or no copied harness state would not inspect the source of truth for the pending capture.",
            "why_larger_is_wasteful": "Additional markets or broad replay would not prove whether this exact pending capture has matured."
        },
        "source_of_truth": [
            raw_path.display().to_string(),
            harness_root.join(CRYPTO_CAPTURE_STATE_FILE).display().to_string(),
            harness_root.join("live-capture-vault").display().to_string()
        ],
        "raw_sha256": raw_hash,
        "gamma_readback": gamma,
        "decision": decision,
        "join_readback": join_value,
        "edge_cases": edge_cases,
        "physical_files": files
    });
    let report_path = root.join("issue238_live_resolution_recheck_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!(
        "ISSUE238_LIVE_RESOLUTION_RECHECK_READBACK={}",
        report_path.display()
    );
}

fn get_market_body() -> Vec<u8> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .get(&format!(
            "https://gamma-api.polymarket.com/markets/{MARKET_ID}"
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

fn gamma_resolution_readback(value: &Value) -> Result<GammaResolutionReadback, String> {
    let outcomes = parse_string_array(value, "outcomes")?;
    let outcome_prices = parse_price_array(value, "outcomePrices")?;
    let closed = value.get("closed").and_then(Value::as_bool);
    let winner = closed
        .filter(|is_closed| *is_closed)
        .and_then(|_| clean_winner(&outcome_prices));
    let resolution = match winner {
        Some(idx) => Some(Resolution {
            condition_id: text(value, "conditionId").unwrap_or_else(|| CONDITION_ID.to_string()),
            winning_outcome_index: idx as u32,
            winning_label: outcomes
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("outcome_{idx}")),
            resolved_ts: resolution_ts(value)?,
            source: "gamma-closed-derived".to_string(),
            disputed: false,
        }),
        None => None,
    };
    Ok(GammaResolutionReadback {
        market_id: text(value, "id").unwrap_or_else(|| MARKET_ID.to_string()),
        condition_id: text(value, "conditionId").unwrap_or_else(|| CONDITION_ID.to_string()),
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

fn resolution_ts(value: &Value) -> Result<u64, String> {
    for field in ["updatedAt", "endDate", "endDateIso"] {
        if let Some(ts) = text(value, field).and_then(|text| iso8601_to_unix(&text)) {
            return Ok(ts);
        }
    }
    Err("Gamma market lacks parseable updatedAt/endDate timestamp".to_string())
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

fn source_capture_root() -> PathBuf {
    std::env::var_os("POLY_ISSUE238_LIVE_CAPTURE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target/fsv").join(CAPTURE_ROOT_NAME))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn copy_tree(src: &Path, dst: &Path) {
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn assert_c_drive(path: &Path) {
    #[cfg(not(windows))]
    let _ = path;
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}
