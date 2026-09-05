use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::RootAlias;
#[cfg(target_os = "linux")]
use crate::protocol::{LeafName, ObjectId};

pub const MAX_OPAQUE_HANDLE_BYTES: usize = 128;
pub const MAX_DELETE_DEPTH: usize = 128;
pub const MAX_DELETE_ENTRIES: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct FsRootSpec {
    pub alias: RootAlias,
    pub common_ancestor: PathBuf,
    pub shared_path: PathBuf,
    pub private_path: PathBuf,
    pub broker_uid: u32,
    pub broker_gid: u32,
    pub published_uid: u32,
    pub published_gid: u32,
    pub shared_mode: u32,
    pub private_mode: u32,
    pub published_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueHandle {
    mount_id: i32,
    handle_type: i32,
    bytes: Vec<u8>,
}

impl OpaqueHandle {
    pub fn new(mount_id: i32, handle_type: i32, bytes: Vec<u8>) -> Result<Self, FsTxError> {
        if bytes.is_empty() || bytes.len() > MAX_OPAQUE_HANDLE_BYTES {
            return Err(FsTxError::InvalidSpec(format!(
                "opaque handle length {} is outside 1..={MAX_OPAQUE_HANDLE_BYTES}",
                bytes.len()
            )));
        }
        Ok(Self {
            mount_id,
            handle_type,
            bytes,
        })
    }

    pub fn mount_id(&self) -> i32 {
        self.mount_id
    }

    pub fn handle_type(&self) -> i32 {
        self.handle_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectIdentity {
    pub device: u64,
    pub inode: u64,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub mode: u32,
    pub opaque: OpaqueHandle,
}

impl ObjectIdentity {
    pub fn same_authority(&self, other: &Self) -> bool {
        self.opaque == other.opaque
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MismatchDisposition {
    UnchangedShared,
    RestoredShared,
    PreservedPrivate { quarantine_name: String },
    PreservedOpenHandle,
}

#[derive(Debug, Error)]
pub enum FsTxError {
    #[error("invalid filesystem root specification: {0}")]
    InvalidSpec(String),
    #[error("required Linux capability {capability} is unavailable: {detail}")]
    CapabilityUnavailable {
        capability: &'static str,
        detail: String,
    },
    #[error("filesystem operation {operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("filesystem object already exists at {0}")]
    Collision(String),
    #[error(
        "object identity mismatch at {path}; disposition={disposition:?}; expected={expected:?}; observed={observed:?}"
    )]
    IdentityMismatch {
        path: String,
        expected: Box<ObjectIdentity>,
        observed: Box<ObjectIdentity>,
        disposition: MismatchDisposition,
    },
    #[error("delete traversal exceeded {limit_name}={limit}")]
    DeleteLimit {
        limit_name: &'static str,
        limit: usize,
    },
    #[error("{primary}; exact rollback also failed: {cleanup}")]
    RollbackFailed {
        #[source]
        primary: Box<FsTxError>,
        cleanup: String,
    },
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{FsRoot, PreparedObject, PublishedObject, QuarantinedObject};

#[cfg(not(target_os = "linux"))]
mod unsupported {
    use super::*;

    #[derive(Debug)]
    pub struct FsRoot;
    #[derive(Debug)]
    pub struct PreparedObject;
    #[derive(Debug)]
    pub struct PublishedObject;
    #[derive(Debug)]
    pub struct QuarantinedObject;

    impl FsRoot {
        pub fn open(_spec: FsRootSpec) -> Result<Self, FsTxError> {
            Err(FsTxError::CapabilityUnavailable {
                capability: "Linux descriptor-relative filesystem authority",
                detail: "calyx-gatebrokerd filesystem transactions require Linux".into(),
            })
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub use unsupported::{FsRoot, PreparedObject, PublishedObject, QuarantinedObject};
