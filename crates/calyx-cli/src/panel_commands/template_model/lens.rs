use std::fs;

use calyx_core::{CalyxError, LensId};
use calyx_registry::{
    FrozenLensContract, LensForgeManifest, LensSpec, derive_runtime_contract_from_spec,
    lens_spec_from_manifest,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::lens_commands::support::runtime_name;

use super::{
    LENS_SNAPSHOT_VERSION, TEMPLATE_INVALID, TemplateLensRef, TemplateLensSnapshot, template_error,
};

pub(in crate::panel_commands) fn lens_ref_from_catalog(
    entry: &super::super::LensCatalogEntry,
) -> CliResult<TemplateLensRef> {
    let manifest_path = fs::canonicalize(&entry.manifest).map_err(|error| {
        template_error(
            TEMPLATE_INVALID,
            format!(
                "canonicalize lens manifest {} failed: {error}",
                entry.manifest.display()
            ),
            "restore the commissioned manifest and its artifacts before saving the template",
        )
    })?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        template_error(
            TEMPLATE_INVALID,
            format!(
                "read lens manifest {} failed: {error}",
                manifest_path.display()
            ),
            "restore the commissioned manifest and its artifacts before saving the template",
        )
    })?;
    let manifest: LensForgeManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        template_error(
            TEMPLATE_INVALID,
            format!(
                "parse lens manifest {} failed: {error}",
                manifest_path.display()
            ),
            "repair and recommission the lens manifest before saving the template",
        )
    })?;
    let manifest_base_dir = manifest_path
        .parent()
        .ok_or_else(|| {
            template_error(
                TEMPLATE_INVALID,
                format!(
                    "lens manifest {} has no parent directory",
                    manifest_path.display()
                ),
                "store commissioned manifests in a stable directory",
            )
        })?
        .to_path_buf();
    // This is deliberately the verifying reader, not the metadata-only reader:
    // no template object is published until every real artifact hash matches.
    let spec = lens_spec_from_manifest(&manifest, &manifest_base_dir)?;
    let runtime_contract = derive_runtime_contract_from_spec(&spec)?;
    let catalog_lens_id: LensId = entry
        .lens_id
        .parse()
        .map_err(|err| CliError::usage(format!("parse lens_id {}: {err}", entry.lens_id)))?;
    let manifest_lens_id = spec.lens_id();
    if catalog_lens_id != manifest_lens_id {
        return Err(template_error(
            TEMPLATE_INVALID,
            format!(
                "lens catalog entry {} has lens_id {}, but manifest {} resolves to {}",
                entry.name,
                catalog_lens_id,
                entry.manifest.display(),
                manifest_lens_id
            ),
            "repair the lens catalog with `calyx lens add --manifest <manifest> --home <dir>` before saving templates",
        ));
    }
    let snapshot = TemplateLensSnapshot {
        derivation_version: Some(calyx_registry::derivation::LENS_DERIVATION_VERSION),
        schema_version: LENS_SNAPSHOT_VERSION,
        manifest_blake3: json_blake3(&manifest)?,
        spec_blake3: json_blake3(&spec)?,
        runtime_contract_blake3: json_blake3(&runtime_contract)?,
        manifest,
        manifest_base_dir,
        spec: spec.clone(),
        runtime_contract,
    };
    Ok(TemplateLensRef {
        slot_key: slug(&entry.name),
        lens_name: entry.name.clone(),
        lens_id: catalog_lens_id,
        runtime_lens_id: None,
        weights_sha256: entry.weights_sha256.clone(),
        runtime: runtime_name(&spec.runtime).to_string(),
        modality: spec.modality,
        shape: spec.output,
        placement: entry.placement,
        cost: entry.cost,
        manifest: manifest_path.display().to_string(),
        immutable_snapshot: Some(snapshot),
        counts_toward_a35: true,
    })
}

