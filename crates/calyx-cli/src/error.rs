//! CLI error contract: every failure serializes to a stable, machine-dispatchable
//! envelope on stderr and exits `2`.
//!
//! `CliError` wraps the structured [`CalyxError`] catalog (PRD 18) plus three
//! CLI-local conditions — I/O, usage, and runtime — that have no catalog entry
//! but still must surface a stable `code` + `remediation` so an agent (A17) can
//! self-correct without parsing prose. Every variant serializes to the exact
//! wire shape `{"code":"CALYX_*","message":"…","remediation":"…"}` so the JSON
//! emitted on stderr is byte-identical to what [`CalyxError`] produces over any
//! other surface (MCP, API).
//!
//! Classification is explicit at every failure site (issue #1145):
//! - [`CliError::Calyx`] — a typed catalog error propagated verbatim via `?`;
//!   its `code` and `remediation` must never be rewritten.
//! - [`CliError::Usage`] — the caller must change the command line; reserved
//!   for argument/flag parse failures only.
//! - [`CliError::Io`] — the OS refused a file/path operation.
//! - [`CliError::Runtime`] — the command and arguments were valid but an
//!   operation failed without a catalog entry (gate mismatch, worker panic,
//!   malformed stored data).
//!
//! There is deliberately no `From<String>` / `From<&str>` conversion: a blanket
//! string conversion silently reclassified stringified [`CalyxError`]s (and
//! I/O errors) as usage errors, destroying their typed `code`/`remediation`
//! and pointing operators at `--help` for storage-state failures (#1145).
//! Removing the impls makes that misclassification a compile error: every
//! string-typed failure must name its class via [`CliError::usage`],
//! [`CliError::io`], or [`CliError::runtime`].
//!
//! Stream/exit contract follows POSIX + the dual-consumer guidance in the
//! references on the card: errors go to stderr, success output to stdout, and a
//! non-zero exit (`2`, "command misuse") pairs with an explicit `code` so
//! automation never has to rely on the exit number alone.

use std::io;
use std::process;

use calyx_core::CalyxError;
use serde::Serialize;

/// Sentinel code for an OS/I/O failure with no PRD 18 catalog entry.
pub(crate) const CALYX_CLI_IO_ERROR: &str = "CALYX_CLI_IO_ERROR";
/// Remediation for [`CALYX_CLI_IO_ERROR`].
const CLI_IO_REMEDIATION: &str = "check the path/permissions in the message and retry";

/// Sentinel code for a misused command (bad/missing args, unknown subcommand).
pub(crate) const CALYX_CLI_USAGE_ERROR: &str = "CALYX_CLI_USAGE_ERROR";
/// Remediation for [`CALYX_CLI_USAGE_ERROR`].
const CLI_USAGE_REMEDIATION: &str =
    "run `calyx --help` and fix the command/flags shown in the message";

/// Sentinel code for a runtime failure (valid command, failed operation) with
/// no PRD 18 catalog entry.
pub(crate) const CALYX_CLI_RUNTIME_ERROR: &str = "CALYX_CLI_RUNTIME_ERROR";
/// Remediation for [`CALYX_CLI_RUNTIME_ERROR`].
const CLI_RUNTIME_REMEDIATION: &str = "the command and flags were valid; inspect the failure \
     detail in the message, fix the named state or input, and retry";
/// Remediation for subsystem-local `CALYX_*` errors that are not PRD 18 entries.
const CLI_SUBSYSTEM_REMEDIATION: &str =
    "follow the emitted CALYX_* subsystem code and inspect the named source of truth";

/// Exit code emitted for every CLI error (POSIX "command misuse").
pub(crate) const CLI_ERROR_EXIT: u8 = 2;

pub(crate) type CliResult<T = ()> = std::result::Result<T, CliError>;

/// Canonical CLI error. Either a structured catalog error or a CLI-local
/// condition (I/O, usage). All three render to the same `{code,message,
/// remediation}` envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliError {
    /// A PRD 18 catalog error carried verbatim (code + remediation preserved).
    Calyx(CalyxError),
    /// An OS/I/O failure surfaced under [`CALYX_CLI_IO_ERROR`].
    Io(String),
    /// A command-misuse failure surfaced under [`CALYX_CLI_USAGE_ERROR`].
    Usage(String),
    /// A valid command whose operation failed without a catalog entry,
    /// surfaced under [`CALYX_CLI_RUNTIME_ERROR`].
    Runtime(String),
}

/// Private serialization shape — guarantees byte-identical field order
/// (`code`, `message`, `remediation`) across every variant and matches the
/// `serde` layout of [`CalyxError`] itself.
#[derive(Serialize)]
struct Wire<'a> {
    code: &'a str,
    message: &'a str,
    remediation: &'a str,
}

