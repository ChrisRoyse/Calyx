use calyx_core::CalyxError;

pub const CALYX_STRUCTURED_BINDING_INVALID: &str = "CALYX_STRUCTURED_BINDING_INVALID";
pub const CALYX_STRUCTURED_CATEGORY_UNKNOWN: &str = "CALYX_STRUCTURED_CATEGORY_UNKNOWN";
pub const CALYX_STRUCTURED_DATE_INVALID: &str = "CALYX_STRUCTURED_DATE_INVALID";
pub const CALYX_STRUCTURED_DERIVATION_INVALID: &str = "CALYX_STRUCTURED_DERIVATION_INVALID";

pub(crate) fn structured_runtime_error(
    code: &'static str,
    message: impl Into<String>,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: remediation(code),
    }
}

fn remediation(code: &str) -> &'static str {
    match code {
        CALYX_STRUCTURED_BINDING_INVALID => {
            "repair the frozen selector/type/encoder contract and register a new LensId"
        }
        CALYX_STRUCTURED_CATEGORY_UNKNOWN => {
            "add the category to a newly registered frozen vocabulary or repair the input"
        }
        CALYX_STRUCTURED_DATE_INVALID => {
            "supply an exact Gregorian YYYY-MM-DD value or register a different source type"
        }
        CALYX_STRUCTURED_DERIVATION_INVALID => {
            "repair the declared numeric operands/operation and register a new LensId"
        }
        _ => "repair the structured record and retry without coercion or fallback",
    }
}
