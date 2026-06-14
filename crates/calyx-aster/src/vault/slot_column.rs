use super::{AsterVault, encode};
use crate::cf::ColumnFamily;
use crate::mmap_col::MmapColumn;
use crate::sst::arrow::{decode_column_chunk, encode_column_chunk};
use calyx_core::{CalyxError, Clock, CxId, Result, Seq, SlotId, SlotVector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MANIFEST_MAGIC: &str = "CXSC1";
const MANIFEST_VERSION: u32 = 1;
const CHUNK_FILE: &str = "slot-column.cxa1";
const MANIFEST_FILE: &str = "slot-column-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotColumnMaterialization {
    pub slot: SlotId,
    pub snapshot: Seq,
    pub rows: usize,
    pub dim: u32,
    pub manifest_path: PathBuf,
    pub chunk_path: PathBuf,
    pub manifest_sha256: String,
    pub chunk_sha256: String,
    pub cx_ids: Vec<CxId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotColumnManifest {
    pub magic: String,
    pub version: u32,
    pub slot: SlotId,
    pub snapshot: Seq,
    pub rows: usize,
    pub dim: u32,
    pub cx_ids: Vec<CxId>,
    pub chunk_file: String,
    pub chunk_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotColumnReadback {
    pub manifest: SlotColumnManifest,
    pub manifest_path: PathBuf,
    pub chunk_path: PathBuf,
    pub rows: Vec<SlotColumnRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotColumnRow {
    pub cx_id: CxId,
    pub values: Vec<f32>,
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn materialize_slot_column_at(
        &self,
        snapshot: Seq,
        slot: SlotId,
        output_dir: impl AsRef<Path>,
    ) -> Result<SlotColumnMaterialization> {
        let rows = self.dense_slot_rows_at(snapshot, slot)?;
        let output_dir = output_dir.as_ref();
        fs::create_dir_all(output_dir)
            .map_err(|error| storage_error("create slot-column output dir", error))?;

        let refs = rows
            .iter()
            .map(|row| row.values.as_slice())
            .collect::<Vec<_>>();
        let chunk_bytes = encode_column_chunk(&refs)?;
        let chunk_sha256 = sha256_hex(&chunk_bytes);
        let chunk_path = output_dir.join(CHUNK_FILE);
        write_atomic(&chunk_path, &chunk_bytes)?;

        let dim = rows
            .first()
            .ok_or_else(|| CalyxError::stale_derived("slot column has no dense rows"))?
            .values
            .len() as u32;
        let manifest = SlotColumnManifest {
            magic: MANIFEST_MAGIC.to_string(),
            version: MANIFEST_VERSION,
            slot,
            snapshot,
            rows: rows.len(),
            dim,
            cx_ids: rows.iter().map(|row| row.cx_id).collect(),
            chunk_file: CHUNK_FILE.to_string(),
            chunk_sha256: chunk_sha256.clone(),
        };
        let manifest_bytes = encode_manifest(&manifest)?;
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        let manifest_path = output_dir.join(MANIFEST_FILE);
        write_atomic(&manifest_path, &manifest_bytes)?;

        Ok(SlotColumnMaterialization {
            slot,
            snapshot,
            rows: manifest.rows,
            dim,
            manifest_path,
            chunk_path,
            manifest_sha256,
            chunk_sha256,
            cx_ids: manifest.cx_ids,
        })
    }

    fn dense_slot_rows_at(&self, snapshot: Seq, slot: SlotId) -> Result<Vec<SlotColumnRow>> {
        let rows = self.rows.scan_cf_at(
            self.snapshot_handle(snapshot),
            ColumnFamily::slot(slot),
            &self.clock,
        )?;
        if rows.is_empty() {
            return Err(CalyxError::stale_derived(format!(
                "slot {slot} has no rows to materialize"
            )));
        }

        let mut out = Vec::with_capacity(rows.len());
        let mut dim = None;
        for (key, value) in rows {
            let cx_id = cx_id_from_key(&key)?;
            let vector = encode::decode_slot_vector(&value)?;
            let SlotVector::Dense { dim: row_dim, data } = vector else {
                return Err(CalyxError::stale_derived(
                    "slot column materialization requires dense slot vectors",
                ));
            };
            if let Some(expected) = dim {
                if expected != row_dim {
                    return Err(CalyxError::aster_corrupt_shard(
                        "slot column dense dimensions differ",
                    ));
                }
            } else {
                dim = Some(row_dim);
            }
            out.push(SlotColumnRow {
                cx_id,
                values: data,
            });
        }
        Ok(out)
    }
}

pub fn read_materialized_slot_column(
    manifest_path: impl AsRef<Path>,
) -> Result<SlotColumnReadback> {
    let manifest_path = manifest_path.as_ref();
    let manifest_bytes =
        fs::read(manifest_path).map_err(|error| storage_error("read slot manifest", error))?;
    let manifest = decode_manifest(&manifest_bytes)?;
    let parent = manifest_path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("slot manifest has no parent"))?;
    if manifest.chunk_file != CHUNK_FILE {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column manifest chunk path invalid",
        ));
    }
    let chunk_path = parent.join(CHUNK_FILE);
    let column = MmapColumn::open(&chunk_path)?;
    let chunk_bytes = column.as_bytes();
    let actual_sha256 = sha256_hex(chunk_bytes);
    if actual_sha256 != manifest.chunk_sha256 {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column chunk sha256 mismatch",
        ));
    }

    let chunk = decode_column_chunk(chunk_bytes)?;
    if chunk.n_rows() != manifest.rows || chunk.dim() != manifest.dim as usize {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column manifest shape mismatch",
        ));
    }
    if manifest.cx_ids.len() != manifest.rows {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column cx_id count mismatch",
        ));
    }

    let mut rows = Vec::with_capacity(manifest.rows);
    for (index, cx_id) in manifest.cx_ids.iter().copied().enumerate() {
        rows.push(SlotColumnRow {
            cx_id,
            values: chunk.row(index)?.to_vec(),
        });
    }
    Ok(SlotColumnReadback {
        manifest,
        manifest_path: manifest_path.to_path_buf(),
        chunk_path,
        rows,
    })
}

