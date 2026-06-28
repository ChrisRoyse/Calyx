use super::*;

pub(super) fn read_and_verify_files(
    manifest: &LensForgeManifest,
    base_dir: &Path,
) -> Result<Vec<VerifiedFile>> {
    let mut files = Vec::with_capacity(manifest.files.len());
    for file in ordered_manifest_files(&manifest.files) {
        let path = resolve_manifest_path(base_dir, &file.path);
        let actual = plain_sha256_file(&path)?;
        if !hex_eq(&actual.sha256, &file.sha256) {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lensforge artifact {} sha256 {} != manifest {}",
                path.display(),
                actual.sha256,
                file.sha256
            )));
        }
        if file.bytes != 0 && file.bytes != actual.bytes {
            return Err(config_invalid(format!(
                "lensforge artifact {} byte count {} != manifest {}",
                path.display(),
                actual.bytes,
                file.bytes
            )));
        }
        files.push(VerifiedFile {
            role: file.role.clone(),
            path,
            sha256: actual.sha256,
            bytes: actual.bytes,
        });
    }
    Ok(files)
}

pub(super) fn spec_weights_sha256(
    manifest: &LensForgeManifest,
    artifacts: &[VerifiedFile],
) -> Result<[u8; 32]> {
    if is_algorithmic_runtime(&manifest.runtime) && artifacts.is_empty() {
        return Ok(sha256_digest(&[
            b"lensforge-algorithmic-v1",
            manifest.name.as_bytes(),
            manifest.runtime.as_bytes(),
            &manifest.dim.to_be_bytes(),
            modality_token(manifest.modality).as_bytes(),
        ]));
    }
    let model = weight_anchor(manifest, artifacts)?;
    if !hex_eq(&model.sha256, &manifest.weights_sha256) {
        return Err(CalyxError::lens_frozen_violation(format!(
            "lensforge model weights sha256 {} != manifest {}",
            model.sha256, manifest.weights_sha256
        )));
    }
    if let Some(expected) = &manifest.artifact_set_sha256 {
        let contract_artifacts = contract_artifacts(manifest, artifacts)?;
        let actual = artifact_set_sha256_hex(&contract_artifacts)?;
        if !hex_eq(&actual, expected) {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lensforge artifact_set_sha256 {actual} != manifest {expected}"
            )));
        }
        parse_hex_32(expected)
    } else {
        parse_hex_32(&manifest.weights_sha256)
    }
}

fn weight_anchor<'a>(
    manifest: &LensForgeManifest,
    artifacts: &'a [VerifiedFile],
) -> Result<&'a VerifiedFile> {
    artifacts
        .iter()
        .find(|file| is_model_role(&file.role))
        .or_else(|| {
            is_adapter_runtime(&manifest.runtime)
                .then(|| artifacts.iter().find(|file| file.role == "adapter"))
                .flatten()
        })
        .ok_or_else(|| config_invalid("lensforge manifest requires a model file"))
}

pub(super) fn is_tei_runtime(runtime: &str) -> bool {
    matches!(runtime, "tei" | "tei-http" | "tei_http")
}

fn is_adapter_runtime(runtime: &str) -> bool {
    matches!(
        runtime,
        "adapter" | "multimodal-adapter" | "multimodal_adapter"
    )
}

fn ordered_manifest_files(files: &[LensForgeFile]) -> Vec<&LensForgeFile> {
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|file| (role_rank(&file.role), file.path.clone()));
    ordered
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

fn contract_artifacts<'a>(
    manifest: &LensForgeManifest,
    artifacts: &'a [VerifiedFile],
) -> Result<Vec<&'a VerifiedFile>> {
    match manifest.runtime.as_str() {
        "model2vec" | "static_lookup" | "static-lookup" => Ok(vec![
            artifact_ref_by_role(artifacts, is_model_role)?,
            artifact_ref_by_role(artifacts, |role| role == "tokenizer")?,
        ]),
        _ => Ok(artifacts.iter().collect()),
    }
}

fn artifact_ref_by_role(
    artifacts: &[VerifiedFile],
    predicate: impl Fn(&str) -> bool,
) -> Result<&VerifiedFile> {
    artifacts
        .iter()
        .find(|file| predicate(&file.role))
        .ok_or_else(|| config_invalid("lensforge manifest missing static lookup artifact"))
}

fn is_model_role(role: &str) -> bool {
    matches!(role, "model" | "weights" | "embeddings")
}

fn resolve_manifest_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

struct FileDigest {
    sha256: String,
    bytes: u64,
}

fn plain_sha256_file(path: &Path) -> Result<FileDigest> {
    let file = fs::File::open(path).map_err(|err| {
        config_invalid(format!(
            "open lensforge artifact {} for hashing failed: {err}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|err| {
        config_invalid(format!(
            "stat lensforge artifact {} for hashing failed: {err}",
            path.display()
        ))
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; STREAM_HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            config_invalid(format!(
                "read lensforge artifact {} while hashing failed: {err}",
                path.display()
            ))
        })?;
        if read == 0 {
            let digest: [u8; 32] = hasher.finalize().into();
            return Ok(FileDigest {
                sha256: hex_from_bytes(&digest),
                bytes: metadata.len(),
            });
        }
        hasher.update(&buffer[..read]);
    }
}

fn artifact_set_sha256_hex(files: &[&VerifiedFile]) -> Result<String> {
    let mut contract = LengthDelimitedSha256::new();
    let mut buffer = vec![0_u8; STREAM_HASH_BUFFER_BYTES];
    for file in files {
        hash_verified_file_into(file, &mut contract, &mut buffer)?;
    }
    Ok(hex_from_bytes(&contract.finalize()))
}

fn hash_verified_file_into(
    file: &VerifiedFile,
    contract: &mut LengthDelimitedSha256,
    buffer: &mut [u8],
) -> Result<()> {
    let handle = fs::File::open(&file.path).map_err(|err| {
        config_invalid(format!(
            "open lensforge artifact {} for artifact_set hashing failed: {err}",
            file.path.display()
        ))
    })?;
    let metadata = handle.metadata().map_err(|err| {
        config_invalid(format!(
            "stat lensforge artifact {} for artifact_set hashing failed: {err}",
            file.path.display()
        ))
    })?;
    if metadata.len() != file.bytes {
        return Err(config_invalid(format!(
            "lensforge artifact {} byte count changed from {} to {} while hashing artifact_set",
            file.path.display(),
            file.bytes,
            metadata.len()
        )));
    }
    contract.begin_part(file.bytes);
    let mut plain = Sha256::new();
    let mut reader = BufReader::new(handle);
    loop {
        let read = reader.read(buffer).map_err(|err| {
            config_invalid(format!(
                "read lensforge artifact {} while hashing artifact_set failed: {err}",
                file.path.display()
            ))
        })?;
        if read == 0 {
            let digest: [u8; 32] = plain.finalize().into();
            let actual = hex_from_bytes(&digest);
            if !hex_eq(&actual, &file.sha256) {
                return Err(CalyxError::lens_frozen_violation(format!(
                    "lensforge artifact {} sha256 changed from {} to {} while hashing artifact_set",
                    file.path.display(),
                    file.sha256,
                    actual
                )));
            }
            return Ok(());
        }
        let chunk = &buffer[..read];
        plain.update(chunk);
        contract.update_chunk(chunk);
    }
}
