use std::collections::BTreeSet;

use crate::raw_large_corpus_profile::{LargeCorpusFieldProfile, LargeCorpusFieldStats};
use crate::raw_large_corpus_types::LargeCorpusManifest;
use crate::schema_derivation_types::{
    SchemaBlockedRuntimeSource, SchemaDatasetContract, SchemaFieldContract,
};

pub(crate) fn dataset_contracts(
    profiles: &[LargeCorpusFieldProfile],
) -> Vec<SchemaDatasetContract> {
    profiles
        .iter()
        .map(|profile| SchemaDatasetContract {
            dataset: profile.dataset.clone(),
            source: profile.source.clone(),
            record_count: profile.record_count,
            field_count: profile.fields.len(),
            storage_family: storage_family(&profile.source),
        })
        .collect()
}

pub(crate) fn field_contracts(profiles: &[LargeCorpusFieldProfile]) -> Vec<SchemaFieldContract> {
    let mut contracts = Vec::new();
    for profile in profiles {
        for field in &profile.fields {
            contracts.push(SchemaFieldContract {
                dataset: profile.dataset.clone(),
                source: profile.source.clone(),
                field: field.name.clone(),
                present_count: field.present_count,
                missing_count: field.missing_count,
                null_count: field.null_count,
                type_counts: field.type_counts.clone(),
                json_string_count: field.json_string_count,
                roles: field_roles(&field.name, field),
                variant_contract: variant_contract(field),
                example_sha256: field.example_sha256.clone(),
            });
        }
    }
    contracts
}

pub(crate) fn blocked_runtime_sources(
    manifest: &LargeCorpusManifest,
) -> Vec<SchemaBlockedRuntimeSource> {
    let mut blocked = Vec::new();
    if manifest
        .edge_cases
        .iter()
        .any(|edge| edge.name.contains("equity") && edge.no_payload_window && edge.expectation_met)
    {
        blocked.push(SchemaBlockedRuntimeSource {
            source: "websocket-rtds/equity_prices".to_string(),
            issue: "#198".to_string(),
            reason: "this corpus has no persisted RTDS equity_prices payload bytes".to_string(),
        });
    }
    blocked
}

pub(crate) fn raw_retention_rules() -> Vec<String> {
    vec![
        "retain raw body path, byte count, SHA256, source, endpoint, URL/query, page/window index, request body hash, and capture timestamp before projection".to_string(),
        "retain WebSocket raw frames and no-payload windows separately from parsed event rows".to_string(),
        "retain JSON-string fields before secondary decoding".to_string(),
        "retain binary/archive source bytes by format before extraction".to_string(),
    ]
}

pub(crate) fn derived_contracts() -> Vec<String> {
    vec![
        "association edges derive only from observed join identifiers after raw retention".to_string(),
        "forecast features derive from normalized price, book, trade, liquidity, volume, and outcome-linked fields".to_string(),
        "outcome/scoring rows derive only after resolved outcome anchors are physically present".to_string(),
        "RTDS equity payload tables require real persisted equity_prices bytes; keep docs-only corpora blocked-runtime".to_string(),
    ]
}

fn field_roles(name: &str, field: &LargeCorpusFieldStats) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut roles = BTreeSet::new();
    if is_join_field(&lower) {
        roles.insert("association-input");
        roles.insert("normalized");
    }
    if lower.contains("price")
        || lower.contains("volume")
        || lower.contains("liquidity")
        || lower.contains("bid")
        || lower.contains("ask")
        || lower.contains("spread")
        || lower.contains("tick")
        || lower == "value"
        || lower == "size"
    {
        roles.insert("forecast-input");
        roles.insert("normalized");
    }
    if lower.contains("outcome")
        || lower.contains("resolved")
        || lower.contains("resolution")
        || lower.contains("payout")
    {
        roles.insert("outcome-scoring-input");
        roles.insert("normalized");
    }
    if lower.contains("time")
        || lower.contains("date")
        || lower.contains("created")
        || lower.contains("updated")
        || lower.contains("start")
        || lower.contains("end")
    {
        roles.insert("normalized");
    }
    if field.json_string_count > 0 || field.array_max_len.is_some() {
        roles.insert("derived-decode-input");
    }
    if roles.is_empty() {
        roles.insert("raw-only");
    }
    roles.into_iter().map(str::to_string).collect()
}

fn is_join_field(lower: &str) -> bool {
    lower.contains("condition")
        || lower.contains("question")
        || lower.contains("token")
        || lower.contains("asset")
        || lower.contains("market")
        || lower.contains("wallet")
        || lower.contains("transaction")
        || lower == "id"
}

fn variant_contract(field: &LargeCorpusFieldStats) -> String {
    if field.type_counts.len() > 1 {
        "union_required".to_string()
    } else if field.null_count > 0 || field.missing_count > 0 {
        "nullable_or_missing_allowed".to_string()
    } else {
        "single_non_null_type".to_string()
    }
}

fn storage_family(source: &str) -> String {
    match source {
        "websocket-market" | "websocket-rtds" | "websocket-sports" => {
            "windowed-frame-stream".to_string()
        }
        "polygon-rpc" | "goldsky-subgraph" => "query-response-with-request-provenance".to_string(),
        "historical-dump" => "external-archive-by-format".to_string(),
        "clob" => "market-data-endpoint-family".to_string(),
        _ => "raw-response-family".to_string(),
    }
}
