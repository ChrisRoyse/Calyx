use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use calyx_registry::{
    Registry, lens_spec_from_manifest_path, load_vault_panel_state, persist_vault_panel_state,
    swap_panel_to_target,
};
use serde::Serialize;

pub(super) use super::template_cards::ensemble_card_from_capability_cards;
pub(super) use super::template_model::{
    CATALOG_VERSION, MIN_CONTENT_LENSES, OBJECT_VERSION, PanelTemplateCatalog,
    PanelTemplateIndexEntry, PanelTemplateVersionRef, SavedPanelTemplate, TEMPLATE_INVALID,
    TEMPLATE_NOT_FOUND, TemplateDraft, TemplateEnsembleCard, TemplateLensRef,
    default_time_controls, lens_ref_from_catalog, template_error,
};
use crate::error::CliResult;
use crate::lens_commands::support::register_manifest_runtime;

#[derive(Clone, Debug)]
pub(super) struct TemplateStore {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TemplateSave {
    pub template_id: String,
    pub object_path: PathBuf,
    pub index_path: PathBuf,
    pub template: SavedPanelTemplate,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TemplateSummary {
    pub name: String,
    pub active_template_id: String,
    pub version: u32,
    pub content_lens_count: usize,
    pub time_control_count: usize,
    pub has_ensemble_card: bool,
    pub a37_gate_eligible: bool,
    pub a37_status: String,
    pub object_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TemplateSwap {
    pub template_id: String,
    pub template_name: String,
    pub vault: PathBuf,
    pub content_lens_count: usize,
    pub a37_gate_eligible: bool,
    pub a37_status: String,
    pub registered_lenses_added: usize,
    pub manifest_seq: u64,
    pub panel_ref: String,
    pub registry_ref: String,
    pub diff: calyx_registry::PanelDiff,
}

impl TemplateStore {
    pub(super) fn open(home: impl AsRef<Path>) -> Self {
        Self {
            root: home.as_ref().join("panels").join("templates"),
        }
    }

    pub(super) fn save(&self, draft: TemplateDraft, saved_at_ms: u64) -> CliResult<TemplateSave> {
        let mut catalog = self.read_catalog()?;
        let version = next_version(&catalog, &draft.name);
        let template = SavedPanelTemplate {
            schema_version: OBJECT_VERSION,
            name: draft.name,
            version,
            notes: draft.notes,
            min_content_lenses: MIN_CONTENT_LENSES,
            lenses: draft.lenses,
            time_controls: default_time_controls(),
            ensemble_card: draft.ensemble_card,
        };
        template.validate()?;
        let bytes = object_bytes(&template)?;
        let template_id = blake3::hash(&bytes).to_hex().to_string();
        let object_path = self.object_path(&template_id);
        write_immutable(&object_path, &bytes)?;
        self.upsert_index(
            &mut catalog,
            &template,
            &template_id,
            object_rel_path(&template_id),
            bytes.len() as u64,
            saved_at_ms,
        );
        self.write_catalog(&catalog)?;
        Ok(TemplateSave {
            template_id,
            object_path,
            index_path: self.index_path(),
            template,
        })
    }

    pub(super) fn fork(
        &self,
        selector: &str,
        name: String,
        notes: Option<String>,
        saved_at_ms: u64,
    ) -> CliResult<TemplateSave> {
        let source = self.load(selector)?;
        self.save(
            TemplateDraft {
                name,
                notes: notes.unwrap_or(source.notes),
                lenses: source.lenses,
                ensemble_card: source.ensemble_card,
            },
            saved_at_ms,
        )
    }

    pub(super) fn profile(
        &self,
        selector: &str,
        card: TemplateEnsembleCard,
        saved_at_ms: u64,
    ) -> CliResult<TemplateSave> {
        let source = self.load(selector)?;
        self.save(
            TemplateDraft {
                name: source.name,
                notes: source.notes,
                lenses: source.lenses,
                ensemble_card: Some(card),
            },
            saved_at_ms,
        )
    }

    pub(super) fn list(&self) -> CliResult<Vec<TemplateSummary>> {
        let catalog = self.read_catalog()?;
        catalog
            .templates
            .iter()
            .map(|entry| {
                let active = version_ref(entry, &entry.active_template_id)?;
                let template = self.read_object(&active.object_path, &active.blake3_hex)?;
                let a37 = template.a37_admission();
                Ok(TemplateSummary {
                    name: entry.name.clone(),
                    active_template_id: entry.active_template_id.clone(),
                    version: template.version,
                    content_lens_count: template.content_lens_count(),
                    time_control_count: template.time_controls.len(),
                    has_ensemble_card: template.ensemble_card.is_some(),
                    a37_gate_eligible: template.a37_gate_eligible(),
                    a37_status: a37.status,
                    object_path: active.object_path.clone(),
                })
            })
            .collect()
    }

    pub(super) fn load(&self, selector: &str) -> CliResult<SavedPanelTemplate> {
        let catalog = self.read_catalog()?;
        if let Some(entry) = catalog
            .templates
            .iter()
            .find(|entry| entry.name == selector)
        {
            let active = version_ref(entry, &entry.active_template_id)?;
            return self.read_object(&active.object_path, &active.blake3_hex);
        }
        for entry in &catalog.templates {
            if let Some(version) = entry
                .versions
                .iter()
                .find(|version| version.template_id == selector)
            {
                return self.read_object(&version.object_path, &version.blake3_hex);
            }
        }
        Err(template_error(
            TEMPLATE_NOT_FOUND,
            format!("panel template {selector} is not saved"),
            "save or seed the panel template before selecting it",
        ))
    }

    pub(super) fn swap_into_vault(
        &self,
        selector: &str,
        vault_dir: &Path,
        now_ms: u64,
        require_a37_gate: bool,
    ) -> CliResult<TemplateSwap> {
        let mut template = self.load(selector)?;
        template.validate()?;
        if require_a37_gate {
            template.require_a37_gate()?;
        }
        let a37 = template.a37_admission();
        let template_id = id_for_loaded(&template)?;
        let mut state = load_vault_panel_state(vault_dir)?;
        let registered_lenses_added = register_template_lenses(&mut state.registry, &mut template)?;
        let mut panel = state.panel;
        let target = template.to_target_panel(now_ms);
        let diff = swap_panel_to_target(&mut panel, &target, now_ms);
        let write = persist_vault_panel_state(vault_dir, &panel, &state.registry)?;
        Ok(TemplateSwap {
            template_id,
            template_name: template.name,
            vault: vault_dir.to_path_buf(),
            content_lens_count: target
                .slots
                .iter()
                .filter(|slot| !slot.retrieval_only && !slot.excluded_from_dedup)
                .count(),
            a37_gate_eligible: a37.gate_eligible,
            a37_status: a37.status,
            registered_lenses_added,
            manifest_seq: write.manifest_seq,
            panel_ref: write.panel_ref.logical_path,
            registry_ref: write.registry_ref.logical_path,
            diff,
        })
    }

    fn read_catalog(&self) -> CliResult<PanelTemplateCatalog> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(PanelTemplateCatalog {
                schema_version: CATALOG_VERSION,
                templates: Vec::new(),
            });
        }
        let catalog: PanelTemplateCatalog = serde_json::from_slice(&fs::read(&path)?)?;
        if catalog.schema_version != CATALOG_VERSION {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "unsupported panel template catalog {}",
                    catalog.schema_version
                ),
                "migrate the panel template catalog through a compatible reader",
            ));
        }
        Ok(catalog)
    }

    fn write_catalog(&self, catalog: &PanelTemplateCatalog) -> CliResult {
        let bytes = serde_json::to_vec_pretty(catalog)?;
        write_atomic(&self.index_path(), &bytes)
    }

    fn upsert_index(
        &self,
        catalog: &mut PanelTemplateCatalog,
        template: &SavedPanelTemplate,
        template_id: &str,
        object_path: String,
        size_bytes: u64,
        saved_at_ms: u64,
    ) {
        let version = PanelTemplateVersionRef {
            version: template.version,
            template_id: template_id.to_string(),
            object_path,
            blake3_hex: template_id.to_string(),
            size_bytes,
            saved_at_ms,
        };
        match catalog
            .templates
            .iter_mut()
            .find(|entry| entry.name == template.name)
        {
            Some(entry) => {
                entry.active_template_id = template_id.to_string();
                entry.versions.push(version);
            }
            None => catalog.templates.push(PanelTemplateIndexEntry {
                name: template.name.clone(),
                active_template_id: template_id.to_string(),
                versions: vec![version],
            }),
        }
        catalog
            .templates
            .sort_by(|left, right| left.name.cmp(&right.name));
    }

    fn read_object(&self, object_path: &str, expected: &str) -> CliResult<SavedPanelTemplate> {
        let path = self.root.join(object_path);
        let bytes = fs::read(&path)?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != expected {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("template object {} hash mismatch", path.display()),
                "do not edit immutable template objects; re-save the template",
            ));
        }
        let template: SavedPanelTemplate = serde_json::from_slice(&bytes)?;
        template.validate()?;
        Ok(template)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    fn object_path(&self, template_id: &str) -> PathBuf {
        self.root.join(object_rel_path(template_id))
    }
}