impl CliError {
    /// Builds a usage error (bad/missing args, unknown subcommand).
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// Builds an I/O error from a context message.
    pub(crate) fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }

    /// Builds a runtime error (valid command, failed operation).
    pub(crate) fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }

    /// Returns the stable, machine-dispatchable code.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Calyx(error) => error.code,
            Self::Io(_) => CALYX_CLI_IO_ERROR,
            Self::Usage(_) => CALYX_CLI_USAGE_ERROR,
            Self::Runtime(_) => CALYX_CLI_RUNTIME_ERROR,
        }
    }

    /// Returns the concrete failure detail.
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Calyx(error) => &error.message,
            Self::Io(message) | Self::Usage(message) | Self::Runtime(message) => message,
        }
    }

    /// Returns the stable remediation text.
    pub(crate) fn remediation(&self) -> &'static str {
        match self {
            Self::Calyx(error) => error.remediation,
            Self::Io(_) => CLI_IO_REMEDIATION,
            Self::Usage(_) => CLI_USAGE_REMEDIATION,
            Self::Runtime(_) => CLI_RUNTIME_REMEDIATION,
        }
    }

    /// Serializes to the canonical wire envelope
    /// `{"code":"CALYX_*","message":"…","remediation":"…"}`.
    ///
    /// `serde_json` on a fixed-field struct cannot fail here (all fields are
    /// `&str`); if it ever does we surface the serializer error verbatim rather
    /// than hiding it, so a serialization regression is never silently empty.
    pub(crate) fn to_json(&self) -> String {
        let wire = Wire {
            code: self.code(),
            message: self.message(),
            remediation: self.remediation(),
        };
        serde_json::to_string(&wire)
            .unwrap_or_else(|error| panic!("CliError envelope must serialize: {error}"))
    }

    /// Writes the JSON envelope to stderr and exits the process with
    /// [`CLI_ERROR_EXIT`]. Never returns.
    pub(crate) fn emit(&self) -> ! {
        eprintln!("{}", self.to_json());
        process::exit(i32::from(CLI_ERROR_EXIT));
    }
}

impl From<CalyxError> for CliError {
    fn from(error: CalyxError) -> Self {
        Self::Calyx(error)
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::io(error.to_string())
    }
}

impl From<calyx_oracle::OracleError> for CliError {
    fn from(error: calyx_oracle::OracleError) -> Self {
        Self::Calyx(CalyxError::from(error))
    }
}

impl From<calyx_forge::ForgeError> for CliError {
    fn from(error: calyx_forge::ForgeError) -> Self {
        subsystem_calyx_error(error.code(), &error.to_string())
    }
}

impl From<calyx_lodestar::LodestarError> for CliError {
    fn from(error: calyx_lodestar::LodestarError) -> Self {
        subsystem_calyx_error(error.code(), &error.to_string())
    }
}

impl From<calyx_ward::WardError> for CliError {
    fn from(error: calyx_ward::WardError) -> Self {
        subsystem_calyx_error(error.code(), &error.to_string())
    }
}

impl From<calyx_paths::PathsError> for CliError {
    fn from(error: calyx_paths::PathsError) -> Self {
        subsystem_calyx_error(error.code(), &error.to_string())
    }
}

