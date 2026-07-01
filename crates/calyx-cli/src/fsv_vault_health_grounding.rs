use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use serde_json::json;

use crate::cmd::vault::{ResolvedVault, vault_salt};
use crate::fsv_grounding::{
    ANCHOR_CF_DRIFT_CODE, GROUNDING_REMEDIATION, NO_GROUNDED_CANDIDATES_CODE, audit_grounding,
};
use crate::fsv_vault_health::{VaultHealthCheck, failed, failed_from_calyx, failed_from_cli};

pub(crate) fn check_grounded_candidates(resolved: &ResolvedVault) -> VaultHealthCheck {
    let vault = match AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            restore_ledger_hook: false,
            read_only: true,
            selected_cfs: Some(vec![ColumnFamily::Base, ColumnFamily::Anchors]),
            ..VaultOptions::default()
        },
    ) {
        Ok(vault) => vault,
        Err(error) => {
            return failed_from_calyx(
                "grounded_candidates",
                &error,
                json!({"source": "read-only Base and anchors CF vault open"}),
            );
        }
    };
    let audit = match audit_grounding(&vault, &[]) {
        Ok(audit) => audit,
        Err(error) => {
            return failed_from_cli(
                "grounded_candidates",
                &error,
                json!({"source": "pinned Base and anchors CF grounding audit"}),
            );
        }
    };
    if audit.missing_anchor_cf_row_count > 0 || audit.mismatched_anchor_cf_row_count > 0 {
        return failed(
            "grounded_candidates",
            ANCHOR_CF_DRIFT_CODE,
            format!(
                "vault grounding drifted: {} Base anchors missing from anchors CF and {} anchors CF rows mismatched at pinned seq {}",
                audit.missing_anchor_cf_row_count,
                audit.mismatched_anchor_cf_row_count,
                audit.pinned_seq
            ),
            GROUNDING_REMEDIATION,
            json!({"grounding": audit}),
        );
    }
    if audit.accepted_eligible_base_row_count == 0 {
        return failed(
            "grounded_candidates",
            NO_GROUNDED_CANDIDATES_CODE,
            format!(
                "vault has zero persisted anchor-eligible Base rows at pinned seq {}",
                audit.pinned_seq
            ),
            GROUNDING_REMEDIATION,
            json!({"grounding": audit}),
        );
    }
    crate::fsv_vault_health::ok(
        "grounded_candidates",
        "vault has persisted anchor-eligible Base rows for FSV",
        json!({"grounding": audit}),
    )
}
