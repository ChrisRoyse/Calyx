use rayon::prelude::*;

use super::*;

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
    let results = manifest
        .segments
        .par_iter()
        .map(|segment| {
            bounds::ensure_segment_ref_bounded(slot, token_dim, segment)?;
            let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
            binary::score_binary_segment_collect(
                binary::BinarySegmentSearchSpec {
                    path: &path,
                    sha256: &segment.sha256,
                    row_count: segment.row_count as u64,
                    token_count: segment.token_count as u64,
                },
                slot,
                token_dim,
                query_tokens,
                candidates,
            )
        })
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut scored = Vec::new();
    for result in results {
        let result = result?;
        for cx_id in result.seen {
            if !seen.insert(cx_id) {
                return Err(stale(format!(
                    "persistent segmented multi sidecars repeat {cx_id}; rebuild the vault search indexes"
                )));
            }
        }
        scored.extend(result.scored);
    }
    if seen.len() != manifest.row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != scanned row count {}; rebuild the vault search indexes",
            manifest.row_count,
            seen.len()
        )));
    }
    Ok(ranked(top_k(scored, k)))
}
