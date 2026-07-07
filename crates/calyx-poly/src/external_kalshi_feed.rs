//! Read-only Kalshi external feed client and signal admission (#34).

use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_assay::{MIN_ASSAY_SAMPLES, TrustTag, ksg_mi_continuous_discrete};
use calyx_core::Clock;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{PolyError, Result};
pub use crate::external_kalshi_feed_parse::{
    encode_kalshi_market_signal, kalshi_market_outcome_label, kalshi_market_signal_observations,
    parse_kalshi_market, parse_kalshi_markets_value,
};
pub use crate::external_kalshi_feed_types::{
    ExternalSignalAdmissionReport, ExternalSignalOutcomeObservation, KALSHI_EXTERNAL_API_BASE_URL,
    KalshiEncodedSignal, KalshiFeedClientConfig, KalshiMarketRecord, KalshiMarketsPage,
    KalshiMarketsRequest, KalshiPersistedFeedReport,
};
use crate::lens_autobuild::LensCandidateMeasurement;

pub const KALSHI_FEED_SCHEMA_VERSION: &str = "poly.external_kalshi_feed.v1";
pub const KALSHI_FEED_ARTIFACT_KIND: &str = "poly_external_kalshi_feed";
pub const EXTERNAL_SIGNAL_ADMISSION_SCHEMA_VERSION: &str = "poly.external_signal_admission.v1";
pub const EXTERNAL_SIGNAL_ADMISSION_ARTIFACT_KIND: &str = "poly_external_signal_admission";
pub const KALSHI_SIGNAL_ADMIT_BITS_THRESHOLD: f32 = 0.05;
pub const DEFAULT_EXTERNAL_SIGNAL_K: usize = 3;

pub const ERR_KALSHI_REQUEST_INVALID: &str = "CALYX_POLY_KALSHI_REQUEST_INVALID";
pub const ERR_KALSHI_HTTP: &str = "CALYX_POLY_KALSHI_HTTP";
pub const ERR_KALSHI_BODY_READ: &str = "CALYX_POLY_KALSHI_BODY_READ";
pub const ERR_KALSHI_JSON: &str = "CALYX_POLY_KALSHI_JSON";
pub const ERR_KALSHI_MARKET_INVALID: &str = "CALYX_POLY_KALSHI_MARKET_INVALID";
pub const ERR_KALSHI_READBACK: &str = "CALYX_POLY_KALSHI_READBACK";
pub const ERR_KALSHI_ENCODE_INVALID: &str = "CALYX_POLY_KALSHI_ENCODE_INVALID";
pub const ERR_EXTERNAL_SIGNAL_ADMISSION_INVALID: &str =
    "CALYX_POLY_EXTERNAL_SIGNAL_ADMISSION_INVALID";

pub const EXTERNAL_SIGNAL_ADMITTED: &str = "CALYX_POLY_EXTERNAL_SIGNAL_ADMITTED";
pub const EXTERNAL_SIGNAL_REFUSED_BELOW_THRESHOLD: &str =
    "CALYX_POLY_EXTERNAL_SIGNAL_REFUSED_BELOW_THRESHOLD";
pub const EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED: &str =
    "CALYX_POLY_EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED";
pub const EXTERNAL_SIGNAL_REFUSED_SINGLE_CLASS: &str =
    "CALYX_POLY_EXTERNAL_SIGNAL_REFUSED_SINGLE_CLASS";

pub struct KalshiFeedClient {
    config: KalshiFeedClientConfig,
    agent: ureq::Agent,
}

