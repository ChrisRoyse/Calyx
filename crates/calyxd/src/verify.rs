//! `calyx verify-restore` — byte-level read-back verification of a restored
//! Aster vault directory (PH67 T03).
//!
//! Opens a restored vault WITHOUT a running daemon and WITHOUT any write
//! side-effect, following the RocksDB read-only-instance discipline: every
//! file is opened read-only and the tool never creates, truncates, locks, or
//! replays anything into the vault directory. In particular it does NOT use
//! `CfRouter::open` (which creates `cf/` directories) or `DurableVault::open`
//! (which opens a WAL writer and truncates torn tails).
//!
//! A return value is a claim; the bytes are the verdict — every count comes
//! from physically scanning SST files and WAL frames, and the ledger walk
//! recomputes every hash link from genesis to tip via
//! `calyx_ledger::verify_chain`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::ledger_head::read_head_anchor;
use calyx_aster::ledger_view::parse_aster_ledger_seq;
use calyx_aster::sst::SstEntry;
use calyx_aster::sst::level::SstLevel;
use calyx_aster::vault::encode::{
    decode_constellation_base, decode_slot_vector, decode_write_batch,
};
use calyx_aster::wal::replay_dir;
use calyx_core::{CalyxError, Result as CalyxResult};
use calyx_ledger::{
    LedgerCfStore, LedgerHeadAnchor, LedgerRow, VerifyResult, decode as decode_ledger_entry,
    verify_chain,
};
use serde::Serialize;

use crate::error::DaemonError;

/// Rebuildable index directories excluded from the restic backup set (PH67
/// T01). Their absence in a restored vault is expected: it is logged but never
/// fails verification and never appears in the report.
const OPTIONAL_REBUILDABLE_DIRS: [&str; 3] = ["ann", "kernel", "guard"];

/// WAL rows grouped per column family, in replay (commit) order.
type WalOverlay = HashMap<ColumnFamily, Vec<(Vec<u8>, Vec<u8>)>>;

/// Byte-level verification report for a restored vault — the DR-drill SoT.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyRestoreReport {
    pub vault_path: PathBuf,
    pub constellation_count: u64,
    pub anchor_count: u64,
    pub ledger_entry_count: u64,
    /// Hex of the last verified chain hash (all-zero hex for an empty chain).
    pub ledger_tip_hash: String,
    pub chain_intact: bool,
    /// Total bytes across `wal/*.wal` files.
    pub wal_bytes_present: u64,
    /// Hex of the first CxId read back completely (all slot columns).
    pub first_cx_id: Option<String>,
    /// Exact `CALYX_*` failure code + detail when verification failed.
    pub error: Option<String>,
}

impl VerifyRestoreReport {
    fn empty(vault_path: &Path) -> Self {
        Self {
            vault_path: vault_path.to_path_buf(),
            constellation_count: 0,
            anchor_count: 0,
            ledger_entry_count: 0,
            ledger_tip_hash: String::new(),
            chain_intact: false,
            wal_bytes_present: 0,
            first_cx_id: None,
            error: None,
        }
    }

    /// The DR-drill pass predicate: intact chain AND real data present.
    pub fn success(&self) -> bool {
        self.error.is_none()
            && self.chain_intact
            && self.constellation_count > 0
            && self.anchor_count > 0
            && self.wal_bytes_present > 0
    }

    /// Names every unmet pass criterion so a failure is never silent.
    ///
    /// When a scan error aborted verification, only that error is reported —
    /// the counts were never measured, so naming them would be misleading.
    pub fn failure_reasons(&self) -> Vec<String> {
        if let Some(error) = &self.error {
            return vec![error.clone()];
        }
        let mut reasons = Vec::new();
        if !self.chain_intact {
            reasons.push("ledger chain not verified intact".to_string());
        }
        if self.constellation_count == 0 {
            reasons.push(
                "constellation_count=0: no constellation readable from the base CF".to_string(),
            );
        }
        if self.anchor_count == 0 {
            reasons.push("anchor_count=0: no anchor readable from the anchors CF".to_string());
        }
        if self.wal_bytes_present == 0 {
            reasons
                .push("wal_bytes_present=0: no wal/*.wal bytes in the restored vault".to_string());
        }
        reasons
    }
}

