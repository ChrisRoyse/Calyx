//! Read-only Polymarket Data API client (issue #26).

use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub use crate::data_api_parse::{
    parse_data_api_activity_value, parse_data_api_holders_value,
    parse_data_api_market_positions_value, parse_data_api_open_interest_value,
    parse_data_api_positions_value, parse_data_api_trades_value,
};
pub use crate::data_api_types::{
    DATA_API_BASE_URL, DATA_API_TRADES_OFFSET_CAP, DataApiActivityPage, DataApiActivityRecord,
    DataApiBoundedWindowPage, DataApiClientConfig, DataApiConcentrationInputs,
    DataApiEvidenceStatus, DataApiHolderGroup, DataApiHolderRecord, DataApiHoldersPage,
    DataApiJsonPage, DataApiMarketPositionGroup, DataApiMarketPositionsPage,
    DataApiOpenInterestPage, DataApiOpenInterestRecord, DataApiPositionRecord,
    DataApiPositionsPage, DataApiTradeRecord, DataApiTradeSide, DataApiTradesPage,
    build_data_api_concentration_inputs,
};
use crate::error::{PolyError, Result};

pub const ERR_DATA_API_REQUEST_INVALID: &str = "CALYX_POLY_DATA_API_REQUEST_INVALID";
pub const ERR_DATA_API_HTTP: &str = "CALYX_POLY_DATA_API_HTTP";
pub const ERR_DATA_API_BODY_READ: &str = "CALYX_POLY_DATA_API_BODY_READ";
pub const ERR_DATA_API_JSON: &str = "CALYX_POLY_DATA_API_JSON";
pub const ERR_DATA_API_ROW_INVALID: &str = "CALYX_POLY_DATA_API_ROW_INVALID";
pub const ERR_DATA_API_BOUNDED_WINDOW: &str = "CALYX_POLY_DATA_API_BOUNDED_WINDOW";
pub const ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE: &str =
    "CALYX_POLY_DATA_API_MAKER_EVIDENCE_UNAVAILABLE";

pub struct DataApiClient {
    config: DataApiClientConfig,
    agent: ureq::Agent,
}

