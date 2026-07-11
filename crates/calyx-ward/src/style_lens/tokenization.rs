use crate::error::WardError;

use super::STYLE_MAX_TOKENS;

pub(super) fn model_inputs(
    token_ids: &[u32],
    attention_mask: &[u32],
) -> Result<(Vec<i64>, Vec<i64>), WardError> {
    if token_ids.len() != attention_mask.len() {
        return Err(WardError::InvalidInput {
            reason: format!(
                "style tokenizer emitted {} token IDs but {} attention values",
                token_ids.len(),
                attention_mask.len()
            ),
        });
    }

    let len = token_ids.len();
    if len == 0 {
        return Err(WardError::InvalidInput {
            reason: "style tokenizer emitted no tokens".to_string(),
        });
    }
    if len > STYLE_MAX_TOKENS {
        return Err(WardError::InvalidInput {
            reason: format!(
                "style input encoded to {len} tokens, exceeding the complete-coverage limit of \
                 {STYLE_MAX_TOKENS}; split the input into segments of at most {STYLE_MAX_TOKENS} \
                 tokens before style measurement"
            ),
        });
    }

    let ids = token_ids
        .iter()
        .map(|value| i64::from(*value))
        .collect::<Vec<_>>();
    let attention = attention_mask
        .iter()
        .map(|value| i64::from(*value))
        .collect::<Vec<_>>();
    Ok((ids, attention))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_model_window_is_preserved() {
        let ids = vec![7; STYLE_MAX_TOKENS];
        let attention = vec![1; STYLE_MAX_TOKENS];

        let (actual_ids, actual_attention) = model_inputs(&ids, &attention).unwrap();

        assert_eq!(actual_ids.len(), STYLE_MAX_TOKENS);
        assert_eq!(actual_attention.len(), STYLE_MAX_TOKENS);
    }

    #[test]
    fn overlong_token_sequence_is_rejected_instead_of_truncated() {
        let ids = vec![7; STYLE_MAX_TOKENS + 1];
        let attention = vec![1; STYLE_MAX_TOKENS + 1];

        let error = model_inputs(&ids, &attention)
            .expect_err("style measurement must cover every encoded token or fail closed");

        assert_eq!(error.code(), "CALYX_WARD_INVALID_INPUT");
        assert!(error.to_string().contains("513"));
        assert!(error.to_string().contains("512"));
    }

    #[test]
    fn mismatched_tokenizer_outputs_are_rejected() {
        let error = model_inputs(&[7, 8], &[1])
            .expect_err("token IDs and attention values must describe the same input");

        assert_eq!(error.code(), "CALYX_WARD_INVALID_INPUT");
        assert!(error.to_string().contains("2 token IDs"));
        assert!(error.to_string().contains("1 attention values"));
    }
}
