use super::discovery::{
    RESIDENT_DISCOVERY_SCHEMA, ResidentDiscovery, remove_resident_discovery, unix_now_ms,
    write_resident_discovery,
};
use super::dispatch::{dispatch_request, readiness};
use super::stream::serve_binary_measure_batch;
use super::*;

pub(crate) struct ResidentService {
    pub(crate) state: ResidentWarmState,
    pub(crate) bind: SocketAddr,
    pub(crate) started: Instant,
}

pub(crate) fn serve(args: &[String]) -> CliResult {
    let mut flags = parse_serve_flags(args)?;
    let bind = flags.bind.unwrap_or(parse_addr(DEFAULT_BIND)?);
    ensure_loopback(bind)?;
    let home = resolve_home(&mut flags)?;
    if flags.template.is_some() == flags.vault.is_some() {
        return Err(CliError::usage(
            "calyx panel resident serve requires exactly one of --template <name-or-id> or --vault <vault>",
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
    let state = load_resident_warm_state(warm_options(home.clone(), flags))?;
    let service = Arc::new(ResidentService {
        state,
        bind: local_addr,
        started: Instant::now(),
    });
    let ready = readiness(&service);
    if let Some(path) = service.state.ready_out.clone() {
        write_json_file(path, &ready)?;
    }
    let discovery_path = write_resident_discovery(
        &home,
        &ResidentDiscovery {
            schema: RESIDENT_DISCOVERY_SCHEMA.to_string(),
            bind: local_addr,
            process_id: std::process::id(),
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
    served?;
    removed
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
    while running.load(Ordering::SeqCst) {
        let (stream, peer) = listener.accept()?;
        if !peer.ip().is_loopback() {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        handle_client(stream, Arc::clone(&service), Arc::clone(&running))?;
    }
    Ok(())
}

fn handle_client(
    mut stream: TcpStream,
    service: Arc<ResidentService>,
    running: Arc<AtomicBool>,
) -> CliResult {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first_line = Vec::new();
    reader.read_until(b'\n', &mut first_line)?;
    if first_line == RESIDENT_BINARY_MAGIC {
        serve_binary_measure_batch(&mut reader, &mut stream, &service)?;
        stream.flush()?;
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
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}
