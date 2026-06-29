use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{ProbeMatrixArtifact, accepted_hit_count, hex_lower, refusal_count};
use crate::error::{CliError, CliResult};

pub(super) struct PersistedProbeMatrix {
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) sha256: String,
    pub(super) readback_record_count: usize,
    pub(super) readback_productive_count: usize,
    pub(super) readback_accepted_hit_count: usize,
    pub(super) readback_refusal_count: usize,
}

pub(super) fn persist_probe_matrix(
    vault_dir: &Path,
    explicit: Option<&Path>,
    artifact: &ProbeMatrixArtifact,
) -> CliResult<PersistedProbeMatrix> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    let matrix_id = blake3::hash(&bytes).to_hex().to_string();
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        vault_dir
            .join("idx")
            .join("probe_matrix")
            .join(matrix_id)
            .join("matrix.json")
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different probe matrix {}",
                path.display()
            )));
        }
    } else {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &path)?;
    }
    let readback = fs::read(&path)?;
    if readback != bytes {
        return Err(CliError::usage(format!(
            "probe matrix readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: ProbeMatrixArtifact = serde_json::from_slice(&readback)?;
    Ok(PersistedProbeMatrix {
        path,
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_record_count: decoded.log.records.len(),
        readback_productive_count: decoded.log.productive.len(),
        readback_accepted_hit_count: accepted_hit_count(&decoded.log),
        readback_refusal_count: refusal_count(&decoded.log),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}
