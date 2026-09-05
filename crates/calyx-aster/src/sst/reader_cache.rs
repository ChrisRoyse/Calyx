//! Process-wide bounded shared `SstReader` cache — the LSM "table cache".
//!
//! `SstReader::open` computes a whole-body CRC32 over the mapped file on every
//! open (see [`super::read_header`]). While every SST is small that cost is
//! invisible, but once compaction collapses a column family to one large SST,
//! any read loop that re-opens the same file per item re-checksums the whole
//! file each time and pins a core at 100%.
//!
//! This cache keeps a bounded set of already-opened [`SstReader`]s keyed by the
//! caller-provided absolute file path and hands out cheap `Arc` clones.
//!
//! # Zero-syscall hit path (#1986)
//!
//! A cache hit performs **no filesystem syscall**: it neither `canonicalize`s
//! nor `stat`s the path, matching RocksDB's `TableCache::FindTable` fast path
//! (keyed by manifest-stable identity, zero syscalls on a hit). Correctness
//! rests on two contracts, not on per-hit revalidation:
//!
//! 1. **The LSM immutability invariant.** SSTs get unique names (flush
//!    ordinals, durable commit seqs, compaction output seqs) and are never
//!    rewritten in place. A given path therefore always maps to the same
//!    bytes for the life of that file, so a cached reader stays valid.
//! 2. **The explicit invalidate-before-delete contract.** Every reclaim site
//!    calls [`invalidate_reader`] immediately before `fs::remove_file`, so the
//!    cache never serves a reader for a path that has been (or is about to be)
//!    reused. Identity drift *without* an [`invalidate_reader`] call may serve
//!    the cached reader — that is the immutability invariant's responsibility,
//!    deliberately not re-checked on every read.
//!
//! The cache is keyed by the caller's path, which **must be absolute**. A
//! relative path is a programming error and fails closed (see
//! [`absolute_cache_key`]) rather than silently canonicalizing to a form that
//! would miss both the cache and later invalidation. All internal callers pass
//! vault-derived absolute paths. The key is normalized purely lexically (drop
//! `.` components, no filesystem access) so that [`shared_reader`] and
//! [`invalidate_reader`] always derive the same key for the same file on both
//! Unix and Windows.
//!
//! Windows nuance: an active memory mapping keeps `DeleteFile` failing, so a
//! cached reader can block reclaim of its file. Callers therefore invalidate
//! via [`invalidate_reader`] immediately before `fs::remove_file`; this drops
//! the cache's `Arc`, shrinking the window in which a delete can fail to the
//! set of reads still holding their own clone in flight (the same fail-closed
//! behaviour as before this cache existed — deletion never silently races a
//! live mapping).

use super::SstReader;
use calyx_core::{CalyxError, Result};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Default upper bound on concurrently cached readers, sized to comfortably
/// cover a healthy post-compaction vault's leveled files while still bounding
/// file descriptors and resident index/bloom metadata. Overridable via
/// `CALYX_SST_READER_CACHE_CAP` (see [`max_cached_readers`]). The prior 256 was
/// structurally useless against a fanned-out uncompacted CF (#1986).
const DEFAULT_MAX_CACHED_READERS: usize = 4096;

/// Environment override for the reader cache capacity.
const CACHE_CAP_ENV: &str = "CALYX_SST_READER_CACHE_CAP";

/// Module-local fail-closed error code for reader-cache configuration/usage
/// faults (relative path, invalid capacity override).
const CACHE_CONFIG_CODE: &str = "CALYX_ASTER_SST_READER_CACHE_CONFIG";

