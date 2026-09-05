use super::discovery::{
    RESIDENT_DISCOVERY_SCHEMA, ResidentDiscovery, remove_resident_discovery, unix_now_ms,
    write_resident_discovery,
};
use super::dispatch::{dispatch_request, readiness};
use super::kernel_rank::{
    ResidentKernelRank, load as load_kernel_rank, serve as serve_kernel_rank,
};
use super::search::serve as serve_search;
use super::stream::serve_binary_measure_batch;
use super::*;
use calyx_registry::{
    VaultPanelState, load_vault_panel_contract_state, require_vault_registry_contracts,
};
use calyx_search::PersistedSearchIndexes;
use std::path::Path;

/// cuVS/CUDA context, loaded module, stream, and search workspace headroom.
/// This is outside the generation bytes reported by the CAGRA cache. The live
/// RTX 5090 peak is 504 MiB above cached generations, so admission rounds that
/// measured fixed cost up to 512 MiB.
const CAGRA_RUNTIME_OVERHEAD_MIB: u64 = 512;
const CAGRA_CACHE_MAX_ENTRIES: usize = 64;

pub(crate) struct ResidentService {
    pub(crate) state: ResidentWarmState,
    pub(crate) bind: SocketAddr,
    /// Live host GPU lease this resident republishes for ingest lease
    /// validation (#1936).
    pub(crate) reservation_id: String,
    pub(crate) lease_pid: u32,
    pub(crate) started: Instant,
    pub(crate) max_runtime_batch: usize,
    pub(crate) capacity_probe_input_count: usize,
    pub(crate) capacity_probe_ms: u128,
    pub(crate) capacity_probe_modalities: Vec<Modality>,
    pub(crate) onnx_shape_budget: Option<calyx_registry::OnnxShapeBucketBudget>,
    pub(crate) kernel_rank: Option<ResidentKernelRank>,
    pub(crate) vault_path: Option<PathBuf>,
    /// Startup-attested, measurement-incapable panel state used by exact
    /// search requests. Keeping it here prevents every request from
    /// reconstructing and probing all external lens runtimes again.
    pub(crate) search_contract_state: Option<VaultPanelState>,
    pub(crate) search_slot_cache: Mutex<SearchSlotCache>,
}

