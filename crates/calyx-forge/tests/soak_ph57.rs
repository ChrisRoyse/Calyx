use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use calyx_forge::{
    AdmissionController, BlockDeallocator, DevicePtr, ForgeError, GpuBlockRegistry,
    RESERVED_HEADROOM_BYTES, Result, VramBudgeter, VramProbe, VramStats,
};
use serde::Serialize;

const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * MIB;
const BUDGET_CODE: &str = "CALYX_FORGE_VRAM_BUDGET";
const MAX_VRAM_MIB: u64 = 31 * 1024;
const MAX_POWER_W: u32 = 600;

#[derive(Clone, Copy)]
struct TeiEndpoint {
    port: u16,
    path: &'static str,
    body: &'static str,
}

const TEI_ENDPOINTS: [TeiEndpoint; 3] = [
    TeiEndpoint {
        port: 8088,
        path: "/embed",
        body: r#"{"inputs":"calyx ph57 synthetic latency probe"}"#,
    },
    TeiEndpoint {
        port: 8089,
        path: "/rerank",
        body: r#"{"query":"calyx ph57","texts":["synthetic latency probe","different text"]}"#,
    },
    TeiEndpoint {
        port: 8090,
        path: "/embed",
        body: r#"{"inputs":"calyx ph57 synthetic latency probe"}"#,
    },
];

#[derive(Serialize)]
struct SoakReadback {
    source_of_truth: &'static str,
    tei_health: Vec<HealthReadback>,
    tei_baseline: TeiLoadReadback,
    tei_during_forge: TeiLoadReadback,
    one_tei_edge: TeiLoadReadback,
    one_tei_forge: ForgeLoadReadback,
    forge: ForgeLoadReadback,
    nvidia_smi_samples: Vec<NvidiaSample>,
    max_memory_used_mib: u64,
    max_power_draw_w: u32,
    metric_text: String,
    admission_overhead_ns: f64,
}

#[derive(Clone, Serialize)]
struct HealthReadback {
    port: u16,
    status: u16,
    latency_ms: f64,
}

#[derive(Clone, Serialize)]
struct TeiRequestReadback {
    port: u16,
    path: &'static str,
    status: u16,
    latency_ms: f64,
    ok: bool,
}

#[derive(Clone, Serialize)]
struct TeiLoadReadback {
    requested: usize,
    successes: usize,
    failures: usize,
    p50_ms: f64,
    p99_ms: f64,
    requests: Vec<TeiRequestReadback>,
}

#[derive(Serialize)]
struct ForgeLoadReadback {
    requested: usize,
    split_successes: usize,
    queued_or_failed_budget_errors: usize,
    panics: usize,
    other_errors: usize,
    before: VramStats,
    after: VramStats,
}

#[derive(Clone, Serialize)]
struct NvidiaSample {
    elapsed_ms: u128,
    memory_used_mib: u64,
    power_draw_w: u32,
}

struct StaticProbe {
    free: usize,
}

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(self.free)
    }
}

#[derive(Clone, Default)]
struct NoopDealloc;

impl BlockDeallocator for NoopDealloc {
    fn free(&self, _ptr: DevicePtr, _size: usize) -> Result<()> {
        Ok(())
    }
}