/// Verifies a restored vault with zero write side-effects.
///
/// Fail-closed contract:
/// - missing / non-directory vault path → `CALYX_DAEMON_CONFIG_INVALID`
/// - directory without any Aster state → `CALYX_DAEMON_CONFIG_INVALID`
/// - any scan or chain failure → `Ok(report)` with `chain_intact == false`
///   and `error` holding the exact `CALYX_*` code (never a silent zero-fill)
pub fn verify_restore(vault_path: &Path) -> Result<VerifyRestoreReport, DaemonError> {
    if !vault_path.is_dir() {
        return Err(DaemonError::config_invalid(format!(
            "vault path {} does not exist or is not a directory",
            vault_path.display()
        )));
    }
    if !vault_path.join("cf").is_dir() && !vault_path.join("wal").is_dir() {
        return Err(DaemonError::config_invalid(format!(
            "vault path {} holds no Aster state (neither cf/ nor wal/ exists)",
            vault_path.display()
        )));
    }
    for dir in OPTIONAL_REBUILDABLE_DIRS {
        if !vault_path.join(dir).is_dir() {
            eprintln!(
                "calyx verify-restore: optional dir {dir}/ absent in {} — rebuildable, \
                 excluded from backup; skipping",
                vault_path.display()
            );
        }
    }

    let mut report = VerifyRestoreReport::empty(vault_path);
    // Stat the WAL bytes first so the report stays truthful about what is
    // physically on disk even when a later CF scan fails closed.
    report.wal_bytes_present = match wal_total_bytes(vault_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            report.error = Some(error.to_string());
            return Ok(report);
        }
    };
    let scan = match scan_vault(vault_path) {
        Ok(scan) => scan,
        Err(error) => {
            report.error = Some(error.to_string());
            return Ok(report);
        }
    };
    report.constellation_count = scan.constellation_count;
    report.anchor_count = scan.anchor_count;
    report.ledger_entry_count = scan.ledger_rows.len() as u64;
    report.first_cx_id = scan.first_cx_id;

    let store = RestoredLedgerRows {
        rows: scan.ledger_rows,
        anchor: scan.ledger_anchor,
    };
    let head = store.rows.last().map_or(0, |row| row.seq.saturating_add(1));
    match verify_chain(&store, 0..head) {
        Ok(VerifyResult::Intact { .. }) => match tip_hash(&store.rows) {
            Ok(hash) => {
                report.chain_intact = true;
                report.ledger_tip_hash = hash;
            }
            Err(error) => report.error = Some(error.to_string()),
        },
        Ok(VerifyResult::Broken { at_seq, .. }) => {
            report.error = Some(format!("CALYX_LEDGER_CHAIN_BROKEN at seq={at_seq}"));
        }
        Ok(VerifyResult::Corrupt { at_seq, reason }) => {
            report.error = Some(format!("CALYX_LEDGER_CORRUPT at seq={at_seq}: {reason}"));
        }
        Err(error) => report.error = Some(error.to_string()),
    }
    Ok(report)
}

struct VaultScan {
    constellation_count: u64,
    anchor_count: u64,
    first_cx_id: Option<String>,
    ledger_rows: Vec<LedgerRow>,
    ledger_anchor: Option<LedgerHeadAnchor>,
}

fn scan_vault(vault: &Path) -> CalyxResult<VaultScan> {
    let overlay = read_wal_overlay(vault)?;
    let base = merged_cf(vault, ColumnFamily::Base, &overlay)?;
    let anchors = merged_cf(vault, ColumnFamily::Anchors, &overlay)?;
    let ledger_rows = merged_ledger_rows(vault, &overlay)?;
    let ledger_anchor = read_head_anchor(vault)?;
    let first_cx_id = match base.iter().next() {
        Some((key, value)) => Some(read_back_first_constellation(vault, &overlay, key, value)?),
        None => None,
    };
    Ok(VaultScan {
        constellation_count: base.len() as u64,
        anchor_count: anchors.len() as u64,
        first_cx_id,
        ledger_rows,
        ledger_anchor,
    })
}

