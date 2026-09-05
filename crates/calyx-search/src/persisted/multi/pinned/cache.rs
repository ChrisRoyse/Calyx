#[cfg(feature = "cuda")]
use std::collections::BTreeMap;
#[cfg(feature = "cuda")]
use std::sync::{Arc, Mutex, OnceLock};

use super::*;

#[cfg(feature = "cuda")]
pub(in crate::persisted::multi) struct PinnedIndexAccess {
    pub(in crate::persisted::multi) index: Arc<PinnedMultiIndex>,
    #[cfg(feature = "cuda")]
    pub(in crate::persisted::multi) resident_cache_hit: bool,
    #[cfg(feature = "cuda")]
    pub(in crate::persisted::multi) physical_rows_scanned: usize,
    #[cfg(feature = "cuda")]
    pub(in crate::persisted::multi) physical_tokens_decoded: usize,
    #[cfg(feature = "cuda")]
    pub(in crate::persisted::multi) physical_bytes_read: u64,
}

#[cfg(feature = "cuda")]
struct PinnedGeneration {
    entry_sha256: String,
    index: Arc<PinnedMultiIndex>,
}

#[cfg(feature = "cuda")]
type PinCache = Mutex<BTreeMap<(String, u16), PinnedGeneration>>;

#[cfg(feature = "cuda")]
fn cache() -> &'static PinCache {
    static CACHE: OnceLock<PinCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// CPU-only builds stream each segment and never pin a resident generation
/// (issue #1845), so there is nothing to evict on a generation change. The
/// verify-once bounded-segment memo (keyed by entry sha256) still enforces
/// freshness on that build path.
#[cfg(not(feature = "cuda"))]
pub(in crate::persisted::multi) fn observe_generation(
    _vault_dir: &Path,
    _slot: SlotId,
    _entry_sha256: &str,
) -> CliResult<()> {
    Ok(())
}

#[cfg(feature = "cuda")]
pub(in crate::persisted::multi) fn observe_generation(
    vault_dir: &Path,
    slot: SlotId,
    entry_sha256: &str,
) -> CliResult<()> {
    let cache_key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    let removed = {
        let mut cache = cache().lock().expect("multi pin cache poisoned");
        let Some(generation) = cache
            .get(&cache_key)
            .filter(|generation| generation.entry_sha256 != entry_sha256)
        else {
            return Ok(());
        };
        if Arc::strong_count(&generation.index) != 1 {
            return Err(stale(format!(
                "persistent MaxSim generation changed for slot {slot} while the prior resident generation is in use; retry"
            )));
        }
        cache.remove(&cache_key)
    };
    if removed.is_some() {
        drop(removed);
        pinned::release(&PinKey::new(vault_dir, slot.get(), PIN_KIND)?);
    }
    Ok(())
}

/// Return the verified pin and whether this exact entry sha was already
/// resident. A changed manifest entry can never reuse the previous matrix.
#[cfg(feature = "cuda")]
pub(in crate::persisted::multi) fn pinned_index(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    specs: &[PinnedSegmentSpec],
) -> CliResult<PinnedIndexAccess> {
    let entry_sha256 = entry.require_sha256(slot)?.to_string();
    let cache_key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    {
        let cache = cache().lock().expect("multi pin cache poisoned");
        if let Some(generation) = cache.get(&cache_key)
            && generation.entry_sha256 == entry_sha256
        {
            return Ok(PinnedIndexAccess {
                index: Arc::clone(&generation.index),
                #[cfg(feature = "cuda")]
                resident_cache_hit: true,
                #[cfg(feature = "cuda")]
                physical_rows_scanned: 0,
                #[cfg(feature = "cuda")]
                physical_tokens_decoded: 0,
                #[cfg(feature = "cuda")]
                physical_bytes_read: 0,
            });
        }
    }
    load_and_cache(vault_dir, entry, slot, specs, cache_key, entry_sha256)
}

#[cfg(feature = "cuda")]
fn load_and_cache(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    specs: &[PinnedSegmentSpec],
    cache_key: (String, u16),
    entry_sha256: String,
) -> CliResult<PinnedIndexAccess> {
    let token_dim = entry.require_token_dim(slot)?;
    let pin_key = PinKey::new(vault_dir, slot.get(), PIN_KIND)?;
    let predicted_bytes = predicted_pin_bytes(entry, token_dim)?;
    pinned::reserve(&pin_key, predicted_bytes)?;
    let index = match load_verified(slot, token_dim, entry, specs, &pin_key, predicted_bytes) {
        Ok(index) => Arc::new(index),
        Err(error) => {
            pinned::release(&pin_key);
            cache()
                .lock()
                .expect("multi pin cache poisoned")
                .remove(&cache_key);
            return Err(error);
        }
    };
    if let Err(error) = pinned::reserve(&pin_key, index.approx_bytes()) {
        pinned::release(&pin_key);
        return Err(error);
    }
    cache().lock().expect("multi pin cache poisoned").insert(
        cache_key,
        PinnedGeneration {
            entry_sha256,
            index: Arc::clone(&index),
        },
    );
    #[cfg(feature = "cuda")]
    let physical_bytes_read = specs.iter().try_fold(0_u64, |total, spec| {
        total
            .checked_add(spec.byte_len)
            .ok_or_else(|| stale("persistent MaxSim physical byte telemetry overflow"))
    })?;
    Ok(PinnedIndexAccess {
        index,
        #[cfg(feature = "cuda")]
        resident_cache_hit: false,
        #[cfg(feature = "cuda")]
        physical_rows_scanned: entry.len,
        #[cfg(feature = "cuda")]
        physical_tokens_decoded: entry.token_count.unwrap_or_default(),
        #[cfg(feature = "cuda")]
        physical_bytes_read,
    })
}
