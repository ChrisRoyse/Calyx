//! `calyx htap-validate` — PH53 HTAP dual-path FSV (#587).
//!
//! Proves the HTAP contract (PRD `20 §1/§2`): one collection slot served as BOTH
//! a transactional row store and an analytical column at a single MVCC snapshot,
//! where a point read (row path) and a columnar aggregate (column path) return
//! identical underlying values at the same seq — and a later write cannot leak
//! into an earlier snapshot on either path.
//!
//! Writes deterministic synthetic rows (row i, col c = i * 10^c — hand-computable
//! aggregates, the 2+2=4 discipline) to a real durable Aster vault, runs
//! `htap_dual_read_at`, audits >=3 edge cases (snapshot isolation under update,
//! single row, empty slot), and persists byte-level FSV artifacts for independent
//! readback. Fail-closed: any path divergence returns the exact `CALYX_FSV_HTAP_*`
//! code.

use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::encode::encode_slot_vector;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, Seq, SlotId, SlotVector, SystemClock, VaultId};
use serde_json::json;

use crate::error::{CliError, CliResult};

const PATHS_DIVERGED: &str = "CALYX_FSV_HTAP_PATHS_DIVERGED";
const AGGREGATE_MISMATCH: &str = "CALYX_FSV_HTAP_AGGREGATE_MISMATCH";
const SNAPSHOT_LEAK: &str = "CALYX_FSV_HTAP_SNAPSHOT_LEAK";
const EMPTY_NOT_REJECTED: &str = "CALYX_FSV_HTAP_EMPTY_SLOT_NOT_REJECTED";
const REMEDIATION: &str =
    "inspect the named source of truth: row CF point reads vs the Arrow column at the snapshot seq";

const DATA_SLOT: SlotId = SlotId::new(7);
const SINGLE_SLOT: SlotId = SlotId::new(8);
const EMPTY_SLOT: SlotId = SlotId::new(9);

struct Args {
    vault: PathBuf,
    out: PathBuf,
    rows: usize,
    dim: usize,
    value_column: usize,
}

impl Args {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut out = None;
        let mut rows = 256usize;
        let mut dim = 4usize;
        let mut value_column = 1usize;
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--vault" => vault = Some(PathBuf::from(next()?)),
                "--out" => out = Some(PathBuf::from(next()?)),
                "--rows" => {
                    rows = next()?
                        .parse()
                        .map_err(|_| CliError::usage("--rows must be a number"))?
                }
                "--dim" => {
                    dim = next()?
                        .parse()
                        .map_err(|_| CliError::usage("--dim must be a number"))?
                }
                "--value-column" => {
                    value_column = next()?
                        .parse()
                        .map_err(|_| CliError::usage("--value-column must be a number"))?
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        let out = out.unwrap_or_else(|| vault.join("htap-fsv"));
        if rows == 0 {
            return Err(CliError::usage("--rows must be > 0"));
        }
        if value_column >= dim {
            return Err(CliError::usage("--value-column must be < --dim"));
        }
        Ok(Self {
            vault,
            out,
            rows,
            dim,
            value_column,
        })
    }
}

/// Synthetic value: row `i`, column `c` = `i * 10^c`. Exact in f32 for the
/// default scale (i<=255, c<=3 -> max 255000 < 2^24), so f64 sums are exact.
fn value(i: usize, c: usize) -> f32 {
    (i as f32) * 10f32.powi(c as i32)
}

