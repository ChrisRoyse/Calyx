use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use calyx_core::{
    AnchorKind, CalyxError, CxId, Input, Lens, LensId, Modality, SlotId, SlotShape, SlotVector,
};
use calyx_ward::{
    CalibrationMeta, GenerateInput, GenerateOutput, GuardId, GuardPolicy, GuardProfile,
    GuardVerdict, IdentityProfile, IdentitySlotConfig, NoveltyAction, NoveltyRecord,
    SlotCalibrationMeta, SlotVerdict, SpeakerProviderPolicy, VaultSink, WardError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(super) const DEFAULT_IDENTITY_DIR: &str = "/var/lib/calyx/data/identity_fsv";
pub(super) const DEFAULT_SPEAKER_FIXTURE: &str = "speaker_tts_espeak_ng_20260609_v2";
pub(super) const DEFAULT_WAVLM_MODEL_PATH: &str =
    "/var/lib/calyx/models/wavlm/wavlm-base-plus-sv.onnx";
pub(super) const CLOCK_TS: u64 = 274_000;
pub(super) const MIN_IN_REGION: usize = 20;
pub(super) const MIN_CROSS: usize = 5;
const DEFAULT_PH37_ROOT: &str = "/var/lib/calyx/data/fsv-issue263-ph37-t06-20260609-4cde3b7";
const DEFAULT_PH38_ROOT: &str =
    "/var/lib/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995";
const DEFAULT_PH39_STYLE_ROOT: &str =
    "/var/lib/calyx/data/fsv-issue273-ph39-t05-20260609-8d2572b-ort126-sm120";

#[derive(Clone, Debug, Deserialize)]
pub(super) struct SpeakerFixtureSpec {
    pub(super) guard_id: String,
    pub(super) panel_version: u32,
    pub(super) domain: String,
    pub(super) speaker_slot: u16,
    pub(super) tau: f32,
    pub(super) target_mean_cos: f32,
    pub(super) target_voice: String,
    pub(super) cross_voice: String,
    pub(super) sample_rate: u32,
    pub(super) matched_speaker_file: String,
    pub(super) in_region_dir: String,
    pub(super) cross_speaker_dir: String,
    pub(super) source: String,
    pub(super) espeak_version: String,
    #[serde(default)]
    pub(super) items: Vec<SpeakerFixtureItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct SpeakerFixtureItem {
    name: String,
    pcm_f32le: String,
    sample_rate: u32,
    samples: usize,
    sha256: String,
    text: String,
    voice: String,
    wav: String,
}

#[derive(Clone, Debug)]
pub(super) struct FileVault {
    root: PathBuf,
}

impl FileVault {
    pub(super) fn new(root: PathBuf) -> Self {
        fs::create_dir_all(&root).expect("create speaker novelty vault root");
        Self { root }
    }
}

impl VaultSink for FileVault {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError> {
        fs::write(
            self.root.join(format!("{}.json", record.novel_id)),
            serde_json::to_vec_pretty(record).expect("serialize novelty record"),
        )
        .map_err(|error| WardError::NoveltySink {
            reason: error.to_string(),
        })
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        collect_files(&self.root)
            .into_iter()
            .map(|path| {
                serde_json::from_slice(&fs::read(&path).map_err(novelty_io)?).map_err(|error| {
                    WardError::NoveltySink {
                        reason: error.to_string(),
                    }
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(super) struct UnusedStyleLens;

impl Lens for UnusedStyleLens {
    fn id(&self) -> LensId {
        LensId::from_bytes([0x74; 16])
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, _input: &Input) -> calyx_core::Result<SlotVector> {
        Err(CalyxError {
            code: "CALYX_WARD_UNUSED_STYLE_LENS",
            message: "speaker-only identity profile must not call style lens".to_string(),
            remediation: "check identity slot anchor kinds before calling guard_generate",
        })
    }
}

pub(super) fn speaker_identity_profile(
    spec: &SpeakerFixtureSpec,
    speaker_slot: SlotId,
    matched_speaker: Vec<f32>,
    profile_path: &Path,
) -> IdentityProfile {
    assert!(spec.tau.is_finite() && (0.0..=1.0).contains(&spec.tau));
    let guard_id = spec.guard_id.parse::<GuardId>().expect("valid guard id");
    let corpus_hash = sha256_file(profile_path);
    let per_slot = BTreeMap::from([(
        speaker_slot,
        SlotCalibrationMeta {
            corpus_hash,
            estimator: "identity_speaker_fixture_tau_manual_v1".to_string(),
            far: 0.01,
            frr: 0.0,
            confidence: 0.99,
            ts: CLOCK_TS as i64,
        },
    )]);
    IdentityProfile::new(
        GuardProfile {
            guard_id,
            panel_version: u64::from(spec.panel_version),
            domain: spec.domain.clone(),
            tau: BTreeMap::from([(speaker_slot, spec.tau)]),
            required_slots: vec![speaker_slot],
            policy: GuardPolicy::AllRequired,
            calibration: Some(CalibrationMeta {
                corpus_hash,
                estimator: "identity_speaker_fixture_tau_manual_v1".to_string(),
                far: 0.01,
                frr: 0.0,
                confidence: 0.99,
                ts: CLOCK_TS as i64,
                per_slot,
            }),
            novelty_action: NoveltyAction::RejectClosed,
        },
        vec![IdentitySlotConfig {
            slot_id: speaker_slot,
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: None,
        }],
        BTreeMap::from([(speaker_slot, matched_speaker)]),
    )
    .expect("valid speaker identity profile")
}

pub(super) fn speaker_input(audio: Vec<f32>, sample_rate: u32, seed: u8) -> GenerateInput {
    let mut bytes = [0_u8; 16];
    bytes[0] = seed;
    GenerateInput {
        candidate_audio: Some(audio),
        candidate_text: None,
        sample_rate,
        matched_cx_id: CxId::from_bytes(bytes),
    }
}

pub(super) fn accepted_slot(output: &GenerateOutput, slot: SlotId) -> SlotVerdict {
    let GenerateOutput::Accepted {
        verdict,
        provenance_tag,
        ..
    } = output
    else {
        panic!("expected accepted speaker output");
    };
    assert_eq!(provenance_tag, calyx_ward::GUARDED_PASS_TAG);
    assert!(verdict.overall_pass);
    let slot = slot_verdict(verdict, slot).clone();
    assert!(slot.pass);
    assert!(slot.cos >= slot.tau);
    slot
}

pub(super) fn rejected_verdict(output: &GenerateOutput) -> &GuardVerdict {
    let GenerateOutput::Rejected { verdict, .. } = output else {
        panic!("expected rejected cross-speaker output");
    };
    verdict
}

pub(super) fn slot_verdict(verdict: &GuardVerdict, slot: SlotId) -> &SlotVerdict {
    verdict
        .per_slot
        .iter()
        .find(|entry| entry.slot == slot)
        .expect("speaker slot verdict")
}

pub(super) fn write_stage8_summary(root: &Path, mean_cos: f32, target: f32) -> PathBuf {
    let ph37_root = env_path("CALYX_WARD_PH37_FSV_ROOT", DEFAULT_PH37_ROOT);
    let ph38_root = env_path("CALYX_WARD_PH38_FSV_ROOT", DEFAULT_PH38_ROOT);
    let style_root = env_path("CALYX_WARD_PH39_STYLE_FSV_ROOT", DEFAULT_PH39_STYLE_ROOT);
    let ph37 = read_json_value(&ph37_root.join("average-attack-verdict.json"));
    let ph38 = read_json_value(&ph38_root.join("heldout-block-rate.json"));
    let style = read_json_value(&style_root.join("injection-quarantine-readback.json"));
    let ph37_pass = ph37["average_would_pass"].as_bool() == Some(true)
        && ph37["verdict"]["overall_pass"].as_bool() == Some(false);
    let block_rate = value_f32(&ph38, "block_rate");
    let required_block_rate = value_f32(&ph38, "required_block_rate");
    let achieved_far = value_f32(&ph38, "achieved_far");
    let target_far = value_f32(&ph38, "target_far");
    let ph38_pass = block_rate >= required_block_rate && achieved_far <= target_far;
    let record = &style["output"]["Novel"]["record"];
    let style_pass = record["status"].as_str() == Some("Quarantined")
        && record["action_taken"].as_str() == Some("Quarantine");
    let speaker_pass = mean_cos >= target;
    let stage_pass = ph37_pass && ph38_pass && style_pass && speaker_pass;
    assert!(stage_pass, "Stage 8 summary did not pass");
    write_json(
        root,
        "stage8-summary-readback.json",
        &json!({
            "ph37_no_flatten_gate": {
                "source": ph37_root,
                "average_would_pass": ph37["average_would_pass"],
                "overall_pass": ph37["verdict"]["overall_pass"],
                "pass": ph37_pass,
            },
            "ph38_injection_block": {
                "source": ph38_root,
                "heldout_block_rate": block_rate,
                "required_block_rate": required_block_rate,
                "achieved_far": achieved_far,
                "target_far": target_far,
                "pass": ph38_pass,
            },
            "ph39_speaker_similarity": {
                "mean": mean_cos,
                "target": target,
                "pass": speaker_pass,
            },
            "ph39_style_quarantine": {
                "source": style_root,
                "status": record["status"],
                "action_taken": record["action_taken"],
                "pass": style_pass,
            },
            "stage8_ward_exit": stage_pass,
        }),
    )
}

pub(super) fn write_edge_readbacks(root: &Path) {
    let empty_dir = root.join("edge-empty-tts");
    fs::create_dir_all(&empty_dir).expect("create empty edge dir");
    let empty_error = pcm_file_paths(&empty_dir, 1).expect_err("empty edge must fail");
    let malformed = root.join("edge-malformed.f32le");
    fs::write(&malformed, [0_u8, 1, 2]).expect("write malformed edge");
    let malformed_error = read_pcm_f32le(&malformed).expect_err("malformed edge must fail");
    let nan = root.join("edge-nan.f32le");
    fs::write(&nan, f32::NAN.to_le_bytes()).expect("write nan edge");
    let nan_error = read_pcm_f32le(&nan).expect_err("nan edge must fail");
    write_json(
        root,
        "edge-failclosed-readback.json",
        &json!({
            "empty_tts_dir_before_count": 0,
            "empty_tts_dir_after_error": empty_error,
            "malformed_f32le_error": malformed_error,
            "nonfinite_audio_error": nan_error,
        }),
    );
}

pub(super) fn speaker_provider_policy() -> SpeakerProviderPolicy {
    match env::var("CALYX_WARD_SPEAKER_PROVIDER").as_deref() {
        Ok("cuda") => SpeakerProviderPolicy::CudaFailLoud,
        _ => SpeakerProviderPolicy::CpuExplicit,
    }
}

pub(super) fn resolve_fixture_path(identity_dir: &Path, fixture_dir: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }
    let under_fixture = fixture_dir.join(&path);
    if under_fixture.exists() {
        under_fixture
    } else {
        identity_dir.join(path)
    }
}

pub(super) fn pcm_file_paths(dir: &Path, min_count: usize) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Err(format!("missing pcm fixture dir {}", dir.display()));
    }
    let mut paths = fs::read_dir(dir)
        .map_err(|error| format!("read {}: {error}", dir.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("f32le"));
    paths.sort();
    if paths.len() < min_count {
        return Err(format!(
            "{} has {} f32le samples, need at least {min_count}",
            dir.display(),
            paths.len()
        ));
    }
    Ok(paths)
}

pub(super) fn read_pcm_f32le(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "{} must be non-empty little-endian f32 PCM",
            path.display()
        ));
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(format!("{} contains NaN or Inf", path.display()));
    }
    Ok(values)
}

pub(super) fn required_path_env(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is required"))
}

pub(super) fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}

pub(super) fn assert_empty_or_absent(path: &Path) {
    if path.exists() {
        assert!(
            fs::read_dir(path).expect("read FSV root").next().is_none(),
            "FSV root must be absent or empty before trigger: {}",
            path.display()
        );
    }
}

pub(super) fn read_json<T>(path: &Path) -> T
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
        panic!("read json fixture {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse json fixture {}: {error}", path.display()))
}