/// Replays the WAL directory into per-CF row lists without touching disk.
///
/// A torn tail fails closed: live recovery would truncate it, but a restored
/// snapshot must replay cleanly end-to-end — a torn segment means the backup
/// captured an inconsistent WAL and the restore cannot be trusted.
fn read_wal_overlay(vault: &Path) -> CalyxResult<WalOverlay> {
    let wal_dir = vault.join("wal");
    let mut overlay = WalOverlay::new();
    if !wal_dir.is_dir() {
        return Ok(overlay);
    }
    let replay = replay_dir(&wal_dir)?;
    if let Some(torn) = replay.torn_tail {
        return Err(torn.error());
    }
    for record in replay.records {
        for row in decode_write_batch(&record.payload)? {
            overlay
                .entry(row.cf)
                .or_default()
                .push((row.key, row.value));
        }
    }
    Ok(overlay)
}

/// Merged view of one CF: SST files (newest file wins) overlaid with WAL rows
/// in commit order (latest write wins) — the engine's own merge semantics.
fn merged_cf(
    vault: &Path,
    cf: ColumnFamily,
    overlay: &WalOverlay,
) -> CalyxResult<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut rows = BTreeMap::new();
    for entry in read_cf_ssts(vault, cf)? {
        rows.insert(entry.key, entry.value);
    }
    if let Some(wal_rows) = overlay.get(&cf) {
        for (key, value) in wal_rows {
            rows.insert(key.clone(), value.clone());
        }
    }
    Ok(rows)
}

/// Lists `<vault>/cf/<name>/*.sst` and merges them newest-file-wins, exactly
/// like the engine's level ordering — without creating any directory.
fn read_cf_ssts(vault: &Path, cf: ColumnFamily) -> CalyxResult<Vec<SstEntry>> {
    let dir = vault.join("cf").join(cf.name());
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in
        fs::read_dir(&dir).map_err(|error| read_error(&dir, "read CF dir", &error.to_string()))?
    {
        let path = entry
            .map_err(|error| read_error(&dir, "read CF dir entry", &error.to_string()))?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort();
    SstLevel::from_oldest_first(files).iter()
}

/// Merges ledger rows from SSTs and WAL. The ledger CF is append-only: two
/// different byte strings for one seq is corruption, never a legitimate
/// update (mirrors `calyx_aster::ledger_view`).
fn merged_ledger_rows(vault: &Path, overlay: &WalOverlay) -> CalyxResult<Vec<LedgerRow>> {
    let mut rows: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    for entry in read_cf_ssts(vault, ColumnFamily::Ledger)? {
        let seq = parse_aster_ledger_seq(&entry.key)?;
        insert_ledger_bytes(&mut rows, seq, entry.value)?;
    }
    if let Some(wal_rows) = overlay.get(&ColumnFamily::Ledger) {
        for (key, value) in wal_rows {
            let seq = parse_aster_ledger_seq(key)?;
            insert_ledger_bytes(&mut rows, seq, value.clone())?;
        }
    }
    Ok(rows
        .into_iter()
        .map(|(seq, bytes)| LedgerRow { seq, bytes })
        .collect())
}

fn insert_ledger_bytes(
    rows: &mut BTreeMap<u64, Vec<u8>>,
    seq: u64,
    bytes: Vec<u8>,
) -> CalyxResult<()> {
    if let Some(existing) = rows.get(&seq) {
        if existing == &bytes {
            return Ok(());
        }
        return Err(CalyxError::ledger_corrupt(format!(
            "divergent ledger bytes for seq {seq} between SST and WAL"
        )));
    }
    rows.insert(seq, bytes);
    Ok(())
}

/// Reads the first constellation back COMPLETELY: the base row decodes, the
/// base key matches the embedded CxId byte-for-byte, and every slot column
/// listed in the base header is physically present and decodable.
fn read_back_first_constellation(
    vault: &Path,
    overlay: &WalOverlay,
    key: &[u8],
    value: &[u8],
) -> CalyxResult<String> {
    if value.is_empty() {
        return Err(CalyxError::aster_corrupt_shard(
            "first base CF row is empty",
        ));
    }
    let constellation = decode_constellation_base(value)?;
    if key != constellation.cx_id.as_bytes() {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "base CF key {} does not match embedded cx_id {}",
            hex(key),
            hex(constellation.cx_id.as_bytes())
        )));
    }
    for slot in constellation.slots.keys() {
        let slot_rows = merged_cf(vault, ColumnFamily::slot(*slot), overlay)?;
        let bytes = slot_rows
            .get(&slot_key(constellation.cx_id))
            .ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "slot {slot} column missing for first constellation {}",
                    hex(constellation.cx_id.as_bytes())
                ))
            })?;
        decode_slot_vector(bytes)?;
    }
    Ok(hex(key))
}

