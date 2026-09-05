//! Audited rebinding of a frozen TEI lens endpoint to a different live lane.
//!
//! A `FrozenLensContract` hashes the endpoint string into the `LensId`, so the
//! address a lens was commissioned against is part of its identity. That is a
//! sound default -- it stops a lens silently becoming a measurement of some
//! other service -- but it also means a lens can never be moved off a host:port
//! without invalidating every slot vector ever measured through it.
//!
//! #2053 made that cost concrete. Six of the fifteen frozen content lenses in
//! the production law vault were commissioned against ports owned by the
//! LeapableMemory TEI fleet, a different project that redeploys those
//! containers on its own schedule. When that fleet came back advertising
//! `max_client_batch_size = 1`, every one of those lenses failed closed and
//! leapablelaw.com lost its doctrine kernel. Re-embedding 109,133 opinions
//! through six lenses to change six port numbers is not a proportionate
//! remedy.
//!
//! What the endpoint hash is actually standing in for is *instrument identity*:
//! the claim that the thing answering now is the same measuring device that
//! produced the corpus. Calyx already holds a far stronger witness for that
//! claim than an address -- the persisted runtime golden, which replays a
//! frozen probe input and requires the observed vector to reproduce within
//! `PROCESS_RUNTIME_GOLDEN_TOLERANCE`. A relocation is therefore admitted only
//! on three independent proofs, in increasing order of strength:
//!
//! 1. an explicit operator-declared record naming both endpoints (intent),
//! 2. the live `/info` bytes matching the model id, revision, dtype and
//!    `max_input_length` declared in that record (physical identity), and
//! 3. the persisted runtime golden reproducing on the relocated lane, which is
//!    enforced by [`crate::persistence`] and is the binding proof.
//!
//! Anything short of all three fails closed. There is no default relocation,
//! no wildcard, and no "try the old endpoint if the new one is down".

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::runtime::tei_http::read_tei_service_info;

pub const CALYX_LENS_RELOCATION_REJECTED: &str = "CALYX_LENS_RELOCATION_REJECTED";

/// Path to the relocation table. Absent means no lens may be relocated.
pub const ENDPOINT_RELOCATION_ENV: &str = "CALYX_LENS_ENDPOINT_RELOCATION";

const RELOCATION_SCHEMA: &str = "calyx-lens-endpoint-relocation-v1";

/// One audited rebinding of a frozen endpoint onto a live lane.
///
/// The model fields are not decoration: they are the operator's declaration of
/// which physical instrument the relocated lane is required to be, and they are
/// checked against the live `/info` bytes before the lens is admitted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct EndpointRelocation {
    /// Endpoint hashed into the frozen contract, and therefore into the LensId.
    pub frozen_endpoint: String,
    /// Endpoint the runtime must actually dial.
    pub live_endpoint: String,
    /// `/info.model_id` the relocated lane is required to serve.
    pub model_id: String,
    /// `/info.model_sha` the relocated lane is required to serve.
    pub model_sha: String,
    /// `/info.model_dtype` the relocated lane is required to serve.
    pub model_dtype: String,
    /// `/info.max_input_length` the relocated lane is required to advertise.
    pub max_input_length: usize,
    /// Free-text audit note: why this relocation exists.
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize)]
struct RawRelocationTable {
    schema: String,
    relocations: Vec<EndpointRelocation>,
}

/// A loaded, validated relocation table plus the identity of the bytes it came
/// from, so a report can name exactly which table admitted a rebinding.
#[derive(Clone, Debug)]
pub struct EndpointRelocationTable {
    source: PathBuf,
    source_sha256: String,
    relocations: Vec<EndpointRelocation>,
}

impl EndpointRelocationTable {
    /// Reads and validates a relocation table.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            relocation_rejected(format!(
                "read lens endpoint relocation table {} failed: {error}",
                path.display()
            ))
        })?;
        let source_sha256 = sha256_hex(&bytes);
        let raw: RawRelocationTable = serde_json::from_slice(&bytes).map_err(|error| {
            relocation_rejected(format!(
                "lens endpoint relocation table {} is not valid {RELOCATION_SCHEMA} JSON: {error}",
                path.display()
            ))
        })?;
        if raw.schema != RELOCATION_SCHEMA {
            return Err(relocation_rejected(format!(
                "lens endpoint relocation table {} declares schema {} but only {RELOCATION_SCHEMA} is accepted",
                path.display(),
                raw.schema
            )));
        }

        let mut seen = BTreeSet::new();
        for record in &raw.relocations {
            for (field, value) in [
                ("frozen_endpoint", record.frozen_endpoint.as_str()),
                ("live_endpoint", record.live_endpoint.as_str()),
                ("model_id", record.model_id.as_str()),
                ("model_sha", record.model_sha.as_str()),
                ("model_dtype", record.model_dtype.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(relocation_rejected(format!(
                        "lens endpoint relocation for {} has an empty {field}; a relocation must name the exact instrument it is rebinding to",
                        record.frozen_endpoint
                    )));
                }
            }
            if record.max_input_length == 0 {
                return Err(relocation_rejected(format!(
                    "lens endpoint relocation for {} declares max_input_length 0",
                    record.frozen_endpoint
                )));
            }
            // A record that rebinds an endpoint onto itself is always an
            // operator mistake, and admitting it would let a table look like it
            // is doing something while changing nothing.
            if record.frozen_endpoint == record.live_endpoint {
                return Err(relocation_rejected(format!(
                    "lens endpoint relocation for {} rebinds the endpoint onto itself",
                    record.frozen_endpoint
                )));
            }
            if !seen.insert(record.frozen_endpoint.clone()) {
                return Err(relocation_rejected(format!(
                    "lens endpoint relocation table {} maps frozen endpoint {} more than once; a frozen endpoint has exactly one live lane",
                    path.display(),
                    record.frozen_endpoint
                )));
            }
        }

        Ok(Self {
            source: path.to_path_buf(),
            source_sha256,
            relocations: raw.relocations,
        })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn len(&self) -> usize {
        self.relocations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.relocations.is_empty()
    }

    pub fn records(&self) -> &[EndpointRelocation] {
        &self.relocations
    }

    /// Returns the relocation for a frozen endpoint, if one is declared.
    pub fn get(&self, frozen_endpoint: &str) -> Option<&EndpointRelocation> {
        self.relocations
            .iter()
            .find(|record| record.frozen_endpoint == frozen_endpoint)
    }
}