pub(crate) fn serve(args: &[String], lease: super::ResidentLeaseIdentity) -> CliResult {
    let mut flags = parse_serve_flags(args)?;
    let bind = flags.bind.unwrap_or(parse_addr(DEFAULT_BIND)?);
    ensure_loopback(bind)?;
    let home = resolve_home(&mut flags)?;
    if flags.template.is_some() == flags.vault.is_some() {
        return Err(CliError::usage(
            "calyx panel resident serve requires exactly one of --template <name-or-id> or --vault <vault>",
        ));
    }
    if flags.kernel_rank && flags.vault.is_none() {
        return Err(CliError::usage(
            "--kernel-rank requires panel resident serve --vault <vault>",
        ));
    }
    let listener = TcpListener::bind(bind)?;
    let local_addr = listener.local_addr()?;
    // Canonicalize the vault source before warm state consumes the flags so
    // discovery-file consumers can compare vault identity path-for-path.
    let discovery_vault = match flags.vault.as_deref() {
        Some(vault) => Some(vault.canonicalize().map_err(|error| {
            CliError::io(format!(
                "canonicalize resident --vault {}: {error}",
                vault.display()
            ))
        })?),
        None => None,
    };
    let discovery_template = flags.template.clone();
    let pin_kernel_rank = flags.kernel_rank;
    let max_runtime_batch = flags.max_runtime_batch.unwrap_or(DEFAULT_MAX_RUNTIME_BATCH);
    let state = load_resident_warm_state(warm_options(home.clone(), flags))?;
    configure_search_gpu_cache(discovery_vault.as_deref(), &state, lease.requested_mib)?;
    let search_contract_state = discovery_vault
        .as_deref()
        .map(|vault| {
            require_vault_registry_contracts(vault)?;
            load_vault_panel_contract_state(vault)
        })
        .transpose()?;
    let mut service = ResidentService {
        state,
        bind: local_addr,
        reservation_id: lease.reservation_id,
        lease_pid: lease.lease_pid,
        started: Instant::now(),
        max_runtime_batch,
        capacity_probe_input_count: 0,
        capacity_probe_ms: 0,
        capacity_probe_modalities: Vec::new(),
        onnx_shape_budget: None,
        kernel_rank: None,
        vault_path: discovery_vault.clone(),
        search_contract_state,
        search_slot_cache: Mutex::new(SearchSlotCache::new()),
    };
    let capacity = super::capacity::run(&service)?;
    service.capacity_probe_input_count = capacity.input_count;
    service.capacity_probe_ms = capacity.elapsed_ms;
    service.capacity_probe_modalities = capacity.modalities;
    service.onnx_shape_budget = capacity.onnx_shape_budget;
    service.kernel_rank = if pin_kernel_rank {
        load_kernel_rank(discovery_vault.as_deref())?
    } else {
        None
    };
    let service = Arc::new(service);
    let ready = readiness(&service);
    // Install ownership-aware lifecycle cleanup immediately after publishing
    // readiness. Every return path after this point either explicitly writes
    // ready=false or triggers the guard's fail-loud unwind update (#1657).
    let ready_file = service
        .state
        .ready_out
        .clone()
        .map(|path| ReadyFileGuard::publish(path, &ready))
        .transpose()?;
    let discovery_path = write_resident_discovery(
        &home,
        &ResidentDiscovery {
            schema: RESIDENT_DISCOVERY_SCHEMA.to_string(),
            bind: local_addr,
            process_id: std::process::id(),
            reservation_id: service.reservation_id.clone(),
            lease_pid: service.lease_pid,
            vault: discovery_vault,
            template: discovery_template,
            written_at_unix_ms: unix_now_ms(),
        },
    )?;
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=discovery_written path={} bind={local_addr}",
        discovery_path.display()
    );
    print_json(&ready)?;
    let served = serve_loop(listener, service);
    // Best-effort removal of this process's own record on graceful shutdown;
    // a crashed service leaves the file behind and ingest discovery detects
    // that via the live readiness probe.
    let removed = remove_resident_discovery(&home, std::process::id());
    let lifecycle = ready_file.map_or(Ok(()), |guard| {
        let status = if served.is_ok() { "stopped" } else { "failed" };
        guard.finish(status, served.as_ref().err())
    });
    served?;
    removed?;
    lifecycle
}

struct ReadyFileGuard {
    path: PathBuf,
    published: Value,
    process_id: u32,
    reservation_id: String,
    finalized: bool,
}

impl ReadyFileGuard {
    fn publish(path: PathBuf, ready: &ReadyResponse) -> CliResult<Self> {
        let published = serde_json::to_value(ready).map_err(|error| {
            CliError::runtime(format!("serialize resident readiness lifecycle: {error}"))
        })?;
        write_json_file(path.clone(), &published)?;
        Ok(Self {
            path,
            published,
            process_id: ready.process_id,
            reservation_id: ready.reservation_id.clone(),
            finalized: false,
        })
    }

    fn finish(mut self, lifecycle: &str, error: Option<&CliError>) -> CliResult {
        self.finalized = true;
        self.write_terminal(lifecycle, error)
    }

    fn write_terminal(&self, lifecycle: &str, error: Option<&CliError>) -> CliResult {
        let bytes = std::fs::read(&self.path).map_err(|read_error| {
            ready_lifecycle_error(format!(
                "read current readiness owner {} before lifecycle update: {read_error}",
                self.path.display()
            ))
        })?;
        let current: Value = serde_json::from_slice(&bytes).map_err(|decode_error| {
            ready_lifecycle_error(format!(
                "decode current readiness owner {} before lifecycle update: {decode_error}",
                self.path.display()
            ))
        })?;
        let current_pid = current.get("process_id").and_then(Value::as_u64);
        let current_reservation = current.get("reservation_id").and_then(Value::as_str);
        if current_pid != Some(u64::from(self.process_id))
            || current_reservation != Some(self.reservation_id.as_str())
        {
            return Err(ready_lifecycle_error(format!(
                "refuse to replace readiness {} because owner changed: expected process_id={} reservation_id={}, found process_id={current_pid:?} reservation_id={current_reservation:?}",
                self.path.display(),
                self.process_id,
                self.reservation_id
            )));
        }

        let mut terminal = self.published.clone();
        terminal["ready"] = json!(false);
        terminal["lifecycle"] = json!(lifecycle);
        terminal["lifecycle_unix_ms"] = json!(unix_now_ms());
        terminal["lifecycle_reason"] = json!(match (lifecycle, error.is_some()) {
            ("stopped", false) => "resident serve scope ended after an owned shutdown",
            (_, true) => "resident serve scope returned an error",
            _ => "resident serve scope unwound after readiness publication",
        });
        if let Some(error) = error {
            terminal["error"] = json!({
                "code": error.code(),
                "message": error.message(),
                "remediation": error.remediation(),
            });
        }
        write_json_file(self.path.clone(), &terminal)
    }
}

