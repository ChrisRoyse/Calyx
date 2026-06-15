mod hazards;

use hazards::resource::{HazardResult, run_hazards_1_5};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("calyx-hazard-soak: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    validate_args(&args)?;
    let root = fsv_root();
    fs::create_dir_all(&root).map_err(|error| format!("create FSV root: {error}"))?;
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue488 PH59 T01 synthetic FSV data\n",
    )
    .map_err(|error| format!("write cleanup tag: {error}"))?;

    let results = run_hazards_1_5(&root);
    let pass_count = results.iter().filter(|result| result.passed).count();
    let passed = pass_count == results.len();
    let artifact = json!({
        "issue": 488,
        "phase": "PH59",
        "task": "T01",
        "suite": "hazards_1_5_resource_ops",
        "passed": passed,
        "pass_count": pass_count,
        "hazard_count": results.len(),
        "source_of_truth": {
            "fsv_root": root,
            "root_artifact": root.join("ph59_hazards_1_5.json"),
            "target_artifact": repo_root().join("target/ph59_hazards_1_5.json"),
            "metrics": root.join("ph59_hazards_1_5.prom")
        },
        "results": results
    });
    write_artifacts(&root, &artifact, &results)?;
    println!(
        "PH59_HAZARDS_1_5_ROOT={} passed={}/{}",
        root.display(),
        pass_count,
        results.len()
    );
    if passed {
        Ok(())
    } else {
        Err("one or more PH59 T01 hazard probes failed".to_string())
    }
}

fn validate_args(args: &[String]) -> Result<(), String> {
    if args.is_empty() || args == ["--hazards", "1-5"] || args == ["--all-hazards"] {
        return Ok(());
    }
    Err(format!(
        "unsupported args {args:?}; this crate currently implements --hazards 1-5"
    ))
}

fn write_artifacts(root: &Path, artifact: &Value, results: &[HazardResult]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(artifact).map_err(|error| error.to_string())?;
    fs::write(root.join("ph59_hazards_1_5.json"), &bytes)
        .map_err(|error| format!("write root artifact: {error}"))?;
    let target = repo_root().join("target");
    fs::create_dir_all(&target).map_err(|error| format!("create target dir: {error}"))?;
    fs::write(target.join("ph59_hazards_1_5.json"), &bytes)
        .map_err(|error| format!("write target artifact: {error}"))?;
    fs::write(root.join("ph59_hazards_1_5.prom"), metrics_text(results))
        .map_err(|error| format!("write metrics artifact: {error}"))?;
    Ok(())
}

fn metrics_text(results: &[HazardResult]) -> String {
    let mut text = String::new();
    let pass_count = results.iter().filter(|result| result.passed).count();
    text.push_str(&format!(
        "calyx_hazard_pass_count{{suite=\"ph59_t01\"}} {pass_count}\n"
    ));
    for result in results {
        text.push_str(&format!(
            "calyx_hazard_pass{{suite=\"ph59_t01\",hazard=\"H{}\"}} {}\n",
            result.hazard_id,
            u8::from(result.passed)
        ));
        if let Some(metrics) = result.evidence.get("metrics_text").and_then(Value::as_str) {
            text.push_str(metrics);
            if !metrics.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    text
}

fn fsv_root() -> PathBuf {
    env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("calyx-ph59-t01-{}", std::process::id())))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}
