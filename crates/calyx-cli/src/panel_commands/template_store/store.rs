use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Panel, SlotState};
use calyx_registry::{
    Registry, load_vault_panel_state, persist_vault_panel_state, swap_panel_to_target,
};

use crate::error::{CliError, CliResult};

use super::readback;
use super::resident_registration::register_template_lenses_from_resident;
use super::storage_io::{object_rel_path, write_atomic, write_immutable};
use super::types::{TemplateSave, TemplateStore, TemplateSummary, TemplateSwap};
use super::{
    CATALOG_VERSION, MIN_CONTENT_LENSES, OBJECT_VERSION, PanelTemplateCatalog,
    PanelTemplateIndexEntry, PanelTemplateVersionRef, SavedPanelTemplate, TEMPLATE_INVALID,
    TEMPLATE_NOT_FOUND, TemplateDraft, TemplateEnsembleCard, default_time_controls, id_for_loaded,
    object_bytes, template_error,
};
use crate::panel_commands::template_model::admission::require_gpu_lens_admission;

impl TemplateStore {
    pub(in crate::panel_commands) fn open(home: impl AsRef<Path>) -> Self {
        Self {
            root: home.as_ref().join("panels").join("templates"),
        }
    }

    pub(in crate::panel_commands) fn save(
        &self,
        draft: TemplateDraft,
        saved_at_ms: u64,
    ) -> CliResult<TemplateSave> {
        // Covers the full catalog read-modify-write transaction across threads
        // and processes; atomic replacement alone cannot prevent a lost update.
        let _catalog_guard = crate::durable_write::DurableWriteLockGuard::acquire(
            &self.root.join(".catalog.lock"),
            "panel template catalog transaction",
        )?;
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
        // Fail-closed GPU deployment policy (#1490): no silent CPU lenses.
        require_gpu_lens_admission(&template, "save")?;
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

    pub(in crate::panel_commands) fn fork(
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

    pub(in crate::panel_commands) fn profile(
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

    pub(in crate::panel_commands) fn list(&self) -> CliResult<Vec<TemplateSummary>> {
        let catalog = self.read_catalog()?;
        catalog
            .templates
            .iter()
            .map(|entry| {
                let active = version_ref(entry, &entry.active_template_id)?;
                // Listing is an inventory operation, not a deployment read.
                // Admit the immediately preceding schema only so the operator
                // can see exactly which immutable object needs an explicit
                // refresh; ordinary loads remain fail-closed.
                let template =
                    self.read_object_for_refresh(&active.object_path, &active.blake3_hex)?;
                let migration_required = template.schema_version != OBJECT_VERSION;
                let a37 = template.a37_admission();
                Ok(TemplateSummary {
                    name: entry.name.clone(),
                    active_template_id: entry.active_template_id.clone(),
                    version: template.version,
                    object_schema_version: template.schema_version,
                    migration_required,
                    refresh_command: migration_required.then(|| {
                        format!(
                            "calyx panel template refresh --template {} --home {}",
                            entry.active_template_id,
                            self.root
                                .parent()
                                .and_then(Path::parent)
                                .unwrap_or(&self.root)
                                .display()
                        )
                    }),
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

    pub(in crate::panel_commands) fn load(&self, selector: &str) -> CliResult<SavedPanelTemplate> {
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

    /// Load either the current object schema or the immediately preceding
    /// schema exclusively for the explicit `template refresh` migration path.
    /// All ordinary consumers continue to fail closed on legacy objects.
    pub(in crate::panel_commands) fn load_for_refresh(
        &self,
        selector: &str,
    ) -> CliResult<SavedPanelTemplate> {
        let catalog = self.read_catalog()?;
        if let Some(entry) = catalog
            .templates
            .iter()
            .find(|entry| entry.name == selector)
        {
            let active = version_ref(entry, &entry.active_template_id)?;
            return self.read_object_for_refresh(&active.object_path, &active.blake3_hex);
        }
        for entry in &catalog.templates {
            if let Some(version) = entry
                .versions
                .iter()
                .find(|version| version.template_id == selector)
            {
                return self.read_object_for_refresh(&version.object_path, &version.blake3_hex);
            }
        }
        Err(template_error(
            TEMPLATE_NOT_FOUND,
            format!("panel template {selector} is not saved"),
            "save or seed the panel template before selecting it",
        ))
    }

    pub(in crate::panel_commands) fn swap_into_vault(
        &self,
        selector: &str,
        vault_dir: &Path,
        now_ms: u64,
        require_a37_gate: bool,
        resident_addr: std::net::SocketAddr,
    ) -> CliResult<TemplateSwap> {
        let mut template = self.load(selector)?;
        template.validate()?;
        // GPU deployment policy (#1490): swap is the second admission boundary.
        require_gpu_lens_admission(&template, "swap")?;
        if require_a37_gate {
            template.require_a37_gate()?;
        }
        let a37 = template.a37_admission();
        let template_id = id_for_loaded(&template)?;
        let mut state = load_vault_panel_state(vault_dir)?;
        let (registered_lenses_added, resident_attestation) =
            register_template_lenses_from_resident(
                &mut state.registry,
                &mut template,
                resident_addr,
            )?;
        let mut panel = state.panel;
        let target = template.to_target_panel(now_ms);
        let diff = swap_panel_to_target(&mut panel, &target, now_ms);
        // Swap is the boundary that publishes the panel; it must not persist a
        // constellation that ingest cannot then read (#1947). Assert the same
        // "active slot => registered frozen lens" invariant the built-in
        // materializer enforces, here, where the offending slot is still local.
        ensure_active_slots_are_registered(&template.name, &panel, &state.registry)?;
        let write = persist_vault_panel_state(vault_dir, &panel, &state.registry)?;
        readback::verify_persisted_swap(vault_dir, &panel, &state.registry, &write.panel_ref)?;
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
            resident_attestation: Some(resident_attestation),
            readback_verified: true,
        })
    }

    pub(super) fn read_catalog(&self) -> CliResult<PanelTemplateCatalog> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(PanelTemplateCatalog {
                schema_version: CATALOG_VERSION,
                templates: Vec::new(),
            });
        }
        let catalog: PanelTemplateCatalog = serde_json::from_slice(&fs::read(&path)?)
            .map_err(|error| CliError::runtime(format!("parse template catalog: {error}")))?;
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
        let bytes = serde_json::to_vec_pretty(catalog)
            .map_err(|error| CliError::runtime(format!("serialize template catalog: {error}")))?;
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
                format!(
                    "template object {} hash mismatch: expected={expected} actual={actual}",
                    path.display()
                ),
                "do not edit immutable template objects; re-save the template",
            ));
        }
        let template: SavedPanelTemplate = serde_json::from_slice(&bytes)
            .map_err(|error| CliError::runtime(format!("parse template object: {error}")))?;
        template.validate_with_id(expected)?;
        Ok(template)
    }

    fn read_object_for_refresh(
        &self,
        object_path: &str,
        expected: &str,
    ) -> CliResult<SavedPanelTemplate> {
        let path = self.root.join(object_path);
        let bytes = fs::read(&path)?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != expected {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "template object {} hash mismatch before refresh: expected={expected} actual={actual}",
                    path.display()
                ),
                "restore the immutable source object from a verified backup before migration",
            ));
        }
        let template: SavedPanelTemplate = serde_json::from_slice(&bytes)
            .map_err(|error| CliError::runtime(format!("parse template object: {error}")))?;
        template.validate_refresh_source(expected)?;
        Ok(template)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    fn object_path(&self, template_id: &str) -> PathBuf {
        self.root.join(object_rel_path(template_id))
    }
}

