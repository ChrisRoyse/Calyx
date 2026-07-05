//! Multi-lens measurement fans out across panel slots (#1153).
//!
//! Sequential embedder execution is a production defect (#1152): warm
//! co-resident sessions exist precisely so every runnable slot measures
//! concurrently. Each chunk spawns one scoped thread per runnable slot,
//! joins in slot order, and records per-slot monotonic spans. When two or
//! more slots each ran past the overlap floor, zero pairwise span overlap
//! means something serialized them — a shared lock or a regression to a
//! serial loop — and fails loud as `CALYX_EMBED_SEQUENTIAL_EXECUTION`
//! (#1154) unless explicitly downgraded.

use calyx_core::Result;
use calyx_registry::measure_registry_batch_with_runtime_limit;

use super::server::ResidentService;
use super::*;

pub(super) const REQUIRE_PARALLEL_ENV: &str = "CALYX_EMBED_REQUIRE_PARALLEL";
pub(super) const OVERLAP_FLOOR_ENV: &str = "CALYX_EMBED_OVERLAP_FLOOR_MS";
const DEFAULT_OVERLAP_FLOOR_MS: u128 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequireParallelPolicy {
    Off,
    Warn,
    Error,
}

/// Zero-overlap response policy. Defaults to `Error`: the resident is the
/// production embedding path, so a serialized panel must stop the stream,
/// not degrade it 10x silently.
pub(super) fn require_parallel_policy() -> Result<RequireParallelPolicy> {
    let Ok(raw) = std::env::var(REQUIRE_PARALLEL_ENV) else {
        return Ok(RequireParallelPolicy::Error);
    };
    match raw.trim() {
        "" | "1" | "true" | "error" => Ok(RequireParallelPolicy::Error),
        "warn" => Ok(RequireParallelPolicy::Warn),
        "0" | "false" | "off" => Ok(RequireParallelPolicy::Off),
        other => Err(CalyxError {
            code: "CALYX_EMBED_REQUIRE_PARALLEL_INVALID",
            message: format!("{REQUIRE_PARALLEL_ENV}={other} is not a known policy"),
            remediation: "set CALYX_EMBED_REQUIRE_PARALLEL to error (default), warn, or off",
        }),
    }
}

/// Spans shorter than this never participate in the zero-overlap check:
/// under it, thread scheduling noise — not serialization — dominates.
pub(super) fn overlap_floor_ms() -> Result<u128> {
    let Ok(raw) = std::env::var(OVERLAP_FLOOR_ENV) else {
        return Ok(DEFAULT_OVERLAP_FLOOR_MS);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(DEFAULT_OVERLAP_FLOOR_MS);
    }
    raw.parse::<u128>().map_err(|_| CalyxError {
        code: "CALYX_EMBED_OVERLAP_FLOOR_INVALID",
        message: format!("{OVERLAP_FLOOR_ENV}={raw} is not a non-negative integer millisecond count"),
        remediation: "set CALYX_EMBED_OVERLAP_FLOOR_MS to a non-negative integer (default 25), or unset it",
    })
}

/// One slot's measurement outcome with its monotonic span relative to the
/// chunk epoch.
pub(super) struct SlotOutcome<T> {
    pub(super) started_us: u128,
    pub(super) ended_us: u128,
    pub(super) result: std::thread::Result<T>,
}

