//! Shared discovery-run manifest preflight for biomedical stage CLIs.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{
    DiscoveryRunManifest, LodestarError, manifest_sha256, validate_discovery_run_manifest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

pub(crate) const RUN_MANIFEST_FLAG: &str = "--run-manifest";
pub(crate) const RUN_STAGE_ID_FLAG: &str = "--run-stage-id";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryRunPreflightArgs {
    pub manifest: Option<PathBuf>,
    pub stage_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiscoveryRunPreflightReadback {
    pub manifest: PathBuf,
    pub manifest_sha256: String,
    pub stage_id: String,
    pub upstream_stage_id: Option<String>,
    pub expected_input_sha256: String,
    pub observed_input_sha256: String,
}

pub(crate) struct PreflightInput<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
}

impl<'a> PreflightInput<'a> {
    pub(crate) fn new(path: &'a Path, bytes: &'a [u8]) -> Self {
        Self { path, bytes }
    }
}

impl DiscoveryRunPreflightArgs {
    pub(crate) fn validate_for_command(&self, command: &str) -> CliResult {
        match (&self.manifest, &self.stage_id) {
            (None, None) | (Some(_), Some(_)) => Ok(()),
            (Some(_), None) => Err(CliError::usage(format!(
                "{command} requires {RUN_STAGE_ID_FLAG} when {RUN_MANIFEST_FLAG} is set"
            ))),
            (None, Some(_)) => Err(CliError::usage(format!(
                "{command} requires {RUN_MANIFEST_FLAG} when {RUN_STAGE_ID_FLAG} is set"
            ))),
        }
    }
}

pub(crate) fn preflight_input_bytes(
    preflight: &DiscoveryRunPreflightArgs,
    input_bytes: &[u8],
) -> CliResult<Option<DiscoveryRunPreflightReadback>> {
    preflight_input_sha256(preflight, sha256_hex(input_bytes))
}