impl KalshiFeedClient {
    pub fn new(config: KalshiFeedClientConfig) -> Result<Self> {
        validate_config(&config)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { config, agent })
    }

    pub fn fetch_markets(&self, request: &KalshiMarketsRequest) -> Result<KalshiMarketsPage> {
        validate_request(request)?;
        let url = market_url(&self.config.base_url, request);
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| kalshi_error(ERR_KALSHI_HTTP, format!("GET {url}: {err}")))?;
        let status_code = response.status().as_u16();
        let max = u64::try_from(self.config.max_body_bytes).map_err(|err| {
            kalshi_error(ERR_KALSHI_REQUEST_INVALID, format!("body limit: {err}"))
        })?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(max)
            .read_to_vec()
            .map_err(|err| kalshi_error(ERR_KALSHI_BODY_READ, format!("read {url}: {err}")))?;
        if !(200..300).contains(&status_code) {
            return Err(kalshi_error(
                ERR_KALSHI_HTTP,
                format!("GET {url} returned HTTP {status_code}"),
            ));
        }
        KalshiMarketsPage::from_raw(url, status_code, bytes)
    }
}

impl KalshiMarketsPage {
    pub fn from_raw(url: String, status_code: u16, raw_body: Vec<u8>) -> Result<Self> {
        let value = if raw_body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&raw_body)
                .map_err(|err| kalshi_error(ERR_KALSHI_JSON, format!("decode {url}: {err}")))?
        };
        let markets = parse_kalshi_markets_value(&value)?;
        Ok(Self {
            url,
            status_code,
            body_bytes: raw_body.len() as u64,
            body_sha256: sha256_hex(&raw_body),
            raw_body,
            markets,
        })
    }
}

pub fn measure_external_signal_admission(
    source: &str,
    signal_name: &str,
    observations: &[ExternalSignalOutcomeObservation],
    clock: &dyn Clock,
    k: usize,
) -> Result<ExternalSignalAdmissionReport> {
    if source.trim().is_empty() || signal_name.trim().is_empty() || k == 0 {
        return Err(admission_error(
            "source, signal_name, and k must be non-empty",
        ));
    }
    let n = observations.len();
    let positive_count = observations.iter().filter(|row| row.outcome).count();
    let negative_count = n.saturating_sub(positive_count);
    let base = |code: &str, reason: String, bits: f32, low: f32, high: f32, admitted: bool| {
        ExternalSignalAdmissionReport {
            schema_version: EXTERNAL_SIGNAL_ADMISSION_SCHEMA_VERSION.to_string(),
            artifact_kind: EXTERNAL_SIGNAL_ADMISSION_ARTIFACT_KIND.to_string(),
            source: source.to_string(),
            signal_name: signal_name.to_string(),
            estimator: "ksg_mi_continuous_discrete".to_string(),
            n_samples: n,
            positive_count,
            negative_count,
            bits,
            ci_low_bits: low,
            ci_high_bits: high,
            threshold_bits: KALSHI_SIGNAL_ADMIT_BITS_THRESHOLD,
            admitted,
            code: code.to_string(),
            reason,
            computed_at: clock.now(),
        }
    };
    if n < MIN_ASSAY_SAMPLES {
        return Ok(base(
            EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED,
            format!("need at least {MIN_ASSAY_SAMPLES} rows; got n={n}"),
            0.0,
            0.0,
            0.0,
            false,
        ));
    }
    if positive_count == 0 || negative_count == 0 {
        return Ok(base(
            EXTERNAL_SIGNAL_REFUSED_SINGLE_CLASS,
            "outcome labels are single-class; external signal bits are undefined".to_string(),
            0.0,
            0.0,
            0.0,
            false,
        ));
    }
    if positive_count <= k || negative_count <= k {
        return Ok(base(
            EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED,
            format!(
                "need at least k+1 rows per outcome class; got positive={positive_count}, negative={negative_count}, k={k}"
            ),
            0.0,
            0.0,
            0.0,
            false,
        ));
    }
    let mut x = Vec::with_capacity(n);
    let mut labels = Vec::with_capacity(n);
    for row in observations {
        if !row.signal_value.is_finite() {
            return Err(admission_error("external signal value must be finite"));
        }
        x.push(vec![row.signal_value]);
        labels.push(usize::from(row.outcome));
    }
    let estimate = ksg_mi_continuous_discrete(&x, &labels, k)?;
    let admitted = estimate.bits >= KALSHI_SIGNAL_ADMIT_BITS_THRESHOLD;
    let (code, reason) = if admitted {
        (
            EXTERNAL_SIGNAL_ADMITTED,
            format!(
                "measured {:.6} bits >= {:.6} bit admission floor",
                estimate.bits, KALSHI_SIGNAL_ADMIT_BITS_THRESHOLD
            ),
        )
    } else {
        (
            EXTERNAL_SIGNAL_REFUSED_BELOW_THRESHOLD,
            format!(
                "measured {:.6} bits < {:.6} bit admission floor",
                estimate.bits, KALSHI_SIGNAL_ADMIT_BITS_THRESHOLD
            ),
        )
    };
    Ok(base(
        code,
        reason,
        estimate.bits,
        estimate.ci_low,
        estimate.ci_high,
        admitted,
    ))
}

