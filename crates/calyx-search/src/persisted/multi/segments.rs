use super::*;

use crate::persisted::RebuildProgress;

#[path = "segments/path.rs"]
mod path;
use path::{checked_rel, checked_segment_path};
#[path = "segments/manifest.rs"]
mod manifest;
use manifest::validate_segments_manifest_shape;
#[path = "segments/bounds.rs"]
mod bounds;
#[path = "segments/reuse.rs"]
mod reuse;
#[path = "segments/search.rs"]
mod search;
pub(super) use bounds::ensure_entry_bounded;
use reuse::reusable_segments;
pub(super) use search::search_segments;

const MULTI_SEGMENTS_FORMAT: &str = "calyx-search-multi-maxsim-segments-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MultiSegmentsManifest {
    format: String,
    slot: u16,
    token_dim: u32,
    base_seq: u64,
    row_count: usize,
    token_count: usize,
    segments: Vec<MultiSegmentRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MultiSegmentRef {
    pub(super) index_rel: String,
    pub(super) sha256: String,
    pub(super) base_seq: u64,
    pub(super) row_count: usize,
    pub(super) token_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) ids: Vec<CxId>,
}

#[derive(Debug)]
struct ReusedMultiSegments {
    refs: Vec<MultiSegmentRef>,
    ids: BTreeSet<CxId>,
    token_count: usize,
}

struct SegmentManifestBuild {
    token_dim: u32,
    row_count: usize,
    token_count: usize,
    base_seq: u64,
    segments: Vec<MultiSegmentRef>,
}

