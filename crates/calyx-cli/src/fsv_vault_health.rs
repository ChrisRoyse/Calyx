//! Fail-closed vault readiness checks for Full State Verification workflows.

use std::path::{Path, PathBuf};

use calyx_aster::base_page_index::{read_base_page_index_manifest, read_indexed_base_rows};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::manifest::{ManifestStore, VaultManifest};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::CalyxError;
use calyx_registry::audit_vault_registry_contracts;
use calyx_search::PersistedSearchIndexes;
use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use crate::durable_write::write_json_value_atomic;
use crate::error::{CliError, CliResult};
use crate::fsv_grounding::{
    ANCHOR_CF_DRIFT_CODE, GROUNDING_FLAG_DRIFT_CODE, NO_GROUNDED_CANDIDATES_CODE,
};
use crate::fsv_vault_health_grounding::check_grounded_candidates;
use crate::fsv_vault_health_marker::{REBUILD_REQUIRED_CODE, check_search_rebuild_marker};
use crate::fsv_vault_health_quarantine::{
    QUARANTINE_FILE, QUARANTINED_CODE, check_marker as check_quarantine_marker,
    write_marker as write_quarantine_marker,
};
use crate::output::print_json;

pub(crate) const REPORT_SCHEMA: &str = "calyx.fsv.vault_health.v1";
pub(crate) const SOURCE_OF_TRUTH: &str = "vault CURRENT/MANIFEST, registry snapshot asset, Base/anchors CF grounding rows, idx/search/manifest.json, idx/search/rebuild-required.json, base_page_index_v1/manifest.json, and fsv_quarantine.json";
const UNREADY_CODE: &str = "CALYX_FSV_VAULT_UNREADY";
const REGISTRY_CONTRACT_DRIFT_CODE: &str = "CALYX_REGISTRY_CONTRACT_DRIFT";
pub(crate) const REMEDIATION: &str = "repair stale derived indexes with explicit rebuild commands or restore the real missing persisted registry snapshot before using this vault for FSV";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) vault: String,
    pub(crate) out: Option<PathBuf>,
    pub(crate) write_quarantine: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct VaultHealthReport {
    pub(crate) schema: &'static str,
    pub(crate) source_of_truth: &'static str,
    pub(crate) vault_ref: String,
    pub(crate) vault_dir: String,
    pub(crate) vault_id: String,
    pub(crate) vault_name: String,
    pub(crate) fsv_ready: bool,
    pub(crate) quarantine_required: bool,
    pub(crate) quarantine_marker_path: String,
    pub(crate) quarantine_marker_sha256: Option<String>,
    pub(crate) checks: Vec<VaultHealthCheck>,
    pub(crate) repair_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct VaultHealthCheck {
    pub(crate) name: &'static str,
    pub(crate) status: &'static str,
    pub(crate) code: Option<String>,
    pub(crate) message: String,
    pub(crate) remediation: Option<String>,
    pub(crate) details: Value,
}

pub(crate) fn run(args: &[String]) -> CliResult {
    let args = parse_args(args)?;
    let home = home_dir()?;
    let resolved = resolve_vault_info(&home, &args.vault)?;
    let mut report = build_report(&args.vault, &resolved)?;

    if args.write_quarantine && !report.fsv_ready {
        let write = write_quarantine_marker(&report)?;
        report.quarantine_marker_sha256 = Some(write.sha256_hex);
    }
    if let Some(path) = &args.out {
        let value = serde_json::to_value(&report)?;
        write_json_value_atomic(path, &value, "fsv vault-health report")?;
    }
    print_json(&report)?;
    if report.fsv_ready {
        return Ok(());
    }
    Err(CliError::Calyx(CalyxError {
        code: UNREADY_CODE,
        message: format!(
            "vault {} is not ready for Full State Verification; failed checks: {}",
            report.vault_ref,
            failed_check_codes(&report.checks).join(",")
        ),
        remediation: REMEDIATION,
    }))
}

pub(crate) fn parse_args(args: &[String]) -> CliResult<Args> {
    let mut vault = None;
    let mut out = None;
    let mut write_quarantine = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--vault" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    CliError::usage("usage: calyx fsv vault-health --vault <vault> [--out <json>] [--write-quarantine]")
                })?;
                vault = Some(value.clone());
            }
            "--out" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    CliError::usage("usage: calyx fsv vault-health --vault <vault> [--out <json>] [--write-quarantine]")
                })?;
                out = Some(PathBuf::from(value));
            }
            "--write-quarantine" => write_quarantine = true,
            other => {
                return Err(CliError::usage(format!(
                    "unknown fsv vault-health argument {other}; usage: calyx fsv vault-health --vault <vault> [--out <json>] [--write-quarantine]"
                )));
            }
        }
        index += 1;
    }
    let vault = vault.ok_or_else(|| {
        CliError::usage(
            "usage: calyx fsv vault-health --vault <vault> [--out <json>] [--write-quarantine]",
        )
    })?;
    Ok(Args {
        vault,
        out,
        write_quarantine,
    })
}

