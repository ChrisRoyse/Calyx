use super::*;

#[path = "segments/path.rs"]
mod path;
use path::{checked_rel, checked_segment_path};

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

pub(super) fn write(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: MultiSlotRows,
    base_seq: u64,
    previous: Option<&SearchIndexEntry>,
) -> CliResult<SearchIndexEntry> {
    let row_count = rows.rows.len();
    let token_count = rows.rows.iter().map(|row| row.1.len()).sum::<usize>();
    let current_ids = rows
        .rows
        .iter()
        .map(|(cx_id, _)| *cx_id)
        .collect::<BTreeSet<_>>();
    if let Some(reused) =
        reusable_segments(vault_dir, slot, rows.token_dim, &current_ids, previous)?
    {
        let mut refs = reused.refs;
        let mut segment_token_count = reused.token_count;
        let missing = rows
            .rows
            .iter()
            .filter(|(cx_id, _)| !reused.ids.contains(cx_id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let segment = write_binary_segment(
                vault_dir,
                root,
                slot,
                rows.token_dim,
                &missing,
                base_seq,
                refs.len(),
            )?;
            segment_token_count += segment.token_count;
            refs.push(segment);
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
    let segment = write_binary_segment(
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
            segments: vec![segment],
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

fn reusable_segments(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    current_ids: &BTreeSet<CxId>,
    previous: Option<&SearchIndexEntry>,
) -> CliResult<Option<ReusedMultiSegments>> {
    let Some(previous) = previous else {
        return Ok(None);
    };
    if previous.slot != slot.get() {
        return Err(stale(format!(
            "previous persistent multi slot {} cannot be reused for slot {slot}",
            previous.slot
        )));
    }
    match previous.kind.as_str() {
        "multi_maxsim" => {
            let summary = binary::summarize_binary_entry(vault_dir, previous, slot)?;
            if summary.ids.iter().any(|cx_id| !current_ids.contains(cx_id)) {
                return Ok(None);
            }
            let row_count = usize::try_from(summary.row_count).map_err(|_| {
                stale(format!(
                    "persistent binary multi sidecar row_count {} does not fit usize",
                    summary.row_count
                ))
            })?;
            let token_count = usize::try_from(summary.token_count).map_err(|_| {
                stale(format!(
                    "persistent binary multi sidecar token_count {} does not fit usize",
                    summary.token_count
                ))
            })?;
            Ok(Some(ReusedMultiSegments {
                refs: vec![MultiSegmentRef {
                    index_rel: previous.require_index_rel(slot)?.to_string(),
                    sha256: summary.sha256,
                    base_seq: summary.base_seq,
                    row_count,
                    token_count,
                }],
                ids: summary.ids,
                token_count,
            }))
        }
        "multi_maxsim_segments" => {
            let manifest =
                read_segments_manifest(vault_dir, previous, previous.built_at_seq, slot)?;
            let reused = summarize_segment_files(vault_dir, slot, token_dim, &manifest)?;
            if reused.ids.iter().any(|cx_id| !current_ids.contains(cx_id)) {
                return Ok(None);
            }
            Ok(Some(reused))
        }
        _ => Ok(None),
    }
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
) -> CliResult {
    let _ = summarize_segment_files(vault_dir, slot, token_dim, manifest)?;
    Ok(())
}

pub(super) fn search_segments(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    let manifest = read_segments_manifest(vault_dir, entry, manifest_base_seq, slot)?;
    let mut seen = BTreeSet::new();
    let mut scored = Vec::new();
    let token_dim = entry.require_token_dim(slot)?;
    for segment in &manifest.segments {
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
        binary::score_binary_segment(
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
            &mut seen,
            &mut scored,
        )?;
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

fn validate_segments_manifest_shape(
    manifest: &MultiSegmentsManifest,
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    row_count: usize,
    token_count: usize,
) -> CliResult {
    if manifest.format != MULTI_SEGMENTS_FORMAT {
        return Err(stale(format!(
            "persistent segmented multi manifest has format {}; expected {MULTI_SEGMENTS_FORMAT}",
            manifest.format
        )));
    }
    if manifest.slot != slot.get() {
        return Err(stale(format!(
            "persistent segmented multi manifest slot {} != query slot {}",
            manifest.slot,
            slot.get()
        )));
    }
    if manifest.token_dim != token_dim {
        return Err(stale(format!(
            "persistent segmented multi manifest token_dim {} != expected token_dim {token_dim}",
            manifest.token_dim
        )));
    }
    if manifest.base_seq != base_seq {
        return Err(stale(format!(
            "persistent segmented multi manifest seq {} != expected seq {base_seq}; rebuild the vault search indexes",
            manifest.base_seq
        )));
    }
    if manifest.row_count != row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != expected {row_count}; rebuild the vault search indexes",
            manifest.row_count
        )));
    }
    if manifest.token_count != token_count {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != expected {token_count}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    if manifest.row_count > 0 && manifest.segments.is_empty() {
        return Err(stale(
            "persistent segmented multi manifest has rows but no segment files; rebuild the vault search indexes",
        ));
    }
    let row_sum = manifest.segments.iter().try_fold(0usize, |sum, segment| {
        checked_rel(&segment.index_rel)?;
        if segment.sha256.len() != 64
            || !segment.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(stale(format!(
                "persistent segmented multi segment {} has invalid sha256",
                segment.index_rel
            )));
        }
        sum.checked_add(segment.row_count)
            .ok_or_else(|| stale("persistent segmented multi manifest row_count overflow"))
    })?;
    let token_sum = manifest.segments.iter().try_fold(0usize, |sum, segment| {
        sum.checked_add(segment.token_count)
            .ok_or_else(|| stale("persistent segmented multi manifest token_count overflow"))
    })?;
    if row_sum != manifest.row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != segment row sum {row_sum}; rebuild the vault search indexes",
            manifest.row_count
        )));
    }
    if token_sum != manifest.token_count {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != segment token sum {token_sum}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    Ok(())
}

fn summarize_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
) -> CliResult<ReusedMultiSegments> {
    let mut ids = BTreeSet::new();
    let mut token_count = 0usize;
    for segment in &manifest.segments {
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
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
        refs: manifest.segments.clone(),
        ids,
        token_count,
    })
}
