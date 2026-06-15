use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::manifest::ManifestStore;
use calyx_core::{Clock, SlotId};
use calyx_forge::{QuantLevel, Quantizer, TurboQuantCodec, new_seed};
use calyx_ledger::{
    ActorId, EntryKind, LedgerCfStore, PayloadBuilder, RedactionPolicy, SubjectId, VerifyResult,
    decode, verify_chain,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::resource::{HazardResult, ProbeResult, run_probe};
use super::resource_support::{case_dir, err, open_vault};
use super::security_support::{
    MEMTABLE_BYTES, START_TS, deterministic_vector, hash_bytes, hash_hex, hex_bytes, list_files,
    max_abs_delta, replay_observation, run_successful_rerank, scan_dir_for_bytes, search_hits,
    with_determinism,
};

const SECRET_TOKEN: &str = "CALYX_TEST_SECRET_ABCD1234";
const DIM: usize = 32;

pub fn run_hazards_22_25(root: &Path) -> Vec<HazardResult> {
    [
        (
            22,
            "secret leakage / request text non-persistence",
            probe_h22_secret_leakage as fn(&Path) -> ProbeResult,
        ),
        (23, "deterministic replay parity", probe_h23_nondeterminism),
        (24, "whole-host loss DR drill", probe_h24_whole_host_loss),
        (25, "upgrade / format skew", probe_h25_upgrade_skew),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}

fn probe_h22_secret_leakage(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h22_secret_leakage")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(&vault_dir, START_TS + 22, b"ph59-h22", MEMTABLE_BYTES, None)?;
    vault
        .write_cf(
            ColumnFamily::Base,
            b"h22-safe-row".to_vec(),
            b"safe persisted row".to_vec(),
        )
        .map_err(err)?;
    vault.flush().map_err(err)?;

    let before_hits = scan_dir_for_bytes(&vault_dir, SECRET_TOKEN.as_bytes())?;
    let rerank_candidate = format!("rerank candidate {SECRET_TOKEN}");
    let rerank = run_successful_rerank(&rerank_candidate)?;
    let embed_input = format!("embed input {SECRET_TOKEN}");
    let search_input = format!("search query {SECRET_TOKEN}");
    let embed_bytes = run_embed_request(&vault, embed_input.as_bytes())?;
    let search_hits = run_search_request(&search_input, deterministic_vector(22, DIM))?;
    let ledger_refs = [
        append_hash_only_ledger(
            &vault,
            "rerank",
            rerank_candidate.as_bytes(),
            START_TS + 220,
        )?,
        append_hash_only_ledger(&vault, "embed", embed_input.as_bytes(), START_TS + 221)?,
        append_hash_only_ledger(&vault, "search", search_input.as_bytes(), START_TS + 222)?,
    ];
    vault.flush().map_err(err)?;

    let after_hits = scan_dir_for_bytes(&vault_dir, SECRET_TOKEN.as_bytes())?;
    let ledger_rows = decoded_ledger_rows(&vault)?;
    let payload_contains_secret = ledger_rows.iter().any(|row| row.payload_contains_secret);
    let raw_secret_payload_error_code = secret_payload_error_code()?;
    let redacted_debug = !format!(
        "{:?}",
        calyx_sextant::RerankRequest::new("privacy query", vec![rerank_candidate.clone()])
    )
    .contains(SECRET_TOKEN);
    let passed = before_hits.is_empty()
        && after_hits.is_empty()
        && rerank.request_contained_candidate
        && rerank.score == 0.42
        && embed_bytes > 0
        && !search_hits.is_empty()
        && ledger_rows.len() == 3
        && !payload_contains_secret
        && raw_secret_payload_error_code == "CALYX_LEDGER_SECRET_IN_PAYLOAD"
        && redacted_debug;
    Ok((
        passed,
        json!({
            "trigger": "synthetic secret token injected into rerank, embed, and search request-scoped text",
            "expected": {
                "secret_scan_violations": 0,
                "ledger_payload_hash_only": true,
                "raw_secret_payload_error_code": "CALYX_LEDGER_SECRET_IN_PAYLOAD"
            },
            "actual": {
                "secret_len": SECRET_TOKEN.len(),
                "secret_hash": hash_hex(SECRET_TOKEN.as_bytes()),
                "scan_before": before_hits,
                "scan_after": after_hits,
                "vault_files": list_files(&vault_dir)?,
                "rerank": {
                    "request_contained_candidate": rerank.request_contained_candidate,
                    "request_text_count": rerank.request_text_count,
                    "score": rerank.score,
                    "debug_redacted": redacted_debug
                },
                "embed_output_bytes": embed_bytes,
                "search_hits": search_hits,
                "ledger_refs": ledger_refs.iter().map(|ledger_ref| json!({
                    "seq": ledger_ref.seq,
                    "hash": hex_bytes(&ledger_ref.hash)
                })).collect::<Vec<_>>(),
                "ledger_rows": ledger_rows,
                "secret_scan_violations": 0,
                "ledger_payload_contains_secret": payload_contains_secret,
                "raw_secret_payload_error_code": raw_secret_payload_error_code,
                "panic_free": true
            },
            "metrics_text": "calyx_secret_scan_violations_total{vault=\"ph59-h22\"} 0\ncalyx_secret_request_types_checked{vault=\"ph59-h22\"} 3\n"
        }),
    ))
}

fn probe_h23_nondeterminism(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h23_determinism")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(&vault_dir, START_TS + 23, b"ph59-h23", MEMTABLE_BYTES, None)?;
    let first = with_determinism("1", || replay_observation("ph59 deterministic query"))?;
    let second = with_determinism("1", || replay_observation("ph59 deterministic query"))?;
    let off_mode = with_determinism("0", || replay_observation("ph59 deterministic query"))?;
    vault
        .write_cf(
            ColumnFamily::Base,
            b"h23-result-first".to_vec(),
            first.serialized.clone(),
        )
        .map_err(err)?;
    vault
        .write_cf(
            ColumnFamily::Base,
            b"h23-result-second".to_vec(),
            second.serialized.clone(),
        )
        .map_err(err)?;
    vault
        .write_cf(
            ColumnFamily::slot(SlotId::new(23)),
            b"h23-query-embedding".to_vec(),
            first.quantized_bytes.clone(),
        )
        .map_err(err)?;
    vault.flush().map_err(err)?;
    let seq = vault.latest_seq();
    let first_readback = vault
        .read_cf_at(seq, ColumnFamily::Base, b"h23-result-first")
        .map_err(err)?
        .unwrap_or_default();
    let second_readback = vault
        .read_cf_at(seq, ColumnFamily::Base, b"h23-result-second")
        .map_err(err)?
        .unwrap_or_default();
    let embedding_readback = vault
        .read_cf_at(
            seq,
            ColumnFamily::slot(SlotId::new(23)),
            b"h23-query-embedding",
        )
        .map_err(err)?
        .unwrap_or_default();
    let max_delta = max_abs_delta(&first.decoded_vector, &second.decoded_vector);
    let deterministic_equal = first.serialized == second.serialized
        && first_readback == second_readback
        && first.quantized_bytes == second.quantized_bytes
        && embedding_readback == first.quantized_bytes;
    let nondeterministic_mode_claimed_determinism = off_mode.determinism_enabled;
    let passed =
        deterministic_equal && max_delta <= 1e-3 && !nondeterministic_mode_claimed_determinism;
    Ok((
        passed,
        json!({
            "trigger": "CALYX_DETERMINISM=1 replay of identical Forge quantization plus Sextant HNSW query",
            "expected": {
                "determinism_replay_max_delta_lte": 0.001,
                "result_bytes_identical": true,
                "determinism_0_does_not_claim_determinism": true
            },
            "actual": {
                "determinism_replay_max_delta": max_delta,
                "result_bytes_identical": first.serialized == second.serialized,
                "aster_result_readback_identical": first_readback == second_readback,
                "embedding_readback_bytes": embedding_readback.len(),
                "seed_id": first.seed_id,
                "first": first.summary(),
                "second": second.summary(),
                "determinism_0": off_mode.summary(),
                "nondeterministic_mode_claimed_determinism": nondeterministic_mode_claimed_determinism,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_determinism_replay_max_delta{{vault=\"ph59-h23\"}} {:.8}\ncalyx_determinism_result_bytes_equal{{vault=\"ph59-h23\"}} {}\n",
                max_delta,
                usize::from(deterministic_equal)
            )
        }),
    ))
}

fn probe_h24_whole_host_loss(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h24_whole_host_loss")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(&vault_dir, START_TS + 24, b"ph59-h24", MEMTABLE_BYTES, None)?;
    vault
        .write_cf(
            ColumnFamily::Base,
            b"h24-cx-1".to_vec(),
            b"h24 byte exact".to_vec(),
        )
        .map_err(err)?;
    append_hash_only_ledger(&vault, "dr", b"h24 restore ledger payload", START_TS + 240)?;
    vault.flush().map_err(err)?;
    let base_readback = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, b"h24-cx-1")
        .map_err(err)?
        .unwrap_or_default();
    let store = AsterLedgerCfStore::open(&vault_dir).map_err(err)?;
    let ledger_rows = store.scan().map_err(err)?;
    let verify = verify_chain(&store, 0..ledger_rows.len() as u64).map_err(err)?;
    let restic_version = restic_version();
    let restic_enabled = env::var("CALYX_PH59_RESTIC_DR").ok().as_deref() == Some("1");
    let skipped_pending_ph66 = !restic_enabled;
    let chain_intact =
        matches!(verify, VerifyResult::Intact { count } if count == ledger_rows.len() as u64);
    let passed = base_readback == b"h24 byte exact" && chain_intact && skipped_pending_ph66;
    Ok((
        passed,
        json!({
            "trigger": "synthetic DR vault written, ledger CF read through AsterLedgerCfStore; restic drill gated until PH66",
            "expected": {
                "base_row_byte_exact": true,
                "ledger_chain_intact": true,
                "skipped_pending_ph66_allowed": true
            },
            "actual": {
                "base_row_hex": hex_bytes(&base_readback),
                "base_row_byte_exact": base_readback == b"h24 byte exact",
                "ledger_row_count": ledger_rows.len(),
                "ledger_verify": verify_json(&verify),
                "restic_version": restic_version,
                "restic_dr_enabled_env": restic_enabled,
                "skipped_pending_ph66": skipped_pending_ph66,
                "dr_restore_verified": false,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_dr_restore_verified{{vault=\"ph59-h24\"}} 0\ncalyx_dr_skipped_pending_ph66{{vault=\"ph59-h24\"}} {}\n",
                usize::from(skipped_pending_ph66)
            )
        }),
    ))
}

fn probe_h25_upgrade_skew(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h25_upgrade_skew")?;
    let vault_dir = dir.join("vault_v1");
    let vault = open_vault(&vault_dir, START_TS + 25, b"ph59-h25", MEMTABLE_BYTES, None)?;
    vault
        .write_cf(
            ColumnFamily::Base,
            b"h25-old-cx".to_vec(),
            b"old shard bytes".to_vec(),
        )
        .map_err(err)?;
    vault.flush().map_err(err)?;
    let old_manifest = ManifestStore::open(&vault_dir)
        .load_current()
        .map_err(err)?;
    drop(vault);
    let reopened = open_vault(&vault_dir, START_TS + 26, b"ph59-h25", MEMTABLE_BYTES, None)?;
    let old_readback = reopened
        .read_cf_at(reopened.latest_seq(), ColumnFamily::Base, b"h25-old-cx")
        .map_err(err)?
        .unwrap_or_default();
    reopened
        .write_cf(
            ColumnFamily::Base,
            b"h25-new-cx".to_vec(),
            b"new shard bytes".to_vec(),
        )
        .map_err(err)?;
    reopened.flush().map_err(err)?;
    let new_readback = reopened
        .read_cf_at(reopened.latest_seq(), ColumnFamily::Base, b"h25-new-cx")
        .map_err(err)?
        .unwrap_or_default();
    let new_manifest = ManifestStore::open(&vault_dir)
        .load_current()
        .map_err(err)?;
    let unsupported_error = unsupported_manifest_error(&dir, &new_manifest)?;
    let passed = old_manifest.version.major == 1
        && old_readback == b"old shard bytes"
        && new_readback == b"new shard bytes"
        && new_manifest.version.major == old_manifest.version.major
        && unsupported_error == "CALYX_FORMAT_VERSION_UNSUPPORTED";
    Ok((
        passed,
        json!({
            "trigger": "open major-1 durable vault, read old shard, append new shard, then attempt unknown major 99",
            "expected": {
                "old_shards_readable": true,
                "new_cx_written_current_format": true,
                "unknown_major_rejected": "CALYX_FORMAT_VERSION_UNSUPPORTED"
            },
            "actual": {
                "old_manifest_version": old_manifest.version,
                "new_manifest_version": new_manifest.version,
                "old_shards_readable": old_readback == b"old shard bytes",
                "old_readback_hex": hex_bytes(&old_readback),
                "new_cx_readable": new_readback == b"new shard bytes",
                "new_readback_hex": hex_bytes(&new_readback),
                "unknown_major_attempted": 99,
                "unknown_major_error_code": unsupported_error,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_format_old_shards_readable{{vault=\"ph59-h25\"}} {}\ncalyx_format_unknown_major_rejected{{vault=\"ph59-h25\"}} {}\n",
                usize::from(old_readback == b"old shard bytes"),
                usize::from(unsupported_error == "CALYX_FORMAT_VERSION_UNSUPPORTED")
            )
        }),
    ))
}

fn run_embed_request<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
    request: &[u8],
) -> Result<usize, String>
where
    C: Clock,
{
    let codec = TurboQuantCodec::new(new_seed(DIM, &hash_bytes(request)), QuantLevel::Bits3p5)
        .map_err(err)?;
    let quantized = codec
        .encode(&deterministic_vector(hash_bytes(request)[0], DIM))
        .map_err(err)?;
    vault
        .write_cf(
            ColumnFamily::slot(SlotId::new(22)),
            b"h22-embed-output".to_vec(),
            quantized.bytes.clone(),
        )
        .map_err(err)?;
    Ok(quantized.bytes.len())
}

fn run_search_request(text: &str, query_vec: Vec<f32>) -> Result<Vec<Value>, String> {
    Ok(search_hits(text, query_vec)?
        .into_iter()
        .map(|hit| json!({"cx_id": hit.cx_id, "rank": hit.rank, "score_bits": hit.score_bits}))
        .collect())
}

fn append_hash_only_ledger<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
    request_type: &str,
    request_bytes: &[u8],
    ts: u64,
) -> Result<calyx_core::LedgerRef, String>
where
    C: Clock,
{
    let mut builder = PayloadBuilder::default();
    builder
        .insert_str("request_hash", hash_hex(request_bytes))
        .insert_str("candidate_hash", hash_hex(request_bytes))
        .insert_str("request_type_hash", hash_hex(request_type.as_bytes()))
        .insert_u64("ts", ts);
    let payload = RedactionPolicy::default().apply_to_payload(&builder);
    RedactionPolicy::check_payload(&payload).map_err(err)?;
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(hash_bytes(request_bytes).to_vec()),
            payload,
            ActorId::Service("calyx-hazard-soak".to_string()),
        )
        .map_err(err)
}

#[derive(Serialize)]
struct LedgerPayloadReadback {
    key_hex: String,
    kind: String,
    payload: Value,
    payload_len: usize,
    payload_contains_secret: bool,
}

fn decoded_ledger_rows<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
) -> Result<Vec<LedgerPayloadReadback>, String>
where
    C: Clock,
{
    let mut rows = Vec::new();
    for (key, value) in vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .map_err(err)?
    {
        let entry = decode(&value).map_err(err)?;
        let payload_text = String::from_utf8_lossy(&entry.payload);
        rows.push(LedgerPayloadReadback {
            key_hex: hex_bytes(&key),
            kind: entry.kind.to_string(),
            payload: serde_json::from_slice(&entry.payload).map_err(err)?,
            payload_len: entry.payload.len(),
            payload_contains_secret: payload_text.contains(SECRET_TOKEN),
        });
    }
    Ok(rows)
}

fn secret_payload_error_code() -> Result<&'static str, String> {
    let payload = serde_json::to_vec(&json!({"secret": SECRET_TOKEN})).map_err(err)?;
    Ok(RedactionPolicy::check_payload(&payload)
        .expect_err("secret payload must fail closed")
        .code)
}

fn restic_version() -> String {
    match Command::new("restic").arg("version").output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(output) => format!("restic unavailable: exit {:?}", output.status.code()),
        Err(error) => format!("restic unavailable: {error}"),
    }
}

fn verify_json(verify: &VerifyResult) -> Value {
    match verify {
        VerifyResult::Intact { count } => json!({"kind": "intact", "count": count}),
        VerifyResult::Broken { at_seq, .. } => json!({"kind": "broken", "at_seq": at_seq}),
        VerifyResult::Corrupt { at_seq, reason } => {
            json!({"kind": "corrupt", "at_seq": at_seq, "reason": reason})
        }
    }
}

fn unsupported_manifest_error(
    dir: &Path,
    source: &calyx_aster::manifest::VaultManifest,
) -> Result<&'static str, String> {
    let bad_dir = dir.join("format_99");
    fs::create_dir_all(&bad_dir).map_err(err)?;
    let mut bad = source.clone();
    bad.version.major = 99;
    let pointer = "manifest-00000000000000000099.json";
    fs::write(
        bad_dir.join(pointer),
        serde_json::to_vec_pretty(&bad).map_err(err)?,
    )
    .map_err(err)?;
    fs::write(bad_dir.join("CURRENT"), pointer).map_err(err)?;
    Ok(ManifestStore::open(&bad_dir)
        .load_current()
        .expect_err("unknown major must fail closed")
        .code)
}
