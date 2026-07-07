use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

#[allow(dead_code)]
pub fn known_healthy_market_integrity() -> calyx_poly::risk::MarketIntegrityScreen {
    calyx_poly::risk::MarketIntegrityScreen {
        ok: true,
        code: calyx_poly::risk::MARKET_INTEGRITY_OK.to_string(),
        reason: "synthetic known-healthy holder and maker concentration evidence".to_string(),
        holder_count: 10,
        holder_herfindahl: 0.10,
        top_holder_share: 0.10,
        invalid_holder_rows: 0,
        maker_count: 4,
        maker_herfindahl: 0.25,
        top_maker_share: 0.25,
        invalid_maker_rows: 0,
    }
}

#[allow(dead_code)]
pub fn known_healthy_oracle_risk() -> calyx_poly::oracle::OracleRiskScreen {
    known_healthy_oracle_risk_for(0.94)
}

#[allow(dead_code)]
pub fn known_healthy_oracle_risk_for(p_win: f64) -> calyx_poly::oracle::OracleRiskScreen {
    calyx_poly::oracle::OracleRiskScreen {
        ok: true,
        code: calyx_poly::oracle::ORACLE_RISK_OK.to_string(),
        reason: "synthetic known-healthy UMA oracle evidence".to_string(),
        oracle: "uma".to_string(),
        raw_p_win: p_win,
        p_win_haircut: 0.0,
        p_win_adjusted: p_win,
        dispute_risk: 0.0,
        active_dispute: false,
        liveness_seconds_remaining: 0.0,
        market_price: 0.80,
        near_certain_price: false,
    }
}

#[allow(dead_code)]
pub fn known_healthy_wash_trade() -> calyx_poly::wash::WashTradeScreen {
    calyx_poly::wash::WashTradeScreen {
        ok: true,
        code: calyx_poly::wash::WASH_TRADE_OK.to_string(),
        reason: "synthetic known-healthy distinct-counterparty volume evidence".to_string(),
        raw_volume: 100_000.0,
        distinct_counterparty_count: 5,
        distinct_counterparty_volume: 100_000.0,
        distinct_counterparty_volume_ratio: 1.0,
        top_counterparty_share: 0.20,
        invalid_counterparty_rows: 0,
    }
}

pub fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create JSON parent directory");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("encode JSON evidence"),
    )
    .expect("write JSON evidence");
}

#[allow(dead_code)]
pub fn write_blake3sums(root: &Path) {
    let mut paths = Vec::new();
    collect_path_list(root, &mut paths);
    paths.sort();
    let mut lines = Vec::new();
    for path in paths {
        if path.file_name().and_then(|name| name.to_str()) == Some("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read file for BLAKE3");
        lines.push(format!(
            "{}  {}",
            blake3::hash(&bytes).to_hex(),
            path.strip_prefix(root).expect("strip root").display()
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines.join("\n")).expect("write BLAKE3SUMS");
}

#[allow(dead_code)]
pub fn collect_files(dir: &Path, out: &mut Vec<Value>) {
    let mut paths = Vec::new();
    collect_path_list(dir, &mut paths);
    paths.sort();
    for path in paths {
        let meta = fs::metadata(&path).expect("metadata for physical file");
        out.push(json!({
            "path": path.display().to_string(),
            "bytes": meta.len()
        }));
    }
}

#[allow(dead_code)]
fn collect_path_list(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read FSV directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_path_list(&path, out);
        } else {
            out.push(path);
        }
    }
}

pub fn named_fsv_root(env: &str, fallback_name: &str) -> (PathBuf, bool) {
    if let Some(value) = std::env::var_os(env) {
        return (PathBuf::from(value), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_name}-{}", std::process::id())),
        false,
    )
}

pub fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove previous FSV root");
    }
    fs::create_dir_all(path).expect("create FSV root");
}

#[allow(dead_code)]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
