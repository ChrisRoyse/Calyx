use std::fs;
use std::path::Path;

use calyx_core::CxId;
use calyx_lodestar::{RecallQuery, RecallTestParams};
use calyx_poly::Domain;
use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use calyx_poly::error::PolyError;
use calyx_poly::kernel_recall::{
    DomainKernelRecallInput, KernelRecallVerificationReport, POLY_KERNEL_RECALL_GATE_PASSED,
    read_kernel_recall_report, verify_kernel_recall_per_domain, write_kernel_recall_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue136_kernel_recall_gate_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE136_FSV_ROOT", "poly-issue136-recall");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy-two-domains",
        EdgeInputClass::HappyPath,
        POLY_KERNEL_RECALL_GATE_PASSED,
        true,
        happy_inputs(),
    );
    let empty = run_case(
        &root,
        "empty-domain-set",
        EdgeInputClass::EmptyInput,
        "CALYX_POLY_KERNEL_RECALL_NO_DOMAINS",
        false,
        Vec::new(),
    );
    let max = run_case(
        &root,
        "max-threshold-below-gate",
        EdgeInputClass::MaxLimit,
        "CALYX_KERNEL_RECALL_BELOW_GATE",
        false,
        max_threshold_below_gate_inputs(),
    );
    let invalid = run_case(
        &root,
        "invalid-lowered-policy-min",
        EdgeInputClass::InvalidInput,
        "CALYX_POLY_KERNEL_RECALL_MIN_BELOW_POLICY",
        false,
        lowered_policy_min_inputs(),
    );
    let duplicate = run_case(
        &root,
        "duplicate-domain",
        EdgeInputClass::InvalidInput,
        "CALYX_POLY_KERNEL_RECALL_DUPLICATE_DOMAIN",
        false,
        duplicate_domain_inputs(),
    );
    let nondeterministic = run_case(
        &root,
        "nondeterministic-seed",
        EdgeInputClass::InvalidInput,
        "CALYX_POLY_KERNEL_RECALL_NONDETERMINISTIC_SEED",
        false,
        nondeterministic_seed_inputs(),
    );

    for outcome in [
        &happy,
        &empty,
        &max,
        &invalid,
        &duplicate,
        &nondeterministic,
    ] {
        assert!(
            outcome.ok,
            "{} expected {} got {}",
            outcome.name, outcome.expected_code, outcome.observed_code
        );
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 136,
        "source_of_truth": [
            "per-case before.json files read from disk",
            "per-case after.json files read from disk",
            "happy-path kernel-recall-report.json readback",
            "edge-case-outcome.json readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "empty_input": empty,
            "max_limit": max,
            "invalid_input": invalid,
            "duplicate_domain": duplicate,
            "nondeterministic_seed": nondeterministic
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue136_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue136_fsv_root={}", root.display());
    }
}

fn run_case(
    root: &Path,
    name: &str,
    input_class: EdgeInputClass,
    expected_code: &str,
    expect_state_change: bool,
    inputs: Vec<DomainKernelRecallInput>,
) -> calyx_poly::edge_audit::EdgeCaseOutcome {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let report_path = case_dir.join("kernel-recall-report.json");

    drive_edge_case(
        EdgeCaseSpec {
            case_dir: &case_dir,
            name,
            input_class,
            expected_code,
            expect_state_change,
        },
        EdgeCaseDriver {
            read_before: || state(&case_dir, &report_path, &inputs),
            execute: || {
                let result = verify_kernel_recall_per_domain(&inputs);
                if let Ok(report) = &result {
                    write_kernel_recall_report(&report_path, report).expect("write recall report");
                    let readback =
                        read_kernel_recall_report(&report_path).expect("read recall report");
                    assert_eq!(&readback, report);
                }
                result
            },
            read_after: || state(&case_dir, &report_path, &inputs),
            decision_record: |result| decision_record(result, &report_path),
        },
    )
    .expect("drive edge case")
}

fn decision_record(
    result: calyx_poly::Result<KernelRecallVerificationReport>,
    report_path: &Path,
) -> (String, Value) {
    match result {
        Ok(report) => (
            POLY_KERNEL_RECALL_GATE_PASSED.to_string(),
            json!({
                "code": POLY_KERNEL_RECALL_GATE_PASSED,
                "report_path": report_path.display().to_string(),
                "report": report_summary(&report)
            }),
        ),
        Err(error) => {
            let code = error_code(&error);
            (
                code.clone(),
                json!({
                    "code": code,
                    "error": error.to_string()
                }),
            )
        }
    }
}

fn state(case_dir: &Path, report_path: &Path, inputs: &[DomainKernelRecallInput]) -> Value {
    let bytes = fs::read(report_path).ok();
    json!({
        "case_dir": file_state(case_dir),
        "input": input_state(inputs),
        "report": {
            "path": report_path.display().to_string(),
            "exists": report_path.exists(),
            "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "readback": bytes.as_ref().and_then(|bytes| {
                serde_json::from_slice::<KernelRecallVerificationReport>(bytes)
                    .ok()
                    .map(|report| report_summary(&report))
            })
        }
    })
}

