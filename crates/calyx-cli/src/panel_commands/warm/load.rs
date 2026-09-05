use super::load_progress::{
    PrimeErrorEvent, append_shared_progress, emit_registration_progress_shared, prime_error_record,
    prime_progress_record, task_progress_record, warm_prime_cli_error, warm_prime_error,
};
use super::probe::{probe_bytes, runtime_detail, vector_kind_len};
use super::*;

#[derive(Clone)]
pub(super) struct WarmLensTask {
    pub(super) template_idx: usize,
    pub(super) position: usize,
    pub(super) total: usize,
    pub(super) template_id: String,
    pub(super) lens: template_store::TemplateLensRef,
}

struct WarmPreparedLens {
    task: WarmLensTask,
    prepared: PreparedRuntimeLens,
    prepare_ms: u128,
}

pub(super) fn register_and_prime_warm_lenses_parallel(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    progress_log: &SharedProgressLog,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<usize> {
    register_and_prime_selected_warm_lenses_parallel(
        registry,
        template,
        None,
        progress_log,
        selector,
        load_limit,
        load_parallelism,
    )
}

pub(super) fn register_and_prime_scoped_warm_lenses_parallel(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    selected_slots: &BTreeSet<SlotId>,
    progress_log: &SharedProgressLog,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<usize> {
    register_and_prime_selected_warm_lenses_parallel(
        registry,
        template,
        Some(selected_slots),
        progress_log,
        selector,
        load_limit,
        load_parallelism,
    )
}

#[allow(clippy::too_many_arguments)]
fn register_and_prime_selected_warm_lenses_parallel(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    selected_slots: Option<&BTreeSet<SlotId>>,
    progress_log: &SharedProgressLog,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<usize> {
    let tasks = warm_lens_tasks(template, selected_slots)?;
    let prepared =
        prepare_warm_lenses_parallel(tasks, selector, load_limit, load_parallelism, progress_log)?;
    register_prepared_warm_lenses(registry, template, prepared, selector, progress_log)
}

fn warm_lens_tasks(
    template: &template_store::SavedPanelTemplate,
    selected_slots: Option<&BTreeSet<SlotId>>,
) -> CliResult<Vec<WarmLensTask>> {
    let template_id = template_store::id_for_loaded(template)?;
    let mut tasks = template
        .lenses
        .iter()
        .cloned()
        .enumerate()
        .filter(|(template_idx, _)| {
            selected_slots.is_none_or(|slots| {
                u16::try_from(*template_idx)
                    .ok()
                    .is_some_and(|slot| slots.contains(&SlotId::new(slot)))
            })
        })
        .map(|(template_idx, lens)| WarmLensTask {
            template_idx,
            position: 0,
            total: 0,
            template_id: template_id.clone(),
            lens,
        })
        .collect::<Vec<_>>();
    let total = tasks.len();
    for (position, task) in tasks.iter_mut().enumerate() {
        task.position = position + 1;
        task.total = total;
    }
    tasks.sort_by(|left, right| {
        warm_prepare_weight(&right.lens)
            .cmp(&warm_prepare_weight(&left.lens))
            .then_with(|| left.lens.slot_key.cmp(&right.lens.slot_key))
    });
    Ok(tasks)
}

fn warm_prepare_weight(lens: &template_store::TemplateLensRef) -> (u8, u64) {
    let placement_rank = if lens.placement == Placement::Gpu {
        1
    } else {
        0
    };
    (placement_rank, lens.cost.vram_bytes)
}

fn prepare_warm_lenses_parallel(
    tasks: Vec<WarmLensTask>,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
    progress_log: &SharedProgressLog,
) -> CliResult<Vec<WarmPreparedLens>> {
    let total = tasks.len();
    if total == 0 {
        return Ok(Vec::new());
    }
    let worker_count = load_parallelism.min(total).max(1);
    let mut start = run_progress_record(selector, "parallel_prepare_start");
    start.lens_count = Some(total);
    start.load_parallelism = Some(worker_count);
    append_shared_progress(progress_log, &start)?;

    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let (tx, rx) = mpsc::channel();
    for _ in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let selector = selector.to_string();
        let progress_log = progress_log.clone();
        thread::spawn(move || {
            loop {
                let task = match queue.lock() {
                    Ok(mut guard) => guard.pop_front(),
                    Err(_) => {
                        let _ = tx.send(Err(CliError::from(CalyxError::lens_unreachable(
                            "panel warm prepare queue mutex was poisoned",
                        ))));
                        return;
                    }
                };
                let Some(task) = task else {
                    return;
                };
                if tx
                    .send(prepare_warm_lens(task, &selector, &progress_log))
                    .is_err()
                {
                    return;
                }
            }
        });
    }
    drop(tx);

    let mut prepared = Vec::with_capacity(total);
    while prepared.len() < total {
        let item = load_limit.recv(
            &rx,
            WarmLoadWait {
                selector,
                phase: "parallel_prepare_prime",
                completed: prepared.len(),
                total,
                load_parallelism: worker_count,
                progress_log,
            },
        )??;
        prepared.push(item);
    }
    prepared.sort_by_key(|item| item.task.template_idx);

    let mut ok = run_progress_record(selector, "parallel_prepare_ok");
    ok.elapsed_ms = Some(load_limit.elapsed_ms());
    ok.lens_count = Some(total);
    ok.load_parallelism = Some(worker_count);
    append_shared_progress(progress_log, &ok)?;
    Ok(prepared)
}

fn prepare_warm_lens(
    task: WarmLensTask,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult<WarmPreparedLens> {
    append_shared_progress(
        progress_log,
        &task_progress_record(selector, "prepare_start", &task),
    )?;
    let started = Instant::now();
    let result = (|| {
        let spec = task.lens.verified_materialization_spec(&task.template_id)?;
        let mut start = task_progress_record(selector, "runtime_prepare_start", &task);
        start.runtime = Some(runtime_name(&spec.runtime).to_string());
        start.runtime_detail = Some(runtime_detail(&spec.runtime));
        append_shared_progress(progress_log, &start)?;
        let prepared = prepare_manifest_runtime(spec).map_err(|error| {
            task.lens
                .materialization_error(&task.template_id, "warm_runtime_prepare", error)
        })?;
        let expected_contract = task.lens.expected_runtime_contract().ok_or_else(|| {
            template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!(
                    "template {} lens {} is missing its frozen runtime contract",
                    task.template_id, task.lens.lens_name
                ),
                "explicitly refresh the template from verified commissioned artifacts",
            )
        })?;
        if &prepared.contract != expected_contract {
            return Err(template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!(
                    "template {} lens {} warm runtime contract conflict: prepared={} expected={}",
                    task.template_id,
                    task.lens.lens_name,
                    prepared.contract.lens_id(),
                    expected_contract.lens_id()
                ),
                "recommission the lens and explicitly save a new template version; never reinterpret the existing object",
            ));
        }
        Ok(prepared)
    })();
    match result {
        Ok(prepared) => {
            let mut record = task_progress_record(selector, "runtime_prepare_ok", &task);
            record.runtime = Some(runtime_name(&prepared.spec.runtime).to_string());
            record.runtime_detail = Some(runtime_detail(&prepared.spec.runtime));
            record.elapsed_ms = Some(started.elapsed().as_millis());
            append_shared_progress(progress_log, &record)?;
            prime_prepared_warm_lens(&task, &prepared, selector, progress_log)?;
            Ok(WarmPreparedLens {
                task,
                prepared,
                prepare_ms: started.elapsed().as_millis(),
            })
        }
        Err(error) => {
            let mut record = task_progress_record(selector, "runtime_prepare_error", &task);
            record.elapsed_ms = Some(started.elapsed().as_millis());
            record.error_code = Some(error.code().to_string());
            record.error_message = Some(error.message().to_string());
            record.remediation = Some(error.remediation().to_string());
            append_shared_progress(progress_log, &record)?;
            Err(error)
        }
    }
}

fn prime_prepared_warm_lens(
    task: &WarmLensTask,
    prepared: &PreparedRuntimeLens,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult {
    let lens = &task.lens;
    let runtime_lens_id = prepared.lens.id();
    let spec = &prepared.spec;
    append_shared_progress(
        progress_log,
        &prime_progress_record(
            selector,
            "prime_start",
            task.position,
            task.total,
            lens,
            runtime_lens_id,
            &spec.runtime,
        ),
    )?;
    let input = Input::new(lens.modality, probe_bytes(lens.modality)?);
    let started = Instant::now();
    let vector = match prepared.lens.measure(&input) {
        Ok(vector) => vector,
        Err(error) => {
            append_shared_progress(
                progress_log,
                &prime_error_record(
                    selector,
                    task.position,
                    task.total,
                    lens,
                    runtime_lens_id,
                    &spec.runtime,
                    PrimeErrorEvent {
                        elapsed_ms: started.elapsed().as_millis(),
                        error_code: error.code.to_string(),
                        error_message: error.message.clone(),
                    },
                ),
            )?;
            return Err(warm_prime_error(
                lens,
                spec.name.as_str(),
                &spec.runtime,
                error,
            ));
        }
    };
    if let Err(error) = validate_vector_contract(&vector, lens.shape, spec.norm_policy) {
        append_shared_progress(
            progress_log,
            &prime_error_record(
                selector,
                task.position,
                task.total,
                lens,
                runtime_lens_id,
                &spec.runtime,
                PrimeErrorEvent {
                    elapsed_ms: started.elapsed().as_millis(),
                    error_code: error.code().to_string(),
                    error_message: error.message().to_string(),
                },
            ),
        )?;
        return Err(warm_prime_cli_error(
            lens,
            spec.name.as_str(),
            &spec.runtime,
            error,
        ));
    }
    let mut record = prime_progress_record(
        selector,
        "prime_ok",
        task.position,
        task.total,
        lens,
        runtime_lens_id,
        &spec.runtime,
    );
    record.elapsed_ms = Some(started.elapsed().as_millis());
    let (kind, len) = vector_kind_len(&vector);
    record.vector_kind = Some(kind);
    record.vector_len = Some(len);
    record.norm = Some(slot_norm(&vector));
    record.first_values = Some(slot_prefix(&vector, 4));
    append_shared_progress(progress_log, &record)?;
    Ok(())
}

fn register_prepared_warm_lenses(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    prepared: Vec<WarmPreparedLens>,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult<usize> {
    let mut staged_registry = registry.clone();
    let mut staged_template = template.clone();
    let added = register_prepared_warm_lenses_staged(
        &mut staged_registry,
        &mut staged_template,
        prepared,
        selector,
        progress_log,
    )?;
    *registry = staged_registry;
    *template = staged_template;
    Ok(added)
}

fn register_prepared_warm_lenses_staged(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    prepared: Vec<WarmPreparedLens>,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult<usize> {
    let mut added = 0;
    for item in prepared {
        let lens = &mut template.lenses[item.task.template_idx];
        let spec_lens_id = item.prepared.spec.lens_id();
        if let Some(existing) = registry.find_lens_by_spec_id(spec_lens_id) {
            if registry.lens_spec(existing) != Some(&item.prepared.spec) {
                return Err(template_store::template_error(
                    template_store::TEMPLATE_INVALID,
                    format!(
                        "registry lens {existing} does not match manifest {}",
                        lens.manifest
                    ),
                    "recommission the lens so the registry snapshot and manifest are identical",
                ));
            }
            if let Some(expected) = lens.runtime_lens_id
                && existing != expected
            {
                return Err(template_store::template_error(
                    template_store::TEMPLATE_INVALID,
                    format!("runtime resolved {existing}, expected {expected}"),
                    "recommission the lens so runtime and manifest contracts agree",
                ));
            }
            lens.runtime_lens_id = Some(existing);
            emit_registration_progress_shared(
                progress_log,
                selector,
                "existing_matched",
                item.task.position,
                item.task.total,
                lens,
                Some(item.prepare_ms),
            )?;
            continue;
        }
        emit_registration_progress_shared(
            progress_log,
            selector,
            "runtime_register_start",
            item.task.position,
            item.task.total,
            lens,
            Some(item.prepare_ms),
        )?;
        let runtime_lens_id = item.prepared.contract.lens_id();
        if let Some(expected) = lens.runtime_lens_id
            && runtime_lens_id != expected
        {
            return Err(template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!("runtime registered {runtime_lens_id}, expected {expected}"),
                "recommission the lens so runtime and manifest contracts agree",
            ));
        }
        let registered = register_prepared_manifest_runtime(registry, item.prepared)?;
        if let Some(expected) = lens.runtime_lens_id
            && registered != expected
        {
            return Err(template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!("runtime registered {registered}, expected {expected}"),
                "recommission the lens so runtime and manifest contracts agree",
            ));
        }
        lens.runtime_lens_id = Some(registered);
        emit_registration_progress_shared(
            progress_log,
            selector,
            "runtime_register_ok",
            item.task.position,
            item.task.total,
            lens,
            Some(item.prepare_ms),
        )?;
        added += 1;
    }
    Ok(added)
}
