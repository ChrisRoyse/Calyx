use calyx_core::{CalyxError, Result};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

static PROCESS_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();

pub(crate) struct FileLockGuard {
    _process_guard: MutexGuard<'static, ()>,
    _file: File,
}

impl FileLockGuard {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let key = lock_key(path)?;
        let process_guard = process_mutex(&key)
            .lock()
            .map_err(|_| CalyxError::backpressure("file lock mutex poisoned"))?;
        let existed = path.exists();
        let mut options = OpenOptions::new();
        options.create(true).read(true).append(true);
        crate::private_fs::harden_open_options(&mut options);
        let file = options
            .open(path)
            .map_err(|error| CalyxError::disk_pressure(format!("open lock file: {error}")))?;
        if !existed {
            crate::private_fs::enforce_private_file_mode(&file, path)?;
        }
        file.lock()
            .map_err(|error| CalyxError::backpressure(format!("lock file: {error}")))?;
        Ok(Self {
            _process_guard: process_guard,
            _file: file,
        })
    }
}

fn process_mutex(path: &Path) -> &'static Mutex<()> {
    let locks = PROCESS_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks.lock().expect("file lock registry poisoned");
    if let Some(lock) = locks.get(path) {
        return lock;
    }
    let lock = Box::leak(Box::new(Mutex::new(())));
    locks.insert(path.to_path_buf(), lock);
    lock
}

fn lock_key(path: &Path) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        crate::private_fs::create_private_dir_all(parent)?;
        let parent = parent.canonicalize().map_err(|error| {
            CalyxError::disk_pressure(format!("canonicalize lock dir: {error}"))
        })?;
        let name = path
            .file_name()
            .ok_or_else(|| CalyxError::disk_pressure("lock path has no file name"))?;
        return Ok(parent.join(name));
    }
    Ok(path.to_path_buf())
}
