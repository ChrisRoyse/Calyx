use crate::{ForgeError, QuantLevel, Result};

const OP: &str = "compression_report";
const INPUT_REMEDIATION: &str =
    "Rebuild the report from persisted Forge, Assay, Ward, and Lodestar measurements";
const LOSS_REMEDIATION: &str =
    "Use a gentler quantization level or recompute measurements before accepting compression";

pub(crate) fn validate_vault(vault_id: &str) -> Result<()> {
    if vault_id.trim().is_empty() {
        return Err(quant_error("report", "vault_id is empty"));
    }
    Ok(())
}

pub(crate) fn validate_slot_id(slot_id: &str, level: QuantLevel) -> Result<()> {
    if slot_id.trim().is_empty() {
        return Err(quant_error(level, "slot_id is empty"));
    }
    Ok(())
}

pub(crate) fn require_bytes(original: u64, compressed: u64, level: QuantLevel) -> Result<()> {
    require_positive_u64(original, "original_bytes", level)?;
    require_positive_u64(compressed, "compressed_bytes", level)?;
    if compressed > original {
        return Err(quant_error(
            level,
            format!("compressed_bytes {compressed} exceeds original_bytes {original}"),
        ));
    }
    Ok(())
}

pub(crate) fn require_positive_u64(value: u64, name: &str, level: QuantLevel) -> Result<()> {
    if value == 0 {
        return Err(quant_error(level, format!("{name} must be positive")));
    }
    Ok(())
}

pub(crate) fn require_positive_f64(value: f64, name: &str, level: QuantLevel) -> Result<()> {
    require_finite_f64(value, name, level)?;
    if value <= 0.0 {
        return Err(quant_error(level, format!("{name} must be positive")));
    }
    Ok(())
}

pub(crate) fn require_nonnegative_f64(value: f64, name: &str, level: QuantLevel) -> Result<()> {
    require_finite_f64(value, name, level)?;
    if value < 0.0 {
        return Err(quant_error(level, format!("{name} must be nonnegative")));
    }
    Ok(())
}

pub(crate) fn require_unit_interval(value: f64, name: &str, level: QuantLevel) -> Result<()> {
    require_range_f64(value, name, 0.0, 1.0, level)
}

pub(crate) fn require_range_f64(
    value: f64,
    name: &str,
    min: f64,
    max: f64,
    level: QuantLevel,
) -> Result<()> {
    require_finite_f64(value, name, level)?;
    if value < min || value > max {
        return Err(quant_error(
            level,
            format!("{name} must be in [{min}, {max}]"),
        ));
    }
    Ok(())
}

pub(crate) fn require_finite_f64(value: f64, name: &str, level: QuantLevel) -> Result<()> {
    if !value.is_finite() {
        return Err(quant_error(level, format!("{name} must be finite")));
    }
    Ok(())
}

pub(crate) fn checked_add(left: u64, right: u64, name: &str) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| quant_error("report", format!("{name} overflow")))
}

pub(crate) fn reject_if(condition: bool, slot: &str, detail: String) -> Result<()> {
    if condition {
        return Err(intelligence_loss(slot, detail));
    }
    Ok(())
}

pub(crate) fn ratio(numerator: f64, denominator: f64) -> f64 {
    numerator / denominator
}

pub(crate) fn quant_error(level: impl ToString, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: OP.to_string(),
        level: level.to_string(),
        detail: detail.into(),
        remediation: INPUT_REMEDIATION.to_string(),
    }
}

pub(crate) fn intelligence_loss(slot: impl Into<String>, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantIntelligenceLoss {
        slot: slot.into(),
        detail: detail.into(),
        remediation: LOSS_REMEDIATION.to_string(),
    }
}
