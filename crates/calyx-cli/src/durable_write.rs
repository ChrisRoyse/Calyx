use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::Value;

use crate::error::{CliError, CliResult};

mod immutable;

pub(crate) use immutable::write_bytes_atomic_new;

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);
static PROCESS_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();

pub(crate) struct DurableWriteLockGuard {
    _process_guard: MutexGuard<'static, ()>,
    _file: File,
}

impl DurableWriteLockGuard {
    pub(crate) fn acquire(path: &Path, label: &str) -> CliResult<Self> {
        let key = lock_key(path, label)?;
        let process_guard = process_mutex(&key)
            .lock()
            .map_err(|_| CliError::io(format!("{label} process lock mutex was poisoned")))?;
        let existed = key.exists();
        let mut options = OpenOptions::new();
        options.create(true).read(true).append(true);
        // The durable-write lock file is a vault descendant; create it
        // owner-only (0600 + O_NOFOLLOW) so it cannot inherit the operator
        // umask like the evidence it guards (issue #1931).
        crate::private_fs::harden_open_options(&mut options);
        let file = options.open(&key).map_err(|error| {
            CliError::io(format!(
                "open {label} lock file {} failed: {error}",
                key.display()
            ))
        })?;
        if !existed {
            crate::private_fs::enforce_private_file_mode(&file, &key)?;
        }
        file.lock().map_err(|error| {
            CliError::io(format!(
                "acquire {label} lock file {} failed: {error}",
                key.display()
            ))
        })?;
        Ok(Self {
            _process_guard: process_guard,
            _file: file,
        })
    }

    /// Acquire the same lock used by [`write_bytes_atomic`] for `path`.
    ///
    /// This is intentionally separate from `acquire`: callers performing an
    /// atomic read/inspect/archive/replace transaction must hold the target's
    /// write lock across the entire transaction, not merely across the final
    /// rename.
    pub(crate) fn acquire_for_target(path: &Path, label: &str) -> CliResult<Self> {
        Self::acquire(&write_lock_path(path)?, label)
    }
}

pub(crate) fn write_json_value_atomic(path: &Path, value: &Value, label: &str) -> CliResult {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {label}: {error}")))?;
    bytes.push(10);
    write_bytes_atomic(path, &bytes, label)
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    ensure_parent(path, label)?;
    let _write_guard = DurableWriteLockGuard::acquire_for_target(path, label)?;
    write_bytes_atomic_locked(path, bytes, label)
}

/// Publish bytes atomically while the caller holds
/// [`DurableWriteLockGuard::acquire_for_target`] for this exact path.
///
/// Keeping this primitive explicit prevents a reconciler from deadlocking by
/// reacquiring its own non-reentrant process mutex while still allowing the
/// read and replacement to be one serialized transaction.
pub(crate) fn write_bytes_atomic_locked(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    let parent = ensure_parent(path, label)?;
    let tmp = temp_path(path)?;
    // Runtime evidence written through this durable path (ingest status.json,
    // etc.) must be owner-only; create the staging file 0600 + O_NOFOLLOW and
    // prove the mode before publishing (issue #1931).
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::private_fs::harden_open_options(&mut options);
    let mut file = options.open(&tmp).map_err(|error| {
        CliError::io(format!(
            "create-new temporary {label} {} failed: {error}",
            tmp.display()
        ))
    })?;
    crate::private_fs::enforce_private_file_mode(&file, &tmp)?;
    let publish = (|| {
        file.write_all(bytes).map_err(|error| {
            CliError::io(format!(
                "write temporary {label} {} failed: {error}",
                tmp.display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            CliError::io(format!(
                "sync temporary {label} {} failed: {error}",
                tmp.display()
            ))
        })?;
        drop(file);
        publish_replace(&tmp, path, label)?;
        sync_parent_dir(parent, label)
    })();
    match publish {
        Ok(()) => Ok(()),
        Err(error) => cleanup_staging_after_error(&tmp, label, error),
    }
}

fn ensure_parent<'a>(path: &'a Path, label: &str) -> CliResult<&'a Path> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::io(format!("{label} path {} has no parent", path.display())))?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::io(format!(
            "create {label} parent directory {} failed: {error}",
            parent.display()
        ))
    })?;
    Ok(parent)
}

