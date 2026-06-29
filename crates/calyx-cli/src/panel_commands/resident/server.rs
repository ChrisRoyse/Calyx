use super::codec::write_binary_response;
use super::dispatch::{dispatch_binary_measure_batch, dispatch_request, readiness};
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
    let state = load_resident_warm_state(warm_options(home, flags))?;
    let service = Arc::new(ResidentService {
        state,
        bind: local_addr,
        started: Instant::now(),
    });
    let ready = readiness(&service);
    if let Some(path) = service.state.ready_out.clone() {
        write_json_file(path, &ready)?;
    }
    print_json(&ready)?;
    serve_loop(listener, service)
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
        let response = dispatch_binary_measure_batch(&mut reader, &service);
        write_binary_response(&mut stream, &response)?;
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
