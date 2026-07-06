use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use calyx_registry::{LensForgeManifest, LensRuntime, LensSpec, lens_spec_from_manifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::a35_signal::lens_spec_signal_kind_name;
use crate::assay_corpus_build::lens::projection::projected_slot_dim;
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::dim;

use super::args::Args;
use super::{io_error, local_error};

const KEY_PREFIX: &[u8] = b"calyx/assay/stream-fbin/lens-template/v1/";
const VALUE_MAGIC: &[u8] = b"CSFLTP1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const DEFAULT_ASSOCIATION_KEY: &str = "stream_fbin_lens_template";
pub(crate) const FORMAT: &str = "calyx-assay-stream-fbin-lens-template-v1";
pub(crate) const MODE: &str = "stream_fbin_lens_template";

mod direct;
mod import_args;
mod lens_spec_db;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LensTemplateRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) roster_sha256: String,
    pub(crate) descriptors: Vec<LensTemplateDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LensTemplateDescriptor {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) lens_id: String,
    pub(crate) weights_sha256: String,
    pub(crate) signal_kind: String,
    pub(crate) dim: usize,
    pub(crate) native_dim: usize,
    pub(crate) runtime: String,
    pub(crate) model_id: String,
    pub(crate) endpoint: Option<String>,
    pub(crate) dtype: String,
    pub(crate) quantization: String,
    pub(crate) max_batch: Option<usize>,
    pub(crate) activation_status: String,
    pub(crate) admission_status: String,
    pub(crate) source_path: String,
    pub(crate) import_manifest_sha256: String,
    pub(crate) spec_sha256: String,
    #[serde(with = "lens_spec_db")]
    pub(crate) spec: LensSpec,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensTemplateDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) row_key_sha256: String,
    pub(crate) value_bytes: usize,
    pub(crate) value_sha256: String,
    pub(crate) roster_sha256: String,
    pub(crate) descriptor_count: usize,
    pub(crate) lens_names: Vec<String>,
    pub(crate) readback_matches: bool,
}

pub(crate) struct RuntimeTemplate {
    pub(crate) readback: Option<LensTemplateDbReadback>,
}

impl RuntimeTemplate {
    fn none() -> Self {
        Self { readback: None }
    }
}

pub(crate) fn run_import(raw: &[String]) -> CliResult {
    let args = import_args::ImportArgs::parse(raw)?;
    let record = args.record()?;
    let readback = write(&args.cf_root, &args.association_key, &record).map_err(CliError::Calyx)?;
    println!(
        "stream_fbin_lens_template_db cf_root={} association_key={} descriptors={} roster_sha256={} value_bytes={} value_sha256={} readback_matches={}",
        readback.cf_root,
        readback.association_key,
        readback.descriptor_count,
        readback.roster_sha256,
        readback.value_bytes,
        readback.value_sha256,
        readback.readback_matches
    );
    Ok(())
}

pub(crate) fn materialize_for_stream(args: &mut Args) -> CliResult<RuntimeTemplate> {
    let Some(cf_root) = args.lens_template_cf_root.as_ref() else {
        if args.mode.requires_gate() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_REQUIRED",
                "gate mode requires a DB-native lens template; file manifests are diagnostic/import-only",
                "import the roster with assay stream-fbin-lens-template and rerun with --lens-template-cf-root",
            ));
        }
        return Ok(RuntimeTemplate::none());
    };
    if !args.manifests.is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_CONFLICT",
            "stream-fbin cannot mix --lens-template-cf-root with --manifest inputs",
            "use the DB-native lens template for gate-bearing streams; keep manifests diagnostic/import-only",
        ));
    }
    let (record, readback) = read(cf_root, &args.lens_template_key).map_err(CliError::Calyx)?;
    validate_record(&record)?;
    args.lens_template_specs = record
        .descriptors
        .iter()
        .map(|descriptor| descriptor.spec.clone())
        .collect();
    Ok(RuntimeTemplate {
        readback: Some(readback),
    })
}

