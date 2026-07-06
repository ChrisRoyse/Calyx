use std::fs;
use std::path::Path;
use std::process::ExitStatus;

use crate::error::CliResult;

use super::StreamWorkerReport;

pub(super) fn read_worker_report(
    report: &Path,
    stderr: &Path,
    status: ExitStatus,
    descriptor: &str,
) -> CliResult<StreamWorkerReport> {
    let bytes = fs::read(report).map_err(|error| {
        crate::assay_stream_fbin::local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_REPORT_MISSING",
            format!(
                "descriptor={} status={status}; read {} failed: {error}; {}",
                descriptor,
                report.display(),
                stderr_tail(stderr)
            ),
            "inspect the lens worker stderr and rerun after fixing the runtime",
        )
    })?;
    if !status.success() {
        return Err(crate::assay_stream_fbin::local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_FAILED",
            format!(
                "descriptor={} status={status}; {}; report_bytes={}",
                descriptor,
                stderr_tail(stderr),
                bytes.len()
            ),
            "inspect the lens worker stderr and rerun after fixing the runtime",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        crate::assay_stream_fbin::local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_REPORT_INVALID",
            format!("parse {} failed: {error}", report.display()),
            "fix worker report serialization before trusting streamed FBIN",
        )
    })
}

fn stderr_tail(path: &Path) -> String {
    const TAIL_BYTES: usize = 4096;
    match fs::read(path) {
        Ok(bytes) if bytes.is_empty() => format!("stderr {} was empty", path.display()),
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(TAIL_BYTES);
            format!(
                "stderr_tail {}: {}",
                path.display(),
                String::from_utf8_lossy(&bytes[start..]).trim()
            )
        }
        Err(error) => format!("read stderr {} failed: {error}", path.display()),
    }
}
