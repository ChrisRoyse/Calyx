use std::collections::BTreeMap;

use calyx_core::{CalyxError, Input, Lens, Result};
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use serde_json::Value;
use tokenizers::Tokenizer;

use crate::runtime::common::{DEFAULT_MAX_TOKENS, text_from_input};

use super::config_invalid;

pub(in crate::runtime::onnx) struct TokenBatch {
    pub(in crate::runtime::onnx) batch: usize,
    pub(in crate::runtime::onnx) seq: usize,
    pub(in crate::runtime::onnx) ids: Vec<i64>,
    pub(in crate::runtime::onnx) mask: Vec<i64>,
    pub(in crate::runtime::onnx) indices: Vec<usize>,
}

struct EncodedInput {
    index: usize,
    seq: usize,
    ids: Vec<i64>,
    mask: Vec<i64>,
}

pub(in crate::runtime::onnx) fn max_tokens_from_config(value: &Value) -> Result<usize> {
    let max_tokens = value
        .get("max_position_embeddings")
        .or_else(|| value.get("max_sequence_length"))
        .or_else(|| value.get("model_max_length"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .min(DEFAULT_MAX_TOKENS);
    if max_tokens == 0 {
        return Err(config_invalid("custom ONNX max token count must be > 0"));
    }
    Ok(max_tokens)
}

pub(in crate::runtime::onnx) fn token_batches(
    tokenizer: &Tokenizer,
    lens: &dyn Lens,
    inputs: &[Input],
    max_tokens: usize,
) -> Result<Vec<TokenBatch>> {
    let mut groups: BTreeMap<usize, Vec<EncodedInput>> = BTreeMap::new();
    for (index, input) in inputs.iter().enumerate() {
        let text = text_from_input(lens, input)?;
        let encoded = tokenizer
            .encode(text, true)
            .map_err(|err| config_invalid(format!("tokenizer encode failed: {err}")))?;
        let (ids, mask) = token_inputs(&encoded, max_tokens);
        let seq = stable_seq_len(ids.len(), max_tokens)?;
        groups.entry(seq).or_default().push(EncodedInput {
            index,
            seq,
            ids,
            mask,
        });
    }
    groups
        .into_values()
        .map(|group| build_batch(&group))
        .collect()
}

fn stable_seq_len(len: usize, max_tokens: usize) -> Result<usize> {
    let max_tokens = max_tokens.max(1);
    let len = len.clamp(1, max_tokens);
    let bucket = len.next_power_of_two().min(max_tokens);
    if bucket < len {
        return Err(CalyxError::lens_dim_mismatch(
            "custom ONNX stable sequence bucket is shorter than tokenized input",
        ));
    }
    Ok(bucket)
}

fn build_batch(encoded: &[EncodedInput]) -> Result<TokenBatch> {
    let batch = encoded.len();
    let seq = encoded
        .first()
        .map(|input| input.seq)
        .ok_or_else(|| CalyxError::lens_dim_mismatch("custom ONNX token batch is empty"))?;
    let mut flat_ids = Vec::with_capacity(batch * seq);
    let mut flat_mask = Vec::with_capacity(batch * seq);
    let mut indices = Vec::with_capacity(batch);
    for item in encoded {
        if item.seq != seq {
            return Err(CalyxError::lens_dim_mismatch(
                "custom ONNX token batch mixed sequence buckets",
            ));
        }
        indices.push(item.index);
        for index in 0..seq {
            flat_ids.push(item.ids.get(index).copied().unwrap_or(0));
            flat_mask.push(item.mask.get(index).copied().unwrap_or(0));
        }
    }
    Ok(TokenBatch {
        batch,
        seq,
        ids: flat_ids,
        mask: flat_mask,
        indices,
    })
}

fn token_inputs(encoding: &tokenizers::Encoding, max_tokens: usize) -> (Vec<i64>, Vec<i64>) {
    let mut ids = encoding
        .get_ids()
        .iter()
        .take(max_tokens)
        .map(|id| i64::from(*id))
        .collect::<Vec<_>>();
    let mut mask = encoding
        .get_attention_mask()
        .iter()
        .take(max_tokens)
        .map(|value| i64::from(*value))
        .collect::<Vec<_>>();
    if ids.is_empty() {
        ids.push(0);
        mask.push(0);
    }
    if mask.len() != ids.len() {
        mask.resize(ids.len(), 1);
    }
    (ids, mask)
}

pub(in crate::runtime::onnx) fn session_inputs<'a>(
    session: &Session,
    batch: &TokenBatch,
) -> Result<Vec<(String, SessionInputValue<'a>)>> {
    let shape = vec![batch.batch as i64, batch.seq as i64];
    let mut values = Vec::with_capacity(session.inputs().len());
    for input in session.inputs() {
        let name = input.name();
        let tensor = if name.contains("token_type_ids") || name.contains("segment") {
            Tensor::from_array((shape.clone(), vec![0_i64; batch.ids.len()]))
        } else if name.contains("input_ids") || name.contains("token") {
            Tensor::from_array((shape.clone(), batch.ids.clone()))
        } else if name.contains("attention_mask") || name.contains("mask") {
            Tensor::from_array((shape.clone(), batch.mask.clone()))
        } else if name.contains("position_ids") || name.contains("position") {
            Tensor::from_array((shape.clone(), position_ids(batch)))
        } else {
            return Err(config_invalid(format!(
                "unsupported custom ONNX input {}",
                input.name()
            )));
        }
        .map_err(|err| config_invalid(format!("build ONNX tensor {} failed: {err}", name)))?;
        values.push((name.to_string(), SessionInputValue::from(tensor)));
    }
    Ok(values)
}

fn position_ids(batch: &TokenBatch) -> Vec<i64> {
    let mut out = Vec::with_capacity(batch.batch * batch.seq);
    for _ in 0..batch.batch {
        out.extend(0..batch.seq as i64);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_sequence_buckets_are_input_local() {
        assert_eq!(stable_seq_len(1, 512).unwrap(), 1);
        assert_eq!(stable_seq_len(9, 512).unwrap(), 16);
        assert_eq!(stable_seq_len(257, 512).unwrap(), 512);
        assert_eq!(stable_seq_len(700, 512).unwrap(), 512);
    }

    #[test]
    fn batch_builder_preserves_original_indices() {
        let rows = vec![
            EncodedInput {
                index: 3,
                seq: 4,
                ids: vec![1, 2],
                mask: vec![1, 1],
            },
            EncodedInput {
                index: 1,
                seq: 4,
                ids: vec![7],
                mask: vec![1],
            },
        ];
        let batch = build_batch(&rows).unwrap();

        assert_eq!(batch.indices, vec![3, 1]);
        assert_eq!(batch.ids, vec![1, 2, 0, 0, 7, 0, 0, 0]);
        assert_eq!(batch.mask, vec![1, 1, 0, 0, 1, 0, 0, 0]);
    }
}
