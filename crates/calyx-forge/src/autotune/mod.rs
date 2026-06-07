use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{BestConfig, ForgeError, Result};

mod explorer;
mod microbench;
mod promotion;
pub use explorer::{
    EPSILON, Explorer, ExplorerPolicy, MIN_PROMOTE_MARGIN, MIN_PROMOTE_TRIALS, next_candidate,
    promote_if_winner, record_trial, should_promote,
};
pub use microbench::{BenchCudaContext, BenchResult, microbench};
pub use promotion::{
    AbHook, PromotionAction, PromotionEvent, autotune, log_promotion, rollback_promotion,
    should_use_challenger,
};

const CACHE_REMEDIATION: &str = "Use a readable same-filesystem JSON cache path, or discard the corrupt cache and rerun autotune";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutotuneKey {
    pub op: String,
    pub shape: Vec<usize>,
    pub dtype: String,
    pub device: String,
    pub recall_tgt: f32,
}

impl AutotuneKey {
    pub fn default_for(op: &str, shape: &[usize], dtype: &str, device: &str) -> Self {
        Self {
            op: op.to_string(),
            shape: shape.to_vec(),
            dtype: dtype.to_string(),
            device: device.to_string(),
            recall_tgt: 0.95,
        }
    }

    pub fn recall_quantum(&self) -> i32 {
        quantize_recall(self.recall_tgt)
    }
}

impl PartialEq for AutotuneKey {
    fn eq(&self, other: &Self) -> bool {
        self.op == other.op
            && self.shape == other.shape
            && self.dtype == other.dtype
            && self.device == other.device
            && self.recall_quantum() == other.recall_quantum()
    }
}

impl Eq for AutotuneKey {}

impl Hash for AutotuneKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.op.hash(state);
        self.shape.hash(state);
        self.dtype.hash(state);
        self.device.hash(state);
        self.recall_quantum().hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct AutotuneCache {
    entries: HashMap<AutotuneKey, BestConfig>,
    path: PathBuf,
}

impl AutotuneCache {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Self::from_bytes(path, &bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self {
                entries: HashMap::new(),
                path: path.to_path_buf(),
            }),
            Err(err) => Err(cache_error("load", path, format!("read failed: {err}"))),
        }
    }

    pub fn get(&self, key: &AutotuneKey) -> Option<&BestConfig> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: AutotuneKey, config: BestConfig) {
        self.entries.insert(key, config);
    }

    pub fn persist(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.persisted()).map_err(|err| {
            cache_error("persist", &self.path, format!("serialize failed: {err}"))
        })?;
        let tmp = tmp_path_for(&self.path)?;
        write_tmp(&tmp, &bytes)?;
        fs::rename(&tmp, &self.path).map_err(|err| {
            let _ = fs::remove_file(&tmp);
            cache_error(
                "persist",
                &self.path,
                format!(
                    "atomic rename {} -> {} failed: {err}",
                    tmp.display(),
                    self.path.display()
                ),
            )
        })
    }

    pub fn rollback(&mut self, key: &AutotuneKey, previous: BestConfig) {
        self.entries.insert(key.clone(), previous);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self> {
        let persisted: PersistedCache = serde_json::from_slice(bytes)
            .map_err(|err| cache_error("load", path, format!("malformed JSON: {err}")))?;
        let mut entries = HashMap::with_capacity(persisted.entries.len());
        for entry in persisted.entries {
            entries.insert(entry.key, entry.config);
        }
        Ok(Self {
            entries,
            path: path.to_path_buf(),
        })
    }

    fn persisted(&self) -> PersistedCache {
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(key, config)| PersistedEntry {
                key: key.clone(),
                config: config.clone(),
            })
            .collect();
        entries.sort_by(|left, right| entry_sort_key(left).cmp(&entry_sort_key(right)));
        PersistedCache { entries }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistedCache {
    entries: Vec<PersistedEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistedEntry {
    key: AutotuneKey,
    config: BestConfig,
}

fn quantize_recall(recall_tgt: f32) -> i32 {
    if recall_tgt.is_nan() {
        return i32::MIN;
    }
    let scaled = (recall_tgt * 100.0).round();
    if scaled >= i32::MAX as f32 {
        i32::MAX
    } else if scaled <= i32::MIN as f32 {
        i32::MIN + 1
    } else {
        scaled as i32
    }
}

fn entry_sort_key(entry: &PersistedEntry) -> (&str, &[usize], &str, &str, i32) {
    (
        &entry.key.op,
        &entry.key.shape,
        &entry.key.dtype,
        &entry.key.device,
        entry.key.recall_quantum(),
    )
}

fn tmp_path_for(path: &Path) -> Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        cache_error(
            "persist",
            path,
            "cache path must include a file name for same-directory temp writes",
        )
    })?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    Ok(path.with_file_name(tmp_name))
}

fn write_tmp(path: &Path, bytes: &[u8]) -> Result<()> {
    let write_result = (|| {
        let mut file = fs::File::create(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    write_result.map_err(|err| {
        let _ = fs::remove_file(path);
        cache_error("persist", path, format!("write failed: {err}"))
    })
}

fn cache_error(op: &str, path: &Path, detail: impl Into<String>) -> ForgeError {
    ForgeError::CacheError {
        op: op.to_string(),
        path: path.display().to_string(),
        detail: detail.into(),
        remediation: CACHE_REMEDIATION.to_string(),
    }
}

#[cfg(test)]
mod explorer_tests;
#[cfg(test)]
mod microbench_tests;
#[cfg(test)]
mod promotion_tests;
#[cfg(test)]
mod tests;
