use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use calyx_registry::LensForgeFile;
use calyx_registry::frozen::sha256_digest;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::support::hex_from_bytes;
use crate::error::{CliError, CliResult};

#[derive(Clone, Serialize)]
pub(super) struct FileReport {
    role: String,
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

pub(super) struct Artifact {
    pub(super) role: String,
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

pub(super) fn artifact(role: &str, path: PathBuf) -> CliResult<Artifact> {
    let bytes = fs::read(&path)?;
    Ok(Artifact {
        role: role.to_string(),
        path,
        sha256: plain_sha256_hex(&bytes),
        bytes: bytes.len() as u64,
    })
}

pub(super) fn artifact_set_sha256(artifacts: &[Artifact]) -> CliResult<String> {
    let mut ordered = artifacts.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|item| (role_rank(&item.role), item.path.clone()));
    let mut owned = Vec::with_capacity(ordered.len());
    for item in ordered {
        owned.push(fs::read(&item.path)?);
    }
    let parts = owned.iter().map(Vec::as_slice).collect::<Vec<_>>();
    Ok(hex_from_bytes(&sha256_digest(&parts)))
}

pub(super) fn manifest_files(base: &Path, artifacts: &[Artifact]) -> CliResult<Vec<LensForgeFile>> {
    let mut files = artifacts.iter().collect::<Vec<_>>();
    files.sort_by_key(|item| (role_rank(&item.role), item.path.clone()));
    files
        .into_iter()
        .map(|item| {
            Ok(LensForgeFile {
                role: item.role.clone(),
                path: relative_to(base, &item.path)?,
                sha256: item.sha256.clone(),
                bytes: item.bytes,
            })
        })
        .collect()
}

pub(super) fn file_report(artifact: &Artifact) -> FileReport {
    FileReport {
        role: artifact.role.clone(),
        path: artifact.path.clone(),
        sha256: artifact.sha256.clone(),
        bytes: artifact.bytes,
    }
}

pub(super) fn require_named(root: &Path, name: &str) -> CliResult<PathBuf> {
    let path = root.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(CliError::from(CalyxError::lens_unreachable(format!(
            "required artifact {} is missing",
            path.display()
        ))))
    }
}

pub(super) fn require_named_fallback(
    primary: &Path,
    fallback: &Path,
    name: &str,
) -> CliResult<PathBuf> {
    let primary_path = primary.join(name);
    if primary_path.is_file() {
        return Ok(primary_path);
    }
    require_named(fallback, name)
}

pub(super) fn add_optional(artifacts: &mut Vec<Artifact>, role: &str, path: PathBuf) -> CliResult {
    if path.is_file() {
        artifacts.push(artifact(role, path)?);
    }
    Ok(())
}

pub(super) fn find_preferred(
    root: &Path,
    preferred: &[&str],
    extension: &str,
) -> CliResult<PathBuf> {
    for name in preferred {
        let path = root.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    let mut matches = Vec::new();
    collect_by_extension(root, extension, &mut matches)?;
    if matches.len() == 1 {
        return Ok(matches.remove(0));
    }
    Err(CliError::from(CalyxError::lens_unreachable(format!(
        "expected exactly one .{extension} artifact under {}, found {}",
        root.display(),
        matches.len()
    ))))
}

pub(super) fn read_hidden_size(config: &Path) -> CliResult<u32> {
    let bytes = fs::read(config)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let raw = value
        .get("hidden_size")
        .or_else(|| value.get("dim"))
        .or_else(|| value.get("embedding_dim"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CliError::usage(format!(
                "cannot infer lens dim from {}; pass --dim",
                config.display()
            ))
        })?;
    u32::try_from(raw).map_err(|_| CliError::usage("inferred dim exceeds u32"))
}

fn role_rank(role: &str) -> u8 {
    match role {
        "model" | "weights" | "embeddings" => 0,
        "tokenizer" => 1,
        "config" => 2,
        "preprocessor" => 3,
        "tokenizer_config" => 4,
        "special_tokens_map" => 5,
        _ => 9,
    }
}

fn relative_to(base: &Path, path: &Path) -> CliResult<PathBuf> {
    path.strip_prefix(base).map(Path::to_path_buf).map_err(|_| {
        CliError::usage(format!(
            "artifact {} is not under {}",
            path.display(),
            base.display()
        ))
    })
}

fn collect_by_extension(root: &Path, extension: &str, out: &mut Vec<PathBuf>) -> CliResult {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_by_extension(&path, extension, out)?;
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn plain_sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex_from_bytes(&digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_file_hash_uses_plain_sha256_not_contract_digest() {
        let root =
            std::env::temp_dir().join(format!("calyx-cli-artifact-hash-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("artifact.bin");
        let bytes = b"manifest file hash bytes";
        fs::write(&path, bytes).unwrap();

        let report = artifact("model", path).unwrap();
        let plain = plain_sha256_hex(bytes);
        let contract = hex_from_bytes(&sha256_digest(&[bytes]));

        assert_eq!(report.sha256, plain);
        assert_ne!(report.sha256, contract);
    }
}