fn build_report(vault_ref: &str, resolved: &ResolvedVault) -> CliResult<VaultHealthReport> {
    let quarantine_path = resolved.path.join(QUARANTINE_FILE);
    let manifest_result = ManifestStore::open(&resolved.path).load_current();
    let latest_seq_result = open_latest_seq(resolved);
    let checks = vec![
        check_quarantine_marker(&quarantine_path),
        check_manifest(&manifest_result),
        check_registry_ref(&manifest_result),
        check_registry_contracts(&resolved.path),
        check_grounded_candidates(resolved),
        check_search_index(&resolved.path, latest_seq_result),
        check_search_rebuild_marker(&resolved.path),
        check_base_page_index(&resolved.path),
    ];

    let repair_actions = repair_actions(&checks);
    let fsv_ready = checks.iter().all(|check| check.status == "ok");
    Ok(VaultHealthReport {
        schema: REPORT_SCHEMA,
        source_of_truth: SOURCE_OF_TRUTH,
        vault_ref: vault_ref.to_string(),
        vault_dir: resolved.path.display().to_string(),
        vault_id: resolved.vault_id.to_string(),
        vault_name: resolved.name.clone(),
        fsv_ready,
        quarantine_required: !fsv_ready,
        quarantine_marker_path: quarantine_path.display().to_string(),
        quarantine_marker_sha256: None,
        checks,
        repair_actions,
    })
}

struct VaultSeqReadback {
    latest_seq: u64,
    derived_content_seq: u64,
}

fn open_latest_seq(resolved: &ResolvedVault) -> Result<VaultSeqReadback, CliError> {
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            restore_ledger_hook: false,
            read_only: true,
            selected_cfs: Some(vec![ColumnFamily::Base]),
            ..VaultOptions::default()
        },
    )?;
    Ok(VaultSeqReadback {
        latest_seq: vault.latest_seq(),
        derived_content_seq: vault.derived_content_seq(),
    })
}

fn check_manifest(result: &calyx_core::Result<VaultManifest>) -> VaultHealthCheck {
    match result {
        Ok(manifest) => ok(
            "manifest_current",
            "current manifest loaded and immutable refs verified",
            manifest_details(manifest),
        ),
        Err(error) => failed_from_calyx(
            "manifest_current",
            error,
            json!({"source": "CURRENT and pointed MANIFEST"}),
        ),
    }
}

fn check_registry_ref(result: &calyx_core::Result<VaultManifest>) -> VaultHealthCheck {
    match result {
        Ok(manifest) => match &manifest.registry_ref {
            Some(reference) => ok(
                "registry_snapshot_ref",
                "manifest contains a persisted registry snapshot ref",
                json!({"logical_path": reference.logical_path, "blake3_hex": reference.blake3_hex}),
            ),
            None => failed(
                "registry_snapshot_ref",
                "CALYX_ASTER_CORRUPT_SHARD",
                "vault manifest has no persisted registry snapshot ref".to_string(),
                "restore the real registry snapshot and manifest ref; do not synthesize a registry snapshot from guesses",
                manifest_details(manifest),
            ),
        },
        Err(error) => failed_from_calyx(
            "registry_snapshot_ref",
            error,
            json!({"blocked_by": "manifest_current"}),
        ),
    }
}