impl TemplateLensSnapshot {
    pub(super) fn validate_summary(&self, template_id: &str, lens: &TemplateLensRef) -> CliResult {
        if self.schema_version != LENS_SNAPSHOT_VERSION {
            return Err(snapshot_error(
                template_id,
                lens,
                format!("unsupported snapshot schema {}", self.schema_version),
            ));
        }
        validate_snapshot_hash(
            template_id,
            lens,
            "manifest",
            &self.manifest_blake3,
            &self.manifest,
        )?;
        validate_snapshot_hash(template_id, lens, "spec", &self.spec_blake3, &self.spec)?;
        validate_snapshot_hash(
            template_id,
            lens,
            "runtime_contract",
            &self.runtime_contract_blake3,
            &self.runtime_contract,
        )?;
        let spec_id = self.spec.lens_id();
        let expected_weights = hex32(&self.spec.weights_sha256);
        let expected_runtime = runtime_name(&self.spec.runtime);
        // A summary whose ONLY conflict is `lens_id` is the #1943 signature:
        // byte-identical artifacts re-identified by an evolved derivation
        // function. That is a migration event, not tampering, and gets its own
        // typed code so operators are not sent hunting for a tamper that never
        // happened. Everything else still fails as TEMPLATE_INVALID.
        if lens.lens_id != spec_id
            && lens.lens_name == self.spec.name
            && lens.weights_sha256 == expected_weights
            && lens.runtime == expected_runtime
            && lens.modality == self.spec.modality
            && lens.shape == self.spec.output
        {
            let verdict = calyx_registry::derivation::classify(self.derivation_version)
                .for_runtime(spec_runtime_variant(&self.spec.runtime));
            if verdict
                .steps()
                .iter()
                .any(|step| step.moves_field("lens_id"))
            {
                let error = calyx_registry::derivation::panel_template_derivation_stale(
                    &verdict,
                    format!(
                        "template {template_id} lens {} summary id {} was derived by an older                          derivation than this binary, which resolves the same immutable spec to                          {spec_id}; the manifest, spec and contract hashes all still verify",
                        lens.lens_name, lens.lens_id
                    ),
                );
                return Err(template_error(error.code, error.message, error.remediation));
            }
        }
        if lens.lens_id != spec_id
            || lens.lens_name != self.spec.name
            || lens.weights_sha256 != expected_weights
            || lens.runtime != expected_runtime
            || lens.modality != self.spec.modality
            || lens.shape != self.spec.output
        {
            return Err(snapshot_error(
                template_id,
                lens,
                format!(
                    "summary conflicts with immutable spec: expected id={spec_id} name={} weights={} runtime={} modality={:?} shape={:?}; actual id={} name={} weights={} runtime={} modality={:?} shape={:?}",
                    self.spec.name,
                    expected_weights,
                    expected_runtime,
                    self.spec.modality,
                    self.spec.output,
                    lens.lens_id,
                    lens.lens_name,
                    lens.weights_sha256,
                    lens.runtime,
                    lens.modality,
                    lens.shape
                ),
            ));
        }
        Ok(())
    }
}

impl TemplateLensRef {
    pub(in crate::panel_commands) fn verified_materialization_spec(
        &self,
        template_id: &str,
    ) -> CliResult<LensSpec> {
        let snapshot = self.immutable_snapshot.as_ref().ok_or_else(|| {
            snapshot_error(
                template_id,
                self,
                "immutable manifest/spec snapshot is missing",
            )
        })?;
        snapshot.validate_summary(template_id, self)?;
        let actual = lens_spec_from_manifest(&snapshot.manifest, &snapshot.manifest_base_dir)
            .map_err(|error| self.materialization_error(template_id, "artifact_verify", error))?;
        let actual_spec_blake3 = json_blake3(&actual)?;
        if actual != snapshot.spec {
            return Err(snapshot_error(
                template_id,
                self,
                format!(
                    "verified artifacts resolve to spec id={} blake3={}, expected id={} blake3={} (manifest_blake3={})",
                    actual.lens_id(),
                    actual_spec_blake3,
                    snapshot.spec.lens_id(),
                    snapshot.spec_blake3,
                    snapshot.manifest_blake3
                ),
            ));
        }
        let actual_contract = derive_runtime_contract_from_spec(&actual).map_err(|error| {
            self.materialization_error(template_id, "runtime_contract_derive", error)
        })?;
        let actual_contract_blake3 = json_blake3(&actual_contract)?;
        if actual_contract != snapshot.runtime_contract {
            return Err(snapshot_error(
                template_id,
                self,
                format!(
                    "derived runtime contract id={} blake3={}, expected id={} blake3={} (spec_blake3={})",
                    actual_contract.lens_id(),
                    actual_contract_blake3,
                    snapshot.runtime_contract.lens_id(),
                    snapshot.runtime_contract_blake3,
                    snapshot.spec_blake3
                ),
            ));
        }
        Ok(actual)
    }

