//! `calyx-poly-recall-run` — the local computed-kernel recall gate over ingested resolved markets
//! (issues #219 / #223; the #172/#173 → #219 bridge).
//!
//! This is the missing executable that turns the (previously unwired) corpus builder in
//! [`calyx_poly::resolved_market_corpus`] into a runnable gate over real, locally-ingested data.
//! It reads Polymarket **gamma closed-market** pages from a landing directory, derives a
//! `(MarketSnapshot, Resolution)` per market, and — critically — admits a market into the recall
//! corpus only if it clears **as-of / no-look-ahead** and **non-degenerate** guards, then runs the
//! real Lodestar recall engine via [`run_local_computed_kernel_recall`] and persists the report.
//!
//! ## Why the guards exist (do not remove)
//! A *closed*-market gamma record is the market's **terminal** state: `volume24hr == 0`, a
//! `bestBid=0 / bestAsk=1 → spread=1` book, and `outcomePrices ≈ [0, 1]` — i.e. the price **is** the
//! resolution. Feeding that into the record vector would leak the outcome into the feature it is
//! meant to be predicted from — a look-ahead violation (issue #80) that would produce a *misleading*
//! recall ratio. So a resolved market is admitted only when it carries a genuine **pre-resolution**
//! snapshot (real volume/liquidity/spread, snapshot time strictly before resolution time). On
//! today's local corpus every record is terminal-state, so this binary **fails closed with an exact
//! census** rather than manufacturing a green over a broken data state. When a real pre-resolution
//! snapshot source lands (issue #77), the same binary admits those rows and runs the gate.
//!
//! Exit codes: `0` gate passed · `1` gate ran but did not clear the floor (or no market was
//! admissible — a data gap, reported with the census) · `2` hard error.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use calyx_core::SystemClock;
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallTestParams};
use calyx_poly::resolved_market_corpus::{LocalRecallRunParams, run_local_computed_kernel_recall};
use calyx_poly::resolved_market_gamma_loader::load_admissible_markets;
use calyx_poly::{Domain, PolyError, PolyLogEvent, StructuredLogSink, log_context};
use serde_json::json;

const C_USAGE: &str = "POLY_RECALL_RUN_USAGE";
const C_READ_DIR: &str = "POLY_RECALL_RUN_READ_DIR";
const C_NO_ADMISSIBLE: &str = "POLY_RECALL_RUN_NO_ADMISSIBLE_MARKET";
const C_ENCODE: &str = "POLY_RECALL_RUN_ENCODE";

/// Cosine floor for admitting a between-record agreement edge.
const DEFAULT_AGREEMENT_THRESHOLD: f32 = 0.99;
struct Cli {
    input_dir: PathBuf,
    out_dir: PathBuf,
    log_path: PathBuf,
    panel_version: u32,
    vault_salt: String,
    domain: Domain,
    min_recall_ratio: f32,
}

enum CliAction {
    Help,
    Run(Cli),
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            let payload = json!({ "ok": false, "error": error.diagnostic() });
            eprintln!(
                "{}",
                serde_json::to_string(&payload).unwrap_or_else(|_| {
                    "{\"ok\":false,\"code\":\"POLY_RECALL_RUN_ERROR_ENCODE_FAILED\"}".to_string()
                })
            );
            process::exit(2);
        }
    }
}