pub(crate) fn write(
    cf_root: &Path,
    association_key: &str,
    record: &LensTemplateRecord,
) -> Result<LensTemplateDbReadback> {
    let row_key = row_key(association_key)?;
    let value = encode(record)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    if router.get(ColumnFamily::Graph, &row_key)?.is_some() {
        return Err(error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_EXISTS",
            "stream-fbin lens template row already exists in Graph CF",
        ));
    }
    router.put(ColumnFamily::Graph, &row_key, &value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let readback = router.get(ColumnFamily::Graph, &row_key)?.ok_or_else(|| {
        error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_MISSING",
            "stream-fbin lens template row missing after Graph CF write",
        )
    })?;
    if readback != value {
        return Err(error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_MISMATCH",
            "stream-fbin lens template Graph CF readback bytes changed after write",
        ));
    }
    Ok(readback_report(
        cf_root,
        association_key,
        &row_key,
        &readback,
        record,
        true,
    ))
}

pub(crate) fn read(
    cf_root: &Path,
    association_key: &str,
) -> Result<(LensTemplateRecord, LensTemplateDbReadback)> {
    let row_key = row_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let value = router.get(ColumnFamily::Graph, &row_key)?.ok_or_else(|| {
        error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_MISSING",
            "stream-fbin lens template row missing in Graph CF",
        )
    })?;
    let record: LensTemplateRecord = decode(&value)?;
    let readback = readback_report(cf_root, association_key, &row_key, &value, &record, true);
    Ok((record, readback))
}

pub(crate) fn record_from_manifests(manifests: &[PathBuf]) -> CliResult<LensTemplateRecord> {
    if manifests.is_empty() {
        return Err(CliError::usage(
            "lens template import requires at least one manifest",
        ));
    }
    let mut names = BTreeSet::new();
    let mut descriptors = Vec::with_capacity(manifests.len());
    for (slot, path) in manifests.iter().enumerate() {
        let descriptor = descriptor_from_manifest(slot, path)?;
        if !names.insert(descriptor.name.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DUPLICATE",
                format!("duplicate lens {} in template import", descriptor.name),
                "deduplicate the stream-fbin lens template roster",
            ));
        }
        descriptors.push(descriptor);
    }
    let roster_sha256 = roster_sha256(&descriptors);
    Ok(LensTemplateRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        roster_sha256,
        descriptors,
    })
}

fn descriptor_from_manifest(slot: usize, path: &Path) -> CliResult<LensTemplateDescriptor> {
    let source_bytes = fs::read(path).map_err(io_error)?;
    let mut manifest: LensForgeManifest =
        serde_json::from_slice(&source_bytes).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_INVALID",
                format!("parse {} failed: {error}", path.display()),
                "import only valid frozen lens manifests into the stream-fbin lens template",
            )
        })?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    normalize_manifest_paths(&mut manifest, base_dir);
    let spec = lens_spec_from_manifest(&manifest, Path::new("")).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_INVALID",
            format!("{}: {}", path.display(), error.message),
            "fix the frozen lens manifest before importing it into Calyx/Aster",
        )
    })?;
    let spec_sha256 = spec_sha256(&spec)?;
    Ok(LensTemplateDescriptor {
        slot: u16::try_from(slot).map_err(|_| CliError::usage("lens template slot exceeds u16"))?,
        name: spec.name.clone(),
        lens_id: spec.lens_id().to_string(),
        weights_sha256: hex_sha256(&spec.weights_sha256),
        signal_kind: lens_spec_signal_kind_name(&spec).to_string(),
        dim: projected_slot_dim(spec.output) as usize,
        native_dim: dim(spec.output) as usize,
        runtime: manifest.runtime,
        model_id: manifest.source_hf_id,
        endpoint: manifest.endpoint,
        dtype: manifest.dtype,
        quantization: format!("{:?}", manifest.quant_default),
        max_batch: spec.max_batch,
        activation_status: activation_status(&spec.runtime).to_string(),
        admission_status: "requires_a37_db_gate".to_string(),
        source_path: path.display().to_string(),
        import_manifest_sha256: hex_sha256(&source_bytes),
        spec_sha256,
        spec,
    })
}