pub(in crate::persisted) fn write(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: MultiSlotRows,
    base_seq: u64,
    previous: Option<&SearchIndexEntry>,
    on_event: &mut dyn FnMut(RebuildProgress<'_>) -> CliResult,
) -> CliResult<SearchIndexEntry> {
    let row_count = rows.rows.len();
    let token_count = rows.rows.iter().map(|row| row.1.len()).sum::<usize>();
    let current_ids = rows
        .rows
        .iter()
        .map(|(cx_id, _)| *cx_id)
        .collect::<BTreeSet<_>>();
    if let Some(reused) = reusable_segments(
        vault_dir,
        slot,
        rows.token_dim,
        &current_ids,
        previous,
        on_event,
    )? {
        let mut refs = reused.refs;
        let mut segment_token_count = reused.token_count;
        let missing = rows
            .rows
            .iter()
            .filter(|(cx_id, _)| !reused.ids.contains(cx_id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let segments = write_binary_segments(
                vault_dir,
                root,
                slot,
                rows.token_dim,
                &missing,
                base_seq,
                refs.len(),
            )?;
            segment_token_count += segments
                .iter()
                .map(|segment| segment.token_count)
                .sum::<usize>();
            refs.extend(segments);
        }
        if refs.iter().map(|segment| segment.row_count).sum::<usize>() == row_count
            && segment_token_count == token_count
        {
            return write_segments_manifest(
                vault_dir,
                root,
                slot,
                SegmentManifestBuild {
                    token_dim: rows.token_dim,
                    row_count,
                    token_count,
                    base_seq,
                    segments: refs,
                },
            );
        }
    }
    let segments = write_binary_segments(
        vault_dir,
        root,
        slot,
        rows.token_dim,
        &rows.rows,
        base_seq,
        0,
    )?;
    write_segments_manifest(
        vault_dir,
        root,
        slot,
        SegmentManifestBuild {
            token_dim: rows.token_dim,
            row_count,
            token_count,
            base_seq,
            segments,
        },
    )
}

pub(super) fn referenced_segment_artifacts(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult<Vec<PathBuf>> {
    let manifest = read_segments_manifest(vault_dir, entry, entry.built_at_seq, slot)?;
    manifest
        .segments
        .iter()
        .map(|segment| checked_segment_path(vault_dir, &segment.index_rel, slot))
        .collect()
}

fn write_binary_segments(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    token_dim: u32,
    rows: &[(CxId, Vec<Vec<f32>>)],
    base_seq: u64,
    start_ordinal: usize,
) -> CliResult<Vec<MultiSegmentRef>> {
    bounds::split_row_ranges_by_segment_budget(slot, token_dim, rows)?
        .into_iter()
        .enumerate()
        .map(|(offset, range)| {
            write_binary_segment(
                vault_dir,
                root,
                slot,
                token_dim,
                &rows[range],
                base_seq,
                start_ordinal + offset,
            )
        })
        .collect()
}

fn write_binary_segment(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    token_dim: u32,
    rows: &[(CxId, Vec<Vec<f32>>)],
    base_seq: u64,
    ordinal: usize,
) -> CliResult<MultiSegmentRef> {
    let path = root.join(format!(
        "slot_{:05}_seq_{base_seq:020}_seg_{ordinal:05}_n_{:010}.multi.bin",
        slot.get(),
        rows.len()
    ));
    let token_count = rows.iter().map(|row| row.1.len()).sum::<usize>();
    let sha256 = binary::write_binary_atomic_hashed(&path, slot, token_dim, rows, base_seq)?;
    Ok(MultiSegmentRef {
        index_rel: rel(vault_dir, &path)?,
        sha256,
        base_seq,
        row_count: rows.len(),
        token_count,
        ids: rows.iter().map(|(cx_id, _)| *cx_id).collect(),
    })
}

fn write_segments_manifest(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    build: SegmentManifestBuild,
) -> CliResult<SearchIndexEntry> {
    let manifest = MultiSegmentsManifest {
        format: MULTI_SEGMENTS_FORMAT.to_string(),
        slot: slot.get(),
        token_dim: build.token_dim,
        base_seq: build.base_seq,
        row_count: build.row_count,
        token_count: build.token_count,
        segments: build.segments,
    };
    validate_segments_manifest_shape(
        &manifest,
        slot,
        build.token_dim,
        build.base_seq,
        build.row_count,
        build.token_count,
    )?;
    let path = root.join(format!(
        "slot_{:05}_seq_{:020}_n_{:010}.multi.segments.json",
        slot.get(),
        build.base_seq,
        build.row_count
    ));
    let sha256 = write_json_atomic_hashed(&path, &manifest)?;
    Ok(SearchIndexEntry::multi_segments(
        slot,
        build.token_dim,
        build.row_count,
        build.token_count,
        build.base_seq,
        rel(vault_dir, &path)?,
        sha256,
    ))
}

pub(super) fn read_segments_manifest(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult<MultiSegmentsManifest> {
    entry.require_kind("multi_maxsim_segments", slot)?;
    let path = checked_segment_path(vault_dir, entry.require_index_rel(slot)?, slot)?;
    let bytes = fs::read(&path)?;
    let actual = sha256_hex(&bytes);
    let expected = entry.require_sha256(slot)?;
    if actual != expected {
        return Err(stale(format!(
            "persistent segmented multi manifest sha256 {actual} != manifest {expected}; rebuild the vault search indexes"
        )));
    }
    let manifest: MultiSegmentsManifest = serde_json::from_slice(&bytes).map_err(|err| {
        stale(format!(
            "persistent segmented multi manifest {} is not valid JSON: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    validate_segments_manifest_shape(
        &manifest,
        slot,
        entry.require_token_dim(slot)?,
        manifest_base_seq,
        entry.len,
        entry.token_count.unwrap_or_default(),
    )?;
    Ok(manifest)
}

pub(super) fn validate_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
) -> CliResult<Vec<super::pinned::BoundedSegmentFile>> {
    let mut files = Vec::with_capacity(manifest.segments.len());
    for segment in &manifest.segments {
        bounds::ensure_segment_ref_bounded(slot, token_dim, segment)?;
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
        let expected =
            bounds::segment_estimated_bytes(token_dim, segment.row_count, segment.token_count)?;
        let actual = fs::metadata(&path)?.len();
        if actual != expected {
            return Err(stale(format!(
                "persistent segmented multi sidecar {} has {actual} bytes, expected {expected}; rebuild the vault search indexes",
                segment.index_rel
            )));
        }
        files.push(super::pinned::BoundedSegmentFile {
            path,
            index_rel: segment.index_rel.clone(),
            expected_bytes: expected,
        });
    }
    Ok(files)
}

fn summarize_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
    verify_binary: bool,
) -> CliResult<ReusedMultiSegments> {
    let mut ids = BTreeSet::new();
    let mut token_count = 0usize;
    let mut refs = Vec::with_capacity(manifest.segments.len());
    for segment in &manifest.segments {
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
        let mut segment_ref = segment.clone();
        if !verify_binary && !segment.ids.is_empty() {
            if segment.ids.len() != segment.row_count {
                return Err(stale(format!(
                    "persistent segmented multi manifest {} id count {} != row_count {}; rebuild the vault search indexes",
                    segment.index_rel,
                    segment.ids.len(),
                    segment.row_count
                )));
            }
            for cx_id in &segment.ids {
                if !ids.insert(*cx_id) {
                    return Err(stale(format!(
                        "persistent segmented multi sidecars repeat {cx_id}; rebuild the vault search indexes"
                    )));
                }
            }
            token_count = token_count
                .checked_add(segment.token_count)
                .ok_or_else(|| stale("persistent segmented multi sidecar token_count overflow"))?;
            refs.push(segment_ref);
            continue;
        }
        let summary = binary::summarize_binary_path(
            &path,
            &segment.sha256,
            slot,
            token_dim,
            Some(segment.row_count as u64),
            Some(segment.token_count as u64),
        )?;
        if summary.base_seq != segment.base_seq {
            return Err(stale(format!(
                "persistent segmented multi sidecar {} seq {} != segment manifest seq {}; rebuild the vault search indexes",
                segment.index_rel, summary.base_seq, segment.base_seq
            )));
        }
        segment_ref.ids = summary.ids.iter().copied().collect();
        for cx_id in summary.ids {
            if !ids.insert(cx_id) {
                return Err(stale(format!(
                    "persistent segmented multi sidecars repeat {cx_id}; rebuild the vault search indexes"
                )));
            }
        }
        token_count = token_count
            .checked_add(segment.token_count)
            .ok_or_else(|| stale("persistent segmented multi sidecar token_count overflow"))?;
        refs.push(segment_ref);
    }
    if ids.len() != manifest.row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != unique row count {}; rebuild the vault search indexes",
            manifest.row_count,
            ids.len()
        )));
    }
    if token_count != manifest.token_count {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != sidecar token count {token_count}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    Ok(ReusedMultiSegments {
        refs,
        ids,
        token_count,
    })
}
