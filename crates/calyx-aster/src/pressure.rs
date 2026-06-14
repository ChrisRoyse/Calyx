//! Disk-pressure guard for fail-closed hotpool write admission.

use calyx_core::{CalyxError, Clock, Result, Ts};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

pub const CALYX_IO_ERROR: &str = "CALYX_IO_ERROR";
pub const DEFAULT_HIGH_WATER_RATIO: f64 = 0.85;

const IO_REMEDIATION: &str = "inspect hotpool path, permissions, and statvfs support; reject writes until disk state is readable";

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskSample {
    pub blocks: u64,
    pub blocks_available: u64,
}

impl DiskSample {
    pub fn used_ratio(self) -> f64 {
        if self.blocks == 0 {
            return 0.0;
        }
        let available = self.blocks_available.min(self.blocks) as f64 / self.blocks as f64;
        (1.0 - available).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiskStatus {
    Ok {
        hotpool_path: PathBuf,
        used_ratio: f64,
        blocks_available: u64,
        blocks_total: u64,
        checked_at: Ts,
    },
}

pub trait DiskSpaceProbe: Send + Sync {
    fn sample(&self, path: &Path) -> Result<DiskSample>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OsDiskSpaceProbe;

impl DiskSpaceProbe for OsDiskSpaceProbe {
    fn sample(&self, path: &Path) -> Result<DiskSample> {
        os_disk_sample(path)
    }
}

#[derive(Clone)]
pub struct DiskPressureGuard {
    pub hotpool_path: PathBuf,
    pub high_water_ratio: f64,
    pub clock: Arc<dyn Clock>,
    probe: Arc<dyn DiskSpaceProbe>,
    spill_trigger: Option<SpillTrigger>,
}

impl fmt::Debug for DiskPressureGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskPressureGuard")
            .field("hotpool_path", &self.hotpool_path)
            .field("high_water_ratio", &self.high_water_ratio)
            .field("spill_trigger", &self.spill_trigger)
            .finish_non_exhaustive()
    }
}

impl DiskPressureGuard {
    pub fn new(
        hotpool_path: impl Into<PathBuf>,
        high_water_ratio: f64,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_probe(
            hotpool_path,
            high_water_ratio,
            clock,
            Arc::new(OsDiskSpaceProbe),
        )
    }

    pub fn with_probe(
        hotpool_path: impl Into<PathBuf>,
        high_water_ratio: f64,
        clock: Arc<dyn Clock>,
        probe: Arc<dyn DiskSpaceProbe>,
    ) -> Self {
        Self {
            hotpool_path: hotpool_path.into(),
            high_water_ratio,
            clock,
            probe,
            spill_trigger: None,
        }
    }

    pub fn with_spill_trigger(mut self, spill_trigger: SpillTrigger) -> Self {
        self.spill_trigger = Some(spill_trigger);
        self
    }

    pub fn check(&self) -> Result<DiskStatus> {
        if !(0.0..=1.0).contains(&self.high_water_ratio) {
            return Err(CalyxError::disk_pressure(format!(
                "invalid high_water_ratio={:.6} for hotpool {}",
                self.high_water_ratio,
                self.hotpool_path.display()
            )));
        }
        let sample = self.probe.sample(&self.hotpool_path)?;
        let used_ratio = sample.used_ratio();
        let checked_at = self.clock.now();
        if used_ratio >= self.high_water_ratio {
            return Err(CalyxError::disk_pressure(format!(
                "hotpool {} used_ratio={used_ratio:.6} high_water_ratio={:.6} blocks_available={} blocks_total={} checked_at={checked_at}",
                self.hotpool_path.display(),
                self.high_water_ratio,
                sample.blocks_available,
                sample.blocks
            )));
        }
        Ok(DiskStatus::Ok {
            hotpool_path: self.hotpool_path.clone(),
            used_ratio,
            blocks_available: sample.blocks_available,
            blocks_total: sample.blocks,
            checked_at,
        })
    }

    pub fn is_under_pressure(&self) -> bool {
        self.check().is_err()
    }