pub fn kalshi_lens_candidate_from_admission(
    report: &ExternalSignalAdmissionReport,
    evidence_artifact: impl Into<String>,
    trust: TrustTag,
) -> Result<LensCandidateMeasurement> {
    if report.source != "kalshi" {
        return Err(admission_error(format!(
            "expected kalshi admission source, got {}",
            report.source
        )));
    }
    Ok(LensCandidateMeasurement {
        lens_key: format!("external_kalshi_{}", sanitize_key(&report.signal_name)),
        encoder_kind: "kalshi_numeric_v1".to_string(),
        source_fields: vec![
            "kalshi.ticker".to_string(),
            "kalshi.title".to_string(),
            "kalshi.yes_bid_dollars".to_string(),
            "kalshi.yes_ask_dollars".to_string(),
            "kalshi.last_price_dollars".to_string(),
            "kalshi.liquidity_dollars".to_string(),
            "kalshi.volume_fp".to_string(),
            "kalshi.open_interest_fp".to_string(),
            "kalshi.result".to_string(),
        ],
        measured_gain_bits: report.bits,
        ci_low_bits: report.ci_low_bits,
        ci_high_bits: report.ci_high_bits,
        n_samples: report.n_samples,
        trust,
        evidence_artifact: evidence_artifact.into(),
        requested_action: "append_lens_spec".to_string(),
    })
}

