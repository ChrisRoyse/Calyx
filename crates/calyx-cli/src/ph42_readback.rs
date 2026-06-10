use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const TOPICS: &[&str] = &[
    "assay-report",
    "temporal-cross-term",
    "kernel-weights",
    "kernel-window",
    "ward-novelty",
    "compression-ratio",
    "anneal-schedule",
];

struct ArtifactArgs {
    artifact: PathBuf,
    field: Option<String>,
}

pub fn is_topic(topic: &str) -> bool {
    TOPICS.contains(&topic)
}

pub fn readback_topic(topic: &str, args: &[String]) -> Result<(), String> {
    let args = parse_args(topic, args)?;
    let bytes = fs::read(&args.artifact)
        .map_err(|error| format!("read PH42 artifact {}: {error}", args.artifact.display()))?;
    let artifact_json: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse PH42 artifact {} as JSON: {error}",
            args.artifact.display()
        )
    })?;
    let selected = match &args.field {
        Some(field) => select_field(&artifact_json, field)?.clone(),
        None => artifact_json,
    };
    let readback = json!({
        "surface": topic,
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
