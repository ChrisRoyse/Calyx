use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::parse::validate_text;
use crate::error::{CliError, CliResult};

#[derive(Deserialize)]
struct BatchLine {
    text: String,
}

pub(super) fn read_batch_texts(path: &Path) -> CliResult<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let mut texts = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: BatchLine = serde_json::from_str(line).map_err(|err| {
            CliError::io(format!("batch JSONL line {} is invalid: {err}", index + 1))
        })?;
        validate_text(&parsed.text)?;
        texts.push(parsed.text);
    }
    Ok(texts)
}
