use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use calyx_core::Clock;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use super::{AutotuneCache, AutotuneKey, cache_error};
use crate::{BackendKind, BestConfig, Result};

const CLOCK_MS_TO_NS: u64 = 1_000_000;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbHook {
    pub rate: f64,
}

pub fn log_promotion(event: &PromotionEvent, log_path: &Path) -> Result<()> {
    let mut line = serde_json::to_vec(event).map_err(|err| {
        cache_error(
            "promotion_log",
            log_path,
            format!("serialize failed: {err}"),
        )
    })?;
    line.push(b'\n');
    // PH16 owns a local append-only audit stub; real Ledger wiring is cross-engine work.
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        file.write_all(&line)?;
        file.sync_all()
    })();
    write_result
        .map_err(|err| cache_error("promotion_log", log_path, format!("append failed: {err}")))
}

pub fn rollback_promotion(
    cache: &mut AutotuneCache,
    log_path: &Path,
    key: &AutotuneKey,
    clock: &dyn Clock,
) -> Result<Option<BestConfig>> {
    let Some(event) = last_promoted_event(log_path, key)? else {
        return Ok(None);
    };
    cache.rollback(key, event.old_config.clone());
    let demoted = event.new_config.clone();
    let rollback = PromotionEvent {
        key: key.clone(),
        old_config: demoted.clone(),
        new_config: event.old_config,
        timestamp_ns: clock.now().saturating_mul(CLOCK_MS_TO_NS),
        action: PromotionAction::RolledBack,
    };
    log_promotion(&rollback, log_path)?;
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

fn last_promoted_event(log_path: &Path, key: &AutotuneKey) -> Result<Option<PromotionEvent>> {
    let mut events = read_log(log_path)?;
    events.reverse();
    Ok(events
        .into_iter()
        .find(|event| event.key == *key && event.action == PromotionAction::Promoted))
}

fn read_log(log_path: &Path) -> Result<Vec<PromotionEvent>> {
    let raw = match fs::read_to_string(log_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(cache_error(
                "promotion_read",
                log_path,
                format!("read failed: {err}"),
            ));
        }
    };
    let mut events = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str(line).map_err(|err| {
            cache_error(
                "promotion_parse",
                log_path,
                format!("line {} malformed JSONL: {err}; content={line}", idx + 1),
            )
        })?;
        events.push(event);
    }
    Ok(events)
}
