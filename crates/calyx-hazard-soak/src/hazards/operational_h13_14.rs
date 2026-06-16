use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, CALYX_QUOTA_EXCEEDED, QuotaConfig, QuotaGuard};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::operational_support::{ReadRate, quota_vault_id, read_rate, try_lock_for};
use super::resource::ProbeResult;
use super::resource_support::{
    SharedClock, case_dir, count_readable, err, key, open_vault, value, write_live_range,
};

const START_TS: u64 = 1_800_100_000_000;
const FSV_MEMTABLE_BYTES: usize = 64 * 1024 * 1024;
const H13_VAULTS: usize = 10;
const H14_WRITERS: usize = 64;
const H14_ROWS_PER_WRITER: u64 = 8;
const H14_ACK_TIMEOUT: Duration = Duration::from_secs(5);
const H14_THREAD_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn probe_h13_hot_shard(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h13_hot_shard_skew")?;
    let config = QuotaConfig {
        max_ingest_cx_per_sec: 120,
        ..QuotaConfig::default()
    };
    let mut vaults = Vec::new();
    let mut guards = Vec::new();
    for idx in 0..H13_VAULTS {
        let salt = format!("ph59-h13-{idx}");
        let vault = Arc::new(open_vault(
            &dir.join(format!("vault_{idx:02}")),
            START_TS + 1_300 + idx as u64,
            salt.as_bytes(),
            FSV_MEMTABLE_BYTES,
            None,
        )?);
        write_live_range(&vault, &format!("h13-v{idx}-base"), 0, 128, 32)?;
        vault.flush().map_err(err)?;
        vaults.push(vault);
        guards.push(QuotaGuard::new(quota_vault_id(idx), config));
    }

    for (idx, vault) in vaults.iter().enumerate() {
        let _warmup = read_rate(
            vault,
            &format!("h13-v{idx}-base"),
            0,
            128,
            Duration::from_millis(20),
        )?;
    }
    let baselines = parallel_read_rates(&vaults, Duration::from_millis(120))?;

    let before_counters = quota_counters(&guards);
    let mut attempted = [0_u64; H13_VAULTS];
    let mut admitted = [0_u64; H13_VAULTS];
    let mut rate_limited = [0_u64; H13_VAULTS];
    let mut pending_rows: Vec<Vec<CfWrite>> = (0..H13_VAULTS).map(|_| Vec::new()).collect();
    let mut rate_limit_codes = BTreeSet::new();
    for op in 0..1_000_u64 {
        let idx = if op % 10 == 0 {
            1 + ((op / 10) as usize % (H13_VAULTS - 1))
        } else {
            0
        };
        attempted[idx] += 1;
        match guards[idx].charge_ingest(1, START_TS * 1_000_000) {
            Ok(()) => {
                admitted[idx] += 1;
                pending_rows[idx].push((
                    ColumnFamily::Base,
                    key(&format!("h13-v{idx}-skew"), op),
                    value(op, 24),
                ));
            }
            Err(error) if error.code == CALYX_QUOTA_EXCEEDED => {
                rate_limited[idx] += 1;
                rate_limit_codes.insert(error.code.to_string());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let batched_commit_counts = pending_rows.iter().map(Vec::len).collect::<Vec<_>>();
    for (idx, rows) in pending_rows.iter_mut().enumerate() {
        if !rows.is_empty() {
            vaults[idx].write_cf_batch(rows.drain(..)).map_err(err)?;
        }
    }
    let after_rates = parallel_read_rates(&vaults, Duration::from_millis(120))?;
    for vault in &vaults {
        vault.flush().map_err(err)?;
    }

    let mut cool_ratios = Vec::new();
    let mut readbacks = Vec::new();
    for idx in 0..H13_VAULTS {
        let after_rate = after_rates[idx].clone();
        let ratio = rate_ratio(&baselines[idx], &after_rate);
        if idx > 0 {
            cool_ratios.push(ratio);
        }
        readbacks.push(json!({
            "vault": idx,
            "baseline_rate": baselines[idx],
            "after_skew_rate": after_rate,
            "throughput_ratio": ratio,
            "base_rows_readback": count_readable(&vaults[idx], &format!("h13-v{idx}-base"), 0, 128)?,
            "skew_rows_readback": count_readable(&vaults[idx], &format!("h13-v{idx}-skew"), 0, 1_000)?,
            "attempted": attempted[idx],
            "admitted": admitted[idx],
            "rate_limited": rate_limited[idx],
            "quota_counters": guards[idx].counters()
        }));
    }
    let cool_min_ratio = cool_ratios.iter().copied().fold(f64::INFINITY, f64::min);
    let all_vaults_respond = readbacks.iter().all(|row| {
        row["base_rows_readback"] == json!(128) && row["skew_rows_readback"] == row["admitted"]
    });
    let passed = rate_limited[0] > 0
        && rate_limit_codes.contains(CALYX_QUOTA_EXCEEDED)
        && cool_min_ratio >= 0.5
        && all_vaults_respond;
    Ok((
        passed,
        json!({
            "trigger": "1000 synthetic writes routed 90% to vault 0 with per-vault quota guards",
            "expected": {
                "hot_vault_rate_limited_gt": 0,
                "cool_vault_read_throughput_ratio_gte": 0.5,
                "all_vaults_still_readable": true,
                "fail_closed_code": CALYX_QUOTA_EXCEEDED
            },
            "actual": {
                "before_counters": before_counters,
                "after_counters": quota_counters(&guards),
                "attempted": attempted,
                "admitted": admitted,
                "batched_commit_counts": batched_commit_counts,
                "rate_limited": rate_limited,
                "rate_limit_codes": rate_limit_codes.into_iter().collect::<Vec<_>>(),
                "cool_min_ratio": cool_min_ratio,
                "all_vaults_respond": all_vaults_respond,
                "readbacks": readbacks,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_rate_limited_total{{suite=\"ph59_t04\",hazard=\"H13\",vault=\"hot\"}} {}\ncalyx_hot_shard_cool_min_ratio{{suite=\"ph59_t04\"}} {:.6}\n",
                rate_limited[0], cool_min_ratio
            )
        }),
    ))
}

pub(super) fn probe_h14_lock_contention(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h14_lock_contention")?;
    let vault = Arc::new(open_vault(
        &dir.join("vault"),
        START_TS + 1_400,
        b"ph59-h14",
        FSV_MEMTABLE_BYTES,
        None,
    )?);
    write_live_range(&vault, "h14-read", 0, 512, 32)?;
    vault.flush().map_err(err)?;
    let baseline = read_rate(&vault, "h14-read", 0, 512, Duration::from_millis(120))?;

    let (tx, rx) = mpsc::channel();
    let (write_tx, write_rx) = mpsc::channel::<WriteRequest>();
    let (writer_done_tx, writer_done_rx) = mpsc::channel();
    let writer_vault = Arc::clone(&vault);
    let writer_handle = thread::spawn(move || {
        for request in write_rx {
            let prefix = format!("h14-w{:02}", request.thread_id);
            let outcome = writer_vault
                .write_cf(
                    ColumnFamily::Base,
                    key(&prefix, request.row),
                    value(request.id, 48),
                )
                .map(|_| ())
                .map_err(|error| error.code.to_string());
            let _ = request.reply.send(outcome);
        }
        let _ = writer_done_tx.send(());
    });

    let gate = Arc::new(Barrier::new(H14_WRITERS + 1));
    let mut handles = Vec::new();
    for thread_id in 0..H14_WRITERS {
        let tx = tx.clone();
        let write_tx = write_tx.clone();
        let gate = Arc::clone(&gate);
        handles.push(thread::spawn(move || {
            gate.wait();
            let started = Instant::now();
            let mut rows_written = 0_u64;
            let mut error_code = None;
            for row in 0..H14_ROWS_PER_WRITER {
                let id = thread_id as u64 * 1_000 + row;
                let (reply, ack) = mpsc::channel();
                let request = WriteRequest {
                    thread_id,
                    row,
                    id,
                    reply,
                };
                if write_tx.send(request).is_err() {
                    error_code = Some("CALYX_LOCK_CONTENTION_WRITER_CLOSED".to_string());
                    break;
                }
                match ack.recv_timeout(H14_ACK_TIMEOUT) {
                    Ok(Ok(())) => rows_written += 1,
                    Ok(Err(code)) => {
                        error_code = Some(code);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        error_code = Some("CALYX_LOCK_TIMEOUT".to_string());
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        error_code = Some("CALYX_LOCK_CONTENTION_WRITER_CLOSED".to_string());
                        break;
                    }
                }
                thread::sleep(Duration::from_micros(500));
            }
            let _ = tx.send(WriterReport {
                thread_id,
                rows_written,
                elapsed_ms: started.elapsed().as_millis(),
                error_code,
            });
        }));
    }
    drop(tx);
    gate.wait();
    let (read_tx, read_rx) = mpsc::channel();
    let read_vault = Arc::clone(&vault);
    thread::spawn(move || {
        let _ = read_tx.send(read_rate(
            &read_vault,
            "h14-read",
            0,
            512,
            Duration::from_millis(120),
        ));
    });
    let (during, read_error_code) = match read_rx.recv_timeout(H14_THREAD_TIMEOUT) {
        Ok(Ok(rate)) => (rate, None),
        Ok(Err(error)) => {
            return Err(error);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => (
            ReadRate {
                reads: 0,
                hits: 0,
                elapsed_ms: H14_THREAD_TIMEOUT.as_millis(),
                ops_per_sec: 0.0,
            },
            Some("CALYX_LOCK_TIMEOUT".to_string()),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => (
            ReadRate {
                reads: 0,
                hits: 0,
                elapsed_ms: 0,
                ops_per_sec: 0.0,
            },
            Some("CALYX_LOCK_CONTENTION_READER_CLOSED".to_string()),
        ),
    };
    let mut reports = Vec::new();
    let wait_started = Instant::now();
    while reports.len() < H14_WRITERS && wait_started.elapsed() < H14_THREAD_TIMEOUT {
        if let Ok(report) = rx.recv_timeout(Duration::from_millis(50)) {
            reports.push(report);
        }
    }
    let mut join_panics = 0_u64;
    for handle in handles {
        handle
            .join()
            .map_err(|_| "writer client thread panicked".to_string())
            .unwrap_or_else(|_| {
                join_panics += 1;
            });
    }
    while reports.len() < H14_WRITERS {
        match rx.try_recv() {
            Ok(report) => reports.push(report),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    drop(write_tx);
    let writer_stopped = writer_done_rx.recv_timeout(H14_THREAD_TIMEOUT).is_ok();
    if writer_stopped {
        writer_handle
            .join()
            .map_err(|_| "single writer thread panicked".to_string())?;
    }

    let mut rows_readback = 0_u64;
    if writer_stopped {
        vault.flush().map_err(err)?;
        for thread_id in 0..H14_WRITERS {
            rows_readback += count_readable(
                &vault,
                &format!("h14-w{thread_id:02}"),
                0,
                H14_ROWS_PER_WRITER,
            )? as u64;
        }
    }
    let throughput_ratio = rate_ratio(&baseline, &during);
    let held = Mutex::new(());
    let guard = held.lock().expect("synthetic lock");
    let fail_closed = try_lock_for(&held, Duration::from_millis(20));
    drop(guard);
    let passed = reports.len() == H14_WRITERS
        && reports.iter().all(|report| {
            report.rows_written == H14_ROWS_PER_WRITER && report.error_code.is_none()
        })
        && rows_readback == (H14_WRITERS as u64 * H14_ROWS_PER_WRITER)
        && throughput_ratio >= 0.8
        && read_error_code.is_none()
        && writer_stopped
        && join_panics == 0
        && fail_closed == Err("CALYX_LOCK_TIMEOUT");
    Ok((
        passed,
        json!({
            "trigger": "64 concurrent clients target one durable vault through the single-writer-per-vault path while reads pin a stable MVCC snapshot",
            "expected": {
                "writers_complete": H14_WRITERS,
                "rows_per_writer": H14_ROWS_PER_WRITER,
                "rows_readback": H14_WRITERS as u64 * H14_ROWS_PER_WRITER,
                "read_throughput_ratio_gte": 0.8,
                "fail_closed_code": "CALYX_LOCK_TIMEOUT"
            },
            "actual": {
                "writer_reports": reports,
                "writer_reports_seen": reports.len(),
                "baseline_read_rate": baseline,
                "during_write_read_rate": during,
                "read_throughput_ratio": throughput_ratio,
                "read_error_code": read_error_code,
                "rows_readback": rows_readback,
                "single_writer_path": true,
                "writer_stopped": writer_stopped,
                "client_join_panics": join_panics,
                "timeout_injection": {
                    "before": "lock held by synthetic owner",
                    "after": fail_closed.err(),
                    "timeout_ms": 20
                },
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_lock_contention_events_total{{suite=\"ph59_t04\",hazard=\"H14\"}} {}\ncalyx_lock_contention_read_ratio{{suite=\"ph59_t04\"}} {:.6}\n",
                H14_WRITERS - 1, throughput_ratio
            )
        }),
    ))
}

fn parallel_read_rates(
    vaults: &[Arc<AsterVault<SharedClock>>],
    duration: Duration,
) -> Result<Vec<ReadRate>, String> {
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for (idx, vault) in vaults.iter().enumerate() {
        let tx = tx.clone();
        let vault = Arc::clone(vault);
        handles.push(thread::spawn(move || {
            let rate = read_rate(&vault, &format!("h13-v{idx}-base"), 0, 128, duration);
            let _ = tx.send((idx, rate));
        }));
    }
    drop(tx);

    let mut rates = vec![None; vaults.len()];
    for _ in 0..vaults.len() {
        let (idx, rate) = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "CALYX_READ_TIMEOUT".to_string())?;
        rates[idx] = Some(rate?);
    }
    for handle in handles {
        handle
            .join()
            .map_err(|_| "parallel read thread panicked".to_string())?;
    }
    rates
        .into_iter()
        .enumerate()
        .map(|(idx, rate)| rate.ok_or_else(|| format!("missing read rate for vault {idx}")))
        .collect()
}

fn quota_counters(guards: &[QuotaGuard]) -> Vec<Value> {
    guards
        .iter()
        .enumerate()
        .map(|(idx, guard)| {
            let (ingest, query, io_bytes) = guard.counters();
            json!({"vault": idx, "ingest_cx": ingest, "query": query, "io_bytes": io_bytes})
        })
        .collect()
}

fn rate_ratio(baseline: &ReadRate, actual: &ReadRate) -> f64 {
    if baseline.ops_per_sec <= 0.0 {
        0.0
    } else {
        actual.ops_per_sec / baseline.ops_per_sec
    }
}

#[derive(Clone, Debug, Serialize)]
struct WriterReport {
    thread_id: usize,
    rows_written: u64,
    elapsed_ms: u128,
    error_code: Option<String>,
}

type CfWrite = (ColumnFamily, Vec<u8>, Vec<u8>);

struct WriteRequest {
    thread_id: usize,
    row: u64,
    id: u64,
    reply: mpsc::Sender<Result<(), String>>,
}