/// Hex of the last entry's verified chain hash; all-zero hex when empty.
fn tip_hash(rows: &[LedgerRow]) -> CalyxResult<String> {
    match rows.last() {
        Some(row) => Ok(hex(&decode_ledger_entry(&row.bytes)?.entry_hash)),
        None => Ok(hex(&[0u8; 32])),
    }
}

fn wal_total_bytes(vault: &Path) -> CalyxResult<u64> {
    let wal_dir = vault.join("wal");
    if !wal_dir.is_dir() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in fs::read_dir(&wal_dir)
        .map_err(|error| read_error(&wal_dir, "read WAL dir", &error.to_string()))?
    {
        let path = entry
            .map_err(|error| read_error(&wal_dir, "read WAL dir entry", &error.to_string()))?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("wal") {
            total += fs::metadata(&path)
                .map_err(|error| read_error(&path, "stat WAL file", &error.to_string()))?
                .len();
        }
    }
    Ok(total)
}

fn read_error(path: &Path, action: &str, detail: &str) -> CalyxError {
    CalyxError::disk_pressure(format!("{action} {}: {detail}", path.display()))
}

/// In-memory read-only ledger view over the restored rows.
struct RestoredLedgerRows {
    rows: Vec<LedgerRow>,
    anchor: Option<LedgerHeadAnchor>,
}

impl LedgerCfStore for RestoredLedgerRows {
    fn scan(&self) -> CalyxResult<Vec<LedgerRow>> {
        Ok(self.rows.clone())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> CalyxResult<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "verify-restore is read-only; rejected append for seq {seq}"
        )))
    }

    fn head_anchor(&self) -> CalyxResult<Option<LedgerHeadAnchor>> {
        Ok(self.anchor.clone())
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intact_report() -> VerifyRestoreReport {
        VerifyRestoreReport {
            vault_path: PathBuf::from("/tmp/v"),
            constellation_count: 3,
            anchor_count: 3,
            ledger_entry_count: 3,
            ledger_tip_hash: "ab".repeat(32),
            chain_intact: true,
            wal_bytes_present: 128,
            first_cx_id: Some("01".repeat(16)),
            error: None,
        }
    }

    #[test]
    fn success_requires_all_pass_criteria() {
        assert!(intact_report().success());
        for mutate in [
            (|report: &mut VerifyRestoreReport| report.chain_intact = false)
                as fn(&mut VerifyRestoreReport),
            |report| report.constellation_count = 0,
            |report| report.anchor_count = 0,
            |report| report.wal_bytes_present = 0,
            |report| report.error = Some("CALYX_LEDGER_CHAIN_BROKEN at seq=5".to_string()),
        ] {
            let mut report = intact_report();
            mutate(&mut report);
            assert!(!report.success());
            assert!(!report.failure_reasons().is_empty());
        }
    }

    #[test]
    fn failure_reasons_name_every_unmet_criterion() {
        let mut report = intact_report();
        report.constellation_count = 0;
        report.anchor_count = 0;
        report.wal_bytes_present = 0;
        let reasons = report.failure_reasons().join("\n");
        assert!(reasons.contains("constellation_count=0"));
        assert!(reasons.contains("anchor_count=0"));
        assert!(reasons.contains("wal_bytes_present=0"));
    }

    #[test]
    fn hex_round_trips_known_bytes() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}