fn temp_path(path: &Path) -> CliResult<PathBuf> {
    let filename = path.file_name().ok_or_else(|| {
        CliError::io(format!(
            "atomic write path {} has no filename",
            path.display()
        ))
    })?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(filename);
    let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    tmp_name.push(format!(".{}.{nonce}.tmp", std::process::id()));
    Ok(path.with_file_name(tmp_name))
}

fn write_lock_path(path: &Path) -> CliResult<PathBuf> {
    let filename = path.file_name().ok_or_else(|| {
        CliError::io(format!(
            "atomic write path {} has no filename",
            path.display()
        ))
    })?;
    let mut lock_name = OsString::from(".");
    lock_name.push(filename);
    lock_name.push(".write.lock");
    Ok(path.with_file_name(lock_name))
}

fn lock_key(path: &Path, label: &str) -> CliResult<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        CliError::io(format!(
            "{label} lock path {} has no parent",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::io(format!(
            "create {label} lock directory {} failed: {error}",
            parent.display()
        ))
    })?;
    let parent = parent.canonicalize().map_err(|error| {
        CliError::io(format!(
            "canonicalize {label} lock directory {} failed: {error}",
            parent.display()
        ))
    })?;
    let filename = path.file_name().ok_or_else(|| {
        CliError::io(format!(
            "{label} lock path {} has no filename",
            path.display()
        ))
    })?;
    Ok(parent.join(filename))
}

fn process_mutex(path: &Path) -> &'static Mutex<()> {
    let locks = PROCESS_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks.lock().expect("durable write lock registry poisoned");
    if let Some(lock) = locks.get(path) {
        return lock;
    }
    let lock = Box::leak(Box::new(Mutex::new(())));
    locks.insert(path.to_path_buf(), lock);
    lock
}

