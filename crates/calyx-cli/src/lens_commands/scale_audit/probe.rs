use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Input, Modality};
use sha2::{Digest, Sha256};

use super::model::{Flags, ProbeEvidence};

#[derive(Debug)]
pub(super) struct ProbeSet {
    pub(super) inputs: Vec<Input>,
    pub(super) evidence: Vec<ProbeEvidence>,
}

pub(super) fn supports_probe_measurement(modality: Modality) -> bool {
    matches!(
        modality,
        Modality::Text | Modality::Code | Modality::Image | Modality::Audio
    )
}

pub(super) fn probe_set(
    flags: &Flags,
    modality: Modality,
    sample_rows: usize,
) -> Result<ProbeSet, CalyxError> {
    let probes = explicit_or_default_probes(flags, modality)?;
    let evidence = probes.iter().map(ProbeBytes::evidence).collect::<Vec<_>>();
    let inputs = (0..sample_rows.max(1))
        .map(|idx| {
            let probe = &probes[idx % probes.len()];
            Input::new(modality, probe.bytes.clone()).with_pointer(probe.pointer())
        })
        .collect::<Vec<_>>();
    Ok(ProbeSet { inputs, evidence })
}

fn explicit_or_default_probes(
    flags: &Flags,
    modality: Modality,
) -> Result<Vec<ProbeBytes>, CalyxError> {
    let mut probes = Vec::new();
    for path in &flags.probe_files {
        if let Some(probe) = ProbeBytes::from_file_for_modality(path, modality)? {
            probes.push(probe);
        }
    }
    if matches!(modality, Modality::Text | Modality::Code) {
        for (idx, text) in flags.probes.iter().enumerate() {
            probes.push(ProbeBytes::from_text(idx, text));
        }
    } else if !flags.probes.is_empty() {
        return Err(CalyxError {
            code: "CALYX_LENS_SCALE_PROBE_UNSUPPORTED",
            message: format!("--probe text is not valid for modality {modality:?}"),
            remediation: "use --probe-file with valid image/audio bytes or omit probes for built-in modality fixtures",
        });
    }
    if !flags.probe_files.is_empty() && probes.is_empty() {
        return Err(CalyxError {
            code: "CALYX_LENS_SCALE_PROBE_UNSUPPORTED",
            message: format!("no --probe-file matched modality {modality:?}"),
            remediation: "provide a PNG/JPEG for image lenses or RIFF/WAVE for audio lenses",
        });
    }
    if probes.is_empty() {
        probes.extend(default_probes(modality)?);
    }
    Ok(probes)
}

#[derive(Clone)]
struct ProbeBytes {
    source: String,
    path: Option<PathBuf>,
    bytes: Vec<u8>,
}

impl ProbeBytes {
    fn from_file_for_modality(path: &Path, modality: Modality) -> Result<Option<Self>, CalyxError> {
        let bytes = fs::read(path).map_err(|error| CalyxError {
            code: "CALYX_LENS_SCALE_PROBE_READ_FAILED",
            message: format!("read probe file {}: {error}", path.display()),
            remediation: "provide readable --probe-file bytes or omit it to use built-in probes",
        })?;
        match (modality, media_probe_modality(&bytes)) {
            (Modality::Image, Some(Modality::Image)) | (Modality::Audio, Some(Modality::Audio)) => {
            }
            (Modality::Image | Modality::Audio, Some(_)) => return Ok(None),
            (Modality::Image | Modality::Audio, None) => {
                return Err(CalyxError {
                    code: "CALYX_LENS_SCALE_PROBE_UNSUPPORTED",
                    message: format!(
                        "probe file {} is not PNG/JPEG or RIFF/WAVE bytes",
                        path.display()
                    ),
                    remediation: "provide modality-specific probe bytes that pass the lens adapter preflight",
                });
            }
            _ => {}
        }
        Ok(Some(Self {
            source: "file".to_string(),
            path: Some(path.to_path_buf()),
            bytes,
        }))
    }

    fn from_text(idx: usize, text: &str) -> Self {
        Self {
            source: format!("--probe[{idx}]"),
            path: None,
            bytes: text.as_bytes().to_vec(),
        }
    }

    fn from_default(source: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            source: source.to_string(),
            path: None,
            bytes,
        }
    }

    fn evidence(&self) -> ProbeEvidence {
        ProbeEvidence {
            source: self.source.clone(),
            path: self.path.clone(),
            sha256: sha256_hex(&self.bytes),
            bytes: self.bytes.len(),
        }
    }

    fn pointer(&self) -> String {
        format!(
            "scale-audit://{}#sha256={}",
            self.source,
            sha256_hex(&self.bytes)
        )
    }
}

fn default_probes(modality: Modality) -> Result<Vec<ProbeBytes>, CalyxError> {
    match modality {
        Modality::Text | Modality::Code => Ok(default_text_probes()
            .into_iter()
            .enumerate()
            .map(|(idx, text)| ProbeBytes::from_text(idx, text))
            .collect()),
        Modality::Image => Ok(vec![ProbeBytes::from_default(
            "builtin:image/png-1x1-rgb",
            default_png().to_vec(),
        )]),
        Modality::Audio => Ok(vec![ProbeBytes::from_default(
            "builtin:audio/wav-16khz-1s-sine",
            default_wav(),
        )]),
        other => Err(CalyxError {
            code: "CALYX_LENS_SCALE_MODALITY_UNSUPPORTED",
            message: format!("scale-audit has no default probe for modality {other:?}"),
            remediation: "add modality-specific probe support or pass --probe-file for auditable binary bytes",
        }),
    }
}

fn default_text_probes() -> Vec<&'static str> {
    vec![
        "Calyx PH68 scale audit uses real frozen lens measurements",
        "GPU-native batching must preserve the single-row vector contract",
        "Temporal capture walks forward backward and as-of outside content floors",
        "Provider placement evidence must fail closed on CPU fallback",
        "Associational panels need enough independent content lenses",
        "Source of truth bytes are read after the trigger finishes",
        "Dense semantic and lexical signals should not collapse into one axis",
        "The CUDA GPU path needs batchable runtime placement proof",
    ]
}

fn default_png() -> &'static [u8] {
    &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
        0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 100, 248, 207, 80,
        15, 0, 3, 134, 1, 128, 90, 52, 125, 107, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}

fn default_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const FRAMES: usize = 16_000;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    let data_len = (FRAMES * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..FRAMES {
        let radians = frame as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
        let sample = (radians.sin() * 0.25 * i16::MAX as f32) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn media_probe_modality(bytes: &[u8]) -> Option<Modality> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Modality::Image)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Some(Modality::Audio)
    } else {
        None
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