#[test]
fn concurrent_tei_and_forge_soak_writes_readback() -> Result<()> {
    let health = check_tei_health(&[8088, 8089, 8090]);
    if !health
        .iter()
        .any(|item| item.port == 8088 && item.status == 200)
    {
        println!("SKIP_PH57_SOAK: TEI :8088 /health not responsive");
        return Ok(());
    }
    assert!(health.iter().all(|item| item.status == 200));

    let tei_baseline = background_tei_load(100, &TEI_ENDPOINTS);
    assert_eq!(tei_baseline.failures, 0);

    let samples = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = start_nvidia_sampler(Arc::clone(&samples), Arc::clone(&stop));

    let started = Instant::now();
    let (tei_during_forge, forge) = thread::scope(|scope| {
        let tei = scope.spawn(|| background_tei_load(100, &TEI_ENDPOINTS));
        let forge = scope.spawn(|| forge_load(50, 2 * GIB));
        (tei.join().unwrap(), forge.join().unwrap())
    });
    while started.elapsed() < Duration::from_secs(6) {
        let _ = background_tei_load(9, &TEI_ENDPOINTS);
    }
    stop.store(true, Ordering::Release);
    sampler.join().expect("nvidia sampler thread");

    let one_tei_edge = background_tei_load(12, &TEI_ENDPOINTS[..1]);
    let one_tei_forge = forge_load(50, 2 * GIB);
    let sample_values = samples.lock().unwrap().clone();
    let max_memory_used_mib = sample_values
        .iter()
        .map(|sample| sample.memory_used_mib)
        .max()
        .unwrap_or(0);
    let max_power_draw_w = sample_values
        .iter()
        .map(|sample| sample.power_draw_w)
        .max()
        .unwrap_or(0);
    let admission_overhead_ns = measure_admission_overhead_ns();
    let metric_text = forge.after.admission_metrics_text();

    assert_eq!(tei_during_forge.failures, 0);
    assert!(tei_during_forge.p99_ms <= tei_baseline.p99_ms.max(1.0) * 2.0);
    assert!(forge.after.splits_total + forge.after.failed_total >= 1);
    assert!(forge.after.failed_total >= 1);
    assert_eq!(forge.panics, 0);
    assert_eq!(forge.other_errors, 0);
    assert!(metric_text.contains("calyx_forge_vram_budget_exceeded_total"));
    assert!(!sample_values.is_empty());
    assert!(max_memory_used_mib > 0);
    assert!(max_power_draw_w > 0);
    assert!(max_memory_used_mib <= MAX_VRAM_MIB);
    assert!(max_power_draw_w <= MAX_POWER_W);
    assert_eq!(one_tei_edge.failures, 0);
    assert!(one_tei_forge.after.failed_total >= 1);
    assert_eq!(one_tei_forge.panics, 0);
    assert_eq!(one_tei_forge.other_errors, 0);
    assert!(admission_overhead_ns < 1_000.0);

    let readback = SoakReadback {
        source_of_truth: "ph57_soak_vram.json + VramStats Prometheus text + nvidia-smi samples",
        tei_health: health,
        tei_baseline,
        tei_during_forge,
        one_tei_edge,
        one_tei_forge,
        forge,
        nvidia_smi_samples: sample_values,
        max_memory_used_mib,
        max_power_draw_w,
        metric_text,
        admission_overhead_ns,
    };
    let root = fsv_root();
    std::fs::create_dir_all(&root).map_err(io_error)?;
    let json_path = root.join("ph57_soak_vram.json");
    let prom_path = root.join("ph57_soak_metrics.prom");
    let json = serde_json::to_vec_pretty(&readback).map_err(|err| ForgeError::CacheError {
        op: "serialize PH57 soak readback".into(),
        path: json_path.display().to_string(),
        detail: err.to_string(),
        remediation: "fix PH57 soak serialization".into(),
    })?;
    std::fs::write(&json_path, json).map_err(io_error)?;
    std::fs::write(&prom_path, readback.metric_text.as_bytes()).map_err(io_error)?;

    println!("PH57_SOAK_JSON {}", json_path.display());
    println!("PH57_SOAK_PROM {}", prom_path.display());
    println!(
        "PH57_SOAK_SUMMARY baseline_p99_ms={:.3} loaded_p99_ms={:.3} max_vram_mib={} max_power_w={} failed_total={} overhead_ns={:.2}",
        readback.tei_baseline.p99_ms,
        readback.tei_during_forge.p99_ms,
        readback.max_memory_used_mib,
        readback.max_power_draw_w,
        readback.forge.after.failed_total,
        readback.admission_overhead_ns
    );
    Ok(())
}

fn background_tei_load(n: usize, endpoints: &[TeiEndpoint]) -> TeiLoadReadback {
    let requests = Arc::new(Mutex::new(Vec::with_capacity(n)));
    thread::scope(|scope| {
        for idx in 0..n {
            let endpoint = endpoints[idx % endpoints.len()];
            let out = Arc::clone(&requests);
            scope.spawn(move || {
                let started = Instant::now();
                let status = http_post(endpoint).unwrap_or(0);
                let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
                out.lock().unwrap().push(TeiRequestReadback {
                    port: endpoint.port,
                    path: endpoint.path,
                    status,
                    latency_ms,
                    ok: (200..300).contains(&status),
                });
            });
        }
    });
    let mut requests = requests.lock().unwrap().clone();
    requests.sort_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms));
    let successes = requests.iter().filter(|item| item.ok).count();
    let failures = requests.len().saturating_sub(successes);
    let latencies = requests
        .iter()
        .map(|item| item.latency_ms)
        .collect::<Vec<_>>();
    TeiLoadReadback {
        requested: n,
        successes,
        failures,
        p50_ms: percentile(&latencies, 0.50),
        p99_ms: percentile(&latencies, 0.99),
        requests,
    }
}

