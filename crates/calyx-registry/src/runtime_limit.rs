use calyx_core::{CalyxError, Input, LensId, Result, SlotVector};

use crate::Registry;
use crate::runtime::onnx;
use crate::spec::LensRuntime;

pub fn measure_registry_batch_with_runtime_limit(
    registry: &Registry,
    lens_id: LensId,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> Result<Vec<SlotVector>> {
    if runtime_batch_limit == Some(0) {
        return Err(CalyxError::lens_unreachable(
            "runtime batch limit must be > 0 when supplied",
        ));
    }
    if runtime_uses_scoped_batch_limit(registry.lens_spec(lens_id)) {
        return onnx::with_runtime_batch_limit(runtime_batch_limit, || {
            registry.measure_batch(lens_id, inputs)
        });
    }
    let Some(limit) = runtime_batch_limit else {
        return registry.measure_batch(lens_id, inputs);
    };
    let mut out = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(limit) {
        out.extend(registry.measure_batch(lens_id, chunk)?);
    }
    Ok(out)
}

pub(crate) fn runtime_uses_scoped_batch_limit(spec: Option<&crate::LensSpec>) -> bool {
    matches!(
        spec.map(|spec| &spec.runtime),
        Some(LensRuntime::Onnx { .. } | LensRuntime::OnnxColbert { .. })
    )
}