pub(super) fn write_json<T: Serialize>(root: &Path, name: &str, value: &T) -> PathBuf {
    let path = root.join(name);
    let file = File::create(&path).expect("create readback json");
    serde_json::to_writer_pretty(file, value).expect("write readback json");
    path
}

pub(super) fn write_manifest(root: &Path, primary_files: &[PathBuf]) {
    let mut files = primary_files.to_vec();
    files.push(root.join("edge-failclosed-readback.json"));
    files.extend(collect_files(&root.join("cross-reject-records")));
    files.sort();
    let manifest = files
        .iter()
        .map(|path| {
            format!(
                "{}  {}",
                sha256_file_hex(path),
                path.strip_prefix(root).unwrap_or(path).display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("SHA256SUMS.txt"), format!("{manifest}\n")).expect("write manifest");
}

fn read_json_value(path: &Path) -> Value {
    read_json(path)
}

fn collect_files(root: &Path) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("read files") {
        let path = entry.expect("read entry").path();
        if path.is_dir() {
            files.extend(collect_files(&path));
        } else if path.is_file() {
            files.push(path);
        }
    }
    files
}

fn sha256_file(path: &Path) -> [u8; 32] {
    let mut file = File::open(path).unwrap_or_else(|error| {
        panic!("open {} for sha256: {error}", path.display());
    });
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).expect("read sha256 input");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}

pub(super) fn sha256_file_hex(path: &Path) -> String {
    hex(&sha256_file(path))
}

pub(super) fn cosine(left: &[f32], right: &[f32]) -> f32 {
    assert_eq!(left.len(), right.len());
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

pub(super) fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

pub(super) fn norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

pub(super) fn prefix(values: &[f32], count: usize) -> Vec<f32> {
    values.iter().take(count).copied().collect()
}

fn value_f32(value: &Value, key: &str) -> f32 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("missing numeric key {key}")) as f32
}

fn novelty_io(error: std::io::Error) -> WardError {
    WardError::NoveltySink {
        reason: error.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
