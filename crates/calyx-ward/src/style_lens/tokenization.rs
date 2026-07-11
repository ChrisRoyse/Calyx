use crate::error::WardError;

use super::STYLE_MAX_TOKENS;

pub(super) fn model_inputs(
    token_ids: &[u32],
    attention_mask: &[u32],
) -> Result<(Vec<i64>, Vec<i64>), WardError> {
    let len = token_ids.len().min(STYLE_MAX_TOKENS);
    if len == 0 {
        return Err(WardError::InvalidInput {
            reason: "style tokenizer emitted no tokens".to_string(),
        });
    }
    let ids = token_ids
        .iter()
        .take(len)
        .map(|value| i64::from(*value))
        .collect::<Vec<_>>();
    let attention = attention_mask
        .iter()
        .take(len)
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
}
