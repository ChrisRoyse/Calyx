//! PH53 HTAP — serve one slot as BOTH a transactional row store (point reads via
//! `read_cf_at`) and an analytical column (Arrow materialization + OLAP scan), at a
//! single MVCC snapshot.
//!
//! `htap_dual_read_at` runs both independent access paths at one `seq` and proves
//! they return identical data — the HTAP contract from PRD `20 §1/§2`: OLTP and
//! OLAP served from one core with no ETL, and snapshot-consistent (a write at a
//! later seq cannot leak into an earlier snapshot on either path).

use std::path::Path;

use super::slot_column::read_materialized_slot_column;
use super::{AsterVault, SlotColumnMaterialization, encode};
use crate::cf::{ColumnFamily, slot_key};
use crate::olap::{OlapScanPlan, OlapScanResult, scan_materialized_slot_column_aggregate};
use calyx_core::{CalyxError, Clock, Result, Seq, SlotId, SlotVector};

/// Result of an HTAP dual read: the analytical column path, the transactional
/// row path, and the identity verdicts between them at one snapshot.
#[derive(Debug, Clone)]
pub struct HtapDualRead {
    pub snapshot: Seq,
    pub slot: SlotId,
    pub value_column: usize,
    pub row_count: usize,
    pub dim: u32,
    /// Analytical path: the materialized Arrow column.
    pub column: SlotColumnMaterialization,
    /// Analytical path: the OLAP aggregate over `value_column`.
    pub olap: OlapScanResult,
    /// Transactional path: aggregate recomputed from independent point reads.
    pub row_path_count: usize,
    pub row_path_sum: f64,
    pub row_path_min: f32,
    pub row_path_max: f32,
    /// Every cx's row-CF point-read bytes equal the column-path row bytes.
    pub per_row_bit_identical: bool,
    /// The transactional and analytical aggregates are bit-identical.
    pub aggregates_identical: bool,
}

