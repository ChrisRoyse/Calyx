use super::*;

#[derive(Serialize)]
struct BatchLimitReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    manifest_seq: u64,
    durable_seq: u64,
    registry_ref: String,
    wrote_manifest: bool,
    requested_count: usize,
    changed_count: usize,
    preflight_count: usize,
    changes: Vec<BatchLimitChangeReport>,
}

#[derive(Serialize)]
struct BatchLimitChangeReport {
    lens_id: String,
    name: String,
    before: Option<usize>,
    after: usize,
    changed: bool,
    active_slot_count: usize,
    reloaded_max_batch: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    preflight: Option<BatchLimitPreflightReport>,
}

#[derive(Clone, Debug, Serialize)]
struct BatchLimitPreflightReport {
    input_count: usize,
    runtime_batch_limit: Option<usize>,
    effective_chunk_size: usize,
    chunk_count: usize,
    runtime_load_ms: u128,
    measure_ms: u128,
    total_ms: u128,
}

pub(super) fn batch_limit(args: &[String]) -> CliResult {
    let flags = BatchLimitFlags::parse(args)?;
    let state = load_vault_panel_state(&flags.vault)?;
    let snapshot = state.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(
            "vault has no persisted registry snapshot; cannot update lens batch limits",
        ))
    })?;
    let updates = resolve_batch_limit_updates(snapshot, &flags.sets)?;
    let mut preview_snapshot = snapshot.clone();
    let preview_changes = apply_registry_snapshot_batch_limits(&mut preview_snapshot, &updates)?;
    let preflight = preflight_batch_limit_changes(&preview_snapshot, &preview_changes, &flags)?;
    let write = set_vault_registry_batch_limits(&flags.vault, &updates)?;
    let reloaded = load_vault_panel_state(&flags.vault)?;
    let changes = verify_batch_limit_write(&flags.vault, &reloaded, &write.changes, &preflight)?;
    print_json(&BatchLimitReport {
        status: "batch_limits_updated",
        source_of_truth: "vault MANIFEST registry_ref plus manifest-backed registry asset reloaded via load_vault_panel_state",
        vault: flags.vault,
        manifest_seq: write.manifest_seq,
        durable_seq: write.durable_seq,
        registry_ref: write.registry_ref.logical_path,
        wrote_manifest: write.wrote_manifest,
        requested_count: updates.len(),
        changed_count: write.changes.iter().filter(|change| change.changed).count(),
        preflight_count: preflight.len(),
        changes,
    })
}

fn resolve_batch_limit_updates(
    snapshot: &VaultRegistrySnapshot,
    sets: &[BatchLimitSet],
) -> CliResult<Vec<RegistryBatchLimitUpdate>> {
    let mut updates = Vec::with_capacity(sets.len());
    for set in sets {
        let lens_id = resolve_batch_limit_selector(snapshot, &set.selector)?;
        updates.push(RegistryBatchLimitUpdate {
            lens_id,
            max_batch: set.max_batch,
        });
    }
    Ok(updates)
}