impl DataApiClient {
    pub fn new(config: DataApiClientConfig) -> Result<Self> {
        validate_config(&config)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { config, agent })
    }

    pub fn fetch_trades_by_market(
        &self,
        condition_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<DataApiTradesPage> {
        validate_condition(condition_id)?;
        validate_trades_window(limit, offset)?;
        let page = self.get_json(format!(
            "trades?market={}&limit={limit}&offset={offset}",
            condition_id.trim()
        ))?;
        Ok(DataApiTradesPage {
            trades: parse_data_api_trades_value(&page.value)?,
            http: page,
            bounded_window: true,
        })
    }

    pub fn fetch_trades_by_user(
        &self,
        user: &str,
        limit: usize,
        offset: usize,
    ) -> Result<DataApiTradesPage> {
        validate_wallet(user)?;
        validate_trades_window(limit, offset)?;
        let page = self.get_json(format!(
            "trades?user={}&limit={limit}&offset={offset}",
            user.trim()
        ))?;
        Ok(DataApiTradesPage {
            trades: parse_data_api_trades_value(&page.value)?,
            http: page,
            bounded_window: true,
        })
    }

    pub fn fetch_holders(&self, condition_id: &str, limit: usize) -> Result<DataApiHoldersPage> {
        validate_condition(condition_id)?;
        validate_limit(limit)?;
        let page = self.get_json(format!(
            "holders?market={}&limit={limit}",
            condition_id.trim()
        ))?;
        let (groups, status) = parse_data_api_holders_value(&page.value)?;
        Ok(DataApiHoldersPage {
            http: page,
            groups,
            status,
        })
    }

    pub fn fetch_market_positions(
        &self,
        condition_id: &str,
        limit: usize,
    ) -> Result<DataApiMarketPositionsPage> {
        validate_condition(condition_id)?;
        validate_limit(limit)?;
        let page = self.get_json(format!(
            "v1/market-positions?market={}&limit={limit}",
            condition_id.trim()
        ))?;
        let (groups, status) = parse_data_api_market_positions_value(&page.value)?;
        Ok(DataApiMarketPositionsPage {
            http: page,
            groups,
            status,
        })
    }

    pub fn fetch_positions(&self, user: &str, limit: usize) -> Result<DataApiPositionsPage> {
        validate_wallet(user)?;
        validate_limit(limit)?;
        let page = self.get_json(format!("positions?user={}&limit={limit}", user.trim()))?;
        let (positions, status) = parse_data_api_positions_value(&page.value)?;
        Ok(DataApiPositionsPage {
            http: page,
            positions,
            status,
        })
    }

    pub fn fetch_activity(&self, user: &str, limit: usize) -> Result<DataApiActivityPage> {
        validate_wallet(user)?;
        validate_limit(limit)?;
        let page = self.get_json(format!("activity?user={}&limit={limit}", user.trim()))?;
        let (activity, status) = parse_data_api_activity_value(&page.value)?;
        Ok(DataApiActivityPage {
            http: page,
            activity,
            status,
        })
    }

    pub fn fetch_open_interest(&self, condition_id: &str) -> Result<DataApiOpenInterestPage> {
        validate_condition(condition_id)?;
        let page = self.get_json(format!("oi?market={}", condition_id.trim()))?;
        Ok(DataApiOpenInterestPage {
            rows: parse_data_api_open_interest_value(&page.value)?,
            http: page,
        })
    }

    pub fn probe_trades_offset_cap(&self) -> Result<DataApiBoundedWindowPage> {
        let page = self.get_json_allow_http(format!(
            "trades?limit=1&offset={DATA_API_TRADES_OFFSET_CAP}"
        ))?;
        Ok(DataApiBoundedWindowPage {
            endpoint: "trades".to_string(),
            bounded: page.status_code == 400,
            reason: "Data API /trades is a bounded activity window, not all-time history"
                .to_string(),
            http: page,
        })
    }

    pub fn require_true_maker_evidence(&self) -> Result<Vec<crate::MakerShare>> {
        Err(data_api_error(
            ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE,
            "Data API does not expose true resting CLOB maker-address size",
        ))
    }

    fn get_json(&self, endpoint: String) -> Result<DataApiJsonPage> {
        let page = self.get_json_allow_http(endpoint)?;
        if !(200..300).contains(&page.status_code) {
            return Err(data_api_error(
                ERR_DATA_API_HTTP,
                format!("GET {} returned HTTP {}", page.url, page.status_code),
            ));
        }
        Ok(page)
    }

    fn get_json_allow_http(&self, endpoint: String) -> Result<DataApiJsonPage> {
        let url = self.url(&endpoint);
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| data_api_error(ERR_DATA_API_HTTP, format!("GET {url}: {err}")))?;
        self.read_json_response(url, &mut response)
    }

    fn read_json_response(
        &self,
        url: String,
        response: &mut ureq::http::Response<ureq::Body>,
    ) -> Result<DataApiJsonPage> {
        let status_code = response.status().as_u16();
        let max = u64::try_from(self.config.max_body_bytes).map_err(|err| {
            data_api_error(ERR_DATA_API_REQUEST_INVALID, format!("body limit: {err}"))
        })?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(max)
            .read_to_vec()
            .map_err(|err| data_api_error(ERR_DATA_API_BODY_READ, format!("read {url}: {err}")))?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|err| data_api_error(ERR_DATA_API_JSON, format!("decode {url}: {err}")))?
        };
        Ok(DataApiJsonPage {
            method: "GET".to_string(),
            url,
            status_code,
            body_bytes: bytes.len() as u64,
            body_sha256: sha256_hex(&bytes),
            raw_body: bytes,
            value,
        })
    }

    fn url(&self, endpoint: &str) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        )
    }
}

fn validate_config(config: &DataApiClientConfig) -> Result<()> {
    if config.base_url.trim().is_empty() || config.timeout_secs == 0 || config.max_body_bytes == 0 {
        return Err(data_api_error(
            ERR_DATA_API_REQUEST_INVALID,
            "Data API base_url, timeout_secs, and max_body_bytes must be non-empty",
        ));
    }
    Ok(())
}

fn validate_trades_window(limit: usize, offset: usize) -> Result<()> {
    validate_limit(limit)?;
    if offset >= DATA_API_TRADES_OFFSET_CAP {
        return Err(data_api_error(
            ERR_DATA_API_BOUNDED_WINDOW,
            format!(
                "Data API /trades offset {offset} reaches bounded-window cap {DATA_API_TRADES_OFFSET_CAP}; use on-chain OrderFilled for durable history"
            ),
        ));
    }
    Ok(())
}

fn validate_limit(limit: usize) -> Result<()> {
    if limit == 0 || limit > 10_000 {
        return Err(data_api_error(
            ERR_DATA_API_REQUEST_INVALID,
            format!("Data API limit {limit} must be in 1..=10000"),
        ));
    }
    Ok(())
}

fn validate_condition(value: &str) -> Result<()> {
    let value = value.trim();
    if value.len() != 66
        || !value.starts_with("0x")
        || !value[2..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(data_api_error(
            ERR_DATA_API_REQUEST_INVALID,
            "condition id must be a 0x-prefixed 64-hex string",
        ));
    }
    Ok(())
}

fn validate_wallet(value: &str) -> Result<()> {
    let value = value.trim();
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(data_api_error(
            ERR_DATA_API_REQUEST_INVALID,
            "wallet must be a 0x-prefixed 40-hex address",
        ));
    }
    Ok(())
}

pub(crate) fn data_api_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
