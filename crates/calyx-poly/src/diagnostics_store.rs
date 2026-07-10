//! On-disk persistence for panel diagnostics (#207) and association materialization plans (#208).
//!
//! The persisted JSON file is the Full-State-Verification source of truth: a diagnostic is "real"
//! only when it can be written and read back byte-for-byte from disk. Every write fails closed with
//! a structured `{code, message}` if the directory cannot be created, the value cannot be
//! serialized, or the file cannot be written; every read fails closed if the file is missing or does
//! not deserialize into the expected schema.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{PolyError, Result};

/// A diagnostics artifact directory could not be created.
pub const ERR_MKDIR: &str = "CALYX_POLY_DIAG_STORE_MKDIR_FAILED";
/// A diagnostics artifact could not be serialized.
pub const ERR_SERIALIZE: &str = "CALYX_POLY_DIAG_STORE_SERIALIZE_FAILED";
/// A diagnostics artifact could not be written.
pub const ERR_WRITE: &str = "CALYX_POLY_DIAG_STORE_WRITE_FAILED";
/// A diagnostics artifact could not be read back.
pub const ERR_READ: &str = "CALYX_POLY_DIAG_STORE_READ_FAILED";
/// A diagnostics artifact did not deserialize into the expected schema.
pub const ERR_DESERIALIZE: &str = "CALYX_POLY_DIAG_STORE_DESERIALIZE_FAILED";

/// Writes `value` as pretty JSON to `dir/{file_name}`, creating `dir` if needed. Returns the path.
pub fn write_json<T: Serialize>(dir: &Path, file_name: &str, value: &T) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|err| {
        PolyError::diagnostics(
            ERR_MKDIR,
            format!("create diagnostics dir {}: {err}", dir.display()),
        )
    })?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        PolyError::diagnostics(
            ERR_SERIALIZE,
            format!("serialize diagnostics artifact: {err}"),
        )
    })?;
    let path = dir.join(file_name);
    std::fs::write(&path, &bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_WRITE,
            format!("write diagnostics artifact {}: {err}", path.display()),
        )
    })?;
    Ok(path)
}

/// Reads and deserializes a diagnostics artifact from `path`. Fails closed if it is missing or does
/// not match the schema `T`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_READ,
            format!("read diagnostics artifact {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_DESERIALIZE,
            format!("deserialize diagnostics artifact {}: {err}", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct FloatArtifact {
        value: f64,
    }

    #[test]
    fn read_missing_fails_closed() {
        let err = read_json::<u32>(Path::new("does-not-exist-xyz.json")).unwrap_err();
        assert_eq!(err.code(), ERR_READ);
    }

    #[test]
    fn json_float_bits_round_trip_exactly() {
        let dir = TestDir::new("calyx-poly-diagnostics-float-roundtrip");
        let original = f64::from_bits(0x3fd9_803e_f746_8a5a);
        let path = write_json(&dir.0, "float.json", &FloatArtifact { value: original }).unwrap();
        let readback: FloatArtifact = read_json(&path).unwrap();

        assert_eq!(readback.value.to_bits(), original.to_bits());
    }
}
