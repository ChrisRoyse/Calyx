use std::collections::BTreeMap;

use calyx_core::{CalyxError, Result};
use ort::memory::{AllocationDevice, Allocator, AllocatorType, MemoryInfo, MemoryType};
use ort::session::{IoBinding, RunOptions, Session, SessionOutputs};
use ort::value::{DynTensorValueType, Shape, Tensor, TensorElementType, ValueType};

use super::arena::{ARENA_SHRINK_ENV, ArenaShrinkPolicy};
use super::config_invalid;

pub(super) const CUDA_GRAPHS_ENV: &str = "CALYX_ONNX_CUDA_GRAPHS";

#[derive(Debug, Default)]
pub(super) struct CudaGraphRunConfig {
    enabled: bool,
    graph_ids: BTreeMap<(usize, usize), u32>,
    bindings: BTreeMap<(usize, usize), CudaGraphBinding>,
}

#[derive(Debug)]
struct CudaGraphBinding {
    binding: IoBinding,
    input_tensors: BTreeMap<String, Tensor<i64>>,
}

pub(super) struct CudaGraphRunRequest<'a> {
    pub(super) label: &'a str,
    pub(super) device_id: i32,
    pub(super) shape: (usize, usize),
    pub(super) options: Option<&'a RunOptions>,
}

impl CudaGraphRunConfig {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            graph_ids: BTreeMap::new(),
            bindings: BTreeMap::new(),
        }
    }

    pub(super) const fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn add_run_options(
        &mut self,
        options: &mut RunOptions,
        label: &str,
        shape: (usize, usize),
        new_shape: bool,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let next_id = u32::try_from(self.graph_ids.len()).map_err(|_| CalyxError {
            code: "CALYX_ONNX_CUDA_GRAPH_ID_OVERFLOW",
            message: format!("{label} exhausted CUDA graph ids"),
            remediation: "lower CALYX_ONNX_MAX_DISTINCT_SHAPES or disable CUDA graphs for this session",
        })?;
        let graph_id = *self.graph_ids.entry(shape).or_insert(next_id);
        options
            .add_config_entry("gpu_graph_id", graph_id.to_string())
            .map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph run-option config failed for {label}: {err}"
                ))
            })?;
        if new_shape {
            eprintln!(
                "CALYX_ONNX_RUNTIME phase=cuda_graph_shape label={} batch={} seq={} gpu_graph_id={} distinct_graphs={}",
                label,
                shape.0,
                shape.1,
                graph_id,
                self.graph_ids.len()
            );
        }
        Ok(())
    }

    pub(super) fn run_extract<R>(
        &mut self,
        session: &mut Session,
        request: CudaGraphRunRequest<'_>,
        inputs: Vec<(String, Tensor<i64>)>,
        extract: impl FnOnce(&SessionOutputs<'_>) -> Result<R>,
    ) -> Result<R> {
        let label = request.label;
        let shape = request.shape;
        let is_new = !self.bindings.contains_key(&shape);
        if is_new {
            let binding = CudaGraphBinding::new(session, label, request.device_id, shape, &inputs)?;
            self.bindings.insert(shape, binding);
        }
        let binding = self.bindings.get_mut(&shape).ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_CUDA_GRAPH_BINDING_MISSING",
            message: format!("{label} has no CUDA graph binding for batch={} seq={}", shape.0, shape.1),
            remediation: "rerun with CALYX_ONNX_CUDA_GRAPHS unset and report the missing graph binding state",
        })?;
        if !is_new {
            binding.update_inputs(label, inputs)?;
        }
        let mut outputs = match request.options {
            Some(options) => session.run_binding_with_options(&binding.binding, options),
            None => session.run_binding(&binding.binding),
        }
        .map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph inference failed for {label} batch={} seq={}: {err}",
                shape.0, shape.1
            ))
        })?;
        copy_outputs_to_cpu(label, &mut outputs)?;
        let result = extract(&outputs)?;
        drop(outputs);
        Ok(result)
    }
}

