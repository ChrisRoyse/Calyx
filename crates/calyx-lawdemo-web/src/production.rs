//! Shared live production-state inspection for health and research tools.

use serde_json::{Value, json};

use crate::config::Config;

// Must match the binary kernel-rank protocol spoken by the pinned `calyx`
// executable. A protocol bump is an intentional incompatibility: production
// must remain not-ready until the web health contract is updated with the
// corresponding client/server deployment.
const EXPECTED_KERNEL_RANK_PROTOCOL_VERSION: u64 = 3;

/// Read the physical manifests and serving resident without substituting demo state.
pub async fn production_status(cfg: &Config) -> Value {
    let resident = crate::health::resident_status(cfg).await;
    let corpus = crate::health::corpus_status(cfg);
    let expected_lenses = corpus
        .get("expected_content_lenses")
        .and_then(Value::as_u64);
    let expected_template = format!("vault:{}", cfg.vault);
    let ready = corpus.get("verified").and_then(Value::as_bool) == Some(true)
        && resident.get("ready").and_then(Value::as_bool) == Some(true)
        && resident.get("bind").and_then(Value::as_str) == Some(cfg.resident_addr.as_str())
        && resident.get("template_source").and_then(Value::as_str)
            == Some(expected_template.as_str())
        && resident.get("content_lens_count").and_then(Value::as_u64) == expected_lenses
        && resident.get("warmed_lens_count").and_then(Value::as_u64) == expected_lenses
        && resident
            .get("gpu_content_lens_count")
            .and_then(Value::as_u64)
            == expected_lenses
        && resident
            .get("cpu_content_lens_count")
            .and_then(Value::as_u64)
            == Some(0)
        && resident
            .pointer("/kernel_rank/protocol_version")
            .and_then(Value::as_u64)
            == Some(EXPECTED_KERNEL_RANK_PROTOCOL_VERSION)
        && resident
            .pointer("/kernel_rank/manifest_sha256")
            .and_then(Value::as_str)
            == corpus.get("kernel_scope_sha256").and_then(Value::as_str)
        && resident
            .pointer("/kernel_rank/kernel_id")
            .and_then(Value::as_str)
            == corpus.get("kernel_id").and_then(Value::as_str)
        && resident
            .pointer("/kernel_rank/index_sha256")
            .and_then(Value::as_str)
            == corpus.get("kernel_index_sha256").and_then(Value::as_str)
        && resident
            .pointer("/kernel_rank/index_bytes")
            .and_then(Value::as_u64)
            == corpus.get("kernel_index_bytes").and_then(Value::as_u64)
        && resident
            .pointer("/kernel_rank/index_rows")
            .and_then(Value::as_u64)
            .is_some_and(|rows| rows > 0);

    json!({
        "status": if ready { "ready" } else { "not_ready" },
        "ready": ready,
        "scope": corpus.get("scope").cloned().unwrap_or(Value::Null),
        "corpus": corpus,
        "resident": resident,
    })
}
