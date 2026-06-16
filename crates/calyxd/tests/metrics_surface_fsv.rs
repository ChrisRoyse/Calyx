//! Full-State-Verification for the PH66 T03 `/metrics` surface (issue #538).
//!
//! The source of truth is the bytes a Prometheus scrape actually reads over the
//! wire. This test drives synthetic-but-real observations through the production
//! recording API (`CalyxMetrics::observe_*`/`set_*`), binds the real
//! [`MetricsServer`] on a loopback port, performs a real HTTP/1.1 `GET /metrics`
//! over TCP, and asserts every required family appears with the exact value
//! computed from the known input — plus the 404/405 error paths. The
//! dynamic-cardinality families (guard/assay/kernel/anneal) materialize only
//! once their subsystems report, so they are exercised here through the same
//! code path the real ingest/search/guard dispatchers will use.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use calyxd::metrics::{
    CalyxMetrics, ChainVerifyMetrics, SearchStrategy, VerifyOutcome, ZfsDatasetChecksum,
    ZfsIntegritySnapshot, ZfsPoolIntegrity,
};
use calyxd::server::MetricsServer;
use tokio_util::sync::CancellationToken;

const VAULT: &str = "/data/fsv-vault";

fn recorded_surface() -> Arc<CalyxMetrics> {
    let labels = [VAULT.to_string()];
    let chain = Arc::new(ChainVerifyMetrics::new(&labels));
    // A proven-intact chain so the composed chain-verify family reads ok=1.
    chain.record(VAULT, &VerifyOutcome::Intact { entries: 7 }, 1_770_000_000);

    let surface = Arc::new(CalyxMetrics::new(Arc::clone(&chain), &labels));
    // Synthetic inputs with hand-computable exposition outputs:
    surface.observe_ingest(VAULT, 0.150, true); // 0.150s → le="0.25" bucket
    surface.observe_search(VAULT, SearchStrategy::WeightedRrf, 0.020, true);
    surface.set_recall_tripwire(VAULT, false); // tripped → 0
    surface.set_guard_rates(VAULT, "subject", 0.01, 0.02);
    surface.set_assay_n_eff(VAULT, "default", 128.0);
    surface.set_kernel_recall_ratio(VAULT, "global", 0.97);
    surface.record_anneal_exposure("beamwidth", "treatment");
    surface.set_anneal_improvement("beamwidth", 1.15);
    surface.set_vram_budget(4096, 8192);
    surface.record_zfs_integrity(&ZfsIntegritySnapshot {
        datasets: vec![ZfsDatasetChecksum {
            dataset: "hotpool/calyx".to_string(),
            enabled: true,
        }],
        pools: vec![ZfsPoolIntegrity {
            pool: "hotpool".to_string(),
            healthy: true,
            cksum_errors: 0,
            scrub_age_seconds: Some(86_400),
        }],
    });
    surface
        .set_hazard("disk_full", true)
        .expect("disk_full is a registered hazard");
    surface
}

#[test]
fn full_surface_served_over_real_http_with_recorded_values() {
    let surface = recorded_surface();
    let server =
        MetricsServer::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&surface)).expect("bind");
    let addr = server.local_addr().expect("local_addr").to_string();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let join = std::thread::spawn(move || server.run(cancel).expect("server run"));

    let body = http_get(&addr, "/metrics");
    assert!(body.starts_with("HTTP/1.1 200 OK"), "response: {body}");
    assert!(
        body.contains("Content-Type: text/plain; version=0.0.4"),
        "missing prometheus content-type: {body}"
    );

    // Composed chain-verify family (issue #602).
    assert_line(
        &body,
        "calyx_ledger_chain_verify_ok{vault=\"/data/fsv-vault\"} 1",
    );

    // Ingest: counter + histogram bucketing of the 0.150s sample.
    assert_line(
        &body,
        "calyx_ingest_total{status=\"ok\",vault=\"/data/fsv-vault\"} 1",
    );
    assert_line(
        &body,
        "calyx_ingest_duration_seconds_bucket{vault=\"/data/fsv-vault\",le=\"0.25\"} 1",
    );
    assert_line(
        &body,
        "calyx_ingest_duration_seconds_bucket{vault=\"/data/fsv-vault\",le=\"0.1\"} 0",
    );

    // Search: per-strategy counter and the tripped recall gauge.
    assert_line(
        &body,
        "calyx_search_total{status=\"ok\",strategy=\"weighted_rrf\",vault=\"/data/fsv-vault\"} 1",
    );
    assert_line(
        &body,
        "calyx_search_recall_tripwire{vault=\"/data/fsv-vault\"} 0",
    );

    // Dynamic-cardinality families materialize on first observation.
    assert_line(
        &body,
        "calyx_guard_far{slot=\"subject\",vault=\"/data/fsv-vault\"} 0.01",
    );
    assert_line(
        &body,
        "calyx_guard_frr{slot=\"subject\",vault=\"/data/fsv-vault\"} 0.02",
    );
    assert_line(
        &body,
        "calyx_assay_n_eff{panel=\"default\",vault=\"/data/fsv-vault\"} 128",
    );
    assert_line(
        &body,
        "calyx_kernel_recall_ratio{scope=\"global\",vault=\"/data/fsv-vault\"} 0.97",
    );
    assert_line(
        &body,
        "calyx_anneal_ab_variant_total{experiment=\"beamwidth\",variant=\"treatment\"} 1",
    );
    assert_line(
        &body,
        "calyx_anneal_ab_improvement_ratio{experiment=\"beamwidth\"} 1.15",
    );

    // VRAM budget — exact MiB values.
    assert_line(&body, "calyx_vram_budget_used_mib 4096");
    assert_line(&body, "calyx_vram_budget_limit_mib 8192");

    // ZFS integrity snapshot sourced from the same recording API the daemon uses.
    assert_line(&body, "calyx_zfs_pool_healthy{pool=\"hotpool\"} 1");
    assert_line(&body, "calyx_zfs_cksum_errors_total{pool=\"hotpool\"} 0");
    assert_line(&body, "calyx_zfs_scrub_age_seconds{pool=\"hotpool\"} 86400");
    assert_line(
        &body,
        "calyx_zfs_dataset_checksum_enabled{dataset=\"hotpool/calyx\"} 1",
    );

    // All 25 hazard gauges present; disk_full tripped to 1, the rest 0.
    let hazard_lines: Vec<&str> = body
        .lines()
        .filter(|line| line.starts_with("calyx_hazard_"))
        .collect();
    assert_eq!(hazard_lines.len(), 25, "expected 25 hazard lines: {body}");
    assert_line(&body, "calyx_hazard_disk_full{hazard=\"disk_full\"} 1");
    assert_line(&body, "calyx_hazard_vram_oom{hazard=\"vram_oom\"} 0");

    // Error paths: unknown path → 404, non-GET → 405.
    assert!(
        http_get(&addr, "/healthz").starts_with("HTTP/1.1 404"),
        "unknown path must 404"
    );
    let mut stream = TcpStream::connect(&addr).expect("connect");
    write!(stream, "POST /metrics HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("send");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read");
    assert!(
        response.starts_with("HTTP/1.1 405"),
        "non-GET must 405: {response}"
    );

    stop.cancel();
    join.join().expect("metrics server joins after cancel");
}

fn http_get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to metrics server");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("send request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn assert_line(text: &str, expected: &str) {
    assert!(
        text.lines().any(|line| line == expected),
        "expected exposition line {expected:?} in:\n{text}"
    );
}
