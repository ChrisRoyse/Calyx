//! Issue #28 - UMA resolution watcher and finality gate.
//!
//! Source of truth: durable AsterVault Anchors/Ledger CF rows plus persisted watcher/readback JSON.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use calyx_aster::cf::{ColumnFamily, anchor_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{Anchor, AnchorKind, CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::constellation::resolution_anchor;
use calyx_poly::grounding::{
    GroundingKind, ResolutionSupersessionKind, grounding_kind_of, supersede_gamma_closed_resolution,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use calyx_poly::{
    ERR_UMA_RESOLUTION_LOG_INVALID, ERR_UMA_RESOLUTION_NOT_FINAL, UmaResolutionObservation,
    UmaResolutionWatcherRequest, decode_condition_resolution_data, evaluate_uma_resolution,
    parse_condition_resolution_log_value, require_groundable_uma_resolution,
    run_uma_resolution_watcher,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue028-uma-resolution";

#[test]
fn issue028_uma_resolution_parser_known_truth_edges() {
    let (slots, payouts) = decode_condition_resolution_data(&abi_data(&[1, 0])).unwrap();
    assert_eq!(slots, 2);
    assert_eq!(payouts, vec![1, 0]);

    let log = condition_resolution_log(&[0, 1]);
    let obs = parse_condition_resolution_log_value(&log, labels(), 1_785_600_000).unwrap();
    assert_eq!(obs.condition_id, topic_word("11"));
    assert_eq!(obs.oracle, "0x0000000000000000000000000000000000000022");
    assert_eq!(obs.payout_numerators, vec![0, 1]);
    assert_eq!(obs.source_block_number, Some(16));
    assert_eq!(obs.source_log_index, Some(2));

    let err = decode_condition_resolution_data("0x1234").unwrap_err();
    assert_eq!(err.code(), ERR_UMA_RESOLUTION_LOG_INVALID);

    let mut bad_log = log;
    bad_log["topics"] = json!([topic_word("aa")]);
    assert_eq!(
        parse_condition_resolution_log_value(&bad_log, labels(), 1)
            .unwrap_err()
            .code(),
        ERR_UMA_RESOLUTION_LOG_INVALID
    );
}

#[test]
fn issue028_uma_resolution_watcher_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE028_FSV_ROOT", "issue028-uma-resolution");
    reset_dir(&root);

    let observations = vec![
        finalized("cond028final", vec![1, 0]),
        in_liveness("cond028live"),
        disputed("cond028dispute"),
        finalized("cond028void", vec![1, 1]),
        finalized("cond028umawins", vec![0, 1]),
    ];
    let watcher = run_uma_resolution_watcher(
        &root.join("watcher"),
        &UmaResolutionWatcherRequest {
            observations: observations.clone(),
        },
    )
    .expect("run UMA watcher");
    assert_eq!(watcher.report.observed_count, 5);
    assert_eq!(watcher.report.finalized_resolution_count, 2);
    assert_eq!(watcher.report.held_count, 2);
    assert_eq!(watcher.report.voided_count, 1);

    let final_case = finalized_grounding_roundtrip(&root, &observations[0]);
    let held_cases = held_states_do_not_ground(&root, &observations[1..4]);
    let correction = gamma_disagreement_uma_wins(&root, &observations[4]);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 28,
        "proof_claim": "UMA observations emit groundable Resolution records only for finalized, non-disputed, single-winner CTF payout vectors; liveness-open, active-dispute, and void/invalid states remain held and cannot write grounding anchors; a Gamma-derived disagreement is superseded by UMA-onchain provenance.",
        "minimum_sufficient_proof_corpus": {
            "observations": 5,
            "synthetic_parser_edges": 4,
            "why_this_is_sufficient": "One finalized winner proves the happy grounding path; one in-liveness case, one active dispute, and one void payout prove the required fail-closed states; one Gamma-vs-UMA disagreement proves UMA finality supersedes derived Gamma inference.",
            "why_smaller_is_insufficient": "Fewer than five observations would omit either a required finality edge or the Gamma disagreement edge from #28.",
            "why_larger_is_wasteful": "Additional markets would repeat the same decision, grounding, and readback paths without adding a new #28 invariant."
        },
        "source_of_truth": "durable AsterVault Anchors/Ledger CF rows plus persisted UMA watcher JSON readback",
        "watcher_report_path": watcher.report_path.display().to_string(),
        "watcher_report": watcher.report,
        "finalized_grounding": final_case,
        "held_states": held_cases,
        "gamma_disagreement_uma_wins": correction,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue028_uma_resolution_watcher_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires live public Polygon RPC"]
fn issue028_uma_resolution_live_rpc_probe_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE028_LIVE_ROOT", "issue028-uma-live-rpc");
    reset_dir(&root);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let block = post_json(
        &agent,
        "https://polygon.drpc.org",
        json_rpc("eth_blockNumber", json!([])),
    );
    let latest = block["result"].as_str().expect("block result");
    let logs = post_json(
        &agent,
        "https://polygon.drpc.org",
        json_rpc(
            "eth_getLogs",
            json!([{
                "fromBlock": latest,
                "toBlock": latest,
                "address": "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045",
                "topics": [Value::Null, topic_word("11")]
            }]),
        ),
    );
    let evidence = json!({
        "issue": 28,
        "proof_claim": "The live public Polygon RPC source is reachable for minimal bounded UMA/CTF reads without using a historical backfill range.",
        "minimum_sufficient_proof_corpus": {
            "rpc_calls": 2,
            "why_this_is_sufficient": "One block-number read proves live Polygon JSON-RPC reachability; one same-block eth_getLogs call proves the ConditionResolution filter shape is accepted while keeping the proof corpus to one block.",
            "why_smaller_is_insufficient": "A block-number read alone would not prove the log filter surface.",
            "why_larger_is_wasteful": "A broad historical scan is not needed to prove #28 watcher logic and public RPC filter compatibility."
        },
        "block_number_body": block,
        "same_block_logs_body": logs
    });
    let path = root.join("live-rpc-readback.json");
    write_json(&path, &evidence);
    let readback: Value =
        serde_json::from_slice(&fs::read(&path).expect("read live evidence")).expect("decode");
    assert_eq!(readback, evidence);
    write_blake3sums(&root);
}

fn finalized_grounding_roundtrip(root: &Path, observation: &UmaResolutionObservation) -> Value {
    let fixture = ingest_fixture(&root.join("finalized-grounding"), &observation.condition_id);
    let decision = evaluate_uma_resolution(observation).expect("evaluate final");
    let resolution = require_groundable_uma_resolution(&decision).expect("groundable");
    let refs = ground_market(&fixture.vault, &[fixture.cx_id], resolution, 0).expect("ground");
    fixture.vault.flush().expect("flush final");
    let anchor = read_anchor(&fixture.vault, fixture.cx_id, AnchorKind::TestPass);
    let ledger = ledger_payload(&fixture.vault, refs[0].seq);
    assert_eq!(
        grounding_kind_of(&anchor).unwrap(),
        GroundingKind::ResolvedUma
    );
    persist_case(
        root,
        "finalized-grounding",
        json!({"decision": decision, "anchor": anchor, "ledger": ledger}),
    )
}

fn held_states_do_not_ground(root: &Path, observations: &[UmaResolutionObservation]) -> Value {
    let mut cases = Vec::new();
    for observation in observations {
        let fixture = ingest_fixture(
            &root.join(format!("held-{}", observation.condition_id)),
            &observation.condition_id,
        );
        let before = vault_counts(&fixture.vault);
        let decision = evaluate_uma_resolution(observation).expect("evaluate held");
        let err = require_groundable_uma_resolution(&decision).unwrap_err();
        assert_eq!(err.code(), ERR_UMA_RESOLUTION_NOT_FINAL);
        fixture.vault.flush().expect("flush held");
        let after = vault_counts(&fixture.vault);
        assert_eq!(before, after);
        cases.push(persist_case(
            root,
            &format!("held-{}", observation.condition_id),
            json!({"decision": decision, "error_code": err.code(), "before": before, "after": after}),
        ));
    }
    json!(cases)
}

fn gamma_disagreement_uma_wins(root: &Path, observation: &UmaResolutionObservation) -> Value {
    let fixture = ingest_fixture(&root.join("gamma-disagreement"), &observation.condition_id);
    let gamma = resolution(&observation.condition_id, 0, "YES", "gamma-closed-derived");
    ground_market(&fixture.vault, &[fixture.cx_id], &gamma, 0).expect("gamma ground");
    fixture.vault.flush().expect("flush gamma");
    let gamma_anchor = read_anchor(&fixture.vault, fixture.cx_id, AnchorKind::TestPass);
    let decision = evaluate_uma_resolution(observation).expect("evaluate UMA correction");
    let uma = require_groundable_uma_resolution(&decision).expect("UMA final");
    assert_eq!(uma.winning_outcome_index, 1);
    let uma_anchor = resolution_anchor(uma, 0);
    let supersession = supersede_gamma_closed_resolution(&gamma_anchor, &uma_anchor).unwrap();
    assert_eq!(
        supersession.kind,
        ResolutionSupersessionKind::CorrectionOnDisagreement
    );
    let payload = json!({
        "event": "poly.uma_resolution_supersedes_gamma",
        "condition_id": observation.condition_id,
        "uma_source": supersession.uma_source,
        "gamma_source": supersession.gamma_source,
        "kind": format!("{:?}", supersession.kind),
        "gamma_outcome": supersession.gamma_outcome,
        "uma_outcome": supersession.uma_outcome
    });
    let ledger_ref = fixture
        .vault
        .append_ledger_entry(
            EntryKind::Grounding,
            SubjectId::Cx(fixture.cx_id),
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service("calyx-poly-uma-resolution".to_string()),
        )
        .expect("append correction ledger");
    fixture.vault.flush().expect("flush correction");
    let readback = ledger_payload(&fixture.vault, ledger_ref.seq);
    assert_eq!(readback, payload);
    persist_case(
        root,
        "gamma-disagreement",
        json!({
            "decision": decision,
            "gamma_anchor": gamma_anchor,
            "uma_anchor": uma_anchor,
            "supersession": supersession,
            "ledger": readback
        }),
    )
}

fn ingest_fixture(root: &Path, condition_id: &str) -> Fixture {
    reset_dir(root);
    let vault = AsterVault::new_durable(
        root.join("vault"),
        VAULT_ID.parse().unwrap(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let panel = default_panel(1, vec!["global".to_string()]);
    let cx_id = ingest_snapshot(
        &vault,
        &panel,
        &snapshot(condition_id),
        VAULT_ID.parse::<VaultId>().unwrap(),
        VAULT_SALT,
    )
    .unwrap();
    Fixture { vault, cx_id }
}

fn read_anchor(vault: &AsterVault, cx_id: CxId, kind: AnchorKind) -> Anchor {
    let row = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Anchors,
            &anchor_key(cx_id, &kind),
        )
        .unwrap()
        .expect("anchor present");
    encode::decode_anchor(&row).unwrap()
}

fn ledger_payload(vault: &AsterVault, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger present");
    let ledger = decode_ledger(&row).unwrap();
    serde_json::from_slice(&ledger.payload).unwrap()
}

fn vault_counts(vault: &AsterVault) -> Value {
    json!({
        "anchors": vault.scan_cf_at(vault.snapshot(), ColumnFamily::Anchors).unwrap().len(),
        "ledger": vault.scan_cf_at(vault.snapshot(), ColumnFamily::Ledger).unwrap().len()
    })
}

fn persist_case(root: &Path, name: &str, value: Value) -> Value {
    let path = root.join(name).join("readback.json");
    write_json(&path, &value);
    let readback: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(readback, value);
    json!({"path": path.display().to_string(), "readback_equal": true, "value": readback})
}

fn finalized(condition_id: &str, payouts: Vec<u64>) -> UmaResolutionObservation {
    UmaResolutionObservation {
        condition_id: condition_id.to_string(),
        question_id: Some(format!("{condition_id}-question")),
        oracle: "0x6A9D222616C90FcA5754cd1333cFD9b7fb6a4F74".to_string(),
        outcome_labels: labels(),
        payout_numerators: payouts,
        proposed_ts: Some(1_785_590_000),
        expiration_ts: Some(1_785_597_200),
        observed_ts: 1_785_600_000,
        resolved_ts: Some(1_785_600_000),
        active_dispute: false,
        voided_invalid: false,
        source_tx_hash: Some(tx_hash(condition_id)),
        source_block_number: Some(89_800_000),
        source_log_index: Some(1),
    }
}

fn in_liveness(condition_id: &str) -> UmaResolutionObservation {
    let mut obs = finalized(condition_id, Vec::new());
    obs.observed_ts = 1_785_590_100;
    obs.resolved_ts = None;
    obs
}

fn disputed(condition_id: &str) -> UmaResolutionObservation {
    let mut obs = in_liveness(condition_id);
    obs.active_dispute = true;
    obs
}

fn snapshot(condition_id: &str) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{condition_id}-yes"),
        condition_id: condition_id.to_string(),
        outcome_index: 0,
        slug: condition_id.to_string(),
        question: Some(format!("{condition_id}?")),
        event_id: None,
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec![],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(1000.0),
        liquidity: Some(500.0),
        one_hour_change: None,
        one_day_change: None,
        ofi: None,
        yes_no_residual: None,
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn resolution(condition_id: &str, winner: u32, label: &str, source: &str) -> Resolution {
    Resolution {
        condition_id: condition_id.to_string(),
        winning_outcome_index: winner,
        winning_label: label.to_string(),
        resolved_ts: 1_785_600_000,
        source: source.to_string(),
        disputed: false,
    }
}

fn labels() -> Vec<String> {
    vec!["YES".to_string(), "NO".to_string()]
}

fn condition_resolution_log(payouts: &[u64]) -> Value {
    json!({
        "topics": [topic_word("aa"), topic_word("11"), topic_word("22"), topic_word("33")],
        "data": abi_data(payouts),
        "transactionHash": "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "blockNumber": "0x10",
        "logIndex": "0x2"
    })
}

fn abi_data(payouts: &[u64]) -> String {
    let mut words = vec![
        word(payouts.len() as u64),
        word(64),
        word(payouts.len() as u64),
    ];
    words.extend(payouts.iter().map(|value| word(*value)));
    format!("0x{}", words.join(""))
}

fn topic_word(byte: &str) -> String {
    format!("0x{:0>64}", byte)
}

fn tx_hash(seed: &str) -> String {
    let byte = seed.bytes().fold(0u8, |acc, value| acc.wrapping_add(value));
    format!("0x{}", format!("{byte:02x}").repeat(32))
}

fn word(value: u64) -> String {
    format!("{value:064x}")
}

fn json_rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

fn post_json(agent: &ureq::Agent, url: &str, body: Value) -> Value {
    let request_path = std::env::temp_dir().join("poly_issue028_rpc_request.json");
    write_json(&request_path, &body);
    let bytes = fs::read(&request_path).unwrap();
    let mut response = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send(&bytes)
        .expect("RPC post");
    serde_json::from_slice(&response.body_mut().read_to_vec().expect("read RPC")).unwrap()
}

struct Fixture {
    vault: AsterVault,
    cx_id: CxId,
}
