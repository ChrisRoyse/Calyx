use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

/// Schema tag for the resident-service discovery file. Bump when the shape
/// changes so stale readers fail closed instead of misinterpreting fields.
pub(crate) const RESIDENT_DISCOVERY_SCHEMA: &str = "calyx-panel-resident-discovery-v1";

/// Well-known discovery record written by `calyx panel resident serve` under
/// `<CALYX_HOME>/resident/discovery.json`. Consumers (ingest route resolution)
/// treat every anomaly as "no discovered route" and record the reason; the
/// fail-closed enforcement happens at the GPU measurement gate, so a corrupt
/// or stale file never silently degrades a CPU-only ingest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ResidentDiscovery {
    pub(crate) schema: String,
    pub(crate) bind: SocketAddr,
    pub(crate) process_id: u32,
    /// Canonicalized vault path when the service warmed from `--vault`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) vault: Option<PathBuf>,
    /// Template selector when the service warmed from `--template`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) template: Option<String>,
    pub(crate) written_at_unix_ms: u64,
}

pub(crate) fn resident_discovery_path(home: &Path) -> PathBuf {
    home.join("resident").join("discovery.json")
}

pub(crate) fn write_resident_discovery(
    home: &Path,
    discovery: &ResidentDiscovery,
) -> CliResult<PathBuf> {
    let path = resident_discovery_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CliError::io(format!(
                "create resident discovery dir {}: {error}",
                parent.display()
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(discovery)
        .map_err(|error| CliError::runtime(format!("serialize resident discovery: {error}")))?;
    std::fs::write(&path, bytes).map_err(|error| {
        CliError::io(format!(
            "write resident discovery file {}: {error}",
            path.display()
        ))
    })?;
    Ok(path)
}

/// Read the discovery file. `Ok(None)` means "not discoverable" (missing file);
/// parse/schema anomalies also resolve to `Ok(None)` with the reason returned so
/// the caller can surface it at the GPU gate — see the struct-level contract.
pub(crate) fn read_resident_discovery(
    home: &Path,
) -> CliResult<Result<ResidentDiscovery, &'static str>> {
    let path = resident_discovery_path(home);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Err("no_discovery_file"));
        }
        Err(error) => {
            return Err(CliError::io(format!(
                "read resident discovery file {}: {error}",
                path.display()
            )));
        }
    };
    let Ok(discovery) = serde_json::from_slice::<ResidentDiscovery>(&bytes) else {
        return Ok(Err("discovery_file_unparseable"));
    };
    if discovery.schema != RESIDENT_DISCOVERY_SCHEMA {
        return Ok(Err("discovery_schema_mismatch"));
    }
    if !discovery.bind.ip().is_loopback() {
        return Ok(Err("discovery_addr_not_loopback"));
    }
    Ok(Ok(discovery))
}

pub(crate) fn remove_resident_discovery(home: &Path, process_id: u32) -> CliResult<()> {
    let path = resident_discovery_path(home);
    // Only remove a record this process wrote; a newer service instance may
    // have already replaced it.
    match read_resident_discovery(home)? {
        Ok(discovery) if discovery.process_id == process_id => {
            std::fs::remove_file(&path).map_err(|error| {
                CliError::io(format!(
                    "remove resident discovery file {}: {error}",
                    path.display()
                ))
            })
        }
        _ => Ok(()),
    }
}

pub(crate) fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "calyx-resident-discovery-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp home");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample(dir: &Path) -> ResidentDiscovery {
        ResidentDiscovery {
            schema: RESIDENT_DISCOVERY_SCHEMA.to_string(),
            bind: "127.0.0.1:8787".parse().unwrap(),
            process_id: std::process::id(),
            vault: Some(dir.join("vaults").join("01TEST")),
            template: None,
            written_at_unix_ms: unix_now_ms(),
        }
    }

    #[test]
    fn discovery_roundtrip_and_owned_removal() {
        let dir = TempHome::new("roundtrip");
        let record = sample(dir.path());
        let path = write_resident_discovery(dir.path(), &record).unwrap();
        assert!(path.exists());
        let read = read_resident_discovery(dir.path()).unwrap().unwrap();
        assert_eq!(read.bind, record.bind);
        assert_eq!(read.process_id, record.process_id);
        assert_eq!(read.vault, record.vault);
        // A foreign pid must not remove the record.
        remove_resident_discovery(dir.path(), record.process_id + 1).unwrap();
        assert!(path.exists());
        remove_resident_discovery(dir.path(), record.process_id).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn discovery_missing_file_is_not_an_error() {
        let dir = TempHome::new("missing");
        assert_eq!(
            read_resident_discovery(dir.path()).unwrap().unwrap_err(),
            "no_discovery_file"
        );
    }

    #[test]
    fn discovery_rejects_wrong_schema_and_non_loopback() {
        let dir = TempHome::new("reject");
        let mut record = sample(dir.path());
        record.schema = "other-schema".to_string();
        write_resident_discovery(dir.path(), &record).unwrap();
        assert_eq!(
            read_resident_discovery(dir.path()).unwrap().unwrap_err(),
            "discovery_schema_mismatch"
        );
        record.schema = RESIDENT_DISCOVERY_SCHEMA.to_string();
        record.bind = "10.0.0.5:8787".parse().unwrap();
        write_resident_discovery(dir.path(), &record).unwrap();
        assert_eq!(
            read_resident_discovery(dir.path()).unwrap().unwrap_err(),
            "discovery_addr_not_loopback"
        );
        std::fs::write(resident_discovery_path(dir.path()), b"{not json").unwrap();
        assert_eq!(
            read_resident_discovery(dir.path()).unwrap().unwrap_err(),
            "discovery_file_unparseable"
        );
    }
}