impl Drop for ReadyFileGuard {
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        if let Err(error) = self.write_terminal("failed", None) {
            eprintln!(
                "CALYX_PANEL_RESIDENT_RUNTIME {}",
                json!({
                    "event_code": "CALYX_PANEL_RESIDENT_READY_LIFECYCLE_FAILED",
                    "phase": "ready_file_guard_drop",
                    "error_code": error.code(),
                    "error_message": error.message(),
                    "remediation": error.remediation(),
                    "ready_out": self.path,
                    "process_id": self.process_id,
                    "reservation_id": self.reservation_id,
                })
            );
        }
    }
}

fn configure_search_gpu_cache(
    vault_path: Option<&Path>,
    state: &ResidentWarmState,
    lease_mib: u64,
) -> CliResult {
    let Some(vault_path) = vault_path else {
        return Ok(());
    };
    let indexes = PersistedSearchIndexes::open(vault_path)?;
    if !indexes.uses_gpu_dense_serving()? {
        return Ok(());
    }
    if state.max_resident_vram_mib > lease_mib {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_GPU_BUDGET",
            message: format!(
                "resident --max-resident-vram-mib={} exceeds its physically admitted GPU lease of {lease_mib} MiB",
                state.max_resident_vram_mib
            ),
            remediation: "launch `calyx gpu run --mib` with at least the same measured bound passed to `panel resident serve --max-resident-vram-mib`",
        }));
    }
    let available_mib = state
        .max_resident_vram_mib
        .checked_sub(state.estimated_resident_vram_mib)
        .ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_RESIDENT_SEARCH_GPU_BUDGET",
                message: format!(
                    "resident search GPU budget underflow: max_resident_vram_mib={} estimated_resident_vram_mib={}",
                    state.max_resident_vram_mib, state.estimated_resident_vram_mib
                ),
                remediation:
                    "correct the resident lens VRAM declarations or increase the brokered resident lease",
            })
        })?;
    let cache_mib = available_mib
        .checked_sub(CAGRA_RUNTIME_OVERHEAD_MIB)
        .ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_RESIDENT_SEARCH_GPU_BUDGET",
                message: format!(
                    "resident has {available_mib} MiB available for search, but CAGRA requires {CAGRA_RUNTIME_OVERHEAD_MIB} MiB of non-cache runtime headroom"
                ),
                remediation:
                    "increase --max-resident-vram-mib and the enclosing GPU lease by the measured search working set",
            })
        })?;
    let cache_bytes = cache_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| CliError::usage("resident CAGRA cache byte budget overflow"))?;
    calyx_sextant::index::configure_cagra_serving_cache(cache_bytes, CAGRA_CACHE_MAX_ENTRIES)?;
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=search_gpu_cache_configured lease_mib={lease_mib} available_mib={available_mib} runtime_overhead_mib={CAGRA_RUNTIME_OVERHEAD_MIB} cache_mib={cache_mib} max_entries={CAGRA_CACHE_MAX_ENTRIES}"
    );
    Ok(())
}

fn ready_lifecycle_error(message: String) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_PANEL_RESIDENT_READY_LIFECYCLE_FAILED",
        message,
        remediation: "inspect the named owner-only readiness file, process id, live socket, and reservation row; never treat ready=true as authoritative unless all four identities agree",
    })
}

