mod hazards;

use hazards::numerical::run_hazards_9_12;
use hazards::operational::{run_hazards_13_16, run_hazards_17_21};
use hazards::resource::{HazardResult, run_hazards_1_5};
use hazards::resource_hazards_6_8::run_hazards_6_8;
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
    let suite = Suite::parse(&args)?;
    let root = fsv_root(suite);
    fs::create_dir_all(&root).map_err(|error| format!("create FSV root: {error}"))?;
    fs::write(root.join("cleanup-tag.txt"), suite.cleanup_tag().as_bytes())
        .map_err(|error| format!("write cleanup tag: {error}"))?;

    let results = suite.run(&root);
    let pass_count = results.iter().filter(|result| result.passed).count();
    let passed = pass_count == results.len();
    let artifact = json!({
        "issue": suite.issue(),
        "phase": "PH59",
        "task": suite.task(),
        "suite": suite.suite_name(),
        "passed": passed,
        "pass_count": pass_count,
        "hazard_count": results.len(),
        "source_of_truth": {
            "fsv_root": root,
            "root_artifact": root.join(suite.json_artifact()),
            "target_artifact": repo_root().join("target").join(suite.json_artifact()),
            "metrics": root.join(suite.prom_artifact())
        },
        "results": results
    });
    write_artifacts(&root, &artifact, &results, suite)?;
    println!(
        "{}={} passed={}/{}",
        suite.root_env_name(),
        root.display(),
        pass_count,
        results.len()
    );
    println!("PH59_{}_FSV_ROOT={}", suite.task(), root.display());
    if passed {
        Ok(())
    } else {
        Err(format!(
            "one or more PH59 {} hazard probes failed",
            suite.task()
        ))
    }
}

#[derive(Clone, Copy)]
enum Suite {
    Hazards1To5,
    Hazards6To8,
    Hazards9To12,
    Hazards13To16,
    Hazards17To21,
    Hazards1To8,
    Hazards1To12,
    AllImplemented,
}