/// Run `run(0..count)` concurrently on scoped threads, returning outcomes in
/// input order. A single item runs inline — there is nothing to overlap.
pub(super) fn fan_out<T: Send>(
    count: usize,
    run: impl Fn(usize) -> T + Sync,
) -> Vec<SlotOutcome<T>> {
    let epoch = Instant::now();
    if count <= 1 {
        return (0..count)
            .map(|index| {
                let started_us = epoch.elapsed().as_micros();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(index)));
                SlotOutcome {
                    started_us,
                    ended_us: epoch.elapsed().as_micros(),
                    result,
                }
            })
            .collect();
    }
    std::thread::scope(|scope| {
        let run = &run;
        let handles: Vec<_> = (0..count)
            .map(|index| {
                scope.spawn(move || {
                    let started_us = epoch.elapsed().as_micros();
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(index)));
                    SlotOutcome {
                        started_us,
                        ended_us: epoch.elapsed().as_micros(),
                        result,
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| match handle.join() {
                Ok(outcome) => outcome,
                // catch_unwind above already captured the panic payload; a
                // join error means the thread died outside it, which still
                // must surface as this slot's failure, not a process abort.
                Err(payload) => SlotOutcome {
                    started_us: 0,
                    ended_us: 0,
                    result: Err(payload),
                },
            })
            .collect()
    })
}

/// Measure one chunk through every runnable slot concurrently. Results keep
/// panel slot order; the first failing slot (in slot order) fails the chunk
/// after every failure has been logged with its lens attribution.
pub(super) fn measure_chunk_lenses(
    service: &ResidentService,
    modality: Modality,
    chunk: &[Input],
    runtime_batch_limit: Option<usize>,
) -> CliResult<BTreeMap<LensId, Vec<SlotVector>>> {
    let policy = require_parallel_policy()?;
    let floor_us = overlap_floor_ms()?.saturating_mul(1000);
    let mut runnable = Vec::new();
    for slot in &service.state.build.panel.slots {
        if slot.state != SlotState::Active
            || slot.modality != modality
            || !service.state.build.registry.contains(slot.lens_id)
            || runnable
                .iter()
                .any(|seen: &&calyx_core::Slot| seen.lens_id == slot.lens_id)
        {
            continue;
        }
        runnable.push(slot);
    }
    let registry = &service.state.build.registry;
    let outcomes = fan_out(runnable.len(), |index| {
        measure_registry_batch_with_runtime_limit(
            registry,
            runnable[index].lens_id,
            chunk,
            runtime_batch_limit,
        )
    });

    let mut measured_by_lens = BTreeMap::new();
    let mut spans = Vec::new();
    let mut first_error: Option<CalyxError> = None;
    for (slot, outcome) in runnable.iter().zip(outcomes) {
        let error = match outcome.result {
            Ok(Ok(vectors)) if vectors.len() == chunk.len() => {
                eprintln!(
                    "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_lens_ok process_id={} lens_id={} slot={} inputs={} elapsed_ms={} span_start_us={} span_end_us={}",
                    std::process::id(),
                    slot.lens_id,
                    slot.slot_id.get(),
                    chunk.len(),
                    (outcome.ended_us - outcome.started_us) / 1000,
                    outcome.started_us,
                    outcome.ended_us
                );
                spans.push((outcome.started_us, outcome.ended_us));
                measured_by_lens.insert(slot.lens_id, vectors);
                continue;
            }
            Ok(Ok(vectors)) => CalyxError::lens_dim_mismatch(format!(
                "resident measure_batch lens {} returned {} vectors for {} inputs",
                slot.lens_id,
                vectors.len(),
                chunk.len()
            )),
            Ok(Err(error)) => error,
            Err(_) => CalyxError::lens_unreachable(format!(
                "resident measure_batch thread for lens {} panicked",
                slot.lens_id
            )),
        };
        eprintln!(
            "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_lens_err process_id={} lens_id={} slot={} code={} message={}",
            std::process::id(),
            slot.lens_id,
            slot.slot_id.get(),
            error.code,
            error.message
        );
        if first_error.is_none() {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error.into());
    }
    enforce_overlap(&spans, policy, floor_us)?;
    Ok(measured_by_lens)
}

/// #1154: log chunk-level overlap stats and fail loud on serialized spans.
pub(super) fn enforce_overlap(
    spans: &[(u128, u128)],
    policy: RequireParallelPolicy,
    floor_us: u128,
) -> Result<()> {
    let wall_us = spans
        .iter()
        .map(|(_, end)| *end)
        .max()
        .unwrap_or(0)
        .saturating_sub(spans.iter().map(|(start, _)| *start).min().unwrap_or(0));
    let busy_us: u128 = spans.iter().map(|(start, end)| end - start).sum();
    let significant: Vec<&(u128, u128)> = spans
        .iter()
        .filter(|(start, end)| end - start >= floor_us)
        .collect();
    let mut overlapping_pairs = 0usize;
    for (index, a) in significant.iter().enumerate() {
        for b in &significant[index + 1..] {
            if a.0 < b.1 && b.0 < a.1 {
                overlapping_pairs += 1;
            }
        }
    }
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_overlap process_id={} slots={} significant={} overlapping_pairs={} wall_us={wall_us} busy_us={busy_us}",
        std::process::id(),
        spans.len(),
        significant.len(),
        overlapping_pairs
    );
    if significant.len() < 2 || overlapping_pairs > 0 || policy == RequireParallelPolicy::Off {
        return Ok(());
    }
    let message = format!(
        "{} slots each ran >= {}us with zero pairwise span overlap: multi-lens measurement executed sequentially",
        significant.len(),
        floor_us
    );
    match policy {
        RequireParallelPolicy::Warn => {
            eprintln!("CALYX_EMBED_SEQUENTIAL_EXECUTION (warn) {message}");
            Ok(())
        }
        _ => Err(CalyxError {
            code: "CALYX_EMBED_SEQUENTIAL_EXECUTION",
            message,
            remediation: "embedders must run concurrently: find what serialized the slots (shared lock, serial loop regression); set CALYX_EMBED_REQUIRE_PARALLEL=warn only to diagnose",
        }),
    }
}
