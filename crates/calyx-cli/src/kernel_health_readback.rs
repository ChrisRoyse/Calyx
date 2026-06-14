//! `calyx readback kernel-health` — prints the persisted kernel health
//! aggregate (PRD 08 §8) for manual FSV inspection.

use std::path::Path;
use std::str::FromStr;

use calyx_core::CxId;
use calyx_lodestar::{FsKernelStore, kernel_health};

pub fn readback_kernel_health(root: &Path, kernel_id: &str) -> crate::error::CliResult {
    let kernel_id =
        CxId::from_str(kernel_id).map_err(|error| format!("invalid --kernel-id: {error}"))?;
    let store = FsKernelStore::new(root);
    let health = kernel_health(kernel_id, &store)?;
    let json = serde_json::to_string_pretty(&health).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}