fn cx(i: usize) -> CxId {
    let mut bytes = [0u8; 16];
    bytes[8..16].copy_from_slice(&(i as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn write_dense(
    vault: &AsterVault<SystemClock>,
    slot: SlotId,
    id: CxId,
    data: Vec<f32>,
) -> CliResult<Seq> {
    let dim = data.len() as u32;
    let bytes = encode_slot_vector(&SlotVector::Dense { dim, data }).map_err(CliError::Calyx)?;
    vault
        .write_cf(ColumnFamily::slot(slot), slot_key(id), bytes)
        .map_err(CliError::Calyx)
}

fn fsv_err(code: &'static str, message: String) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message,
        remediation: REMEDIATION,
    })
}

fn write_json(out: &Path, name: &str, value: &serde_json::Value) -> CliResult<()> {
    std::fs::write(
        out.join(name),
        serde_json::to_vec_pretty(value).map_err(CliError::from)?,
    )
    .map_err(|e| CliError::io(format!("write {name}: {e}")))
}

pub(crate) fn run(args: &[String]) -> CliResult {
    let args = Args::parse(args)?;
    std::fs::create_dir_all(&args.out).map_err(|e| CliError::io(format!("create out dir: {e}")))?;
    let vault = AsterVault::new_durable(
        &args.vault,
        VaultId::from_ulid(ulid::Ulid::from_bytes([0x5a; 16])),
        b"htap-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .map_err(CliError::Calyx)?;

    // --- Write N synthetic rows (transactional path source of truth) ---
    for i in 0..args.rows {
        let data = (0..args.dim).map(|c| value(i, c)).collect::<Vec<_>>();
        write_dense(&vault, DATA_SLOT, cx(i), data)?;
    }
    vault.flush().map_err(CliError::Calyx)?;
    let snapshot = vault.latest_seq();

    // --- HTAP dual read at one snapshot ---
    let dual = vault
        .htap_dual_read_at(
            snapshot,
            DATA_SLOT,
            args.value_column,
            args.out.join("col-main"),
        )
        .map_err(CliError::Calyx)?;

    // Gate 1: the two access paths must be bit-identical.
    if !dual.paths_identical() {
        return Err(fsv_err(
            PATHS_DIVERGED,
            format!(
                "per_row_bit_identical={} aggregates_identical={} at seq {snapshot}",
                dual.per_row_bit_identical, dual.aggregates_identical
            ),
        ));
    }

    // Gate 2: hand-computed aggregate (2+2=4) must equal the analytical aggregate.
    let c = args.value_column;
    let expect_sum: f64 = (0..args.rows).map(|i| f64::from(value(i, c))).sum();
    let expect_min = value(0, c);
    let expect_max = value(args.rows - 1, c);
    let agg = &dual.olap.aggregate;
    if agg.count != args.rows
        || agg.sum.to_bits() != expect_sum.to_bits()
        || agg.min.to_bits() != expect_min.to_bits()
        || agg.max.to_bits() != expect_max.to_bits()
    {
        return Err(fsv_err(
            AGGREGATE_MISMATCH,
            format!(
                "expected count={} sum={expect_sum} min={expect_min} max={expect_max}, got count={} sum={} min={} max={}",
                args.rows, agg.count, agg.sum, agg.min, agg.max
            ),
        ));
    }

    // --- Edge 1: MVCC snapshot isolation under a later UPDATE ---
    // Update row 0's value column at a higher seq; the original snapshot must NOT
    // see it on EITHER path, and the new snapshot must see it on BOTH.
    let mut updated = (0..args.dim).map(|cc| value(0, cc)).collect::<Vec<_>>();
    let bumped = value(0, c) + 1_000_000.0;
    updated[c] = bumped;
    write_dense(&vault, DATA_SLOT, cx(0), updated)?;
    vault.flush().map_err(CliError::Calyx)?;
    let snapshot2 = vault.latest_seq();

    let at_old = vault
        .htap_dual_read_at(snapshot, DATA_SLOT, c, args.out.join("col-old"))
        .map_err(CliError::Calyx)?;
    let at_new = vault
        .htap_dual_read_at(snapshot2, DATA_SLOT, c, args.out.join("col-new"))
        .map_err(CliError::Calyx)?;
    if at_old.olap.aggregate.sum.to_bits() != expect_sum.to_bits() || !at_old.paths_identical() {
        return Err(fsv_err(
            SNAPSHOT_LEAK,
            format!(
                "snapshot {snapshot} leaked the seq-{snapshot2} update: old sum={} expected {expect_sum}",
                at_old.olap.aggregate.sum
            ),
        ));
    }
    let expect_sum_new = expect_sum - f64::from(value(0, c)) + f64::from(bumped);
    if at_new.olap.aggregate.sum.to_bits() != expect_sum_new.to_bits() || !at_new.paths_identical()
    {
        return Err(fsv_err(
            SNAPSHOT_LEAK,
            format!(
                "snapshot {snapshot2} did not reflect the update: new sum={} expected {expect_sum_new}",
                at_new.olap.aggregate.sum
            ),
        ));
    }

    // --- Edge 2: single-row slot — aggregate equals that one row ---
    write_dense(&vault, SINGLE_SLOT, cx(0), vec![3.5; args.dim])?;
    vault.flush().map_err(CliError::Calyx)?;
    let single = vault
        .htap_dual_read_at(
            vault.latest_seq(),
            SINGLE_SLOT,
            c,
            args.out.join("col-single"),
        )
        .map_err(CliError::Calyx)?;
    if single.olap.aggregate.count != 1
        || single.olap.aggregate.sum != 3.5
        || !single.paths_identical()
    {
        return Err(fsv_err(
            AGGREGATE_MISMATCH,
            format!(
                "single-row slot: count={} sum={}",
                single.olap.aggregate.count, single.olap.aggregate.sum
            ),
        ));
    }

    // --- Edge 3: empty slot — dual read MUST fail closed, not return zeros ---
    let empty = vault.htap_dual_read_at(
        vault.latest_seq(),
        EMPTY_SLOT,
        c,
        args.out.join("col-empty"),
    );
    let empty_code = match &empty {
        Ok(_) => {
            return Err(fsv_err(
                EMPTY_NOT_REJECTED,
                "empty slot returned Ok".to_string(),
            ));
        }
        Err(e) => e.code,
    };

    // --- Persist FSV artifacts (independent readback source of truth) ---
    let sample: Vec<serde_json::Value> = dual
        .column
        .cx_ids
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, id)| json!({ "row": i, "cx_id": format!("{id}"), "col_value": value(i, c) }))
        .collect();
    let report = json!({
        "trigger": "calyx htap-validate",
        "vault": args.vault.to_string_lossy(),
        "rows": args.rows,
        "dim": args.dim,
        "value_column": c,
        "snapshot": snapshot,
        "snapshot_after_update": snapshot2,
        "row_path_count": dual.row_path_count,
        "row_path_sum": dual.row_path_sum,
        "row_path_min": dual.row_path_min,
        "row_path_max": dual.row_path_max,
        "column_aggregate": {
            "count": agg.count, "sum": agg.sum, "min": agg.min, "max": agg.max, "avg": agg.avg,
        },
        "expected": { "sum": expect_sum, "min": expect_min, "max": expect_max },
        "per_row_bit_identical": dual.per_row_bit_identical,
        "aggregates_identical": dual.aggregates_identical,
        "paths_identical": dual.paths_identical(),
        "column_manifest_sha256": dual.column.manifest_sha256,
        "column_chunk_sha256": dual.column.chunk_sha256,
        "edges": {
            "mvcc_old_snapshot_sum": at_old.olap.aggregate.sum,
            "mvcc_new_snapshot_sum": at_new.olap.aggregate.sum,
            "mvcc_expected_new_sum": expect_sum_new,
            "single_row_sum": single.olap.aggregate.sum,
            "empty_slot_failed_closed_code": empty_code,
        },
        "sample_rows": sample,
    });
    write_json(&args.out, "htap-report.json", &report)?;
    let report_bytes = serde_json::to_vec(&report).map_err(CliError::from)?;
    write_json(
        &args.out,
        "htap-blake3.json",
        &json!({ "htap-report.json": blake3::hash(&report_bytes).to_hex().to_string() }),
    )?;

    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}
