use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality,
    SystemClock,
};
use serde::Deserialize;

use super::request::SoakRequest;
use crate::error::{CliError, CliResult};

const PANEL_VERSION: u32 = 70;
const INGEST_BATCH_ROWS: usize = 1_000;

#[derive(Clone)]
pub(super) struct CorpusStats {
    pub(crate) rows: usize,
    pub(crate) bytes: usize,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) corpus_hash: String,
}

#[derive(Deserialize)]
struct CorpusRow {
    row: u64,
    label: String,
    text: String,
    #[serde(default)]
    source: Option<String>,
}

pub(super) fn ingest_corpus(vault: &AsterVault, request: &SoakRequest) -> CliResult<CorpusStats> {
    let file = File::open(&request.corpus_jsonl).map_err(|error| {
        CliError::io(format!(
            "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE: open corpus {}: {error}",
            request.corpus_jsonl.display()
        ))
    })?;
    let mut stats = CorpusStats {
        rows: 0,
        bytes: 0,
        label_counts: BTreeMap::new(),
        corpus_hash: String::new(),
    };
    let mut hasher = blake3::Hasher::new();
    let clock = SystemClock;
    let mut batch = Vec::with_capacity(INGEST_BATCH_ROWS);
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| CliError::io(format!("read corpus JSONL: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let row: CorpusRow = serde_json::from_str(&line)
            .map_err(|error| CliError::runtime(format!("parse corpus JSONL: {error}")))?;
        if row.text.trim().is_empty() || row.label.trim().is_empty() {
            return Err(CliError::runtime(
                "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE: empty text or label row",
            ));
        }
        hasher.update(row.text.as_bytes());
        hasher.update(row.label.as_bytes());
        stats.rows += 1;
        stats.bytes += row.text.len();
        *stats.label_counts.entry(row.label.clone()).or_insert(0) += 1;
        batch.push(constellation_for(&clock, vault, &row));
        if batch.len() >= INGEST_BATCH_ROWS {
            flush_batch(vault, &mut batch)?;
        }
    }
    flush_batch(vault, &mut batch)?;
    if stats.rows < request.min_docs {
        return Err(CliError::runtime(format!(
            "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE: corpus rows {} below required {}",
            stats.rows, request.min_docs
        )));
    }
    stats.corpus_hash = hasher.finalize().to_hex().to_string();
    vault.flush()?;
    Ok(stats)
}

fn flush_batch(vault: &AsterVault, batch: &mut Vec<Constellation>) -> CliResult {
    if batch.is_empty() {
        return Ok(());
    }
    vault.put_batch(std::mem::take(batch))?;
    Ok(())
}

fn constellation_for(clock: &dyn Clock, vault: &AsterVault, row: &CorpusRow) -> Constellation {
    let input_hash = blake3::hash(row.text.as_bytes());
    let pointer = row
        .source
        .clone()
        .unwrap_or_else(|| format!("ag_news://train.parquet#row={}", row.row));
    let mut scalars = BTreeMap::new();
    scalars.insert("ph70_row".to_string(), row.row as f64);
    scalars.insert("ph70_text_bytes".to_string(), row.text.len() as f64);
    let mut metadata = BTreeMap::new();
    metadata.insert("dataset".to_string(), "ag_news".to_string());
    metadata.insert("row".to_string(), row.row.to_string());
    metadata.insert("label".to_string(), row.label.clone());
    metadata.insert("source".to_string(), pointer.clone());
    Constellation {
        cx_id: vault.cx_id_for_input(row.text.as_bytes(), PANEL_VERSION),
        vault_id: vault.vault_id(),
        panel_version: PANEL_VERSION,
        created_at: clock.now(),
        input_ref: InputRef {
            hash: *input_hash.as_bytes(),
            pointer: Some(pointer),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars,
        metadata,
        anchors: vec![Anchor {
            kind: AnchorKind::Label("ag_news_class".to_string()),
            value: AnchorValue::Enum(row.label.clone()),
            source: "PH69 ag_news train.parquet".to_string(),
            observed_at: clock.now(),
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}
