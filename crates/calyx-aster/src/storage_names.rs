//! Canonical on-disk file-name contract for Aster-owned directories.
//!
//! Aster's `cf/<family>/` directories are shared by three writers with
//! disjoint canonical name shapes, and the WAL directory has one:
//!
//! - LSM router flush: `{seq:020}.sst`
//! - durable group-commit batch: `{seq:020}-{index:04}.sst`
//! - compaction output: `compacted-{seq:020}.sst`
//! - WAL segment: `{index:020}.wal`
//!
//! Recovery and scan paths previously claimed files by "parse failure means
//! the file belongs to another subsystem", which silently dropped corrupt or
//! foreign names from replay and durable readback. This module is the single
//! fail-closed authority: every `*.sst` / `*.wal` name must classify into a
//! canonical shape, otherwise the caller receives a typed
//! `CALYX_ASTER_CORRUPT_SHARD` error instead of silent data loss.

use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Result, SlotId};
use std::path::Path;

/// Canonical SST file-name classes; each variant names the subsystem that
/// owns files of that shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SstName {
    /// LSM router memtable flush: `{seq:020}.sst`.
    Router { seq: u64 },
    /// Durable group-commit batch (and CLI/recurrence compaction slots in the
    /// `-9000..=-9999` index range): `{seq:020}-{index:04}.sst`.
    DurableBatch { seq: u64, index: usize },
    /// Compaction output: `compacted-{seq:020}.sst`.
    Compacted { seq: u64 },
}

/// Canonical chronological order for SST files inside one CF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SstOrderKey {
    pub seq: u64,
    pub class_rank: u8,
    pub index: usize,
}

impl SstOrderKey {
    fn from_name(name: SstName) -> Self {
        match name {
            SstName::Router { seq } => Self {
                seq,
                class_rank: 1,
                index: 0,
            },
            SstName::DurableBatch { seq, index } => Self {
                seq,
                class_rank: 2,
                index,
            },
            SstName::Compacted { seq } => Self {
                seq,
                class_rank: 3,
                index: usize::MAX,
            },
        }
    }
}

/// Classifies an SST path. Returns `Ok(None)` for paths without an `sst`
/// extension (foreign files such as locks and dot-temp files are not Aster's
/// to judge), `Ok(Some(_))` for canonical names, and a typed error for any
/// `*.sst` name that matches no canonical writer shape.
pub fn classify_sst(path: &Path) -> Result<Option<SstName>> {
    if path.extension().and_then(|value| value.to_str()) != Some("sst") {
        return Ok(None);
    }
    let stem = path.file_stem().and_then(|value| value.to_str());
    stem.and_then(classify_sst_stem)
        .map(Some)
        .ok_or_else(|| unrecognized_name(path, "SST"))
}

/// Returns the chronological sort key for a canonical SST file.
pub fn sst_order_key(path: &Path) -> Result<Option<SstOrderKey>> {
    Ok(classify_sst(path)?.map(SstOrderKey::from_name))
}

