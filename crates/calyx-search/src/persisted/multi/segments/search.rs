use super::*;
use crate::persisted::multi::pinned::{self, PinnedSegmentSpec};

pub(in crate::persisted::multi) fn search_segments(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    let manifest = read_segments_manifest(vault_dir, entry, manifest_base_seq, slot)?;
    let token_dim = entry.require_token_dim(slot)?;
    let mut specs = Vec::with_capacity(manifest.segments.len());
    for segment in &manifest.segments {
        bounds::ensure_segment_ref_bounded(slot, token_dim, segment)?;
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
        specs.push(PinnedSegmentSpec {
            path,
            index_rel: segment.index_rel.clone(),
            sha256: segment.sha256.clone(),
            base_seq: segment.base_seq,
            row_count: segment.row_count as u64,
            token_count: segment.token_count as u64,
        });
    }
    let index = pinned::pinned_index(vault_dir, entry, slot, specs)?;
    if index.row_count() != manifest.row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != scanned row count {}; rebuild the vault search indexes",
            manifest.row_count,
            index.row_count()
        )));
    }
    Ok(ranked(top_k(index.score(query_tokens, candidates), k)))
}
