//! `GET /api/health` — aggregate readiness for the Research Console.
//!
//! Combines the signals the console honestly renders:
//!   1. The physical 15-lane/109,133-row search manifest and the hash-addressed
//!      kernel jurisdiction manifest with an independently recomputed scope digest.
//!   2. Resident readiness, including `ready` and `warmed_lens_count`.
//!   3. Whether the grounded-chat surface is enabled (`modes.chat`, `chat`).
//!
//! It NEVER fails the request when a backend is down — an unreachable resident is
//! reported as `ready: false` with the reason, not a 500.

use std::fs;
use std::path::{Component, Path};

use axum::{Json, extract::State, response::IntoResponse};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::config::Config;

const EXPECTED_PHYSICAL_OPINIONS: u64 = 109_133;
const EXPECTED_CONTENT_LENSES: usize = 15;
const EXPECTED_CUYAHOGA_OPINIONS: u64 = 12_596;
const EXPECTED_OHIO_SUPREME_OPINIONS: u64 = 96_537;
const CUYAHOGA_DATASET: &str = "courtlistener-cuyahoga-2026-06-30-canonical-v5";
const OHIO_SUPREME_DATASET: &str = "courtlistener-ohio-sc-2026-06-30-canonical-v5";
const EXPECTED_CUYAHOGA_COURTS: &[&str] = &[
    "ohcirctcuyahoga",
    "ohctapp8cuyahog",
    "ohctcomplcuyaho",
    "ohctinsolvcuyah",
    "ohioctapp",
    "ohjuvctcuyahoga",
    "ohmunictcuyahog",
    "ohprobctcuyahog",
];
const MAX_KERNEL_MANIFEST_BYTES: u64 = 1_048_576;

/// Independently read the physical search manifest that the resident serves. The
/// console must never claim the canonical corpus size merely because it was compiled
/// with that number: every content lane must physically name the same 109,133 rows.
pub(crate) fn corpus_status(cfg: &Config) -> Value {
    let path = Path::new(&cfg.vault).join("idx/search/manifest.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return json!({
                "verified": false,
                "source_of_truth": path,
                "error": format!("read physical search manifest: {error}"),
            });
        }
    };
    let manifest: Value = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            return json!({
                "verified": false,
                "source_of_truth": path,
                "manifest_sha256": hex_sha256(&bytes),
                "error": format!("decode physical search manifest: {error}"),
            });
        }
    };
    let Some(slots) = manifest.get("slots").and_then(Value::as_array) else {
        return json!({
            "verified": false,
            "source_of_truth": path,
            "manifest_sha256": hex_sha256(&bytes),
            "error": "physical search manifest has no slots array",
        });
    };
    let row_counts = slots
        .iter()
        .map(|slot| slot.get("len").and_then(Value::as_u64))
        .collect::<Option<Vec<_>>>();
    let Some(row_counts) = row_counts else {
        return json!({
            "verified": false,
            "source_of_truth": path,
            "manifest_sha256": hex_sha256(&bytes),
            "content_lenses": slots.len(),
            "error": "one or more physical search slots have no integer len",
        });
    };
    let physical_opinions = row_counts.first().copied().unwrap_or(0);
    let filter_opinions = manifest
        .pointer("/filter/len")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let search_verified = slots.len() == EXPECTED_CONTENT_LENSES
        && physical_opinions == EXPECTED_PHYSICAL_OPINIONS
        && filter_opinions == EXPECTED_PHYSICAL_OPINIONS
        && row_counts
            .iter()
            .all(|&row_count| row_count == physical_opinions)
        && manifest
            .get("diskann_build_backend")
            .and_then(Value::as_str)
            == Some("cuvs-cagra")
        && manifest
            .get("sextant_cuvs_compiled")
            .and_then(Value::as_bool)
            == Some(true);
    let kernel_scope = kernel_scope_status(cfg);
    let verified = search_verified && kernel_scope.is_ok();
    let (
        scope,
        kernel_scope_source_of_truth,
        kernel_scope_sha256,
        kernel_id,
        kernel_index_sha256,
        kernel_index_bytes,
        build_ledger_seq,
        build_ledger_hash,
        kernel_error,
    ) = match kernel_scope {
        Ok(status) => (
            status.get("scope").cloned(),
            status.get("source_of_truth").cloned(),
            status.get("manifest_sha256").cloned(),
            status.get("kernel_id").cloned(),
            status.get("index_sha256").cloned(),
            status.get("index_bytes").cloned(),
            status.get("build_ledger_seq").cloned(),
            status.get("build_ledger_hash").cloned(),
            None,
        ),
        Err(error) => (None, None, None, None, None, None, None, None, Some(error)),
    };
    json!({
        "verified": verified,
        "source_of_truth": path,
        "manifest_sha256": hex_sha256(&bytes),
        "physical_opinions": physical_opinions,
        "filter_opinions": filter_opinions,
        "content_lenses": slots.len(),
        "all_slot_row_counts_match": row_counts.iter().all(|&row_count| row_count == physical_opinions),
        "expected_physical_opinions": EXPECTED_PHYSICAL_OPINIONS,
        "expected_content_lenses": EXPECTED_CONTENT_LENSES,
        "diskann_build_backend": manifest.get("diskann_build_backend"),
        "scope": scope,
        "kernel_scope_source_of_truth": kernel_scope_source_of_truth,
        "kernel_scope_sha256": kernel_scope_sha256,
        "kernel_id": kernel_id,
        "kernel_index_sha256": kernel_index_sha256,
        "kernel_index_bytes": kernel_index_bytes,
        "build_ledger_seq": build_ledger_seq,
        "build_ledger_hash": build_ledger_hash,
        "kernel_scope_error": kernel_error,
    })
}