/// A cached reader. Identity is the (immutable) file at its key; no length or
/// mtime is retained because the hit path performs no revalidation.
struct Entry {
    reader: Arc<SstReader>,
    last_used: u64,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Monotonic LRU stamp. Hand-rolled so the cache needs no extra dependency.
fn next_tick() -> u64 {
    static CLOCK: AtomicU64 = AtomicU64::new(0);
    CLOCK.fetch_add(1, Ordering::Relaxed)
}

/// Locks the cache, recovering from a poisoned mutex: entries are just `Arc`s,
/// so a panic mid-mutation leaves no torn invariant.
fn lock() -> MutexGuard<'static, HashMap<PathBuf, Entry>> {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Resolved cache capacity, parsed once from the environment and memoized.
///
/// Reads `CALYX_SST_READER_CACHE_CAP` exactly once. An unset variable uses
/// [`DEFAULT_MAX_CACHED_READERS`]; a value of `0`, a non-integer, or non-unicode
/// bytes fail closed with a config error rather than silently degrading the
/// cache. The parsed outcome (value or error message) is memoized so repeated
/// calls never touch the environment again.
fn max_cached_readers() -> Result<usize> {
    static CAP: OnceLock<std::result::Result<usize, String>> = OnceLock::new();
    match CAP.get_or_init(|| {
        interpret_cache_cap(match std::env::var(CACHE_CAP_ENV) {
            Ok(raw) => Some(raw),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{CACHE_CAP_ENV} is set to non-unicode bytes"));
            }
        })
    }) {
        Ok(cap) => Ok(*cap),
        Err(message) => Err(cache_config_error(message.clone())),
    }
}

/// Pure capacity parser: `None` (unset) -> default; otherwise a positive
/// integer, else an error message. Factored out of [`max_cached_readers`] so it
/// can be unit-tested without mutating the process environment.
fn interpret_cache_cap(raw: Option<String>) -> std::result::Result<usize, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_MAX_CACHED_READERS);
    };
    let trimmed = raw.trim();
    match trimmed.parse::<usize>() {
        Ok(0) | Err(_) => Err(format!(
            "{CACHE_CAP_ENV} must be a positive integer, got {trimmed:?}"
        )),
        Ok(cap) => Ok(cap),
    }
}

fn cache_config_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CACHE_CONFIG_CODE,
        message: message.into(),
        remediation: "pass a vault-derived absolute SST path and set CALYX_SST_READER_CACHE_CAP to a positive integer (or leave it unset)",
    }
}

/// Absolute, lexically-normalized cache key. Fails closed on a relative path
/// (a programming error) instead of canonicalizing — canonicalization is a
/// per-call realpath component walk, exactly the syscall storm this cache
/// exists to avoid (#1986). Normalization is purely lexical (drops `.`
/// components, no filesystem access) so [`shared_reader`] and
/// [`invalidate_reader`] always agree on the key for a given file.
fn absolute_cache_key(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(cache_config_error(format!(
            "SST reader cache requires an absolute path; got relative {}",
            path.display()
        )));
    }
    Ok(normalize_lexical(path))
}

/// Purely lexical path normalization: drops `.` components and collapses
/// redundant separators via `Path::components`, never touching the filesystem.
/// Applied identically by every cache and invalidation path so keys agree.
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir) {
            continue;
        }
        out.push(component.as_os_str());
    }
    out
}

/// Returns a shared reader for `path`, opening (and checksumming) it at most
/// once per distinct file. `path` must be absolute (relative fails closed).
///
/// A cache hit performs no filesystem syscall: it bumps the LRU stamp and
/// returns a cheap `Arc` clone. A miss opens the file (which stats and reads
/// it, as before). Correctness rests on the LSM immutability invariant plus
/// the explicit [`invalidate_reader`]-before-delete contract; see the module
/// docs.
pub fn shared_reader(path: &Path) -> Result<Arc<SstReader>> {
    let cap = max_cached_readers()?;
    let key = absolute_cache_key(path)?;

    {
        let mut cache = lock();
        if let Some(entry) = cache.get_mut(&key) {
            entry.last_used = next_tick();
            return Ok(entry.reader.clone());
        }
    }

    // Open outside the lock so the whole-file checksum never serializes other
    // readers. A benign race may open the same immutable file twice; both
    // readers are valid and the last insert simply wins.
    let reader = Arc::new(SstReader::open(&key)?);

    let mut cache = lock();
    cache.insert(
        key,
        Entry {
            reader: reader.clone(),
            last_used: next_tick(),
        },
    );
    evict_over_cap(&mut cache, cap);
    Ok(reader)
}

/// Drops the cached reader for `path` (if any) so a subsequent `fs::remove_file`
/// is not blocked by this cache's live mapping. Best-effort and infallible.
///
/// Derives the key exactly as [`shared_reader`] does, so invalidation always
/// hits the entry the cache stored. A relative path could never have been
/// cached (`shared_reader` fails closed on it), so there is nothing to drop.
pub fn invalidate_reader(path: &Path) {
    if !path.is_absolute() {
        return;
    }
    let key = normalize_lexical(path);
    lock().remove(&key);
}

