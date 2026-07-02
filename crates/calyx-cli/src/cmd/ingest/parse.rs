use super::super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand, value};
use super::session::{IngestStatusArgs, validate_session_id};
use super::types::IngestOutput;
use crate::error::{CliError, CliResult};
use std::net::{IpAddr, SocketAddr};

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
    let mut output = IngestOutput::Summary;
    let mut resident_addr = None;
    let mut allow_cold_gpu_workers = false;
    let mut session_id = None;
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
            "--output" => {
                idx += 1;
                output = parse_ingest_output(value(rest, idx, "--output")?)?;
            }
            "--resident-addr" => {
                idx += 1;
                resident_addr = Some(parse_resident_addr(value(rest, idx, "--resident-addr")?)?);
            }
            "--allow-cold-gpu-workers" => allow_cold_gpu_workers = true,
            "--session-id" => {
                idx += 1;
                let value = value(rest, idx, "--session-id")?;
                validate_session_id(value)?;
                session_id = Some(value.to_string());
            }
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
            "ingest --file requires --modality <image|audio|video>",
        ));
    }
    if file.is_none() && modality.is_some() {
        return Err(CliError::usage("--modality is only valid with --file"));
    }
    if session_id.is_some() && batch.is_none() {
        return Err(CliError::usage("--session-id is only valid with --batch"));
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
        output,
        resident_addr,
        allow_cold_gpu_workers,
        session_id,
    }))
}

pub(crate) fn parse_ingest_status(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("ingest-status requires <vault>"))?
        .clone();
    let mut session_id = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--session" => {
                idx += 1;
                let value = value(rest, idx, "--session")?;
                validate_session_id(value)?;
                session_id = Some(value.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected ingest-status flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::IngestStatus(IngestStatusArgs {
        vault,
        session_id: session_id
            .ok_or_else(|| CliError::usage("ingest-status requires --session <id>"))?,
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

fn parse_ingest_output(value: &str) -> CliResult<IngestOutput> {
    match value {
        "summary" => Ok(IngestOutput::Summary),
        "rows" => Ok(IngestOutput::Rows),
        other => Err(CliError::usage(format!(
            "invalid --output {other}; expected summary or rows"
        ))),
    }
}

pub(super) fn parse_resident_addr(raw: &str) -> CliResult<SocketAddr> {
    let addr = raw
        .parse::<SocketAddr>()
        .map_err(|error| CliError::usage(format!("parse --resident-addr {raw}: {error}")))?;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(addr),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(addr),
        _ => Err(CliError::from(calyx_core::CalyxError {
            code: "CALYX_INGEST_RESIDENT_ADDR_REFUSED",
            message: format!("--resident-addr {addr} is not loopback"),
            remediation: "bind and use the resident measurement service only on 127.0.0.1 or [::1]",
        })),
    }
}
