//! Durable lazy backfill scheduler state.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, CxId, LensId, Result, SlotId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillPriority {
    Normal,
    Hot,
    Kernel,
}

impl BackfillPriority {
    const fn rank(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Hot => 1,
            Self::Kernel => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillRequest {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub priority: BackfillPriority,
    pub candidates: Vec<CxId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillConfig {
    pub max_concurrent: usize,
    pub batch_size: usize,
    pub throttle_ms: u64,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            batch_size: 16,
            throttle_ms: 50,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillBatch {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub candidates: Vec<CxId>,
    pub throttled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillWatermark {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub priority: BackfillPriority,
    pub processed: usize,
    pub pending: usize,
    pub in_flight: usize,
    pub complete: bool,
    pub last_processed: Option<CxId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedScheduler {
    config: BackfillConfig,
    next_allowed_ms: u64,
    requests: BTreeMap<String, RequestState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RequestState {
    request: BackfillRequest,
    next_index: usize,
    in_flight: Vec<CxId>,
    last_processed: Option<CxId>,
    complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackfillScheduler {
    path: PathBuf,
    state: PersistedScheduler,
}

impl BackfillScheduler {
    pub fn open(path: impl Into<PathBuf>, config: BackfillConfig) -> Result<Self> {
        let path = path.into();
        if path.exists() {
            let bytes = fs::read(&path).map_err(|err| io_error(&path, err))?;
            let mut state: PersistedScheduler = serde_json::from_slice(&bytes)
                .map_err(|err| CalyxError::stale_derived(err.to_string()))?;
            let needs_persist = state.config != config
                || state
                    .requests
                    .values()
                    .any(|request| !request.in_flight.is_empty());
            state.config = config;
            for request in state.requests.values_mut() {
                request.in_flight.clear();
            }
            let scheduler = Self { path, state };
            if needs_persist {
                scheduler.persist()?;
            }
            return Ok(scheduler);
        }
        Ok(Self {
            path,
            state: PersistedScheduler {
                config,
                next_allowed_ms: 0,
                requests: BTreeMap::new(),
            },
        })
    }

    pub fn enqueue(&mut self, request: BackfillRequest) -> Result<()> {
        self.mutate_and_persist(|state| {
            let key = request_key(request.slot_id, request.lens_id);
            state.requests.entry(key).or_insert_with(|| RequestState {
                request,
                next_index: 0,
                in_flight: Vec::new(),
                last_processed: None,
                complete: false,
            });
            Ok(())
        })
    }

    pub fn claim_next_batch(&mut self, now_ms: u64) -> Result<Option<BackfillBatch>> {
        if now_ms < self.state.next_allowed_ms {
            return Ok(Some(BackfillBatch {
                slot_id: SlotId::new(0),
                lens_id: LensId::from_bytes([0; 16]),
                candidates: Vec::new(),
                throttled: true,
            }));
        }
        if self.active_count() >= self.state.config.max_concurrent.max(1) {
            return Ok(None);
        }
        let Some(key) = self.next_request_key() else {
            return Ok(None);
        };
        let batch_size = self.state.config.batch_size.max(1);
        self.mutate_and_persist(|state| {
            let state = state.requests.get_mut(&key).expect("key selected from map");
            let start = state.next_index;
            let end = (start + batch_size).min(state.request.candidates.len());
            if start >= end {
                state.complete = true;
                return Ok(None);
            }
            state.in_flight = state.request.candidates[start..end].to_vec();
            Ok(Some(BackfillBatch {
                slot_id: state.request.slot_id,
                lens_id: state.request.lens_id,
                candidates: state.in_flight.clone(),
                throttled: false,
            }))
        })
    }

    pub fn complete_batch(&mut self, slot_id: SlotId, lens_id: LensId, now_ms: u64) -> Result<()> {
        self.mutate_and_persist(|state| {
            let key = request_key(slot_id, lens_id);
            let request = state.requests.get_mut(&key).ok_or_else(|| {
                CalyxError::stale_derived(format!("backfill request {key} missing"))
            })?;
            if request.in_flight.is_empty() {
                return Err(CalyxError::stale_derived(format!(
                    "backfill request {key} has no in-flight batch"
                )));
            }
            request.next_index += request.in_flight.len();
            request.last_processed = request.in_flight.last().copied();
            request.in_flight.clear();
            request.complete = request.next_index >= request.request.candidates.len();
            state.next_allowed_ms = now_ms.saturating_add(state.config.throttle_ms);
            Ok(())
        })
    }

    pub fn watermarks(&self) -> Vec<BackfillWatermark> {
        self.state
            .requests
            .values()
            .map(|state| {
                let total = state.request.candidates.len();
                BackfillWatermark {
                    slot_id: state.request.slot_id,
                    lens_id: state.request.lens_id,
                    priority: state.request.priority,
                    processed: state.next_index,
                    pending: total.saturating_sub(state.next_index),
                    in_flight: state.in_flight.len(),
                    complete: state.complete,
                    last_processed: state.last_processed,
                }
            })
            .collect()
    }

    pub fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| io_error(parent, err))?;
        }
        let bytes = serde_json::to_vec_pretty(&self.state)
            .map_err(|err| CalyxError::stale_derived(err.to_string()))?;
        atomic_write(&self.path, &bytes)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn mutate_and_persist<T>(
        &mut self,
        mutate: impl FnOnce(&mut PersistedScheduler) -> Result<T>,
    ) -> Result<T> {
        let before = self.state.clone();
        let output = match mutate(&mut self.state) {
            Ok(output) => output,
            Err(error) => {
                self.state = before;
                return Err(error);
            }
        };
        if let Err(error) = self.persist() {
            self.state = before;
            if let Err(rollback_error) = self.persist() {
                return Err(CalyxError::stale_derived(format!(
                    "scheduler persist failed: {error}; rollback persist failed: {rollback_error}"
                )));
            }
            return Err(error);
        }
        Ok(output)
    }

    fn active_count(&self) -> usize {
        self.state
            .requests
            .values()
            .filter(|state| !state.in_flight.is_empty())
            .count()
    }

    fn next_request_key(&self) -> Option<String> {
        self.state
            .requests
            .iter()
            .filter(|(_, state)| {
                !state.complete
                    && state.in_flight.is_empty()
                    && state.next_index < state.request.candidates.len()
            })
            .max_by_key(|(_, state)| {
                (
                    state.request.priority.rank(),
                    std::cmp::Reverse(state.next_index),
                )
            })
            .map(|(key, _)| key.clone())
    }
}

fn request_key(slot_id: SlotId, lens_id: LensId) -> String {
    format!("{slot_id}:{lens_id}")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = temp_path(path)?;
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|err| io_error(&tmp, err))?;
        file.write_all(bytes).map_err(|err| io_error(&tmp, err))?;
        file.sync_all().map_err(|err| io_error(&tmp, err))?;
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_error(path, err));
    }
    injected_post_rename_failure(path)?;
    sync_parent(parent)
}

fn temp_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CalyxError::stale_derived(format!("invalid scheduler path {}", path.display()))
        })?;
    Ok(path.with_file_name(format!(".{name}.tmp-{}", std::process::id())))
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|err| io_error(parent, err))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(debug_assertions)]
fn injected_post_rename_failure(path: &Path) -> Result<()> {
    let marker = post_rename_failure_marker(path)?;
    if !marker.exists() {
        return Ok(());
    }
    fs::remove_file(&marker).map_err(|err| io_error(&marker, err))?;
    Err(CalyxError::stale_derived(format!(
        "{}: injected post-rename persist failure",
        path.display()
    )))
}

