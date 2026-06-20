use std::fs::File;
use std::io::Read;
use std::path::Path;

use calyx_core::CalyxError;
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

pub(super) fn sha256_file(path: &Path) -> CliResult<String> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(io_error)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

pub(super) fn io_error(error: std::io::Error) -> CliError {
    st_error(
        "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_IO",
        error.to_string(),
        "inspect the named path and retry with readable/writable files",
    )
}

pub(super) fn st_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write");
    }
    out
}
