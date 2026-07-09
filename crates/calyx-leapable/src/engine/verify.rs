use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::EngineResult;
use crate::lifecycle::verify_restore_value_with_crypto;
use calyx_aster::security::SharedVaultContext;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VerifyMode {
    Full,
    #[default]
    Target,
    None,
}

impl VerifyMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Target => "target",
            Self::None => "none",
        }
    }

    pub(super) fn verifies_source(self) -> bool {
        matches!(self, Self::Full)
    }

    pub(super) fn verifies_target(self) -> bool {
        matches!(self, Self::Full | Self::Target)
    }
}

pub(super) fn maybe_verify_path_with_crypto(
    enabled: bool,
    path: &Path,
    context: &SharedVaultContext,
) -> EngineResult<Option<Value>> {
    Ok(enabled
        .then(|| verify_restore_value_with_crypto(path, context))
        .transpose()?)
}

pub(super) fn lifecycle_progress(operation: &str, phase: &str, vault_ref: &str) {
    eprintln!("calyx-leapable: {operation} {phase} vault_ref={vault_ref}");
}
