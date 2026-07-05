use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use calyx_core::{CalyxError, Modality, SlotId};

use super::DEFAULT_BIND;
use super::protocol::{ClientMeasureInput, hex_decode, hex_encode};
use crate::error::{CliError, CliResult};

#[derive(Debug, Default)]
pub(super) struct ServeFlags {
    pub(super) home: Option<PathBuf>,
    pub(super) template: Option<String>,
    pub(super) vault: Option<PathBuf>,
    pub(super) slots: Vec<SlotId>,
    pub(super) modality: Option<Modality>,
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
    pub(super) inputs: Vec<ClientMeasureInput>,
    pub(super) runtime_batch_limit: Option<usize>,
    pub(super) summary_only: bool,
}

pub(super) fn parse_serve_flags(args: &[String]) -> CliResult<ServeFlags> {
    let mut flags = ServeFlags::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--home" => flags.home = Some(PathBuf::from(value(args, idx + 1, "--home")?)),
            "--template" => flags.template = Some(value(args, idx + 1, "--template")?.to_string()),
            "--vault" => flags.vault = Some(PathBuf::from(value(args, idx + 1, "--vault")?)),
            "--slot" => flags
                .slots
                .push(parse_slot(value(args, idx + 1, "--slot")?)?),
            "--modality" => {
                flags.modality = Some(parse_modality(value(args, idx + 1, "--modality")?)?)
            }
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
    let mut inputs = Vec::new();
    let mut runtime_batch_limit = None;
    let mut summary_only = false;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--addr" => addr = parse_addr(value(args, idx + 1, "--addr")?)?,
            "--out" => out = Some(PathBuf::from(value(args, idx + 1, "--out")?)),
            "--modality" => modality = Some(parse_modality(value(args, idx + 1, "--modality")?)?),
            "--input" => set_input(
                &mut input,
                &mut inputs,
                ClientMeasureInput::Utf8(value(args, idx + 1, "--input")?.to_string()),
                "--input",
                op,
            )?,
            "--input-file" => {
                let path = PathBuf::from(value(args, idx + 1, "--input-file")?);
                let bytes = std::fs::read(&path).map_err(|error| {
                    CliError::io(format!("read --input-file {path:?}: {error}"))
                })?;
                set_input(
                    &mut input,
                    &mut inputs,
                    ClientMeasureInput::Hex(hex_encode(&bytes)),
                    "--input-file",
                    op,
                )?;
            }
            "--inputs-jsonl" => {
                if op != "measure-batch" {
                    return Err(CliError::usage(
                        "--inputs-jsonl is only valid for calyx panel resident measure-batch",
                    ));
                }
                let path = PathBuf::from(value(args, idx + 1, "--inputs-jsonl")?);
                let bytes = std::fs::read(&path).map_err(|error| {
                    CliError::io(format!("read --inputs-jsonl {path:?}: {error}"))
                })?;
                for (line_index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    if line.is_empty() {
                        continue;
                    }
                    let value: serde_json::Value =
                        serde_json::from_slice(line).map_err(|error| {
                            CliError::usage(format!(
                                "parse --inputs-jsonl {} line {}: {error}",
                                path.display(),
                                line_index + 1
                            ))
                        })?;
                    let text = value
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| {
                            CliError::usage(format!(
                                "--inputs-jsonl {} line {} must contain a string text field",
                                path.display(),
                                line_index + 1
                            ))
                        })?;
                    inputs.push(ClientMeasureInput::Utf8(text.to_string()));
                }
            }
            "--input-hex" => {
                let raw = value(args, idx + 1, "--input-hex")?;
                let bytes = hex_decode(raw).map_err(CliError::usage)?;
                set_input(
                    &mut input,
                    &mut inputs,
                    ClientMeasureInput::Hex(hex_encode(&bytes)),
                    "--input-hex",
                    op,
                )?;
            }
            "--runtime-batch-limit" => {
                runtime_batch_limit = Some(parse_usize(
                    value(args, idx + 1, "--runtime-batch-limit")?,
                    "--runtime-batch-limit",
                )?)
            }
            "--summary-only" => {
                if op != "measure-batch" {
                    return Err(CliError::usage(
                        "--summary-only is only valid for calyx panel resident measure-batch",
                    ));
                }
                summary_only = true;
                idx += 1;
                continue;
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
    if op == "measure-batch" && (modality.is_none() || inputs.is_empty()) {
        return Err(CliError::usage(
            "calyx panel resident measure-batch requires --modality <name> and one or more input flags",
        ));
    }
    if op != "measure-batch" && runtime_batch_limit.is_some() {
        return Err(CliError::usage(
            "--runtime-batch-limit is only valid for calyx panel resident measure-batch",
        ));
    }
    Ok(ClientFlags {
        addr,
        out,
        modality,
        input,
        inputs,
        runtime_batch_limit,
        summary_only,
    })
}

