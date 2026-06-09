use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_core::{
    AnchorKind, CalyxError, CxId, FixedClock, Input, Lens, LensId, Modality, SlotId, SlotShape,
    SlotVector,
};
use calyx_ledger::{LedgerEntry, LedgerRow};
use calyx_ward::{
    CalibrationMeta, GenerateInput, GuardId, GuardPolicy, GuardProfile, IdentityProfile,
    IdentitySlotConfig, NoveltyAction, NoveltyHandler, NoveltyRecord, SlotCalibrationMeta,
    VaultSink, WardError,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";
pub const SPEAKER_SLOT: SlotId = SlotId::new(8);
pub const STYLE_SLOT: SlotId = SlotId::new(9);

#[derive(Clone, Debug, Default)]
pub struct MemorySink {
    records: Arc<Mutex<Vec<NoveltyRecord>>>,
}

impl VaultSink for MemorySink {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        Ok(self.records.lock().unwrap().clone())
    }
}

#[derive(Clone, Debug)]
pub struct FileSink {
    root: PathBuf,
}

impl FileSink {
    pub fn new(root: PathBuf) -> Self {
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl VaultSink for FileSink {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError> {
        let path = self.root.join(format!("{}.json", record.novel_id));
        fs::write(path, serde_json::to_vec_pretty(record).unwrap()).map_err(|error| {
            WardError::NoveltySink {
                reason: error.to_string(),
            }
        })
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| WardError::NoveltySink {
            reason: error.to_string(),
        })? {
            let path = entry
                .map_err(|error| WardError::NoveltySink {
                    reason: error.to_string(),
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                records.push(serde_json::from_slice(&fs::read(path).unwrap()).unwrap());
            }
        }
        Ok(records)
    }
}

pub struct FailingSink;

impl VaultSink for FailingSink {
    fn write_novel(&self, _record: &NoveltyRecord) -> Result<(), WardError> {
        Err(WardError::NoveltySink {
            reason: "synthetic write failure".to_string(),
        })
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug)]
pub struct MockLens {
    id: LensId,
    modality: Modality,
    vector: Vec<f32>,
    calls: Arc<AtomicUsize>,
}

impl MockLens {
    pub fn audio(vector: Vec<f32>) -> Self {
        Self::new(Modality::Audio, vector, [8; 16])
    }

    pub fn text(vector: Vec<f32>) -> Self {
        Self::new(Modality::Text, vector, [9; 16])
    }

    fn new(modality: Modality, vector: Vec<f32>, id: [u8; 16]) -> Self {
        Self {
            id: LensId::from_bytes(id),
            modality,
            vector,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Lens for MockLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.vector.len() as u32)
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> calyx_core::Result<SlotVector> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if input.modality != self.modality {
            return Err(CalyxError::lens_dim_mismatch("mock modality mismatch"));
        }
        Ok(SlotVector::Dense {
            dim: self.vector.len() as u32,
            data: self.vector.clone(),
        })
    }
}

pub fn identity_profile(action: NoveltyAction, calibrated: bool) -> IdentityProfile {
    let mut tau = BTreeMap::new();
    tau.insert(SPEAKER_SLOT, 0.70);
    tau.insert(STYLE_SLOT, 0.70);
    IdentityProfile::new(
        GuardProfile {
            guard_id: guard_id(),
            panel_version: 42,
            domain: "synthetic-generation".to_string(),
            tau,
            required_slots: vec![SPEAKER_SLOT, STYLE_SLOT],
            policy: GuardPolicy::AllRequired,
            calibration: calibrated.then(calibration_meta),
            novelty_action: action,
        },
        vec![
            IdentitySlotConfig {
                slot_id: SPEAKER_SLOT,
                anchor_kind: AnchorKind::SpeakerMatch,
                tau_override: None,
            },
            IdentitySlotConfig {
                slot_id: STYLE_SLOT,
                anchor_kind: AnchorKind::StyleHold,
                tau_override: None,
            },
        ],
        slot_vectors(&[(SPEAKER_SLOT, base_vec()), (STYLE_SLOT, base_vec())]),
    )
    .unwrap()
}

pub fn empty_profile() -> IdentityProfile {
    IdentityProfile::new(
        GuardProfile {
            guard_id: guard_id(),
            panel_version: 42,
            domain: "empty".to_string(),
            tau: BTreeMap::new(),
            required_slots: Vec::new(),
            policy: GuardPolicy::AllRequired,
            calibration: None,
            novelty_action: NoveltyAction::NewRegion,
        },
        Vec::new(),
        BTreeMap::new(),
    )
    .unwrap()
}

fn calibration_meta() -> CalibrationMeta {
    let mut per_slot = BTreeMap::new();
    for slot in [SPEAKER_SLOT, STYLE_SLOT] {
        per_slot.insert(
            slot,
            SlotCalibrationMeta {
                corpus_hash: [2; 32],
                estimator: "synthetic".to_string(),
                far: 0.01,
                frr: 0.0,
                confidence: 0.99,
                ts: 27_200,
            },
        );
    }
    CalibrationMeta {
        corpus_hash: [1; 32],
        estimator: "synthetic".to_string(),
        far: 0.01,
        frr: 0.0,
        confidence: 0.99,
        ts: 27_200,
        per_slot,
    }
}

pub fn generate_input(audio: bool, text: bool) -> GenerateInput {
    GenerateInput {
        candidate_audio: audio.then(|| vec![0.1, 0.2, 0.3, 0.4]),
        candidate_text: text.then(|| "measured candidate style".to_string()),
        sample_rate: 16_000,
        matched_cx_id: CxId::from_bytes([1; 16]),
    }
}

pub fn handler_for<S>(sink: S) -> NoveltyHandler
where
    S: VaultSink + 'static,
{
    NoveltyHandler::new(Arc::new(sink), Arc::new(FixedClock::new(27_200)))
}

fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> BTreeMap<SlotId, Vec<f32>> {
    entries.iter().cloned().collect()
}

pub fn base_vec() -> Vec<f32> {
    vec![1.0, 0.0]
}

pub fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().unwrap()
}

pub fn row_readback(rows: &[LedgerRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| json!({"seq": row.seq, "bytes_hex": hex(&row.bytes)}))
        .collect()
}

pub fn entry_readback(entries: &[LedgerEntry]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "subject": entry.subject,
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
                "entry_hash": hex(&entry.entry_hash),
            })
        })
        .collect()
}

pub fn write_json(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

pub fn write_manifest(root: &Path, files: &[PathBuf]) {
    let mut manifest = File::create(root.join("SHA256SUMS.txt")).unwrap();
    for path in files {
        writeln!(
            manifest,
            "{}  {}",
            sha256_file_hex(path),
            path.file_name().unwrap().to_string_lossy()
        )
        .unwrap();
    }
}

fn sha256_file_hex(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
