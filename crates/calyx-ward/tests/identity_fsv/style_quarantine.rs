use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_core::{
    AnchorKind, CalyxError, CxId, FixedClock, Input, Lens, LensId, Modality, SlotId, SlotShape,
    SlotVector,
};
use calyx_ward::{
    CalibrationMeta, DEFAULT_STYLE_MODEL_PATH, DEFAULT_STYLE_TOKENIZER_PATH, GUARDED_PASS_TAG,
    GenerateInput, GenerateOutput, GuardId, GuardPolicy, GuardProfile, IdentityProfile,
    IdentitySlotConfig, NoveltyAction, NoveltyHandler, NoveltyRecord, NoveltyStatus,
    SlotCalibrationMeta, StyleLens, StyleProviderPolicy, VaultSink, WardError, guard_generate,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_FIXTURE_DIR: &str = "/var/lib/calyx/data/identity_fsv";
const CLOCK_TS: u64 = 273_000;

#[derive(Clone, Debug, Deserialize)]
struct StyleFixtureSpec {
    guard_id: String,
    panel_version: u32,
    domain: String,
    style_slot: u16,
    tau: f32,
    in_persona_text_file: String,
    injection_text_file: String,
    borderline_text_file: String,
    injection_source_file: String,
}

#[derive(Clone, Debug)]
struct FileVault {
    root: PathBuf,
}

impl FileVault {
    fn new(root: PathBuf) -> Self {
        fs::create_dir_all(&root).expect("create novelty vault root");
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
        let mut paths = fs::read_dir(&self.root)
            .map_err(|error| WardError::NoveltySink {
                reason: error.to_string(),
            })?
            .map(|entry| {
                entry
                    .map_err(|error| WardError::NoveltySink {
                        reason: error.to_string(),
                    })
                    .map(|entry| entry.path())
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();

        paths
            .into_iter()
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .map(|path| {
                serde_json::from_slice(&fs::read(path).map_err(|error| WardError::NoveltySink {
                    reason: error.to_string(),
                })?)
                .map_err(|error| WardError::NoveltySink {
                    reason: error.to_string(),
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
struct UnusedSpeakerLens;

impl Lens for UnusedSpeakerLens {
    fn id(&self) -> LensId {
        LensId::from_bytes([0x73; 16])
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Audio
    }

    fn measure(&self, _input: &Input) -> calyx_core::Result<SlotVector> {
        Err(CalyxError {
            code: "CALYX_WARD_UNUSED_SPEAKER_LENS",
            message: "style-only identity profile must not call speaker lens".to_string(),
            remediation: "check identity slot anchor kinds before calling guard_generate",
        })
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_IDENTITY_FSV_DIR"]
fn issue273_identity_injection_quarantine_fsv_writes_readbacks() {
    let root = required_path_env("CALYX_WARD_IDENTITY_FSV_DIR");
    assert_empty_or_absent(&root);
    fs::create_dir_all(&root).expect("create FSV root");

    let fixture_dir = env_path("CALYX_WARD_IDENTITY_FIXTURE_DIR", DEFAULT_FIXTURE_DIR);
    let spec_path = fixture_dir.join("style_profile.json");
    let spec: StyleFixtureSpec = read_json(&spec_path);
    let style_slot = SlotId::new(spec.style_slot);
    let in_persona_path = fixture_dir.join(&spec.in_persona_text_file);
    let injection_path = fixture_dir.join(&spec.injection_text_file);
    let borderline_path = fixture_dir.join(&spec.borderline_text_file);
    let injection_source_path = fixture_dir.join(&spec.injection_source_file);
    let in_persona = read_text(&in_persona_path);
    let injection = read_text(&injection_path);
    let borderline = read_text(&borderline_path);
    let injection_source: Value = read_json(&injection_source_path);

    let model_path = env_path("CALYX_WARD_STYLE_MODEL", DEFAULT_STYLE_MODEL_PATH);
    let tokenizer_path = env_path("CALYX_WARD_STYLE_TOKENIZER", DEFAULT_STYLE_TOKENIZER_PATH);
    let style_lens = StyleLens::new_with_tokenizer_and_provider_policy(
        &model_path,
        &tokenizer_path,
        provider_policy(),
    )
    .expect("load real style lens");

    let matched_style = style_lens
        .embed_style(&in_persona)
        .expect("embed matched style");
    let profile = style_identity_profile(&spec, style_slot, matched_style.clone(), &spec_path);
    let vault = FileVault::new(root.join("novelty-records"));
    let handler = NoveltyHandler::new(Arc::new(vault.clone()), Arc::new(FixedClock::new(CLOCK_TS)));
    let speaker_lens = UnusedSpeakerLens;

    let fixture_readback = write_json(
        &root,
        "fixture-readback.json",
        &json!({
            "fixture_dir": fixture_dir,
            "style_profile_sha256": sha256_file_hex(&spec_path),
            "in_persona_sha256": sha256_file_hex(&in_persona_path),
            "injection_sha256": sha256_file_hex(&injection_path),
            "borderline_sha256": sha256_file_hex(&borderline_path),
            "injection_source_sha256": sha256_file_hex(&injection_source_path),
            "injection_source": injection_source,
            "style_slot": style_slot,
            "tau": spec.tau,
            "style_model": model_path,
            "style_model_sha256": sha256_file_hex(&model_path),
            "style_tokenizer": tokenizer_path,
            "style_tokenizer_sha256": sha256_file_hex(&tokenizer_path),
        }),
    );
    let matched_readback = write_json(
        &root,
        "matched-style-readback.json",
        &json!({
            "dim": matched_style.len(),
            "norm": norm(&matched_style),
            "prefix": prefix(&matched_style, 5),
            "style_slot": style_slot,
        }),
    );

    let injection_output = guard_generate(
        &profile,
        &style_input(injection, [0x27; 16]),
        &speaker_lens,
        &style_lens,
        &handler,
        false,
    )
    .expect("injection guard_generate");
    let injection_record = expect_quarantine(injection_output.clone(), style_slot);
    let injection_readback = write_json(
        &root,
        "injection-quarantine-readback.json",
        &json!({
            "output": injection_output,
            "record": injection_record,
        }),
    );

    let accepted_output = guard_generate(
        &profile,
        &style_input(in_persona, [0x28; 16]),
        &speaker_lens,
        &style_lens,
        &handler,
        false,
    )
    .expect("in-persona guard_generate");
    assert_accepted(&accepted_output, style_slot);
    let accepted_readback = write_json(
        &root,
        "in-persona-accepted-readback.json",
        &serde_json::to_value(&accepted_output).expect("accepted json"),
    );

    let borderline_output = guard_generate(
        &profile,
        &style_input(borderline, [0x29; 16]),
        &speaker_lens,
        &style_lens,
        &handler,
        false,
    )
    .expect("borderline guard_generate");
    assert_borderline_consistent(&borderline_output, style_slot);
    let borderline_readback = write_json(
        &root,
        "borderline-verdict-readback.json",
        &serde_json::to_value(&borderline_output).expect("borderline json"),
    );

    let records = vault.novel_records().expect("read novelty records");
    assert!(
        records.iter().any(|record| {
            record.novel_id == injection_record.novel_id
                && record.status == NoveltyStatus::Quarantined
                && record.action_taken == NoveltyAction::Quarantine
        }),
        "durable novelty records must include the injection quarantine"
    );
    let quarantine_readback = write_json(
        &root,
        "quarantine-record-readback.json",
        &serde_json::to_value(&records).expect("records json"),
    );

    write_manifest(
        &root,
        &[
            fixture_readback,
            matched_readback,
            injection_readback,
            accepted_readback,
            borderline_readback,
            quarantine_readback,
        ],
    );
}

fn style_identity_profile(
    spec: &StyleFixtureSpec,
    style_slot: SlotId,
    matched_style: Vec<f32>,
    profile_path: &Path,
) -> IdentityProfile {
    assert!(spec.tau.is_finite() && (0.0..=1.0).contains(&spec.tau));
    let guard_id = spec.guard_id.parse::<GuardId>().expect("valid guard id");
    let mut tau = BTreeMap::new();
    tau.insert(style_slot, spec.tau);
    let corpus_hash = sha256_file(profile_path);
    let mut per_slot = BTreeMap::new();
    per_slot.insert(
        style_slot,
        SlotCalibrationMeta {
            corpus_hash,
            estimator: "identity_fixture_tau_manual_v1".to_string(),
            far: 0.01,
            frr: 0.0,
            confidence: 0.99,
            ts: CLOCK_TS as i64,
        },
    );
    let mut matched = BTreeMap::new();
    matched.insert(style_slot, matched_style);

    IdentityProfile::new(
        GuardProfile {
            guard_id,
            panel_version: u64::from(spec.panel_version),
            domain: spec.domain.clone(),
            tau,
            required_slots: vec![style_slot],
            policy: GuardPolicy::AllRequired,
            calibration: Some(CalibrationMeta {
                corpus_hash,
                estimator: "identity_fixture_tau_manual_v1".to_string(),
                far: 0.01,
                frr: 0.0,
                confidence: 0.99,
                ts: CLOCK_TS as i64,
                per_slot,
            }),
            novelty_action: NoveltyAction::Quarantine,
        },
        vec![IdentitySlotConfig {
            slot_id: style_slot,
            anchor_kind: AnchorKind::StyleHold,
            tau_override: None,
        }],
        matched,
    )
    .expect("valid style identity profile")
}

fn style_input(text: String, cx_bytes: [u8; 16]) -> GenerateInput {
    GenerateInput {
        candidate_audio: None,
        candidate_text: Some(text),
        sample_rate: 16_000,
        matched_cx_id: CxId::from_bytes(cx_bytes),
    }
}

fn expect_quarantine(output: GenerateOutput, style_slot: SlotId) -> NoveltyRecord {
    let GenerateOutput::Novel { record } = output else {
        panic!("expected quarantined Novel output");
    };
    assert_eq!(record.status, NoveltyStatus::Quarantined);
    assert_eq!(record.action_taken, NoveltyAction::Quarantine);
    let failing = record
        .failing_verdicts
        .iter()
        .find(|verdict| verdict.slot == style_slot)
        .expect("style slot must fail");
    assert!(!failing.pass);
    assert!(
        failing.cos < failing.tau,
        "style injection must be outside tau: cos={} tau={}",
        failing.cos,
        failing.tau
    );
    assert!(!record.novel_id.as_uuid().is_nil());
    record
}

fn assert_accepted(output: &GenerateOutput, style_slot: SlotId) {
    let GenerateOutput::Accepted {
        verdict,
        provenance_tag,
        ..
    } = output
    else {
        panic!("expected accepted output");
    };
    assert_eq!(provenance_tag, GUARDED_PASS_TAG);
    assert!(verdict.overall_pass);
    let slot = verdict
        .per_slot
        .iter()
        .find(|verdict| verdict.slot == style_slot)
        .expect("accepted style verdict");
    assert!(slot.pass);
    assert!(slot.cos >= slot.tau);
}

fn assert_borderline_consistent(output: &GenerateOutput, style_slot: SlotId) {
    let per_slot = match output {
        GenerateOutput::Accepted { verdict, .. } | GenerateOutput::Rejected { verdict, .. } => {
            &verdict.per_slot
        }
        GenerateOutput::Novel { record } => &record.failing_verdicts,
    };
    let slot = per_slot
        .iter()
        .find(|verdict| verdict.slot == style_slot)
        .expect("borderline style verdict");
    assert_eq!(slot.pass, slot.cos >= slot.tau);
}

fn provider_policy() -> StyleProviderPolicy {
    match env::var("CALYX_WARD_STYLE_PROVIDER").as_deref() {
        Ok("cpu") => StyleProviderPolicy::CpuExplicit,
        _ => StyleProviderPolicy::CudaFailLoud,
    }
}

fn required_path_env(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is required"))
}

fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}

fn assert_empty_or_absent(path: &Path) {
    if path.exists() {
        assert!(
            fs::read_dir(path).expect("read FSV root").next().is_none(),
            "FSV root must be absent or empty before trigger: {}",
            path.display()
        );
    }
}

fn read_text(path: &Path) -> String {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read text fixture {}: {error}", path.display()));
    assert!(!text.trim().is_empty(), "text fixture must be non-empty");
    text
}

fn read_json<T>(path: &Path) -> T
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
        panic!("read json fixture {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse json fixture {}: {error}", path.display()))
}

fn write_json(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(name);
    fs::write(
        &path,
        serde_json::to_vec_pretty(value).expect("serialize readback"),
    )
    .expect("write readback json");
    path
}

fn write_manifest(root: &Path, primary_files: &[PathBuf]) {
    let mut files = primary_files.to_vec();
    files.extend(collect_files(&root.join("novelty-records")));
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

fn collect_files(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .expect("read nested files")
        .map(|entry| entry.expect("read entry").path())
        .filter(|path| path.is_file())
        .collect()
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

fn sha256_file_hex(path: &Path) -> String {
    hex(&sha256_file(path))
}

fn norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn prefix(values: &[f32], count: usize) -> Vec<f32> {
    values.iter().take(count).copied().collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
