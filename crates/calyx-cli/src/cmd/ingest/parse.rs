use super::super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand, value};
use crate::error::{CliError, CliResult};

pub(crate) fn parse_ingest(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("ingest requires <vault>"))?
        .clone();
    let mut text = None;
    let mut batch = None;
    let mut file = None;
    let mut modality = None;
    let mut idempotent = true;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--text" => {
                idx += 1;
                let raw = value(rest, idx, "--text")?;
                validate_text(raw)?;
                text = Some(raw.to_string());
            }
            "--batch" => {
                idx += 1;
                batch = Some(value(rest, idx, "--batch")?.into());
            }
            "--file" => {
                idx += 1;
                file = Some(value(rest, idx, "--file")?.into());
            }
            "--modality" => {
                idx += 1;
                modality = Some(crate::raw_media::parse_audio_video_modality(value(
                    rest,
                    idx,
                    "--modality",
                )?)?);
            }
            "--idempotent" => {
                if let Some(raw) = rest.get(idx + 1).filter(|next| !next.starts_with("--")) {
                    idx += 1;
                    idempotent = parse_bool(raw, "--idempotent")?;
                } else {
                    idempotent = true;
                }
            }
            "--no-idempotent" => idempotent = false,
            other => return Err(CliError::usage(format!("unexpected ingest flag {other}"))),
        }
        idx += 1;
    }
    let payload_count =
        usize::from(text.is_some()) + usize::from(batch.is_some()) + usize::from(file.is_some());
    if payload_count != 1 {
        return Err(CliError::usage(
            "ingest requires exactly one of --text <s>, --batch <jsonl-path>, or --file <path>",
        ));
    }
    if file.is_some() && modality.is_none() {
        return Err(CliError::usage(
            "ingest --file requires --modality <audio|video>",
        ));
    }
    if file.is_none() && modality.is_some() {
        return Err(CliError::usage("--modality is only valid with --file"));
    }
    if !idempotent {
        return Err(CliError::usage(
            "non-idempotent ingest is not supported by Calyx",
        ));
    }
    Ok(Subcommand::Ingest(IngestArgs {
        vault,
        text,
        batch,
        file,
        modality,
        idempotent,
    }))
}

pub(crate) fn parse_anchor(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("anchor requires <vault>"))?
        .clone();
    let cx_id = rest
        .get(1)
        .ok_or_else(|| CliError::usage("anchor requires <cx_id>"))?
        .clone();
    let mut kind = None;
    let mut anchor_value = None;
    let mut confidence = None;
    let mut source = None;
    let mut idx = 2;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--kind" => {
                idx += 1;
                kind = Some(value(rest, idx, "--kind")?.to_string());
            }
            "--value" => {
                idx += 1;
                anchor_value = Some(value(rest, idx, "--value")?.to_string());
            }
            "--confidence" => {
                idx += 1;
                let raw = value(rest, idx, "--confidence")?;
                let parsed = raw
                    .parse::<f32>()
                    .map_err(|err| CliError::usage(format!("parse --confidence {raw}: {err}")))?;
                validate_confidence(parsed)?;
                confidence = Some(parsed);
            }
            "--source" => {
                idx += 1;
                source = Some(value(rest, idx, "--source")?.to_string());
            }
            other => return Err(CliError::usage(format!("unexpected anchor flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Anchor(AnchorArgs {
        vault,
        cx_id,
        kind: kind.ok_or_else(|| CliError::usage("anchor requires --kind <kind>"))?,
        value: anchor_value.ok_or_else(|| CliError::usage("anchor requires --value <v>"))?,
        confidence,
        source,
    }))
}

pub(crate) fn parse_measure(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("measure requires <vault>"))?
        .clone();
    let mut text = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--text" => {
                idx += 1;
                let raw = value(rest, idx, "--text")?;
                validate_text(raw)?;
                text = Some(raw.to_string());
            }
            other => return Err(CliError::usage(format!("unexpected measure flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Measure(MeasureArgs {
        vault,
        text: text.ok_or_else(|| CliError::usage("measure requires --text <s>"))?,
    }))
}

pub(super) fn validate_text(value: &str) -> CliResult {
    if value.is_empty() {
        return Err(CliError::usage("--text must not be empty"));
    }
    Ok(())
}

pub(super) fn validate_confidence(value: f32) -> CliResult {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(CliError::usage(
        "--confidence must be finite and within [0, 1]",
    ))
}

pub(super) fn parse_bool(value: &str, flag: &str) -> CliResult<bool> {
    value
        .parse::<bool>()
        .map_err(|err| CliError::usage(format!("parse {flag} {value}: {err}")))
}
