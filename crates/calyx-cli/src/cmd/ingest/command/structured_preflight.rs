use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use calyx_core::{CalyxError, Input, LensId, Modality, SlotState};
use calyx_registry::LensSpec;

use super::*;

pub(super) fn measure_input(
    text: String,
    schema: Option<String>,
    record_path: Option<&Path>,
) -> CliResult<Input> {
    let input = Input::new(Modality::Text, text.into_bytes());
    match (schema, record_path) {
        (None, None) => Ok(input),
        (Some(schema), Some(path)) => {
            let bytes = std::fs::read(path)?;
            let value = serde_json::from_slice(&bytes).map_err(|error| CalyxError {
                code: calyx_core::CALYX_STRUCTURED_RECORD_INVALID,
                message: format!(
                    "measure record file {} is invalid JSON: {error}",
                    path.display()
                ),
                remediation: "provide one complete typed JSON object and retry",
            })?;
            Ok(
                input.with_structured(calyx_core::StructuredRecord::from_json_value(
                    schema, &value,
                )?),
            )
        }
        _ => Err(CalyxError {
            code: calyx_core::CALYX_STRUCTURED_SCHEMA_MISMATCH,
            message: "measure requires --record-schema and --record together".to_string(),
            remediation: "supply both the exact stable schema id and its JSON object file",
        }
        .into()),
    }
}

/// Resolves every active lens over every batch row and validates required
/// structured bindings before GPU admission, session state, or vault writes.
///
/// Reads the batch through a [`HashingReader`] and verifies the digest of the
/// bytes it consumed against the immutable snapshot (issue #1923) so a mutation
/// between validation and structured preflight fails here, before GPU admission
/// or any vault write.
pub(super) fn preflight_batch_lenses(
    path: &Path,
    state: &VaultPanelState,
    identity: &batch_identity::BatchSnapshotIdentity,
) -> CliResult<Vec<LensSpec>> {
    let active_lenses = active_lenses(state)?;
    let mut applicable = BTreeMap::new();

    let file = std::fs::File::open(path)
        .map_err(|error| CliError::io(format!("open batch {}: {error}", path.display())))?;
    let mut reader = std::io::BufReader::new(batch_identity::HashingReader::new(file));
    let mut observed_rows = 0usize;
    for (index, line) in (&mut reader).lines().enumerate() {
        let line =
            line.map_err(|error| CliError::io(format!("read batch line {}: {error}", index + 1)))?;
        let Some(row) = parse_batch_line(index, &line)? else {
            continue;
        };
        observed_rows = observed_rows.saturating_add(1);
        let mut input = Input::new(Modality::Text, row.text.into_bytes());
        if let Some(record) = row.structured {
            input = input.with_structured(record);
        }
        for (lens_id, spec, requires_structured) in &active_lenses {
            if *requires_structured {
                state.registry.measure(*lens_id, &input).map_err(|error| {
                    CliError::from(CalyxError {
                        code: error.code,
                        message: format!(
                            "batch JSONL line {} failed structured lens {} preflight: {}",
                            index + 1,
                            lens_id,
                            error.message
                        ),
                        remediation: error.remediation,
                    })
                })?;
            }
            if state.registry.accepts_input(*lens_id, &input)? {
                applicable.insert(*lens_id, spec.clone());
            }
        }
    }

    // Cheap fast-fail first, then the authoritative digest of the exact bytes read.
    identity.verify_row_count(path, observed_rows, "structured preflight")?;
    let observed = reader.into_inner().finish();
    identity.verify_observed(path, &observed, "structured preflight")?;
    Ok(applicable.into_values().collect())
}

fn active_lenses(state: &VaultPanelState) -> CliResult<Vec<(LensId, LensSpec, bool)>> {
    let mut lenses = Vec::new();
    for slot in &state.panel.slots {
        if slot.state != SlotState::Active {
            continue;
        }
        // Compute-on-recall sidecars carry no frozen registry model by design and
        // are measured as NotApplicable at ingest, so they are not preflightable
        // lenses. Demanding a registry entry here rejected every panel carrying
        // temporal time controls (#1947). Content slots are unaffected.
        if slot.is_compute_on_recall_sidecar() {
            continue;
        }
        if !state.registry.contains(slot.lens_id) {
            return Err(CalyxError {
                code: calyx_registry::CALYX_PANEL_LENS_MISSING,
                message: format!(
                    "active slot {} ({}) references unregistered lens {} during batch preflight",
                    slot.slot_id.get(),
                    slot.slot_key.key(),
                    slot.lens_id
                ),
                remediation: "restore the frozen registry entry for every active slot or park the slot before retrying; never ingest a partial constellation",
            }
            .into());
        }
        let spec = state.registry.lens_spec(slot.lens_id).cloned().ok_or_else(|| {
            CliError::from(CalyxError {
                code: calyx_registry::CALYX_PANEL_LENS_MISSING,
                message: format!(
                    "active slot {} ({}) lens {} has no frozen runtime spec during batch preflight",
                    slot.slot_id.get(),
                    slot.slot_key.key(),
                    slot.lens_id
                ),
                remediation: "restore the exact frozen LensSpec for every active slot or park the slot before retrying; GPU admission cannot be inferred from a missing runtime contract",
            })
        })?;
        let requires_structured = state.registry.requires_structured_record(slot.lens_id)?;
        lenses.push((slot.lens_id, spec, requires_structured));
    }
    lenses.sort_unstable_by_key(|(lens_id, _, _)| *lens_id);
    lenses.dedup_by_key(|(lens_id, _, _)| *lens_id);
    Ok(lenses)
}
