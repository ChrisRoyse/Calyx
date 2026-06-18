use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, CxFlags, InputRef, LedgerRef, Lens, Modality, Panel, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::AlgorithmicLens;
use serde::Serialize;
use serde_json::json;

use crate::raw_media::{
    RetainedMediaInput, hex, media_metadata, retain_media_input, retained_pointer_path,
    verify_retained_pointer,
};

use super::data::VideoMetadata;
use super::request::{VideoReadbackRequest, VideoValidateRequest};

const VIDEO_SLOT: SlotId = SlotId::new(0);
const VIDEO_LENS_NAME: &str = "ph70-media-video-byte-features";
const METRIC_KEY: &[u8] = b"ph70/media/video/raw_ingest";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct VideoEvidence {
    pub(crate) status: &'static str,
    pub(crate) video_rows: usize,
    pub(crate) cx_ids: Vec<String>,
    pub(crate) retained_blob_count: usize,
    pub(crate) retained_bytes: u64,
    pub(crate) decoded_frame_count: u64,
    pub(crate) slot: u16,
    pub(crate) vector_dim: u32,
    pub(crate) summary_path: String,
    pub(crate) metric_cf: String,
    pub(crate) metric_cf_key_hex: String,
    pub(crate) metric_cf_value_bytes: usize,
    pub(crate) metric_cf_seq: u64,
    pub(crate) readback: VideoReadbackEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct VideoReadbackEvidence {
    pub(crate) video_rows: usize,
    pub(crate) slot_rows: usize,
    pub(crate) retained_blob_count: usize,
    pub(crate) retained_bytes: u64,
    pub(crate) cx_ids: Vec<String>,
    pub(crate) pointers: Vec<String>,
    pub(crate) source_sha256: Vec<String>,
}

struct PreparedVideo {
    row: VideoMetadata,
    retained: RetainedMediaInput,
}

pub(crate) fn validate_video_corpus(
    request: &VideoValidateRequest,
    rows: &[VideoMetadata],
) -> crate::error::CliResult<VideoEvidence> {
    let dataset_root = dataset_root(request)?;
    let lens = AlgorithmicLens::byte_features(VIDEO_LENS_NAME, Modality::Video);
    let vault_id = parse_vault_id(&request.vault_id)?;
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions {
            panel: Some(video_panel(&lens)),
            ..VaultOptions::default()
        },
    )?;
    let mut prepared = Vec::with_capacity(rows.len());
    for row in rows {
        let source = checked_dataset_path(&dataset_root, &row.path)?;
        let retained = retain_media_input(&request.vault, &source, Modality::Video)?;
        ensure_metadata_matches(row, &retained)?;
        prepared.push(PreparedVideo {
            row: row.clone(),
            retained,
        });
    }
    let mut constellations = Vec::with_capacity(prepared.len());
    for item in &prepared {
        constellations.push(video_constellation(&vault, &lens, item)?);
    }
    let ids = vault.put_batch(constellations)?;
    vault.flush()?;
    let readback = readback_vault(&vault, &request.vault)?;
    if readback.video_rows < ids.len() {
        return Err(
            "CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: fewer video rows than ingested".into(),
        );
    }
    let summary = json!({
        "trigger": "calyx media video-validate on verified media_fsv_mini videos",
        "intended_outcome": "persist real retained video bytes, decoded metadata, video vectors, provenance, and online summary",
        "video_rows": ids.len(),
        "cx_ids": ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "retained_blob_count": readback.retained_blob_count,
        "retained_bytes": readback.retained_bytes,
        "decoded_frame_count": prepared.iter().map(|item| item.row.frame_count).sum::<u64>(),
        "slot": VIDEO_SLOT.get(),
        "vector_dim": 16,
    });
    let summary_bytes = serde_json::to_vec_pretty(&summary)?;
    fs::create_dir_all(&request.metrics_dir)?;
    let summary_path = request.metrics_dir.join("media_video_ingest_summary.json");
    fs::write(&summary_path, &summary_bytes)?;
    let metric_seq = vault.write_cf(
        ColumnFamily::Online,
        METRIC_KEY.to_vec(),
        summary_bytes.clone(),
    )?;
    vault.flush()?;
    Ok(VideoEvidence {
        status: "ok",
        video_rows: ids.len(),
        cx_ids: ids.iter().map(ToString::to_string).collect(),
        retained_blob_count: readback.retained_blob_count,
        retained_bytes: readback.retained_bytes,
        decoded_frame_count: prepared.iter().map(|item| item.row.frame_count).sum(),
        slot: VIDEO_SLOT.get(),
        vector_dim: 16,
        summary_path: summary_path.display().to_string(),
        metric_cf: "online".to_string(),
        metric_cf_key_hex: hex(METRIC_KEY),
        metric_cf_value_bytes: summary_bytes.len(),
        metric_cf_seq: metric_seq,
        readback,
    })
}