fn validate_record(record: &LensTemplateRecord) -> CliResult {
    if record.format != FORMAT || record.mode != MODE {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_INVALID",
            "lens template row decoded to the wrong format or mode",
            "rewrite the stream-fbin lens template through Calyx/Aster Graph CF",
        ));
    }
    if record.descriptors.len() < super::MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!(
                "lens template has {} descriptors; A35 requires at least {}",
                record.descriptors.len(),
                super::MIN_A35_LENSES
            ),
            "import at least ten real frozen content lenses into the DB-native template",
        ));
    }
    if record.roster_sha256 != roster_sha256(&record.descriptors) {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_MISMATCH",
            "lens template roster_sha256 does not match descriptor rows",
            "rewrite the stream-fbin lens template through Calyx/Aster Graph CF",
        ));
    }
    for descriptor in &record.descriptors {
        if spec_sha256(&descriptor.spec)? != descriptor.spec_sha256 {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_MISMATCH",
                format!("template lens {} spec hash changed", descriptor.name),
                "rewrite the stream-fbin lens template through Calyx/Aster Graph CF",
            ));
        }
        if descriptor.spec.name != descriptor.name
            || descriptor.spec.lens_id().to_string() != descriptor.lens_id
            || hex_sha256(&descriptor.spec.weights_sha256) != descriptor.weights_sha256
            || lens_spec_signal_kind_name(&descriptor.spec) != descriptor.signal_kind
            || projected_slot_dim(descriptor.spec.output) as usize != descriptor.dim
            || dim(descriptor.spec.output) as usize != descriptor.native_dim
            || descriptor.spec.max_batch != descriptor.max_batch
        {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_MISMATCH",
                format!(
                    "template lens {} identity changed after DB readback",
                    descriptor.name
                ),
                "rewrite the stream-fbin lens template from valid normalized manifests",
            ));
        }
    }
    Ok(())
}

fn normalize_manifest_paths(manifest: &mut LensForgeManifest, base_dir: &Path) {
    for file in &mut manifest.files {
        if !file.path.is_absolute() {
            file.path = base_dir.join(&file.path);
        }
    }
}

fn activation_status(runtime: &LensRuntime) -> &'static str {
    match runtime {
        LensRuntime::TeiHttp { .. } => "active_resident_service_required",
        _ => "active_runtime_required",
    }
}

fn spec_sha256(spec: &LensSpec) -> CliResult<String> {
    let bytes = bincode::serde::encode_to_vec(lens_spec_db::stored(spec), config::standard())
        .map_err(|error| CliError::runtime(format!("encode lens spec: {error}")))?;
    Ok(hex_sha256(&bytes))
}

fn roster_sha256(descriptors: &[LensTemplateDescriptor]) -> String {
    let mut hasher = Sha256::new();
    for descriptor in descriptors {
        hasher.update(descriptor.slot.to_be_bytes());
        hasher.update(descriptor.name.as_bytes());
        hasher.update(descriptor.lens_id.as_bytes());
        hasher.update(descriptor.weights_sha256.as_bytes());
        hasher.update(descriptor.spec_sha256.as_bytes());
    }
    hex_from_digest(&hasher.finalize())
}

fn row_key(association_key: &str) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_INVALID_KEY",
            "stream-fbin lens template association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode_payload(record)?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn bincode_payload<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_ENCODE",
            format!("encode stream-fbin lens template failed: {err}"),
        )
    })
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_INVALID",
            "stream-fbin lens template row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_DECODE",
                format!("decode stream-fbin lens template failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DB_INVALID",
            "stream-fbin lens template row has trailing bytes",
        ));
    }
    Ok(record)
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    row_key: &[u8],
    value: &[u8],
    record: &LensTemplateRecord,
    readback_matches: bool,
) -> LensTemplateDbReadback {
    LensTemplateDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        row_key_sha256: hex_sha256(row_key),
        value_bytes: value.len(),
        value_sha256: hex_sha256(value),
        roster_sha256: record.roster_sha256.clone(),
        descriptor_count: record.descriptors.len(),
        lens_names: record
            .descriptors
            .iter()
            .map(|descriptor| descriptor.name.clone())
            .collect(),
        readback_matches,
    }
}

fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read stream-fbin lens templates through Calyx/Aster Graph CF",
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex_from_digest(&Sha256::digest(bytes))
}

fn hex_from_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
