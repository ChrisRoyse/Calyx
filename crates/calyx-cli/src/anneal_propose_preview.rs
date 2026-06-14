use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_anneal::{DeficitMap, describe, synthesize};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    VaultId,
};
use serde::Deserialize;
use serde_json::json;

const CALYX_ASTER_CF_UNAVAILABLE: &str = "CALYX_ASTER_CF_UNAVAILABLE";

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = ProposePreviewRequest::parse(args)?;
    let deficit_bytes = read_bytes(&request.deficit, "deficit")?;
    let corpus_bytes = read_bytes(&request.corpus, "corpus")?;
    let deficit = serde_json::from_slice::<DeficitMap>(&deficit_bytes).map_err(|error| {
        format!(
            "{CALYX_ASTER_CF_UNAVAILABLE}: parse deficit fixture {}: {error}",
            request.deficit.display()
        )
    })?;
    let corpus_rows = serde_json::from_slice::<Vec<CorpusRow>>(&corpus_bytes).map_err(|error| {
        format!(
            "{CALYX_ASTER_CF_UNAVAILABLE}: parse corpus fixture {}: {error}",
            request.corpus.display()
        )
    })?;
    let corpus = corpus_rows
        .into_iter()
        .enumerate()
        .map(|(idx, row)| row.into_constellation(idx))
        .collect::<Result<Vec<_>, _>>()?;
    let top_anchor = deficit
        .top_gaps
        .first()
        .map(|gap| gap.anchor_class.as_str())
        .unwrap_or("");
    if !top_anchor.is_empty() && top_anchor != request.anchor {
        return Err(format!(
            "CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT: requested anchor {} does not match top gap {}",
            request.anchor, top_anchor
        )
        .into());
    }
    let candidate = synthesize(&deficit, &corpus).map_err(format_calyx_error)?;
    let readback = json!({
        "source_of_truth": "deficit and corpus fixture bytes read from paths; candidate recomputed by calyx anneal propose-preview",
        "anchor": request.anchor,
        "deficit_path": request.deficit.display().to_string(),
        "deficit_len": deficit_bytes.len(),
        "deficit_blake3": blake3::hash(&deficit_bytes).to_hex().to_string(),
        "corpus_path": request.corpus.display().to_string(),
        "corpus_len": corpus_bytes.len(),
        "corpus_blake3": blake3::hash(&corpus_bytes).to_hex().to_string(),
        "corpus_count": corpus.len(),
        "description": describe(&candidate),
        "candidate": candidate,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct ProposePreviewRequest {
    anchor: String,
    deficit: PathBuf,
    corpus: PathBuf,
}

impl ProposePreviewRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut anchor = None;
        let mut deficit = None;
        let mut corpus = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--anchor" => {
                    anchor = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--deficit" => {
                    deficit = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--corpus" => {
                    corpus = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                other => return Err(format!("unknown propose-preview arg: {other}")),
            }
        }
        Ok(Self {
            anchor: anchor.ok_or_else(|| "propose-preview requires --anchor".to_string())?,
            deficit: deficit.ok_or_else(|| "propose-preview requires --deficit".to_string())?,
            corpus: corpus.ok_or_else(|| "propose-preview requires --corpus".to_string())?,
        })
    }
}

#[derive(Deserialize)]
struct CorpusRow {
    cx_id: CxId,
    created_at: u64,
    modality: Modality,
    #[serde(default)]
    scalars: BTreeMap<String, f64>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

impl CorpusRow {
    fn into_constellation(self, idx: usize) -> Result<Constellation, String> {
        let id_byte = u8::try_from((idx % 251) + 1).map_err(|error| error.to_string())?;
        Ok(Constellation {
            cx_id: self.cx_id,
            vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV"
                .parse::<VaultId>()
                .map_err(|error| error.to_string())?,
            panel_version: 1,
            created_at: self.created_at,
            input_ref: InputRef {
                hash: [id_byte; 32],
                pointer: None,
                redacted: false,
            },
            modality: self.modality,
            slots: BTreeMap::new(),
            scalars: self.scalars,
            metadata: self.metadata,
            anchors: vec![Anchor {
                kind: AnchorKind::Label("propose_preview".to_string()),
                value: AnchorValue::Enum("fixture".to_string()),
                source: "issue419".to_string(),
                observed_at: self.created_at,
                confidence: 1.0,
            }],
            provenance: LedgerRef {
                seq: idx as u64,
                hash: [id_byte; 32],
            },
            flags: CxFlags::default(),
        })
    }
}

fn read_bytes(path: &PathBuf, label: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| {
        format!(
            "{CALYX_ASTER_CF_UNAVAILABLE}: read {label} fixture {}: {error}",
            path.display()
        )
    })
}

fn format_calyx_error(error: calyx_core::CalyxError) -> String {
    format!("{}: {} ({})", error.code, error.message, error.remediation)
}
