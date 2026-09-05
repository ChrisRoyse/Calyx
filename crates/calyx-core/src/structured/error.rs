use crate::CalyxError;

pub const CALYX_STRUCTURED_SCHEMA_INVALID: &str = "CALYX_STRUCTURED_SCHEMA_INVALID";
pub const CALYX_STRUCTURED_SELECTOR_INVALID: &str = "CALYX_STRUCTURED_SELECTOR_INVALID";
pub const CALYX_STRUCTURED_FIELD_MISSING: &str = "CALYX_STRUCTURED_FIELD_MISSING";
pub const CALYX_STRUCTURED_TYPE_MISMATCH: &str = "CALYX_STRUCTURED_TYPE_MISMATCH";
pub const CALYX_STRUCTURED_NON_FINITE: &str = "CALYX_STRUCTURED_NON_FINITE";
pub const CALYX_STRUCTURED_SCHEMA_MISMATCH: &str = "CALYX_STRUCTURED_SCHEMA_MISMATCH";
pub const CALYX_STRUCTURED_RECORD_INVALID: &str = "CALYX_STRUCTURED_RECORD_INVALID";
pub const CALYX_STRUCTURED_RESERVED_METADATA: &str = "CALYX_STRUCTURED_RESERVED_METADATA";

pub(crate) fn structured_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: remediation(code),
    }
}

fn remediation(code: &str) -> &'static str {
    match code {
        CALYX_STRUCTURED_SCHEMA_INVALID => {
            "declare a non-empty stable schema id and a supported typed JSON object"
        }
        CALYX_STRUCTURED_SELECTOR_INVALID => {
            "use a valid non-root RFC 6901 field pointer and register a new LensId"
        }
        CALYX_STRUCTURED_FIELD_MISSING => {
            "supply the required selected field under the frozen record schema"
        }
        CALYX_STRUCTURED_TYPE_MISMATCH => {
            "supply the declared source type without coercion or register a new binding"
        }
        CALYX_STRUCTURED_NON_FINITE => {
            "supply a finite representable number and preserve the original source value"
        }
        CALYX_STRUCTURED_SCHEMA_MISMATCH => {
            "use the exact frozen schema/binding for ingest, query, replay, and backfill"
        }
        CALYX_STRUCTURED_RESERVED_METADATA => {
            "remove calyx.structured.* user metadata; those keys are engine-owned"
        }
        _ => "supply a canonical supported structured record and retry",
    }
}
