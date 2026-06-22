//! Support for the PH70 / issue #605 QQP+PAWS dedup intelligence FSV.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupResult, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
    contested_with_key, decode_contested_with, dedup_audit, ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use serde_json::json;

#[path = "dedup_fsv_io.rs"]
mod dedup_fsv_io;

pub(crate) use dedup_fsv_io::{write_blake3_sums, write_json};

pub const CONTENT_SLOT: u16 = 0;
pub const PANEL_VERSION: u32 = 70;
pub const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
pub const SALT: &[u8] = b"dedup-qqp-paws-fsv-issue605";

#[derive(Clone, Debug, PartialEq)]
pub struct PairRow {
    pub source: String,
    pub split: String,
    pub pair_id: String,
    pub label: u8,
    pub text_a: String,
    pub text_b: String,
}

/// Parses `dedup_fsv_pairs.tsv`; fails closed on any malformed row.
pub fn parse_pairs_tsv(text: &str) -> Result<Vec<PairRow>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 {
            return Err(format!(
                "line {} has {} fields != 8",
                index + 1,
                fields.len()
            ));
        }
        let label = match fields[3] {
            "0" => 0,
            "1" => 1,
            other => return Err(format!("line {} label {other:?} not 0/1", index + 1)),
        };
        if fields[6].is_empty() || fields[7].is_empty() {
            return Err(format!("line {} has an empty text side", index + 1));
        }
        rows.push(PairRow {
            source: fields[0].to_string(),
            split: fields[1].to_string(),
            pair_id: fields[2].to_string(),
            label,
            text_a: fields[6].to_string(),
            text_b: fields[7].to_string(),
        });
    }
    if rows.is_empty() {
        return Err("pairs TSV contained no data rows".to_string());
    }
    Ok(rows)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Confusion {
    pub tp: usize,
    pub fp: usize,
    pub fn_: usize,
    pub tn: usize,
}

impl Confusion {
    pub fn precision(&self) -> f64 {
        if self.tp + self.fp == 0 {
            return 0.0;
        }
        self.tp as f64 / (self.tp + self.fp) as f64
    }

    pub fn recall(&self) -> f64 {
        if self.tp + self.fn_ == 0 {
            return 0.0;
        }
        self.tp as f64 / (self.tp + self.fn_) as f64
    }

    pub fn observe(&mut self, merged: bool, label: u8) {
        match (merged, label) {
            (true, 1) => self.tp += 1,
            (true, 0) => self.fp += 1,
            (false, 1) => self.fn_ += 1,
            (false, 0) => self.tn += 1,
            _ => unreachable!("label is validated to 0/1"),
        }
    }
}

pub fn confusion_at_tau(cosines: &[(f32, u8)], tau: f32) -> Confusion {
    let mut confusion = Confusion::default();
    for (cos, label) in cosines {
        confusion.observe(*cos >= tau, *label);
    }
    confusion
}

/// Precision-first calibration: the smallest observed-cosine threshold whose
/// calibration precision meets `precision_floor` (maximises recall subject to
/// the floor). Returns `(tau, precision, recall)`.
pub fn calibrate_tau(cosines: &[(f32, u8)], precision_floor: f64) -> Option<(f32, f64, f64)> {
    let mut candidates: Vec<f32> = cosines.iter().map(|(cos, _)| *cos).collect();
    candidates.sort_by(|left, right| left.total_cmp(right));
    candidates.dedup();
    for tau in candidates {
        let confusion = confusion_at_tau(cosines, tau);
        if confusion.tp + confusion.fp > 0 && confusion.precision() >= precision_floor {
            return Some((tau, confusion.precision(), confusion.recall()));
        }
    }
    None
}

pub fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

pub fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}

pub fn tct_policy(tau: f32, action: DedupAction) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(CONTENT_SLOT)],
            TauStrategy::PerSlot(vec![(slot(CONTENT_SLOT), tau)]),
            action,
        )
        .expect("valid tct config"),
    )
}

pub fn durable_vault(dir: &Path, tau: f32, action: DedupAction) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        SALT.to_vec(),
        VaultOptions {
            dedup_policy: Some(tct_policy(tau, action)),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault")
}

pub fn text_input(text: &str, vector: Vec<f32>) -> IngestInput {
    let dim = u32::try_from(vector.len()).expect("vector dim fits u32");
    IngestInput::new(text.as_bytes().to_vec(), PANEL_VERSION, Modality::Text)
        .with_slot(slot(CONTENT_SLOT), SlotVector::Dense { dim, data: vector })
}

pub fn label_anchor(axis: &str, value: &str, source: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::Label(axis.to_string()),
        value: AnchorValue::Text(value.to_string()),
        source: source.to_string(),
        observed_at: 1,
        confidence: 1.0,
    }
}

