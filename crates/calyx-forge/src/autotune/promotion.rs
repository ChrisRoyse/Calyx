use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AutotuneCache, AutotuneKey, cache_error};
use crate::{BackendKind, BestConfig, ForgeError, Result};

const CLOCK_MS_TO_NS: u64 = 1_000_000;
const LEDGER_REMEDIATION: &str = "inspect the calyx-ledger row store, verify the append-only chain, and rerun Forge autotune promotion after repairing ledger corruption";
const PROMOTION_LEDGER_SUBJECT_PREFIX: &[u8] = b"calyx-forge-autotune-promotion\0";
pub const PROMOTION_LEDGER_SCHEMA_VERSION: &str = "calyx.forge.autotune.promotion.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromotionEvent {
    pub key: AutotuneKey,
    pub old_config: BestConfig,
    pub new_config: BestConfig,
    pub timestamp_ns: u64,
    pub action: PromotionAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PromotionAction {
    Promoted,
    RolledBack,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PromotionLedgerPayload {
    schema_version: String,
    action: PromotionAction,
    autotune_selector: AutotuneKey,
    old_config: BestConfig,
    new_config: BestConfig,
    timestamp_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbHook {
    pub rate: f64,
}

pub fn log_promotion<S, C>(
    event: &PromotionEvent,
    ledger: &mut LedgerAppender<S, C>,
    actor: ActorId,
    jsonl_export_path: Option<&Path>,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    let payload = PromotionLedgerPayload {
        schema_version: PROMOTION_LEDGER_SCHEMA_VERSION.to_string(),
        action: event.action,
        autotune_selector: event.key.clone(),
        old_config: event.old_config.clone(),
        new_config: event.new_config.clone(),
        timestamp_ns: event.timestamp_ns,
    };
    let bytes = serde_json::to_vec(&payload)
        .map_err(|err| ledger_error("promotion_ledger_serialize", err))?;
    let ledger_ref = ledger
        .append(
            EntryKind::Anneal,
            promotion_ledger_subject(&event.key)?,
            bytes,
            actor,
        )
        .map_err(|err| ledger_error("promotion_ledger_append", err))?;
    if let Some(path) = jsonl_export_path {
        export_promotion_jsonl(event, path)?;
    }
    Ok(ledger_ref)
}

pub fn promotion_ledger_events<S, C>(ledger: &LedgerAppender<S, C>) -> Result<Vec<PromotionEvent>>
where
    S: LedgerCfStore,
    C: Clock,
{
    let mut events = Vec::new();
    for entry in ledger
        .scan_entries()
        .map_err(|err| ledger_error("promotion_ledger_scan", err))?
    {
        if entry.kind != EntryKind::Anneal || !is_promotion_subject(&entry.subject) {
            continue;
        }
        events.push(decode_promotion_ledger_payload(&entry.payload)?);
    }
    Ok(events)
}

pub fn decode_promotion_ledger_payload(payload: &[u8]) -> Result<PromotionEvent> {
    let payload: PromotionLedgerPayload = serde_json::from_slice(payload)
        .map_err(|err| ledger_error("promotion_ledger_parse", err))?;
    if payload.schema_version != PROMOTION_LEDGER_SCHEMA_VERSION {
        return Err(ledger_error(
            "promotion_ledger_schema",
            format!(
                "expected schema {} got {}",
                PROMOTION_LEDGER_SCHEMA_VERSION, payload.schema_version
            ),
        ));
    }
    Ok(PromotionEvent {
        key: payload.autotune_selector,
        old_config: payload.old_config,
        new_config: payload.new_config,
        timestamp_ns: payload.timestamp_ns,
        action: payload.action,
    })
}

pub fn promotion_ledger_subject(key: &AutotuneKey) -> Result<SubjectId> {
    let encoded =
        serde_json::to_vec(key).map_err(|err| ledger_error("promotion_subject_serialize", err))?;
    let digest = Sha256::digest(encoded);
    let mut subject = Vec::with_capacity(PROMOTION_LEDGER_SUBJECT_PREFIX.len() + digest.len());
    subject.extend_from_slice(PROMOTION_LEDGER_SUBJECT_PREFIX);
    subject.extend_from_slice(&digest);
    Ok(SubjectId::Kernel(subject))
}

fn export_promotion_jsonl(event: &PromotionEvent, log_path: &Path) -> Result<()> {
    let mut line = serde_json::to_vec(event).map_err(|err| {
        cache_error(
            "promotion_jsonl_export",
            log_path,
            format!("serialize failed: {err}"),
        )
    })?;
    line.push(b'\n');
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        file.write_all(&line)?;
        file.sync_all()
    })();
    write_result.map_err(|err| {
        cache_error(
            "promotion_jsonl_export",
            log_path,
            format!("append failed: {err}"),
        )
    })
}

