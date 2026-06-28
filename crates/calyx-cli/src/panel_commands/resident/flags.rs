use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use calyx_core::{CalyxError, Modality};

use super::DEFAULT_BIND;
use super::protocol::{ClientMeasureInput, hex_decode, hex_encode};
use crate::error::{CliError, CliResult};

#[derive(Debug, Default)]
pub(super) struct ServeFlags {
    pub(super) home: Option<PathBuf>,
    pub(super) template: Option<String>,
    pub(super) bind: Option<SocketAddr>,
    pub(super) ready_out: Option<PathBuf>,
    pub(super) progress_out: Option<PathBuf>,
    pub(super) max_resident_vram_mib: Option<u64>,
    pub(super) resident_overhead_multiplier_milli: Option<u64>,
    pub(super) max_load_secs: Option<u64>,
    pub(super) load_parallelism: Option<usize>,
}

#[derive(Debug)]
pub(super) struct ClientFlags {
    pub(super) addr: SocketAddr,
    pub(super) out: Option<PathBuf>,
    pub(super) modality: Option<Modality>,
    pub(super) input: Option<ClientMeasureInput>,
}

pub(super) fn parse_serve_flags(args: &[String]) -> CliResult<ServeFlags> {
    let mut flags = ServeFlags::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--home" => flags.home = Some(PathBuf::from(value(args, idx + 1, "--home")?)),
            "--template" => flags.template = Some(value(args, idx + 1, "--template")?.to_string()),
            "--bind" => flags.bind = Some(parse_addr(value(args, idx + 1, "--bind")?)?),
            "--ready-out" => {
                flags.ready_out = Some(PathBuf::from(value(args, idx + 1, "--ready-out")?))
            }
            "--progress-out" => {
                flags.progress_out = Some(PathBuf::from(value(args, idx + 1, "--progress-out")?))
            }
            "--max-resident-vram-mib" => {
                flags.max_resident_vram_mib = Some(parse_u64(
                    value(args, idx + 1, "--max-resident-vram-mib")?,
                    "--max-resident-vram-mib",
                )?)
            }
            "--resident-overhead-multiplier" => {
                flags.resident_overhead_multiplier_milli = Some(parse_multiplier_milli(value(
                    args,
                    idx + 1,
                    "--resident-overhead-multiplier",
                )?)?)
            }
            "--max-load-secs" => {
                flags.max_load_secs = Some(parse_u64(
                    value(args, idx + 1, "--max-load-secs")?,
                    "--max-load-secs",
                )?)
            }
            "--load-parallelism" => {
                flags.load_parallelism = Some(parse_usize(
                    value(args, idx + 1, "--load-parallelism")?,
                    "--load-parallelism",
                )?)
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected panel resident serve flag {other}"
                )));
            }
        }
        idx += 2;
    }
    Ok(flags)
}

pub(super) fn parse_client_flags(args: &[String], op: &str) -> CliResult<ClientFlags> {
    let mut addr = parse_addr(DEFAULT_BIND)?;
    let mut out = None;
    let mut modality = None;
    let mut input = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--addr" => addr = parse_addr(value(args, idx + 1, "--addr")?)?,
            "--out" => out = Some(PathBuf::from(value(args, idx + 1, "--out")?)),
            "--modality" => modality = Some(parse_modality(value(args, idx + 1, "--modality")?)?),
            "--input" => set_input(
                &mut input,
                ClientMeasureInput::Utf8(value(args, idx + 1, "--input")?.to_string()),
                "--input",
            )?,
            "--input-file" => {
                let path = PathBuf::from(value(args, idx + 1, "--input-file")?);
                let bytes = std::fs::read(&path).map_err(|error| {
                    CliError::io(format!("read --input-file {path:?}: {error}"))
                })?;
                set_input(
                    &mut input,
                    ClientMeasureInput::Hex(hex_encode(&bytes)),
                    "--input-file",
                )?;
            }
            "--input-hex" => {
                let raw = value(args, idx + 1, "--input-hex")?;
                let bytes = hex_decode(raw).map_err(CliError::usage)?;
                set_input(
                    &mut input,
                    ClientMeasureInput::Hex(hex_encode(&bytes)),
                    "--input-hex",
                )?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected panel resident {op} flag {other}"
                )));
            }
        }
        idx += 2;
    }
    if op == "measure" && (modality.is_none() || input.is_none()) {
        return Err(CliError::usage(
            "calyx panel resident measure requires --modality <name> and exactly one input flag",
        ));
    }
    Ok(ClientFlags {
        addr,
        out,
        modality,
        input,
    })
}

fn set_input(
    slot: &mut Option<ClientMeasureInput>,
    value: ClientMeasureInput,
    flag: &str,
) -> CliResult {
    if slot.is_some() {
        return Err(CliError::usage(format!(
            "calyx panel resident measure accepts only one input flag; duplicate at {flag}"
        )));
    }
    *slot = Some(value);
    Ok(())
}

pub(super) fn parse_addr(raw: &str) -> CliResult<SocketAddr> {
    raw.parse::<SocketAddr>()
        .map_err(|error| CliError::usage(format!("parse socket address {raw}: {error}")))
}

pub(super) fn ensure_loopback(addr: SocketAddr) -> CliResult {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(()),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(()),
        _ => Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_BIND_REFUSED",
            message: format!("resident service address {addr} is not loopback"),
            remediation: "bind resident services only to 127.0.0.1 or [::1]",
        })),
    }
}

fn parse_modality(raw: &str) -> CliResult<Modality> {
    match raw {
        "text" => Ok(Modality::Text),
        "code" => Ok(Modality::Code),
        "image" => Ok(Modality::Image),
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        "protein" => Ok(Modality::Protein),
        "dna" => Ok(Modality::Dna),
        "molecule" => Ok(Modality::Molecule),
        "structured" => Ok(Modality::Structured),
        "mixed" => Ok(Modality::Mixed),
        other => Err(CliError::usage(format!("unknown modality {other}"))),
    }
}

fn parse_u64(raw: &str, flag: &str) -> CliResult<u64> {
    raw.parse::<u64>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))
}

fn parse_usize(raw: &str, flag: &str) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))?;
    if value == 0 {
        return Err(CliError::usage(format!("{flag} must be greater than zero")));
    }
    Ok(value)
}

fn parse_multiplier_milli(raw: &str) -> CliResult<u64> {
    let value = raw.parse::<f64>().map_err(|error| {
        CliError::usage(format!(
            "parse --resident-overhead-multiplier {raw}: {error}"
        ))
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(CliError::usage(format!(
            "--resident-overhead-multiplier must be a positive finite number, got {raw}"
        )));
    }
    Ok((value * 1000.0).ceil() as u64)
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

pub(super) fn calyx_home() -> CliResult<PathBuf> {
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))
}
