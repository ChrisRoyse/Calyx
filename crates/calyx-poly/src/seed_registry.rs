//! Central registry for seed-bearing frozen Poly encoders (issue #22).
//!
//! The default panel must not hide random-feature seeds inside lens construction. This registry is
//! the source of truth for every production RFF lens seed, shape, scale, source field, and transform.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::encode::RffEncoder;
use crate::error::{PolyError, Result};

pub const SEED_REGISTRY_SCHEMA_VERSION: &str = "poly.seed_registry.v1";
pub const SEED_REGISTRY_ARTIFACT_KIND: &str = "poly_frozen_encoder_seed_registry";
pub const SEED_REGISTRY_FILE: &str = "poly_seed_registry.json";
pub const SEED_REGISTRY_VERSION: &str = "poly.default_panel.v1.rff.v1";

pub const ERR_SEED_REGISTRY_INVALID: &str = "CALYX_POLY_SEED_REGISTRY_INVALID";
pub const ERR_SEED_REGISTRY_COLLISION: &str = "CALYX_POLY_SEED_REGISTRY_COLLISION";
pub const ERR_SEED_REGISTRY_MISSING: &str = "CALYX_POLY_SEED_REGISTRY_MISSING";
pub const ERR_SEED_REGISTRY_READBACK_MISMATCH: &str = "CALYX_POLY_SEED_REGISTRY_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrozenRffSeedSpec {
    pub slot: u16,
    pub lens_id: &'static str,
    pub seed: u64,
    pub dim: usize,
    pub sigma: f64,
    pub source_field: &'static str,
    pub transform: &'static str,
    pub purpose: &'static str,
}

impl FrozenRffSeedSpec {
    pub fn encoder(&self) -> RffEncoder {
        RffEncoder::new(self.seed, self.dim, self.sigma)
    }

    pub fn entry(&self) -> SeedRegistryEntry {
        SeedRegistryEntry {
            slot: self.slot,
            lens_id: self.lens_id.to_string(),
            encoder_kind: FrozenEncoderKind::Rff,
            seed: self.seed,
            seed_hex: seed_hex(self.seed),
            dim: self.dim,
            sigma: self.sigma,
            source_field: self.source_field.to_string(),
            transform: self.transform.to_string(),
            purpose: self.purpose.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenEncoderKind {
    Rff,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedRegistryEntry {
    pub slot: u16,
    #[serde(rename = "lens_key")]
    pub lens_id: String,
    pub encoder_kind: FrozenEncoderKind,
    pub seed: u64,
    pub seed_hex: String,
    pub dim: usize,
    pub sigma: f64,
    pub source_field: String,
    pub transform: String,
    pub purpose: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedRegistryArtifact {
    pub schema_version: String,
    pub artifact_kind: String,
    pub registry_version: String,
    pub entries: Vec<SeedRegistryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedRegistryValidation {
    pub registry_version: String,
    pub entry_count: usize,
    pub seed_count: usize,
    pub required_lens_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeedRegistryRun {
    pub registry_path: PathBuf,
    pub registry: SeedRegistryArtifact,
    pub validation: SeedRegistryValidation,
}

pub const PRICE_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 0,
    lens_id: "price_rff",
    seed: 0x9E37_79B1,
    dim: 32,
    sigma: 0.05,
    source_field: "price_or_mid",
    transform: "identity",
    purpose: "probability-price locality around observed market price",
};

pub const DISTANCE_FROM_50_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 1,
    lens_id: "distance_from_50",
    seed: 0x0D15_7A17,
    dim: 16,
    sigma: 0.05,
    source_field: "abs(price_or_mid - 0.5)",
    transform: "identity",
    purpose: "market indecision distance from an even-price contract",
};

pub const SPREAD_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 2,
    lens_id: "spread_rff",
    seed: 0x5F9E_AD01,
    dim: 16,
    sigma: 0.01,
    source_field: "spread",
    transform: "signed_log",
    purpose: "top-of-book spread geometry after heavy-tail compression",
};

pub const OFI_VEC_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 5,
    lens_id: "ofi_vec",
    seed: 0x000F_1CEE,
    dim: 24,
    sigma: 0.25,
    source_field: "ofi",
    transform: "identity",
    purpose: "order-flow imbalance direction and magnitude",
};

pub const MOMENTUM_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 6,
    lens_id: "momentum_rff",
    seed: 0x0ADD_9EAD,
    dim: 24,
    sigma: 0.05,
    source_field: "one_day_change_or_one_hour_change",
    transform: "signed_log",
    purpose: "short-horizon price movement geometry",
};

pub const ARB_RESIDUAL_RFF: FrozenRffSeedSpec = FrozenRffSeedSpec {
    slot: 7,
    lens_id: "arb_residual",
    seed: 0x0A2B_5E11,
    dim: 16,
    sigma: 0.02,
    source_field: "yes_no_residual",
    transform: "identity",
    purpose: "yes/no residual mispricing geometry",
};

pub const FROZEN_RFF_SEED_SPECS: &[FrozenRffSeedSpec] = &[
    PRICE_RFF,
    DISTANCE_FROM_50_RFF,
    SPREAD_RFF,
    OFI_VEC_RFF,
    MOMENTUM_RFF,
    ARB_RESIDUAL_RFF,
];

pub const REQUIRED_RFF_LENS_KEYS: &[&str] = &[
    "price_rff",
    "distance_from_50",
    "spread_rff",
    "ofi_vec",
    "momentum_rff",
    "arb_residual",
];

pub fn default_seed_registry_artifact() -> SeedRegistryArtifact {
    SeedRegistryArtifact {
        schema_version: SEED_REGISTRY_SCHEMA_VERSION.to_string(),
        artifact_kind: SEED_REGISTRY_ARTIFACT_KIND.to_string(),
        registry_version: SEED_REGISTRY_VERSION.to_string(),
        entries: FROZEN_RFF_SEED_SPECS
            .iter()
            .map(|spec| spec.entry())
            .collect(),
    }
}

pub fn seed_spec_for_lens(lens_key: &str) -> Result<&'static FrozenRffSeedSpec> {
    FROZEN_RFF_SEED_SPECS
        .iter()
        .find(|spec| spec.lens_id == lens_key)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_SEED_REGISTRY_MISSING,
                format!("no frozen RFF seed registry entry for lens_key={lens_key}"),
            )
        })
}

