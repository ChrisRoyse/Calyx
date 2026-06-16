mod cli;
mod hazards;

use calyx_hazard_soak::soak::{SoakReport, run_integrated_soak_at, write_soak_artifacts};
use cli::{RunConfig, dmesg_oom_count};
use hazards::numerical::run_hazards_9_12;
use hazards::operational::{run_hazards_13_16, run_hazards_17_21};
use hazards::resource::{HazardResult, run_hazards_1_5};
use hazards::resource_hazards_6_8::run_hazards_6_8;
use hazards::security::run_hazards_22_25;
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
    let config = RunConfig::parse(&args)?;
    let suite = config.suite;
    let root = fsv_root(suite);
    fs::create_dir_all(&root).map_err(|error| format!("create FSV root: {error}"))?;
    fs::write(root.join("cleanup-tag.txt"), suite.cleanup_tag().as_bytes())
        .map_err(|error| format!("write cleanup tag: {error}"))?;

    let results = suite.run(&root);
    let pass_count = results.iter().filter(|result| result.passed).count();
    let passed = pass_count == results.len();
    let mut soak_report = None;
    if suite.runs_final_soak() {
        let soak_root = root.join("final_soak");
        let report = std::panic::catch_unwind(|| {
            run_integrated_soak_at(&soak_root, config.soak_ops, config.seed)
        })
        .map_err(|_| "PH59 final soak panicked".to_string())??;
        write_soak_artifacts(&root, &report)?;
        soak_report = Some(report);
    }
    let oom_count = dmesg_oom_count();
    let artifact = stage_artifact(StageArtifactInput {
        suite,
        root: &root,
        results: &results,
        hazards_passed: passed,
        pass_count,
        config,
        soak: soak_report.as_ref(),
        oom_count,
    });
    write_artifacts(&root, &artifact, &results, suite, soak_report.as_ref())?;
    if let Some(soak) = &soak_report {
        println!(
            "STAGE13 EXIT GATE: hazard_pass_count={} rss_bounded={} vram_bounded={} oscillation={}",
            pass_count, soak.rss_bounded, soak.vram_bounded, soak.soak_oscillation_detected
        );
    }
    println!(
        "{}={} passed={}/{}",
        suite.root_env_name(),
        root.display(),
        pass_count,
        results.len()
    );
    println!("PH59_{}_FSV_ROOT={}", suite.task(), root.display());
    if stage_passed(passed, soak_report.as_ref(), oom_count) {
        Ok(())
    } else if suite.runs_final_soak() {
        Err("PH59 Stage 13 exit gate failed".to_string())
    } else {
        Err(format!(
            "one or more PH59 {} hazard probes failed",
            suite.task()
        ))
    }
}

struct StageArtifactInput<'a> {
    suite: Suite,
    root: &'a Path,
    results: &'a [HazardResult],
    hazards_passed: bool,
    pass_count: usize,
    config: RunConfig,
    soak: Option<&'a SoakReport>,
    oom_count: Option<u64>,
}

fn stage_artifact(input: StageArtifactInput<'_>) -> Value {
    let base = json!({
        "issue": input.suite.issue(),
        "phase": "PH59",
        "task": input.suite.task(),
        "suite": input.suite.suite_name(),
        "passed": input.hazards_passed,
        "pass_count": input.pass_count,
        "hazard_count": input.results.len(),
        "source_of_truth": {
            "fsv_root": input.root,
            "root_artifact": input.root.join(input.suite.json_artifact()),
            "target_artifact": repo_root().join("target").join(input.suite.json_artifact()),
            "metrics": input.root.join(input.suite.prom_artifact())
        },
        "results": input.results
    });
    let Some(soak) = input.soak else {
        return base;
    };
    json!({
        "issue": input.suite.issue(),
        "phase": "PH59",
        "task": input.suite.task(),
        "suite": input.suite.suite_name(),
        "passed": stage_passed(input.hazards_passed, Some(soak), input.oom_count),
        "hazard_pass_count": input.pass_count,
        "hazard_count": input.results.len(),
        "soak_rss_bounded": soak.rss_bounded,
        "soak_vram_bounded": soak.vram_bounded,
        "soak_oscillation_detected": soak.soak_oscillation_detected,
        "dmesg_oom_count": input.oom_count,
        "seed_input": input.config.seed_input,
        "seed": input.config.seed,
        "soak_ops": input.config.soak_ops,
        "source_of_truth": {
            "fsv_root": input.root,
            "hazard_results": input.root.join(input.suite.json_artifact()),
            "final_soak": input.root.join("ph59_final_soak.json"),
            "target_hazard_results": repo_root().join("target").join(input.suite.json_artifact()),
            "target_final_soak": repo_root().join("target").join("ph59_final_soak.json"),
            "metrics": input.root.join(input.suite.prom_artifact())
        },
        "soak": soak,
        "results": input.results
    })
}