fn next_version(catalog: &PanelTemplateCatalog, name: &str) -> u32 {
    catalog
        .templates
        .iter()
        .find(|entry| entry.name == name)
        .and_then(|entry| entry.versions.iter().map(|item| item.version).max())
        .map_or(1, |version| version.saturating_add(1))
}

fn version_ref<'a>(
    entry: &'a PanelTemplateIndexEntry,
    template_id: &str,
) -> CliResult<&'a PanelTemplateVersionRef> {
    entry
        .versions
        .iter()
        .find(|version| version.template_id == template_id)
        .ok_or_else(|| {
            template_error(
                TEMPLATE_INVALID,
                format!("index entry {} points at missing version", entry.name),
                "repair the template catalog index from immutable objects",
            )
        })
}

fn register_template_lenses(
    registry: &mut Registry,
    template: &mut SavedPanelTemplate,
) -> CliResult<usize> {
    let mut added = 0;
    for lens in &mut template.lenses {
        if lens.runtime_lens_id.is_some_and(|id| registry.contains(id)) {
            continue;
        }
        let spec = lens_spec_from_manifest_path(Path::new(&lens.manifest))?;
        let spec_lens_id = spec.lens_id();
        if spec_lens_id != lens.lens_id {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "manifest {} no longer resolves to {}",
                    lens.manifest, lens.lens_id
                ),
                "rebuild the template from the current frozen lens manifest",
            ));
        }
        if let Some(existing) = registry.find_lens_by_spec_id(spec_lens_id) {
            if registry.lens_spec(existing) != Some(&spec) {
                return Err(template_error(
                    TEMPLATE_INVALID,
                    format!(
                        "registry lens {existing} does not match manifest {}",
                        lens.manifest
                    ),
                    "recommission the lens so the registry snapshot and manifest are identical",
                ));
            }
            if let Some(expected) = lens.runtime_lens_id
                && existing != expected
            {
                return Err(template_error(
                    TEMPLATE_INVALID,
                    format!("runtime resolved {existing}, expected {expected}"),
                    "recommission the lens so runtime and manifest contracts agree",
                ));
            }
            lens.runtime_lens_id = Some(existing);
            continue;
        }
        let registered = register_manifest_runtime(registry, spec)?;
        if let Some(expected) = lens.runtime_lens_id
            && registered != expected
        {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("runtime registered {registered}, expected {expected}"),
                "recommission the lens so runtime and manifest contracts agree",
            ));
        }
        lens.runtime_lens_id = Some(registered);
        added += 1;
    }
    Ok(added)
}

fn id_for_loaded(template: &SavedPanelTemplate) -> CliResult<String> {
    Ok(blake3::hash(&object_bytes(template)?).to_hex().to_string())
}

fn object_bytes(template: &SavedPanelTemplate) -> CliResult<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(template)?)
}

fn object_rel_path(template_id: &str) -> String {
    format!("objects/{template_id}.json")
}

fn write_immutable(path: &Path, bytes: &[u8]) -> CliResult {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "immutable template object {} already exists with different bytes",
                    path.display()
                ),
                "do not edit immutable template objects; save a new template version",
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
