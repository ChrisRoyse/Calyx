use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use calyx_core::{CalyxError, Input, Lens, Result, SlotVector};
use serde_json::Value;
use tokenizers::Tokenizer;

pub(in crate::runtime::onnx) mod batch;
mod output;
mod pipeline;
mod rows;

use super::cuda_guard::CudaDropGuard;
use super::io_binding::OnnxRunPlan;
use super::session::{ManagedOnnxSession, build_session};
use super::{OnnxFileSpec, OnnxLens, OnnxModelFiles, config_invalid};
use crate::frozen::{FrozenLensContract, LensDType, sha256_digest};
use crate::runtime::common::hash_files;
use batch::{TokenBatch, session_inputs, stream_token_batches, token_batches};
#[cfg(test)]
pub(super) use output::pool_output;
pub(in crate::runtime::onnx) use output::pooling_from_config;
use output::{CustomOutput, output_from_session, vectors_from_output};
use pipeline::{
    log_pipeline_start, pipeline_batch_window, pipeline_output_window, should_pipeline,
};
use rows::{finalize_slot_rows, write_slot_rows};

pub struct CustomOnnxRuntime {
    session: ManagedOnnxSession,
    run_plan: OnnxRunPlan,
    tokenizer: Tokenizer,
    output: CustomOutput,
    max_tokens: usize,
}

impl CustomOnnxRuntime {
    pub const fn dim(&self) -> u32 {
        self.output.dim()
    }

    pub fn measure_batch(
        &mut self,
        lens: &dyn Lens,
        inputs: &[Input],
        max_batch: Option<usize>,
    ) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let max_batch = super::scoped_max_batch(max_batch)?;
        if should_pipeline(inputs.len(), max_batch) {
            return self.measure_inputs_pipelined(lens, inputs, max_batch);
        }
        let batches = token_batches(
            &self.tokenizer,
            lens,
            inputs,
            self.max_tokens,
            max_batch,
            self.run_plan.pads_batches(),
        )?;
        self.measure_token_batches_serial(&batches, inputs.len())
    }

    fn measure_token_batches_serial(
        &mut self,
        batches: &[TokenBatch],
        input_len: usize,
    ) -> Result<Vec<SlotVector>> {
        let mut rows = vec![None; input_len];
        for batch in batches {
            let vectors = self.run_token_batch(batch)?;
            write_slot_rows(&mut rows, batch, vectors)?;
        }
        finalize_slot_rows(rows)
    }

    fn measure_inputs_pipelined(
        &mut self,
        lens: &dyn Lens,
        inputs: &[Input],
        max_batch: Option<usize>,
    ) -> Result<Vec<SlotVector>> {
        let batch_window = pipeline_batch_window()?;
        let output_window = pipeline_output_window()?;
        log_pipeline_start(inputs.len(), batch_window, output_window, max_batch);
        let (batch_sender, batch_receiver) = mpsc::sync_channel(batch_window);
        let (row_sender, row_receiver) =
            mpsc::sync_channel::<(TokenBatch, Vec<SlotVector>)>(output_window);
        let tokenizer = self.tokenizer.clone();
        let max_tokens = self.max_tokens;
        let pad_batches = self.run_plan.pads_batches();
        let input_len = inputs.len();
        thread::scope(|scope| {
            let producer = scope.spawn(move || {
                stream_token_batches(
                    &tokenizer,
                    lens,
                    inputs,
                    max_tokens,
                    max_batch,
                    pad_batches,
                    |batch| {
                        batch_sender.send(batch).map_err(|_| {
                            CalyxError::lens_unreachable("custom ONNX pipeline session stopped")
                        })
                    },
                )
            });
            let finalizer = scope.spawn(move || {
                let mut rows = vec![None; input_len];
                while let Ok((batch, vectors)) = row_receiver.recv() {
                    write_slot_rows(&mut rows, &batch, vectors)?;
                }
                finalize_slot_rows(rows)
            });
            let mut session_error = None;
            while let Ok(batch) = batch_receiver.recv() {
                match self.run_token_batch(&batch) {
                    Ok(pooled) => {
                        if row_sender.send((batch, pooled)).is_err() {
                            session_error = Some(CalyxError::lens_unreachable(
                                "custom ONNX pipeline finalizer stopped",
                            ));
                            break;
                        }
                    }
                    Err(error) => {
                        session_error = Some(error);
                        break;
                    }
                }
            }
            drop(batch_receiver);
            drop(row_sender);
            let producer_result = producer.join().map_err(|_| {
                CalyxError::lens_unreachable("custom ONNX tokenizer worker panicked")
            })?;
            let finalizer_result = finalizer.join().map_err(|_| {
                CalyxError::lens_unreachable("custom ONNX finalizer worker panicked")
            })?;
            if let Some(error) = session_error {
                return Err(error);
            }
            producer_result?;
            finalizer_result
        })
    }
}