fn stage_passed(
    hazards_passed: bool,
    soak: Option<&SoakReport>,
    dmesg_oom_count: Option<u64>,
) -> bool {
    if let Some(soak) = soak {
        hazards_passed
            && soak.rss_bounded
            && soak.vram_bounded
            && !soak.soak_oscillation_detected
            && dmesg_oom_count.unwrap_or(0) == 0
    } else {
        hazards_passed
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Suite {
    Hazards1To5,
    Hazards6To8,
    Hazards9To12,
    Hazards13To16,
    Hazards17To21,
    Hazards22To25,
    Hazards1To8,
    Hazards1To12,
    AllImplemented,
    Stage13Exit,
}

impl Suite {
    pub(crate) fn from_hazards_range(range: &str) -> Result<Self, String> {
        match range {
            "1-5" => Ok(Self::Hazards1To5),
            "6-8" => Ok(Self::Hazards6To8),
            "9-12" => Ok(Self::Hazards9To12),
            "13-16" => Ok(Self::Hazards13To16),
            "17-21" => Ok(Self::Hazards17To21),
            "22-25" => Ok(Self::Hazards22To25),
            "1-8" => Ok(Self::Hazards1To8),
            "1-12" => Ok(Self::Hazards1To12),
            "1-16" | "1-21" | "1-25" => Ok(Self::AllImplemented),
            _ => Err(format!("unsupported hazard range {range:?}")),
        }
    }

    fn run(self, root: &Path) -> Vec<HazardResult> {
        match self {
            Self::Hazards1To5 => run_hazards_1_5(root),
            Self::Hazards6To8 => run_hazards_6_8(root),
            Self::Hazards9To12 => run_hazards_9_12(root),
            Self::Hazards13To16 => run_hazards_13_16(root),
            Self::Hazards17To21 => run_hazards_17_21(root),
            Self::Hazards22To25 => run_hazards_22_25(root),
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
                results.extend(run_hazards_22_25(root));
                results
            }
            Self::Stage13Exit => Self::AllImplemented.run(root),
        }
    }

    fn issue(self) -> Value {
        match self {
            Self::Hazards1To5 => json!(488),
            Self::Hazards6To8 => json!(489),
            Self::Hazards9To12 => json!(490),
            Self::Hazards13To16 => json!(491),
            Self::Hazards17To21 => json!(492),
            Self::Hazards22To25 => json!(493),
            Self::Hazards1To8 => json!([488, 489]),
            Self::Hazards1To12 => json!([488, 489, 490]),
            Self::AllImplemented => json!([488, 489, 490, 491, 492, 493]),
            Self::Stage13Exit => json!(494),
        }
    }

    fn task(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "T01",
            Self::Hazards6To8 => "T02",
            Self::Hazards9To12 => "T03",
            Self::Hazards13To16 => "T04",
            Self::Hazards17To21 => "T05",
            Self::Hazards22To25 => "T06",
            Self::Hazards1To8 => "T01_T02",
            Self::Hazards1To12 => "T01_T03",
            Self::AllImplemented => "T01_T06",
            Self::Stage13Exit => "T07",
        }
    }

    fn suite_name(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "hazards_1_5_resource_ops",
            Self::Hazards6To8 => "hazards_6_8_mvcc_vram_heap",
            Self::Hazards9To12 => "hazards_9_12_numerical_index",
            Self::Hazards13To16 => "hazards_13_16_operational_concurrency",
            Self::Hazards17To21 => "hazards_17_21_operational_resilience",
            Self::Hazards22To25 => "hazards_22_25_security_upgrade",
            Self::Hazards1To8 => "hazards_1_8_resource_ops",
            Self::Hazards1To12 => "hazards_1_12_resource_numerical_index",
            Self::AllImplemented => "hazards_1_25_resource_numerical_operational_security",
            Self::Stage13Exit => "stage13_resource_exit_gate",
        }
    }

    fn metrics_suite(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_t01",
            Self::Hazards6To8 => "ph59_t02",
            Self::Hazards9To12 => "ph59_t03",
            Self::Hazards13To16 => "ph59_t04",
            Self::Hazards17To21 => "ph59_t05",
            Self::Hazards22To25 => "ph59_t06",
            Self::Hazards1To8 => "ph59_t01_t02",
            Self::Hazards1To12 => "ph59_t01_t03",
            Self::AllImplemented => "ph59_t01_t06",
            Self::Stage13Exit => "ph59_t07",
        }
    }

    fn json_artifact(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_hazards_1_5.json",
            Self::Hazards6To8 => "ph59_hazards_6_8.json",
            Self::Hazards9To12 => "ph59_hazards_9_12.json",
            Self::Hazards13To16 => "ph59_hazards_13_16.json",
            Self::Hazards17To21 => "ph59_hazards_17_21.json",
            Self::Hazards22To25 => "ph59_hazards_22_25.json",
            Self::Hazards1To8 => "ph59_hazards_1_8.json",
            Self::Hazards1To12 => "ph59_hazards_1_12.json",
            Self::AllImplemented => "ph59_hazards_1_25.json",
            Self::Stage13Exit => "ph59_hazard_results.json",
        }
    }

    fn prom_artifact(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "ph59_hazards_1_5.prom",
            Self::Hazards6To8 => "ph59_hazards_6_8.prom",
            Self::Hazards9To12 => "ph59_hazards_9_12.prom",
            Self::Hazards13To16 => "ph59_hazards_13_16.prom",
            Self::Hazards17To21 => "ph59_hazards_17_21.prom",
            Self::Hazards22To25 => "ph59_hazards_22_25.prom",
            Self::Hazards1To8 => "ph59_hazards_1_8.prom",
            Self::Hazards1To12 => "ph59_hazards_1_12.prom",
            Self::AllImplemented => "ph59_hazards_1_25.prom",
            Self::Stage13Exit => "ph59_hazard_results.prom",
        }
    }

    fn root_env_name(self) -> &'static str {
        match self {
            Self::Hazards1To5 => "PH59_HAZARDS_1_5_ROOT",
            Self::Hazards6To8 => "PH59_HAZARDS_6_8_ROOT",
            Self::Hazards9To12 => "PH59_HAZARDS_9_12_ROOT",
            Self::Hazards13To16 => "PH59_HAZARDS_13_16_ROOT",
            Self::Hazards17To21 => "PH59_HAZARDS_17_21_ROOT",
            Self::Hazards22To25 => "PH59_HAZARDS_22_25_ROOT",
            Self::Hazards1To8 => "PH59_HAZARDS_1_8_ROOT",
            Self::Hazards1To12 => "PH59_HAZARDS_1_12_ROOT",
            Self::AllImplemented => "PH59_HAZARDS_1_25_ROOT",
            Self::Stage13Exit => "PH59_STAGE13_ROOT",
        }
    }

    fn cleanup_tag(self) -> String {
        format!(
            "issue{} PH59 {} synthetic FSV data\n",
            self.issue(),
            self.task()
        )
    }

    fn runs_final_soak(self) -> bool {
        matches!(self, Self::Stage13Exit)
    }
}