impl HtapDualRead {
    /// True iff the transactional (row) and analytical (column) access paths
    /// returned identical data at the same snapshot — the HTAP guarantee.
    pub fn paths_identical(&self) -> bool {
        self.per_row_bit_identical && self.aggregates_identical
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// HTAP dual read of `slot` at `snapshot`: materialize the analytical Arrow
    /// column + OLAP-aggregate `value_column`, then INDEPENDENTLY point-read every
    /// cx through the transactional row CF and recompute the same aggregate. Both
    /// paths read the one MVCC snapshot, so they must agree bit-for-bit. The
    /// recomputation mirrors the OLAP accumulator (`sum += f64::from(v)`, f32
    /// min/max, same row order) so equality is bit-exact, not merely approximate.
    pub fn htap_dual_read_at(
        &self,
        snapshot: Seq,
        slot: SlotId,
        value_column: usize,
        output_dir: impl AsRef<Path>,
    ) -> Result<HtapDualRead> {
        // --- Analytical (column) path ---
        let column = self.materialize_slot_column_at(snapshot, slot, &output_dir)?;
        let plan = OlapScanPlan::new(value_column);
        let olap = scan_materialized_slot_column_aggregate(&column.manifest_path, plan)?;
        let readback = read_materialized_slot_column(&column.manifest_path)?;

        // --- Transactional (row) path: independent point reads at the same seq ---
        let mut per_row_bit_identical = readback.rows.len() == column.cx_ids.len();
        let mut count = 0usize;
        let mut sum = 0.0f64;
        let mut min = 0.0f32;
        let mut max = 0.0f32;
        for (idx, cx) in column.cx_ids.iter().enumerate() {
            let raw = self
                .read_cf_at(snapshot, ColumnFamily::slot(slot), &slot_key(*cx))?
                .ok_or_else(|| {
                    CalyxError::stale_derived(format!(
                        "htap point read missing cx {cx} at snapshot {snapshot}"
                    ))
                })?;
            let SlotVector::Dense { data, .. } = encode::decode_slot_vector(&raw)? else {
                return Err(CalyxError::stale_derived(
                    "htap dual read requires dense slot vectors",
                ));
            };
            if value_column >= data.len() {
                return Err(CalyxError::stale_derived(format!(
                    "htap value_column {value_column} outside dim {}",
                    data.len()
                )));
            }
            // Independent access paths must yield bit-identical row bytes.
            match readback.rows.get(idx) {
                Some(col_row) if col_row.values == data => {}
                _ => per_row_bit_identical = false,
            }
            let value = data[value_column];
            if count == 0 {
                min = value;
                max = value;
            } else {
                min = min.min(value);
                max = max.max(value);
            }
            count += 1;
            sum += f64::from(value);
        }

        let aggregates_identical = count == olap.aggregate.count
            && sum.to_bits() == olap.aggregate.sum.to_bits()
            && min.to_bits() == olap.aggregate.min.to_bits()
            && max.to_bits() == olap.aggregate.max.to_bits();

        Ok(HtapDualRead {
            snapshot,
            slot,
            value_column,
            row_count: count,
            dim: column.dim,
            column,
            olap,
            row_path_count: count,
            row_path_sum: sum,
            row_path_min: min,
            row_path_max: max,
            per_row_bit_identical,
            aggregates_identical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::VaultOptions;
    use calyx_core::{CxId, SystemClock, VaultId};
    use std::sync::atomic::{AtomicU64, Ordering};
    use ulid::Ulid;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_dir(tag: &str) -> std::path::PathBuf {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("calyx-htap-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cx(idx: u64) -> CxId {
        let mut bytes = [0u8; 16];
        bytes[8..16].copy_from_slice(&idx.to_be_bytes());
        CxId::from_bytes(bytes)
    }

    fn vault(dir: &Path) -> AsterVault<SystemClock> {
        AsterVault::new_durable(
            dir,
            VaultId::from_ulid(Ulid::from_bytes([7u8; 16])),
            b"salt".to_vec(),
            VaultOptions::default(),
        )
        .expect("durable vault")
    }

    fn write_dense(v: &AsterVault<SystemClock>, slot: SlotId, id: CxId, data: Vec<f32>) -> Seq {
        let dim = data.len() as u32;
        v.write_cf(
            ColumnFamily::slot(slot),
            slot_key(id),
            encode::encode_slot_vector(&SlotVector::Dense { dim, data }).expect("encode"),
        )
        .expect("write slot row")
    }

    #[test]
    fn dual_read_paths_identical_on_known_data() {
        let dir = test_dir("dual");
        let v = vault(&dir);
        let slot = SlotId::new(2);
        // Known rows: row i = [i, i*10, i*100]; column 1 (the i*10 column):
        // values 0,10,20 -> count 3, sum 30, min 0, max 20, avg 10.
        for i in 0..3u64 {
            write_dense(
                &v,
                slot,
                cx(i),
                vec![i as f32, (i * 10) as f32, (i * 100) as f32],
            );
        }
        v.flush().expect("flush");
        let snap = v.latest_seq();

        let out = dir.join("col");
        let dual = v.htap_dual_read_at(snap, slot, 1, &out).expect("dual read");

        assert!(
            dual.paths_identical(),
            "row and column paths must match: {dual:?}"
        );
        assert_eq!(dual.row_count, 3);
        assert_eq!(dual.olap.aggregate.count, 3);
        assert_eq!(dual.olap.aggregate.sum, 30.0);
        assert_eq!(dual.olap.aggregate.min, 0.0);
        assert_eq!(dual.olap.aggregate.max, 20.0);
        // Hand-computed (2+2=4): row path agrees with analytical aggregate.
        assert_eq!(
            dual.row_path_sum.to_bits(),
            dual.olap.aggregate.sum.to_bits()
        );
    }

    #[test]
    fn snapshot_isolation_across_both_paths() {
        let dir = test_dir("iso");
        let v = vault(&dir);
        let slot = SlotId::new(3);
        let id = cx(42);
        // seq S1: value column0 = 5.0
        write_dense(&v, slot, id, vec![5.0]);
        write_dense(&v, slot, cx(43), vec![7.0]);
        v.flush().expect("flush");
        let s1 = v.latest_seq();

        // seq S2: UPDATE cx 42 column0 -> 100.0
        write_dense(&v, slot, id, vec![100.0]);
        v.flush().expect("flush");
        let s2 = v.latest_seq();
        assert!(s2 > s1, "update must advance seq");

        // At S1 BOTH paths see the OLD value (5+7=12); the S2 write must not leak.
        let at_s1 = v
            .htap_dual_read_at(s1, slot, 0, dir.join("s1"))
            .expect("s1");
        assert!(at_s1.paths_identical());
        assert_eq!(
            at_s1.olap.aggregate.sum, 12.0,
            "S1 snapshot must exclude the S2 update"
        );

        // At S2 BOTH paths see the NEW value (100+7=107).
        let at_s2 = v
            .htap_dual_read_at(s2, slot, 0, dir.join("s2"))
            .expect("s2");
        assert!(at_s2.paths_identical());
        assert_eq!(
            at_s2.olap.aggregate.sum, 107.0,
            "S2 snapshot must include the update"
        );
    }
}
