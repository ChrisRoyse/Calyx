//! ZFS integrity metric collector for calyxd (issue #729).
//!
//! The collector reads the same source-of-truth surfaces as a local ZFS
//! integrity check: `zfs get checksum`,
//! `zpool status -x`, and the `zpool status -v` scan/error rows. Unknown or
//! unreadable state is recorded fail-closed by the metric handles initialized at
//! registration time.

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use prometheus::core::Collector;
use prometheus::{IntGaugeVec, Opts, Registry};

pub const DEFAULT_ZFS_DATASETS: [&str; 3] =
    ["hotpool/calyx", "archive/calyx", "archive/calyx-restic"];
pub const ZFS_SCRUB_MAX_AGE_SECONDS: i64 = 40 * 24 * 60 * 60;
const UNKNOWN_SCRUB_AGE_SECONDS: i64 = ZFS_SCRUB_MAX_AGE_SECONDS + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsDatasetChecksum {
    pub dataset: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsPoolIntegrity {
    pub pool: String,
    pub healthy: bool,
    pub cksum_errors: u64,
    pub scrub_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZfsIntegritySnapshot {
    pub datasets: Vec<ZfsDatasetChecksum>,
    pub pools: Vec<ZfsPoolIntegrity>,
}

pub struct ZfsIntegrityMetrics {
    pool_healthy: IntGaugeVec,
    cksum_errors_total: IntGaugeVec,
    scrub_age_seconds: IntGaugeVec,
    dataset_checksum_enabled: IntGaugeVec,
}

impl ZfsIntegrityMetrics {
    pub fn register(registry: &Registry, datasets: &[&str]) -> Self {
        let pool_healthy = register(
            registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_zfs_pool_healthy",
                    "1 when zpool status -x reports the pool is healthy, 0 otherwise",
                ),
                &["pool"],
            )
            .expect("define calyx_zfs_pool_healthy"),
        );
        let cksum_errors_total = register(
            registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_zfs_cksum_errors_total",
                    "Current CKSUM error count read from the pool row in zpool status -v",
                ),
                &["pool"],
            )
            .expect("define calyx_zfs_cksum_errors_total"),
        );
        let scrub_age_seconds = register(
            registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_zfs_scrub_age_seconds",
                    "Age in seconds of the last completed or currently-running scrub",
                ),
                &["pool"],
            )
            .expect("define calyx_zfs_scrub_age_seconds"),
        );
        let dataset_checksum_enabled = register(
            registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_zfs_dataset_checksum_enabled",
                    "1 when zfs get checksum for the dataset is not off, 0 otherwise",
                ),
                &["dataset"],
            )
            .expect("define calyx_zfs_dataset_checksum_enabled"),
        );
        let metrics = Self {
            pool_healthy,
            cksum_errors_total,
            scrub_age_seconds,
            dataset_checksum_enabled,
        };
        metrics.preinitialize(datasets);
        metrics
    }

    pub fn record(&self, snapshot: &ZfsIntegritySnapshot) {
        for dataset in &snapshot.datasets {
            self.dataset_checksum_enabled
                .with_label_values(&[&dataset.dataset])
                .set(i64::from(dataset.enabled));
        }
        for pool in &snapshot.pools {
            self.pool_healthy
                .with_label_values(&[&pool.pool])
                .set(i64::from(pool.healthy));
            self.cksum_errors_total
                .with_label_values(&[&pool.pool])
                .set(i64::try_from(pool.cksum_errors).unwrap_or(i64::MAX));
            self.scrub_age_seconds.with_label_values(&[&pool.pool]).set(
                pool.scrub_age_seconds
                    .and_then(|age| i64::try_from(age).ok())
                    .unwrap_or(UNKNOWN_SCRUB_AGE_SECONDS),
            );
        }
    }

    fn preinitialize(&self, datasets: &[&str]) {
        for dataset in datasets {
            self.dataset_checksum_enabled
                .with_label_values(&[dataset])
                .set(0);
        }
        for pool in pools_from_datasets(datasets) {
            self.pool_healthy.with_label_values(&[&pool]).set(0);
            self.cksum_errors_total.with_label_values(&[&pool]).set(0);
            self.scrub_age_seconds
                .with_label_values(&[&pool])
                .set(UNKNOWN_SCRUB_AGE_SECONDS);
        }
    }
}