fn check_registry_contracts(vault_dir: &Path) -> VaultHealthCheck {
    match audit_vault_registry_contracts(vault_dir) {
        Ok(audit) if audit.valid => ok(
            "registry_contracts",
            "persisted registry contracts match runtime reconstruction",
            json!({"checked_count": audit.checked_count, "diff_count": audit.diffs.len()}),
        ),
        Ok(audit) => failed(
            "registry_contracts",
            REGISTRY_CONTRACT_DRIFT_CODE,
            format!(
                "persisted registry contracts drifted for {} of {} lenses",
                audit.diffs.len(),
                audit.checked_count
            ),
            "run `calyx panel registry-repair --vault <vault> --slot <slot>` after inspecting the emitted registry diff",
            json!({"checked_count": audit.checked_count, "diff_count": audit.diffs.len()}),
        ),
        Err(error) => failed_from_calyx(
            "registry_contracts",
            &error,
            json!({"source": "manifest registry_ref and persisted registry snapshot"}),
        ),
    }
}

fn check_search_index(
    vault_dir: &Path,
    latest_seq_result: Result<VaultSeqReadback, CliError>,
) -> VaultHealthCheck {
    let seqs = match latest_seq_result {
        Ok(seqs) => seqs,
        Err(error) => {
            return failed_from_cli(
                "search_index_freshness",
                &error,
                json!({"source": "read-only Base CF vault open"}),
            );
        }
    };
    let (latest_seq, derived_content_seq) = (seqs.latest_seq, seqs.derived_content_seq);
    let indexes = match PersistedSearchIndexes::open(vault_dir) {
        Ok(indexes) => indexes,
        Err(error) => {
            let error: CliError = error.into();
            return failed(
                "search_index_freshness",
                error.code(),
                error.message().to_string(),
                "run `calyx rebuild-search-index <vault>` and rerun vault-health before search or FSV",
                json!({"source": "idx/search/manifest.json", "pinned_vault_seq": latest_seq}),
            );
        }
    };
    let base_seq = indexes.base_seq();
    let details = json!({
        "search_manifest_base_seq": base_seq,
        "pinned_vault_seq": latest_seq,
        "derived_content_seq": derived_content_seq,
    });
    // Same Fresh doctrine as PersistedSearchIndexes::ensure_fresh_at_snapshot
    // (issue #1100): fresh iff derived_content_seq <= base_seq <= latest_seq.
    if derived_content_seq <= base_seq && base_seq <= latest_seq {
        return ok(
            "search_index_freshness",
            "persistent search index covers every derived-content commit at the pinned vault sequence",
            details,
        );
    }
    let message = if base_seq > latest_seq {
        format!(
            "persistent search manifest base seq {base_seq} is ahead of pinned vault seq {latest_seq}"
        )
    } else {
        format!(
            "persistent search manifest base seq {base_seq} is behind derived content seq {derived_content_seq} (pinned vault seq {latest_seq})"
        )
    };
    failed(
        "search_index_freshness",
        "CALYX_STALE_DERIVED",
        message,
        "run `calyx rebuild-search-index <vault>` and rerun vault-health before search or FSV",
        details,
    )
}

fn check_base_page_index(vault_dir: &Path) -> VaultHealthCheck {
    let manifest = match read_base_page_index_manifest(vault_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            return failed_from_calyx(
                "base_page_index",
                &error,
                json!({"source": "base_page_index_v1/manifest.json"}),
            );
        }
    };
    match read_indexed_base_rows(vault_dir, 1) {
        Ok(rows) => ok(
            "base_page_index",
            "Base page index manifest and sampled source rows read back successfully",
            json!({
                "ledger_head_height": manifest.ledger_head_height,
                "ledger_head_tip_hash_hex": manifest.ledger_head_tip_hash_hex,
                "live_entries": manifest.live_entries,
                "pages": manifest.pages.len(),
                "sampled_live_rows": rows.len()
            }),
        ),
        Err(error) => failed_from_calyx(
            "base_page_index",
            &error,
            json!({
                "ledger_head_height": manifest.ledger_head_height,
                "ledger_head_tip_hash_hex": manifest.ledger_head_tip_hash_hex,
                "live_entries": manifest.live_entries,
                "pages": manifest.pages.len()
            }),
        ),
    }
}

