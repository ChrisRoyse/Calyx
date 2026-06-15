//! Atomic manifest and recovery ordering for Aster vaults.

mod quarantine;

use crate::dedup::DedupPolicy;
use crate::sst::SstReader;
use crate::timetravel::RetentionHorizon;
use crate::wal::{ReplayRecord, TornTail, replay_dir};
use calyx_core::{CalyxError, Result, TemporalPolicy};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const CURRENT_FILE: &str = "CURRENT";
const MANIFEST_FILE: &str = "MANIFEST";
const MANIFEST_PREFIX: &str = "manifest-";
const MANIFEST_SUFFIX: &str = ".json";
const SUPPORTED_MANIFEST_MAJOR: u16 = 1;
const SUPPORTED_MANIFEST_MINOR: u16 = 0;

pub use quarantine::QuarantineRecord;

/// Version guard for MANIFEST bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestVersion {
    pub major: u16,
    pub minor: u16,
}

impl ManifestVersion {
    pub const fn current() -> Self {
        Self {
            major: SUPPORTED_MANIFEST_MAJOR,
            minor: SUPPORTED_MANIFEST_MINOR,
        }
    }

    fn validate(self) -> Result<()> {
        if self.major != SUPPORTED_MANIFEST_MAJOR {
            return Err(format_version_unsupported(format!(
                "unsupported MANIFEST major version {}; supported major is {}",
                self.major, SUPPORTED_MANIFEST_MAJOR
            )));
        }
        Ok(())
    }
}

/// Content-addressed immutable reference captured by a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImmutableRef {
    pub logical_path: String,
    pub blake3_hex: String,
}

impl ImmutableRef {
    pub fn from_bytes(logical_path: impl Into<String>, bytes: &[u8]) -> Result<Self> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        Self::new(logical_path, hash)
    }

    pub fn new(logical_path: impl Into<String>, blake3_hex: impl Into<String>) -> Result<Self> {
        let reference = Self {
            logical_path: logical_path.into(),
            blake3_hex: blake3_hex.into().to_ascii_lowercase(),
        };
        reference.validate()?;
        Ok(reference)
    }

    fn validate(&self) -> Result<()> {
        if self.logical_path.is_empty() || self.logical_path.starts_with('/') {
            return Err(CalyxError::aster_corrupt_shard(
                "manifest immutable ref path must be vault-relative",
            ));
        }
        if Path::new(&self.logical_path)
            .components()
            .any(invalid_component)
        {
            return Err(CalyxError::aster_corrupt_shard(
                "manifest immutable ref path escapes vault",
            ));
        }
        if self.logical_path == CURRENT_FILE
            || self.logical_path == MANIFEST_FILE
            || self.logical_path.ends_with(".tmp")
        {
            return Err(CalyxError::aster_corrupt_shard(
                "manifest immutable ref points at mutable control file",
            ));
        }
        if self.blake3_hex.len() != 64 || !self.blake3_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CalyxError::aster_corrupt_shard(
                "manifest immutable ref hash must be 32-byte hex blake3",
            ));
        }
        Ok(())
    }
}

/// Durable Aster vault manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultManifest {
    pub version: ManifestVersion,
    pub manifest_seq: u64,
    pub durable_seq: u64,
    pub panel_ref: ImmutableRef,
    #[serde(default)]
    pub registry_ref: Option<ImmutableRef>,
    pub codebook_refs: Vec<ImmutableRef>,
    #[serde(default)]
    pub temporal_policy: Option<TemporalPolicy>,
    #[serde(default)]
    pub dedup_policy: Option<DedupPolicy>,
    #[serde(default)]
    pub retention_horizon: RetentionHorizon,
    pub degraded_rebuildable: bool,
    #[serde(default)]
    pub quarantines: Vec<QuarantineRecord>,
}

impl VaultManifest {
    pub fn new(
        manifest_seq: u64,
        durable_seq: u64,
        panel_ref: ImmutableRef,
        codebook_refs: Vec<ImmutableRef>,
    ) -> Result<Self> {
        Self::new_with_temporal_policy(
            manifest_seq,
            durable_seq,
            panel_ref,
            codebook_refs,
            Some(TemporalPolicy::default()),
        )
    }

    pub fn new_with_temporal_policy(
        manifest_seq: u64,
        durable_seq: u64,
        panel_ref: ImmutableRef,
        codebook_refs: Vec<ImmutableRef>,
        temporal_policy: Option<TemporalPolicy>,
    ) -> Result<Self> {
        Self::new_with_policies(
            manifest_seq,
            durable_seq,
            panel_ref,
            codebook_refs,
            temporal_policy,
            Some(DedupPolicy::default()),
        )
    }