pub fn collect_default_zfs_integrity() -> Result<ZfsIntegritySnapshot, String> {
    collect_zfs_integrity(&DEFAULT_ZFS_DATASETS, unix_now_secs())
}

pub fn collect_zfs_integrity(
    datasets: &[&str],
    now_secs: i64,
) -> Result<ZfsIntegritySnapshot, String> {
    let mut dataset_snapshots = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        let output = run_command("zfs", &["get", "-H", "-o", "value", "checksum", dataset])?;
        let enabled = output.success && checksum_enabled(&output.stdout);
        dataset_snapshots.push(ZfsDatasetChecksum {
            dataset: (*dataset).to_string(),
            enabled,
        });
    }

    let mut pools = Vec::new();
    for pool in pools_from_datasets(datasets) {
        let status_x = run_command("zpool", &["status", "-x", &pool])?;
        let status_x_text = status_x.combined_output();
        let healthy = status_x.success && status_x_text.contains("is healthy");
        let status_v = run_command("zpool", &["status", "-v", &pool])?;
        let status_v_text = status_v.combined_output();
        let cksum_errors = parse_cksum_errors(&status_v_text, &pool).unwrap_or(0);
        let scrub_age_seconds = parse_scan_line(&status_v_text)
            .and_then(parse_scrub_date)
            .and_then(|scrub_date| date_to_epoch(scrub_date).ok())
            .map(|scrub_epoch| scrub_age_seconds(now_secs, scrub_epoch));
        pools.push(ZfsPoolIntegrity {
            pool,
            healthy,
            cksum_errors,
            scrub_age_seconds,
        });
    }

    Ok(ZfsIntegritySnapshot {
        datasets: dataset_snapshots,
        pools,
    })
}

fn register<C: Collector + Clone + 'static>(registry: &Registry, collector: C) -> C {
    registry
        .register(Box::new(collector.clone()))
        .expect("register ZFS metric family (duplicate registration is a bug)");
    collector
}

fn pools_from_datasets(datasets: &[&str]) -> Vec<String> {
    datasets
        .iter()
        .filter_map(|dataset| dataset.split('/').next())
        .filter(|pool| !pool.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn checksum_enabled(value: &str) -> bool {
    let checksum = value.trim();
    !checksum.is_empty() && checksum != "off"
}

fn parse_scan_line(status: &str) -> Option<&str> {
    status
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("scan:"))
}

fn parse_scrub_date(scan_line: &str) -> Option<&str> {
    let line = scan_line.trim();
    if line.contains("none requested") || !line.contains("scrub") {
        return None;
    }
    line.rsplit_once(" on ")
        .or_else(|| line.rsplit_once(" since "))
        .map(|(_, date)| date.trim())
        .filter(|date| !date.is_empty())
}

fn parse_cksum_errors(status: &str, pool: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.first().copied() == Some(pool) && columns.len() >= 5 {
            parse_zpool_count(columns[4])
        } else {
            None
        }
    })
}

fn parse_zpool_count(value: &str) -> Option<u64> {
    let suffix = value.chars().last()?;
    let (number, multiplier) = match suffix {
        'K' | 'k' => (&value[..value.len() - 1], 1024.0),
        'M' | 'm' => (&value[..value.len() - 1], 1024.0 * 1024.0),
        'G' | 'g' => (&value[..value.len() - 1], 1024.0 * 1024.0 * 1024.0),
        'T' | 't' => (&value[..value.len() - 1], 1024.0 * 1024.0 * 1024.0 * 1024.0),
        _ => (value, 1.0),
    };
    let parsed = number.parse::<f64>().ok()?;
    if parsed.is_finite() && parsed >= 0.0 {
        Some((parsed * multiplier).round().min(u64::MAX as f64) as u64)
    } else {
        None
    }
}