/// Ingests one labeled pair through the real engine in a fresh durable vault.
/// Returns the second ingest's result (the dedup decision under test).
pub fn engine_pair_decision(
    vault_dir: &Path,
    tau: f32,
    vec_a: Vec<f32>,
    vec_b: Vec<f32>,
    pair: &PairRow,
    anchors: Option<(Anchor, Anchor)>,
) -> Result<(DedupResult, AsterVault), calyx_core::CalyxError> {
    let vault = durable_vault(vault_dir, tau, DedupAction::Collapse);
    let mut input_a = text_input(&pair.text_a, vec_a);
    let mut input_b = text_input(&pair.text_b, vec_b);
    if let Some((anchor_a, anchor_b)) = anchors {
        input_a = input_a.with_anchor(anchor_a);
        input_b = input_b.with_anchor(anchor_b);
    }
    ingest_at(&vault, &input_a, EpochSecs(100), None)?;
    let second = ingest_at(&vault, &input_b, EpochSecs(200), None)?;
    vault.flush()?;
    Ok((second, vault))
}

pub fn merged(result: &DedupResult) -> bool {
    matches!(
        result,
        DedupResult::DedupMerge { .. } | DedupResult::ExactDuplicate(_)
    )
}

/// Unit vector at an exact hand-computed cosine to `[1, 0]`.
pub fn vector_at_cos(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

/// Reads the contested_with rows and both Base CF rows back from the vault
/// bytes - the merge-block must be physically present, both regions intact.
pub fn contested_readback(
    vault: &AsterVault,
    dir: &Path,
    row: &PairRow,
    cos: f32,
    keep_vault: bool,
) -> serde_json::Value {
    let snapshot = vault.snapshot();
    let id_a = vault.cx_id_for_input(row.text_a.as_bytes(), PANEL_VERSION);
    let id_b = vault.cx_id_for_input(row.text_b.as_bytes(), PANEL_VERSION);
    let mut contested = Vec::new();
    for (this, other) in [(id_a, id_b), (id_b, id_a)] {
        let bytes = vault
            .read_cf_at(snapshot, ColumnFamily::Online, &contested_with_key(this))
            .expect("read contested row")
            .unwrap_or_else(|| panic!("contested_with row missing for {this}"));
        let decoded = decode_contested_with(&bytes).expect("decode contested row");
        assert_eq!(
            decoded.contested_with, other,
            "contested row must point at the other side"
        );
        contested.push(
            json!({"cx_id": this, "contested_with": decoded.contested_with,
            "reason": format!("{:?}", decoded.reason), "raw_len": bytes.len()}),
        );
    }
    for id in [id_a, id_b] {
        assert!(
            vault
                .read_cf_at(snapshot, ColumnFamily::Base, &base_key(id))
                .expect("read base row")
                .is_some(),
            "both constellations must remain as separate regions ({id})"
        );
    }
    let audit = dedup_audit(vault, id_a).expect("audit a");
    assert!(
        audit.merges.is_empty(),
        "blocked pair must have zero merges"
    );
    assert_eq!(audit.anchor_conflict_blocks, vec![id_b]);
    json!({
        "pair_id": row.pair_id,
        "cos": cos,
        "vault_dir": if keep_vault { Some(dir.display().to_string()) } else { None },
        "base_rows_present": [id_a, id_b],
        "contested": contested,
        "merges": 0,
    })
}

/// Builds the candidate constellation the same way ingest stages it, so the
/// decision engine can be driven directly to exercise the DPI limit.
pub fn probe_constellation(vault: &AsterVault, input: &IngestInput) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id: vault.cx_id_for_input(&input.raw_bytes, input.panel_version),
        vault_id: vault.vault_id(),
        panel_version: input.panel_version,
        created_at: 400,
        input_ref: calyx_core::InputRef {
            hash: *blake3::hash(&input.raw_bytes).as_bytes(),
            pointer: Some("synthetic://issue605/dpi-probe".to_string()),
            redacted: true,
        },
        modality: Modality::Text,
        slots: input.slots.clone(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: calyx_core::LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: calyx_core::CxFlags::default(),
    }
}

/// Mean cosine per label over `(cos, label)` pairs.
pub fn label_means(cosines: &[(f32, u8)]) -> BTreeMap<u8, f64> {
    let mut sums: BTreeMap<u8, (f64, usize)> = BTreeMap::new();
    for (cos, label) in cosines {
        let entry = sums.entry(*label).or_default();
        entry.0 += f64::from(*cos);
        entry.1 += 1;
    }
    sums.into_iter()
        .map(|(label, (sum, count))| (label, sum / count as f64))
        .collect()
}
