use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
use super::A38_BUNDLE_NOT_FOUND;
use super::model::{
    A38_COVERAGE_STATUS, A38BundleCatalog, A38BundleIndexEntry, A38BundleVersionRef,
    BUNDLE_CATALOG_VERSION, BUNDLE_OBJECT_VERSION, BundleDraft, BundleSummary, SavedA38Bundle, mib,
    mib_to_bytes, modality_counts, object_bytes,
};
use super::{A38_BUNDLE_INVALID, bundle_error};
use crate::error::CliResult;

#[derive(Clone, Debug)]
pub(super) struct A38BundleStore {
    root: PathBuf,
}

pub(super) struct BundleSave {
    pub bundle_id: String,
    pub object_path: PathBuf,
    pub index_path: PathBuf,
    pub bundle: SavedA38Bundle,
}

impl A38BundleStore {
    pub(super) fn open(home: impl AsRef<Path>) -> Self {
        Self {
            root: home.as_ref().join("panels").join("a38-bundles"),
        }
    }

    pub(super) fn save(&self, draft: BundleDraft, saved_at_ms: u64) -> CliResult<BundleSave> {
        let mut catalog = self.read_catalog()?;
        let version = next_version(&catalog, &draft.name);
        let total_vram_bytes = draft
            .lenses
            .iter()
            .map(|lens| lens.cost.vram_bytes)
            .fold(0_u64, u64::saturating_add);
        let budget_vram_bytes = mib_to_bytes(draft.budget_vram_mib);
        let bundle = SavedA38Bundle {
            schema_version: BUNDLE_OBJECT_VERSION,
            name: draft.name,
            version,
            base_template: draft.base_template,
            registry_ref: draft.registry_ref,
            required_modalities: draft.required_modalities,
            modality_counts: modality_counts(&draft.lenses),
            content_lens_count: draft.lenses.len(),
            evidence_refs: draft.evidence_refs,
            lenses: draft.lenses,
            total_vram_bytes,
            total_vram_mib: mib(total_vram_bytes),
            budget_vram_bytes,
            budget_vram_mib: draft.budget_vram_mib,
            under_budget: total_vram_bytes <= budget_vram_bytes,
            coverage_status: A38_COVERAGE_STATUS.to_string(),
            created_at_ms: saved_at_ms,
        };
        bundle.validate()?;
        let bytes = object_bytes(&bundle)?;
        let bundle_id = blake3::hash(&bytes).to_hex().to_string();
        let object_path = self.object_path(&bundle_id);
        write_immutable(&object_path, &bytes)?;
        self.upsert_index(
            &mut catalog,
            &bundle,
            &bundle_id,
            object_rel_path(&bundle_id),
            bytes.len() as u64,
            saved_at_ms,
        );
        self.write_catalog(&catalog)?;
        Ok(BundleSave {
            bundle_id,
            object_path,
            index_path: self.index_path(),
            bundle,
        })
    }