fn date_to_epoch(date: &str) -> Result<i64, String> {
    let output = run_command("date", &["-d", date, "+%s"])?;
    if !output.success {
        return Err(format!("date -d {date:?} failed: {}", output.stderr.trim()));
    }
    output
        .stdout
        .trim()
        .parse::<i64>()
        .map_err(|error| format!("parse date epoch for {date:?}: {error}"))
}

fn scrub_age_seconds(now_secs: i64, scrub_epoch: i64) -> u64 {
    u64::try_from(now_secs.saturating_sub(scrub_epoch)).unwrap_or(0)
}

fn unix_now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

struct CommandRead {
    success: bool,
    stdout: String,
    stderr: String,
}

impl CommandRead {
    fn combined_output(&self) -> String {
        let mut text = self.stdout.clone();
        text.push_str(&self.stderr);
        text
    }
}

fn run_command(program: &str, args: &[&str]) -> Result<CommandRead, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("spawn {program} {}: {error}", args.join(" ")))?;
    Ok(CommandRead {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::TextEncoder;

    #[test]
    fn register_preinitializes_fail_closed_zfs_series() {
        let registry = Registry::new();
        let _metrics = ZfsIntegrityMetrics::register(&registry, &DEFAULT_ZFS_DATASETS);
        let text = encode(&registry);
        assert!(text.contains("calyx_zfs_pool_healthy{pool=\"archive\"} 0"));
        assert!(text.contains("calyx_zfs_pool_healthy{pool=\"hotpool\"} 0"));
        assert!(text.contains("calyx_zfs_cksum_errors_total{pool=\"hotpool\"} 0"));
        assert!(text.contains("calyx_zfs_scrub_age_seconds{pool=\"archive\"} 3456001"));
        assert!(
            text.contains("calyx_zfs_dataset_checksum_enabled{dataset=\"archive/calyx-restic\"} 0")
        );
    }

    #[test]
    fn record_updates_pool_and_dataset_gauges() {
        let registry = Registry::new();
        let metrics = ZfsIntegrityMetrics::register(&registry, &DEFAULT_ZFS_DATASETS);
        metrics.record(&ZfsIntegritySnapshot {
            datasets: vec![ZfsDatasetChecksum {
                dataset: "hotpool/calyx".to_string(),
                enabled: true,
            }],
            pools: vec![ZfsPoolIntegrity {
                pool: "hotpool".to_string(),
                healthy: true,
                cksum_errors: 2,
                scrub_age_seconds: Some(123),
            }],
        });
        let text = encode(&registry);
        assert!(text.contains("calyx_zfs_pool_healthy{pool=\"hotpool\"} 1"));
        assert!(text.contains("calyx_zfs_cksum_errors_total{pool=\"hotpool\"} 2"));
        assert!(text.contains("calyx_zfs_scrub_age_seconds{pool=\"hotpool\"} 123"));
        assert!(text.contains("calyx_zfs_dataset_checksum_enabled{dataset=\"hotpool/calyx\"} 1"));
    }

    #[test]
    fn parse_completed_and_in_progress_scrub_dates() {
        assert_eq!(
            parse_scrub_date(
                "scan: scrub repaired 0B in 00:02:18 with 0 errors on Sun Jun 14 00:26:24 2026"
            ),
            Some("Sun Jun 14 00:26:24 2026")
        );
        assert_eq!(
            parse_scrub_date("scan: scrub in progress since Sun Jun 14 00:24:01 2026"),
            Some("Sun Jun 14 00:24:01 2026")
        );
        assert_eq!(parse_scrub_date("scan: none requested"), None);
    }

    #[test]
    fn parse_pool_cksum_error_row() {
        let status = "\
            NAME        STATE     READ WRITE CKSUM\n\
            hotpool     ONLINE       0     0     7\n\
              nvme0n1   ONLINE       0     0     0\n";
        assert_eq!(parse_cksum_errors(status, "hotpool"), Some(7));
        assert_eq!(parse_cksum_errors(status, "archive"), None);
        assert_eq!(parse_zpool_count("1.5K"), Some(1536));
    }

    fn encode(registry: &Registry) -> String {
        let mut buffer = String::new();
        TextEncoder::new()
            .encode_utf8(&registry.gather(), &mut buffer)
            .unwrap();
        buffer
    }
}
