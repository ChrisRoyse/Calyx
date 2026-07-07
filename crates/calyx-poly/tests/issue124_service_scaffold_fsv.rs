use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_SERVICE_SCAFFOLD_FORBIDDEN, ERR_SERVICE_SCAFFOLD_MALFORMED,
    ERR_SERVICE_SCAFFOLD_MISSING_CONFIG, LocalOnlyPolicy, PolyAction,
    SERVICE_SCAFFOLD_MANIFEST_FILE, SERVICE_SCHEDULER_STATE_FILE, ServiceScaffoldManifest,
    ServiceScaffoldRequest, default_local_service_configs, read_service_scaffold_manifest,
    run_service_scaffold,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const EMITTED_AT: u64 = 1_785_400_000_000;
const HEALTHY_CODE: &str = "CALYX_POLY_SERVICE_SCAFFOLD_CONFIGURED";

#[test]
fn issue124_service_scaffold_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE124_FSV_ROOT", "poly-issue124-service-scaffold");
    reset_dir(&root);

    let happy = run_case(&root, "happy", default_local_service_configs, HEALTHY_CODE);
    let missing = run_case(
        &root,
        "missing-config",
        Vec::new,
        ERR_SERVICE_SCAFFOLD_MISSING_CONFIG,
    );
    let forbidden = run_case(
        &root,
        "forbidden-action",
        forbidden_action_config,
        ERR_SERVICE_SCAFFOLD_FORBIDDEN,
    );
    let malformed = run_case(
        &root,
        "malformed-duplicate",
        duplicate_config,
        ERR_SERVICE_SCAFFOLD_MALFORMED,
    );

    for case in [&happy, &missing, &forbidden, &malformed] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 124,
        "proof_claim": "Poly has a local-only service/scheduler scaffold that writes and reads back state for ingestor, association, forecaster, admission, scorer, and scheduler, while refusing missing config, forbidden trading/executor action requests, and malformed duplicate service config without success-looking state.",
        "minimum_sufficient_proof_corpus": {
            "selected": "one complete six-service scaffold plus three edge configs: empty, forbidden action, and duplicate service",
            "why_smaller_would_not_prove": "the happy path needs all six required local services to prove #124's full service surface; the three edges are distinct failure classes named in the issue",
            "why_larger_would_be_wasteful": "extra services or repeated configs would exercise the same validation, policy, artifact write, and readback paths without proving another #124 invariant"
        },
        "source_of_truth": [
            "per-service services/*_state.json files read from disk",
            "service_scheduler_state.json read from disk",
            "service_scaffold_manifest.json read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "missing_config": missing,
            "forbidden_action": forbidden,
            "malformed_duplicate": malformed
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue124_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue124_fsv_root={}", root.display());
    }
}

fn run_case(
    root: &Path,
    name: &str,
    builder: fn() -> Vec<calyx_poly::LocalServiceConfig>,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let manifest_path = case_dir.join(SERVICE_SCAFFOLD_MANIFEST_FILE);
    let scheduler_path = case_dir.join(SERVICE_SCHEDULER_STATE_FILE);
    let before = state(&case_dir, &manifest_path, &scheduler_path);
    write_json(&case_dir.join("before.json"), &before);

    let request = ServiceScaffoldRequest {
        out_dir: &case_dir,
        emitted_at_millis: EMITTED_AT,
        policy: LocalOnlyPolicy::default(),
        services: builder(),
    };
    let observed = match run_service_scaffold(&request) {
        Ok(run) => {
            let readback = read_service_scaffold_manifest(&run.manifest_path)
                .expect("read service scaffold manifest");
            assert_eq!(readback, run.manifest, "manifest must round-trip exactly");
            assert_eq!(readback.service_count, 6);
            assert_eq!(readback.scheduler.jobs.len(), 5);
            assert!(readback.no_executor_service);
            for path in &run.service_paths {
                assert!(
                    path.exists(),
                    "service state path must exist: {}",
                    path.display()
                );
            }
            HEALTHY_CODE.to_string()
        }
        Err(err) => err.code().to_string(),
    };

    let after = state(&case_dir, &manifest_path, &scheduler_path);
    write_json(&case_dir.join("after.json"), &after);
    let outcome = json!({
        "case": name,
        "expected_code": expected_code,
        "observed_code": observed,
        "ok": observed == expected_code,
        "before": before,
        "after": after
    });
    write_json(&case_dir.join("outcome.json"), &outcome);
    outcome
}

fn forbidden_action_config() -> Vec<calyx_poly::LocalServiceConfig> {
    let mut configs = default_local_service_configs();
    let scheduler = configs
        .iter_mut()
        .find(|config| matches!(config.kind, calyx_poly::LocalServiceKind::Scheduler))
        .expect("scheduler config");
    scheduler.actions = vec![PolyAction::StartLiveExecutor];
    configs
}

fn duplicate_config() -> Vec<calyx_poly::LocalServiceConfig> {
    let mut configs = default_local_service_configs();
    configs.push(configs[0].clone());
    configs
}

fn state(case_dir: &Path, manifest_path: &Path, scheduler_path: &Path) -> Value {
    let service_dir = case_dir.join("services");
    let mut service_files = Vec::new();
    collect_service_files(&service_dir, &mut service_files);
    json!({
        "case_dir_exists": case_dir.exists(),
        "manifest": manifest_state(manifest_path),
        "scheduler": file_state(scheduler_path),
        "service_files": service_files
    })
}

fn collect_service_files(dir: &Path, out: &mut Vec<Value>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read service dir") {
        let path = entry.expect("service entry").path();
        if path.is_file() {
            out.push(file_state(&path));
        }
    }
    out.sort_by_key(|value| value["path"].as_str().unwrap_or_default().to_string());
}

fn manifest_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let readback = bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice::<ServiceScaffoldManifest>(b).ok());
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(|manifest| json!({
            "service_count": manifest.service_count,
            "scheduler_jobs": manifest.scheduler.jobs.len(),
            "no_executor_service": manifest.no_executor_service,
            "services": manifest.services.iter().map(|service| {
                json!({
                    "service": service.service.slug(),
                    "status": service.status,
                    "actions": service.actions
                })
            }).collect::<Vec<_>>()
        }))
    })
}

fn file_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes()))
    })
}
