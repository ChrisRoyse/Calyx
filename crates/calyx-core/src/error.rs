//! Closed `CALYX_*` error catalog.
//!
//! This module contains only the PRD 18 cross-surface catalog. Subsystem-local
//! `CALYX_*` strings live beside their owning guard/type and build
//! [`CalyxError`] directly unless PRD 18 is amended in the same change.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Structured Calyx error payload for APIs, MCP, and agent remediation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{code}: {message}")]
pub struct CalyxError {
    /// Stable `CALYX_*` code.
    pub code: &'static str,
    /// Concrete failure details.
    pub message: String,
    /// Stable remediation text from PRD 18.
    pub remediation: &'static str,
}

/// Calyx result alias.
pub type Result<T> = std::result::Result<T, CalyxError>;

/// Non-fatal API warning for surfaces that must not be labeled trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CalyxWarning {
    Unprovenanced { surface: String },
}

impl CalyxWarning {
    pub fn unprovenanced(surface: impl Into<String>) -> Self {
        Self::Unprovenanced {
            surface: surface.into(),
        }
    }
}

impl CalyxError {
    /// Builds an error from a catalog code and concrete message.
    pub fn from_code(code: CalyxErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.code(),
            message: message.into(),
            remediation: code.remediation(),
        }
    }
}

macro_rules! error_catalog {
    ($(
        $variant:ident,
        $ctor:ident,
        $code:literal,
        $meaning:literal,
        $remediation:literal;
    )+) => {
        /// Closed set of PRD 18 Calyx error codes.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum CalyxErrorCode {
            $(
                #[doc = $meaning]
                $variant,
            )+
        }

        /// All Calyx error codes in PRD 18 order.
        pub const CALYX_ERROR_CODES: &[CalyxErrorCode] = &[
            $(CalyxErrorCode::$variant,)+
        ];

        impl CalyxErrorCode {
            /// Returns the stable wire code.
            pub const fn code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// Returns the PRD 18 meaning.
            pub const fn meaning(self) -> &'static str {
                match self {
                    $(Self::$variant => $meaning,)+
                }
            }

            /// Returns the PRD 18 remediation.
            pub const fn remediation(self) -> &'static str {
                match self {
                    $(Self::$variant => $remediation,)+
                }
            }

            /// Builds a structured error with this code.
            pub fn error(self, message: impl Into<String>) -> CalyxError {
                CalyxError::from_code(self, message)
            }
        }

        impl CalyxError {
            $(
                #[doc = concat!("Builds `", $code, "`.")]
                pub fn $ctor(message: impl Into<String>) -> Self {
                    CalyxErrorCode::$variant.error(message)
                }
            )+
        }
    };
}

