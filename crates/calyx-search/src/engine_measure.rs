use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use calyx_core::{CalyxError, LensId, MeasurementGroupKey, SlotId, SlotVector};

use crate::engine_trace::SearchTracer;
use crate::error::CliResult;

/// Measure the query through every active text lens that is materialized in the
/// registry, keeping only indexable vectors.
pub fn measure_query_vectors(
    state: &calyx_registry::VaultPanelState,
    query: &str,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    measure_query_vectors_with_slots(state, query, None)
}

/// Measure query vectors for active text slots, optionally restricted to a
/// caller-selected physical slot set.
pub fn measure_query_vectors_with_slots(
    state: &calyx_registry::VaultPanelState,
    query: &str,
    allowed_slots: Option<&BTreeSet<SlotId>>,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    measure_query_vectors_with_slots_traced(state, query, allowed_slots, None)
}

pub(crate) fn measure_query_vectors_with_slots_traced(
    state: &calyx_registry::VaultPanelState,
    query: &str,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    trace: Option<&mut SearchTracer<'_>>,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    use calyx_core::{Input, Modality, SlotState};
    let mut noop_trace;
    let trace = match trace {
        Some(trace) => trace,
        None => {
            noop_trace = SearchTracer::new(None);
            &mut noop_trace
        }
    };
    trace.emit_detail(
        "query.measure.start",
        None,
        Some(state.panel.slots.len()),
        Some(format!("bytes={}", query.len())),
    );
    let input = Input::new(Modality::Text, query.as_bytes().to_vec());
    let mut runnable = Vec::new();
    for slot in &state.panel.slots {
        if allowed_slots.is_some_and(|allowed| !allowed.contains(&slot.slot_id)) {
            continue;
        }
        if slot.state == SlotState::Active
            && slot.modality == Modality::Text
            && state.registry.contains(slot.lens_id)
        {
            trace.emit_detail(
                "query.measure_slot.start",
                Some(slot.slot_id),
                None,
                Some(slot.lens_id.to_string()),
            );
            runnable.push((slot.slot_id, slot.lens_id));
        }
    }

    let jobs = measurement_jobs(&state.registry, &runnable)?;
    trace.emit_detail(
        "query.measure_parallel.start",
        None,
        Some(jobs.len()),
        Some(format!("slots={}", runnable.len())),
    );
    let epoch = Instant::now();
    let outcomes = std::thread::scope(|scope| {
        jobs.iter()
            .map(|job| {
                scope.spawn(|| {
                    let started_ms = epoch.elapsed().as_millis();
                    let measured = measure_job(&state.registry, job, &input);
                    (started_ms, epoch.elapsed().as_millis(), measured)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Vec<_>>()
    });

    let mut measured_by_lens = BTreeMap::new();
    let mut first_error = None;
    for (job, outcome) in jobs.iter().zip(outcomes) {
        let (started_ms, ended_ms, result) = match outcome {
            Ok(outcome) => outcome,
            Err(_) => {
                let error = CalyxError::lens_unreachable(format!(
                    "parallel query measurement thread panicked for lenses [{}]",
                    job.lens_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
                for slot in &job.slots {
                    trace.emit_detail(
                        "query.measure_slot.error",
                        Some(*slot),
                        None,
                        Some(format!("{} {}", error.code, error.message)),
                    );
                }
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
        };
        match result {
            Ok(measured) => {
                trace.emit_detail(
                    "query.measure_parallel.job_done",
                    None,
                    Some(job.lens_ids.len()),
                    Some(format!(
                        "grouped={} started_ms={started_ms} ended_ms={ended_ms}",
                        job.grouped
                    )),
                );
                measured_by_lens.extend(measured);
            }
            Err(error) => {
                for slot in &job.slots {
                    trace.emit_detail(
                        "query.measure_slot.error",
                        Some(*slot),
                        None,
                        Some(format!("{} {}", error.code, error.message)),
                    );
                }
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error.into());
    }

    let mut out = Vec::new();
    for (slot, lens) in runnable {
        let vector = measured_by_lens.get(&lens).ok_or_else(|| {
            CalyxError::lens_dim_mismatch(format!(
                "parallel query measurement omitted lens {lens} for slot {slot}"
            ))
        })?;
        let is_indexable = vector.is_indexable();
        trace.emit_detail(
            "query.measure_slot.done",
            Some(slot),
            Some(is_indexable as usize),
            Some(slot_vector_shape(vector)),
        );
        if is_indexable {
            out.push((slot, vector.clone()));
        }
    }
    trace.emit_detail(
        "query.measure_parallel.done",
        None,
        Some(jobs.len()),
        Some(format!("elapsed_ms={}", epoch.elapsed().as_millis())),
    );
    trace.emit("query.measure.done", None, Some(out.len()));
    Ok(out)
}

struct MeasurementJob {
    lens_ids: Vec<LensId>,
    slots: Vec<SlotId>,
    grouped: bool,
}

fn measurement_jobs(
    registry: &calyx_registry::Registry,
    runnable: &[(SlotId, LensId)],
) -> Result<Vec<MeasurementJob>, CalyxError> {
    let mut jobs: Vec<MeasurementJob> = Vec::new();
    let mut grouped_jobs: BTreeMap<MeasurementGroupKey, usize> = BTreeMap::new();
    let mut lens_jobs: BTreeMap<LensId, usize> = BTreeMap::new();
    for &(slot, lens) in runnable {
        if let Some(&job_index) = lens_jobs.get(&lens) {
            jobs[job_index].slots.push(slot);
            continue;
        }
        let job_index = match registry.measurement_group_key(lens)? {
            Some(group) => {
                if let Some(&job_index) = grouped_jobs.get(&group) {
                    jobs[job_index].lens_ids.push(lens);
                    jobs[job_index].slots.push(slot);
                    job_index
                } else {
                    let job_index = jobs.len();
                    grouped_jobs.insert(group, job_index);
                    jobs.push(MeasurementJob {
                        lens_ids: vec![lens],
                        slots: vec![slot],
                        grouped: true,
                    });
                    job_index
                }
            }
            None => {
                let job_index = jobs.len();
                jobs.push(MeasurementJob {
                    lens_ids: vec![lens],
                    slots: vec![slot],
                    grouped: false,
                });
                job_index
            }
        };
        lens_jobs.insert(lens, job_index);
    }
    Ok(jobs)
}

fn measure_job(
    registry: &calyx_registry::Registry,
    job: &MeasurementJob,
    input: &calyx_core::Input,
) -> Result<BTreeMap<LensId, SlotVector>, CalyxError> {
    if job.grouped {
        let measured =
            registry.measure_grouped_batch(&job.lens_ids, std::slice::from_ref(input))?;
        return measured
            .into_iter()
            .map(|(lens, mut vectors)| {
                let vector = vectors.pop().ok_or_else(|| {
                    CalyxError::lens_dim_mismatch(format!(
                        "grouped query measurement returned no vector for lens {lens}"
                    ))
                })?;
                if !vectors.is_empty() {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "grouped query measurement returned multiple vectors for one input and lens {lens}"
                    )));
                }
                Ok((lens, vector))
            })
            .collect();
    }
    let lens = job.lens_ids[0];
    registry
        .measure(lens, input)
        .map(|vector| BTreeMap::from([(lens, vector)]))
}

pub(crate) fn no_indexable_query_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable query vectors from active text lenses; re-enable a concrete lens or remeasure the panel",
    )
}

pub(crate) fn no_indexable_stored_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable stored slot vectors matching active query lenses; reingest or backfill stale slot rows",
    )
}

pub(crate) fn slot_vector_shape(vector: &SlotVector) -> String {
    match vector {
        SlotVector::Dense { dim, data } => format!("dense dim={dim} len={}", data.len()),
        SlotVector::Sparse { dim, entries } => format!("sparse dim={dim} nnz={}", entries.len()),
        SlotVector::Multi { token_dim, tokens } => {
            format!("multi token_dim={token_dim} tokens={}", tokens.len())
        }
        SlotVector::SegmentedMulti {
            token_dim,
            provenance,
            segments,
        } => format!(
            "segmented_multi token_dim={token_dim} segments={} source_bytes={} source_tokens={} policy_version={}",
            segments.len(),
            provenance.source_byte_len,
            provenance.source_token_count,
            provenance.policy_version
        ),
        SlotVector::Absent { reason } => format!("absent reason={reason:?}"),
    }
}