pub(crate) fn readback_video_vault(
    request: &VideoReadbackRequest,
) -> crate::error::CliResult<VideoReadbackEvidence> {
    let vault_id = parse_vault_id(&request.vault_id)?;
    let vault = AsterVault::open(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )?;
    readback_vault(&vault, &request.vault)
}

fn video_constellation(
    vault: &AsterVault,
    lens: &AlgorithmicLens,
    item: &PreparedVideo,
) -> crate::error::CliResult<calyx_core::Constellation> {
    let input = &item.retained.input;
    let vector = lens.measure(input)?;
    let cx_id = vault.cx_id_for_input(&input.bytes, 1);
    let mut slots = BTreeMap::new();
    slots.insert(VIDEO_SLOT, vector);
    let mut scalars = BTreeMap::new();
    scalars.insert("media.frame_count".to_string(), item.row.frame_count as f64);
    scalars.insert("media.fps".to_string(), item.row.fps);
    scalars.insert("media.width".to_string(), f64::from(item.row.width));
    scalars.insert("media.height".to_string(), f64::from(item.row.height));
    scalars.insert("media.bytes".to_string(), item.row.bytes as f64);
    let mut metadata = media_metadata(&item.retained);
    metadata.insert("media.dataset_path".to_string(), item.row.path.clone());
    metadata.insert("media.mime".to_string(), item.row.mime.clone());
    metadata.insert(
        "media.source_title".to_string(),
        item.row.source_title.clone().unwrap_or_default(),
    );
    metadata.insert(
        "media.license".to_string(),
        item.row.license.clone().unwrap_or_default(),
    );
    metadata.insert(
        "media.source_url".to_string(),
        item.row.source_url.clone().unwrap_or_default(),
    );
    metadata.insert(
        "media.page_url".to_string(),
        item.row.page_url.clone().unwrap_or_default(),
    );
    metadata.insert(
        "media.license_url".to_string(),
        item.row.license_url.clone().unwrap_or_default(),
    );
    Ok(calyx_core::Constellation {
        cx_id,
        vault_id: vault.vault_id(),
        panel_version: 1,
        created_at: now_ms(),
        input_ref: InputRef {
            hash: item.retained.input_blake3,
            pointer: Some(item.retained.pointer.clone()),
            redacted: false,
        },
        modality: Modality::Video,
        slots,
        scalars,
        metadata,
        anchors: Vec::<Anchor>::new(),
        provenance: LedgerRef {
            seq: vault.latest_seq().saturating_add(1),
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    })
}

fn readback_vault(
    vault: &AsterVault,
    vault_dir: &Path,
) -> crate::error::CliResult<VideoReadbackEvidence> {
    let rows = vault.scan_cf_at(vault.snapshot(), ColumnFamily::Base)?;
    let mut cx_ids = Vec::new();
    let mut pointers = Vec::new();
    let mut hashes = Vec::new();
    let mut retained_bytes = 0_u64;
    let mut slot_rows = 0_usize;
    for (_, bytes) in rows {
        let cx = decode_constellation_base(&bytes)?;
        if cx.modality != Modality::Video {
            continue;
        }
        let pointer = cx.input_ref.pointer.clone().ok_or_else(|| {
            crate::error::CliError::from(calyx_core::CalyxError {
                code: "CALYX_MEDIA_POINTER_INVALID",
                message: format!("video cx {} has no retained pointer", cx.cx_id),
                remediation: "inspect the media validation vault base row",
            })
        })?;
        let sha = cx
            .metadata
            .get("media.source_sha256")
            .cloned()
            .ok_or("CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: missing source sha256")?;
        let expected_bytes = cx
            .metadata
            .get("media.bytes")
            .ok_or("CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: missing retained byte count")?
            .parse::<usize>()
            .map_err(|error| format!("CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: {error}"))?;
        verify_retained_pointer(vault_dir, &pointer, &sha, expected_bytes)?;
        let slot_bytes = vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::slot(VIDEO_SLOT),
                &slot_key(cx.cx_id),
            )?
            .ok_or_else(|| {
                format!(
                    "CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: missing slot row for {}",
                    cx.cx_id
                )
            })?;
        match decode_slot_vector(&slot_bytes)? {
            SlotVector::Dense { dim: 16, data } if data.iter().all(|value| value.is_finite()) => {
                slot_rows += 1;
            }
            other => {
                return Err(format!(
                    "CALYX_FSV_MEDIA_VIDEO_READBACK_MISMATCH: invalid video slot vector {other:?}"
                )
                .into());
            }
        }
        retained_bytes += fs::metadata(retained_pointer_path(vault_dir, &pointer)?)?.len();
        cx_ids.push(cx.cx_id.to_string());
        pointers.push(pointer);
        hashes.push(sha);
    }
    Ok(VideoReadbackEvidence {
        video_rows: cx_ids.len(),
        slot_rows,
        retained_blob_count: pointers.len(),
        retained_bytes,
        cx_ids,
        pointers,
        source_sha256: hashes,
    })
}