pub fn run_seed_registry_readback(output_root: &Path) -> Result<SeedRegistryRun> {
    let registry = default_seed_registry_artifact();
    validate_seed_registry_artifact(&registry)?;
    let registry_path = write_seed_registry(output_root, &registry)?;
    let readback = read_seed_registry(&registry_path)?;
    if readback != registry {
        return Err(PolyError::diagnostics(
            ERR_SEED_REGISTRY_READBACK_MISMATCH,
            format!(
                "seed registry {} did not read back as written",
                registry_path.display()
            ),
        ));
    }
    let validation = validate_seed_registry_artifact(&readback)?;
    Ok(SeedRegistryRun {
        registry_path,
        registry: readback,
        validation,
    })
}

pub fn write_seed_registry(dir: &Path, registry: &SeedRegistryArtifact) -> Result<PathBuf> {
    validate_seed_registry_artifact(registry)?;
    write_json(dir, SEED_REGISTRY_FILE, registry)
}

pub fn read_seed_registry(path: &Path) -> Result<SeedRegistryArtifact> {
    read_json(path)
}

pub fn validate_seed_registry_artifact(
    registry: &SeedRegistryArtifact,
) -> Result<SeedRegistryValidation> {
    if registry.schema_version != SEED_REGISTRY_SCHEMA_VERSION
        || registry.artifact_kind != SEED_REGISTRY_ARTIFACT_KIND
        || registry.registry_version.trim().is_empty()
    {
        return invalid("unexpected seed registry schema, artifact kind, or empty version");
    }
    if registry.entries.is_empty() {
        return invalid("seed registry must contain at least one frozen encoder entry");
    }

    let mut keys = BTreeSet::new();
    let mut seeds = BTreeMap::<u64, &str>::new();
    for entry in &registry.entries {
        validate_entry(entry)?;
        if !keys.insert(entry.lens_id.as_str()) {
            return invalid(format!("duplicate lens_key {}", entry.lens_id));
        }
        if let Some(first_key) = seeds.insert(entry.seed, entry.lens_id.as_str()) {
            return Err(PolyError::diagnostics(
                ERR_SEED_REGISTRY_COLLISION,
                format!(
                    "seed {} is assigned to both {} and {}",
                    entry.seed_hex, first_key, entry.lens_id
                ),
            ));
        }
    }

    for required in REQUIRED_RFF_LENS_KEYS {
        if !keys.contains(required) {
            return Err(PolyError::diagnostics(
                ERR_SEED_REGISTRY_MISSING,
                format!("required frozen RFF lens_key {required} missing from seed registry"),
            ));
        }
    }

    Ok(SeedRegistryValidation {
        registry_version: registry.registry_version.clone(),
        entry_count: registry.entries.len(),
        seed_count: seeds.len(),
        required_lens_count: REQUIRED_RFF_LENS_KEYS.len(),
    })
}

fn validate_entry(entry: &SeedRegistryEntry) -> Result<()> {
    if entry.lens_id.trim().is_empty()
        || entry.source_field.trim().is_empty()
        || entry.transform.trim().is_empty()
        || entry.purpose.trim().is_empty()
    {
        return invalid("seed registry entries require lens_key, source_field, transform, purpose");
    }
    if entry.dim == 0 || !entry.sigma.is_finite() || entry.sigma <= 0.0 {
        return invalid(format!(
            "lens_key {} has invalid dim={} sigma={}",
            entry.lens_id, entry.dim, entry.sigma
        ));
    }
    if entry.seed_hex != seed_hex(entry.seed) {
        return invalid(format!(
            "lens_key {} has seed_hex {} but seed {} renders as {}",
            entry.lens_id,
            entry.seed_hex,
            entry.seed,
            seed_hex(entry.seed)
        ));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_SEED_REGISTRY_INVALID,
        message.into(),
    ))
}

fn seed_hex(seed: u64) -> String {
    format!("0x{seed:016X}")
}
