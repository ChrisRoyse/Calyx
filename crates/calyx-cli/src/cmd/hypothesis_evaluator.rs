//! `calyx hypothesis-evaluator-driver` -- versioned evaluator-run generation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_core::CalyxError;
use calyx_lodestar::{
    EvaluatorRun, HypothesisEvaluationInput, HypothesisEvaluationParams,
    HypothesisEvaluationReport, aggregate_hypothesis_evaluations,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::discovery_run_preflight::{
    DiscoveryRunPreflightArgs, RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG, preflight_input_bytes,
};
use super::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod http;

const DRIVER_SCHEMA_VERSION: u32 = 1;
const PROMPT_SET_ID: &str = "biomed_hypothesis_evaluator_v1";
const SYSTEM_PROMPT: &str = "You are a cautious biomedical hypothesis reviewer. Score only the \
provided evidence as a research lead, never as clinical advice. Return JSON only.";
const CLINICAL_PROMPT: &str = "Evaluate plausibility, novelty, testability, and falsifiability for \
the hypothesis using the cited evidence.";
const FALSIFICATION_PROMPT: &str = "Focus on how this hypothesis could be falsified, what evidence \
would weaken it, and whether the cited evidence is sufficient.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HypothesisEvaluatorArgs {
    pub input: PathBuf,
    pub out: PathBuf,
    pub endpoint: String,
    pub auth_env: Option<String>,
    pub model: String,
    pub temperatures: Vec<u16>,
    pub timeout_ms: u64,
    pub preflight: DiscoveryRunPreflightArgs,
}

impl Default for HypothesisEvaluatorArgs {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            out: PathBuf::new(),
            endpoint: String::new(),
            auth_env: None,
            model: String::new(),
            temperatures: vec![20, 80],
            timeout_ms: 30_000,
            preflight: DiscoveryRunPreflightArgs::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct InputFile {
    #[serde(default)]
    schema_version: Option<u32>,
    inputs: Vec<HypothesisEvaluationInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct DriverArtifact {
    schema_version: u32,
    model: String,
    endpoint: String,
    prompt_set_id: String,
    prompt_set_sha256: String,
    temperatures_x100: Vec<u16>,
    source_input: String,
    source_input_sha256: String,
    inputs: Vec<HypothesisEvaluationInput>,
    report: HypothesisEvaluationReport,
}

#[derive(Clone, Debug, Deserialize)]
struct EvaluatorJson {
    plausible_score: f32,
    novelty_score: f32,
    testability_score: f32,
    falsifiability_score: f32,
    justification: String,
    falsification_test: String,
    cited_evidence_ids: Vec<String>,
}

struct PersistedDriver {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_input_count: usize,
    readback_evaluation_count: usize,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != "hypothesis-evaluator-driver" {
        return None;
    }
    if matches!(rest, [flag] if flag == "--help" || flag == "-h") {
        return Some(crate::usage::print_command_usage(command));
    }
    Some(parse_hypothesis_evaluator(rest).and_then(run_hypothesis_evaluator))
}

fn run_hypothesis_evaluator(args: HypothesisEvaluatorArgs) -> CliResult {
    let input_bytes = fs::read(&args.input)
        .map_err(|error| CliError::io(format!("read --input {}: {error}", args.input.display())))?;
    let preflight = preflight_input_bytes(&args.preflight, &input_bytes)?;
    let mut input_file: InputFile = serde_json::from_slice(&input_bytes).map_err(|error| {
        CliError::runtime(format!("parse --input {}: {error}", args.input.display()))
    })?;
    if input_file.inputs.is_empty() {
        return Err(CliError::usage(format!(
            "--input {} did not contain any hypothesis inputs",
            args.input.display()
        )));
    }
    let auth = http::EvaluatorAuth::for_endpoint(&args.endpoint, args.auth_env.as_deref())?;
    let prompt_set_hash = prompt_set_sha256();
    let templates = prompt_templates();
    for input in &mut input_file.inputs {
        let mut runs = Vec::new();
        for template in &templates {
            for temperature in &args.temperatures {
                runs.push(run_variant(
                    input,
                    template,
                    *temperature,
                    &prompt_set_hash,
                    &args,
                    &auth,
                )?);
            }
        }
        input.evaluator_runs = runs;
    }
    let params = HypothesisEvaluationParams::default();
    let report = aggregate_hypothesis_evaluations(&input_file.inputs, &params)?;
    let artifact = DriverArtifact {
        schema_version: DRIVER_SCHEMA_VERSION,
        model: args.model.clone(),
        endpoint: http::artifact_endpoint(&args.endpoint),
        prompt_set_id: PROMPT_SET_ID.to_string(),
        prompt_set_sha256: prompt_set_hash,
        temperatures_x100: args.temperatures.clone(),
        source_input: args.input.display().to_string(),
        source_input_sha256: sha256_hex(&input_bytes),
        inputs: input_file.inputs,
        report,
    };
    let persisted = persist_driver(&args.out, &artifact)?;
    print_json(&json!({
        "status": "ok",
        "input": args.input,
        "preflight": preflight,
        "out": persisted.path,
        "out_bytes": persisted.bytes,
        "out_sha256": persisted.sha256,
        "prompt_set_sha256": artifact.prompt_set_sha256,
        "readback": {
            "input_count": persisted.readback_input_count,
            "evaluation_count": persisted.readback_evaluation_count,
        }
    }))
}

fn run_variant(
    input: &HypothesisEvaluationInput,
    template: &PromptTemplate,
    temperature_x100: u16,
    prompt_set_hash: &str,
    args: &HypothesisEvaluatorArgs,
    auth: &http::EvaluatorAuth,
) -> CliResult<EvaluatorRun> {
    let evidence_ids = input
        .retrieved_evidence
        .iter()
        .map(|row| row.evidence_id.as_str())
        .collect::<BTreeSet<_>>();
    let request = json!({
        "model": args.model,
        "temperature": f64::from(temperature_x100) / 100.0,
        "seed": variant_seed(&input.hypothesis_id, template.id, temperature_x100, prompt_set_hash),
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt(input, template)}
        ],
        "response_format": {"type": "json_object"}
    });
    let raw = http::post_json(
        &args.endpoint,
        &request,
        Duration::from_millis(args.timeout_ms),
        auth,
    )
    .map_err(|error| variant_error(error, input, template, temperature_x100))?;
    let parsed = parse_evaluator_json(raw)
        .map_err(|error| variant_error(error, input, template, temperature_x100))?;
    validate_citations(&parsed, &evidence_ids, input, template, temperature_x100)?;
    Ok(EvaluatorRun {
        prompt_id: template.id.to_string(),
        temperature_x100,
        plausible_score: parsed.plausible_score,
        novelty_score: parsed.novelty_score,
        testability_score: parsed.testability_score,
        falsifiability_score: parsed.falsifiability_score,
        justification: parsed.justification,
        falsification_test: parsed.falsification_test,
        cited_evidence_ids: parsed.cited_evidence_ids,
    })
}