fn ensure_metadata_matches(
    row: &VideoMetadata,
    retained: &RetainedMediaInput,
) -> crate::error::CliResult {
    if retained.source_sha256 != row.sha256 {
        return Err(format!(
            "CALYX_FSV_MEDIA_VIDEO_SOURCE_SHA_MISMATCH: {} != {}",
            retained.source_sha256, row.sha256
        )
        .into());
    }
    if retained.bytes as u64 != row.bytes {
        return Err(format!(
            "CALYX_FSV_MEDIA_VIDEO_SOURCE_SIZE_MISMATCH: {} != {}",
            retained.bytes, row.bytes
        )
        .into());
    }
    let probe = &retained.probe;
    if probe.frame_count != Some(row.frame_count)
        || probe.width != Some(row.width)
        || probe.height != Some(row.height)
        || probe.codec != row.codec
        || probe.container != row.container
        || probe.fps.is_none_or(|fps| (fps - row.fps).abs() > 0.001)
    {
        return Err(format!(
            "CALYX_FSV_MEDIA_VIDEO_DECODE_METADATA_MISMATCH: {}",
            row.path
        )
        .into());
    }
    Ok(())
}

fn dataset_root(request: &VideoValidateRequest) -> crate::error::CliResult<PathBuf> {
    if let Some(root) = &request.dataset_root {
        return Ok(root.clone());
    }
    request
        .metadata
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "CALYX_FSV_MEDIA_VIDEO_INVALID_CONFIG: cannot derive dataset root".into())
}

fn checked_dataset_path(root: &Path, rel: &str) -> crate::error::CliResult<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(
            format!("CALYX_FSV_MEDIA_VIDEO_INVALID_PATH: {rel} escapes dataset root").into(),
        );
    }
    Ok(root.join(rel_path))
}

fn video_panel(lens: &AlgorithmicLens) -> Panel {
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: VIDEO_SLOT,
            slot_key: SlotKey::new(VIDEO_SLOT, "video_byte_features"),
            lens_id: lens.id(),
            shape: SlotShape::Dense(16),
            modality: Modality::Video,
            asymmetry: calyx_core::Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("video_byte_features".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: now_ms(),
        kernel_ref: None,
        guard_ref: None,
    }
}

fn parse_vault_id(raw: &str) -> crate::error::CliResult<VaultId> {
    raw.parse::<VaultId>()
        .map_err(|error| format!("CALYX_FSV_MEDIA_VIDEO_INVALID_CONFIG: {error}").into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