fn forge_load(n: usize, bytes_per_dispatch: usize) -> ForgeLoadReadback {
    let budgeter = VramBudgeter::with_soft_cap(
        8 * GIB,
        StaticProbe {
            free: RESERVED_HEADROOM_BYTES + GIB,
        },
    );
    let before = budgeter.stats();
    let registry = GpuBlockRegistry::new(&budgeter, NoopDealloc, 16);
    let controller = AdmissionController::new(&budgeter, Arc::new(Mutex::new(registry)), 4, 1);
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(n)));
    thread::scope(|scope| {
        for idx in 0..n {
            let out = Arc::clone(&outcomes);
            let ctl = &controller;
            scope.spawn(move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                    let batch = if idx % 2 == 0 { 8 } else { 1 };
                    ctl.run_with_admission(
                        bytes_per_dispatch,
                        batch,
                        Instant::now() + Duration::from_secs(2),
                        |_offset, len| {
                            thread::sleep(Duration::from_millis(20));
                            Ok(vec![0usize; len])
                        },
                    )
                }));
                out.lock().unwrap().push(classify_outcome(outcome));
            });
        }
    });
    let outcomes = outcomes.lock().unwrap();
    let after = budgeter.stats();
    ForgeLoadReadback {
        requested: n,
        split_successes: outcomes
            .iter()
            .filter(|item| matches!(item, ForgeOutcome::Success))
            .count(),
        queued_or_failed_budget_errors: outcomes
            .iter()
            .filter(|item| matches!(item, ForgeOutcome::Budget))
            .count(),
        panics: outcomes
            .iter()
            .filter(|item| matches!(item, ForgeOutcome::Panic))
            .count(),
        other_errors: outcomes
            .iter()
            .filter(|item| matches!(item, ForgeOutcome::Other))
            .count(),
        before,
        after,
    }
}

enum ForgeOutcome {
    Success,
    Budget,
    Other,
    Panic,
}

fn classify_outcome(outcome: std::thread::Result<Result<Vec<usize>>>) -> ForgeOutcome {
    match outcome {
        Err(_) => ForgeOutcome::Panic,
        Ok(Ok(_)) => ForgeOutcome::Success,
        Ok(Err(err)) if err.code() == BUDGET_CODE => ForgeOutcome::Budget,
        Ok(Err(_)) => ForgeOutcome::Other,
    }
}

fn check_tei_health(ports: &[u16]) -> Vec<HealthReadback> {
    ports
        .iter()
        .map(|port| {
            let started = Instant::now();
            let status = http_get(*port, "/health").unwrap_or(0);
            HealthReadback {
                port: *port,
                status,
                latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
            }
        })
        .collect()
}

fn http_get(port: u16, path: &str) -> std::io::Result<u16> {
    http_request(port, "GET", path, "")
}

fn http_post(endpoint: TeiEndpoint) -> std::io::Result<u16> {
    http_request(endpoint.port, "POST", endpoint.path, endpoint.body)
}

fn http_request(port: u16, method: &str, path: &str, body: &str) -> std::io::Result<u16> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response_status(&response).unwrap_or(0))
}

fn response_status(response: &str) -> Option<u16> {
    response
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn start_nvidia_sampler(
    samples: Arc<Mutex<Vec<NvidiaSample>>>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let started = Instant::now();
        loop {
            if let Ok(mut sample) = query_nvidia_smi(started) {
                sample.elapsed_ms = started.elapsed().as_millis();
                samples.lock().unwrap().push(sample);
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_secs(2));
        }
    })
}

fn query_nvidia_smi(started: Instant) -> std::io::Result<NvidiaSample> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("nvidia-smi query failed"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.trim().split(',').map(str::trim);
    let memory_used_mib = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| std::io::Error::other("nvidia-smi memory.used parse failed"))?;
    let power_draw_w = parts
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.ceil() as u32)
        .ok_or_else(|| std::io::Error::other("nvidia-smi power.draw parse failed"))?;
    Ok(NvidiaSample {
        elapsed_ms: started.elapsed().as_millis(),
        memory_used_mib,
        power_draw_w,
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 * p).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[idx]
}

fn measure_admission_overhead_ns() -> f64 {
    let budgeter = VramBudgeter::with_soft_cap(8 * GIB, StaticProbe { free: 64 * GIB });
    let registry = GpuBlockRegistry::new(&budgeter, NoopDealloc, 16);
    let controller = AdmissionController::new(&budgeter, Arc::new(Mutex::new(registry)), 8, 1);
    let deadline = Instant::now() + Duration::from_secs(60);
    let iterations = 10_000;
    let started = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(controller.decide(1024, 1, deadline));
    }
    started.elapsed().as_nanos() as f64 / iterations as f64
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/ph57-soak-fsv"))
}

fn io_error(err: std::io::Error) -> ForgeError {
    ForgeError::CacheError {
        op: "PH57 soak file IO".into(),
        path: fsv_root().display().to_string(),
        detail: err.to_string(),
        remediation: "ensure CALYX_FSV_ROOT exists and is writable".into(),
    }
}