pub fn persist_kalshi_markets_page(
    dir: &Path,
    name: &str,
    page: &KalshiMarketsPage,
) -> Result<KalshiPersistedFeedReport> {
    let case_root = dir.join(sanitize(name));
    std::fs::create_dir_all(&case_root).map_err(|err| {
        kalshi_error(
            ERR_KALSHI_READBACK,
            format!("create Kalshi feed dir {}: {err}", case_root.display()),
        )
    })?;
    let raw_path = case_root.join("body.json");
    std::fs::write(&raw_path, &page.raw_body).map_err(|err| {
        kalshi_error(
            ERR_KALSHI_READBACK,
            format!("write Kalshi raw body {}: {err}", raw_path.display()),
        )
    })?;
    let raw_readback = std::fs::read(&raw_path).map_err(|err| {
        kalshi_error(
            ERR_KALSHI_READBACK,
            format!("read Kalshi raw body {}: {err}", raw_path.display()),
        )
    })?;
    if raw_readback != page.raw_body {
        return Err(kalshi_error(
            ERR_KALSHI_READBACK,
            "Kalshi raw body readback did not match captured bytes",
        ));
    }
    let parsed_from_disk =
        KalshiMarketsPage::from_raw(page.url.clone(), page.status_code, raw_readback)?;
    if parsed_from_disk.markets != page.markets {
        return Err(kalshi_error(
            ERR_KALSHI_READBACK,
            "Kalshi parsed readback did not match captured market records",
        ));
    }
    let parsed_path =
        crate::diagnostics_store::write_json(&case_root, "parsed-markets.json", &page.markets)?;
    let parsed_readback: Vec<KalshiMarketRecord> =
        crate::diagnostics_store::read_json(&parsed_path)?;
    if parsed_readback != page.markets {
        return Err(kalshi_error(
            ERR_KALSHI_READBACK,
            "Kalshi parsed JSON readback did not match market records",
        ));
    }
    let summary_path = case_root.join("summary.json");
    let report = KalshiPersistedFeedReport {
        schema_version: KALSHI_FEED_SCHEMA_VERSION.to_string(),
        artifact_kind: KALSHI_FEED_ARTIFACT_KIND.to_string(),
        source: "kalshi".to_string(),
        url: page.url.clone(),
        status_code: page.status_code,
        body_bytes: page.body_bytes,
        body_sha256: page.body_sha256.clone(),
        body_blake3: blake3_hex(&page.raw_body),
        raw_path: raw_path.display().to_string(),
        parsed_path: parsed_path.display().to_string(),
        summary_path: summary_path.display().to_string(),
        market_count: page.markets.len(),
        tickers: page
            .markets
            .iter()
            .map(|market| market.ticker.clone())
            .collect(),
        raw_readback_equal: true,
        parsed_readback_equal: true,
    };
    crate::diagnostics_store::write_json(&case_root, "summary.json", &report)?;
    let report_readback: KalshiPersistedFeedReport =
        crate::diagnostics_store::read_json(&summary_path)?;
    if report_readback != report {
        return Err(kalshi_error(
            ERR_KALSHI_READBACK,
            "Kalshi feed summary readback did not match report",
        ));
    }
    Ok(report_readback)
}

pub fn write_external_signal_admission_report(
    dir: &Path,
    name: &str,
    report: &ExternalSignalAdmissionReport,
) -> Result<PathBuf> {
    let path =
        crate::diagnostics_store::write_json(dir, &format!("{}.json", sanitize(name)), report)?;
    let readback: ExternalSignalAdmissionReport = crate::diagnostics_store::read_json(&path)?;
    if &readback != report {
        return Err(kalshi_error(
            ERR_KALSHI_READBACK,
            "external signal admission readback did not match report",
        ));
    }
    Ok(path)
}

fn validate_config(config: &KalshiFeedClientConfig) -> Result<()> {
    if config.base_url.trim().is_empty() || config.timeout_secs == 0 || config.max_body_bytes == 0 {
        return Err(kalshi_error(
            ERR_KALSHI_REQUEST_INVALID,
            "Kalshi base_url, timeout_secs, and max_body_bytes must be non-empty",
        ));
    }
    Ok(())
}

fn validate_request(request: &KalshiMarketsRequest) -> Result<()> {
    if request.limit == 0 || request.limit > 1000 {
        return Err(kalshi_error(
            ERR_KALSHI_REQUEST_INVALID,
            format!("Kalshi markets limit {} must be in 1..=1000", request.limit),
        ));
    }
    if let Some(status) = &request.status {
        let status = status.trim();
        let valid = matches!(
            status,
            "unopened" | "open" | "paused" | "closed" | "settled"
        );
        if !valid {
            return Err(kalshi_error(
                ERR_KALSHI_REQUEST_INVALID,
                format!("unsupported Kalshi status filter {status}"),
            ));
        }
    }
    Ok(())
}

fn market_url(base_url: &str, request: &KalshiMarketsRequest) -> String {
    let mut parts = vec![format!("limit={}", request.limit)];
    if let Some(status) = &request.status {
        parts.push(format!("status={}", status.trim()));
    }
    format!(
        "{}/markets?{}",
        base_url.trim_end_matches('/'),
        parts.join("&")
    )
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn sanitize_key(value: &str) -> String {
    sanitize(value).to_ascii_lowercase()
}

fn kalshi_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

fn admission_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_EXTERNAL_SIGNAL_ADMISSION_INVALID, message)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
