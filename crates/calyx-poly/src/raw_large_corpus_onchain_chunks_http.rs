use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::{PolyError, Result};

pub(crate) fn execute_post(
    agent: &ureq::Agent,
    url: &str,
    request_body: &[u8],
    max_body_bytes: usize,
    dataset: &str,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    let endpoint = RateLimitEndpoint::new("polygon-rpc", dataset, "POST");
    execute_rate_limited_request(&endpoint, || {
        let result = agent
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(request_body);
        let mut status_code = None;
        let mut retry_after_ms = None;
        let mut bytes = Vec::new();
        let mut transport_error = None;
        match result {
            Ok(mut response) => {
                status_code = Some(response.status().as_u16());
                retry_after_ms = parse_retry_after_ms(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok()),
                );
                bytes = response
                    .body_mut()
                    .with_config()
                    .limit(max_body_bytes as u64)
                    .read_to_vec()
                    .map_err(|err| {
                        PolyError::raw_source(
                            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_BODY_READ_FAILED",
                            format!("read chunk body for {dataset}: {err}"),
                        )
                    })?;
            }
            Err(err) => transport_error = Some(err.to_string()),
        }
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes, transport_error),
        })
    })
}