pub fn from_files(spec: OnnxFileSpec) -> Result<OnnxLens> {
    let _ort_dylib = super::dynamic_ort::ensure_dynamic_ort(spec.provider_policy)?;
    ensure_file("model", &spec.model_file)?;
    ensure_file("tokenizer", &spec.tokenizer)?;
    ensure_file("config", &spec.config)?;
    let config = validate_config(&spec.config)?;
    let max_tokens = batch::max_tokens_from_config(&config)?;
    let files = model_files(&spec);
    let weights_sha256 = hash_files(&files.artifact_paths())
        .map_err(|err| config_invalid(format!("read custom ONNX artifacts failed: {err}")))?;
    if let Some(expected) = spec.expected_weights_sha256
        && expected != weights_sha256
    {
        return Err(CalyxError::lens_frozen_violation(format!(
            "custom ONNX weights hash drift for {}",
            spec.model_id
        )));
    }
    let run_label = format!("onnx-custom:{}", spec.model_id);
    super::arena::preflight_gpu_mem_limit_for_artifacts(
        &run_label,
        spec.provider_policy,
        files.artifact_paths().iter().map(|path| path.as_path()),
    )?;
    let session = build_session(&run_label, &spec.model_file, spec.provider_policy)?;
    let session = CudaDropGuard::new(session, spec.provider_policy);
    let run_plan = OnnxRunPlan::new(spec.provider_policy, run_label)?;
    let output = output_from_session(
        session.as_ref(),
        spec.expected_shape,
        spec.pooling,
        spec.norm_policy,
    )?;
    let shape = output.shape();
    let tokenizer = Tokenizer::from_file(&spec.tokenizer)
        .map_err(|err| config_invalid(format!("load tokenizer failed: {err}")))?;
    let corpus_hash = custom_corpus_hash(&spec, output);
    let contract = FrozenLensContract::new(
        spec.name,
        weights_sha256,
        corpus_hash,
        shape,
        spec.modality,
        LensDType::F32,
        spec.norm_policy,
    );
    let runtime = CustomOnnxRuntime {
        session: session.into_inner(),
        run_plan,
        tokenizer,
        output,
        max_tokens,
    };
    Ok(OnnxLens::from_custom_parts(
        contract,
        files,
        spec.provider_policy,
        spec.max_batch,
        runtime,
    ))
}

fn custom_corpus_hash(spec: &OnnxFileSpec, output: CustomOutput) -> [u8; 32] {
    match output {
        CustomOutput::Dense { .. } => sha256_digest(&[
            b"onnx-custom-v1",
            spec.model_id.as_bytes(),
            spec.pooling.as_str().as_bytes(),
            format!("{:?}", spec.norm_policy).as_bytes(),
        ]),
        CustomOutput::Sparse { .. } => sha256_digest(&[
            b"onnx-custom-splade-v1",
            spec.model_id.as_bytes(),
            b"sparse-positive-f32",
        ]),
    }
}

impl CustomOnnxRuntime {
    fn run_token_batch(&mut self, batch: &TokenBatch) -> Result<Vec<SlotVector>> {
        let input_tensors = session_inputs(self.session.as_ref(), batch)?;
        let output = self.output;
        self.run_plan.run_extract(
            self.session.as_mut(),
            input_tensors,
            (batch.batch, batch.seq),
            |outputs| vectors_from_output(outputs, batch, output),
        )
    }
}

fn model_files(spec: &OnnxFileSpec) -> OnnxModelFiles {
    let cache_dir = spec
        .model_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    OnnxModelFiles {
        cache_dir,
        model_code: spec.model_id.clone(),
        model_file: spec.model_file.clone(),
        tokenizer: spec.tokenizer.clone(),
        config: spec.config.clone(),
        special_tokens_map: spec.config.clone(),
        tokenizer_config: spec.tokenizer.clone(),
        contract_paths: spec.contract_paths.clone(),
    }
}

fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(config_invalid(format!(
        "custom ONNX {label} file {} is missing",
        path.display()
    )))
}

fn validate_config(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|err| {
        config_invalid(format!("read ONNX config {} failed: {err}", path.display()))
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
        config_invalid(format!(
            "parse ONNX config {} failed: {err}",
            path.display()
        ))
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(config_invalid("ONNX config must be a JSON object"))
    }
}