    pub(super) fn list(&self) -> CliResult<Vec<BundleSummary>> {
        let catalog = self.read_catalog()?;
        catalog
            .bundles
            .iter()
            .map(|entry| {
                let active = version_ref(entry, &entry.active_bundle_id)?;
                let bundle = self.read_object(&active.object_path, &active.blake3_hex)?;
                Ok(BundleSummary {
                    name: entry.name.clone(),
                    active_bundle_id: entry.active_bundle_id.clone(),
                    version: bundle.version,
                    base_template_id: bundle.base_template.template_id,
                    content_lens_count: bundle.content_lens_count,
                    modality_counts: bundle.modality_counts,
                    evidence_ref_count: bundle.evidence_refs.len(),
                    total_vram_bytes: bundle.total_vram_bytes,
                    budget_vram_mib: bundle.budget_vram_mib,
                    under_budget: bundle.under_budget,
                    coverage_status: bundle.coverage_status,
                    object_path: active.object_path.clone(),
                })
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn load(&self, selector: &str) -> CliResult<SavedA38Bundle> {
        let catalog = self.read_catalog()?;
        if let Some(entry) = catalog.bundles.iter().find(|entry| entry.name == selector) {
            let active = version_ref(entry, &entry.active_bundle_id)?;
            return self.read_object(&active.object_path, &active.blake3_hex);
        }
        for entry in &catalog.bundles {
            if let Some(version) = entry
                .versions
                .iter()
                .find(|version| version.bundle_id == selector)
            {
                return self.read_object(&version.object_path, &version.blake3_hex);
            }
        }
        Err(bundle_error(
            A38_BUNDLE_NOT_FOUND,
            format!("A38 bundle {selector} is not saved"),
            "save the A38 bundle before selecting it",
        ))
    }

    pub(super) fn read_catalog(&self) -> CliResult<A38BundleCatalog> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(A38BundleCatalog {
                schema_version: BUNDLE_CATALOG_VERSION,
                bundles: Vec::new(),
            });
        }
        let catalog: A38BundleCatalog = serde_json::from_slice(&fs::read(&path)?)?;
        if catalog.schema_version != BUNDLE_CATALOG_VERSION {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("unsupported A38 bundle catalog {}", catalog.schema_version),
                "migrate the A38 bundle catalog through a compatible reader",
            ));
        }
        Ok(catalog)
    }

    pub(super) fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    fn write_catalog(&self, catalog: &A38BundleCatalog) -> CliResult {
        let bytes = serde_json::to_vec_pretty(catalog)?;
        write_atomic(&self.index_path(), &bytes)
    }

    fn upsert_index(
        &self,
        catalog: &mut A38BundleCatalog,
        bundle: &SavedA38Bundle,
        bundle_id: &str,
        object_path: String,
        size_bytes: u64,
        saved_at_ms: u64,
    ) {
        let version = A38BundleVersionRef {
            version: bundle.version,
            bundle_id: bundle_id.to_string(),
            object_path,
            blake3_hex: bundle_id.to_string(),
            size_bytes,
            saved_at_ms,
        };
        match catalog
            .bundles
            .iter_mut()
            .find(|entry| entry.name == bundle.name)
        {
            Some(entry) => {
                entry.active_bundle_id = bundle_id.to_string();
                entry.versions.push(version);
            }
            None => catalog.bundles.push(A38BundleIndexEntry {
                name: bundle.name.clone(),
                active_bundle_id: bundle_id.to_string(),
                versions: vec![version],
            }),
        }
        catalog
            .bundles
            .sort_by(|left, right| left.name.cmp(&right.name));
    }

    fn read_object(&self, object_path: &str, expected: &str) -> CliResult<SavedA38Bundle> {
        let path = self.root.join(object_path);
        let bytes = fs::read(&path)?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != expected {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("A38 bundle object {} hash mismatch", path.display()),
                "do not edit immutable A38 bundle objects; save a new bundle version",
            ));
        }
        let bundle: SavedA38Bundle = serde_json::from_slice(&bytes)?;
        bundle.validate()?;
        Ok(bundle)
    }

    fn object_path(&self, bundle_id: &str) -> PathBuf {
        self.root.join(object_rel_path(bundle_id))
    }
}

fn next_version(catalog: &A38BundleCatalog, name: &str) -> u32 {
    catalog
        .bundles
        .iter()
        .find(|entry| entry.name == name)
        .and_then(|entry| entry.versions.iter().map(|item| item.version).max())
        .map_or(1, |version| version.saturating_add(1))
}

fn version_ref<'a>(
    entry: &'a A38BundleIndexEntry,
    bundle_id: &str,
) -> CliResult<&'a A38BundleVersionRef> {
    entry
        .versions
        .iter()
        .find(|version| version.bundle_id == bundle_id)
        .ok_or_else(|| {
            bundle_error(
                A38_BUNDLE_INVALID,
                format!("index entry {} points at missing version", entry.name),
                "repair the A38 bundle catalog index from immutable objects",
            )
        })
}

fn object_rel_path(bundle_id: &str) -> String {
    format!("objects/{bundle_id}.json")
}

fn write_immutable(path: &Path, bytes: &[u8]) -> CliResult {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!(
                    "immutable A38 bundle object {} already exists with different bytes",
                    path.display()
                ),
                "do not edit immutable A38 bundle objects; save a new bundle version",
            ));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
        Err(_) => {}
    }
    write_atomic(path, bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