type TableCell = std::result::Result<Option<EndpointRelocationTable>, CalyxError>;

static TABLE: OnceLock<TableCell> = OnceLock::new();

/// Loads the process relocation table once from [`ENDPOINT_RELOCATION_ENV`].
///
/// An unset variable means no lens may be relocated, which is the correct
/// default for every process that has not been deliberately configured for it.
/// A set-but-unreadable variable is an error and never degrades to "no
/// relocations": that would silently send a relocated lens back to the foreign
/// endpoint the relocation existed to escape.
pub fn process_relocation_table() -> Result<Option<&'static EndpointRelocationTable>> {
    let cell = TABLE.get_or_init(|| {
        let loaded = match std::env::var_os(ENDPOINT_RELOCATION_ENV) {
            None => Ok(None),
            Some(raw) if raw.is_empty() => Err(relocation_rejected(format!(
                "{ENDPOINT_RELOCATION_ENV} is set but empty; unset it to forbid relocation or point it at a {RELOCATION_SCHEMA} table"
            ))),
            Some(raw) => EndpointRelocationTable::load(Path::new(&raw)).map(Some),
        };
        // Emitted exactly once per process, from inside the OnceLock, so a
        // report can always name which table bytes were in force. A relocation
        // that is not visible in the log is a relocation the evidence trail
        // cannot account for.
        match &loaded {
            Ok(None) => eprintln!("CALYX_LENS_RELOCATION phase=table_absent relocations=0"),
            Ok(Some(table)) => eprintln!(
                "CALYX_LENS_RELOCATION phase=table_loaded source={} source_sha256={} relocations={}",
                table.source.display(),
                table.source_sha256,
                table.relocations.len()
            ),
            Err(error) => eprintln!(
                "CALYX_LENS_RELOCATION phase=table_rejected code={} message={}",
                error.code, error.message
            ),
        }
        loaded
    });
    match cell {
        Ok(table) => Ok(table.as_ref()),
        Err(error) => Err(error.clone()),
    }
}

/// Resolves the endpoint a TEI runtime must dial for a frozen endpoint.
///
/// Returns the relocation record when one is declared, so the caller can keep
/// deriving the frozen contract from the original address while dialing the
/// live one.
pub fn resolve(frozen_endpoint: &str) -> Result<Option<&'static EndpointRelocation>> {
    Ok(process_relocation_table()?.and_then(|table| table.get(frozen_endpoint)))
}