/// Builds a [`CliError::Calyx`] for a subsystem error that carries a stable
/// `CALYX_*` code but no `&'static` remediation, stripping the `"CODE: "`
/// prefix its `Display` duplicates into the text.
fn subsystem_calyx_error(code: &'static str, text: &str) -> CliError {
    let message = text
        .strip_prefix(code)
        .map(|rest| rest.trim_start_matches([':', ' ']).to_string())
        .unwrap_or_else(|| text.to_string());
    CliError::Calyx(CalyxError {
        code,
        message,
        remediation: CLI_SUBSYSTEM_REMEDIATION,
    })
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CliError {}

impl From<calyx_search::SearchError> for CliError {
    fn from(error: calyx_search::SearchError) -> Self {
        match error {
            calyx_search::SearchError::Calyx(inner) => CliError::Calyx(inner),
            calyx_search::SearchError::Io(message) => CliError::Io(message),
            calyx_search::SearchError::Usage(message) => CliError::Usage(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::error::CALYX_ERROR_CODES;
    use proptest::prelude::*;

    #[test]
    fn calyx_variant_serializes_to_exact_catalog_envelope() {
        let error = CliError::from(CalyxError::lens_dim_mismatch("got 384, expected 768"));

        assert_eq!(
            error.to_json(),
            r#"{"code":"CALYX_LENS_DIM_MISMATCH","message":"got 384, expected 768","remediation":"fix lens or slot shape"}"#
        );
    }

    #[test]
    fn calyx_envelope_is_byte_identical_to_calyx_error_serde() {
        // The CLI envelope must match what CalyxError emits on every other
        // surface, byte for byte (same fields, same order).
        let calyx = CalyxError::forge_vram_budget("requested 40GiB, budget 24GiB");
        let direct = serde_json::to_string(&calyx).expect("serialize CalyxError");
        let via_cli = CliError::from(calyx).to_json();

        assert_eq!(via_cli, direct);
    }

    #[test]
    fn usage_variant_uses_sentinel_code_and_nonempty_remediation() {
        let json = CliError::usage("bad arg").to_json();

        assert!(json.contains(r#""code":"CALYX_CLI_USAGE_ERROR""#), "{json}");
        assert!(json.contains(r#""message":"bad arg""#), "{json}");
        let error = CliError::usage("bad arg");
        assert!(!error.remediation().is_empty());
    }

    #[test]
    fn runtime_variant_uses_sentinel_code_and_nonempty_remediation() {
        let error = CliError::runtime("compaction scheduler panicked");
        let json = error.to_json();

        assert!(
            json.contains(r#""code":"CALYX_CLI_RUNTIME_ERROR""#),
            "{json}"
        );
        assert!(
            json.contains(r#""message":"compaction scheduler panicked""#),
            "{json}"
        );
        assert!(!error.remediation().is_empty());
        assert!(
            !error.remediation().contains("--help"),
            "runtime failures must not point operators at --help: {}",
            error.remediation()
        );
    }

    #[test]
    fn sentinel_codes_are_distinct_and_outside_the_catalog() {
        let codes = [
            CliError::usage("x").code(),
            CliError::io("x").code(),
            CliError::runtime("x").code(),
        ];
        for (index, code) in codes.iter().enumerate() {
            for other in &codes[index + 1..] {
                assert_ne!(code, other);
            }
            assert!(
                CALYX_ERROR_CODES.iter().all(|entry| entry.code() != *code),
                "CLI-local sentinel {code} must not shadow a PRD 18 catalog code"
            );
        }
    }

    #[test]
    fn io_variant_uses_sentinel_code_and_nonempty_remediation() {
        let json = CliError::io("No such file or directory (os error 2)").to_json();

        assert!(json.contains(r#""code":"CALYX_CLI_IO_ERROR""#), "{json}");
        assert!(!CliError::io("x").remediation().is_empty());
    }

    #[test]
    fn oracle_error_keeps_typed_code_and_remediation() {
        let oracle = calyx_oracle::OracleError::DomainNotFound;
        let expected_code = oracle.code();
        let expected_remediation = oracle.remediation();
        let cli = CliError::from(oracle);

        assert_eq!(cli.code(), expected_code);
        assert_eq!(cli.remediation(), expected_remediation);
        assert_ne!(cli.code(), CALYX_CLI_USAGE_ERROR);
        assert_ne!(cli.code(), CALYX_CLI_RUNTIME_ERROR);
    }

    #[test]
    fn forge_error_keeps_typed_code_with_subsystem_remediation() {
        let forge = calyx_forge::ForgeError::Unimplemented {
            op: "matmul".to_string(),
            remediation: "use the CPU path".to_string(),
        };
        let expected_code = forge.code();
        let cli = CliError::from(forge);

        assert_eq!(cli.code(), expected_code);
        assert_eq!(cli.remediation(), CLI_SUBSYSTEM_REMEDIATION);
        assert!(cli.message().contains("matmul"), "{}", cli.message());
        assert!(
            !cli.message().starts_with(expected_code),
            "code must not be duplicated into the message: {}",
            cli.message()
        );
    }

    #[test]
    fn from_io_error_maps_to_io_variant() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "vault.calyx missing");
        let cli: CliError = io_error.into();

        assert!(matches!(cli, CliError::Io(_)));
        assert_eq!(cli.code(), CALYX_CLI_IO_ERROR);
        assert!(cli.message().contains("vault.calyx missing"));
    }

    #[test]
    fn message_with_json_special_chars_is_escaped_not_corrupted() {
        // A message containing quotes/backslashes/newlines must remain valid
        // JSON so downstream parsers never choke — fail-closed on malformed.
        let raw = "path \"C:\\a\\b\" not found\nretry";
        let json = CliError::usage(raw).to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON envelope");

        assert_eq!(
            parsed["message"],
            serde_json::Value::String(raw.to_string())
        );
        assert_eq!(parsed["code"], "CALYX_CLI_USAGE_ERROR");
    }

    proptest! {
        /// Every PRD 18 catalog code round-trips through CliError preserving
        /// `code` and `remediation` verbatim.
        #[test]
        fn every_catalog_code_round_trips_through_cli_error(idx in 0usize..CALYX_ERROR_CODES.len()) {
            let code = CALYX_ERROR_CODES[idx];
            let calyx = code.error("synthetic detail");
            let cli = CliError::from(calyx);

            prop_assert_eq!(cli.code(), code.code());
            prop_assert_eq!(cli.remediation(), code.remediation());
            prop_assert_eq!(cli.message(), "synthetic detail");
        }
    }
}