fn resolve_batch_limit_selector(
    snapshot: &VaultRegistrySnapshot,
    selector: &str,
) -> CliResult<LensId> {
    if let Ok(lens_id) = LensId::from_str(selector) {
        return Ok(lens_id);
    }
    let matches = snapshot
        .lenses
        .iter()
        .filter_map(|lens| {
            lens.spec
                .as_ref()
                .filter(|spec| spec.name == selector)
                .map(|_| lens.lens_id)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [lens_id] => Ok(*lens_id),
        [] => Err(CliError::usage(format!(
            "batch-limit selector {selector} did not match a persisted lens name or LensId"
        ))),
        _ => Err(CliError::usage(format!(
            "batch-limit selector {selector} matched multiple persisted lenses; use LensId"
        ))),
    }
}

fn preflight_batch_limit_changes(
    snapshot: &VaultRegistrySnapshot,
    changes: &[RegistryBatchLimitChange],
    flags: &BatchLimitFlags,
) -> CliResult<Vec<(LensId, BatchLimitPreflightReport)>> {
    let mut reports = Vec::new();
    for change in changes.iter().filter(|change| change.changed) {
        let lens = snapshot
            .lenses
            .iter()
            .find(|lens| lens.lens_id == change.lens_id)
            .ok_or_else(|| {
                CliError::from(CalyxError::lens_unreachable(format!(
                    "preflight lens {} missing from preview registry snapshot",
                    change.lens_id
                )))
            })?;
        let modality = lens.contract.modality();
        if modality != Modality::Text {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_BATCH_LIMIT_PREFLIGHT_UNSUPPORTED",
                message: format!(
                    "batch-limit preflight currently supports Text lenses, but {} ({}) is {:?}",
                    change.name, change.lens_id, modality
                ),
                remediation: "add a modality-specific preflight input generator before changing this non-text lens batch limit",
            }));
        }
        let input_count = flags.preflight_repeat.unwrap_or(change.after).max(1);
        let inputs = (0..input_count)
            .map(|idx| {
                Input::new(
                    Modality::Text,
                    format!("{} #{idx}", flags.preflight_text).into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let (_, stats) =
            measure_registry_snapshot_lens_batch_with_stats(lens, &inputs, Some(change.after))?;
        reports.push((change.lens_id, BatchLimitPreflightReport::from(stats)));
    }
    Ok(reports)
}

fn verify_batch_limit_write(
    vault: &Path,
    reloaded: &calyx_registry::VaultPanelState,
    changes: &[RegistryBatchLimitChange],
    preflight: &[(LensId, BatchLimitPreflightReport)],
) -> CliResult<Vec<BatchLimitChangeReport>> {
    let snapshot = reloaded.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "vault {} lost registry snapshot after batch-limit write",
            vault.display()
        )))
    })?;
    let mut reports = Vec::with_capacity(changes.len());
    for change in changes {
        let lens = snapshot
            .lenses
            .iter()
            .find(|lens| lens.lens_id == change.lens_id)
            .ok_or_else(|| {
                CliError::from(CalyxError::aster_corrupt_shard(format!(
                    "vault {} reloaded registry is missing changed lens {}",
                    vault.display(),
                    change.lens_id
                )))
            })?;
        let reloaded_max_batch = lens
            .spec
            .as_ref()
            .and_then(|spec| spec.max_batch)
            .ok_or_else(|| {
                CliError::from(CalyxError::aster_corrupt_shard(format!(
                    "vault {} reloaded lens {} has no max_batch after batch-limit write",
                    vault.display(),
                    change.lens_id
                )))
            })?;
        if reloaded_max_batch != change.after {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_BATCH_LIMIT_VERIFY_FAILED",
                message: format!(
                    "vault {} reloaded lens {} max_batch={}, expected {}",
                    vault.display(),
                    change.lens_id,
                    reloaded_max_batch,
                    change.after
                ),
                remediation: "inspect the vault MANIFEST registry_ref and retry the batch-limit command after repairing registry persistence",
            }));
        }
        let active_slot_count = reloaded
            .panel
            .slots
            .iter()
            .filter(|slot| slot.lens_id == change.lens_id && slot.state == SlotState::Active)
            .count();
        reports.push(BatchLimitChangeReport {
            lens_id: change.lens_id.to_string(),
            name: change.name.clone(),
            before: change.before,
            after: change.after,
            changed: change.changed,
            active_slot_count,
            reloaded_max_batch,
            preflight: preflight
                .iter()
                .find(|(lens_id, _)| *lens_id == change.lens_id)
                .map(|(_, report)| report.clone()),
        });
    }
    Ok(reports)
}

struct BatchLimitFlags {
    vault: PathBuf,
    sets: Vec<BatchLimitSet>,
    preflight_text: String,
    preflight_repeat: Option<usize>,
}

struct BatchLimitSet {
    selector: String,
    max_batch: usize,
}

impl BatchLimitFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut sets = Vec::new();
        let mut preflight_text = "calyx batch-limit preflight".to_string();
        let mut preflight_repeat = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(value(args, idx, "--vault")?.into());
                }
                "--set" => {
                    idx += 1;
                    sets.push(parse_batch_limit_set(value(args, idx, "--set")?)?);
                }
                "--preflight-text" => {
                    idx += 1;
                    preflight_text = value(args, idx, "--preflight-text")?.to_string();
                    if preflight_text.is_empty() {
                        return Err(CliError::usage("--preflight-text must not be empty"));
                    }
                }
                "--preflight-repeat" => {
                    idx += 1;
                    let raw = value(args, idx, "--preflight-repeat")?;
                    let parsed = raw.parse::<usize>().map_err(|error| {
                        CliError::usage(format!("parse --preflight-repeat {raw}: {error}"))
                    })?;
                    if parsed == 0 {
                        return Err(CliError::usage("--preflight-repeat must be > 0"));
                    }
                    preflight_repeat = Some(parsed);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel batch-limit flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        let vault = vault.ok_or_else(|| CliError::usage("panel batch-limit requires --vault"))?;
        if sets.is_empty() {
            return Err(CliError::usage(
                "panel batch-limit requires at least one --set <name-or-id>=<max_batch>",
            ));
        }
        Ok(Self {
            vault,
            sets,
            preflight_text,
            preflight_repeat,
        })
    }
}

fn parse_batch_limit_set(raw: &str) -> CliResult<BatchLimitSet> {
    let (selector, max_batch) = raw
        .split_once('=')
        .ok_or_else(|| CliError::usage("--set must use <name-or-id>=<max_batch>"))?;
    if selector.is_empty() {
        return Err(CliError::usage("--set selector must not be empty"));
    }
    let max_batch = max_batch
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("parse batch limit {raw}: {error}")))?;
    if max_batch == 0 {
        return Err(CliError::usage("--set max_batch must be > 0"));
    }
    Ok(BatchLimitSet {
        selector: selector.to_string(),
        max_batch,
    })
}

impl From<RegistrySnapshotMeasureStats> for BatchLimitPreflightReport {
    fn from(stats: RegistrySnapshotMeasureStats) -> Self {
        Self {
            input_count: stats.input_count,
            runtime_batch_limit: stats.runtime_batch_limit,
            effective_chunk_size: stats.effective_chunk_size,
            chunk_count: stats.chunk_count,
            runtime_load_ms: stats.runtime_load_ms,
            measure_ms: stats.measure_ms,
            total_ms: stats.total_ms,
        }
    }
}