    pub fn new_with_policies(
        manifest_seq: u64,
        durable_seq: u64,
        panel_ref: ImmutableRef,
        codebook_refs: Vec<ImmutableRef>,
        temporal_policy: Option<TemporalPolicy>,
        dedup_policy: Option<DedupPolicy>,
    ) -> Result<Self> {
        let manifest = Self {
            version: ManifestVersion::current(),
            manifest_seq,
            durable_seq,
            panel_ref,
            registry_ref: None,
            codebook_refs,
            temporal_policy,
            dedup_policy,
            retention_horizon: RetentionHorizon::default(),
            degraded_rebuildable: false,
            quarantines: Vec::new(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        self.version.validate()?;
        if self.manifest_seq == 0 {
            return Err(CalyxError::aster_corrupt_shard(
                "manifest sequence must start at one",
            ));
        }
        self.panel_ref.validate()?;
        require_prefix(&self.panel_ref, "panel/")?;
        if let Some(reference) = &self.registry_ref {
            reference.validate()?;
            require_prefix(reference, "registry/")?;
        }
        let mut seen = BTreeSet::new();
        for reference in &self.codebook_refs {
            reference.validate()?;
            require_prefix(reference, "codebooks/")?;
            if !seen.insert(reference.logical_path.as_str()) {
                return Err(CalyxError::aster_corrupt_shard(
                    "manifest contains duplicate codebook ref",
                ));
            }
        }
        for quarantine in &self.quarantines {
            quarantine.validate()?;
        }
        if let Some(policy) = &self.temporal_policy {
            policy.validate()?;
        }
        if let Some(policy) = &self.dedup_policy {
            policy.validate_manifest()?;
        }
        self.retention_horizon.validate()?;
        Ok(())
    }
}

/// Atomic manifest writer/reader rooted at one vault directory.
#[derive(Debug, Clone)]
pub struct ManifestStore {
    vault_dir: PathBuf,
}

impl ManifestStore {
    pub fn open(vault_dir: impl AsRef<Path>) -> Self {
        Self {
            vault_dir: vault_dir.as_ref().to_path_buf(),
        }
    }

    pub fn write_current(&self, manifest: &VaultManifest) -> Result<ManifestWrite> {
        manifest.validate()?;
        fs::create_dir_all(&self.vault_dir)
            .map_err(|error| storage_error("create vault manifest directory", error))?;
        let pointer = manifest_filename(manifest.manifest_seq);
        let manifest_path = self.vault_dir.join(&pointer);
        let mirror_path = self.vault_dir.join(MANIFEST_FILE);
        let current_path = self.vault_dir.join(CURRENT_FILE);
        let bytes = encode_manifest(manifest)?;

        write_atomic(&manifest_path, &bytes)?;
        write_atomic(&mirror_path, &bytes)?;
        write_atomic(&current_path, pointer.as_bytes())?;

        Ok(ManifestWrite {
            manifest_path,
            mirror_path,
            current_path,
            pointer,
        })
    }

    pub fn load_current(&self) -> Result<VaultManifest> {
        let pointer_bytes = fs::read(self.vault_dir.join(CURRENT_FILE))
            .map_err(|error| storage_error("read CURRENT", error))?;
        let pointer = std::str::from_utf8(&pointer_bytes)
            .map_err(|error| CalyxError::aster_corrupt_shard(format!("CURRENT utf8: {error}")))?
            .trim();
        if !valid_manifest_filename(pointer) {
            return Err(CalyxError::aster_corrupt_shard(
                "CURRENT does not point at immutable manifest file",
            ));
        }
        let bytes = fs::read(self.vault_dir.join(pointer))
            .map_err(|error| storage_error("read pointed MANIFEST", error))?;
        let manifest = decode_manifest(&bytes)?;
        verify_immutable_refs(&self.vault_dir, &manifest)?;
        Ok(manifest)
    }

    pub fn current_pointer(&self) -> Result<String> {
        let pointer = fs::read_to_string(self.vault_dir.join(CURRENT_FILE))
            .map_err(|error| storage_error("read CURRENT", error))?;
        Ok(pointer.trim().to_string())
    }

    pub fn append_quarantine(&self, record: QuarantineRecord) -> Result<VaultManifest> {
        let mut manifest = self.load_current()?;
        if !manifest.quarantines.contains(&record) {
            manifest.quarantines.push(record);
        }
        manifest.manifest_seq = manifest
            .manifest_seq
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("manifest sequence exhausted"))?;
        self.write_current(&manifest)?;
        Ok(manifest)
    }
}

/// Files produced by an atomic manifest swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestWrite {
    pub manifest_path: PathBuf,
    pub mirror_path: PathBuf,
    pub current_path: PathBuf,
    pub pointer: String,
}