fn resolve_home(flags: &mut ServeFlags) -> CliResult<PathBuf> {
    resolve_home_with(flags.home.take(), calyx_home)
}

pub(crate) fn resolve_home_with(
    provided: Option<PathBuf>,
    fallback: impl FnOnce() -> CliResult<PathBuf>,
) -> CliResult<PathBuf> {
    match provided {
        Some(home) => Ok(home),
        None => fallback(),
    }
}

fn warm_options(home: PathBuf, flags: ServeFlags) -> ResidentWarmOptions {
    ResidentWarmOptions {
        home,
        template: flags.template,
        vault: flags.vault,
        slots: flags.slots,
        modality: flags.modality,
        ready_out: flags.ready_out,
        max_resident_vram_mib: flags
            .max_resident_vram_mib
            .unwrap_or(DEFAULT_MAX_RESIDENT_VRAM_MIB),
        resident_overhead_multiplier_milli: flags
            .resident_overhead_multiplier_milli
            .unwrap_or(DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI),
        max_load_secs: flags.max_load_secs.unwrap_or(DEFAULT_MAX_LOAD_SECS),
        load_parallelism: flags.load_parallelism,
        progress_out: flags.progress_out,
    }
}

fn serve_loop(listener: TcpListener, service: Arc<ResidentService>) -> CliResult {
    let running = Arc::new(AtomicBool::new(true));
    #[cfg(unix)]
    let termination_signal = install_termination_flags()?;
    #[cfg(unix)]
    listener.set_nonblocking(true).map_err(|error| {
        CliError::io(format!(
            "set resident listener {} nonblocking for signal-aware shutdown: {error}",
            service.bind
        ))
    })?;
    let served = serve_loop_with_stop(
        listener,
        Arc::clone(&running),
        |stream, running| handle_client(stream, Arc::clone(&service), running),
        || {
            #[cfg(unix)]
            {
                termination_signal.load(Ordering::SeqCst) != 0
            }
            #[cfg(not(unix))]
            {
                false
            }
        },
    );
    #[cfg(unix)]
    {
        let signum = termination_signal.load(Ordering::SeqCst);
        if signum != 0 {
            return Err(CliError::from(CalyxError {
                code: "CALYX_PANEL_RESIDENT_SIGNALLED",
                message: format!(
                    "resident {} received {} and completed owned readiness cleanup",
                    service.bind,
                    resident_signal_name(signum)
                ),
                remediation: "the resident was intentionally interrupted; confirm ready=false, the listener is absent, and the GPU reservation is released before restarting it",
            }));
        }
    }
    served
}

#[cfg(unix)]
fn install_termination_flags() -> CliResult<Arc<std::sync::atomic::AtomicUsize>> {
    use signal_hook::consts::{SIGINT, SIGTERM};

    let signal = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for signum in [SIGTERM, SIGINT] {
        signal_hook::flag::register_usize(signum, Arc::clone(&signal), signum as usize).map_err(
            |error| {
                CliError::io(format!(
                    "install resident termination handler for signal {signum}: {error}"
                ))
            },
        )?;
    }
    Ok(signal)
}

#[cfg(unix)]
fn resident_signal_name(signum: usize) -> &'static str {
    match signum as i32 {
        signal_hook::consts::SIGTERM => "SIGTERM",
        signal_hook::consts::SIGINT => "SIGINT",
        _ => "an unknown signal",
    }
}

#[cfg(test)]
pub(super) fn serve_loop_with(
    listener: TcpListener,
    running: Arc<AtomicBool>,
    handle: impl FnMut(TcpStream, Arc<AtomicBool>) -> CliResult,
) -> CliResult {
    serve_loop_with_stop(listener, running, handle, || false)
}

