//! `calyx weave-loom <vault>` — corpus-scale Loom weave (#870).
//!
//! Populates the **XTerm CF** with within-doc cross-lens agreement cross-terms,
//! and the **graph CF** with the between-doc directed k-NN association graph
//! (nodes = constellations, edges = panel-measured nearest neighbours via the
//! persisted DiskANN index). Emits the acceptance report: XTerm rows persisted,
//! the corpus slot-pair agreement graph, and the association graph's
//! node/edge/groundedness counts. Fail-closed throughout — no fallbacks.

mod between_doc;
mod coverage;
mod csr;
mod fsv;
mod graph_contract;
mod panel_cache;
mod parse;
mod passes;
mod progress;
mod scratch;

use std::time::Instant;

use calyx_aster::cf::{
    WEAVE_XTERM_GENERATION_SCHEMA, WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
    weave_xterm_generation_prefix_range,
};
use calyx_aster::plain_graph::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationState,
    GraphCollectionGenerationStatus, GraphCollectionLifecycle, PhysicalGraphCollectionLifecycle,
    PhysicalPlainGraph, PlainGraph, graph_generation_storage_collection,
};
use calyx_aster::resource::heap_rss_bytes;
use calyx_aster::vault::{AsterVault, IngestCompactionConfig, VaultOptions};
use calyx_core::{CalyxError, SlotId, SlotShape, SlotState, SystemClock};
use calyx_lodestar::{CorpusWeaveReportParams, PANEL_ASTER_ASSOC_COLLECTION, corpus_weave_report};
use calyx_registry::load_vault_stored_vector_panel_state;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Subcommand;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};
use crate::output::print_json;
pub(crate) use coverage::CandidateSelectionMode;
use progress::error_details;

const DEFAULT_KNN: usize = 16;
const DEFAULT_EDGE_SCORE_THRESHOLD: f32 = 0.5;
const DEFAULT_MAX_GROUNDEDNESS_DISTANCE: usize = 3;
const DEFAULT_BATCH: usize = 512;
const XTERM_READBACK_PAGE_ROWS: usize = 2048;
const DERIVED_COMPACTION_TRIGGER_FILES: usize = 12;
const DERIVED_COMPACTION_ROUND_MAX_FILES: usize = 4_096;
const DERIVED_COMPACTION_ROUND_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Optional fail-closed peak-RSS budget for the whole weave run, in bytes. When
/// set, weave-loom refuses (fail closed, `CALYX_BACKPRESSURE`) before the two
/// heaviest phases (vault open, between-doc graph) if the projected peak
/// resident set would exceed it — instead of being OOM-killed mid-run.
const WEAVE_RSS_BUDGET_ENV: &str = "CALYX_WEAVE_RSS_BUDGET_BYTES";

/// Converts every ordinary error/panic after generation creation into a
/// durable `failed` lifecycle row. A hard process/host crash intentionally
/// leaves `writing`, which remains the distinct, diagnosable crash signal.
struct WeaveGenerationFailureGuard<'guard, 'vault> {
    lifecycle: &'guard GraphCollectionLifecycle<'vault, SystemClock>,
    vault: &'vault AsterVault,
    generation: String,
    storage_collection: String,
    armed: bool,
}

impl WeaveGenerationFailureGuard<'_, '_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WeaveGenerationFailureGuard<'_, '_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let failed = GraphCollectionGenerationState::new(
            PANEL_ASTER_ASSOC_COLLECTION,
            self.generation.clone(),
            GraphCollectionGenerationStatus::Failed,
            "weave-loom",
        )
        .with_reason(
            "weave-loom exited before accepted generation physical readback completed; inspect WEAVE_LOOM_PROGRESS for the typed phase error",
        )
        .with_detail(
            GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
            GRAPH_GENERATION_STORAGE_SCHEMA,
        )
        .with_detail(
            GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
            self.storage_collection.clone(),
        )
        .with_detail(
            WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
            WEAVE_XTERM_GENERATION_SCHEMA,
        );
        if let Err(error) = self
            .lifecycle
            .put_state(&failed)
            .and_then(|_| self.vault.flush())
        {
            eprintln!(
                "CALYX_WEAVE_LIFECYCLE {}",
                json!({
                    "event": "failed_generation_state_persist_error",
                    "generation": self.generation,
                    "storage_collection": self.storage_collection,
                    "error": error.to_string(),
                })
            );
        }
    }
}

pub(crate) use parse::parse_weave_loom;

/// Device that generates the between-doc k-NN **candidate** set (#1963). This is
/// an *explicit* operator choice, never auto-selected: `Cpu` keeps the existing
/// implicit serving path (manifest-backend + threshold gated); `Gpu` forces the
/// GPU search kernel per lane and fails closed if it cannot be honoured. Either
/// way the persisted edge evidence is the bit-stable CPU rerank over the union,
/// so the device only affects which candidate pairs are considered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateDevice {
    Cpu,
    Gpu,
}