fn file_state(path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "exists": path.exists()
    })
}

fn input_state(inputs: &[DomainKernelRecallInput]) -> Value {
    json!({
        "domain_count": inputs.len(),
        "domains": inputs.iter().map(domain_input_state).collect::<Vec<_>>()
    })
}

fn domain_input_state(input: &DomainKernelRecallInput) -> Value {
    json!({
        "domain": input.domain.slug(),
        "full_rows": input.full_rows.len(),
        "kernel_members": input.kernel_members.len(),
        "held_out_fraction": input.params.held_out_fraction,
        "top_k": input.params.top_k,
        "min_recall_ratio": input.params.min_recall_ratio,
        "rng_seed": input.params.rng_seed
    })
}

fn report_summary(report: &KernelRecallVerificationReport) -> Value {
    json!({
        "schema_version": report.schema_version,
        "domain_count": report.domain_count,
        "policy_min_recall_ratio": report.policy_min_recall_ratio,
        "domains": report.domains.iter().map(|domain| {
            json!({
                "domain": domain.domain.slug(),
                "corpus_len": domain.corpus_len,
                "kernel_members": domain.kernel_members,
                "ratio": domain.recall.ratio,
                "n_queries_tested": domain.recall.n_queries_tested,
                "gate_passed": domain.gate_passed
            })
        }).collect::<Vec<_>>()
    })
}

fn error_code(error: &PolyError) -> String {
    match error {
        PolyError::KernelRecall { code, .. } | PolyError::Calyx { code, .. } => code.clone(),
        other => format!("UNEXPECTED_POLY_ERROR:{other}"),
    }
}

fn happy_inputs() -> Vec<DomainKernelRecallInput> {
    vec![
        domain_input(
            Domain::Crypto,
            "happy-crypto",
            12,
            12,
            params(0.50, 3, 0.95),
        ),
        domain_input(
            Domain::Politics,
            "happy-politics",
            10,
            10,
            params(0.50, 3, 0.95),
        ),
    ]
}

fn max_threshold_below_gate_inputs() -> Vec<DomainKernelRecallInput> {
    vec![domain_input(
        Domain::Crypto,
        "max-threshold-below",
        10,
        1,
        params(1.0, 2, 1.0),
    )]
}

fn lowered_policy_min_inputs() -> Vec<DomainKernelRecallInput> {
    vec![domain_input(
        Domain::Sports,
        "lowered-policy",
        8,
        8,
        params(0.50, 2, 0.50),
    )]
}

fn duplicate_domain_inputs() -> Vec<DomainKernelRecallInput> {
    vec![
        domain_input(Domain::Crypto, "duplicate-a", 8, 8, params(0.50, 2, 0.95)),
        domain_input(Domain::Crypto, "duplicate-b", 8, 8, params(0.50, 2, 0.95)),
    ]
}

fn nondeterministic_seed_inputs() -> Vec<DomainKernelRecallInput> {
    // Valid in every respect except the seed: rng_seed == 0 is the Lodestar wall-clock sentinel,
    // which makes the gate verdict time-dependent and non-reproducible and must fail closed (#183).
    vec![domain_input(
        Domain::Crypto,
        "nondeterministic-seed",
        10,
        10,
        RecallTestParams {
            held_out_fraction: 0.50,
            top_k: 3,
            rng_seed: 0,
            min_recall_ratio: 0.95,
        },
    )]
}

fn domain_input(
    domain: Domain,
    label: &str,
    row_count: usize,
    kernel_member_count: usize,
    params: RecallTestParams,
) -> DomainKernelRecallInput {
    let full_rows = (0..row_count)
        .map(|ordinal| RecallQuery {
            cx_id: cx(label, ordinal),
            vector: vector_for(ordinal),
        })
        .collect::<Vec<_>>();
    let kernel_members = full_rows
        .iter()
        .take(kernel_member_count)
        .map(|row| row.cx_id)
        .collect();

    DomainKernelRecallInput {
        domain,
        full_rows,
        kernel_members,
        params,
    }
}

fn params(held_out_fraction: f32, top_k: usize, min_recall_ratio: f32) -> RecallTestParams {
    RecallTestParams {
        held_out_fraction,
        top_k,
        rng_seed: 136,
        min_recall_ratio,
    }
}

fn cx(label: &str, ordinal: usize) -> CxId {
    CxId::from_input(
        format!("poly:issue136:{label}:{ordinal}").as_bytes(),
        136,
        b"poly-issue136-kernel-recall",
    )
}

fn vector_for(ordinal: usize) -> Vec<f32> {
    let x = ordinal as f32 + 1.0;
    vec![x, x * 0.5 + 1.0, (ordinal % 3) as f32 + 0.25, 1.0]
}