fn serve_loop_with_stop(
    listener: TcpListener,
    running: Arc<AtomicBool>,
    mut handle: impl FnMut(TcpStream, Arc<AtomicBool>) -> CliResult,
    should_stop: impl Fn() -> bool,
) -> CliResult {
    while running.load(Ordering::SeqCst) && !should_stop() {
        let (stream, peer) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                eprintln!(
                    "CALYX_PANEL_RESIDENT_RUNTIME {}",
                    json!({
                        "event_code": "CALYX_PANEL_RESIDENT_ACCEPT_INTERRUPTED",
                        "phase": "accept_retry",
                        "error_message": error.to_string(),
                        "remediation": "no operator action required; the listener is retrying",
                    })
                );
                continue;
            }
            Err(error) => {
                return Err(CliError::io(format!(
                    "accept resident client on {}: {error}",
                    listener.local_addr().map_or_else(
                        |addr_error| format!("unknown ({addr_error})"),
                        |addr| addr.to_string()
                    )
                )));
            }
        };
        if !peer.ip().is_loopback() {
            eprintln!(
                "CALYX_PANEL_RESIDENT_RUNTIME {}",
                json!({
                    "event_code": "CALYX_PANEL_RESIDENT_NON_LOOPBACK_CLIENT",
                    "phase": "client_rejected",
                    "peer": peer.to_string(),
                    "remediation": "connect only through the configured loopback address",
                })
            );
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        if let Err(error) = handle(stream, Arc::clone(&running)) {
            eprintln!(
                "CALYX_PANEL_RESIDENT_RUNTIME {}",
                json!({
                    "event_code": "CALYX_PANEL_RESIDENT_CLIENT_ERROR",
                    "phase": "client_error",
                    "peer": peer.to_string(),
                    "error_code": error.code(),
                    "error_message": error.message(),
                    "cause_remediation": error.remediation(),
                    "remediation": "inspect and repair only the named client; use the resident JSON-line or binary protocol and read the complete response; the resident remains active",
                })
            );
        }
    }
    Ok(())
}

fn handle_client(
    mut stream: TcpStream,
    service: Arc<ResidentService>,
    running: Arc<AtomicBool>,
) -> CliResult {
    let timeout = Some(client_timeout()?);
    stream
        .set_read_timeout(timeout)
        .map_err(|error| CliError::io(format!("set resident client read timeout: {error}")))?;
    stream
        .set_write_timeout(timeout)
        .map_err(|error| CliError::io(format!("set resident client write timeout: {error}")))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| CliError::io(format!("clone resident client stream: {error}")))?;
    let mut reader = BufReader::new(reader_stream);
    let mut first_line = Vec::new();
    reader
        .read_until(b'\n', &mut first_line)
        .map_err(|error| CliError::io(format!("read resident client request line: {error}")))?;
    if first_line == RESIDENT_BINARY_MAGIC {
        serve_binary_measure_batch(&mut reader, &mut stream, &service)?;
        stream
            .flush()
            .map_err(|error| CliError::io(format!("flush resident binary response: {error}")))?;
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(());
    }
    if first_line == KERNEL_RANK_BINARY_MAGIC {
        serve_kernel_rank(&mut reader, &mut stream, &service)?;
        stream.flush().map_err(|error| {
            CliError::io(format!("flush resident kernel rank response: {error}"))
        })?;
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(());
    }
    if first_line == SEARCH_BINARY_MAGIC {
        serve_search(&mut reader, &mut stream, &service)?;
        stream
            .flush()
            .map_err(|error| CliError::io(format!("flush resident search response: {error}")))?;
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(());
    }

    let response = match String::from_utf8(first_line) {
        Ok(line) => match serde_json::from_str::<ResidentRequest>(&line) {
            Ok(request) => dispatch_request(request, &service, &running),
            Err(error) => error_value(
                "CALYX_PANEL_RESIDENT_BAD_REQUEST",
                format!("decode resident request JSON line: {error}"),
                "send one JSON object per connection with op=ready, measure, or shutdown",
            ),
        },
        Err(error) => error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            format!("resident request was neither binary magic nor valid UTF-8 JSON: {error}"),
            "send one JSON object per connection or the resident binary magic line",
        ),
    };
    serde_json::to_writer(&mut stream, &response)
        .map_err(|error| CliError::runtime(format!("write resident response JSON: {error}")))?;
    stream
        .write_all(b"\n")
        .map_err(|error| CliError::io(format!("write resident response terminator: {error}")))?;
    stream
        .flush()
        .map_err(|error| CliError::io(format!("flush resident JSON response: {error}")))?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}
