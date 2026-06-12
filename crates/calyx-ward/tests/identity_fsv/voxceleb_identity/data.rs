use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_ward::{SpeakerLens, SpeakerProviderPolicy};
use sha2::{Digest, Sha256};

use super::codec::decode_wav_pcm16_mono;
use super::{
    CALYX_WARD_VOXCELEB_BAD_WAV, CALYX_WARD_VOXCELEB_EMPTY_DATASET, CALYX_WARD_VOXCELEB_TAU_OVERLAP,
};

#[derive(Clone, Debug)]
pub(super) struct EmbeddedClip {
    pub(super) rel_path: String,
    pub(super) speaker_id: String,
    pub(super) wav_sha256: String,
    pub(super) wav_blake3: String,
    pub(super) wav_bytes: usize,
    pub(super) sample_rate: u32,
    pub(super) frames: usize,
    pub(super) embedding: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FsvError {
    pub(super) code: &'static str,
    pub(super) message: String,
}

impl FsvError {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(super) fn load_voxceleb_clips(
    root: &Path,
    lens: &SpeakerLens,
) -> Result<Vec<EmbeddedClip>, FsvError> {
    let selected = fs::read_to_string(root.join("selected-files.txt")).map_err(|error| {
        FsvError::new(
            CALYX_WARD_VOXCELEB_EMPTY_DATASET,
            format!("read selected-files.txt: {error}"),
        )
    })?;
    let mut clips = Vec::new();
    for rel in selected.lines().filter(|line| !line.trim().is_empty()) {
        let path = root.join(rel);
        let bytes = fs::read(&path).map_err(|error| {
            FsvError::new(CALYX_WARD_VOXCELEB_BAD_WAV, format!("{rel}: {error}"))
        })?;
        let decoded = decode_wav_pcm16_mono(&bytes)
            .map_err(|error| FsvError::new(error.code, format!("{rel}: {}", error.message)))?;
        let embedding = lens
            .embed_speaker(&decoded.samples, decoded.sample_rate)
            .map_err(|error| FsvError::new(error.code(), error.to_string()))?;
        clips.push(EmbeddedClip {
            rel_path: rel.to_string(),
            speaker_id: speaker_id(rel)?,
            wav_sha256: sha256_hex(&bytes),
            wav_blake3: blake3::hash(&bytes).to_string(),
            wav_bytes: bytes.len(),
            sample_rate: decoded.sample_rate,
            frames: decoded.frames,
            embedding,
        });
    }
    clips.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    Ok(clips)
}

pub(super) fn synthetic_clip(speaker: &str, index: usize, embedding: [f32; 2]) -> EmbeddedClip {
    EmbeddedClip {
        rel_path: format!("{speaker}-{index}.wav"),
        speaker_id: speaker.to_string(),
        wav_sha256: format!("{index:064x}"),
        wav_blake3: format!("{index:064x}"),
        wav_bytes: 2,
        sample_rate: 16_000,
        frames: 1,
        embedding: embedding.to_vec(),
    }
}

pub(super) fn overlap_clips() -> Vec<EmbeddedClip> {
    (0..50)
        .map(|index| {
            synthetic_clip(
                if index < 25 { "speaker-a" } else { "speaker-b" },
                index,
                [1.0, 0.0],
            )
        })
        .collect()
}

pub(super) fn speaker_groups(clips: &[EmbeddedClip]) -> BTreeMap<String, Vec<usize>> {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, clip) in clips.iter().enumerate() {
        groups
            .entry(clip.speaker_id.clone())
            .or_default()
            .push(index);
    }
    groups
}

pub(super) fn speaker_count(clips: &[EmbeddedClip]) -> usize {
    speaker_groups(clips).len()
}

pub(super) fn speaker_provider_policy() -> SpeakerProviderPolicy {
    match env::var("CALYX_WARD_SPEAKER_PROVIDER").as_deref() {
        Ok("cuda") => SpeakerProviderPolicy::CudaFailLoud,
        _ => SpeakerProviderPolicy::CpuExplicit,
    }
}

pub(super) fn required_path_env(name: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is required"))
}

pub(super) fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

pub(super) fn sha256_file_hex(path: &Path) -> String {
    sha256_hex(&fs::read(path).expect("read sha256 input"))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn display(path: &Path) -> String {
    path.display().to_string()
}

fn speaker_id(rel: &str) -> Result<String, FsvError> {
    rel.rsplit('/')
        .next()
        .and_then(|name| name.split('-').next())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| FsvError::new(CALYX_WARD_VOXCELEB_BAD_WAV, format!("bad filename {rel}")))
}

#[allow(dead_code)]
pub(super) fn tau_overlap_error(message: impl Into<String>) -> FsvError {
    FsvError::new(CALYX_WARD_VOXCELEB_TAU_OVERLAP, message)
}