error_catalog! {
    LensFrozenViolation, lens_frozen_violation, "CALYX_LENS_FROZEN_VIOLATION",
    "weights hash != registered", "re-register as new LensId";

    LensDimMismatch, lens_dim_mismatch, "CALYX_LENS_DIM_MISMATCH",
    "output dim != Slot.shape", "fix lens or slot shape";

    LensNumericalInvariant, lens_numerical_invariant, "CALYX_LENS_NUMERICAL_INVARIANT",
    "NaN/Inf/non-unit output", "check lens runtime/normalize";

    LensUnreachable, lens_unreachable, "CALYX_LENS_UNREACHABLE",
    "runtime endpoint down", "restore lens service";

    RegistryDuplicate, registry_duplicate, "CALYX_REGISTRY_DUPLICATE",
    "lens id already registered", "reuse existing LensId or register a distinct frozen spec";

    RegistryUnavailable, registry_unavailable, "CALYX_REGISTRY_UNAVAILABLE",
    "lens registry unavailable", "restore registry before guarded anneal update";

    AssayInsufficientSamples, assay_insufficient_samples, "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
    "< quorum (50) anchors", "anchor more outcomes";

    AssayLowSignal, assay_low_signal, "CALYX_ASSAY_LOW_SIGNAL",
    "lens < 0.05 bits", "park/retire lens";

    AssayRedundant, assay_redundant, "CALYX_ASSAY_REDUNDANT",
    "pair corr > 0.6", "drop duplicate lens";

    KernelUngrounded, kernel_ungrounded, "CALYX_KERNEL_UNGROUNDED",
    "kernel over ungrounded graph", "add anchors (grounding_gaps)";

    GuardProvisional, guard_provisional, "CALYX_GUARD_PROVISIONAL",
    "tau not calibrated", "calibrate before high-stakes use";

    GuardOod, guard_ood, "CALYX_GUARD_OOD",
    "query/output outside trusted region", "new-region or reject per policy";

    ForgeNumericalInvariant, forge_numerical_invariant, "CALYX_FORGE_NUMERICAL_INVARIANT",
    "kernel NaN/Inf", "numerical fail-closed";

    ForgeDeviceUnavailable, forge_device_unavailable, "CALYX_FORGE_DEVICE_UNAVAILABLE",
    "CUDA init failed (server mode)", "fix driver (reboot per gotcha)";

    AsterCorruptShard, aster_corrupt_shard, "CALYX_ASTER_CORRUPT_SHARD",
    "base shard hash mismatch", "restore from restic/snapshot";

    AsterTornWal, aster_torn_wal, "CALYX_ASTER_TORN_WAL",
    "torn tail on recovery", "auto-discarded; logged";

    LedgerChainBroken, ledger_chain_broken, "CALYX_LEDGER_CHAIN_BROKEN",
    "hash-chain verify failed", "quarantine range, investigate";

    LedgerCorrupt, ledger_corrupt, "CALYX_LEDGER_CORRUPT",
    "ledger CF integrity violation", "ledger CF integrity violation — run verify_chain to identify range";

    LedgerAppendOnlyViolation, ledger_append_only_violation, "CALYX_LEDGER_APPEND_ONLY_VIOLATION",
    "ledger CF append-only invariant violated", "ledger CF is append-only; deletes and tombstones are forbidden";

    LedgerSecretInPayload, ledger_secret_in_payload, "CALYX_LEDGER_SECRET_IN_PAYLOAD",
    "ledger payload contains secret-like material", "ledger payload must store hashes/ids only — redact before writing";

    LedgerActorTooLong, ledger_actor_too_long, "CALYX_LEDGER_ACTOR_TOO_LONG",
    "ledger actor id exceeds 64 UTF-8 bytes", "actor id must be <= 64 bytes UTF-8";

    LedgerGroupCommitFailed, ledger_group_commit_failed, "CALYX_LEDGER_GROUP_COMMIT_FAILED",
    "ledger hook failed during group commit", "ledger hook failed — group-commit rolled back; retry the write";

    ReproduceNondeterministic, reproduce_nondeterministic, "CALYX_REPRODUCE_NONDETERMINISTIC",
    "reproduce ledger entry lacks determinism seed", "no determinism seed in ledger entry - cannot guarantee reproduce fidelity";

    ReproduceDriftExceeded, reproduce_drift_exceeded, "CALYX_REPRODUCE_DRIFT_EXCEEDED",
    "reproduce max_drift exceeded tolerance", "reproduce max_drift exceeded 1e-3 - possible lens drift or fusion parameter change";

    VaultAccessDenied, vault_access_denied, "CALYX_VAULT_ACCESS_DENIED",
    "cross-vault read without grant", "request grant";

    EraseAlreadyTombstoned, erase_already_tombstoned, "CALYX_ERASE_ALREADY_TOMBSTONED",
    "erase scope already has an erasure tombstone", "treat as idempotent erasure or inspect ledger tombstone";

    StaleDerived, stale_derived, "CALYX_STALE_DERIVED",
    "fresh required, rebuild pending", "retry or accept StaleOk";

    OracleInsufficient, oracle_insufficient, "CALYX_ORACLE_INSUFFICIENT",
    "I(panel;oracle) < H(Y) - panel can't predict",
    "add outcome/execution lens (propose_lens)";

    ForgeVramBudget, forge_vram_budget, "CALYX_FORGE_VRAM_BUDGET",
    "dispatch exceeds VRAM budget", "split batch / raise budget / wait";

    Backpressure, backpressure, "CALYX_BACKPRESSURE",
    "write/query queue at high-water", "retry with backoff";

    DiskPressure, disk_pressure, "CALYX_DISK_PRESSURE",
    "hotpool near full", "free/spill to archive; writes fail-closed";

    QuantIntelligenceLoss, quant_intelligence_loss, "CALYX_QUANT_INTELLIGENCE_LOSS",
    "quant level would drop bits/cosine/FAR beyond bound", "use a gentler level (A25)";

    ReaderLeaseExpired, reader_lease_expired, "CALYX_READER_LEASE_EXPIRED",
    "long reader aborted to release MVCC version", "re-issue with bounded-staleness snapshot";

    DatasetNotFound, dataset_not_found, "CALYX_DATASET_NOT_FOUND",
    "dataset dir or MANIFEST row missing", "acquire + register via scripts/acquire_datasets.sh";

    DatasetChecksumMismatch, dataset_checksum_mismatch, "CALYX_DATASET_CHECKSUM_MISMATCH",
    "recomputed sha256 != recorded value", "re-acquire at the pinned revision; never edit dataset bytes in place";

    DatasetRowcountMismatch, dataset_rowcount_mismatch, "CALYX_DATASET_ROWCOUNT_MISMATCH",
    "recomputed row count != recorded value", "re-acquire at the pinned revision; check split/decoder drift";

    DatasetManifestInvalid, dataset_manifest_invalid, "CALYX_DATASET_MANIFEST_INVALID",
    "MANIFEST.md or manifest.json missing/malformed/drifted", "re-register via scripts/verify_dataset.sh register";

    DatasetSchemaMismatch, dataset_schema_mismatch, "CALYX_DATASET_SCHEMA_MISMATCH",
    "dataset columns/fields missing or malformed vs the pinned upstream contract",
    "re-acquire at the pinned revision; check upstream schema drift";
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRD_18_CODES: &[&str] = &[
        "CALYX_LENS_FROZEN_VIOLATION",
        "CALYX_LENS_DIM_MISMATCH",
        "CALYX_LENS_NUMERICAL_INVARIANT",
        "CALYX_LENS_UNREACHABLE",
        "CALYX_REGISTRY_DUPLICATE",
        "CALYX_REGISTRY_UNAVAILABLE",
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
        "CALYX_ASSAY_LOW_SIGNAL",
        "CALYX_ASSAY_REDUNDANT",
        "CALYX_KERNEL_UNGROUNDED",
        "CALYX_GUARD_PROVISIONAL",
        "CALYX_GUARD_OOD",
        "CALYX_FORGE_NUMERICAL_INVARIANT",
        "CALYX_FORGE_DEVICE_UNAVAILABLE",
        "CALYX_ASTER_CORRUPT_SHARD",
        "CALYX_ASTER_TORN_WAL",
        "CALYX_LEDGER_CHAIN_BROKEN",
        "CALYX_LEDGER_CORRUPT",
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION",
        "CALYX_LEDGER_SECRET_IN_PAYLOAD",
        "CALYX_LEDGER_ACTOR_TOO_LONG",
        "CALYX_LEDGER_GROUP_COMMIT_FAILED",
        "CALYX_REPRODUCE_NONDETERMINISTIC",
        "CALYX_REPRODUCE_DRIFT_EXCEEDED",
        "CALYX_VAULT_ACCESS_DENIED",
        "CALYX_ERASE_ALREADY_TOMBSTONED",
        "CALYX_STALE_DERIVED",
        "CALYX_ORACLE_INSUFFICIENT",
        "CALYX_FORGE_VRAM_BUDGET",
        "CALYX_BACKPRESSURE",
        "CALYX_DISK_PRESSURE",
        "CALYX_QUANT_INTELLIGENCE_LOSS",
        "CALYX_READER_LEASE_EXPIRED",
        "CALYX_DATASET_NOT_FOUND",
        "CALYX_DATASET_CHECKSUM_MISMATCH",
        "CALYX_DATASET_ROWCOUNT_MISMATCH",
        "CALYX_DATASET_MANIFEST_INVALID",
        "CALYX_DATASET_SCHEMA_MISMATCH",
    ];

    const MODULE_LOCAL_CODES: &[&str] = &[
        "CALYX_TEMPORAL_AP60_VIOLATION",
        "CALYX_TEMPORAL_INVALID_BOOST_CONFIG",
        "CALYX_TEMPORAL_INVALID_PERIOD",
        "CALYX_TEMPORAL_INVALID_WINDOW",
        "CALYX_TEMPORAL_NEGATIVE_WEIGHT",
        "CALYX_TEMPORAL_WEIGHT_SUM",
        "CALYX_RECORD_SCHEMA_VIOLATION",
        "CALYX_MCP_JSONRPC_INVALID",
        "CALYX_AUTHN_REQUIRED",
        "CALYX_TLS_CONFIG_INVALID",
        "CALYX_CONSENT_VIOLATION",
        "CALYX_PII_REDACTION_REQUIRED",
        "CALYX_SBOM_PARSE_ERROR",
        "CALYX_SUPPLY_CHAIN_VULN",
        "CALYX_LENS_WEIGHT_TAMPERED",
        "CALYX_EXTERNAL_CMD_NOT_ALLOWED",
        "CALYX_PROVISIONAL_VAULT",
    ];

    #[test]
    fn catalog_matches_prd_18_exactly() {
        let actual: Vec<_> = CALYX_ERROR_CODES.iter().map(|code| code.code()).collect();
        assert_eq!(actual, PRD_18_CODES);
    }

    #[test]
    fn module_local_codes_are_not_prd_18_catalog_entries() {
        let catalog: Vec<_> = CALYX_ERROR_CODES.iter().map(|code| code.code()).collect();
        for code in MODULE_LOCAL_CODES {
            assert!(
                !catalog.contains(code),
                "{code} is module-local unless PRD 18 is amended"
            );
        }
    }

    #[test]
    fn remediation_text_matches_prd_18() {
        let pairs: Vec<_> = CALYX_ERROR_CODES
            .iter()
            .map(|code| (code.code(), code.remediation()))
            .collect();

        assert_eq!(
            pairs[0],
            ("CALYX_LENS_FROZEN_VIOLATION", "re-register as new LensId")
        );
        assert!(pairs.contains(&(
            "CALYX_REGISTRY_DUPLICATE",
            "reuse existing LensId or register a distinct frozen spec"
        )));
        assert!(pairs.contains(&(
            "CALYX_REGISTRY_UNAVAILABLE",
            "restore registry before guarded anneal update"
        )));
        assert!(pairs.contains(&(
            "CALYX_ORACLE_INSUFFICIENT",
            "add outcome/execution lens (propose_lens)"
        )));
        assert!(pairs.contains(&(
            "CALYX_READER_LEASE_EXPIRED",
            "re-issue with bounded-staleness snapshot"
        )));
        assert!(pairs.contains(&(
            "CALYX_LEDGER_APPEND_ONLY_VIOLATION",
            "ledger CF is append-only; deletes and tombstones are forbidden"
        )));
        assert!(pairs.contains(&(
            "CALYX_LEDGER_SECRET_IN_PAYLOAD",
            "ledger payload must store hashes/ids only — redact before writing"
        )));
        assert!(pairs.contains(&(
            "CALYX_LEDGER_ACTOR_TOO_LONG",
            "actor id must be <= 64 bytes UTF-8"
        )));
        assert!(pairs.contains(&(
            "CALYX_LEDGER_GROUP_COMMIT_FAILED",
            "ledger hook failed — group-commit rolled back; retry the write"
        )));
        assert!(pairs.contains(&(
            "CALYX_REPRODUCE_NONDETERMINISTIC",
            "no determinism seed in ledger entry - cannot guarantee reproduce fidelity"
        )));
        assert!(pairs.contains(&(
            "CALYX_REPRODUCE_DRIFT_EXCEEDED",
            "reproduce max_drift exceeded 1e-3 - possible lens drift or fusion parameter change"
        )));
        assert!(pairs.contains(&(
            "CALYX_DATASET_NOT_FOUND",
            "acquire + register via scripts/acquire_datasets.sh"
        )));
        assert!(pairs.contains(&(
            "CALYX_DATASET_CHECKSUM_MISMATCH",
            "re-acquire at the pinned revision; never edit dataset bytes in place"
        )));
        assert!(pairs.contains(&(
            "CALYX_DATASET_ROWCOUNT_MISMATCH",
            "re-acquire at the pinned revision; check split/decoder drift"
        )));
        assert!(pairs.contains(&(
            "CALYX_DATASET_MANIFEST_INVALID",
            "re-register via scripts/verify_dataset.sh register"
        )));
        assert!(pairs.contains(&(
            "CALYX_DATASET_SCHEMA_MISMATCH",
            "re-acquire at the pinned revision; check upstream schema drift"
        )));
    }

    #[test]
    fn error_serializes_to_wire_shape() {
        let error = CalyxError::lens_dim_mismatch("got 384, expected 768");
        let bytes = serde_json::to_vec(&error).expect("serialize error");

        assert_eq!(
            bytes,
            br#"{"code":"CALYX_LENS_DIM_MISMATCH","message":"got 384, expected 768","remediation":"fix lens or slot shape"}"#
        );
    }

    #[test]
    fn constructors_attach_prd_remediation() {
        let error = CalyxError::forge_vram_budget("requested 40GiB, budget 24GiB");

        assert_eq!(error.code, "CALYX_FORGE_VRAM_BUDGET");
        assert_eq!(error.remediation, "split batch / raise budget / wait");
        assert!(error.to_string().contains("requested 40GiB"));
    }
}
