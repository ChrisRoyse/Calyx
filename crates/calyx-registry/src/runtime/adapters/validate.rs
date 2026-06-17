use calyx_core::{CalyxError, Input, Result};

use super::axis::MultimodalAxis;

pub fn validate_input(axis: MultimodalAxis, input: &Input) -> Result<()> {
    if input.bytes.is_empty() {
        return Err(invalid_input(axis, "input is empty"));
    }
    match axis {
        MultimodalAxis::Image => validate_image(&input.bytes),
        MultimodalAxis::Audio => validate_audio(&input.bytes),
        MultimodalAxis::Protein => validate_alpha(&input.bytes, axis, b"ACDEFGHIKLMNPQRSTVWY"),
        MultimodalAxis::Dna => validate_alpha(&input.bytes, axis, b"ACGTN"),
        MultimodalAxis::Molecule => validate_smiles(&input.bytes),
    }
}

fn validate_image(bytes: &[u8]) -> Result<()> {
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
    if png || jpeg {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Image,
            "expected PNG or JPEG bytes",
        ))
    }
}

fn validate_audio(bytes: &[u8]) -> Result<()> {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Audio,
            "expected RIFF/WAVE bytes",
        ))
    }
}

fn validate_alpha(bytes: &[u8], axis: MultimodalAxis, allowed: &[u8]) -> Result<()> {
    let ok = bytes
        .iter()
        .copied()
        .all(|byte| allowed.contains(&byte.to_ascii_uppercase()));
    if ok {
        Ok(())
    } else {
        Err(invalid_input(axis, "contains unsupported sequence symbol"))
    }
}

fn validate_smiles(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid_input(MultimodalAxis::Molecule, "SMILES input is not UTF-8"))?;
    let allowed = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789[]()=#@+-/\\\\.%";
    if text.chars().any(|ch| ch.is_ascii_alphabetic())
        && text.chars().all(|ch| allowed.contains(ch))
    {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Molecule,
            "SMILES contains unsupported token",
        ))
    }
}

fn invalid_input(axis: MultimodalAxis, message: &str) -> CalyxError {
    CalyxError::lens_dim_mismatch(format!("{} adapter {message}", axis.as_str()))
}
