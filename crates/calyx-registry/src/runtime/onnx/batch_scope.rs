use std::cell::Cell;

use calyx_core::{CalyxError, Result};

use super::config_invalid;

thread_local! {
    static SCOPED_RUNTIME_BATCH_LIMIT: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(crate) fn with_runtime_batch_limit<T>(
    limit: Option<usize>,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if limit == Some(0) {
        return Err(CalyxError::lens_unreachable(
            "ONNX runtime batch limit must be > 0 when supplied",
        ));
    }
    SCOPED_RUNTIME_BATCH_LIMIT.with(|slot| {
        let previous = slot.replace(limit);
        let result = run();
        slot.set(previous);
        result
    })
}

pub(in crate::runtime::onnx) fn scoped_max_batch(spec_max: Option<usize>) -> Result<Option<usize>> {
    if spec_max == Some(0) {
        return Err(config_invalid("LensSpec max_batch must be > 0"));
    }
    let scoped = SCOPED_RUNTIME_BATCH_LIMIT.with(Cell::get);
    let out = match (spec_max, scoped) {
        (Some(spec), Some(limit)) => Some(spec.min(limit)),
        (Some(spec), None) => Some(spec),
        (None, Some(limit)) => Some(limit),
        (None, None) => None,
    };
    Ok(out)
}