fn kernel_scope_status(cfg: &Config) -> Result<Value, String> {
    let vault = Path::new(&cfg.vault);
    let current_path = vault.join("idx/kernel/CURRENT");
    let pointer = fs::read_to_string(&current_path).map_err(|error| {
        format!(
            "read accepted kernel pointer {}: {error}",
            current_path.display()
        )
    })?;
    let relative = pointer
        .strip_suffix('\n')
        .filter(|value| !value.contains('\n') && !value.contains('\r'))
        .ok_or_else(|| {
            format!(
                "accepted kernel pointer {} must be one newline-terminated path",
                current_path.display()
            )
        })?;
    let components = Path::new(relative).components().collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != Component::Normal("idx".as_ref())
        || components[1] != Component::Normal("kernel".as_ref())
        || components[2] != Component::Normal("manifests".as_ref())
    {
        return Err(format!(
            "accepted kernel pointer {} is outside idx/kernel/manifests",
            current_path.display()
        ));
    }
    let filename = components[3]
        .as_os_str()
        .to_str()
        .ok_or_else(|| "accepted kernel manifest filename is not UTF-8".to_string())?;
    let expected_sha256 = filename
        .strip_suffix(".json")
        .filter(|digest| is_lower_hex_sha256(digest))
        .ok_or_else(|| {
            format!("accepted kernel manifest filename is not a lowercase SHA-256: {filename}")
        })?;
    let manifest_path = vault.join(relative);
    let metadata = fs::metadata(&manifest_path).map_err(|error| {
        format!(
            "stat accepted kernel manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_KERNEL_MANIFEST_BYTES {
        return Err(format!(
            "accepted kernel manifest {} must be a nonempty regular file no larger than {MAX_KERNEL_MANIFEST_BYTES} bytes",
            manifest_path.display()
        ));
    }
    let bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "read accepted kernel manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest_sha256 = hex_sha256(&bytes);
    if manifest_sha256 != expected_sha256 {
        return Err(format!(
            "accepted kernel manifest digest mismatch at {}: filename={expected_sha256} bytes={manifest_sha256}",
            manifest_path.display()
        ));
    }
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "decode accepted kernel manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(3)
        || manifest.pointer("/graph/nodes").and_then(Value::as_u64)
            != Some(EXPECTED_PHYSICAL_OPINIONS)
    {
        return Err(
            "accepted kernel manifest schema or physical graph-node count is not canonical"
                .to_string(),
        );
    }
    let jurisdiction = manifest
        .get("jurisdiction")
        .ok_or_else(|| "accepted kernel manifest has no jurisdiction contract".to_string())?;
    let jurisdiction_rows = jurisdiction
        .get("source_rows")
        .and_then(Value::as_u64)
        .ok_or_else(|| "accepted kernel jurisdiction has no source_rows".to_string())?;
    if jurisdiction.get("schema_version").and_then(Value::as_u64) != Some(2)
        || jurisdiction.get("country").and_then(Value::as_str) != Some("United States")
        || jurisdiction.get("court_system").and_then(Value::as_str) != Some("state")
        || jurisdiction.get("state").and_then(Value::as_str) != Some("Ohio")
        || jurisdiction.get("county").is_some()
        || jurisdiction.get("appellate_district").is_some()
        || jurisdiction_rows != EXPECTED_PHYSICAL_OPINIONS
    {
        return Err("accepted kernel jurisdiction global contract is not canonical".to_string());
    }
    let scopes = jurisdiction
        .get("scopes")
        .and_then(Value::as_array)
        .filter(|scopes| scopes.len() == 2)
        .ok_or_else(|| {
            "accepted kernel jurisdiction must contain exactly two scopes".to_string()
        })?;
    validate_scope(
        &scopes[0],
        CUYAHOGA_DATASET,
        EXPECTED_CUYAHOGA_OPINIONS,
        EXPECTED_CUYAHOGA_COURTS,
        Some("Cuyahoga"),
        Some("Eighth"),
    )?;
    validate_scope(
        &scopes[1],
        OHIO_SUPREME_DATASET,
        EXPECTED_OHIO_SUPREME_OPINIONS,
        &["ohio"],
        None,
        None,
    )?;
    let sum = scopes.iter().try_fold(0_u64, |total, scope| {
        scope
            .get("source_rows")
            .and_then(Value::as_u64)
            .and_then(|rows| total.checked_add(rows))
    });
    if sum != Some(EXPECTED_PHYSICAL_OPINIONS) {
        return Err("accepted kernel jurisdiction scope rows do not sum to the corpus".to_string());
    }
    let observed_contract_sha256 = jurisdiction
        .get("metadata_contract_sha256")
        .and_then(Value::as_str)
        .filter(|digest| is_lower_hex_sha256(digest))
        .ok_or_else(|| {
            "accepted kernel jurisdiction metadata digest is not a lowercase SHA-256".to_string()
        })?;
    let recomputed_contract_sha256 = jurisdiction_sha256(jurisdiction, scopes)?;
    if observed_contract_sha256 != recomputed_contract_sha256 {
        return Err(format!(
            "accepted kernel jurisdiction metadata digest mismatch: manifest={observed_contract_sha256} recomputed={recomputed_contract_sha256}"
        ));
    }
    let build_ledger_seq = manifest
        .get("build_ledger_seq")
        .and_then(Value::as_u64)
        .filter(|seq| *seq > 0)
        .ok_or_else(|| {
            "accepted kernel manifest has no positive build ledger sequence".to_string()
        })?;
    let build_ledger_hash = manifest
        .get("build_ledger_hash")
        .and_then(Value::as_str)
        .filter(|digest| is_lower_hex_sha256(digest))
        .ok_or_else(|| {
            "accepted kernel manifest build ledger hash is not a lowercase SHA-256".to_string()
        })?;
    let kernel_id = manifest
        .get("kernel_id")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "accepted kernel manifest has no valid kernel_id".to_string())?;
    let index_sha256 = manifest
        .pointer("/index_json/sha256")
        .and_then(Value::as_str)
        .filter(|digest| is_lower_hex_sha256(digest))
        .ok_or_else(|| "accepted kernel index has no lowercase SHA-256".to_string())?;
    let index_bytes = manifest
        .pointer("/index_json/bytes")
        .and_then(Value::as_u64)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| "accepted kernel index has no positive byte count".to_string())?;
    Ok(json!({
        "source_of_truth": manifest_path,
        "manifest_sha256": manifest_sha256,
        "build_ledger_seq": build_ledger_seq,
        "build_ledger_hash": build_ledger_hash,
        "kernel_id": kernel_id,
        "index_sha256": index_sha256,
        "index_bytes": index_bytes,
        "scope": {
            "description": "Canonical Ohio opinions: Supreme Court of Ohio plus Cuyahoga Eighth District",
            "physical_opinions": jurisdiction_rows,
            "collections": [
                {
                    "source_dataset": OHIO_SUPREME_DATASET,
                    "court": "Supreme Court of Ohio",
                    "physical_opinions": EXPECTED_OHIO_SUPREME_OPINIONS,
                },
                {
                    "source_dataset": CUYAHOGA_DATASET,
                    "court": "Cuyahoga County Eighth District Court of Appeals",
                    "physical_opinions": EXPECTED_CUYAHOGA_OPINIONS,
                },
            ],
        },
    }))
}

