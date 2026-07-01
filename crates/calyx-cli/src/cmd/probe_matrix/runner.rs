use std::path::Path;
use std::time::Instant;

use calyx_aster::vault::AsterVault;
use calyx_core::SlotId;
use calyx_lodestar::{
    ProbeLength, ProbeMatrixSpec, ProbePhrasing, ProbeRecord, build_probe_matrix,
};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};
use serde_json::json;

use super::artifact::{
    MatrixArtifactWriter, error_details, incomplete_error, matrix_log, timeout_with_artifacts,
};
use super::diagnostics::{ProbeMatrixArtifactStatus, QueryVectorCache};
use super::persist::{persist_probe_matrix, persist_probe_matrix_at_path};
use super::progress;
use super::resident;
use super::support::with_persisted_artifact_error;
use super::{
    ProbeMatrixArgs, ensure_useful_log, probe_read_vault_options, probe_variant,
    selected_active_slots, validate_response,
};
use crate::bounded_progress::Deadline;
use crate::cmd::vault::{resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn run_probe_matrix_with_home(home: &Path, args: ProbeMatrixArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    let mut progress =
        progress::ProbeMatrixProgressWriter::create(&resolved.path, &resolved.name, &args)?;
    eprintln!(
        "probe-matrix: opening physical vault name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    progress.write("running", "panel_load_start", json!({}))?;
    let state = load_vault_panel_state(&resolved.path)?;
    progress.write(
        "running",
        "panel_load_complete",
        json!({
            "panel_slots": state.panel.slots.len(),
            "registry_lenses": state
                .registry_snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.lenses.len()),
        }),
    )?;
    let active_slots = match selected_active_slots(&args, &state.panel) {
        Ok(slots) => slots,
        Err(error) => {
            let _ = progress.write(
                "failed",
                "slot_validation_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    let spec = ProbeMatrixSpec {
        frontier: args.frontier.clone(),
        active_slots,
        weighted_profiles: if args.weighted_profiles.is_empty() {
            ProbeMatrixSpec::new(&args.frontier, vec![SlotId::new(0)]).weighted_profiles
        } else {
            args.weighted_profiles.clone()
        },
        phrasings: if args.phrasings.is_empty() {
            ProbePhrasing::all()
        } else {
            args.phrasings.clone()
        },
        lengths: if args.lengths.is_empty() {
            ProbeLength::all()
        } else {
            args.lengths.clone()
        },
        top_k: args.top_k,
    };
    let variants = match build_probe_matrix(&spec) {
        Ok(variants) => variants,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "failed",
                "spec_validation_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    let matrix_path = args
        .out
        .clone()
        .unwrap_or_else(|| progress.run_dir().join("matrix.json"));
    if args.out.is_some() && matrix_path.exists() {
        let detail = format!(
            "refusing to overwrite existing probe matrix output {}",
            matrix_path.display()
        );
        progress.write("failed", "output_path_exists", json!({ "error": detail }))?;
        return Err(CliError::usage(detail));
    }
    let deadline = Deadline::new(args.time_budget_ms);
    let mut records = Vec::<ProbeRecord>::new();
    let mut query_cache = QueryVectorCache::new(spec.active_slots.iter().copied().collect());
    let mut guard_diagnostics = Vec::new();
    let artifacts = MatrixArtifactWriter::new(
        &matrix_path,
        &resolved,
        &spec,
        &args,
        variants.len(),
        progress.path(),
    );
    artifacts.persist_incomplete(
        &records,
        &query_cache,
        &guard_diagnostics,
        started.elapsed().as_millis(),
        "initialized",
    )?;
    progress.write(
        "running",
        "matrix_initialized",
        json!({
            "matrix_artifact": matrix_path.display().to_string(),
            "total_variant_count": variants.len(),
            "max_variants": args.max_variants,
            "time_budget_ms": args.time_budget_ms,
        }),
    )?;
    if let Err(error) = deadline.check("probe-matrix", "matrix_initialized", 0) {
        artifacts.persist_incomplete(
            &records,
            &query_cache,
            &guard_diagnostics,
            started.elapsed().as_millis(),
            "time_budget_exceeded",
        )?;
        progress.write(
            "incomplete",
            "time_budget_exceeded",
            json!({ "error": error_details(&error) }),
        )?;
        return Err(timeout_with_artifacts(
            &error,
            &matrix_path,
            progress.path(),
        ));
    }
    progress.write("running", "vault_open_start", json!({}))?;
    let vault = match AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        probe_read_vault_options(&state.panel, args.guard),
    ) {
        Ok(vault) => vault,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "failed",
                "vault_open_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    eprintln!(
        "probe-matrix: opened vault snapshot_seq={} elapsed_ms={}",
        vault.latest_seq(),
        started.elapsed().as_millis()
    );
    progress.write(
        "running",
        "vault_open_complete",
        json!({ "snapshot_seq": vault.latest_seq() }),
    )?;
    if let Err(error) = deadline.check("probe-matrix", "vault_open_complete", records.len() as u64)
    {
        artifacts.persist_incomplete(
            &records,
            &query_cache,
            &guard_diagnostics,
            started.elapsed().as_millis(),
            "time_budget_exceeded",
        )?;
        progress.write(
            "incomplete",
            "time_budget_exceeded",
            json!({ "error": error_details(&error) }),
        )?;
        return Err(timeout_with_artifacts(
            &error,
            &matrix_path,
            progress.path(),
        ));
    }
    super::grounding::GroundingPreflight {
        vault: &vault,
        spec: &spec,
        artifacts: &artifacts,
        records: &records,
        query_cache: &query_cache,
        guard_diagnostics: &guard_diagnostics,
        elapsed_ms: started.elapsed().as_millis(),
    }
    .run(&mut progress)?;
    let audit = match require_vault_registry_contracts(&resolved.path) {
        Ok(audit) => audit,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "failed",
                "registry_contract_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    eprintln!(
        "probe-matrix: registry contracts valid checked_count={} elapsed_ms={}",
        audit.checked_count,
        started.elapsed().as_millis()
    );
    if args.resident_addr.is_none()
        && let Err(error) =
            resident::require_resident_for_gpu_text_slots(&state, &spec.active_slots)
    {
        artifacts.persist_incomplete(
            &records,
            &query_cache,
            &guard_diagnostics,
            started.elapsed().as_millis(),
            "resident_required",
        )?;
        progress.write(
            "failed",
            "resident_required",
            json!({
                "error": error_details(&error),
                "matrix_artifact": matrix_path.display().to_string(),
            }),
        )?;
        return Err(error);
    }
    eprintln!(
        "probe-matrix: running frontier={:?} slots={} profiles={} phrasings={} lengths={} top_k={} guard={:?} rayon_threads={}",
        spec.frontier,
        spec.active_slots.len(),
        spec.weighted_profiles.len(),
        spec.phrasings.len(),
        spec.lengths.len(),
        spec.top_k,
        args.guard,
        rayon::current_num_threads()
    );

    for variant in variants.iter() {
        if args
            .max_variants
            .is_some_and(|max_variants| records.len() >= max_variants)
        {
            artifacts.persist_incomplete(
                &records,
                &query_cache,
                &guard_diagnostics,
                started.elapsed().as_millis(),
                "variant_budget_exhausted",
            )?;
            progress.write(
                "incomplete",
                "variant_budget_exhausted",
                json!({
                    "completed_variant_count": records.len(),
                    "total_variant_count": variants.len(),
                    "matrix_artifact": matrix_path.display().to_string(),
                }),
            )?;
            return Err(incomplete_error(
                "variant_budget_exhausted",
                &matrix_path,
                progress.path(),
            ));
        }
        if let Err(error) = deadline.check("probe-matrix", "before_variant", records.len() as u64) {
            artifacts.persist_incomplete(
                &records,
                &query_cache,
                &guard_diagnostics,
                started.elapsed().as_millis(),
                "time_budget_exceeded",
            )?;
            progress.write(
                "incomplete",
                "time_budget_exceeded",
                json!({ "error": error_details(&error) }),
            )?;
            return Err(timeout_with_artifacts(
                &error,
                &matrix_path,
                progress.path(),
            ));
        }
        let variant_started = Instant::now();
        progress.write(
            "running",
            "variant_start",
            json!({
                "variant_id": variant.id,
                "completed_variant_count": records.len(),
                "total_variant_count": variants.len(),
            }),
        )?;
        eprintln!(
            "probe-matrix: variant start fusion={:?} emphasis={:?} phrasing={:?} length={:?} top_k={}",
            variant.fusion, variant.lens_emphasis, variant.phrasing, variant.length, variant.top_k
        );
        let mut variant_ctx = super::ProbeVariantContext {
            state: &state,
            vault_dir: &resolved.path,
            guard: args.guard,
            query_cache: &mut query_cache,
            guard_diagnostics: &mut guard_diagnostics,
            resident_addr: args.resident_addr,
            deadline: &deadline,
        };
        let response = match probe_variant(&vault, variant, &mut variant_ctx) {
            Ok(response) => response,
            Err(error) if error.code() == "CALYX_CLI_TIMEOUT" => {
                artifacts.persist_incomplete(
                    &records,
                    &query_cache,
                    &guard_diagnostics,
                    started.elapsed().as_millis(),
                    "time_budget_exceeded",
                )?;
                progress.write(
                    "incomplete",
                    "time_budget_exceeded",
                    json!({ "variant_id": variant.id, "error": error_details(&error) }),
                )?;
                return Err(timeout_with_artifacts(
                    &error,
                    &matrix_path,
                    progress.path(),
                ));
            }
            Err(error) => {
                artifacts.persist_incomplete(
                    &records,
                    &query_cache,
                    &guard_diagnostics,
                    started.elapsed().as_millis(),
                    "variant_error",
                )?;
                let _ = progress.write(
                    "failed",
                    "variant_error",
                    json!({ "variant_id": variant.id, "error": error_details(&error) }),
                );
                return Err(error);
            }
        };
        if let Err(error) = validate_response(&response) {
            artifacts.persist_incomplete(
                &records,
                &query_cache,
                &guard_diagnostics,
                started.elapsed().as_millis(),
                "variant_validation_error",
            )?;
            let _ = progress.write(
                "failed",
                "variant_validation_error",
                json!({ "variant_id": variant.id, "error": error_details(&error) }),
            );
            return Err(error);
        }
        let accepted_hit_count = response.hits.iter().filter(|hit| hit.grounded).count();
        eprintln!(
            "probe-matrix: variant ok hits={} accepted_hits={} refusals={} elapsed_ms={}",
            response.hits.len(),
            accepted_hit_count,
            response.refusals.len(),
            variant_started.elapsed().as_millis()
        );
        records.push(ProbeRecord {
            variant: variant.clone(),
            hits: response.hits,
            refusals: response.refusals,
            accepted_hit_count,
            unique_grounded_hits: Vec::new(),
        });
        let persisted = artifacts.persist_run(
            &records,
            &query_cache,
            &guard_diagnostics,
            ProbeMatrixArtifactStatus::Incomplete,
            artifacts.run_state(
                records.len(),
                started.elapsed().as_millis(),
                false,
                Some("running"),
            ),
        )?;
        progress.write(
            "running",
            "variant_complete",
            json!({
                "variant_id": variant.id,
                "completed_variant_count": records.len(),
                "total_variant_count": variants.len(),
                "matrix_artifact": persisted.path.display().to_string(),
                "matrix_json_bytes": persisted.bytes,
                "matrix_json_sha256": persisted.sha256,
                "readback_record_count": persisted.readback_record_count,
            }),
        )?;
    }

    let log = matrix_log(&spec, &records);
    let status = ProbeMatrixArtifactStatus::from_log(&log);
    let run = artifacts.run_state(records.len(), started.elapsed().as_millis(), true, None);
    let artifact = artifacts.artifact_for(&query_cache, &guard_diagnostics, status, run, log);
    let run_persisted = persist_probe_matrix_at_path(&matrix_path, &artifact, true)?;
    let persisted = if args.out.is_some() {
        run_persisted
    } else {
        persist_probe_matrix(&resolved.path, None, &artifact)?
    };
    eprintln!(
        "probe-matrix: persisted matrix={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    progress.write(
        if status == ProbeMatrixArtifactStatus::Ok {
            "ok"
        } else {
            "refused"
        },
        "complete",
        json!({
            "matrix_artifact": persisted.path.display().to_string(),
            "run_matrix_artifact": matrix_path.display().to_string(),
            "matrix_json_bytes": persisted.bytes,
            "matrix_json_sha256": persisted.sha256,
            "readback_record_count": persisted.readback_record_count,
            "readback_productive_count": persisted.readback_productive_count,
            "readback_accepted_hit_count": persisted.readback_accepted_hit_count,
            "readback_refusal_count": persisted.readback_refusal_count,
        }),
    )?;
    if let Err(error) = ensure_useful_log(&artifact.log) {
        return Err(with_persisted_artifact_error(error, &persisted));
    }
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "artifact": artifact,
        "artifacts": {
            "matrix_json": persisted.path,
            "run_matrix_json": matrix_path,
            "progress_json": progress.path(),
            "matrix_json_bytes": persisted.bytes,
            "matrix_json_sha256": persisted.sha256,
            "readback": {
                "record_count": persisted.readback_record_count,
                "productive_count": persisted.readback_productive_count,
                "accepted_hit_count": persisted.readback_accepted_hit_count,
                "refusal_count": persisted.readback_refusal_count,
            }
        }
    }))
}