fn parse_hypothesis_evaluator(rest: &[String]) -> CliResult<HypothesisEvaluatorArgs> {
    let mut args = HypothesisEvaluatorArgs::default();
    args.temperatures.clear();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--input" => {
                idx += 1;
                args.input = value(rest, idx, "--input")?.into();
            }
            "--out" => {
                idx += 1;
                args.out = value(rest, idx, "--out")?.into();
            }
            "--endpoint" => {
                idx += 1;
                args.endpoint = value(rest, idx, "--endpoint")?.to_string();
            }
            "--auth-env" => {
                idx += 1;
                args.auth_env = Some(value(rest, idx, "--auth-env")?.to_string());
            }
            "--model" => {
                idx += 1;
                args.model = value(rest, idx, "--model")?.to_string();
            }
            "--temperature" => {
                idx += 1;
                args.temperatures
                    .push(parse_temperature(value(rest, idx, "--temperature")?)?);
            }
            "--timeout-ms" => {
                idx += 1;
                args.timeout_ms = value(rest, idx, "--timeout-ms")?
                    .parse::<u64>()
                    .map_err(|err| CliError::usage(format!("parse --timeout-ms: {err}")))?;
            }
            RUN_MANIFEST_FLAG => {
                idx += 1;
                args.preflight.manifest = Some(PathBuf::from(value(rest, idx, RUN_MANIFEST_FLAG)?));
            }
            RUN_STAGE_ID_FLAG => {
                idx += 1;
                args.preflight.stage_id = Some(value(rest, idx, RUN_STAGE_ID_FLAG)?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected hypothesis-evaluator-driver flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.temperatures.is_empty() {
        args.temperatures = HypothesisEvaluatorArgs::default().temperatures;
    }
    if args.input.as_os_str().is_empty() || args.out.as_os_str().is_empty() {
        return Err(CliError::usage(
            "hypothesis-evaluator-driver requires --input <json> and --out <json>",
        ));
    }
    if args.endpoint.trim().is_empty() || args.model.trim().is_empty() {
        return Err(CliError::usage(
            "hypothesis-evaluator-driver requires --endpoint <url> and --model <id>",
        ));
    }
    args.preflight
        .validate_for_command("hypothesis-evaluator-driver")?;
    Ok(args)
}

fn parse_temperature(raw: &str) -> CliResult<u16> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse --temperature {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=2.0).contains(&value) {
        return Err(CliError::usage("--temperature must be finite and in [0,2]"));
    }
    Ok((value * 100.0).round() as u16)
}