/// Refuses to publish a panel whose active content slots are not fully backed by
/// a frozen registry entry.
///
/// Mirrors both checks `cmd::ingest::command::structured_preflight::active_lenses`
/// performs, including its compute-on-recall exemption, so a swap can never hand
/// ingest a constellation it must reject: every active content slot's lens must be
/// registered *and* carry a frozen runtime spec.
pub(super) fn ensure_active_slots_are_registered(
    template: &str,
    panel: &Panel,
    registry: &Registry,
) -> CliResult {
    let unbacked = panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
        // Compute-on-recall sidecars are exempt by the same shared predicate ingest
        // uses, so swap and ingest cannot disagree about what must be registered.
        .filter(|slot| !slot.is_compute_on_recall_sidecar())
        .filter(|slot| {
            !registry.contains(slot.lens_id) || registry.lens_spec(slot.lens_id).is_none()
        })
        .map(|slot| {
            format!(
                "slot={} key={} lens={}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id
            )
        })
        .collect::<Vec<_>>();
    if unbacked.is_empty() {
        return Ok(());
    }
    Err(CalyxError {
        code: calyx_registry::CALYX_PANEL_LENS_MISSING,
        message: format!(
            "panel template {template} swap would publish active slots without a frozen registry \
             entry: [{}]",
            unbacked.join("; ")
        ),
        remediation: "register the frozen lens for every active slot, or materialize \
                      compute-on-recall sidecars as parked, before swapping the template",
    }
    .into())
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
