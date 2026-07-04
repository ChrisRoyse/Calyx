use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{LodestarError, Result};

pub const DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRunStage {
    pub stage_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub upstream_stage_id: Option<String>,
    pub input_sha256: String,
    pub output_sha256: String,
    pub git_sha: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRunManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub corpus_vault_id: String,
    pub panel_manifest_sha256: String,
    pub stages: Vec<DiscoveryRunStage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRunSeal {
    pub manifest: DiscoveryRunManifest,
    pub manifest_sha256: String,
    pub ledger_ref: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedStageOutput {
    pub stage_id: String,
    pub output_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRunReproductionReport {
    pub schema_version: u32,
    pub run_id: String,
    pub stage_count: usize,
    pub manifest_sha256: String,
    pub status: DiscoveryRunReproductionStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryRunReproductionStatus {
    Match,
}

pub fn build_discovery_run_manifest(
    run_id: impl Into<String>,
    corpus_vault_id: impl Into<String>,
    panel_manifest_sha256: impl Into<String>,
    stages: Vec<DiscoveryRunStage>,
) -> Result<DiscoveryRunManifest> {
    let manifest = DiscoveryRunManifest {
        schema_version: DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION,
        run_id: run_id.into(),
        corpus_vault_id: corpus_vault_id.into(),
        panel_manifest_sha256: panel_manifest_sha256.into(),
        stages,
    };
    validate_discovery_run_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_discovery_run_manifest(manifest: &DiscoveryRunManifest) -> Result<()> {
    if manifest.schema_version != DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION {
        return invalid_manifest(format!(
            "schema_version {} != {}",
            manifest.schema_version, DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.run_id.trim().is_empty() || manifest.corpus_vault_id.trim().is_empty() {
        return invalid_manifest("run_id and corpus_vault_id must not be empty");
    }
    validate_sha("panel_manifest_sha256", &manifest.panel_manifest_sha256)?;
    if manifest.stages.is_empty() {
        return invalid_manifest("at least one discovery stage is required");
    }
    let mut seen = BTreeSet::new();
    let mut outputs = BTreeMap::<&str, &str>::new();
    for (index, stage) in manifest.stages.iter().enumerate() {
        validate_stage(stage)?;
        if !seen.insert(stage.stage_id.as_str()) {
            return invalid_manifest(format!("duplicate stage_id {}", stage.stage_id));
        }
        validate_stage_chain(index, stage, &manifest.stages, &outputs)?;
        outputs.insert(stage.stage_id.as_str(), stage.output_sha256.as_str());
    }
    Ok(())
}

pub fn manifest_sha256(manifest: &DiscoveryRunManifest) -> Result<String> {
    validate_discovery_run_manifest(manifest)?;
    let bytes = serde_json::to_vec(manifest).map_err(|error| {
        LodestarError::DiscoveryRunManifestInvalid {
            detail: format!("serialize discovery-run manifest: {error}"),
        }
    })?;
    Ok(sha256_hex(&bytes))
}

pub fn seal_discovery_run_manifest<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    manifest: DiscoveryRunManifest,
) -> Result<DiscoveryRunSeal>
where
    S: LedgerCfStore,
    C: Clock,
{
    let manifest_sha256 = manifest_sha256(&manifest)?;
    let payload = manifest_payload(&manifest, &manifest_sha256)?;
    let ledger_ref = ledger
        .append(
            EntryKind::Assay,
            SubjectId::Query(manifest.run_id.as_bytes().to_vec()),
            payload,
            ActorId::Service("calyx-lodestar".to_string()),
        )
        .map_err(LodestarError::from)?;
    Ok(DiscoveryRunSeal {
        manifest,
        manifest_sha256,
        ledger_ref,
    })
}

pub fn reproduce_discovery_run_manifest(
    manifest: &DiscoveryRunManifest,
    observed_outputs: &[ObservedStageOutput],
) -> Result<DiscoveryRunReproductionReport> {
    validate_discovery_run_manifest(manifest)?;
    let observed = observed_outputs
        .iter()
        .map(|row| (row.stage_id.as_str(), row.output_sha256.as_str()))
        .collect::<BTreeMap<_, _>>();
    for stage in &manifest.stages {
        let Some(output_sha256) = observed.get(stage.stage_id.as_str()) else {
            return manifest_drift(format!("missing reproduced output for {}", stage.stage_id));
        };
        if *output_sha256 != stage.output_sha256 {
            return manifest_drift(format!(
                "stage {} output drift: expected {}, observed {}",
                stage.stage_id, stage.output_sha256, output_sha256
            ));
        }
    }
    Ok(DiscoveryRunReproductionReport {
        schema_version: DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION,
        run_id: manifest.run_id.clone(),
        stage_count: manifest.stages.len(),
        manifest_sha256: manifest_sha256(manifest)?,
        status: DiscoveryRunReproductionStatus::Match,
    })
}

fn validate_stage(stage: &DiscoveryRunStage) -> Result<()> {
    if stage.stage_id.trim().is_empty() || stage.command.trim().is_empty() {
        return invalid_manifest("stage_id and command must not be empty");
    }
    validate_sha("input_sha256", &stage.input_sha256)?;
    validate_sha("output_sha256", &stage.output_sha256)?;
    if stage.git_sha.trim().is_empty() {
        return invalid_manifest(format!(
            "stage {} git_sha must not be empty",
            stage.stage_id
        ));
    }
    Ok(())
}

fn validate_stage_chain<'a>(
    index: usize,
    stage: &DiscoveryRunStage,
    stages: &'a [DiscoveryRunStage],
    outputs: &BTreeMap<&'a str, &'a str>,
) -> Result<()> {
    let expected = if let Some(upstream) = stage.upstream_stage_id.as_deref() {
        outputs.get(upstream).copied().ok_or_else(|| {
            LodestarError::DiscoveryRunManifestMissingUpstream {
                stage: stage.stage_id.clone(),
                upstream: upstream.to_string(),
            }
        })?
    } else if index == 0 {
        return Ok(());
    } else {
        stages[index - 1].output_sha256.as_str()
    };
    if stage.input_sha256 != expected {
        return Err(LodestarError::DiscoveryRunManifestChainBroken {
            stage: stage.stage_id.clone(),
            expected: expected.to_string(),
            found: stage.input_sha256.clone(),
        });
    }
    Ok(())
}

fn manifest_payload(manifest: &DiscoveryRunManifest, manifest_sha256: &str) -> Result<Vec<u8>> {
    let stages = manifest
        .stages
        .iter()
        .map(|stage| {
            json!({
                "stage_id": stage.stage_id,
                "command": stage.command,
                "upstream_stage_id": stage.upstream_stage_id,
                "input_sha256": stage.input_sha256,
                "output_sha256": stage.output_sha256,
                "git_sha": stage.git_sha,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "schema_version": DISCOVERY_RUN_MANIFEST_SCHEMA_VERSION,
        "run_id": manifest.run_id,
        "corpus_vault_id": manifest.corpus_vault_id,
        "panel_manifest_sha256": manifest.panel_manifest_sha256,
        "manifest_sha256": manifest_sha256,
        "stage_count": manifest.stages.len(),
        "stages": stages,
    }))
    .map_err(|error| LodestarError::DiscoveryRunManifestInvalid {
        detail: format!("serialize discovery-run ledger payload: {error}"),
    })
}

fn validate_sha(field: &str, value: &str) -> Result<()> {
    let valid = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        invalid_manifest(format!("{field} must be a 64-hex SHA-256"))
    }
}

fn invalid_manifest<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::DiscoveryRunManifestInvalid {
        detail: detail.into(),
    })
}

fn manifest_drift<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::DiscoveryRunManifestDrift {
        detail: detail.into(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