fn write_artifacts(
    root: &Path,
    artifact: &Value,
    results: &[HazardResult],
    suite: Suite,
    soak: Option<&SoakReport>,
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
        metrics_text(results, suite, soak),
    )
    .map_err(|error| format!("write metrics artifact: {error}"))?;
    Ok(())
}

fn metrics_text(results: &[HazardResult], suite: Suite, soak: Option<&SoakReport>) -> String {
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
    if let Some(soak) = soak {
        text.push_str(&format!(
            concat!(
                "calyx_stage13_hazard_pass_count{{suite=\"ph59_t07\"}} {}\n",
                "calyx_final_soak_rss_bounded{{suite=\"ph59_t07\"}} {}\n",
                "calyx_final_soak_vram_bounded{{suite=\"ph59_t07\"}} {}\n",
                "calyx_final_soak_oscillation_detected{{suite=\"ph59_t07\"}} {}\n",
                "calyx_final_soak_rss_trend_bytes_per_op{{suite=\"ph59_t07\"}} {:.6}\n",
                "calyx_final_soak_vram_max_mib{{suite=\"ph59_t07\"}} {}\n"
            ),
            pass_count,
            u8::from(soak.rss_bounded),
            u8::from(soak.vram_bounded),
            u8::from(soak.soak_oscillation_detected),
            soak.trend_bytes_per_op,
            soak.vram_max_mib
        ));
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