#[cfg(unix)]
fn publish_replace(tmp: &Path, path: &Path, label: &str) -> CliResult {
    fs::rename(tmp, path).map_err(|error| {
        CliError::io(format!(
            "publish {label} {} -> {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

#[cfg(windows)]
fn publish_replace(tmp: &Path, path: &Path, label: &str) -> CliResult {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    // Rust's standard filesystem APIs accept long Windows paths, but this raw
    // Win32 call does not add the extended-length namespace for us. Resolve
    // the existing temp and target parent through `canonicalize`, which yields
    // absolute `\\?\` paths on Windows, before calling MoveFileExW. Without
    // this, a valid temp path just over MAX_PATH fails with ERROR_PATH_NOT_FOUND.
    let canonical_tmp = tmp.canonicalize().map_err(|error| {
        CliError::io(format!(
            "canonicalize temporary {label} {} for Windows publish failed: {error}",
            tmp.display()
        ))
    })?;
    let parent = path.parent().ok_or_else(|| {
        CliError::io(format!(
            "publish {label} target {} has no parent",
            path.display()
        ))
    })?;
    let filename = path.file_name().ok_or_else(|| {
        CliError::io(format!(
            "publish {label} target {} has no filename",
            path.display()
        ))
    })?;
    let canonical_path = parent
        .canonicalize()
        .map_err(|error| {
            CliError::io(format!(
                "canonicalize {label} target parent {} for Windows publish failed: {error}",
                parent.display()
            ))
        })?
        .join(filename);
    let from = canonical_tmp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = canonical_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are NUL-terminated and remain alive for the call.
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved != 0 {
        return Ok(());
    }
    Err(CliError::io(format!(
        "publish {label} {} -> {} failed: {}",
        tmp.display(),
        path.display(),
        std::io::Error::last_os_error()
    )))
}

fn cleanup_staging_after_error(tmp: &Path, label: &str, error: CliError) -> CliResult {
    match fs::remove_file(tmp) {
        Ok(()) => Err(error),
        Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => Err(error),
        Err(cleanup) => Err(CliError::io(format!(
            "{}; cleanup temporary {label} {} also failed: {cleanup}",
            error,
            tmp.display()
        ))),
    }
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path, label: &str) -> CliResult {
    let dir = File::open(parent).map_err(|error| {
        CliError::io(format!(
            "open {label} parent directory {} for sync failed: {error}",
            parent.display()
        ))
    })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync {label} parent directory {} failed: {error}",
            parent.display()
        ))
    })
}

#[cfg(windows)]
fn sync_parent_dir(parent: &Path, label: &str) -> CliResult {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    let dir = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent)
        .map_err(|error| {
            CliError::io(format!(
                "open {label} parent directory {} for Windows sync failed: {error}",
                parent.display()
            ))
        })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync {label} parent directory {} on Windows failed: {error}",
            parent.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn atomic_json_write_publishes_and_removes_temp_file() {
        let root = std::env::temp_dir().join(format!(
            "calyx-durable-write-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp durable-write root");
        let path = root.join("nested").join("matrix.json");
        let value = json!({
            "schema": "calyx-durable-write-test-v1",
            "source_of_truth": "physical file readback after atomic publish"
        });

        write_json_value_atomic(&path, &value, "durable write test").expect("atomic write");

        let bytes = fs::read(&path).expect("read published json");
        let decoded: Value = serde_json::from_slice(&bytes).expect("decode published json");
        assert_eq!(decoded, value);
        assert!(staging_files(&path).is_empty());

        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_overwrites_existing_file_and_reads_back_new_bytes() {
        let root = temp_root("overwrite");
        let path = root.join("progress.json");
        fs::create_dir_all(&root).expect("create temp durable-write root");
        write_bytes_atomic(&path, b"before\n", "overwrite test").expect("initial write");

        write_bytes_atomic(&path, b"after\n", "overwrite test").expect("overwrite");

        assert_eq!(fs::read(&path).expect("read overwritten file"), b"after\n");
        assert!(staging_files(&path).is_empty());
        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn concurrent_writers_publish_one_complete_value_without_staging_leaks() {
        let root = temp_root("concurrent");
        fs::create_dir_all(&root).expect("create temp durable-write root");
        let path = root.join("index.json");
        let barrier = Arc::new(Barrier::new(3));
        let writers = [b"writer-a\n".as_slice(), b"writer-b\n".as_slice()]
            .into_iter()
            .map(|bytes| {
                let path = path.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    write_bytes_atomic(&path, bytes, "concurrent test")
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        for writer in writers {
            writer
                .join()
                .expect("writer thread")
                .expect("atomic writer");
        }

        let actual = fs::read(&path).expect("read concurrent target");
        assert!(actual == b"writer-a\n" || actual == b"writer-b\n");
        assert!(staging_files(&path).is_empty());
        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_fails_when_parent_path_is_file() {
        let root = temp_root("parent-file");
        fs::create_dir_all(&root).expect("create temp durable-write root");
        let blocked_parent = root.join("blocked");
        fs::write(&blocked_parent, b"not a directory").expect("write blocking file");
        let path = blocked_parent.join("progress.json");

        let error = write_bytes_atomic(&path, b"unpublished\n", "parent file test")
            .expect_err("parent file must fail closed");

        assert!(
            error
                .to_string()
                .contains("create parent file test parent directory"),
            "error should name the failing parent creation: {error}"
        );
        assert!(
            !path.exists(),
            "child file must not be published when parent is a file"
        );
        assert_eq!(
            fs::read(&blocked_parent).expect("read blocking file"),
            b"not a directory"
        );
        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_rejects_empty_path_without_publishing() {
        let error = write_bytes_atomic(Path::new(""), b"unpublished\n", "empty path test")
            .expect_err("empty path must fail closed");

        assert!(
            error
                .to_string()
                .contains("empty path test path  has no parent"),
            "error should name the missing parent: {error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_atomic_replace_publishes_beyond_max_path() {
        let root = temp_root("windows-long-path");
        let mut parent = root.clone();
        for index in 0..5 {
            parent.push(format!("segment-{index}-{}", "x".repeat(44)));
        }
        let path = parent.join("physical-source-of-truth.json");
        assert!(
            path.as_os_str().len() > 260,
            "test target must exceed legacy MAX_PATH: {}",
            path.display()
        );

        write_bytes_atomic(&path, b"long-path-published\n", "Windows long-path test")
            .expect("publish beyond MAX_PATH");

        assert_eq!(
            fs::read(&path).expect("read long-path target"),
            b"long-path-published\n"
        );
        assert!(staging_files(&path).is_empty());
        fs::remove_dir_all(&root).expect("cleanup long-path test root");
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "calyx-durable-write-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    fn staging_files(path: &Path) -> Vec<PathBuf> {
        let parent = path.parent().expect("target parent");
        let prefix = format!(".{}.", path.file_name().unwrap().to_string_lossy());
        fs::read_dir(parent)
            .expect("read target parent")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|candidate| {
                candidate.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with(&prefix) && name.ends_with(".tmp")
                })
            })
            .collect()
    }
}