#[cfg(debug_assertions)]
fn post_rename_failure_marker(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CalyxError::stale_derived(format!("invalid scheduler path {}", path.display()))
        })?;
    Ok(path.with_file_name(format!(".{name}.fail-after-rename-once")))
}

#[cfg(not(debug_assertions))]
fn injected_post_rename_failure(_path: &Path) -> Result<()> {
    Ok(())
}

fn io_error(path: &Path, err: std::io::Error) -> CalyxError {
    CalyxError::stale_derived(format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_scheduler_orders_throttles_and_resumes() {
        let path = test_path("durable_scheduler_orders_throttles_and_resumes");
        let _ = fs::remove_file(&path);
        let mut scheduler = BackfillScheduler::open(
            &path,
            BackfillConfig {
                max_concurrent: 1,
                batch_size: 2,
                throttle_ms: 10,
            },
        )
        .unwrap();
        scheduler
            .enqueue(request(1, BackfillPriority::Normal, 3))
            .unwrap();
        scheduler
            .enqueue(request(2, BackfillPriority::Kernel, 2))
            .unwrap();

        let first = scheduler.claim_next_batch(100).unwrap().unwrap();
        assert_eq!(first.slot_id, SlotId::new(2));
        assert_eq!(first.candidates.len(), 2);
        scheduler
            .complete_batch(first.slot_id, first.lens_id, 100)
            .unwrap();
        assert!(scheduler.claim_next_batch(105).unwrap().unwrap().throttled);

        let reopened = BackfillScheduler::open(
            &path,
            BackfillConfig {
                max_concurrent: 1,
                batch_size: 2,
                throttle_ms: 10,
            },
        )
        .unwrap();
        let marks = reopened.watermarks();
        let kernel = marks
            .iter()
            .find(|mark| mark.slot_id == SlotId::new(2))
            .unwrap();
        assert!(kernel.complete);
        assert_eq!(kernel.processed, 2);
    }

    #[test]
    fn claimed_uncompleted_batch_is_retried_after_reopen() {
        let path = test_path("claimed_uncompleted_batch_is_retried_after_reopen");
        let _ = fs::remove_file(&path);
        let mut scheduler = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
        scheduler
            .enqueue(request(7, BackfillPriority::Hot, 2))
            .unwrap();
        let first = scheduler.claim_next_batch(0).unwrap().unwrap();
        assert_eq!(first.candidates.len(), 2);

        let mut reopened = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
        let retry = reopened.claim_next_batch(0).unwrap().unwrap();
        assert_eq!(retry.candidates, first.candidates);
    }

    #[test]
    fn corrupt_scheduler_state_fails_closed() {
        let path = test_path("corrupt_scheduler_state_fails_closed");
        let _ = fs::remove_file(&path);
        fs::write(&path, b"{").unwrap();

        let error = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap_err();

        assert_eq!(error.code, "CALYX_STALE_DERIVED");
    }

    #[test]
    fn post_rename_persist_failure_rolls_back_file_and_state() {
        let path = test_path("post_rename_persist_failure_rolls_back_file_and_state");
        let _ = fs::remove_file(&path);
        let mut scheduler = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
        scheduler
            .enqueue(request(1, BackfillPriority::Normal, 1))
            .unwrap();
        let before_bytes = fs::read(&path).unwrap();
        let before_marks = scheduler.watermarks();
        fs::write(post_rename_failure_marker(&path).unwrap(), b"fail-once").unwrap();

        let error = scheduler
            .enqueue(request(2, BackfillPriority::Kernel, 2))
            .unwrap_err();
        let after_bytes = fs::read(&path).unwrap();
        let reopened = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();

        assert_eq!(error.code, "CALYX_STALE_DERIVED");
        assert_eq!(scheduler.watermarks(), before_marks);
        assert_eq!(reopened.watermarks(), before_marks);
        assert_eq!(after_bytes, before_bytes);
    }

    fn request(slot: u16, priority: BackfillPriority, count: u8) -> BackfillRequest {
        BackfillRequest {
            slot_id: SlotId::new(slot),
            lens_id: LensId::from_bytes([slot as u8; 16]),
            priority,
            candidates: (0..count).map(|idx| CxId::from_bytes([idx; 16])).collect(),
        }
    }

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("calyx-{name}-{}.json", std::process::id()))
    }
}
