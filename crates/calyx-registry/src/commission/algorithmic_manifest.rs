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
        return learned_output_shape(runtime, dim);
    };
    let shape = match kind {
        "byte" | "byte-features" => checked_dense(kind, dim, 16)?,
        "ast-style" | "ast_style" => checked_dense(kind, dim, 8)?,
        "gdelt-cameo" | "gdelt_cameo" => checked_dense(kind, dim, 16)?,
        "gdelt-actor-geo" | "gdelt_actor_geo" => SlotShape::Sparse(checked_positive(kind, dim)?),
        "scalar" => checked_dense(kind, dim, 1)?,
        "sparse" | "sparse-keywords" | "sparse_keywords" => {
            SlotShape::Sparse(checked_positive(kind, dim)?)
        }
        "token-hash" | "token_hash" | "multi-hash" | "multi_hash" => SlotShape::Multi {
            token_dim: checked_positive(kind, dim)?,
        },
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

fn learned_output_shape(runtime: &str, dim: u32) -> Result<SlotShape> {
    match runtime {
        "fastembed-sparse" | "fastembed-bgem3-sparse" => {
            Ok(SlotShape::Sparse(checked_positive(runtime, dim)?))
        }
        "fastembed-bgem3-colbert" | "onnx-colbert" => Ok(SlotShape::Multi {
            token_dim: checked_positive(runtime, dim)?,
        }),
        _ => Ok(SlotShape::Dense(dim)),
    }
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

fn checked_positive(kind: &str, dim: u32) -> Result<u32> {
    if dim > 0 {
        return Ok(dim);
    }
    Err(config_invalid(format!(
        "algorithmic lens {kind} dim must be greater than zero"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdelt_algorithmic_shapes_are_explicit() {
        assert_eq!(
            output_shape("algorithmic:gdelt-cameo", 16).unwrap(),
            SlotShape::Dense(16)
        );
        assert_eq!(
            output_shape("algorithmic:gdelt-actor-geo", 512).unwrap(),
            SlotShape::Sparse(512)
        );
    }

    #[test]
    fn gdelt_cameo_rejects_wrong_manifest_dim() {
        let error = output_shape("algorithmic:gdelt-cameo", 8).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("gdelt-cameo dim 8"));
    }

    #[test]
    fn gdelt_actor_geo_rejects_zero_manifest_dim() {
        let error = output_shape("algorithmic:gdelt-actor-geo", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("gdelt-actor-geo dim must be"));
    }

    #[test]
    fn sparse_algorithmic_kinds_reject_zero_manifest_dim() {
        let error = output_shape("algorithmic:sparse-keywords", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("sparse-keywords dim must be"));
    }

    #[test]
    fn token_algorithmic_kinds_reject_zero_manifest_dim() {
        let error = output_shape("algorithmic:token-hash", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("token-hash dim must be"));
    }

    #[test]
    fn learned_special_runtimes_declare_non_dense_shapes() {
        assert_eq!(
            output_shape("fastembed-sparse", 30_522).unwrap(),
            SlotShape::Sparse(30_522)
        );
        assert_eq!(
            output_shape("fastembed-bgem3-sparse", 250_002).unwrap(),
            SlotShape::Sparse(250_002)
        );
        assert_eq!(
            output_shape("fastembed-bgem3-colbert", 1024).unwrap(),
            SlotShape::Multi { token_dim: 1024 }
        );
        assert_eq!(
            output_shape("onnx-colbert", 96).unwrap(),
            SlotShape::Multi { token_dim: 96 }
        );
    }
}