fn cx_id_from_key(key: &[u8]) -> Result<CxId> {
    if key.len() != 16 {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column row key is not a CxId",
        ));
    }
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(key);
    Ok(CxId::from_bytes(bytes))
}

fn encode_manifest(manifest: &SlotColumnManifest) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(manifest).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("encode slot column manifest: {error}"))
    })
}

fn decode_manifest(bytes: &[u8]) -> Result<SlotColumnManifest> {
    let manifest: SlotColumnManifest = serde_json::from_slice(bytes).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("decode slot column manifest: {error}"))
    })?;
    if manifest.magic != MANIFEST_MAGIC || manifest.version != MANIFEST_VERSION {
        return Err(CalyxError::aster_corrupt_shard(
            "slot column manifest version mismatch",
        ));
    }
    Ok(manifest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file =
            File::create(&tmp).map_err(|error| storage_error("create slot temp", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write slot temp", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync slot temp", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("rename slot artifact", error))?;
    sync_parent(path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("slot artifact has no parent"))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| storage_error("fsync slot artifact dir", error))
}

#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<()> {
    path.parent()
        .ok_or_else(|| CalyxError::disk_pressure("slot artifact has no parent"))?;
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf::{base_key, slot_key};
    use calyx_core::{CxFlags, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn materialized_slot_column_reads_back_consistent_with_row_cf() {
        let root = test_dir("slot-column");
        let vault_dir = root.join("vault");
        let vault = AsterVault::new_durable(
            &vault_dir,
            vault_id(),
            b"salt".to_vec(),
            super::super::VaultOptions::default(),
        )
        .expect("open durable vault");
        let slot = SlotId::new(2);
        let first = constellation(&vault, b"one", slot, &[1.0, 2.0, 3.0]);
        let second = constellation(&vault, b"two", slot, &[4.0, 5.5, 6.25]);
        let first_id = first.cx_id;
        let second_id = second.cx_id;

        vault.put(first).expect("put first");
        vault.put(second).expect("put second");
        vault.flush().expect("flush row CFs");
        let snapshot = vault.latest_seq();
        let first_row = vault
            .read_cf_at(snapshot, ColumnFamily::slot(slot), &slot_key(first_id))
            .expect("read first row")
            .expect("first slot row");
        assert_eq!(first_row[0], 0);

        let output = root.join("materialized").join("slot_02");
        let materialized = vault
            .materialize_slot_column_at(snapshot, slot, &output)
            .expect("materialize slot column");
        assert_eq!(materialized.rows, 2);
        assert_eq!(materialized.dim, 3);
        assert!(
            fs::read(&materialized.chunk_path)
                .expect("read chunk")
                .starts_with(b"CXA1")
        );

        let readback =
            read_materialized_slot_column(&materialized.manifest_path).expect("readback column");
        assert_eq!(readback.manifest.cx_ids, vec![first_id, second_id]);
        assert_bits_eq(&readback.rows[0].values, &[1.0, 2.0, 3.0]);
        assert_bits_eq(&readback.rows[1].values, &[4.0, 5.5, 6.25]);

        let reopened = AsterVault::open(
            &vault_dir,
            vault_id(),
            b"salt".to_vec(),
            super::super::VaultOptions::default(),
        )
        .expect("reopen vault");
        assert!(
            reopened
                .read_cf_at(
                    reopened.latest_seq(),
                    ColumnFamily::Base,
                    &base_key(first_id)
                )
                .expect("read reopened base")
                .is_some()
        );
        assert_eq!(
            reopened
                .read_slot_vector_at(reopened.latest_seq(), first_id, slot)
                .expect("read reopened slot")
                .expect("slot exists"),
            SlotVector::Dense {
                dim: 3,
                data: vec![1.0, 2.0, 3.0]
            }
        );
        let after_reopen =
            read_materialized_slot_column(&materialized.manifest_path).expect("read after reopen");
        assert_bits_eq(&after_reopen.rows[0].values, &[1.0, 2.0, 3.0]);
        cleanup(root);
    }

    #[test]
    fn materialization_edges_fail_closed() {
        let root = test_dir("slot-column-edge");
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(10));
        let slot = SlotId::new(4);
        let empty = vault
            .materialize_slot_column_at(vault.latest_seq(), slot, root.join("empty"))
            .expect_err("empty slot rejected");
        assert_eq!(empty.code, "CALYX_STALE_DERIVED");

        let absent = constellation_absent(&vault, b"absent", slot);
        vault.put(absent).expect("put absent");
        let non_dense = vault
            .materialize_slot_column_at(vault.latest_seq(), slot, root.join("absent"))
            .expect_err("non-dense rejected");
        assert_eq!(non_dense.code, "CALYX_STALE_DERIVED");
        cleanup(root);
    }

    #[test]
    fn corrupt_materialized_chunk_hash_fails_closed() {
        let root = test_dir("slot-column-corrupt");
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(10));
        let slot = SlotId::new(2);
        let cx = constellation(&vault, b"one", slot, &[1.0, 2.0]);
        vault.put(cx).expect("put row");
        let materialized = vault
            .materialize_slot_column_at(vault.latest_seq(), slot, &root)
            .expect("materialize");
        let mut bytes = fs::read(&materialized.chunk_path).expect("read chunk");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&materialized.chunk_path, bytes).expect("corrupt chunk");

        let error = read_materialized_slot_column(&materialized.manifest_path)
            .expect_err("hash mismatch rejected");
        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(root);
    }

    fn constellation(
        vault: &AsterVault<impl Clock>,
        input: &[u8],
        slot: SlotId,
        values: &[f32],
    ) -> calyx_core::Constellation {
        let mut slots = BTreeMap::new();
        slots.insert(
            slot,
            SlotVector::Dense {
                dim: values.len() as u32,
                data: values.to_vec(),
            },
        );
        constellation_with_slots(vault, input, slots)
    }

    fn constellation_absent(
        vault: &AsterVault<impl Clock>,
        input: &[u8],
        slot: SlotId,
    ) -> calyx_core::Constellation {
        let mut slots = BTreeMap::new();
        slots.insert(
            slot,
            SlotVector::Absent {
                reason: calyx_core::AbsentReason::Deferred,
            },
        );
        constellation_with_slots(vault, input, slots)
    }

    fn constellation_with_slots(
        vault: &AsterVault<impl Clock>,
        input: &[u8],
        slots: BTreeMap<SlotId, SlotVector>,
    ) -> calyx_core::Constellation {
        let cx_id = vault.cx_id_for_input(input, 1);
        let mut input_hash = [0_u8; 32];
        input_hash[..input.len()].copy_from_slice(input);
        calyx_core::Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 1,
            created_at: 10,
            input_ref: InputRef {
                hash: input_hash,
                pointer: Some(format!("synthetic://{}", String::from_utf8_lossy(input))),
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 1,
                hash: [7; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                ..CxFlags::default()
            },
        }
    }

    fn assert_bits_eq(left: &[f32], right: &[f32]) {
        let left_bits = left.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        let right_bits = right
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(left_bits, right_bits);
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).expect("cleanup test dir");
    }
}