impl CandidateDevice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CandidateDevice::Cpu => "cpu",
            CandidateDevice::Gpu => "gpu",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WeaveLoomArgs {
    pub vault: String,
    pub content_slot: Option<u16>,
    pub knn: usize,
    pub edge_score_threshold: f32,
    pub max_groundedness_distance: usize,
    pub batch: usize,
    /// Cap the number of constellations processed (0 = all). For bounded FSV
    /// runs; the report records the cap so partial runs are never read as full.
    pub limit: usize,
    /// Which deterministic source-of-truth candidate set to materialize.
    pub candidate_selection: CandidateSelectionMode,
    /// Device used to generate the between-doc k-NN candidate set (#1963).
    /// Explicit operator choice; defaults to CPU (existing behaviour).
    pub candidate_device: CandidateDevice,
    /// Stop after coverage selection and print the selected candidate report.
    pub coverage_only: bool,
    /// Internal wall-clock budget. If exceeded, persist an incomplete progress
    /// artifact and return CALYX_CLI_TIMEOUT before an outer supervisor kill.
    pub time_budget_ms: Option<u64>,
}

impl Default for WeaveLoomArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            content_slot: None,
            knn: DEFAULT_KNN,
            edge_score_threshold: DEFAULT_EDGE_SCORE_THRESHOLD,
            max_groundedness_distance: DEFAULT_MAX_GROUNDEDNESS_DISTANCE,
            batch: DEFAULT_BATCH,
            limit: 0,
            candidate_selection: CandidateSelectionMode::BasePrefix,
            candidate_device: CandidateDevice::Cpu,
            coverage_only: false,
            time_budget_ms: None,
        }
    }
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::WeaveLoom(args) = command else {
        unreachable!("non-weave command routed to weave module");
    };
    run_weave_loom(args)
}

/// Phase-tagged peak-RSS + timing profiler for the weave run.
///
/// Samples the kernel's `VmRSS` (via [`heap_rss_bytes`], the in-repo `/proc`
/// probe) at each milestone, tracks the running high-water mark, and writes a
/// phase-tagged record into the progress artifact so a killed run still leaves
/// its memory/timing trail on disk. Instrumentation is best-effort: a probe
/// failure is recorded verbatim (never silently dropped) and does not fail the
/// weave — the memory *contract* is enforced separately by the fail-closed
/// budget guard ([`enforce_rss_budget`]).
struct RssTracker {
    start: Instant,
    peak_bytes: u64,
    samples: Vec<serde_json::Value>,
}

impl RssTracker {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            peak_bytes: 0,
            samples: Vec::new(),
        }
    }

    fn sample(
        &mut self,
        progress: &progress::WeaveLoomProgressWriter,
        phase: &str,
        budget: Option<u64>,
        scratch_bytes: u64,
        nodes: Option<u64>,
    ) -> CliResult {
        let elapsed_ms = self.start.elapsed().as_millis();
        // Optional `nodes=N` field for sub-phase (mid-pass) samples so a killed
        // run localizes growth to a node offset, not just a coarse phase (#1991
        // round-4). Empty for phase-boundary samples.
        let nodes_field = nodes.map(|n| format!(" nodes={n}")).unwrap_or_default();
        // Always-on stderr runtime line (#1991), same structured convention as
        // CALYX_INGEST_RUNTIME / CALYX_KERNEL_ANSWER_RUNTIME: a killed or
        // OOM-terminated run still leaves the per-phase peak-RSS trail on the
        // host's stderr so boundedness is provable and any residual O(rows x
        // slots) term is localized to a named phase. `scratch_bytes` reports the
        // out-of-core vector store's page-cache footprint so the anon-vs-file
        // split is legible. RSS is the OS physical source of truth via
        // `heap_rss_bytes`.
        match heap_rss_bytes() {
            Ok(bytes) => {
                self.peak_bytes = self.peak_bytes.max(bytes);
                eprintln!(
                    "CALYX_WEAVE_RUNTIME phase={phase}{nodes_field} elapsed_ms={elapsed_ms} rss_bytes={bytes} peak_rss_bytes={} scratch_bytes={scratch_bytes}",
                    self.peak_bytes
                );
                let detail = json!({
                    "phase": phase,
                    "nodes": nodes,
                    "rss_bytes": bytes,
                    "peak_rss_bytes": self.peak_bytes,
                    "scratch_bytes": scratch_bytes,
                    "elapsed_ms": elapsed_ms,
                });
                self.samples.push(detail.clone());
                // Live-RSS budget guard (#1991 fix 3): the budget is enforced
                // against the ACTUAL sampled resident set at every phase
                // boundary, not just a preflight estimate. Exceeding it fails
                // closed with a typed CALYX_BACKPRESSURE error and an incomplete
                // progress artifact instead of marching into a kernel OOM-kill.
                if let Some(budget) = budget
                    && bytes > budget
                {
                    let error: CliError = CalyxError::backpressure(format!(
                        "weave-loom live RSS {bytes} bytes exceeded {WEAVE_RSS_BUDGET_ENV}={budget} bytes at phase {phase}; \
                         out-of-core scratch keeps the vector payload in reclaimable page cache, so an anon-RSS overrun here \
                         is real working-set growth — raise the budget for the complete graph or use a bounded --limit verification run"
                    ))
                    .into();
                    let _ = progress.write(
                        "incomplete",
                        "rss_budget_live_exceeded",
                        json!({
                            "error": error_details(&error),
                            "phase": phase,
                            "rss_bytes": bytes,
                            "budget_bytes": budget,
                            "scratch_bytes": scratch_bytes,
                        }),
                    );
                    return Err(error);
                }
                progress.write("running", "rss_sample", json!({ "rss": detail }))
            }
            Err(error) => {
                eprintln!(
                    "CALYX_WEAVE_RUNTIME phase={phase}{nodes_field} elapsed_ms={elapsed_ms} rss_bytes=probe_error peak_rss_bytes={} scratch_bytes={scratch_bytes} probe_error_code={} probe_error_message={:?}",
                    self.peak_bytes, error.code, error.message
                );
                let detail = json!({
                    "phase": phase,
                    "rss_bytes": serde_json::Value::Null,
                    "peak_rss_bytes": self.peak_bytes,
                    "scratch_bytes": scratch_bytes,
                    "elapsed_ms": elapsed_ms,
                    "probe_error": error.code,
                    "probe_error_message": error.message,
                });
                self.samples.push(detail.clone());
                // With a budget configured, a probe failure cannot prove the
                // budget is honoured, so fail closed (same contract as the
                // preflight guard). With no budget the sample is best-effort
                // observability and never gates the weave.
                if budget.is_some() {
                    let error: CliError = error.into();
                    let _ = progress.write(
                        "incomplete",
                        "rss_budget_probe_unavailable",
                        json!({
                            "error": error_details(&error),
                            "phase": phase,
                        }),
                    );
                    return Err(error);
                }
                progress.write("running", "rss_sample", json!({ "rss": detail }))
            }
        }
    }

    fn report(&self) -> serde_json::Value {
        json!({
            "peak_rss_bytes": self.peak_bytes,
            "elapsed_ms": self.start.elapsed().as_millis(),
            "budget_env": WEAVE_RSS_BUDGET_ENV,
            "samples": &self.samples,
        })
    }
}

