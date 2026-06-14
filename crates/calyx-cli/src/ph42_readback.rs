use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const ARTIFACT_SCHEMA_VERSION: u64 = 1;
const PH42_ARTIFACT_SOURCE_OF_TRUTH: &str = "PH42 persisted artifact";
const PH59_ARTIFACT_SOURCE_OF_TRUTH: &str = "PH59 compression report artifact";
const ARTIFACT_SCHEMA_ERROR: &str = "CALYX_PH42_ARTIFACT_SCHEMA";

const TOPICS: &[&str] = &[
    "assay-report",
    "temporal-cross-term",
    "kernel-weights",
    "kernel-window",
    "ward-novelty",
    "compression-ratio",
    "compression-report",
    "anneal-schedule",
];

struct ArtifactArgs {
    artifact: PathBuf,
    field: Option<String>,
}

pub fn is_topic(topic: &str) -> bool {
    TOPICS.contains(&topic)
}

pub fn readback_topic(topic: &str, args: &[String]) -> crate::error::CliResult {
    let args = parse_args(topic, args)?;
    let bytes = fs::read(&args.artifact)
        .map_err(|error| format!("read PH42 artifact {}: {error}", args.artifact.display()))?;
    let artifact_json: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse PH42 artifact {} as JSON: {error}",
            args.artifact.display()
        )
    })?;
    let schema = validate_artifact_schema(topic, &args.artifact, &artifact_json)?;
    let selected = match &args.field {
        Some(field) => select_field(&artifact_json, field)?.clone(),
        None => artifact_json.clone(),
    };
    let readback = json!({
        "surface": topic,
        "artifact_kind": schema.kind,
        "schema_version": schema.version,
        "artifact": display_path(&args.artifact),
        "artifact_len": bytes.len(),
        "artifact_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "field": args.field,
        "value": selected,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn parse_args(topic: &str, args: &[String]) -> Result<ArtifactArgs, String> {
    let mut artifact = None;
    let mut field = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--artifact" => {
                i += 1;
                artifact = args.get(i).map(PathBuf::from);
            }
            "--field" => {
                i += 1;
                field = args.get(i).cloned();
            }
            other => {
                return Err(format!(
                    "readback {topic} expected --artifact <json> [--field <path>], got {other}"
                ));
            }
        }
        i += 1;
    }
    let artifact = artifact
        .ok_or_else(|| format!("readback {topic} requires --artifact <json> [--field <path>]"))?;
    Ok(ArtifactArgs { artifact, field })
}

struct ArtifactSchema<'a> {
    kind: &'a str,
    version: u64,
}

fn validate_artifact_schema<'a>(
    topic: &str,
    artifact: &Path,
    value: &'a Value,
) -> Result<ArtifactSchema<'a>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| schema_error(artifact, "root must be a JSON object"))?;
    let surface = required_string(artifact, object, "surface")?;
    if surface != topic {
        return Err(schema_error(
            artifact,
            format!("surface mismatch: expected {topic}, found {surface}"),
        ));
    }

    let expected_kind = expected_artifact_kind(topic)
        .ok_or_else(|| schema_error(artifact, format!("unknown readback surface {topic}")))?;
    let kind = required_string(artifact, object, "artifact_kind")?;
    if kind != expected_kind {
        return Err(schema_error(
            artifact,
            format!("artifact_kind mismatch: expected {expected_kind}, found {kind}"),
        ));
    }

    let version = required_u64(artifact, object, "schema_version")?;
    if version != ARTIFACT_SCHEMA_VERSION {
        return Err(schema_error(
            artifact,
            format!(
                "schema_version mismatch: expected {}, found {version}",
                ARTIFACT_SCHEMA_VERSION
            ),
        ));
    }

    let source = required_string(artifact, object, "source_of_truth")?;
    let expected_source = expected_source_of_truth(topic)
        .ok_or_else(|| schema_error(artifact, format!("unknown readback surface {topic}")))?;
    if source != expected_source {
        return Err(schema_error(
            artifact,
            format!("source_of_truth mismatch: expected {expected_source}, found {source}"),
        ));
    }

    Ok(ArtifactSchema { kind, version })
}

fn required_string<'a>(
    artifact: &Path,
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| schema_error(artifact, format!("missing required string field {field}")))
}

fn required_u64(
    artifact: &Path,
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, String> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| schema_error(artifact, format!("missing required u64 field {field}")))
}

fn expected_artifact_kind(topic: &str) -> Option<&'static str> {
    match topic {
        "assay-report" => Some("ph42.assay-report.v1"),
        "temporal-cross-term" => Some("ph42.temporal-cross-term.v1"),
        "kernel-weights" => Some("ph42.kernel-weights.v1"),
        "kernel-window" => Some("ph42.kernel-window.v1"),
        "ward-novelty" => Some("ph42.ward-novelty.v1"),
        "compression-ratio" => Some("ph42.compression-ratio.v1"),
        "compression-report" => Some("ph59.compression-report.v1"),
        "anneal-schedule" => Some("ph42.anneal-schedule.v1"),
        _ => None,
    }
}

fn expected_source_of_truth(topic: &str) -> Option<&'static str> {
    match topic {
        "compression-report" => Some(PH59_ARTIFACT_SOURCE_OF_TRUTH),
        topic if TOPICS.contains(&topic) => Some(PH42_ARTIFACT_SOURCE_OF_TRUTH),
        _ => None,
    }
}

fn schema_error(artifact: &Path, detail: impl AsRef<str>) -> String {
    format!(
        "{ARTIFACT_SCHEMA_ERROR} artifact {}: {}",
        display_path(artifact),
        detail.as_ref()
    )
}

fn select_field<'a>(value: &'a Value, field: &str) -> Result<&'a Value, String> {
    let mut current = value;
    for segment in field.split('.') {
        if segment.is_empty() {
            return Err(format!("invalid empty segment in --field {field}"));
        }
        current = current
            .get(segment)
            .ok_or_else(|| format!("field {field} missing segment {segment}"))?;
    }
    Ok(current)
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