fn validate_scope(
    scope: &Value,
    source_dataset: &str,
    source_rows: u64,
    court_ids: &[&str],
    county: Option<&str>,
    appellate_district: Option<&str>,
) -> Result<(), String> {
    let observed_courts = scope
        .get("court_ids")
        .and_then(Value::as_array)
        .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
    if scope.get("source_dataset").and_then(Value::as_str) != Some(source_dataset)
        || scope.get("source_rows").and_then(Value::as_u64) != Some(source_rows)
        || observed_courts.as_deref() != Some(court_ids)
        || scope.get("county").and_then(Value::as_str) != county
        || scope.get("appellate_district").and_then(Value::as_str) != appellate_district
        || (county.is_none() && scope.get("county").is_some())
        || (appellate_district.is_none() && scope.get("appellate_district").is_some())
    {
        return Err(format!(
            "accepted kernel jurisdiction scope {source_dataset} is not canonical"
        ));
    }
    Ok(())
}

fn jurisdiction_sha256(jurisdiction: &Value, scopes: &[Value]) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"calyx-kernel-jurisdiction-v2");
    for key in ["country", "court_system", "state"] {
        let value = jurisdiction
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("accepted kernel jurisdiction has no string {key}"))?;
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    for scope in scopes {
        let source_dataset = scope
            .get("source_dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| "accepted kernel jurisdiction scope has no dataset".to_string())?;
        digest.update(source_dataset.as_bytes());
        digest.update([0]);
        let court_ids = scope
            .get("court_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| "accepted kernel jurisdiction scope has no court_ids".to_string())?;
        for court_id in court_ids {
            digest.update(
                court_id
                    .as_str()
                    .ok_or_else(|| {
                        "accepted kernel jurisdiction court_id is not a string".to_string()
                    })?
                    .as_bytes(),
            );
            digest.update([0]);
        }
        digest.update([0xff]);
        if let Some(county) = scope.get("county").and_then(Value::as_str) {
            digest.update(county.as_bytes());
        }
        digest.update([0]);
        if let Some(district) = scope.get("appellate_district").and_then(Value::as_str) {
            digest.update(district.as_bytes());
        }
        digest.update([0]);
        digest.update(
            scope
                .get("source_rows")
                .and_then(Value::as_u64)
                .ok_or_else(|| "accepted kernel jurisdiction scope has no source_rows".to_string())?
                .to_le_bytes(),
        );
    }
    digest.update(
        jurisdiction
            .get("source_rows")
            .and_then(Value::as_u64)
            .ok_or_else(|| "accepted kernel jurisdiction has no source_rows".to_string())?
            .to_le_bytes(),
    );
    // `digest` already contains SHA-256(contract). Hex-encode those finalized
    // bytes directly; passing them to `hex_sha256` would hash the digest again.
    Ok(hex_lower(&digest.finalize()))
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Production startup gate for the embedded 109,133-opinion research contract. A
/// binary whose prompt says "109,133" must not boot against a smaller or partially
/// rebuilt physical search manifest.
pub fn verify_canonical_corpus(cfg: &Config) -> Result<Value, String> {
    let status = corpus_status(cfg);
    if status.get("verified").and_then(Value::as_bool) == Some(true) {
        Ok(status)
    } else {
        Err(format!("physical corpus contract mismatch: {status}"))
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

/// Probe resident readiness. Returns a structured value regardless of outcome.
pub(crate) async fn resident_status(cfg: &Config) -> Value {
    let mut cmd = tokio::process::Command::new(&cfg.calyx_binary);
    cmd.args(["panel", "resident", "ready", "--addr", &cfg.resident_addr])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    cfg.apply_subprocess_env(&mut cmd);

    let result = tokio::time::timeout(cfg.resident_probe_timeout, cmd.output()).await;
    match result {
        Err(_) => json!({ "ready": false, "error": "resident readiness probe timed out" }),
        Ok(Err(_)) => json!({
            "ready": false,
            "error": "pinned calyx binary could not be launched",
        }),
        Ok(Ok(out)) if out.status.success() => match serde_json::from_slice::<Value>(&out.stdout) {
            Ok(mut value) => {
                // Surface the ready flag at the top even if the tool nests it.
                if value.get("ready").is_none() {
                    value["ready"] = json!(false);
                }
                value
            }
            Err(_) => json!({ "ready": false, "error": "readiness stdout was not JSON" }),
        },
        Ok(Ok(_)) => json!({ "ready": false, "error": "readiness probe exited non-zero" }),
    }
}

/// The chat streaming/control contract the console feature-detects: the SSE route, the
/// user-control cancel route, and the terminal event names a client must recognise to
/// close cleanly (including the cancellation path added for user-abort). Static and
/// secret-free.
fn chat_contract() -> Value {
    json!({
        "stream": { "method": "POST", "path": "/api/chat", "media_type": "text/event-stream" },
        "cancel": { "method": "POST", "path": "/api/chat/cancel", "body": { "episode_id": "<from the `opened` event>" } },
        "terminal_events": ["done", "error", "cancelled"],
    })
}

/// `GET /api/health`.
pub async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.config;
    let production = crate::production::production_status(cfg).await;

    Json(json!({
        "service": "calyx-lawdemo-web",
        "ready": production.get("ready").cloned().unwrap_or(Value::Bool(false)),
        "scope": production.get("scope").cloned().unwrap_or(Value::Null),
        "evidence_label": cfg.evidence_label,
        "modes": {
            "chat": cfg.chat.is_some(),
        },
        "chat": crate::chat::health_info(cfg),
        "chat_contract": chat_contract(),
        "corpus": production.get("corpus").cloned().unwrap_or(Value::Null),
        "resident": production.get("resident").cloned().unwrap_or(Value::Null),
    }))
}