fn run() -> calyx_poly::Result<i32> {
    let CliAction::Run(cli) = parse_cli(env::args().skip(1).collect())? else {
        println!("{}", usage());
        return Ok(0);
    };
    let sink = StructuredLogSink::new(cli.log_path.clone())?;
    let clock = SystemClock;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "recall_run",
        "start",
        "POLY_RECALL_RUN_STARTED",
        "measuring computed-kernel recall over locally-ingested resolved markets",
        log_context(&[
            ("input_dir", cli.input_dir.display().to_string()),
            ("out_dir", cli.out_dir.display().to_string()),
            ("domain", cli.domain.slug().to_string()),
            ("panel_version", cli.panel_version.to_string()),
        ]),
    )?)?;

    let loaded = load_admissible_markets(&cli.input_dir)?;
    let inputs = loaded.inputs();
    let census = &loaded.census;

    let census_ctx = log_context(&[
        ("files_read", census.files_read.to_string()),
        ("markets_seen", census.markets_seen.to_string()),
        (
            "skipped_not_binary_or_ids",
            census.skipped_not_binary_or_ids.to_string(),
        ),
        (
            "unresolved_no_clean_winner",
            census.unresolved_no_clean_winner.to_string(),
        ),
        (
            "rejected_terminal_degenerate",
            census.rejected_terminal_degenerate.to_string(),
        ),
        ("rejected_lookahead", census.rejected_lookahead.to_string()),
        ("admitted", census.admitted.to_string()),
    ]);

    if inputs.is_empty() {
        // Data gap, reported loud — never a fabricated green. This is the expected outcome on the
        // current local corpus (terminal-state closed markets only; pre-resolution snapshots need
        // issue #77 / #80). Exit 1 = gate could not run over admissible data.
        let err = PolyError::diagnostics(
            C_NO_ADMISSIBLE,
            format!(
                "no admissible resolved market with a pre-resolution snapshot in {} \
                 (seen {}, unresolved {}, terminal/degenerate {}, look-ahead {}); local closed-market \
                 data is terminal-state only — genuine pre-resolution snapshots require issue #77/#80",
                cli.input_dir.display(),
                census.markets_seen,
                census.unresolved_no_clean_winner,
                census.rejected_terminal_degenerate,
                census.rejected_lookahead,
            ),
        );
        sink.append_error(&clock, "recall_run", "admit", &err, census_ctx)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "ok": false, "gate_ran": false, "census": census })
            )
            .unwrap_or_default()
        );
        return Ok(1);
    }

    sink.append_event(&PolyLogEvent::info(
        &clock,
        "recall_run",
        "admit",
        "POLY_RECALL_RUN_CORPUS_ADMITTED",
        "admitted pre-resolution resolved markets into the recall corpus",
        census_ctx,
    )?)?;

    let kp = KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    let rp = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 219,
        min_recall_ratio: cli.min_recall_ratio,
    };
    fs::create_dir_all(&cli.out_dir).map_err(|e| {
        PolyError::diagnostics(
            C_READ_DIR,
            format!("create out_dir {}: {e}", cli.out_dir.display()),
        )
    })?;
    let params = LocalRecallRunParams {
        domain: cli.domain,
        panel_version: cli.panel_version,
        vault_salt: cli.vault_salt.as_bytes(),
        agreement_threshold: DEFAULT_AGREEMENT_THRESHOLD,
        kernel_params: &kp,
        recall_params: &rp,
        persist_dir: Some(cli.out_dir.as_path()),
    };

    let (_corpus, recall) = run_local_computed_kernel_recall(&inputs, &params)?;
    let stdout = serde_json::to_string_pretty(&json!({
        "ok": recall.gate_passed,
        "gate_ran": true,
        "domain": recall.domain.slug(),
        "corpus_len": recall.corpus_len,
        "measured_ratio": recall.measured_ratio,
        "min_ratio": recall.min_ratio,
        "gate_passed": recall.gate_passed,
        "n_queries_tested": recall.n_queries_tested,
        "kernel_member_count": recall.fvs_kernel.kernel_member_count,
        "census": census,
    }))
    .map_err(|e| PolyError::diagnostics(C_ENCODE, format!("encode stdout: {e}")))?;
    println!("{stdout}");

    sink.append_event(&PolyLogEvent::info(
        &clock,
        "recall_run",
        "measured",
        if recall.gate_passed {
            "POLY_RECALL_RUN_GATE_PASSED"
        } else {
            "POLY_RECALL_RUN_GATE_FAILED"
        },
        "computed-kernel recall measured over the local resolved-market corpus",
        log_context(&[
            ("domain", recall.domain.slug().to_string()),
            ("corpus_len", recall.corpus_len.to_string()),
            ("measured_ratio", format!("{:.6}", recall.measured_ratio)),
            ("min_ratio", format!("{:.6}", recall.min_ratio)),
            ("gate_passed", recall.gate_passed.to_string()),
        ]),
    )?)?;

    Ok(if recall.gate_passed { 0 } else { 1 })
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut input_dir: Option<PathBuf> = None;
    let mut out_dir = PathBuf::from("target/fsv/computed_kernel_recall");
    let mut log_path: Option<PathBuf> = None;
    let mut panel_version: u32 = 1;
    let mut vault_salt = String::from("issue219-corpus-salt");
    let mut domain = Domain::Crypto;
    let mut min_recall_ratio: f32 = 0.95;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--input-dir" => {
                i += 1;
                input_dir = Some(PathBuf::from(required_value(&args, i, "--input-dir")?));
            }
            "--out-dir" => {
                i += 1;
                out_dir = PathBuf::from(required_value(&args, i, "--out-dir")?);
            }
            "--log" => {
                i += 1;
                log_path = Some(PathBuf::from(required_value(&args, i, "--log")?));
            }
            "--panel-version" => {
                i += 1;
                let v = required_value(&args, i, "--panel-version")?;
                panel_version = v
                    .parse()
                    .map_err(|e| usage_error(format!("parse --panel-version {v}: {e}")))?;
            }
            "--vault-salt" => {
                i += 1;
                vault_salt = required_value(&args, i, "--vault-salt")?.to_string();
            }
            "--domain" => {
                i += 1;
                let v = required_value(&args, i, "--domain")?;
                domain = domain_from_slug(v)
                    .ok_or_else(|| usage_error(format!("unknown --domain {v}")))?;
            }
            "--min-recall-ratio" => {
                i += 1;
                let v = required_value(&args, i, "--min-recall-ratio")?;
                min_recall_ratio = v
                    .parse()
                    .map_err(|e| usage_error(format!("parse --min-recall-ratio {v}: {e}")))?;
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        i += 1;
    }

    let input_dir = input_dir.ok_or_else(|| usage_error("--input-dir is required"))?;
    let log_path = log_path.unwrap_or_else(|| out_dir.join("recall-run.jsonl"));
    Ok(CliAction::Run(Cli {
        input_dir,
        out_dir,
        log_path,
        panel_version,
        vault_salt,
        domain,
        min_recall_ratio,
    }))
}

fn domain_from_slug(slug: &str) -> Option<Domain> {
    [
        Domain::Crypto,
        Domain::Politics,
        Domain::Sports,
        Domain::Economics,
        Domain::Weather,
        Domain::Culture,
        Domain::Geopolitics,
        Domain::Mentions,
        Domain::Other,
    ]
    .into_iter()
    .find(|d| d.slug() == slug)
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::config(C_USAGE, format!("{}\n{}", message.into(), usage()))
}

fn usage() -> String {
    "usage: calyx-poly-recall-run --input-dir DIR [--out-dir DIR] [--log PATH] \
     [--domain crypto] [--panel-version N] [--vault-salt STR] [--min-recall-ratio F]"
        .to_string()
}
