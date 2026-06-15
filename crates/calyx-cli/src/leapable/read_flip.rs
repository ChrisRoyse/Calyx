use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{CalyxError, LedgerRef, SlotVector, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::{Deserialize, Serialize};

use super::dual_write::aster_dir;
use super::panel_guard_enable::{PanelGuardEnable, PanelSpec};
use super::shadow_harness::read_shadow_manifest;
use super::{ShadowVault, VaultMode};
use crate::error::{CliError, CliResult};
use crate::migrate::adapter::{BASE_SLOT, METADATA_GTE_LENS_ID};
use crate::migrate::manifest::{MigrationManifest, hex_decode, hex_encode};
use crate::migrate::{self};
use crate::output::print_json;

pub(crate) const CALYX_VAULT_FLIP_FAILED: &str = "CALYX_VAULT_FLIP_FAILED";
pub(crate) const CALYX_INVALID_ASK_QUERY: &str = "CALYX_INVALID_ASK_QUERY";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FlipReceipt {
    pub(crate) database_name: String,
    pub(crate) flipped_at_seq: u64,
    pub(crate) panel_lens_count: usize,
    pub(crate) kernel_enabled: bool,
    pub(crate) guard_enabled: bool,
    pub(crate) ledger_ref: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LensProvenance {
    pub(crate) slot_id: u16,
    pub(crate) lens_id: String,
    pub(crate) ledger_ref: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Hit {
    pub(crate) rank: usize,
    pub(crate) chunk_id: String,
    pub(crate) database_name: String,
    pub(crate) cx_id: String,
    pub(crate) score: f64,
    pub(crate) ledger_ref: LedgerRef,
    pub(crate) per_lens_provenance: Vec<LensProvenance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AskResult {
    pub(crate) mode: VaultMode,
    pub(crate) top_k: usize,
    pub(crate) hits: Vec<Hit>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ReadFlipReport {
    sqlite_path: PathBuf,
    calyx_dir: PathBuf,
    manifest_before: super::shadow_harness::ShadowManifestReadback,
    panel: super::panel_guard_enable::PanelEnableReport,
    receipt: FlipReceipt,
    manifest_after: super::shadow_harness::ShadowManifestReadback,
    sqlite_contract_tables: Vec<String>,
    gate: String,
}

pub(crate) struct ReadFlip;

impl ReadFlip {
    pub(crate) fn execute(vault: &mut ShadowVault) -> Result<FlipReceipt, CalyxError> {
        if vault.mode() >= VaultMode::Calyx
            && let Some(receipt) = existing_receipt(vault)?
        {
            return Ok(receipt);
        }
        let (_sqlite_path, calyx_dir) = vault.paths();
        let (aster, manifest) = open_aster(calyx_dir)?;
        let features = vault.manifest_readback()?.features;
        let panel_lens_count = parse_usize(&features, "panel_lens_count").unwrap_or(0);
        let kernel_enabled = feature_bool(&features, "kernel_enabled");
        let guard_enabled = feature_bool(&features, "guard_enabled");
        let payload = serde_json::to_vec(&serde_json::json!({
            "event": "leapable_read_flip_v1",
            "database_name": vault.vault_name(),
            "panel_lens_count": panel_lens_count,
            "kernel_enabled": kernel_enabled,
            "guard_enabled": guard_enabled,
            "vault_id": manifest.vault_id,
        }))
        .map_err(|error| flip_failed(format!("encode flip receipt payload: {error}")))?;
        let subject = SubjectId::Query(
            blake3::hash(vault.vault_name().as_bytes())
                .as_bytes()
                .to_vec(),
        );
        let ledger_ref = aster.append_ledger_entry(
            EntryKind::Admin,
            subject,
            payload,
            ActorId::Service("calyx-leapable-read-flip".to_string()),
        )?;
        aster.flush()?;
        let receipt = FlipReceipt {
            database_name: vault.vault_name().to_string(),
            flipped_at_seq: aster.latest_seq(),
            panel_lens_count,
            kernel_enabled,
            guard_enabled,
            ledger_ref,
        };
        vault
            .set_mode_with_features(VaultMode::Calyx, &receipt_features(&receipt))
            .map_err(|error| flip_failed(error.message))?;
        Ok(receipt)
    }
}

#[cfg(test)]
impl ShadowVault {
    pub(crate) fn ask(&self, query_vec: &[f32], top_k: usize) -> Result<AskResult, CalyxError> {
        PanelGuardEnable::ensure_flipped(self)?;
        ask_calyx(self.paths().1, self.mode(), query_vec, top_k)
    }
}

pub(crate) fn run_read_flip(args: &[String]) -> CliResult {
    let args = parse_flip_args(args)?;
    PanelGuardEnable::validate_guard_tau(args.tau)?;
    let mut vault = ShadowVault::open(&args.sqlite, &args.calyx)?;
    let before = vault.manifest_readback()?;
    let panel = PanelGuardEnable::enable(
        &mut vault,
        &PanelSpec {
            backfill: !args.skip_backfill,
            ..PanelSpec::default()
        },
    )?;
    PanelGuardEnable::enable_kernel(&mut vault)?;
    PanelGuardEnable::enable_guard(&mut vault, args.tau)?;
    let receipt = ReadFlip::execute(&mut vault)?;
    let after = vault.manifest_readback()?;
    let sqlite_contract_tables = vault
        .verify_pg_contract()?
        .tables
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let report = ReadFlipReport {
        sqlite_path: args.sqlite,
        calyx_dir: args.calyx,
        manifest_before: before,
        panel,
        receipt,
        manifest_after: after,
        sqlite_contract_tables,
        gate: "PASS".to_string(),
    };
    print_json(&report)?;
    vault.close()?;
    Ok(())
}

pub(crate) fn run_ask(args: &[String]) -> CliResult {
    let args = parse_ask_args(args)?;
    let query_vec = match args.query {
        AskQuery::Vector(vector) => vector,
        AskQuery::Text(text) => text_query_vector(&text, base_dim(&args.vault)?),
    };
    let manifest = read_shadow_manifest(&args.vault)?;
    if manifest.mode < VaultMode::Calyx {
        return Err(error(
            "CALYX_VAULT_NOT_FLIPPED",
            "Ask is still routed to sqlite-vec shadow mode",
            "run calyx leapable read-flip before asking through Calyx",
        )
        .into());
    }
    let result = ask_calyx(&args.vault, manifest.mode, &query_vec, args.top_k)?;
    print_json(&result)
}

pub(crate) fn ask_calyx(
    calyx_dir: &Path,
    mode: VaultMode,
    query_vec: &[f32],
    top_k: usize,
) -> Result<AskResult, CalyxError> {
    validate_query(query_vec, top_k)?;
    let (aster, _manifest) = open_aster(calyx_dir)?;
    let snapshot = aster.snapshot();
    let mut hits = Vec::new();
    for (_key, bytes) in aster.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let cx = decode_constellation_base(&bytes)?;
        let Some(SlotVector::Dense { dim, data }) =
            aster.read_slot_vector_at(snapshot, cx.cx_id, BASE_SLOT)?
        else {
            continue;
        };
        if dim as usize != query_vec.len() || data.len() != query_vec.len() {
            return Err(error(
                CALYX_INVALID_ASK_QUERY,
                format!(
                    "query dim {} does not match base slot dim {dim}",
                    query_vec.len()
                ),
                "ask with a finite query vector matching the vault base slot dimension",
            ));
        }
        let chunk_id = cx.chunk_id().unwrap_or("").to_string();
        let database_name = cx.database_name().unwrap_or("").to_string();
        let lens_id = cx
            .metadata
            .get(METADATA_GTE_LENS_ID)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        hits.push(Hit {
            rank: 0,
            chunk_id,
            database_name,
            cx_id: cx.cx_id.to_string(),
            score: cosine(query_vec, &data),
            ledger_ref: cx.provenance.clone(),
            per_lens_provenance: vec![LensProvenance {
                slot_id: BASE_SLOT.get(),
                lens_id,
                ledger_ref: cx.provenance,
            }],
        });
    }
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    hits.truncate(top_k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
    Ok(AskResult { mode, top_k, hits })
}

fn open_aster(
    calyx_dir: &Path,
) -> Result<(calyx_aster::vault::AsterVault, MigrationManifest), CalyxError> {
    let aster = aster_dir(calyx_dir);
    let manifest = MigrationManifest::load(&aster)?;
    let vault = migrate::open_vault(&aster, &manifest)?;
    Ok((vault, manifest))
}

fn existing_receipt(vault: &ShadowVault) -> Result<Option<FlipReceipt>, CalyxError> {
    let readback = vault.manifest_readback()?;
    let Some(flipped_at_seq) = parse_u64(&readback.features, "flipped_at_seq") else {
        return Ok(None);
    };
    let ledger_ref = LedgerRef {
        seq: parse_u64(&readback.features, "flip_ledger_seq").unwrap_or(0),
        hash: parse_hash(&readback.features, "flip_ledger_hash").unwrap_or([0; 32]),
    };
    Ok(Some(FlipReceipt {
        database_name: readback.database_name,
        flipped_at_seq,
        panel_lens_count: parse_usize(&readback.features, "panel_lens_count").unwrap_or(0),
        kernel_enabled: feature_bool(&readback.features, "kernel_enabled"),
        guard_enabled: feature_bool(&readback.features, "guard_enabled"),
        ledger_ref,
    }))
}

fn receipt_features(receipt: &FlipReceipt) -> Vec<(&'static str, String)> {
    vec![
        ("read_path", "calyx".to_string()),
        ("sqlite_vec_role", "shadow-write-only".to_string()),
        ("flipped_at_seq", receipt.flipped_at_seq.to_string()),
        ("flip_ledger_seq", receipt.ledger_ref.seq.to_string()),
        ("flip_ledger_hash", hex_encode(&receipt.ledger_ref.hash)),
        ("panel_lens_count", receipt.panel_lens_count.to_string()),
        ("kernel_enabled", receipt.kernel_enabled.to_string()),
        ("guard_enabled", receipt.guard_enabled.to_string()),
    ]
}

fn validate_query(query_vec: &[f32], top_k: usize) -> Result<(), CalyxError> {
    if top_k == 0
        || query_vec.is_empty()
        || query_vec.iter().any(|value| !value.is_finite())
        || norm(query_vec) == 0.0
    {
        return Err(error(
            CALYX_INVALID_ASK_QUERY,
            "query vector must be finite, nonzero, and top_k must be greater than zero",
            "ask with --top-k >= 1 and a finite nonzero query vector",
        ));
    }
    Ok(())
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    dot / (norm(left) * norm(right)).max(f64::MIN_POSITIVE)
}

fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

pub(crate) fn text_query_vector(text: &str, dim: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim);
    let mut counter = 0_u32;
    while out.len() < dim {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"calyx-leapable-ask-text-v1");
        hasher.update(text.as_bytes());
        hasher.update(&counter.to_be_bytes());
        for chunk in hasher.finalize().as_bytes().chunks_exact(4) {
            let raw = u32::from_be_bytes(chunk.try_into().expect("hash chunk"));
            out.push((raw as f32 / u32::MAX as f32) * 2.0 - 1.0);
            if out.len() == dim {
                break;
            }
        }
        counter = counter.saturating_add(1);
    }
    out
}

fn base_dim(calyx_dir: &Path) -> Result<usize, CalyxError> {
    let (aster, _manifest) = open_aster(calyx_dir)?;
    for (_key, bytes) in aster.scan_cf_at(aster.snapshot(), ColumnFamily::Base)? {
        let cx = decode_constellation_base(&bytes)?;
        if let Some(SlotVector::Dense { dim, .. }) =
            aster.read_slot_vector_at(aster.snapshot(), cx.cx_id, BASE_SLOT)?
        {
            return Ok(dim as usize);
        }
    }
    Err(error(
        CALYX_INVALID_ASK_QUERY,
        "vault has no dense base slot rows to infer text-query dimension",
        "dual-write at least one chunk before asking with --query",
    ))
}

fn parse_vector(value: &str) -> CliResult<Vec<f32>> {
    serde_json::from_str(value)
        .map_err(|error| CliError::usage(format!("parse --query-vector JSON array: {error}")))
}

fn parse_flip_args(args: &[String]) -> CliResult<FlipArgs> {
    let mut out = FlipArgs {
        tau: 0.72,
        ..FlipArgs::default()
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--sqlite" => out.sqlite = value(args, idx, "--sqlite")?.into(),
            "--calyx" => out.calyx = value(args, idx, "--calyx")?.into(),
            "--tau" => {
                out.tau = value(args, idx, "--tau")?
                    .parse()
                    .map_err(|error| CliError::usage(format!("parse --tau: {error}")))?;
            }
            "--skip-backfill" => {
                out.skip_backfill = true;
                idx += 1;
                continue;
            }
            other => return Err(CliError::usage(format!("unknown read-flip arg {other}"))),
        }
        idx += 2;
    }
    if out.sqlite.as_os_str().is_empty() || out.calyx.as_os_str().is_empty() {
        return Err(CliError::usage(
            "usage: calyx leapable read-flip --sqlite <db> --calyx <dir> [--tau <f>] [--skip-backfill]",
        ));
    }
    Ok(out)
}

fn parse_ask_args(args: &[String]) -> CliResult<AskArgs> {
    let mut vault = None;
    let mut query = None;
    let mut top_k = 5usize;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--vault" => vault = Some(value(args, idx, "--vault")?.into()),
            "--query-vector" => {
                query = Some(AskQuery::Vector(parse_vector(value(
                    args,
                    idx,
                    "--query-vector",
                )?)?))
            }
            "--query" => query = Some(AskQuery::Text(value(args, idx, "--query")?.to_string())),
            "--top-k" => {
                top_k = value(args, idx, "--top-k")?
                    .parse()
                    .map_err(|error| CliError::usage(format!("parse --top-k: {error}")))?;
            }
            other => return Err(CliError::usage(format!("unknown ask arg {other}"))),
        }
        idx += 2;
    }
    Ok(AskArgs {
        vault: vault.ok_or_else(|| CliError::usage("ask requires --vault <dir>"))?,
        query: query.ok_or_else(|| {
            CliError::usage("ask requires --query-vector <json-array> or --query <text>")
        })?,
        top_k,
    })
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

#[derive(Default)]
struct FlipArgs {
    sqlite: PathBuf,
    calyx: PathBuf,
    tau: f32,
    skip_backfill: bool,
}

struct AskArgs {
    vault: PathBuf,
    query: AskQuery,
    top_k: usize,
}

enum AskQuery {
    Vector(Vec<f32>),
    Text(String),
}

fn feature_bool(map: &std::collections::BTreeMap<String, String>, key: &str) -> bool {
    map.get(key).is_some_and(|value| value == "true")
}

fn parse_u64(map: &std::collections::BTreeMap<String, String>, key: &str) -> Option<u64> {
    map.get(key)?.parse().ok()
}

fn parse_usize(map: &std::collections::BTreeMap<String, String>, key: &str) -> Option<usize> {
    map.get(key)?.parse().ok()
}

fn parse_hash(map: &std::collections::BTreeMap<String, String>, key: &str) -> Option<[u8; 32]> {
    let bytes = hex_decode(map.get(key)?).ok()?;
    bytes.try_into().ok()
}

fn flip_failed(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_VAULT_FLIP_FAILED,
        message,
        "mode remains Shadow; inspect MANIFEST, Aster ledger CF, and retry read-flip",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