    pub(in crate::panel_commands) fn expected_runtime_contract(
        &self,
    ) -> Option<&FrozenLensContract> {
        self.immutable_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.runtime_contract)
    }

    pub(in crate::panel_commands) fn materialization_error(
        &self,
        template_id: &str,
        stage: &str,
        error: CalyxError,
    ) -> CliError {
        let (spec_blake3, manifest_blake3) = self
            .immutable_snapshot
            .as_ref()
            .map(|snapshot| {
                (
                    snapshot.spec_blake3.as_str(),
                    snapshot.manifest_blake3.as_str(),
                )
            })
            .unwrap_or(("missing", "missing"));
        CliError::from(CalyxError {
            code: error.code,
            message: format!(
                "template {template_id} lens {} ({}) stage={stage} failed: {}; spec_blake3={spec_blake3} manifest_blake3={manifest_blake3}",
                self.lens_name, self.lens_id, error.message
            ),
            remediation: error.remediation,
        })
    }
}

fn validate_snapshot_hash<T: Serialize>(
    template_id: &str,
    lens: &TemplateLensRef,
    kind: &str,
    expected: &str,
    value: &T,
) -> CliResult {
    let actual = json_blake3(value)?;
    if expected.len() == 64
        && expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        && expected == actual
    {
        return Ok(());
    }
    Err(snapshot_error(
        template_id,
        lens,
        format!(
            "{kind} snapshot hash mismatch: expected={expected} actual={actual} manifest_blake3={}",
            lens.immutable_snapshot
                .as_ref()
                .map(|snapshot| snapshot.manifest_blake3.as_str())
                .unwrap_or("missing")
        ),
    ))
}

fn snapshot_error(
    template_id: &str,
    lens: &TemplateLensRef,
    detail: impl Into<String>,
) -> CliError {
    template_error(
        TEMPLATE_INVALID,
        format!(
            "template {template_id} lens {} ({}) snapshot invalid: {}",
            lens.lens_name,
            lens.lens_id,
            detail.into()
        ),
        "do not edit immutable template objects; verify artifacts, then save an explicit new template version",
    )
}

pub(super) fn json_blake3<T: Serialize>(value: &T) -> CliResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        CliError::runtime(format!("serialize immutable lens snapshot: {error}"))
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Maps a `LensRuntime` to the variant name used by the derivation changelog.
fn spec_runtime_variant(runtime: &calyx_registry::LensRuntime) -> &'static str {
    use calyx_registry::LensRuntime as R;
    match runtime {
        R::Algorithmic { .. } => "Algorithmic",
        R::TeiHttp { .. } => "TeiHttp",
        R::TeiHttpSegmented { .. } => "TeiHttpSegmented",
        R::CandleLocal { .. } => "CandleLocal",
        R::Onnx { .. } => "Onnx",
        R::OnnxColbert { .. } => "OnnxColbert",
        R::FastembedSparse { .. } => "FastembedSparse",
        R::FastembedBgem3 { .. } => "FastembedBgem3",
        R::FastembedReranker { .. } => "FastembedReranker",
        R::FastembedQwen3 { .. } => "FastembedQwen3",
        R::StaticLookup { .. } => "StaticLookup",
        R::MultimodalAdapter { .. } => "MultimodalAdapter",
        R::ExternalCmd { .. } => "ExternalCmd",
    }
}
