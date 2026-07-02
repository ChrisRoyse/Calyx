//! Bounded-check memo for segmented multi sidecars: the expensive segments
//! manifest re-read/re-hash is done once per manifest generation, while
//! on-disk presence and exact byte size are still verified per call.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use calyx_core::SlotId;

use crate::error::CliResult;
use crate::persisted::pinned;
use crate::persisted::stale;
use std::fs;

/// Memo of segment files that already passed the full bounded validation for
/// a manifest generation. A memo hit never skips on-disk presence/size
/// verification — callers must still run `stat_check_segment_files` — it only
/// skips re-reading and re-hashing the segments manifest JSON.
pub(in crate::persisted::multi) struct BoundedSegmentFile {
    pub(in crate::persisted::multi) path: PathBuf,
    pub(in crate::persisted::multi) index_rel: String,
    pub(in crate::persisted::multi) expected_bytes: u64,
}

struct BoundedGeneration {
    entry_sha256: String,
    files: Arc<Vec<BoundedSegmentFile>>,
}

type BoundedCache = Mutex<BTreeMap<(String, u16), BoundedGeneration>>;

fn bounded_cache() -> &'static BoundedCache {
    static CACHE: OnceLock<BoundedCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(in crate::persisted::multi) fn memoized_bounded_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    entry_sha256: &str,
) -> CliResult<Option<Arc<Vec<BoundedSegmentFile>>>> {
    let key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    let cache = bounded_cache()
        .lock()
        .expect("bounded segment memo poisoned");
    Ok(cache.get(&key).and_then(|generation| {
        (generation.entry_sha256 == entry_sha256).then(|| Arc::clone(&generation.files))
    }))
}

pub(in crate::persisted::multi) fn memoize_bounded_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    entry_sha256: &str,
    files: Vec<BoundedSegmentFile>,
) -> CliResult {
    let key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    let mut cache = bounded_cache()
        .lock()
        .expect("bounded segment memo poisoned");
    cache.insert(
        key,
        BoundedGeneration {
            entry_sha256: entry_sha256.to_string(),
            files: Arc::new(files),
        },
    );
    Ok(())
}

/// Per-call on-disk verification for memoized bounded checks: every segment
/// file must still exist with exactly its validated byte size. Deletion or
/// truncation fails closed with the same errors as the full validation.
pub(in crate::persisted::multi) fn stat_check_segment_files(
    slot: SlotId,
    files: &[BoundedSegmentFile],
) -> CliResult {
    for file in files {
        if !file.path.is_file() {
            return Err(stale(format!(
                "persistent segmented multi sidecar missing for slot {slot} at {}; rebuild the vault search indexes",
                file.path.display()
            )));
        }
        let actual = fs::metadata(&file.path)?.len();
        if actual != file.expected_bytes {
            return Err(stale(format!(
                "persistent segmented multi sidecar {} has {actual} bytes, expected {}; rebuild the vault search indexes",
                file.index_rel, file.expected_bytes
            )));
        }
    }
    Ok(())
}
