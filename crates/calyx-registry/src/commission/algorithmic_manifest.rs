use calyx_core::{CalyxError, Result, SlotShape};

const CONFIG_INVALID: &str = "CALYX_LENS_CONFIG_INVALID";

pub(super) fn is_algorithmic_runtime(runtime: &str) -> bool {
    runtime == "algorithmic" || runtime.starts_with("algorithmic:")
}

pub(super) fn algorithmic_kind(runtime: &str) -> Option<&str> {
    if runtime == "algorithmic" {
        Some("byte-features")
    } else {
        runtime.strip_prefix("algorithmic:")
    }
}

pub(super) fn output_shape(runtime: &str, dim: u32) -> Result<SlotShape> {
    let Some(kind) = algorithmic_kind(runtime) else {
        return Ok(SlotShape::Dense(dim));
    };
    let shape = match kind {
        "byte" | "byte-features" => checked_dense(kind, dim, 16)?,
        "ast-style" | "ast_style" => checked_dense(kind, dim, 8)?,
        "scalar" => checked_dense(kind, dim, 1)?,
        "sparse" | "sparse-keywords" | "sparse_keywords" => SlotShape::Sparse(dim),
        "token-hash" | "token_hash" | "multi-hash" | "multi_hash" => {
            SlotShape::Multi { token_dim: dim }
        }
        value if value.starts_with("one-hot:") || value.starts_with("one_hot:") => {
            checked_dense(kind, dim, parse_dim(value)?)?
        }
        value if value.starts_with("sparse-keywords:") || value.starts_with("sparse_keywords:") => {
            let parsed = parse_dim(value)?;
            checked_match(kind, dim, parsed)?;
            SlotShape::Sparse(parsed)
        }
        value
            if value.starts_with("token-hash:")
                || value.starts_with("token_hash:")
                || value.starts_with("multi-hash:")
                || value.starts_with("multi_hash:") =>
        {
            let parsed = parse_dim(value)?;
            checked_match(kind, dim, parsed)?;
            SlotShape::Multi { token_dim: parsed }
        }
        other => {
            return Err(config_invalid(format!(
                "unsupported algorithmic lens kind {other}"
            )));
        }
    };
    Ok(shape)
}

fn checked_dense(kind: &str, got: u32, expected: u32) -> Result<SlotShape> {
    checked_match(kind, got, expected)?;
    Ok(SlotShape::Dense(expected))
}

fn checked_match(kind: &str, got: u32, expected: u32) -> Result<()> {
    if got == expected {
        return Ok(());
    }
    Err(config_invalid(format!(
        "algorithmic lens {kind} dim {got} != expected {expected}"
    )))
}

fn parse_dim(kind: &str) -> Result<u32> {
    kind.split_once(':')
        .and_then(|(_, dim)| dim.parse::<u32>().ok())
        .filter(|dim| *dim > 0)
        .ok_or_else(|| config_invalid(format!("invalid algorithmic dim in {kind}")))
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CONFIG_INVALID,
        message: message.into(),
        remediation: "fix the lensforge manifest or regenerated artifacts",
    }
}