/// Evicts least-recently-used entries until the cache is within `cap`.
fn evict_over_cap(cache: &mut HashMap<PathBuf, Entry>, cap: usize) {
    while cache.len() > cap {
        let victim = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone());
        match victim {
            Some(key) => {
                cache.remove(&key);
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sst::write_sst;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-sst-reader-cache-{name}-{}-{id}",
            std::process::id()
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn shared_reader_returns_same_instance_until_invalidated() {
        let dir = test_dir("hit");
        let path = dir.join("a.sst");
        write_sst(&path, [(b"k".as_slice(), b"v".as_slice())]).unwrap();

        let first = shared_reader(&path).unwrap();
        let second = shared_reader(&path).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "cache hit must return the same shared reader with no reopen"
        );
        assert_eq!(first.get(b"k").unwrap().as_deref(), Some(b"v".as_slice()));

        invalidate_reader(&path);
        let third = shared_reader(&path).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &third),
            "invalidate-then-read must force a fresh open"
        );

        drop((first, second, third));
        invalidate_reader(&path);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shared_reader_rejects_relative_paths_fail_closed() {
        let relative = Path::new("relative-vault/anchors/000.sst");
        let err = shared_reader(relative).unwrap_err();
        assert_eq!(err.code, CACHE_CONFIG_CODE);
        assert!(
            err.message.contains("absolute"),
            "message names the absolute-path requirement: {}",
            err.message
        );
        // Invalidation of a relative path is an infallible no-op.
        invalidate_reader(relative);
    }

    #[test]
    fn cache_capacity_parsing_is_fail_closed() {
        assert_eq!(interpret_cache_cap(None), Ok(DEFAULT_MAX_CACHED_READERS));
        assert_eq!(interpret_cache_cap(Some("8192".to_string())), Ok(8192));
        assert_eq!(interpret_cache_cap(Some("  512  ".to_string())), Ok(512));
        assert!(interpret_cache_cap(Some("0".to_string())).is_err());
        assert!(interpret_cache_cap(Some("nonsense".to_string())).is_err());
        assert!(interpret_cache_cap(Some("-4".to_string())).is_err());
    }

    /// The new contract: identity drift *without* [`invalidate_reader`] may keep
    /// serving the cached reader (the immutability invariant's responsibility),
    /// while drift *with* an explicit invalidate serves fresh bytes.
    ///
    /// Unix-only: Windows cannot replace a file while the cache's live mapping
    /// exists, which is exactly why reclaim sites call [`invalidate_reader`]
    /// before deletion there.
    #[cfg(unix)]
    #[test]
    fn shared_reader_serves_cached_reader_until_explicitly_invalidated() {
        let dir = test_dir("drift");
        let path = dir.join("b.sst");
        write_sst(&path, [(b"k".as_slice(), b"old".as_slice())]).unwrap();
        let first = shared_reader(&path).unwrap();
        assert_eq!(first.get(b"k").unwrap().as_deref(), Some(b"old".as_slice()));

        // Rewrite the path with different bytes, violating the immutability
        // invariant on purpose. Without invalidate_reader the cache is entitled
        // to keep serving the prior reader — no per-hit syscall re-checks it.
        fs::remove_file(&path).ok();
        write_sst(&path, [(b"k".as_slice(), b"new-longer-value".as_slice())]).unwrap();

        let cached = shared_reader(&path).unwrap();
        assert!(
            Arc::ptr_eq(&first, &cached),
            "drift without invalidate keeps serving the cached reader"
        );
        assert_eq!(
            cached.get(b"k").unwrap().as_deref(),
            Some(b"old".as_slice()),
            "cached reader still serves the original bytes"
        );

        // The explicit invalidate-before-reopen contract is what forces fresh
        // bytes to be served.
        invalidate_reader(&path);
        let fresh = shared_reader(&path).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &fresh),
            "invalidate forces a fresh open"
        );
        assert_eq!(
            fresh.get(b"k").unwrap().as_deref(),
            Some(b"new-longer-value".as_slice()),
            "post-invalidate reader serves the new bytes"
        );

        drop((first, cached, fresh));
        invalidate_reader(&path);
        fs::remove_dir_all(dir).unwrap();
    }
}