pub fn rollback_promotion<S, C>(
    cache: &mut AutotuneCache,
    ledger: &mut LedgerAppender<S, C>,
    key: &AutotuneKey,
    clock: &dyn Clock,
    actor: ActorId,
    jsonl_export_path: Option<&Path>,
) -> Result<Option<BestConfig>>
where
    S: LedgerCfStore,
    C: Clock,
{
    let Some(event) = last_promoted_event(ledger, key)? else {
        return Ok(None);
    };
    let demoted = event.new_config.clone();
    let old_config = event.old_config.clone();
    let rollback = PromotionEvent {
        key: key.clone(),
        old_config: demoted.clone(),
        new_config: old_config.clone(),
        timestamp_ns: clock.now().saturating_mul(CLOCK_MS_TO_NS),
        action: PromotionAction::RolledBack,
    };
    log_promotion(&rollback, ledger, actor, jsonl_export_path)?;
    cache.rollback(key, old_config);
    Ok(Some(demoted))
}

pub fn should_use_challenger(hook: &AbHook, rng: &mut ChaCha8Rng) -> bool {
    if !hook.rate.is_finite() || hook.rate <= 0.0 {
        return false;
    }
    if hook.rate >= 1.0 {
        return true;
    }
    rng.gen_range(0.0..1.0) < hook.rate
}

pub fn autotune(cache: &AutotuneCache, key: &AutotuneKey) -> BestConfig {
    cache
        .get(key)
        .cloned()
        .unwrap_or_else(|| BestConfig::default_for(key))
}

impl BestConfig {
    pub fn default_for(key: &AutotuneKey) -> Self {
        let backend = if cfg!(feature = "cuda") {
            BackendKind::Cuda
        } else {
            BackendKind::Cpu
        };
        Self {
            backend,
            tile_m: 64,
            tile_n: 64,
            tile_k: 32,
            extra: HashMap::from([
                ("op".to_string(), key.op.clone()),
                ("source".to_string(), "autotune-default".to_string()),
            ]),
        }
    }
}

fn last_promoted_event<S, C>(
    ledger: &LedgerAppender<S, C>,
    key: &AutotuneKey,
) -> Result<Option<PromotionEvent>>
where
    S: LedgerCfStore,
    C: Clock,
{
    let mut events = promotion_ledger_events(ledger)?;
    events.reverse();
    Ok(events
        .into_iter()
        .find(|event| event.key == *key && event.action == PromotionAction::Promoted))
}

fn is_promotion_subject(subject: &SubjectId) -> bool {
    matches!(
        subject,
        SubjectId::Kernel(bytes) if bytes.starts_with(PROMOTION_LEDGER_SUBJECT_PREFIX)
    )
}

fn ledger_error(op: &str, detail: impl ToString) -> ForgeError {
    ForgeError::LedgerError {
        op: op.to_string(),
        detail: detail.to_string(),
        remediation: LEDGER_REMEDIATION.to_string(),
    }
}