    pub fn request_spill(&self) {
        if let Some(trigger) = &self.spill_trigger {
            trigger.request_spill();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpillRequest {
    pub hotpool_path: PathBuf,
    pub requested_at: Ts,
}

#[derive(Clone)]
pub struct SpillTrigger {
    sender: Sender<SpillRequest>,
    hotpool_path: PathBuf,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for SpillTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpillTrigger")
            .field("hotpool_path", &self.hotpool_path)
            .finish_non_exhaustive()
    }
}

impl SpillTrigger {
    pub fn new(
        hotpool_path: impl Into<PathBuf>,
        sender: Sender<SpillRequest>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            sender,
            hotpool_path: hotpool_path.into(),
            clock,
        }
    }

    pub fn request_spill(&self) {
        let request = SpillRequest {
            hotpool_path: self.hotpool_path.clone(),
            requested_at: self.clock.now(),
        };
        match self.sender.send(request) {
            Ok(()) => tracing::warn!(
                hotpool_path = %self.hotpool_path.display(),
                "disk pressure spill requested"
            ),
            Err(error) => tracing::warn!(
                hotpool_path = %self.hotpool_path.display(),
                error = %error,
                "disk pressure spill request dropped"
            ),
        }
    }
}

#[derive(Debug)]
pub struct TempFile {
    path: PathBuf,
    file: File,
    keep: bool,
}

impl TempFile {
    pub fn in_dataset(destination_dir: &Path) -> Result<Self> {
        fs::create_dir_all(destination_dir).map_err(|error| {
            io_error(format!(
                "create destination dataset {}: {error}",
                destination_dir.display()
            ))
        })?;
        for _ in 0..128 {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = destination_dir.join(format!(".calyx-tmp-{}-{id}.tmp", std::process::id()));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        keep: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error(format!(
                        "create temp file in dataset {}: {error}",
                        destination_dir.display()
                    )));
                }
            }
        }
        Err(io_error(format!(
            "create temp file in dataset {}: exhausted unique names",
            destination_dir.display()
        )))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    pub fn persist(mut self) -> Result<PathBuf> {
        self.file.sync_all().map_err(|error| {
            io_error(format!("sync temp file {}: {error}", self.path.display()))
        })?;
        self.keep = true;
        Ok(self.path.clone())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn os_disk_sample(path: &Path) -> Result<DiskSample> {
    let stat = nix::sys::statvfs::statvfs(path)
        .map_err(|error| io_error(format!("statvfs {}: {error}", path.display())))?;
    Ok(DiskSample {
        blocks: stat.blocks(),
        blocks_available: stat.blocks_available(),
    })
}

#[cfg(not(unix))]
fn os_disk_sample(path: &Path) -> Result<DiskSample> {
    Err(io_error(format!(
        "statvfs unsupported on this platform for {}",
        path.display()
    )))
}

fn io_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_IO_ERROR,
        message: message.into(),
        remediation: IO_REMEDIATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::FixedClock;
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Debug)]
    struct MockProbe {
        result: Result<DiskSample>,
    }

    impl DiskSpaceProbe for MockProbe {
        fn sample(&self, _path: &Path) -> Result<DiskSample> {
            self.result.clone()
        }
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(FixedClock::new(1_785_000_000))
    }

    fn guard_for(sample: DiskSample, high_water_ratio: f64) -> DiskPressureGuard {
        DiskPressureGuard::with_probe(
            "/hotpool",
            high_water_ratio,
            clock(),
            Arc::new(MockProbe { result: Ok(sample) }),
        )
    }

    #[test]
    fn ninety_percent_used_trips_pressure_with_payload() {
        let guard = guard_for(
            DiskSample {
                blocks: 100,
                blocks_available: 10,
            },
            DEFAULT_HIGH_WATER_RATIO,
        );

        assert!(guard.is_under_pressure());
        let error = guard.check().unwrap_err();
        assert_eq!(error.code, "CALYX_DISK_PRESSURE");
        assert!(error.message.contains("used_ratio=0.900000"));
        assert!(error.message.contains("high_water_ratio=0.850000"));
    }

    #[test]
    fn eighty_percent_used_is_accepted() {
        let guard = guard_for(
            DiskSample {
                blocks: 100,
                blocks_available: 20,
            },
            DEFAULT_HIGH_WATER_RATIO,
        );

        assert!(!guard.is_under_pressure());
        let DiskStatus::Ok { used_ratio, .. } = guard.check().unwrap();
        assert!((used_ratio - 0.80).abs() < f64::EPSILON);
    }

    #[test]
    fn high_water_boundary_is_inclusive() {
        let guard = guard_for(
            DiskSample {
                blocks: 100,
                blocks_available: 15,
            },
            DEFAULT_HIGH_WATER_RATIO,
        );

        assert!(guard.is_under_pressure());
        assert_eq!(guard.check().unwrap_err().code, "CALYX_DISK_PRESSURE");
    }

    #[test]
    fn temp_file_stays_inside_destination_dataset() {
        let dir = std::env::temp_dir().join(format!("calyx-temp-dataset-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let temp = TempFile::in_dataset(&dir).unwrap();
        let path = temp.path().to_path_buf();
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert!(path.exists());
        drop(temp);
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spill_trigger_sends_nonblocking_request() {
        let (sender, receiver) = mpsc::channel();
        let trigger = SpillTrigger::new("/hotpool", sender, clock());

        trigger.request_spill();

        let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(request.hotpool_path, PathBuf::from("/hotpool"));
        assert_eq!(request.requested_at, 1_785_000_000);
    }

    #[test]
    fn statvfs_failure_returns_io_error_and_boolean_fails_closed() {
        let guard = DiskPressureGuard::with_probe(
            "/missing",
            DEFAULT_HIGH_WATER_RATIO,
            clock(),
            Arc::new(MockProbe {
                result: Err(io_error("statvfs /missing: missing")),
            }),
        );

        let error = guard.check().unwrap_err();
        assert_eq!(error.code, CALYX_IO_ERROR);
        assert!(guard.is_under_pressure());
    }

    #[test]
    fn zero_blocks_treats_filesystem_as_empty() {
        let guard = guard_for(
            DiskSample {
                blocks: 0,
                blocks_available: 0,
            },
            DEFAULT_HIGH_WATER_RATIO,
        );

        let DiskStatus::Ok { used_ratio, .. } = guard.check().unwrap();
        assert_eq!(used_ratio, 0.0);
    }
}