pub(crate) fn ok(
    name: &'static str,
    message: impl Into<String>,
    details: Value,
) -> VaultHealthCheck {
    VaultHealthCheck {
        name,
        status: "ok",
        code: None,
        message: message.into(),
        remediation: None,
        details,
    }
}

pub(crate) fn failed(
    name: &'static str,
    code: &'static str,
    message: String,
    remediation: &'static str,
    details: Value,
) -> VaultHealthCheck {
    VaultHealthCheck {
        name,
        status: "failed",
        code: Some(code.to_string()),
        message,
        remediation: Some(remediation.to_string()),
        details,
    }
}

pub(crate) fn failed_from_calyx(
    name: &'static str,
    error: &CalyxError,
    details: Value,
) -> VaultHealthCheck {
    failed(
        name,
        error.code,
        error.message.clone(),
        error.remediation,
        details,
    )
}

pub(crate) fn failed_from_cli(
    name: &'static str,
    error: &CliError,
    details: Value,
) -> VaultHealthCheck {
    failed(
        name,
        error.code(),
        error.message().to_string(),
        error.remediation(),
        details,
    )
}

fn manifest_details(manifest: &VaultManifest) -> Value {
    json!({
        "manifest_seq": manifest.manifest_seq,
        "durable_seq": manifest.durable_seq,
        "panel_ref": {
            "logical_path": manifest.panel_ref.logical_path,
            "blake3_hex": manifest.panel_ref.blake3_hex
        },
        "registry_ref": manifest.registry_ref.as_ref().map(|reference| json!({
            "logical_path": reference.logical_path,
            "blake3_hex": reference.blake3_hex
        }))
    })
}

fn repair_actions(checks: &[VaultHealthCheck]) -> Vec<String> {
    let mut actions = Vec::new();
    if has_code(checks, "CALYX_STALE_DERIVED") || has_code(checks, REBUILD_REQUIRED_CODE) {
        actions.push("run `calyx rebuild-search-index <vault>` and rerun vault-health".to_string());
    }
    if has_code(checks, NO_GROUNDED_CANDIDATES_CODE)
        || has_code(checks, ANCHOR_CF_DRIFT_CODE)
        || has_code(checks, GROUNDING_FLAG_DRIFT_CODE)
    {
        actions.push("ingest or replay real anchored content, rebuild derived indexes, and rerun vault-health".to_string());
    }
    if has_code(checks, "CALYX_BASE_PAGE_INDEX_MISSING")
        || has_code(checks, "CALYX_BASE_PAGE_INDEX_STALE")
    {
        actions.push("run `calyx readback cx-list --vault <vault> --limit <n> --rebuild-base-page-index` and rerun vault-health".to_string());
    }
    if has_code(checks, "CALYX_ASTER_CORRUPT_SHARD") {
        actions.push("restore the real persisted registry snapshot and manifest registry_ref before using the vault for FSV".to_string());
    }
    if has_code(checks, QUARANTINED_CODE) {
        actions.push("leave the vault quarantined until all other source-of-truth checks pass on a fresh vault-health run".to_string());
    }
    actions
}

fn has_code(checks: &[VaultHealthCheck], code: &str) -> bool {
    checks
        .iter()
        .any(|check| check.code.as_deref() == Some(code))
}

fn failed_check_codes(checks: &[VaultHealthCheck]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.status != "ok")
        .map(|check| {
            check
                .code
                .clone()
                .unwrap_or_else(|| format!("{}:failed", check.name))
        })
        .collect()
}
