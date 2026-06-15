use std::fs;
use std::path::Path;

use calyx_core::{CalyxError, SlotId, VaultStore};
use calyx_registry::{PanelTemplate, instantiate_panel};

use super::{PanelReceipt, VaultType, removal_failed, sync_file, sync_parent};
use crate::error::CliError;
use crate::leapable::ShadowVault;
use crate::leapable::dual_write::aster_dir;
use crate::leapable::panel_guard_enable::{PanelGuardEnable, PanelSpec};
use crate::migrate;
use crate::migrate::backfill::BackfillMode;
use crate::migrate::manifest::{MigrationManifest, now_ms, panel_path};
use crate::migrate::reader::{ChunkRow, open_sqlite, stream_rows};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DefaultPanelOptions {
    pub(crate) backfill: bool,
    pub(crate) batch_size: usize,
    pub(crate) expected_base_lens_id: Option<String>,
}

impl Default for DefaultPanelOptions {
    fn default() -> Self {
        Self {
            backfill: true,
            batch_size: 64,
            expected_base_lens_id: None,
        }
    }
}

pub(crate) struct DefaultPanels;

impl DefaultPanels {
    pub(crate) fn install(
        vault: &mut ShadowVault,
        vault_type: VaultType,
    ) -> Result<PanelReceipt, CalyxError> {
        Self::install_with_options(vault, vault_type, &DefaultPanelOptions::default())
    }

    pub(crate) fn install_with_options(
        vault: &mut ShadowVault,
        vault_type: VaultType,
        options: &DefaultPanelOptions,
    ) -> Result<PanelReceipt, CalyxError> {
        let template = vault_type.template();
        let (_, calyx_dir) = vault.paths();
        let aster_dir = aster_dir(calyx_dir);
        let mut manifest = MigrationManifest::load(&aster_dir)?;
        if let Some(expected) = &options.expected_base_lens_id
            && expected != &manifest.base_lens_id
        {
            return Err(CalyxError::lens_frozen_violation(format!(
                "expected base LensId {expected}, found {}",
                manifest.base_lens_id
            )));
        }

        let conn = open_sqlite(vault.sqlite_read_path()).map_err(cli_to_calyx)?;
        let rows = stream_rows(&conn).map_err(cli_to_calyx)?;
        let receipt = if vault_type == VaultType::Text && options.backfill {
            install_text_with_backfill(vault, vault_type, &template, &aster_dir, options)?
        } else {
            install_lazy(vault_type, &template, &aster_dir, &manifest, &rows)?
        };

        manifest.panel_template = receipt.template.clone();
        manifest.panel_version = receipt.lens_count as u32;
        manifest.write(&aster_dir)?;
        vault.set_mode_with_features(
            vault.mode(),
            &[
                ("default_panel_installed", "true".to_string()),
                ("default_panel_vault_type", vault_type.as_str().to_string()),
                ("default_panel_template", receipt.template.clone()),
                ("default_panel_lens_count", receipt.lens_count.to_string()),
                (
                    "default_panel_backfill_pending",
                    receipt.backfill_pending.to_string(),
                ),
                ("panel_lens_count", receipt.lens_count.to_string()),
            ],
        )?;
        Ok(receipt)
    }
}

fn install_text_with_backfill(
    vault: &mut ShadowVault,
    vault_type: VaultType,
    template: &PanelTemplate,
    aster_dir: &Path,
    options: &DefaultPanelOptions,
) -> Result<PanelReceipt, CalyxError> {
    let panel = PanelGuardEnable::enable(
        vault,
        &PanelSpec {
            backfill: true,
            batch_size: options.batch_size,
            mode: BackfillMode::OfflineDeterministic,
            expected_base_lens_id: options.expected_base_lens_id.clone(),
        },
    )?;
    Ok(PanelReceipt {
        vault_type,
        template: template.name.clone(),
        lens_count: panel.panel_lens_count,
        backfill_pending: 0,
        panel_path: panel_path(aster_dir),
    })
}

fn install_lazy(
    vault_type: VaultType,
    template: &PanelTemplate,
    aster_dir: &Path,
    manifest: &MigrationManifest,
    rows: &[ChunkRow],
) -> Result<PanelReceipt, CalyxError> {
    write_instantiated_panel(aster_dir, template)?;
    let aster = migrate::open_vault(aster_dir, manifest)?;
    let adapter = migrate::adapter(manifest)?;
    Ok(PanelReceipt {
        vault_type,
        template: template.name.clone(),
        lens_count: template.slots.len(),
        backfill_pending: missing_slot_rows(&aster, rows, &adapter, template)?,
        panel_path: panel_path(aster_dir),
    })
}

fn write_instantiated_panel(vault_dir: &Path, template: &PanelTemplate) -> Result<(), CalyxError> {
    let instantiated = instantiate_panel(template, now_ms());
    let path = panel_path(vault_dir);
    let bytes = serde_json::to_vec_pretty(&instantiated.panel)
        .map_err(|error| removal_failed(format!("encode panel: {error}")))?;
    fs::write(&path, bytes)
        .map_err(|error| removal_failed(format!("write {}: {error}", path.display())))?;
    sync_file(&path)?;
    sync_parent(&path)
}

fn missing_slot_rows(
    aster: &calyx_aster::vault::AsterVault,
    rows: &[ChunkRow],
    adapter: &crate::migrate::adapter::VaultSqliteAdapter,
    template: &PanelTemplate,
) -> Result<usize, CalyxError> {
    let snapshot = aster.snapshot();
    let mut missing = 0;
    for row in rows {
        let cx_id = adapter.cx_id(row);
        for idx in 0..template.slots.len() {
            if aster
                .read_slot_vector_at(snapshot, cx_id, SlotId::new(idx as u16))?
                .is_none()
            {
                missing += 1;
            }
        }
    }
    Ok(missing)
}

fn cli_to_calyx(error: CliError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.message().to_string(),
        remediation: error.remediation(),
    }
}
