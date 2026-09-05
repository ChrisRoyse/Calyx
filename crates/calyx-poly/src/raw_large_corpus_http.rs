use calyx_core::Clock;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::{PolyError, Result};

pub(crate) fn get_bytes(
    agent: &ureq::Agent,
    source: &str,
    endpoint_name: &str,
    url: &str,
    limit: usize,
    clock: &dyn Clock,
) -> Result<(Option<u16>, Vec<u8>)> {
    let endpoint = RateLimitEndpoint::new(source, endpoint_name, "GET");
    execute_rate_limited_request(clock, &endpoint, || {
        let mut response = agent
            .get(url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_HTTP_TRANSPORT_FAILED",
                    format!("fetch {url}: {err}"),
                )
            })?;
        let status_code = Some(response.status().as_u16());
        let retry_after_ms = parse_retry_after_ms(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        );
        let bytes = response
            .body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_BODY_READ_FAILED",
                    format!("read body from {url}: {err}"),
                )
            })?;
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes),
        })
    })
}