#[derive(Clone, Copy)]
struct PromptTemplate {
    id: &'static str,
    body: &'static str,
}

fn prompt_templates() -> [PromptTemplate; 2] {
    [
        PromptTemplate {
            id: "clinical_plausibility_v1",
            body: CLINICAL_PROMPT,
        },
        PromptTemplate {
            id: "falsification_v1",
            body: FALSIFICATION_PROMPT,
        },
    ]
}

fn user_prompt(input: &HypothesisEvaluationInput, template: &PromptTemplate) -> String {
    let evidence = input
        .retrieved_evidence
        .iter()
        .map(|row| {
            format!(
                "- id={} title={} confidence={:.3}\n  abstract={}",
                row.evidence_id, row.title, row.grounding_confidence, row.abstract_text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n\nHypothesis id: {}\nClaim: {}\nEvidence:\n{}\n\nReturn JSON with plausible_score, novelty_score, testability_score, falsifiability_score, justification, falsification_test, cited_evidence_ids.",
        template.body, input.hypothesis_id, input.claim, evidence
    )
}

fn parse_evaluator_json(raw: Value) -> CliResult<EvaluatorJson> {
    if raw.get("plausible_score").is_some() {
        return serde_json::from_value(raw)
            .map_err(|err| evaluator_malformed(format!("parse evaluator JSON: {err}")).into());
    }
    let content = raw
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            evaluator_malformed(
                "endpoint response lacks direct scores or choices[0].message.content",
            )
        })?;
    serde_json::from_str(content).map_err(|err| {
        evaluator_malformed(format!("parse choices[0].message.content JSON: {err}")).into()
    })
}

fn validate_citations(
    parsed: &EvaluatorJson,
    evidence_ids: &BTreeSet<&str>,
    input: &HypothesisEvaluationInput,
    template: &PromptTemplate,
    temperature_x100: u16,
) -> CliResult {
    if parsed.cited_evidence_ids.is_empty() {
        return Err(evaluator_bad_citation(format!(
            "{} {} temp={} cited no evidence",
            input.hypothesis_id, template.id, temperature_x100
        ))
        .into());
    }
    for cited in &parsed.cited_evidence_ids {
        if !evidence_ids.contains(cited.as_str()) {
            return Err(evaluator_bad_citation(format!(
                "{} {} temp={} cited missing evidence id {}",
                input.hypothesis_id, template.id, temperature_x100, cited
            ))
            .into());
        }
    }
    Ok(())
}

fn persist_driver(path: &Path, artifact: &DriverArtifact) -> CliResult<PersistedDriver> {
    let bytes = serde_json::to_vec_pretty(artifact)
        .map_err(|err| CliError::runtime(format!("serialize evaluator artifact: {err}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different evaluator artifact {}",
                path.display()
            )));
        }
    } else {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, path)?;
    }
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CliError::usage(format!(
            "evaluator artifact readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: DriverArtifact = serde_json::from_slice(&readback)
        .map_err(|err| CliError::runtime(format!("parse evaluator artifact readback: {err}")))?;
    Ok(PersistedDriver {
        path: path.to_path_buf(),
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_input_count: decoded.inputs.len(),
        readback_evaluation_count: decoded.report.evaluations.len(),
    })
}

fn variant_error(
    error: CliError,
    input: &HypothesisEvaluationInput,
    template: &PromptTemplate,
    temperature_x100: u16,
) -> CliError {
    CliError::Calyx(CalyxError {
        code: error.code(),
        message: format!(
            "variant hypothesis={} prompt={} temperature_x100={} failed: {}",
            input.hypothesis_id,
            template.id,
            temperature_x100,
            error.message()
        ),
        remediation: error.remediation(),
    })
}

fn evaluator_endpoint_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_UNREACHABLE",
        message: message.into(),
        remediation: "restore the configured evaluator endpoint and retry",
    }
}

fn evaluator_malformed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVALUATOR_MALFORMED_RESPONSE",
        message: message.into(),
        remediation: "fix the evaluator endpoint to return the required JSON schema",
    }
}

fn evaluator_bad_citation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVALUATOR_BAD_CITATION",
        message: message.into(),
        remediation: "make the evaluator cite only evidence ids present in the input bundle",
    }
}

fn prompt_set_sha256() -> String {
    sha256_hex(
        format!("{PROMPT_SET_ID}\n{SYSTEM_PROMPT}\n{CLINICAL_PROMPT}\n{FALSIFICATION_PROMPT}")
            .as_bytes(),
    )
}

fn variant_seed(
    hypothesis_id: &str,
    prompt_id: &str,
    temperature_x100: u16,
    prompt_set_hash: &str,
) -> String {
    sha256_hex(
        format!("{hypothesis_id}|{prompt_id}|{temperature_x100}|{prompt_set_hash}").as_bytes(),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
