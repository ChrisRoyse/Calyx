use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use super::ShadowVault;
use crate::migrate;
use crate::migrate::backfill::{BackfillMode, BackfillSummary, backfill_default_panel};
use crate::migrate::manifest::MigrationManifest;
use crate::migrate::reader::{open_sqlite, stream_rows};
use crate::migrate::verifier::verify_migration;

pub(crate) const CALYX_GUARD_TAU_NOT_CALIBRATED: &str = "CALYX_GUARD_TAU_NOT_CALIBRATED";

#[derive(Clone, Debug)]
pub(crate) struct PanelSpec {
    pub(crate) backfill: bool,
    pub(crate) batch_size: usize,
    pub(crate) mode: BackfillMode,
    pub(crate) expected_base_lens_id: Option<String>,
}

impl Default for PanelSpec {
    fn default() -> Self {
        Self {
            backfill: true,
            batch_size: 64,
            mode: BackfillMode::OfflineDeterministic,
            expected_base_lens_id: None,
        }
    }
}

impl PanelSpec {
    #[cfg(test)]
    pub(crate) fn without_backfill() -> Self {
        Self {
            backfill: false,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn expecting_lens(mut self, lens_id: impl Into<String>) -> Self {
        self.expected_base_lens_id = Some(lens_id.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PanelEnableReport {
    pub(crate) panel_lens_count: usize,
    pub(crate) grounding_gaps: Vec<String>,
    pub(crate) backfill: Option<BackfillSummary>,
}

pub(crate) struct PanelGuardEnable;

impl PanelGuardEnable {
    pub(crate) fn enable(
        vault: &mut ShadowVault,
        panel_spec: &PanelSpec,
    ) -> Result<PanelEnableReport> {
        let (_, calyx_dir) = vault.paths();
        let conn = open_sqlite(vault.sqlite_read_path()).map_err(cli_to_calyx)?;
        let rows = stream_rows(&conn).map_err(cli_to_calyx)?;
        let aster_dir = super::dual_write::aster_dir(calyx_dir);
        let manifest = MigrationManifest::load(&aster_dir)?;
        if let Some(expected) = &panel_spec.expected_base_lens_id
            && expected != &manifest.base_lens_id
        {
            return Err(CalyxError::lens_frozen_violation(format!(
                "expected base LensId {expected}, found {}",
                manifest.base_lens_id
            )));
        }
        let aster = migrate::open_vault(&aster_dir, &manifest)?;
        let adapter = migrate::adapter(&manifest)?;
        let backfill = if panel_spec.backfill {
            Some(backfill_default_panel(
                &aster,
                &aster_dir,
                &rows,
                &adapter,
                panel_spec.mode,
                panel_spec.batch_size,
            )?)
        } else {
            None
        };
        let verify = verify_migration(&aster, &rows, &adapter, false)?;
        let panel_lens_count = migrate::backfill::default_slot_ids().len();
        vault.set_mode_with_features(
            vault.mode(),
            &[
                ("panel_enabled", "true".to_string()),
                ("panel_lens_count", panel_lens_count.to_string()),
                (
                    "grounding_gaps",
                    serde_json::to_string(&verify.missing_backfill).map_err(|error| {
                        feature_error(format!("encode grounding gaps: {error}"))
                    })?,
                ),
            ],
        )?;
        Ok(PanelEnableReport {
            panel_lens_count,
            grounding_gaps: verify.missing_backfill,
            backfill,
        })
    }

    pub(crate) fn enable_kernel(vault: &mut ShadowVault) -> Result<()> {
        vault.set_mode_with_features(
            vault.mode(),
            &[
                ("kernel_enabled", "true".to_string()),
                ("kernel_route", "kernel_answer".to_string()),
            ],
        )
    }

    pub(crate) fn enable_guard(vault: &mut ShadowVault, tau: f32) -> Result<()> {
        Self::validate_guard_tau(tau)?;
        vault.set_mode_with_features(
            vault.mode(),
            &[
                ("guard_enabled", "true".to_string()),
                ("guard_tau", tau.to_string()),
            ],
        )
    }

    pub(crate) fn validate_guard_tau(tau: f32) -> Result<()> {
        if tau <= 0.0 || !tau.is_finite() {
            return Err(error(
                CALYX_GUARD_TAU_NOT_CALIBRATED,
                "guard tau must be finite and greater than zero",
                "rerun with the calibrated PH38 injection-corpus tau",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn ensure_flipped(vault: &ShadowVault) -> Result<()> {
        if vault.mode() >= super::VaultMode::Calyx {
            Ok(())
        } else {
            Err(error(
                "CALYX_VAULT_NOT_FLIPPED",
                "Ask is still routed to sqlite-vec shadow mode",
                "run calyx leapable read-flip before asking through Calyx",
            ))
        }
    }
}

fn cli_to_calyx(error: crate::error::CliError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.message().to_string(),
        remediation: error.remediation(),
    }
}

fn feature_error(message: impl Into<String>) -> CalyxError {
    error(
        "CALYX_VAULT_FEATURE_STATE_INVALID",
        message,
        "inspect the root MANIFEST feature map and retry the panel enable step",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