impl CudaGraphBinding {
    fn new(
        session: &mut Session,
        label: &str,
        device_id: i32,
        shape: (usize, usize),
        inputs: &[(String, Tensor<i64>)],
    ) -> Result<Self> {
        let mut binding = session.create_binding().map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph binding create failed for {label}: {err}"
            ))
        })?;
        let input_info = MemoryInfo::new(
            AllocationDevice::CUDA,
            device_id,
            AllocatorType::Device,
            MemoryType::Default,
        )
        .map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph input MemoryInfo failed for {label} device {device_id}: {err}"
            ))
        })?;
        let input_allocator = Allocator::new(session, input_info).map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph input allocator failed for {label} device {device_id}: {err}"
            ))
        })?;
        let input_shape = tensor_shape_2d(shape)?;
        let mut input_tensors = BTreeMap::new();
        for (name, source) in inputs {
            let mut target =
                Tensor::<i64>::new(&input_allocator, input_shape.clone()).map_err(|err| {
                    config_invalid(format!(
                        "ONNX CUDA graph input tensor {name} alloc failed for {label}: {err}"
                    ))
                })?;
            source.copy_into(&mut target).map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph input tensor {name} copy failed for {label}: {err}"
                ))
            })?;
            binding.bind_input(name.as_str(), &target).map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph bind_input {name} failed for {label}: {err}"
                ))
            })?;
            input_tensors.insert(name.clone(), target);
        }
        let output_info = MemoryInfo::new(
            AllocationDevice::CUDA,
            device_id,
            AllocatorType::Device,
            MemoryType::Default,
        )
        .map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph output MemoryInfo failed for {label} device {device_id}: {err}"
            ))
        })?;
        let output_allocator = Allocator::new(session, output_info).map_err(|err| {
            config_invalid(format!(
                "ONNX CUDA graph output allocator failed for {label} device {device_id}: {err}"
            ))
        })?;
        for output in session.outputs() {
            let name = output.name();
            let output_shape = concrete_output_shape(label, name, output.dtype(), shape)?;
            let output_tensor =
                Tensor::<f32>::new(&output_allocator, output_shape).map_err(|err| {
                    config_invalid(format!(
                        "ONNX CUDA graph output tensor {name} alloc failed for {label}: {err}"
                    ))
                })?;
            binding.bind_output(name, output_tensor).map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph bind_output {name} failed for {label}: {err}"
                ))
            })?;
        }
        eprintln!(
            "CALYX_ONNX_RUNTIME phase=cuda_graph_binding label={} batch={} seq={} inputs={} outputs={}",
            label,
            shape.0,
            shape.1,
            input_tensors.len(),
            session.outputs().len()
        );
        Ok(Self {
            binding,
            input_tensors,
        })
    }

    fn update_inputs(&mut self, label: &str, inputs: Vec<(String, Tensor<i64>)>) -> Result<()> {
        for (name, source) in inputs {
            let target = self.input_tensors.get_mut(&name).ok_or_else(|| CalyxError {
                code: "CALYX_ONNX_CUDA_GRAPH_INPUT_MISSING",
                message: format!("{label} CUDA graph binding has no input tensor named {name}"),
                remediation: "rerun with CALYX_ONNX_CUDA_GRAPHS unset and verify the ONNX model input schema",
            })?;
            source.copy_into(target).map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph input tensor {name} refresh failed for {label}: {err}"
                ))
            })?;
        }
        Ok(())
    }
}

fn copy_outputs_to_cpu(label: &str, outputs: &mut SessionOutputs<'_>) -> Result<()> {
    let names = outputs.keys().map(str::to_string).collect::<Vec<String>>();
    for name in names {
        let cpu_output = outputs
            .get(&name)
            .ok_or_else(|| CalyxError {
                code: "CALYX_ONNX_CUDA_GRAPH_OUTPUT_MISSING",
                message: format!("{label} CUDA graph run returned no output named {name}"),
                remediation: "rerun with CALYX_ONNX_CUDA_GRAPHS unset and verify the ONNX model output schema",
            })?
            .downcast_ref::<DynTensorValueType>()
            .map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph output {name} downcast failed for {label}: {err}"
                ))
            })?
            .to(AllocationDevice::CPU, 0)
            .map_err(|err| {
                config_invalid(format!(
                    "ONNX CUDA graph output {name} device-to-host copy failed for {label}: {err}"
                ))
            })?
            .into_dyn();
        let output = outputs.get_mut(&name).ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_CUDA_GRAPH_OUTPUT_MISSING",
            message: format!("{label} CUDA graph run lost output named {name} during CPU copy"),
            remediation: "rerun with CALYX_ONNX_CUDA_GRAPHS unset and verify the ONNX model output schema",
        })?;
        *output = cpu_output;
    }
    Ok(())
}

