use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::server::ResidentService;
use super::*;

use crate::cmd::vault::resolve_vault_info;
use crate::cmd::{open_search_vault, search_read_cfs};
use crate::path_identity::vault_template_source;
use calyx_search::{
    PersistedSearchIndexes, SearchBudget, SearchTraceEvent,
    search_outcome_with_query_vectors_freshness_cached_ledger_view,
    search_outcome_with_query_vectors_freshness_cached_ledger_view_exact_metadata,
};

pub(super) fn serve(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    service: &ResidentService,
) -> CliResult {
    let request = match read_frame(reader)
        .and_then(|bytes| decode_binary::<ResidentSearchBinaryRequest>(&bytes))
    {
        Ok(request) => request,
        Err(error) => return write_error(writer, CliError::from(error)),
    };
    let response = match execute(request, service) {
        Ok(response) => response,
        Err(error) => return write_error(writer, error),
    };
    write_response(writer, response)
}

fn execute(
    request: ResidentSearchBinaryRequest,
    service: &ResidentService,
) -> CliResult<ResidentSearchBinaryResponse> {
    if request.protocol_version != SEARCH_BINARY_PROTOCOL_VERSION {
        return Err(protocol_error(format!(
            "resident search protocol version {}, expected {}",
            request.protocol_version, SEARCH_BINARY_PROTOCOL_VERSION
        )));
    }
    let vault_path = service.vault_path.as_deref().ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_VAULT_REQUIRED",
            message: "resident search is unavailable because this service was started from a saved template instead of a vault".to_string(),
            remediation: "restart the resident with `calyx panel resident serve --vault <vault-path>`",
        })
    })?;
    let expected_template_source = vault_template_source(vault_path)?;
    if request.template_source != expected_template_source
        || service.state.template_source != expected_template_source
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_VAULT_MISMATCH",
            message: format!(
                "resident search requested template_source {:?}, service owns {:?}, expected {:?}",
                request.template_source, service.state.template_source, expected_template_source
            ),
            remediation: "restart the CLI and resident against the same canonical vault path",
        }));
    }

    let indexes = PersistedSearchIndexes::open(vault_path)?;
    let generation = indexes.generation()?;
    require_generation(
        &generation,
        &request.manifest_sha256,
        request.base_seq,
        "request",
    )?;

    let resolved = resolve_vault_info(&service.state.home, &vault_path.to_string_lossy())?;
    let state = service.search_contract_state.as_ref().ok_or_else(|| {
        protocol_error("resident vault search contract state was not loaded at startup")
    })?;
    let vault = open_search_vault(&resolved, search_read_cfs(state, request.guard))?;
    let started = Instant::now();
    let mut traces = Vec::new();
    let mut trace_sink = |event: SearchTraceEvent| {
        traces.push(ResidentSearchTrace {
            phase: event.phase.to_string(),
            slot: event.slot,
            elapsed_ms: event.elapsed_ms,
            count: event.count,
            detail: event.detail,
        });
    };
    let mut cache = service
        .search_slot_cache
        .lock()
        .map_err(|_| protocol_error("resident search slot cache lock poisoned"))?;
    let outcome = match request.metadata_query.as_deref() {
        Some(metadata_query) => {
            search_outcome_with_query_vectors_freshness_cached_ledger_view_exact_metadata(
                &vault,
                vault_path,
                &request.query_vectors,
                metadata_query,
                request.k,
                request.fusion,
                request.guard,
                None,
                request.guard_panel_version,
                request.filter.as_deref(),
                request.explain,
                request.freshness,
                SearchBudget::disabled(),
                Some(&mut cache),
                None,
                Some(&mut trace_sink),
            )
        }
        None => search_outcome_with_query_vectors_freshness_cached_ledger_view(
            &vault,
            vault_path,
            &request.query_vectors,
            request.k,
            request.fusion,
            request.guard,
            None,
            request.guard_panel_version,
            request.filter.as_deref(),
            request.explain,
            request.freshness,
            SearchBudget::disabled(),
            Some(&mut cache),
            None,
            Some(&mut trace_sink),
        ),
    }?;
    let observed = outcome.generation.as_ref().ok_or_else(|| {
        protocol_error("resident search outcome omitted its persisted generation")
    })?;
    require_generation(
        observed,
        &request.manifest_sha256,
        request.base_seq,
        "outcome",
    )?;
    let cache_diagnostic = cache.diagnostics();
    drop(cache);

    let outcome_json = serde_json::to_vec(&outcome)
        .map_err(|error| protocol_error(format!("encode resident search outcome JSON: {error}")))?;
    if outcome_json.len() > MAX_RESIDENT_SEARCH_OUTCOME_JSON_BYTES {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_OUTCOME_OVERSIZE",
            message: format!(
                "resident search outcome is {} bytes, above the {} byte protocol bound",
                outcome_json.len(),
                MAX_RESIDENT_SEARCH_OUTCOME_JSON_BYTES
            ),
            remediation: "reduce the bounded search k/explain payload before retrying",
        }));
    }
    let elapsed_ms = started.elapsed().as_millis();
    let cagra = calyx_sextant::index::cagra_serving_diagnostics();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=search_ok manifest_sha256={} base_seq={} query_slots={} hits={} elapsed_ms={} cache_hits={} cache_misses={} cagra_cache_entries={} cagra_resident_bytes={} cagra_max_bytes={} cagra_evictions={}",
        generation.manifest_sha256,
        generation.base_seq,
        request.query_vectors.len(),
        outcome.hits.len(),
        elapsed_ms,
        cache_diagnostic.hit_count,
        cache_diagnostic.miss_count,
        cagra.cache_entries,
        cagra.resident_bytes,
        cagra.resident_max_bytes,
        cagra.cache_evictions,
    );
    Ok(ResidentSearchBinaryResponse::Ok {
        protocol_version: SEARCH_BINARY_PROTOCOL_VERSION,
        process_id: std::process::id(),
        template_source: expected_template_source,
        manifest_sha256: generation.manifest_sha256,
        base_seq: generation.base_seq,
        elapsed_ms,
        outcome_json,
        traces,
        cache: cache_diagnostic,
    })
}

fn require_generation(
    generation: &PersistedSearchGeneration,
    manifest_sha256: &str,
    base_seq: u64,
    source: &str,
) -> CliResult {
    if generation.manifest_sha256 != manifest_sha256 || generation.base_seq != base_seq {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_GENERATION_MISMATCH",
            message: format!(
                "resident search {source} generation manifest={} base_seq={}, expected manifest={manifest_sha256} base_seq={base_seq}",
                generation.manifest_sha256, generation.base_seq
            ),
            remediation: "restart the resident against the current vault search generation and retry; never reuse a stale resident cache",
        }));
    }
    Ok(())
}

fn protocol_error(message: impl Into<String>) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_RESIDENT_SEARCH_PROTOCOL",
        message: message.into(),
        remediation: "restart the resident and CLI from the same Calyx build and current vault generation",
    })
}

fn write_error(writer: &mut dyn Write, error: CliError) -> CliResult {
    write_response(
        writer,
        ResidentSearchBinaryResponse::Err {
            code: error.code().to_string(),
            message: error.message().to_string(),
            remediation: error.remediation().to_string(),
        },
    )
}

fn write_response(writer: &mut dyn Write, response: ResidentSearchBinaryResponse) -> CliResult {
    let bytes = encode_binary(&response)?;
    Ok(write_frame(writer, &bytes)?)
}