impl Drop for RssTracker {
    /// Emit the cumulative `total` runtime line on every exit path — success,
    /// fail-closed refusal, `?`-propagated error, or a deadline abort — so the
    /// peak-RSS trail is never lost even when the weave does not reach its
    /// final phase (#1991). An OS SIGKILL (kernel OOM) still cannot run Drop, so
    /// the per-phase lines emitted eagerly in `sample` remain the authoritative
    /// evidence for a killed run.
    fn drop(&mut self) {
        eprintln!(
            "CALYX_WEAVE_RUNTIME phase=total elapsed_ms={} peak_rss_bytes={}",
            self.start.elapsed().as_millis(),
            self.peak_bytes
        );
    }
}

/// Reads the optional per-run peak-RSS budget from the environment. A malformed
/// or zero value fails closed as a usage error rather than being ignored.
fn configured_rss_budget() -> CliResult<Option<u64>> {
    match std::env::var(WEAVE_RSS_BUDGET_ENV) {
        Ok(raw) => Ok(Some(parse_rss_budget(&raw)?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(CliError::usage(format!(
            "{WEAVE_RSS_BUDGET_ENV} value is not valid UTF-8"
        ))),
    }
}

/// Parses a raw budget string into a positive byte count, failing closed on a
/// non-numeric or zero value (pure, env-free, so the contract is unit-testable).
fn parse_rss_budget(raw: &str) -> CliResult<u64> {
    let bytes = raw.trim().parse::<u64>().map_err(|error| {
        CliError::usage(format!(
            "{WEAVE_RSS_BUDGET_ENV}={raw:?} must be a positive byte count: {error}"
        ))
    })?;
    if bytes == 0 {
        return Err(CliError::usage(format!(
            "{WEAVE_RSS_BUDGET_ENV} must be a positive byte budget; got 0"
        )));
    }
    Ok(bytes)
}

/// Pure budget decision: returns the projected peak RSS iff it exceeds `budget`.
/// `None` means "within budget" (or no budget). Uses saturating arithmetic so a
/// pathological additive estimate can never wrap the comparison.
fn projected_peak_over_budget(
    current_bytes: u64,
    additional_bytes: u64,
    budget: Option<u64>,
) -> Option<u64> {
    let budget = budget?;
    let projected = current_bytes.saturating_add(additional_bytes);
    (projected > budget).then_some(projected)
}

/// Bounded projection of the resident typed-vector working set for the read
/// passes (#1991). With the out-of-core scratch store the corpus payload lives
/// in reclaimable page cache, so the *anonymous* working set is only the batch
/// of node panels within-doc decodes plus the per-node candidate set between-doc
/// reconstructs — never `O(corpus)`. This sizes that bound from the single
/// largest panel record summed across slots (`max_panel_record_bytes`) times the
/// batch, a deliberately conservative over-estimate used only for the fail-closed
/// preflight; the live-RSS sample guard is the authoritative bound.
fn projected_read_working_set_bytes(preflight: &coverage::DenseSlotPreflight, batch: usize) -> u64 {
    let per_panel = preflight.scratch.max_panel_record_bytes();
    // within-doc holds `batch` panels; between-doc holds one query panel plus its
    // bounded candidate set. `batch + 1` conservatively covers the larger of the
    // two, and doubling absorbs decode/allocator overhead per panel.
    per_panel.saturating_mul(batch as u64 + 1).saturating_mul(2)
}

/// Fail-closed peak-RSS pre-flight. When a budget is configured, refuses (with
/// `CALYX_BACKPRESSURE`) before `phase` if the current resident set plus the
/// estimated additional working set would exceed the budget — persisting an
/// `incomplete` progress artifact instead of proceeding toward an OOM kill.
/// With no budget configured it is a no-op. A probe failure fails closed (the
/// budget cannot be proven honoured) rather than silently skipping the guard.
fn enforce_rss_budget(
    progress: &progress::WeaveLoomProgressWriter,
    phase: &str,
    budget: Option<u64>,
    projected_additional_bytes: u64,
) -> CliResult {
    let Some(budget) = budget else {
        return Ok(());
    };
    let current = heap_rss_bytes()?;
    let projected_peak = current.saturating_add(projected_additional_bytes);
    if projected_peak_over_budget(current, projected_additional_bytes, Some(budget)).is_some() {
        let error: CliError = CalyxError::backpressure(format!(
            "weave-loom refused before {phase}: projected peak RSS {projected_peak} bytes \
             (current {current} + estimated additional working set {projected_additional_bytes}) \
             exceeds {WEAVE_RSS_BUDGET_ENV}={budget} bytes; raise the budget for the complete graph or use a bounded --limit verification run"
        ))
        .into();
        let _ = progress.write(
            "incomplete",
            "rss_budget_refused",
            json!({
                "error": error_details(&error),
                "phase": phase,
                "current_rss_bytes": current,
                "projected_additional_bytes": projected_additional_bytes,
                "projected_peak_rss_bytes": projected_peak,
                "budget_bytes": budget,
            }),
        );
        return Err(error);
    }
    progress.write(
        "running",
        "rss_budget_ok",
        json!({
            "phase": phase,
            "current_rss_bytes": current,
            "projected_additional_bytes": projected_additional_bytes,
            "projected_peak_rss_bytes": projected_peak,
            "budget_bytes": budget,
        }),
    )
}

fn run_weave_loom(args: WeaveLoomArgs) -> CliResult {
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let progress =
        progress::WeaveLoomProgressWriter::create(&resolved.path, &resolved.name, &args)?;
    // Fail fast on a malformed budget before any work is done.
    let rss_budget = configured_rss_budget()?;
    let mut rss = RssTracker::new();
    rss.sample(&progress, "run_start", rss_budget, 0, None)?;
    progress.write("running", "panel_load_start", json!({}))?;
    let state = load_vault_stored_vector_panel_state(&resolved.path)?;
    let content_slots = content_lens_slots(&state.panel);
    let incompatible_content_slots = incompatible_content_lens_slots(&state.panel);
    progress.write(
        "running",
        "panel_load_complete",
        json!({
            "panel_capability": "stored_vectors_only",
            "runtime_probe_required": false,
            "registry_lenses": state.registry_snapshot.lenses.len(),
            "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
            "skipped_incompatible_content_slots": incompatible_content_slots,
        }),
    )?;
    if content_slots.len() < 2 {
        let detail = format!(
            "weave-loom needs >=2 active typed content lenses (state=Active, not retrieval_only); panel has {}; incompatible active content slots={:?}",
            content_slots.len(),
            incompatible_content_slots
        );
        progress.write(
            "coverage_failed",
            "panel_content_slots",
            json!({ "error": detail }),
        )?;
        return Err(CliError::usage(detail));
    }
    if args.content_slot.is_some() {
        let error = CliError::usage(
            "--content-slot would collapse the association graph to one lens; omit it so weave-loom binds every active dense content slot",
        );
        let _ = progress.write(
            "coverage_failed",
            "content_slot_validation",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }

    if args.limit == 1 {
        let detail = "weave-loom --limit must be 0 or at least 2 because graph selection needs at least two constellations";
        progress.write(
            "coverage_failed",
            "limit_validation",
            json!({ "error": detail }),
        )?;
        return Err(CliError::usage(detail));
    }

    let deadline = Deadline::new(args.time_budget_ms);
    progress.write("running", "coverage_preflight_start", json!({}))?;
    let scan = match coverage::scan_dense_slot_coverage(
        &resolved.path,
        progress.run_dir(),
        &content_slots,
        None,
        args.limit,
        args.candidate_selection,
        &deadline,
    ) {
        Ok(scan) => scan,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "coverage_preflight_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    progress.write(
        "running",
        "coverage_preflight_complete",
        json!({
            "constellations_in_vault": scan.constellations_in_vault,
            "candidate_scan_rows": scan.candidate_scan_rows,
            "candidate_scan_complete": scan.candidate_scan_complete,
            "base_page_index_live_entries": scan.base_page_index_live_entries,
            "candidate_order": "base_page_index_key_order",
            "candidate_selection_mode": args.candidate_selection.as_str(),
            "coverage": &scan.coverage,
        }),
    )?;
    // Total page-cache footprint of the out-of-core vector store, fixed once the
    // scan finishes; reported as `scratch_bytes` on every subsequent phase line.
    let scratch_bytes = scan.scratch.total_record_bytes();
    rss.sample(
        &progress,
        "coverage_scan_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;

    let preflight = match coverage::materialize_panel_preflight(scan, &content_slots, args.limit) {
        Ok(preflight) => preflight,
        Err(detail) => {
            progress.write(
                "coverage_failed",
                "dense_slot_coverage",
                json!({
                    "error": detail,
                    "progress_artifact": progress.path().display().to_string(),
                }),
            )?;
            return Err(coverage::invalid_params(format!(
                "{detail}; coverage artifact persisted at {}",
                progress.path().display()
            )));
        }
    };
    progress.write(
        "running",
        "candidate_selection_complete",
        json!({
            "selection": "complete_panel_intersection",
            "embedding_slots": content_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
            "candidate_rows": preflight.candidates.len(),
            "candidate_scan_rows": preflight.candidate_scan_rows,
            "candidate_scan_complete": preflight.candidate_scan_complete,
            "selected_candidate_cx_ids": &preflight.selected_candidate_cx_ids,
        }),
    )?;
    if args.coverage_only {
        let output = json!({
            "status": "coverage_only",
            "vault": resolved.name,
            "vault_dir": resolved.path.display().to_string(),
            "progress_artifact": progress.path().display().to_string(),
            "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
            "skipped_incompatible_content_slots": incompatible_content_slots,
            "candidate_selection_mode": args.candidate_selection.as_str(),
            "candidate_order": "base_page_index_key_order",
            "selection": "complete_panel_intersection",
            "embedding_slots": content_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
            "dense_slot_coverage": &preflight.coverage,
            "constellations_in_vault": preflight.constellations_in_vault,
            "candidate_scan_rows": preflight.candidate_scan_rows,
            "candidate_scan_complete": preflight.candidate_scan_complete,
            "base_page_index_live_entries": preflight.base_page_index_live_entries,
            "selected_candidate_rows": preflight.selected_candidate_rows,
            "selected_candidate_cx_ids": &preflight.selected_candidate_cx_ids,
        });
        progress.write("coverage_only", "complete", json!({ "output": &output }))?;
        return print_json(&output);
    }
    rss.sample(
        &progress,
        "candidate_selection_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;
    // Fail-closed pre-flight: the read passes decode a bounded working set (one
    // batch of node panels, plus a per-node candidate set) out of the scratch
    // store; the corpus payload itself stays in reclaimable page cache. Refuse
    // before opening the vault only if that bounded working set would already
    // blow the budget. The live-RSS sample guard enforces the real bound.
    let typed_payload_bytes = projected_read_working_set_bytes(&preflight, args.batch);
    enforce_rss_budget(&progress, "vault_open", rss_budget, typed_payload_bytes)?;
    progress.write(
        "running",
        "vault_open_start",
        json!({
            "embedding_slots": content_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
            "selection_reason": "complete_panel_intersection",
            "candidate_selection_mode": args.candidate_selection.as_str(),
            "candidate_rows": preflight.candidates.len(),
            "candidate_scan_rows": preflight.candidate_scan_rows,
            "candidate_scan_complete": preflight.candidate_scan_complete,
            "typed_payload_bytes": typed_payload_bytes,
        }),
    )?;
    // Issue #1862 shared substrate fix: open write-capable but WITHOUT
    // materializing every MVCC row of every CF into the heap. Latest reads
    // (and read-after-write) resolve through the mmap-backed CF router, so the
    // weave working set is proportional to what it touches instead of the whole
    // physical vault (the 27.2 GB vault-open retention in the issue).
    let vault = match AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    ) {
        Ok(vault) => vault,
        Err(error) => {
            let error: CliError = error.into();
            let _ = progress.write(
                "incomplete",
                "vault_open_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    rss.sample(
        &progress,
        "vault_open_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;
    progress.write(
        "running",
        "within_doc_start",
        json!({
            "embedding_slots": content_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
            "selection_reason": "complete_panel_intersection",
            "candidate_selection_mode": args.candidate_selection.as_str(),
            "candidate_rows": preflight.candidates.len(),
            "candidate_scan_rows": preflight.candidate_scan_rows,
        }),
    )?;

    let graph_generation = format!("weave-loom-{}", ulid::Ulid::new());
    let graph_storage_collection =
        graph_generation_storage_collection(PANEL_ASTER_ASSOC_COLLECTION, &graph_generation)?;
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            PANEL_ASTER_ASSOC_COLLECTION,
            graph_generation.clone(),
            GraphCollectionGenerationStatus::Writing,
            "weave-loom",
        )
        .with_reason("isolated graph and XTerm generation build started")
        .with_detail(
            GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
            GRAPH_GENERATION_STORAGE_SCHEMA,
        )
        .with_detail(
            GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
            graph_storage_collection.clone(),
        )
        .with_detail(
            WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
            WEAVE_XTERM_GENERATION_SCHEMA,
        ),
    )?;
    vault.flush()?;
    let mut generation_failure_guard = WeaveGenerationFailureGuard {
        lifecycle: &lifecycle,
        vault: &vault,
        generation: graph_generation.clone(),
        storage_collection: graph_storage_collection.clone(),
        armed: true,
    };
    let graph =
        PlainGraph::new_generation(&vault, PANEL_ASTER_ASSOC_COLLECTION, &graph_generation)?;
    // `weave_within_doc` reads each node's typed panel on demand from the
    // out-of-core scratch store keyed by its scan ordinal (#1991), so the
    // resident working set is one `batch` of node panels rather than the whole
    // corpus. Nothing corpus-scale is held or copied.
    let constellations_in_vault = preflight.constellations_in_vault;
    // Sub-phase cadence (#1991 round-4): every few thousand nodes, emit a
    // CALYX_WEAVE_RUNTIME within_doc_progress line and sample live RSS against
    // the budget mid-pass, so a residual write-side overrun fails closed with a
    // typed error at a known node offset instead of dying silent between phases.
    let mut within_progress = |update: passes::WithinDocProgress| -> CliResult {
        deadline.check("weave-loom", "within_doc", update.nodes_processed as u64)?;
        rss.sample(
            &progress,
            "within_doc_progress",
            rss_budget,
            scratch_bytes,
            Some(update.nodes_processed as u64),
        )
    };
    let within = match passes::weave_within_doc(
        &vault,
        &graph,
        &preflight.candidates,
        &preflight.candidate_ordinals,
        &preflight.scratch,
        constellations_in_vault,
        &content_slots,
        &graph_generation,
        args.batch,
        Some(&mut within_progress),
    ) {
        Ok(within) => within,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "within_doc_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    if let Err(error) = deadline.check(
        "weave-loom",
        "within_doc_complete",
        within.constellations_processed as u64,
    ) {
        let _ = progress.write(
            "incomplete",
            "within_doc_timeout",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    progress.write(
        "running",
        "within_doc_complete",
        json!({
            "constellations_processed": within.constellations_processed,
            "xterm_rows_persisted": within.xterm_rows_persisted,
            "panel_constellations": within.constellations_processed,
        }),
    )?;
    rss.sample(
        &progress,
        "within_doc_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;
    // Second fail-closed gate before the heaviest read pass (k-NN over the
    // persisted index): if we are already over budget, refuse now.
    enforce_rss_budget(&progress, "between_doc_graph", rss_budget, 0)?;

    let indexes = super::PersistedSearchIndexes::open(&resolved.path)?;
    let mut last_rss_stage: Option<&'static str> = None;
    let mut last_rss_units = 0usize;
    let mut graph_progress = |update: between_doc::BetweenDocProgress| -> CliResult {
        let progress_units = if update.stage == "candidate_generation" {
            update.candidate_queries
        } else {
            update.nodes_processed
        };
        let stage_changed = last_rss_stage != Some(update.stage);
        if stage_changed {
            last_rss_stage = Some(update.stage);
            last_rss_units = 0;
        }
        if let Err(error) = deadline.check("weave-loom", "between_doc_graph", progress_units as u64)
        {
            let _ = progress.write(
                "incomplete",
                "between_doc_timeout",
                json!({
                    "error": error_details(&error),
                    "graph_progress": update,
                }),
            );
            return Err(error);
        }
        // Sub-phase live-RSS sample + budget guard at the same cadence as
        // within-doc, throttled below the 128-node progress cadence (#1991 r4).
        if stage_changed
            || progress_units.saturating_sub(last_rss_units) >= passes::WEAVE_PROGRESS_EVERY_NODES
            || update.nodes_processed == update.nodes_total
        {
            last_rss_units = progress_units;
            let rss_phase = match update.stage {
                "candidate_generation" => "between_doc_candidate_generation",
                "exact_rerank" => "between_doc_exact_rerank",
                _ => "between_doc_progress",
            };
            rss.sample(
                &progress,
                rss_phase,
                rss_budget,
                scratch_bytes,
                Some(progress_units as u64),
            )?;
        }
        progress.write(
            "running",
            "between_doc_graph",
            json!({ "graph_progress": update }),
        )
    };
    let graph_request = between_doc::BetweenDocGraphRequest {
        indexes: &indexes,
        embedding_slots: &content_slots,
        knn: args.knn,
        edge_score_threshold: args.edge_score_threshold,
        candidates: &preflight.candidates,
        candidate_ordinals: &preflight.candidate_ordinals,
        ordinal_map: &preflight.ordinal_map,
        scratch: &preflight.scratch,
        candidate_device: args.candidate_device,
        candidate_domain_complete: preflight.candidate_scan_complete,
    };
    let between_doc = match between_doc::build_between_doc_graph(
        &vault,
        &graph,
        graph_request,
        Some(&mut graph_progress),
    ) {
        Ok(result) => result,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "between_doc_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    rss.sample(
        &progress,
        "between_doc_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;
    let edges_persisted = between_doc.edges_persisted;
    let assoc_graph = between_doc.graph;
    let between_doc_telemetry = between_doc.telemetry;
    // Bulk generation rows were checkpointed directly into authenticated SSTs
    // and remained unpublished. Load each derived CF once for physical graph
    // verification; per-batch reload would rescan the existing level O(batches).
    if let Err(error) = vault.refresh_reconstructible_derived_cfs([
        calyx_aster::cf::ColumnFamily::Graph,
        calyx_aster::cf::ColumnFamily::XTerm,
    ]) {
        let error = CliError::from(error);
        let _ = progress.write(
            "incomplete",
            "derived_router_refresh_error",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    // Generation checkpoint batches and historical failed generations are
    // reconstructible, but their SST fanout is still a serving concern. Drive
    // the same bounded, reclaiming LSM lifecycle used by ingest before any
    // physical acceptance read. Compaction-adoption outputs carry v3 key
    // bounds and are not reclassified as fresh flush debt merely because an
    // exact rolling size lands a few bytes below its target.
    let derived_compaction = IngestCompactionConfig {
        trigger_files: DERIVED_COMPACTION_TRIGGER_FILES,
        max_round_files: DERIVED_COMPACTION_ROUND_MAX_FILES,
        max_round_bytes: DERIVED_COMPACTION_ROUND_MAX_BYTES,
    };
    let mut derived_compaction_outcomes = Vec::new();
    for cf in [
        calyx_aster::cf::ColumnFamily::Graph,
        calyx_aster::cf::ColumnFamily::XTerm,
    ] {
        let outcome = match vault.compact_cf_flush_debt(cf, derived_compaction) {
            Ok(outcome) => outcome,
            Err(error) => {
                let error = CliError::from(error);
                let _ = progress.write(
                    "incomplete",
                    "derived_compaction_error",
                    json!({
                        "cf": cf.name(),
                        "error": error_details(&error),
                    }),
                );
                return Err(error);
            }
        };
        let details = json!({
            "cf": cf.name(),
            "files_before": outcome.files_before,
            "files_after": outcome.files_after,
            "flush_debt_before": outcome.debt_before,
            "flush_debt_after": outcome.debt_after,
            "rounds": outcome.rounds,
            "reclaimed_input_files": outcome.reclaimed_input_files,
            "elapsed_ms": outcome.elapsed_ms,
            "trigger_files": DERIVED_COMPACTION_TRIGGER_FILES,
            "round_max_files": DERIVED_COMPACTION_ROUND_MAX_FILES,
            "round_max_bytes": DERIVED_COMPACTION_ROUND_MAX_BYTES,
        });
        progress.write("running", "derived_compaction_complete", details.clone())?;
        derived_compaction_outcomes.push(details);
    }
    let xterm_physical_readback = match readback_weave_xterm_generation(
        &vault,
        PANEL_ASTER_ASSOC_COLLECTION,
        &graph_generation,
        within.xterm_rows_persisted,
        &deadline,
    ) {
        Ok(readback) => readback,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "xterm_physical_readback_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    let graph_source_seq = vault.latest_seq();
    let graph_contract = graph_contract::GraphContract::from_args(
        content_slots.clone(),
        u64::from(state.panel.version),
        graph_source_seq,
        &args,
    );
    if let Err(error) = graph_contract.persist(&vault, &graph_storage_collection) {
        let error = CliError::from(error);
        let _ = progress.write(
            "incomplete",
            "graph_contract_persist_error",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    csr::persist_assoc_csr(&vault, &graph, &resolved.path, &graph_generation, &progress)?;
    rss.sample(
        &progress,
        "graph_persist_complete",
        rss_budget,
        scratch_bytes,
        None,
    )?;
    // Weave succeeded: remove the out-of-core vector scratch store. On any
    // failure path above the `?` short-circuits before this, leaving the scratch
    // tree on disk for diagnosis (#1991).
    if let Err(error) = preflight.scratch.cleanup() {
        let _ = progress.write(
            "running",
            "scratch_cleanup_failed",
            json!({ "error": error_details(&error) }),
        );
    }

    let report_params = CorpusWeaveReportParams {
        max_groundedness_distance: args.max_groundedness_distance,
        ..CorpusWeaveReportParams::default()
    };
    let report = match corpus_weave_report(&assoc_graph, &within.anchors, &report_params) {
        Ok(report) => report,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "incomplete",
                "weave_acceptance_report_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };

    let accepted_state = GraphCollectionGenerationState::new(
        PANEL_ASTER_ASSOC_COLLECTION,
        graph_generation.clone(),
        GraphCollectionGenerationStatus::Accepted,
        "weave-loom",
    )
    .with_reason("physical graph/CSR readback and weave acceptance report passed")
    .with_detail(
        GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
        GRAPH_GENERATION_STORAGE_SCHEMA,
    )
    .with_detail(
        GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
        graph_storage_collection.clone(),
    )
    .with_detail(
        WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
        WEAVE_XTERM_GENERATION_SCHEMA,
    )
    .with_detail("node_rows", within.constellations_processed.to_string())
    .with_detail("edge_rows", edges_persisted.to_string())
    .with_detail("xterm_rows", within.xterm_rows_persisted.to_string())
    .with_detail("xterm_sha256", xterm_physical_readback.sha256.clone());
    if let Err(error) = lifecycle.put_state(&accepted_state) {
        let error = CliError::from(error);
        let _ = progress.write(
            "incomplete",
            "generation_accept_commit_error",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    if let Err(error) = vault.flush() {
        let error = CliError::from(error);
        let _ = progress.write(
            "incomplete",
            "generation_accept_flush_error",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    let physical = match PhysicalPlainGraph::open_latest_isolated(
        &resolved.path,
        PANEL_ASTER_ASSOC_COLLECTION,
    ) {
        Ok(physical) => physical,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "incomplete",
                "generation_accept_graph_readback_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    if physical.storage_collection() != graph_storage_collection {
        let error = CliError::runtime(format!(
            "accepted graph generation routed to {}, expected {}",
            physical.storage_collection(),
            graph_storage_collection
        ));
        let _ = progress.write(
            "incomplete",
            "generation_accept_graph_route_mismatch",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    let lifecycle_rows = match PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)
        .and_then(|readback| readback.list_states())
    {
        Ok(rows) => rows,
        Err(error) => {
            let error = CliError::from(error);
            let _ = progress.write(
                "incomplete",
                "generation_accept_lifecycle_readback_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    let accepted_generation_read_back = lifecycle_rows.into_iter().any(|row| {
        row.state.collection == PANEL_ASTER_ASSOC_COLLECTION
            && row.state.generation == graph_generation
            && row.state.status == GraphCollectionGenerationStatus::Accepted
            && row
                .state
                .detail
                .get(GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL)
                == Some(&graph_storage_collection)
            && row
                .state
                .detail
                .get(WEAVE_XTERM_STORAGE_SCHEMA_DETAIL)
                .map(String::as_str)
                == Some(WEAVE_XTERM_GENERATION_SCHEMA)
            && row.state.detail.get("xterm_sha256") == Some(&xterm_physical_readback.sha256)
    });
    if !accepted_generation_read_back {
        let error = CliError::runtime(format!(
            "accepted graph lifecycle state missing after physical readback for {}/{}",
            PANEL_ASTER_ASSOC_COLLECTION, graph_generation
        ));
        let _ = progress.write(
            "incomplete",
            "generation_accept_lifecycle_mismatch",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    generation_failure_guard.disarm();

    let total_in_vault = within.constellations_in_vault;
    let output = json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "progress_artifact": progress.path().display().to_string(),
        "graph_generation": graph_generation,
        "graph_storage_collection": graph_storage_collection,
        "graph_generation_accepted_readback": accepted_generation_read_back,
        "bulk_durability": "checkpoint_sst_then_lifecycle_acceptance",
        "bulk_wal_enabled": false,
        "derived_compaction": derived_compaction_outcomes,
        "xterm_storage_schema": WEAVE_XTERM_GENERATION_SCHEMA,
        "xterm_physical_readback": xterm_physical_readback,
        "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
        "skipped_incompatible_content_slots": incompatible_content_slots,
        "embedding_slots": content_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
        "fusion": "typed_block_rrf",
        "rrf_k": calyx_lodestar::PANEL_RRF_K,
        "candidate_selection_mode": args.candidate_selection.as_str(),
        "candidate_selection": "complete_panel_intersection",
        "candidate_device": args.candidate_device.as_str(),
        "dense_slot_coverage": &preflight.coverage,
        "candidate_order": "base_page_index_key_order",
        "candidate_scan_rows": preflight.candidate_scan_rows,
        "candidate_scan_complete": preflight.candidate_scan_complete,
        "base_page_index_live_entries": preflight.base_page_index_live_entries,
        "selected_candidate_rows": preflight.selected_candidate_rows,
        "selected_candidate_cx_ids": &preflight.selected_candidate_cx_ids,
        "knn": args.knn,
        "edge_score_threshold": args.edge_score_threshold,
        "graph_contract": graph_contract,
        "constellations_in_vault": total_in_vault,
        "constellations_processed": within.constellations_processed,
        "limited": within.constellations_processed < total_in_vault,
        "xterm": {
            "rows_persisted": within.xterm_rows_persisted,
            "slot_pair_count": within.agreement_pairs.len(),
            "slot_pairs": within.agreement_pairs,
        },
        "assoc_graph": {
            "edges_persisted": edges_persisted,
            "between_doc_telemetry": between_doc_telemetry,
            "report": report,
        },
        "rss": rss.report(),
    });
    progress.write("ok", "complete", json!({ "output": &output }))?;
    fsv::write(&output)?;
    print_json(&output)
}

#[derive(Clone, Debug, Serialize)]
struct WeaveXTermPhysicalReadback {
    rows: usize,
    key_bytes: u64,
    value_bytes: u64,
    sha256: String,
}

fn readback_weave_xterm_generation<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    generation: &str,
    expected_rows: usize,
    deadline: &Deadline,
) -> CliResult<WeaveXTermPhysicalReadback> {
    let range = weave_xterm_generation_prefix_range(collection, generation)?;
    let snapshot = vault.latest_seq();
    let mut rows = 0usize;
    let mut key_bytes = 0u64;
    let mut value_bytes = 0u64;
    let mut hasher = Sha256::new();
    vault.scan_cf_range_pages_at_renewing_latest(
        snapshot,
        calyx_aster::cf::ColumnFamily::XTerm,
        &range,
        XTERM_READBACK_PAGE_ROWS,
        |page| -> CliResult {
            deadline.check("weave-loom", "xterm.physical_readback", rows as u64)?;
            for (key, value) in &page {
                hasher.update((key.len() as u64).to_be_bytes());
                hasher.update(key);
                hasher.update((value.len() as u64).to_be_bytes());
                hasher.update(value);
                key_bytes = key_bytes.saturating_add(key.len() as u64);
                value_bytes = value_bytes.saturating_add(value.len() as u64);
                rows += 1;
            }
            if rows > expected_rows {
                return Err(CliError::runtime(format!(
                    "physical XTerm generation {collection}/{generation} has more than expected {expected_rows} rows"
                )));
            }
            Ok(())
        },
    )?;
    if rows != expected_rows {
        return Err(CliError::runtime(format!(
            "physical XTerm generation {collection}/{generation} has {rows} rows, expected {expected_rows}"
        )));
    }
    Ok(WeaveXTermPhysicalReadback {
        rows,
        key_bytes,
        value_bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn content_lens_slots(panel: &calyx_core::Panel) -> Vec<SlotId> {
    let mut slots: Vec<SlotId> = panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active && !slot.retrieval_only)
        .map(|slot| slot.slot_id)
        .collect();
    slots.sort();
    slots.dedup();
    slots
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct IncompatibleContentSlot {
    slot_id: u16,
    shape: String,
    reason: &'static str,
}

fn incompatible_content_lens_slots(panel: &calyx_core::Panel) -> Vec<IncompatibleContentSlot> {
    let mut slots: Vec<IncompatibleContentSlot> = panel
        .slots
        .iter()
        .filter(|slot| {
            slot.state == SlotState::Active
                && !slot.retrieval_only
                && !matches!(
                    slot.shape,
                    SlotShape::Dense(_) | SlotShape::Sparse(_) | SlotShape::Multi { .. }
                )
        })
        .map(|slot| IncompatibleContentSlot {
            slot_id: slot.slot_id.get(),
            shape: slot_shape_label(slot.shape),
            reason: "active_content_slot_shape_is_not_dense",
        })
        .collect();
    slots.sort_by_key(|slot| slot.slot_id);
    slots.dedup();
    slots
}

fn slot_shape_label(shape: SlotShape) -> String {
    match shape {
        SlotShape::Dense(dim) => format!("dense:{dim}"),
        SlotShape::Sparse(dim) => format!("sparse:{dim}"),
        SlotShape::Multi { token_dim } => format!("multi:{token_dim}"),
    }
}

#[cfg(test)]
mod tests;