/// Returns the WAL segment index for canonical `{index:020}.wal` names,
/// `Ok(None)` for non-`.wal` files, and a typed error for any `*.wal` name
/// that is not canonical (such files would otherwise be silently excluded
/// from replay, losing committed writes).
pub fn wal_segment_index(path: &Path) -> Result<Option<u64>> {
    if path.extension().and_then(|value| value.to_str()) != Some("wal") {
        return Ok(None);
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .and_then(canonical_seq)
        .map(Some)
        .ok_or_else(|| unrecognized_name(path, "WAL"))
}

/// Parses a `cf/<name>` directory name into its column family, failing closed
/// on unknown or non-canonical names. The parse round-trips through
/// [`ColumnFamily::name`] so a miszero-padded slot directory (which writers
/// would never create) is rejected instead of silently aliasing another CF.
pub fn parse_cf_dir_name(value: &str) -> Result<ColumnFamily> {
    let cf = match value {
        "base" => ColumnFamily::Base,
        "collections" => ColumnFamily::Collections,
        "relational" => ColumnFamily::Relational,
        "document" => ColumnFamily::Document,
        "kv" => ColumnFamily::Kv,
        "timeseries" => ColumnFamily::TimeSeries,
        "blob" => ColumnFamily::Blob,
        "anchors" => ColumnFamily::Anchors,
        "ledger" => ColumnFamily::Ledger,
        "kernel" => ColumnFamily::Kernel,
        "guard" => ColumnFamily::Guard,
        "recurrence" => ColumnFamily::Recurrence,
        "graph" => ColumnFamily::Graph,
        "online" => ColumnFamily::Online,
        "reactive" => ColumnFamily::Reactive,
        "scalars" => ColumnFamily::Scalars,
        "xterm" => ColumnFamily::XTerm,
        "temporal_xterm" => ColumnFamily::TemporalXTerm,
        "assay" => ColumnFamily::Assay,
        "anneal_rollback" => ColumnFamily::AnnealRollback,
        "anneal_health" => ColumnFamily::AnnealHealth,
        "anneal_checksums" => ColumnFamily::AnnealChecksums,
        "anneal_mistakes" => ColumnFamily::AnnealMistakes,
        "anneal_replay" => ColumnFamily::AnnealReplay,
        "anneal_heads" => ColumnFamily::AnnealHeads,
        "anneal_bandit" => ColumnFamily::AnnealBandit,
        "anneal_soak" => ColumnFamily::AnnealSoak,
        "anneal_report" => ColumnFamily::AnnealReport,
        "anneal_growth" => ColumnFamily::AnnealGrowth,
        "anneal_operators" => ColumnFamily::AnnealOperators,
        "time_index" => ColumnFamily::TimeIndex,
        "index_btree" => ColumnFamily::IndexBtree,
        "index_inverted" => ColumnFamily::IndexInverted,
        _ if value.starts_with("slot_") => parse_slot_cf(value)?,
        _ => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "unknown durable CF directory {value}"
            )));
        }
    };
    if cf.name() != value {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "non-canonical CF directory {value} (canonical form is {})",
            cf.name()
        )));
    }
    Ok(cf)
}

fn parse_slot_cf(value: &str) -> Result<ColumnFamily> {
    let raw = value.ends_with(".raw");
    let slot_text = value.trim_start_matches("slot_").trim_end_matches(".raw");
    let slot = slot_text.parse::<u16>().map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("invalid slot CF directory {value}: {error}"))
    })?;
    if raw {
        Ok(ColumnFamily::slot_raw(SlotId::new(slot)))
    } else {
        Ok(ColumnFamily::slot(SlotId::new(slot)))
    }
}

fn classify_sst_stem(stem: &str) -> Option<SstName> {
    if let Some(seq_text) = stem.strip_prefix("compacted-") {
        return Some(SstName::Compacted {
            seq: canonical_seq(seq_text)?,
        });
    }
    if let Some((seq_text, index_text)) = stem.split_once('-') {
        return Some(SstName::DurableBatch {
            seq: canonical_seq(seq_text)?,
            index: canonical_index(index_text)?,
        });
    }
    Some(SstName::Router {
        seq: canonical_seq(stem)?,
    })
}

/// Accepts exactly the output of `format!("{seq:020}")` for a `u64`.
fn canonical_seq(text: &str) -> Option<u64> {
    if text.len() != 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // A 20-digit string can still exceed u64::MAX; parse failure rejects it.
    text.parse().ok()
}

/// Accepts exactly the output of `format!("{index:04}")` for a `usize`.
fn canonical_index(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let index: usize = text.parse().ok()?;
    if format!("{index:04}") != text {
        return None;
    }
    Some(index)
}

