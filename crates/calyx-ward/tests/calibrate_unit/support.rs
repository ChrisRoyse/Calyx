use super::*;

pub(super) fn calibration_input(
    slot: SlotId,
    slot_kind: SlotKind,
    target_far: f32,
) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..100).map(|i| 0.80 + i as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|i| 0.30 + i as f32 * 0.003).collect(),
        slot_kind,
        target_far,
    }
}

pub(super) fn confidence_supported_input(
    slot: SlotId,
    slot_kind: SlotKind,
    target_far: f32,
) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..1_000).map(|i| 0.80 + i as f32 * 0.0001).collect(),
        bad_scores: (0..1_000).map(|i| 0.30 + i as f32 * 0.0003).collect(),
        slot_kind,
        target_far,
    }
}

pub(super) fn profile_template() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

pub(super) fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
}

pub(super) fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

pub(super) fn write_sha_manifest(root: &str) {
    let root = std::path::Path::new(root);
    let mut lines = Vec::new();
    for entry in std::fs::read_dir(root).expect("read fsv root") {
        let path = entry.expect("dir entry").path();
        if path.is_file() && path.file_name().unwrap() != "sha256-manifest.txt" {
            let bytes = std::fs::read(&path).expect("read fsv file");
            lines.push(format!(
                "{:x}  {}\n",
                Sha256::digest(bytes),
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    lines.sort();
    std::fs::write(root.join("sha256-manifest.txt"), lines.concat()).expect("write sha manifest");
}

pub(super) fn hash_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

pub(super) const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