impl Suite {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [] => Ok(Self::Hazards1To5),
            [flag, range] if flag == "--hazards" && range == "1-5" => Ok(Self::Hazards1To5),
            [flag, range] if flag == "--hazards" && range == "6-8" => Ok(Self::Hazards6To8),
            [flag, range] if flag == "--hazards" && range == "9-12" => Ok(Self::Hazards9To12),
            [flag, range] if flag == "--hazards" && range == "13-16" => Ok(Self::Hazards13To16),
            [flag, range] if flag == "--hazards" && range == "17-21" => Ok(Self::Hazards17To21),
            [flag, range] if flag == "--hazards" && range == "1-8" => Ok(Self::Hazards1To8),
            [flag, range] if flag == "--hazards" && range == "1-12" => Ok(Self::Hazards1To12),
            [flag, range] if flag == "--hazards" && (range == "1-16" || range == "1-21") => {
                Ok(Self::AllImplemented)
            }
            [flag] if flag == "--all-hazards" => Ok(Self::AllImplemented),
            _ => Err(format!(
                "unsupported args {args:?}; supported: --hazards 1-5, --hazards 6-8, --hazards 9-12, --hazards 13-16, --hazards 17-21, --hazards 1-8, --hazards 1-12, --hazards 1-16, --hazards 1-21"
            )),
        }
    }

    fn run(self, root: &Path) -> Vec<HazardResult> {
        match self {
            Self::Hazards1To5 => run_hazards_1_5(root),
            Self::Hazards6To8 => run_hazards_6_8(root),
            Self::Hazards9To12 => run_hazards_9_12(root),
            Self::Hazards13To16 => run_hazards_13_16(root),
            Self::Hazards17To21 => run_hazards_17_21(root),
            Self::Hazards1To8 => {
                let mut results = run_hazards_1_5(root);
                results.extend(run_hazards_6_8(root));
                results
            }
            Self::Hazards1To12 => {
                let mut results = run_hazards_1_5(root);
                results.extend(run_hazards_6_8(root));
                results.extend(run_hazards_9_12(root));
                results
            }
            Self::AllImplemented => {
                let mut results = run_hazards_1_5(root);
                results.extend(run_hazards_6_8(root));
                results.extend(run_hazards_9_12(root));
                results.extend(run_hazards_13_16(root));
                results.extend(run_hazards_17_21(root));
                results
            }
        }
    }

    fn issue(self) -> Value {
        match self {
            Self::Hazards1To5 => json!(488),
            Self::Hazards6To8 => json!(489),
            Self::Hazards9To12 => json!(490),
            Self::Hazards13To16 => json!(491),
            Self::Hazards17To21 => json!(492),
            Self::Hazards1To8 => json!([488, 489]),
            Self::Hazards1To12 => json!([488, 489, 490]),
            Self::AllImplemented => json!([488, 489, 490, 491, 492]),
        }
    }

    fn task(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "T01",
            Self::Hazards6To8 => "T02",
            Self::Hazards9To12 => "T03",
            Self::Hazards13To16 => "T04",
            Self::Hazards17To21 => "T05",
            Self::Hazards1To8 => "T01_T02",
            Self::Hazards1To12 => "T01_T03",
            Self::AllImplemented => "T01_T05",
        }
    }

    fn suite_name(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "hazards_1_5_resource_ops",
            Self::Hazards6To8 => "hazards_6_8_mvcc_vram_heap",
            Self::Hazards9To12 => "hazards_9_12_numerical_index",
            Self::Hazards13To16 => "hazards_13_16_operational_concurrency",
            Self::Hazards17To21 => "hazards_17_21_operational_resilience",
            Self::Hazards1To8 => "hazards_1_8_resource_ops",
            Self::Hazards1To12 => "hazards_1_12_resource_numerical_index",
            Self::AllImplemented => "hazards_1_21_resource_numerical_operational",
        }
    }

    fn metrics_suite(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_t01",
            Self::Hazards6To8 => "ph59_t02",
            Self::Hazards9To12 => "ph59_t03",
            Self::Hazards13To16 => "ph59_t04",
            Self::Hazards17To21 => "ph59_t05",
            Self::Hazards1To8 => "ph59_t01_t02",
            Self::Hazards1To12 => "ph59_t01_t03",
            Self::AllImplemented => "ph59_t01_t05",
        }
    }

    fn json_artifact(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_hazards_1_5.json",
            Self::Hazards6To8 => "ph59_hazards_6_8.json",
            Self::Hazards9To12 => "ph59_hazards_9_12.json",
            Self::Hazards13To16 => "ph59_hazards_13_16.json",
            Self::Hazards17To21 => "ph59_hazards_17_21.json",
            Self::Hazards1To8 => "ph59_hazards_1_8.json",
            Self::Hazards1To12 => "ph59_hazards_1_12.json",
            Self::AllImplemented => "ph59_hazards_1_21.json",
        }
    }

    fn prom_artifact(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_hazards_1_5.prom",
            Self::Hazards6To8 => "ph59_hazards_6_8.prom",
            Self::Hazards9To12 => "ph59_hazards_9_12.prom",
            Self::Hazards13To16 => "ph59_hazards_13_16.prom",
            Self::Hazards17To21 => "ph59_hazards_17_21.prom",
            Self::Hazards1To8 => "ph59_hazards_1_8.prom",
            Self::Hazards1To12 => "ph59_hazards_1_12.prom",
            Self::AllImplemented => "ph59_hazards_1_21.prom",
        }
    }

    fn root_env_name(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "PH59_HAZARDS_1_5_ROOT",
            Self::Hazards6To8 => "PH59_HAZARDS_6_8_ROOT",
            Self::Hazards9To12 => "PH59_HAZARDS_9_12_ROOT",
            Self::Hazards13To16 => "PH59_HAZARDS_13_16_ROOT",
            Self::Hazards17To21 => "PH59_HAZARDS_17_21_ROOT",
            Self::Hazards1To8 => "PH59_HAZARDS_1_8_ROOT",
            Self::Hazards1To12 => "PH59_HAZARDS_1_12_ROOT",
            Self::AllImplemented => "PH59_HAZARDS_1_21_ROOT",
        }
    }

    fn cleanup_tag(self) -> String {
        format!(
            "issue{} PH59 {} synthetic FSV data\n",
            self.issue(),
            self.task()
        )
    }
}

fn write_artifacts(
    root: &Path,
    artifact: &Value,
    results: &[HazardResult],
    suite: Suite,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(artifact).map_err(|error| error.to_string())?;
    fs::write(root.join(suite.json_artifact()), &bytes)
        .map_err(|error| format!("write root artifact: {error}"))?;
    let target = repo_root().join("target");
    fs::create_dir_all(&target).map_err(|error| format!("create target dir: {error}"))?;
    fs::write(target.join(suite.json_artifact()), &bytes)
        .map_err(|error| format!("write target artifact: {error}"))?;
    fs::write(
        root.join(suite.prom_artifact()),
        metrics_text(results, suite),
    )
    .map_err(|error| format!("write metrics artifact: {error}"))?;
    Ok(())
}

fn metrics_text(results: &[HazardResult], suite: Suite) -> String {
    let mut text = String::new();
    let pass_count = results.iter().filter(|result| result.passed).count();
    text.push_str(&format!(
        "calyx_hazard_pass_count{{suite=\"{}\"}} {pass_count}\n",
        suite.metrics_suite()
    ));
    for result in results {
        text.push_str(&format!(
            "calyx_hazard_pass{{suite=\"{}\",hazard=\"H{}\"}} {}\n",
            suite.metrics_suite(),
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

fn fsv_root(suite: Suite) -> PathBuf {
    env::var_os(suite.root_env_name())
        .or_else(|| env::var_os("CALYX_FSV_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::temp_dir().join(format!(
                "calyx-ph59-{}-{}",
                suite.task().to_ascii_lowercase(),
                std::process::id()
            ))
        })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}