/// Proves that the relocated lane is the instrument the record declares.
///
/// This runs before the lens is handed back, so a mis-pointed relocation fails
/// with the exact mismatched field instead of surfacing later as an opaque
/// golden-probe drift.
pub fn verify_relocated_service(record: &EndpointRelocation) -> Result<()> {
    let info = read_tei_service_info(&record.live_endpoint).map_err(|error| {
        relocation_rejected(format!(
            "relocated lane {} for frozen endpoint {} is unreadable: {}: {}",
            record.live_endpoint, record.frozen_endpoint, error.code, error.message
        ))
    })?;

    let mut mismatches = Vec::new();
    if info.model_id != record.model_id {
        mismatches.push(format!(
            "model_id declared={} live={}",
            record.model_id, info.model_id
        ));
    }
    if info.model_sha != record.model_sha {
        mismatches.push(format!(
            "model_sha declared={} live={}",
            record.model_sha, info.model_sha
        ));
    }
    if info.model_dtype != record.model_dtype {
        mismatches.push(format!(
            "model_dtype declared={} live={}",
            record.model_dtype, info.model_dtype
        ));
    }
    if info.max_input_length != record.max_input_length {
        mismatches.push(format!(
            "max_input_length declared={} live={}",
            record.max_input_length, info.max_input_length
        ));
    }
    if !mismatches.is_empty() {
        return Err(relocation_rejected(format!(
            "relocated lane {} is not the instrument declared for frozen endpoint {}: {}",
            record.live_endpoint,
            record.frozen_endpoint,
            mismatches.join(", ")
        )));
    }
    // The identity proof that was actually observed, not the one that was
    // declared: model_identity/runtime_identity come off the live /info bytes.
    eprintln!(
        "CALYX_LENS_RELOCATION phase=identity_proven frozen_endpoint={} live_endpoint={} model_identity={} runtime_identity={} max_input_length={}",
        record.frozen_endpoint,
        record.live_endpoint,
        info.model_identity(),
        info.runtime_identity(),
        info.max_input_length
    );
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn relocation_rejected(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LENS_RELOCATION_REJECTED,
        message: message.into(),
        remediation: "point CALYX_LENS_ENDPOINT_RELOCATION at a table whose live lane serves the exact declared model, revision, dtype and max_input_length",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_json(body: &str) -> Vec<u8> {
        body.as_bytes().to_vec()
    }

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn loads_and_indexes_a_valid_table() {
        let path = write_temp(
            "calyx-reloc-valid.json",
            &table_json(
                r#"{"schema":"calyx-lens-endpoint-relocation-v1","relocations":[
                    {"frozen_endpoint":"http://127.0.0.1:8097/embed",
                     "live_endpoint":"http://127.0.0.1:18097/embed",
                     "model_id":"jinaai/jina-embeddings-v2-base-code",
                     "model_sha":"516f4baf13dec4ddddda8631e019b5737c8bc250",
                     "model_dtype":"float16","max_input_length":8192}]}"#,
            ),
        );

        let table = EndpointRelocationTable::load(&path).unwrap();

        assert_eq!(table.len(), 1);
        assert_eq!(
            table
                .get("http://127.0.0.1:8097/embed")
                .unwrap()
                .live_endpoint,
            "http://127.0.0.1:18097/embed"
        );
        assert!(table.get("http://127.0.0.1:9999/embed").is_none());
        assert_eq!(table.source_sha256().len(), 64);
    }

    #[test]
    fn rejects_a_foreign_schema() {
        let path = write_temp(
            "calyx-reloc-schema.json",
            &table_json(r#"{"schema":"something-else","relocations":[]}"#),
        );

        let error = EndpointRelocationTable::load(&path).unwrap_err();

        println!("RELOCATION_SCHEMA_ERROR={}", error.code);
        assert_eq!(error.code, CALYX_LENS_RELOCATION_REJECTED);
    }

    #[test]
    fn rejects_a_self_rebinding_record() {
        let path = write_temp(
            "calyx-reloc-self.json",
            &table_json(
                r#"{"schema":"calyx-lens-endpoint-relocation-v1","relocations":[
                    {"frozen_endpoint":"http://127.0.0.1:8097/embed",
                     "live_endpoint":"http://127.0.0.1:8097/embed",
                     "model_id":"m","model_sha":"s","model_dtype":"float16",
                     "max_input_length":8192}]}"#,
            ),
        );

        let error = EndpointRelocationTable::load(&path).unwrap_err();

        println!("RELOCATION_SELF_ERROR={}", error.message);
        assert_eq!(error.code, CALYX_LENS_RELOCATION_REJECTED);
    }

    #[test]
    fn rejects_a_duplicated_frozen_endpoint() {
        let path = write_temp(
            "calyx-reloc-dup.json",
            &table_json(
                r#"{"schema":"calyx-lens-endpoint-relocation-v1","relocations":[
                    {"frozen_endpoint":"http://127.0.0.1:8097/embed",
                     "live_endpoint":"http://127.0.0.1:18097/embed",
                     "model_id":"m","model_sha":"s","model_dtype":"float16",
                     "max_input_length":8192},
                    {"frozen_endpoint":"http://127.0.0.1:8097/embed",
                     "live_endpoint":"http://127.0.0.1:18098/embed",
                     "model_id":"m","model_sha":"s","model_dtype":"float16",
                     "max_input_length":8192}]}"#,
            ),
        );

        let error = EndpointRelocationTable::load(&path).unwrap_err();

        println!("RELOCATION_DUP_ERROR={}", error.message);
        assert_eq!(error.code, CALYX_LENS_RELOCATION_REJECTED);
    }

    #[test]
    fn rejects_an_empty_declared_field() {
        let path = write_temp(
            "calyx-reloc-empty.json",
            &table_json(
                r#"{"schema":"calyx-lens-endpoint-relocation-v1","relocations":[
                    {"frozen_endpoint":"http://127.0.0.1:8097/embed",
                     "live_endpoint":"http://127.0.0.1:18097/embed",
                     "model_id":"","model_sha":"s","model_dtype":"float16",
                     "max_input_length":8192}]}"#,
            ),
        );

        let error = EndpointRelocationTable::load(&path).unwrap_err();

        println!("RELOCATION_EMPTY_ERROR={}", error.message);
        assert_eq!(error.code, CALYX_LENS_RELOCATION_REJECTED);
    }
}