fn tensor_shape_2d(shape: (usize, usize)) -> Result<Shape> {
    Ok(Shape::new([
        i64::try_from(shape.0).map_err(|_| CalyxError {
            code: "CALYX_ONNX_CUDA_GRAPH_SHAPE_OVERFLOW",
            message: format!("CUDA graph batch {} exceeds i64", shape.0),
            remediation: "lower the ONNX batch bucket before enabling CUDA graphs",
        })?,
        i64::try_from(shape.1).map_err(|_| CalyxError {
            code: "CALYX_ONNX_CUDA_GRAPH_SHAPE_OVERFLOW",
            message: format!("CUDA graph seq {} exceeds i64", shape.1),
            remediation: "lower the ONNX sequence bucket before enabling CUDA graphs",
        })?,
    ]))
}

fn concrete_output_shape(
    label: &str,
    name: &str,
    value_type: &ValueType,
    run_shape: (usize, usize),
) -> Result<Shape> {
    let ValueType::Tensor { ty, shape, .. } = value_type else {
        return Err(config_invalid(format!(
            "ONNX CUDA graph output {name} for {label} is not a tensor"
        )));
    };
    if *ty != TensorElementType::Float32 {
        return Err(config_invalid(format!(
            "ONNX CUDA graph output {name} for {label} is {ty}, expected Float32"
        )));
    }
    let batch = i64::try_from(run_shape.0)
        .map_err(|_| config_invalid(format!("{label} CUDA graph batch exceeds i64")))?;
    let seq = i64::try_from(run_shape.1)
        .map_err(|_| config_invalid(format!("{label} CUDA graph seq exceeds i64")))?;
    let concrete = shape
        .iter()
        .enumerate()
        .map(|(index, dim)| match (*dim > 0, index) {
            (true, _) => Ok(*dim),
            (false, 0) => Ok(batch),
            (false, 1) if shape.len() >= 3 => Ok(seq),
            _ => Err(config_invalid(format!(
                "ONNX CUDA graph output {name} for {label} has unsupported dynamic shape {shape:?}"
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Shape::new(concrete))
}

pub(super) fn configured_cuda_graphs() -> Result<bool> {
    let Ok(raw) = std::env::var(CUDA_GRAPHS_ENV) else {
        return Ok(false);
    };
    let raw = raw.trim();
    if raw.is_empty()
        || raw == "0"
        || raw.eq_ignore_ascii_case("false")
        || raw.eq_ignore_ascii_case("off")
        || raw.eq_ignore_ascii_case("no")
    {
        return Ok(false);
    }
    if raw == "1"
        || raw.eq_ignore_ascii_case("true")
        || raw.eq_ignore_ascii_case("on")
        || raw.eq_ignore_ascii_case("yes")
    {
        return Ok(true);
    }
    Err(CalyxError {
        code: "CALYX_ONNX_CUDA_GRAPHS_INVALID",
        message: format!("{CUDA_GRAPHS_ENV}={raw} is not one of 1/0, true/false, on/off, yes/no"),
        remediation: "set CALYX_ONNX_CUDA_GRAPHS=1 to enable ORT CUDA Graphs for CUDA sessions, or unset it",
    })
}

pub(super) fn compatible_arena_shrink(
    cuda_graphs: bool,
    policy: ArenaShrinkPolicy,
) -> Result<ArenaShrinkPolicy> {
    if !cuda_graphs || policy == ArenaShrinkPolicy::Off {
        return Ok(policy);
    }
    let explicit = std::env::var(ARENA_SHRINK_ENV)
        .map(|raw| !raw.trim().is_empty())
        .unwrap_or(false);
    if !explicit {
        return Ok(ArenaShrinkPolicy::Off);
    }
    Err(CalyxError {
        code: "CALYX_ONNX_CUDA_GRAPHS_ARENA_SHRINK",
        message: format!(
            "{CUDA_GRAPHS_ENV}=1 is incompatible with {ARENA_SHRINK_ENV}={}",
            policy.as_str()
        ),
        remediation: "set CALYX_ONNX_ARENA_SHRINK=off or unset CALYX_ONNX_CUDA_GRAPHS; CUDA graph capture cannot run with ORT arena shrinkage",
    })
}
