//! Issue #36 - central rate-limit governor and 429 backoff.
//!
//! Source of truth: persisted known-truth feed event sequences, read back from disk.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=fsv_support visibility=private

use crate::__calyx_shared_fsv_support_rs as fsv_support;

use std::fs;
use std::path::Path;

use calyx_poly::rate_limit_governor::{
    EndpointRateLimitPolicy, RATE_LIMIT_429_BACKOFF, RATE_LIMIT_GOVERNOR_SCHEMA_VERSION,
    RATE_LIMIT_PERMITTED, RATE_LIMIT_STATUS_RECORDED, RATE_LIMIT_SUSTAINED_429,
    RATE_LIMIT_WAIT_REQUIRED, RateLimitEndpoint, RateLimitGovernor, RateLimitPolicy,
};
use fsv_support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn issue036_rate_limit_governor_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE036_FSV_ROOT",
        "poly-issue036-rate-limit-governor",
    );
    reset_dir(&root);
    let policy = issue_policy();

    let happy = happy_distinct_endpoints_are_independent(&root, &policy);
    let exhausted = edge_token_exhaustion_waits_for_refill(&root, &policy);
    let backoff = edge_429_applies_exponential_backoff_and_resets(&root, &policy);
    let sustained = edge_sustained_429_fails_loud(&root, &policy);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 36,
        "schema_version": RATE_LIMIT_GOVERNOR_SCHEMA_VERSION,
        "proof_claim": "Poly has a central per-endpoint token-bucket governor for read-only feed HTTP capture, applies exponential 429 backoff, resets after a healthy response, and fails loudly after sustained 429s.",
        "minimum_sufficient_proof_corpus": {
            "event_sequences": 4,
            "why_this_is_sufficient": "One happy independent-endpoint sequence proves per-endpoint state isolation; one same-endpoint burst proves token-bucket wait; one 429 sequence proves exponential backoff and reset; one sustained-429 sequence proves fail-loud behavior.",
            "why_larger_is_wasteful": "More feed rows would only repeat the same state transitions and would not prove additional #36 behavior."
        },
        "source_of_truth": {
            "kind": "persisted known-truth feed event JSON",
            "readback": "each event sequence is written, read back, decoded, and compared before this report is accepted"
        },
        "cases": [happy, exhausted, backoff, sustained],
        "physical_file_count_before_report": files.len(),
        "physical_files_before_report": files,
        "passed": true
    });
    let report_path = root.join("issue036_rate_limit_governor_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).expect("read FSV report"))
        .expect("decode FSV report");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

fn happy_distinct_endpoints_are_independent(root: &Path, policy: &RateLimitPolicy) -> Value {
    let gamma = endpoint("gamma", "markets", "GET");
    let clob = endpoint("clob", "book", "GET");
    let mut governor = governor(policy);
    let gamma_decision = governor.reserve(&gamma, 1_000).unwrap();
    let clob_decision = governor.reserve(&clob, 1_000).unwrap();
    let gamma_response = governor
        .observe_response(&gamma, Some(200), 1_000, None)
        .unwrap();
    let clob_response = governor
        .observe_response(&clob, Some(200), 1_000, None)
        .unwrap();

    assert_eq!(gamma_decision.status_code, RATE_LIMIT_PERMITTED);
    assert_eq!(clob_decision.status_code, RATE_LIMIT_PERMITTED);
    assert_eq!(gamma_decision.wait_ms, 0);
    assert_eq!(clob_decision.wait_ms, 0);
    assert_eq!(gamma_response.decision_code, RATE_LIMIT_STATUS_RECORDED);
    assert_eq!(clob_response.decision_code, RATE_LIMIT_STATUS_RECORDED);
    assert_eq!(governor.states.len(), 2);

    persist_case(
        root,
        "happy-distinct-endpoints",
        json!({
            "case": "happy_distinct_endpoints_are_independent",
            "events": {
                "gamma_decision": gamma_decision,
                "clob_decision": clob_decision,
                "gamma_response": gamma_response,
                "clob_response": clob_response
            },
            "assertions": {
                "both_permitted_without_cross_endpoint_wait": true,
                "endpoint_state_count": governor.states.len()
            }
        }),
    )
}

fn edge_token_exhaustion_waits_for_refill(root: &Path, policy: &RateLimitPolicy) -> Value {
    let gamma = endpoint("gamma", "markets", "GET");
    let mut governor = governor(policy);
    let first = governor.reserve(&gamma, 2_000).unwrap();
    let second = governor.reserve(&gamma, 2_000).unwrap();
    let third = governor.reserve(&gamma, 2_000).unwrap();

    assert_eq!(first.status_code, RATE_LIMIT_PERMITTED);
    assert_eq!(second.status_code, RATE_LIMIT_PERMITTED);
    assert_eq!(third.status_code, RATE_LIMIT_WAIT_REQUIRED);
    assert_eq!(third.wait_ms, 100);
    assert_eq!(third.permitted_at_ms, 2_100);

    persist_case(
        root,
        "edge-token-exhaustion",
        json!({
            "case": "edge_token_exhaustion_waits_for_refill",
            "events": [first, second, third],
            "assertions": {
                "third_request_waited_ms": 100,
                "third_permitted_at_ms": 2100
            }
        }),
    )
}

fn edge_429_applies_exponential_backoff_and_resets(root: &Path, policy: &RateLimitPolicy) -> Value {
    let data = endpoint("data-api", "trades", "GET");
    let mut governor = governor(policy);
    let first = governor.reserve(&data, 3_000).unwrap();
    let first_429 = governor
        .observe_response(&data, Some(429), 3_000, None)
        .unwrap();
    let retry_one = governor.reserve(&data, 3_000).unwrap();
    let second_429 = governor
        .observe_response(&data, Some(429), 3_250, None)
        .unwrap();
    let retry_two = governor.reserve(&data, 3_250).unwrap();
    let recovery = governor
        .observe_response(&data, Some(200), 3_750, None)
        .unwrap();

    assert_eq!(first.status_code, RATE_LIMIT_PERMITTED);
    assert_eq!(first_429.decision_code, RATE_LIMIT_429_BACKOFF);
    assert_eq!(first_429.backoff_ms, 250);
    assert_eq!(retry_one.wait_ms, 250);
    assert_eq!(second_429.backoff_ms, 500);
    assert_eq!(retry_two.wait_ms, 500);
    assert_eq!(recovery.consecutive_429s, 0);

    persist_case(
        root,
        "edge-429-backoff-reset",
        json!({
            "case": "edge_429_applies_exponential_backoff_and_resets",
            "events": {
                "first": first,
                "first_429": first_429,
                "retry_one": retry_one,
                "second_429": second_429,
                "retry_two": retry_two,
                "recovery": recovery
            },
            "assertions": {
                "backoff_ms": [250, 500],
                "recovery_resets_consecutive_429s": true
            }
        }),
    )
}

fn edge_sustained_429_fails_loud(root: &Path, policy: &RateLimitPolicy) -> Value {
    let polygon = endpoint("polygon-rpc", "eth_getLogs", "POST");
    let mut governor = governor(policy);
    let first = governor.reserve(&polygon, 4_000).unwrap();
    let first_429 = governor
        .observe_response(&polygon, Some(429), 4_000, None)
        .unwrap();
    let second = governor.reserve(&polygon, 4_250).unwrap();
    let second_429 = governor
        .observe_response(&polygon, Some(429), 4_250, None)
        .unwrap();
    let third = governor.reserve(&polygon, 4_750).unwrap();
    let third_429 = governor
        .observe_response(&polygon, Some(429), 4_750, None)
        .unwrap();

    assert!(!first_429.fail_loud);
    assert!(!second_429.fail_loud);
    assert!(third_429.fail_loud);
    assert_eq!(third_429.decision_code, RATE_LIMIT_SUSTAINED_429);
    assert_eq!(third_429.consecutive_429s, 3);

    persist_case(
        root,
        "edge-sustained-429",
        json!({
            "case": "edge_sustained_429_fails_loud",
            "events": {
                "first": first,
                "first_429": first_429,
                "second": second,
                "second_429": second_429,
                "third": third,
                "third_429": third_429
            },
            "assertions": {
                "fail_loud_code": RATE_LIMIT_SUSTAINED_429,
                "failed_after_consecutive_429s": 3
            }
        }),
    )
}

fn issue_policy() -> RateLimitPolicy {
    RateLimitPolicy {
        default: EndpointRateLimitPolicy {
            capacity: 2,
            refill_interval_ms: 100,
            max_429_retries: 2,
            backoff_initial_ms: 250,
            backoff_multiplier: 2,
            backoff_max_ms: 1_000,
        },
        endpoints: Default::default(),
    }
}

fn governor(policy: &RateLimitPolicy) -> RateLimitGovernor {
    RateLimitGovernor::new(policy.clone()).expect("issue #36 policy validates")
}

fn endpoint(source: &str, endpoint: &str, method: &str) -> RateLimitEndpoint {
    RateLimitEndpoint::new(source, endpoint, method)
}

fn persist_case(root: &Path, name: &str, value: Value) -> Value {
    let path = root.join(name).join("rate-limit-events.json");
    write_json(&path, &value);
    let bytes = fs::read(&path).expect("read persisted rate-limit case");
    let readback: Value = serde_json::from_slice(&bytes).expect("decode persisted rate-limit case");
    assert_eq!(readback, value);
    json!({
        "name": name,
        "path": path.display().to_string(),
        "sha256": sha256_hex(&bytes),
        "readback_equal": true,
        "case": readback["case"].clone(),
        "assertions": readback["assertions"].clone()
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:X}", hasher.finalize())
}