/// Recovery result after loading MANIFEST first, then replaying WAL past it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryOutcome {
    pub manifest: VaultManifest,
    pub wal_records: Vec<ReplayRecord>,
    pub torn_tail: Option<TornTail>,
    pub last_recovered_seq: u64,
    pub degraded_rebuildable: bool,
}

pub fn recover_vault(vault_dir: impl AsRef<Path>) -> Result<RecoveryOutcome> {
    let vault_dir = vault_dir.as_ref();
    let manifest = ManifestStore::open(vault_dir).load_current()?;
    let replay = replay_dir(vault_dir.join("wal"))?;
    let wal_records: Vec<_> = replay
        .records
        .into_iter()
        .filter(|record| record.seq > manifest.durable_seq)
        .collect();
    let last_recovered_seq = wal_records
        .last()
        .map_or(manifest.durable_seq, |record| record.seq);
    let degraded_rebuildable = manifest.degraded_rebuildable;

    Ok(RecoveryOutcome {
        manifest,
        wal_records,
        torn_tail: replay.torn_tail,
        last_recovered_seq,
        degraded_rebuildable,
    })
}

/// Reads a base CF shard through the fail-closed SST path.
pub fn read_base_shard(path: impl AsRef<Path>, key: &[u8]) -> Result<Option<Vec<u8>>> {
    SstReader::open(path)?.get(key)
}

pub fn is_quarantined(manifest: &VaultManifest, seq: u64) -> bool {
    manifest
        .quarantines
        .iter()
        .any(|record| record.contains(seq))
}

pub fn is_vault_seq_quarantined(vault_dir: impl AsRef<Path>, seq: u64) -> Result<bool> {
    let vault_dir = vault_dir.as_ref();
    if !vault_dir.join(CURRENT_FILE).exists() {
        return Ok(false);
    }
    let manifest = ManifestStore::open(vault_dir).load_current()?;
    Ok(is_quarantined(&manifest, seq))
}

fn manifest_filename(seq: u64) -> String {
    format!("{MANIFEST_PREFIX}{seq:020}{MANIFEST_SUFFIX}")
}

fn valid_manifest_filename(name: &str) -> bool {
    if !name.starts_with(MANIFEST_PREFIX) || !name.ends_with(MANIFEST_SUFFIX) {
        return false;
    }
    let digits = &name[MANIFEST_PREFIX.len()..name.len() - MANIFEST_SUFFIX.len()];
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn encode_manifest(manifest: &VaultManifest) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(manifest)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode MANIFEST: {error}")))
}

fn decode_manifest(bytes: &[u8]) -> Result<VaultManifest> {
    let manifest: VaultManifest = serde_json::from_slice(bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode MANIFEST: {error}")))?;
    manifest.validate()?;
    Ok(manifest)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file =
            File::create(&tmp).map_err(|error| storage_error("create atomic temp", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write atomic temp", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync atomic temp", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("rename atomic file", error))?;
    sync_parent(path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("atomic path has no parent"))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| storage_error("fsync manifest directory", error))
}

#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<()> {
    path.parent()
        .ok_or_else(|| CalyxError::disk_pressure("atomic path has no parent"))?;
    Ok(())
}

fn invalid_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
    )
}

fn require_prefix(reference: &ImmutableRef, prefix: &str) -> Result<()> {
    if !reference.logical_path.starts_with(prefix) {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "manifest ref {} must be under {prefix}",
            reference.logical_path
        )));
    }
    Ok(())
}

fn verify_immutable_refs(vault_dir: &Path, manifest: &VaultManifest) -> Result<()> {
    verify_immutable_ref(vault_dir, &manifest.panel_ref)?;
    if let Some(reference) = &manifest.registry_ref {
        verify_immutable_ref(vault_dir, reference)?;
    }
    for reference in &manifest.codebook_refs {
        verify_immutable_ref(vault_dir, reference)?;
    }
    Ok(())
}

fn verify_immutable_ref(vault_dir: &Path, reference: &ImmutableRef) -> Result<()> {
    reference.validate()?;
    let path = vault_dir.join(&reference.logical_path);
    let bytes = fs::read(&path).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!(
            "manifest immutable ref {} unreadable: {error}",
            reference.logical_path
        ))
    })?;
    let actual = blake3::hash(&bytes).to_hex().to_string();
    if actual != reference.blake3_hex {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "manifest immutable ref {} hash mismatch: expected {}, got {}",
            reference.logical_path, reference.blake3_hex, actual
        )));
    }
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

fn format_version_unsupported(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_FORMAT_VERSION_UNSUPPORTED",
        message: message.into(),
        remediation: "refuse unknown format major; migrate through a compatible reader",
    }
}

#[cfg(test)]
mod tests;
