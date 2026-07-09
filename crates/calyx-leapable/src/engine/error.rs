use calyx_core::CalyxError;
use serde::Deserialize;
use serde_json::Value;

use super::{CALYX_LEAPABLE_VAULT_NOT_OPEN, CALYX_LEAPABLE_VAULT_OPEN};
pub(super) type EngineResult<T> = std::result::Result<T, EngineError>;

pub(super) enum EngineError {
    InvalidParams(String),
    Calyx(CalyxError),
}

impl From<CalyxError> for EngineError {
    fn from(error: CalyxError) -> Self {
        Self::Calyx(error)
    }
}

pub(super) fn parse_params<T: for<'de> Deserialize<'de>>(
    params: Option<Value>,
    method: &str,
) -> EngineResult<T> {
    let params = params.unwrap_or(Value::Null);
    serde_json::from_value(params).map_err(|error| {
        EngineError::InvalidParams(format!("{method} params do not match schema: {error}"))
    })
}

pub(super) fn vault_not_open(vault_ref: &str) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_VAULT_NOT_OPEN,
        message: format!("vault_ref {vault_ref:?} is not open in this engine process"),
        remediation: "call vault.open for this vault_ref before issuing vault-scoped requests",
    }
}

pub(super) fn vault_open_error(vault_ref: &str) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_VAULT_OPEN,
        message: format!("vault_ref {vault_ref:?} has an open handle in this engine process"),
        remediation: "call vault.close before delete, restore, or overwriting lifecycle operations",
    }
}