fn unrecognized_name(path: &Path, kind: &str) -> CalyxError {
    CalyxError::aster_corrupt_shard(format!(
        "unrecognized {kind} file name {}: not a canonical Aster storage name; \
         refusing to silently skip it during recovery/scan",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sst(name: &str) -> PathBuf {
        PathBuf::from("/vault/cf/base").join(name)
    }

    /// Guard: every static CF round-trips name → parse_cf_dir_name. A new
    /// `ColumnFamily` that forgets to register its directory name here fails
    /// this test immediately instead of only on a vault reopen.
    #[test]
    fn every_static_cf_dir_name_round_trips() {
        for cf in ColumnFamily::STATIC {
            let parsed = parse_cf_dir_name(&cf.name()).unwrap_or_else(|e| {
                panic!("CF {:?} dir name {} not parseable: {e}", cf, cf.name())
            });
            assert_eq!(
                parsed, cf,
                "parse_cf_dir_name must invert name() for {cf:?}"
            );
        }
        // Slot CFs (quantized + raw) also round-trip.
        for cf in [
            ColumnFamily::slot(calyx_core::SlotId::new(0)),
            ColumnFamily::slot_raw(calyx_core::SlotId::new(42)),
        ] {
            assert_eq!(parse_cf_dir_name(&cf.name()).unwrap(), cf);
        }
    }

    #[test]
    fn canonical_sst_names_classify() {
        assert_eq!(
            classify_sst(&sst("00000000000000000007.sst")).unwrap(),
            Some(SstName::Router { seq: 7 })
        );
        assert_eq!(
            classify_sst(&sst("00000000000000000007-0003.sst")).unwrap(),
            Some(SstName::DurableBatch { seq: 7, index: 3 })
        );
        assert_eq!(
            classify_sst(&sst("00000000000000000007-9999.sst")).unwrap(),
            Some(SstName::DurableBatch {
                seq: 7,
                index: 9999
            })
        );
        assert_eq!(
            classify_sst(&sst("00000000000000000007-12345.sst")).unwrap(),
            Some(SstName::DurableBatch {
                seq: 7,
                index: 12345
            })
        );
        assert_eq!(
            classify_sst(&sst("compacted-00000000000000000042.sst")).unwrap(),
            Some(SstName::Compacted { seq: 42 })
        );
    }

    #[test]
    fn sst_order_key_uses_sequence_not_filename_prefix() {
        let compacted_6 = sst_order_key(&sst("compacted-00000000000000000006.sst"))
            .unwrap()
            .unwrap();
        let router_7 = sst_order_key(&sst("00000000000000000007.sst"))
            .unwrap()
            .unwrap();
        let durable_7 = sst_order_key(&sst("00000000000000000007-0000.sst"))
            .unwrap()
            .unwrap();
        let compacted_7 = sst_order_key(&sst("compacted-00000000000000000007.sst"))
            .unwrap()
            .unwrap();

        assert!(compacted_6 < router_7);
        assert!(router_7 < durable_7);
        assert!(durable_7 < compacted_7);
    }

    #[test]
    fn non_sst_files_are_not_claimed() {
        assert_eq!(classify_sst(&sst(".append.lock")).unwrap(), None);
        assert_eq!(classify_sst(&sst("notes.txt")).unwrap(), None);
        assert_eq!(
            classify_sst(&sst(".00000000000000000007.sst.tmp")).unwrap(),
            None
        );
    }

    #[test]
    fn noncanonical_sst_names_fail_closed() {
        for name in [
            "1.sst",                          // missing zero padding
            "00000000000000000007-1.sst",     // index missing zero padding
            "00000000000000000007-01000.sst", // over-wide zero-padded index
            "compacted-1.sst",                // compacted seq missing padding
            "99999999999999999999.sst",       // 20 digits but > u64::MAX
            "0000000000000000000a.sst",       // non-digit
            "soak-00.sst",                    // legacy CLI soak name
            "compact-1764950000000.sst",      // legacy CLI compact name
            "tiered.sst",                     // legacy CLI tier name
            "00000000000000000007-.sst",      // empty index
            "garbage.sst",
        ] {
            let error = classify_sst(&sst(name)).unwrap_err();
            assert_eq!(
                error.code.to_string(),
                "CALYX_ASTER_CORRUPT_SHARD",
                "{name}"
            );
        }
    }

    #[test]
    fn wal_names_classify_and_fail_closed() {
        assert_eq!(
            wal_segment_index(Path::new("/v/wal/00000000000000000003.wal")).unwrap(),
            Some(3)
        );
        assert_eq!(
            wal_segment_index(Path::new("/v/wal/.append.lock")).unwrap(),
            None
        );
        for name in [
            "3.wal",
            "0000000000000000000x.wal",
            "99999999999999999999.wal",
        ] {
            let error = wal_segment_index(&PathBuf::from("/v/wal").join(name)).unwrap_err();
            assert_eq!(
                error.code.to_string(),
                "CALYX_ASTER_CORRUPT_SHARD",
                "{name}"
            );
        }
    }

    #[test]
    fn cf_dir_names_round_trip_and_fail_closed() {
        for cf in [
            ColumnFamily::Base,
            ColumnFamily::Recurrence,
            ColumnFamily::Document,
            ColumnFamily::slot(SlotId::new(0)),
            ColumnFamily::slot_raw(SlotId::new(7)),
            ColumnFamily::slot(SlotId::new(123)),
        ] {
            assert_eq!(parse_cf_dir_name(&cf.name()).unwrap(), cf);
        }
        for name in ["slot_5", "slot_xyz", "slot_99999", "unknown_cf", "Slot_05"] {
            let error = parse_cf_dir_name(name).unwrap_err();
            assert_eq!(
                error.code.to_string(),
                "CALYX_ASTER_CORRUPT_SHARD",
                "{name}"
            );
        }
    }
}