pub(crate) fn preflight_input_files(
    preflight: &DiscoveryRunPreflightArgs,
    inputs: &[PreflightInput<'_>],
) -> CliResult<Option<DiscoveryRunPreflightReadback>> {
    let observed = match inputs {
        [] => {
            return Err(CliError::usage(
                "discovery-run preflight requires at least one input",
            ));
        }
        [single] => sha256_hex(single.bytes),
        many => combined_input_sha256(many),
    };
    preflight_input_sha256(preflight, observed)
}

pub(crate) fn preflight_input_sha256(
    preflight: &DiscoveryRunPreflightArgs,
    observed_input_sha256: String,
) -> CliResult<Option<DiscoveryRunPreflightReadback>> {
    preflight.validate_for_command("discovery-run preflight")?;
    let Some(manifest_path) = preflight.manifest.as_ref() else {
        return Ok(None);
    };
    let stage_id = preflight
        .stage_id
        .as_ref()
        .expect("validated preflight stage id")
        .clone();
    let manifest_bytes = fs::read(manifest_path).map_err(|error| {
        CliError::io(format!(
            "read {RUN_MANIFEST_FLAG} {}: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest: DiscoveryRunManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            CliError::runtime(format!(
                "parse {RUN_MANIFEST_FLAG} {}: {error}",
                manifest_path.display()
            ))
        })?;
    validate_discovery_run_manifest(&manifest)?;
    let stage = manifest
        .stages
        .iter()
        .find(|stage| stage.stage_id == stage_id)
        .ok_or_else(|| LodestarError::DiscoveryRunManifestMissingUpstream {
            stage: stage_id.clone(),
            upstream: stage_id.clone(),
        })?;
    if stage.input_sha256 != observed_input_sha256 {
        return Err(LodestarError::DiscoveryRunManifestChainBroken {
            stage: stage.stage_id.clone(),
            expected: stage.input_sha256.clone(),
            found: observed_input_sha256,
        }
        .into());
    }
    Ok(Some(DiscoveryRunPreflightReadback {
        manifest: manifest_path.clone(),
        manifest_sha256: manifest_sha256(&manifest)?,
        stage_id: stage.stage_id.clone(),
        upstream_stage_id: stage.upstream_stage_id.clone(),
        expected_input_sha256: stage.input_sha256.clone(),
        observed_input_sha256: stage.input_sha256.clone(),
    }))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn combined_input_sha256(inputs: &[PreflightInput<'_>]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"calyx-discovery-run-preflight-inputs-v1\0");
    for input in inputs {
        hasher.update(input.path.display().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(sha256_hex(input.bytes).as_bytes());
        hasher.update([0]);
    }
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use calyx_lodestar::{DiscoveryRunManifest, DiscoveryRunStage};
    use serde_json::json;

    use super::*;

    #[test]
    fn matching_stage_input_returns_readback() {
        let root = temp_root("match");
        let input = b"rank-input";
        let input_sha = sha256_hex(input);
        let manifest_path = root.join("manifest.json");
        write_manifest(&manifest_path, &manifest(&input_sha));

        let readback = preflight_input_bytes(&preflight(&manifest_path, "hypothesis-rank"), input)
            .unwrap()
            .unwrap();

        assert_eq!(readback.stage_id, "hypothesis-rank");
        assert_eq!(readback.expected_input_sha256, input_sha);
        assert_eq!(readback.observed_input_sha256, input_sha);
        cleanup(root);
    }

    #[test]
    fn stale_stage_input_fails_chain_broken() {
        let root = temp_root("stale");
        let manifest_path = root.join("manifest.json");
        write_manifest(&manifest_path, &manifest(&sha256_hex(b"fresh-input")));

        let err = preflight_input_bytes(&preflight(&manifest_path, "hypothesis-rank"), b"stale")
            .unwrap_err();

        assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN");
        cleanup(root);
    }

    #[test]
    fn missing_upstream_fails_before_stage_work() {
        let root = temp_root("missing-upstream");
        let manifest_path = root.join("manifest.json");
        let input_sha = sha256_hex(b"rank-input");
        let mut manifest = manifest(&input_sha);
        manifest.stages.remove(0);
        write_manifest(&manifest_path, &manifest);

        let err =
            preflight_input_bytes(&preflight(&manifest_path, "hypothesis-rank"), b"rank-input")
                .unwrap_err();

        assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_MISSING_UPSTREAM");
        cleanup(root);
    }

    #[test]
    fn partial_preflight_flags_are_usage_errors() {
        let args = DiscoveryRunPreflightArgs {
            manifest: Some("manifest.json".into()),
            stage_id: None,
        };

        let err = args.validate_for_command("hypothesis-rank").unwrap_err();

        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains(RUN_STAGE_ID_FLAG));
    }

    #[test]
    fn writes_fsv_readback_when_root_is_set() {
        let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
            return;
        };
        let input = b"rank-input";
        let input_sha = sha256_hex(input);
        let manifest_path = root.join("preflight_manifest.json");
        write_manifest(&manifest_path, &manifest(&input_sha));
        let readback = preflight_input_bytes(&preflight(&manifest_path, "hypothesis-rank"), input)
            .unwrap()
            .unwrap();
        let summary = json!({
            "issue": 1218,
            "stage_id": readback.stage_id,
            "expected_input_sha256": readback.expected_input_sha256,
            "observed_input_sha256": readback.observed_input_sha256,
            "manifest_sha256": readback.manifest_sha256,
        });
        let summary_path = root.join("issue1218_discovery_run_preflight_readback.json");
        fs::write(&summary_path, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(stored["expected_input_sha256"], input_sha);
        assert_eq!(stored["observed_input_sha256"], input_sha);
        println!(
            "issue1218_preflight_fsv_path={} bytes={}",
            summary_path.display(),
            fs::metadata(&summary_path).unwrap().len()
        );
    }

    fn preflight(manifest: &Path, stage_id: &str) -> DiscoveryRunPreflightArgs {
        DiscoveryRunPreflightArgs {
            manifest: Some(manifest.to_path_buf()),
            stage_id: Some(stage_id.to_string()),
        }
    }

    fn manifest(rank_input_sha256: &str) -> DiscoveryRunManifest {
        DiscoveryRunManifest {
            schema_version: 1,
            run_id: "issue1218-run".to_string(),
            corpus_vault_id: "clinical-vault".to_string(),
            panel_manifest_sha256: sha('a'),
            stages: vec![
                stage(
                    "typed-association-miner",
                    None,
                    sha('0'),
                    rank_input_sha256.to_string(),
                ),
                stage(
                    "hypothesis-rank",
                    Some("typed-association-miner"),
                    rank_input_sha256.to_string(),
                    sha('2'),
                ),
            ],
        }
    }

    fn stage(
        stage_id: &str,
        upstream_stage_id: Option<&str>,
        input_sha256: String,
        output_sha256: String,
    ) -> DiscoveryRunStage {
        DiscoveryRunStage {
            stage_id: stage_id.to_string(),
            command: format!("calyx {stage_id}"),
            args: vec!["--synthetic".to_string()],
            upstream_stage_id: upstream_stage_id.map(ToString::to_string),
            input_sha256,
            output_sha256,
            git_sha: "946bfb5a".to_string(),
        }
    }

    fn write_manifest(path: &Path, manifest: &DiscoveryRunManifest) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "calyx-discovery-run-preflight-{name}-{}-{}",
            std::process::id(),
            ulid::Ulid::new()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn cleanup(path: PathBuf) {
        fs::remove_dir_all(path).unwrap();
    }

    fn sha(ch: char) -> String {
        std::iter::repeat_n(ch, 64).collect()
    }
}