fn set_input(
    slot: &mut Option<ClientMeasureInput>,
    inputs: &mut Vec<ClientMeasureInput>,
    value: ClientMeasureInput,
    flag: &str,
    op: &str,
) -> CliResult {
    if op == "measure-batch" {
        inputs.push(value);
        return Ok(());
    }
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

fn parse_slot(raw: &str) -> CliResult<SlotId> {
    let value = raw
        .parse::<u16>()
        .map_err(|error| CliError::usage(format!("parse --slot {raw}: {error}")))?;
    Ok(SlotId::new(value))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_serve_accepts_vault_source_and_modality() {
        let flags = parse_serve_flags(&args(&[
            "--vault",
            "C:\\calyx\\vaults\\01TEST",
            "--modality",
            "text",
            "--slot",
            "22",
            "--bind",
            "127.0.0.1:8788",
        ]))
        .unwrap();
        assert_eq!(
            flags.vault.as_deref(),
            Some(Path::new("C:\\calyx\\vaults\\01TEST"))
        );
        assert_eq!(flags.modality, Some(Modality::Text));
        assert_eq!(flags.slots, vec![SlotId::new(22)]);
        assert_eq!(flags.bind, Some("127.0.0.1:8788".parse().unwrap()));
    }

    #[test]
    fn parse_serve_keeps_template_source() {
        let flags = parse_serve_flags(&args(&["--template", "blackwell-42"])).unwrap();
        assert_eq!(flags.template.as_deref(), Some("blackwell-42"));
        assert!(flags.vault.is_none());
    }

    #[test]
    fn parse_measure_batch_accepts_inputs_and_summary() {
        let flags = parse_client_flags(
            &args(&[
                "--modality",
                "text",
                "--input",
                "alpha",
                "--input-hex",
                "62657461",
                "--runtime-batch-limit",
                "4",
                "--summary-only",
            ]),
            "measure-batch",
        )
        .unwrap();

        assert_eq!(flags.modality, Some(Modality::Text));
        assert_eq!(flags.inputs.len(), 2);
        assert_eq!(flags.runtime_batch_limit, Some(4));
        assert!(flags.summary_only);
        match &flags.inputs[0] {
            ClientMeasureInput::Utf8(value) => assert_eq!(value, "alpha"),
            other => panic!("expected utf8 input, got {other:?}"),
        }
        match &flags.inputs[1] {
            ClientMeasureInput::Hex(value) => assert_eq!(value, "62657461"),
            other => panic!("expected hex input, got {other:?}"),
        }
    }

    #[test]
    fn parse_measure_batch_inputs_jsonl_requires_text_field() {
        let path = std::env::temp_dir().join(format!(
            "calyx-resident-inputs-{}.jsonl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "{\"text\":\"alpha\"}\n{\"text\":\"beta\"}\n").unwrap();
        let flags = parse_client_flags(
            &args(&[
                "--modality",
                "text",
                "--inputs-jsonl",
                path.to_str().unwrap(),
            ]),
            "measure-batch",
        )
        .unwrap();
        assert_eq!(flags.inputs.len(), 2);

        std::fs::write(&path, "{\"not_text\":1}\n").unwrap();
        let error = parse_client_flags(
            &args(&[
                "--modality",
                "text",
                "--inputs-jsonl",
                path.to_str().unwrap(),
            ]),
            "measure-batch",
        )
        .unwrap_err();
        let _ = std::fs::remove_file(path);
        assert!(error.message().contains("must contain a string text field"));
    }

    #[test]
    fn parse_measure_rejects_batch_only_flags() {
        let error = parse_client_flags(
            &args(&[
                "--modality",
                "text",
                "--input",
                "alpha",
                "--runtime-batch-limit",
                "4",
            ]),
            "measure",
        )
        .unwrap_err();
        assert!(error.message().contains("--runtime-batch-limit"));
    }
}
