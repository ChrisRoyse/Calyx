mod handlers;
mod policy;
mod recovery;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex};

use crate::accounts::{Account, lookup_account, lookup_group_gid};
use crate::broker_error::BrokerError;
use crate::config::ValidatedConfig;
use crate::exec_root::ExecutionRoot;
use crate::fs_tx::{FsRoot, FsRootSpec, PublishedObject};
use crate::journal::Journal;
use crate::pidfd::OwnerLease;
use crate::protocol::{
    ExecutionRootAlias, LeafName, ObjectId, RootAlias, RunId, RunToken, StageId,
};
use crate::systemd::AbortAuthority;
use crate::transport::SeqpacketListener;

pub use policy::load_config;

const MAX_CONNECTION_THREADS: usize = 128;

pub struct Broker {
    pub(super) config: ValidatedConfig,
    pub(super) journal: Mutex<Journal>,
    pub(super) roots: BTreeMap<RootAlias, Arc<FsRoot>>,
    pub(super) execution_roots: BTreeMap<ExecutionRootAlias, Arc<ExecutionRoot>>,
    pub(super) worker: Account,
    pub(super) client_group_gid: u32,
    pub(super) broker_cgroup: crate::protocol::AbsolutePath,
    pub(super) runs: Mutex<HashMap<RunId, Arc<RunRuntime>>>,
    pub(super) fatal: AtomicBool,
    connections: AtomicUsize,
}

pub(super) struct RunRuntime {
    pub id: RunId,
    pub token: RunToken,
    pub owner_uid: u32,
    pub owner: Arc<OwnerLease>,
    pub operation: Mutex<()>,
    pub objects: Mutex<BTreeMap<ObjectId, LiveObject>>,
    pub stage: (Mutex<Option<LiveStage>>, Condvar),
    pub aborting: AtomicBool,
}

#[derive(Clone)]
pub(super) struct LiveObject {
    pub root_alias: RootAlias,
    pub leaf: LeafName,
    pub published: PublishedObject,
}

pub(super) struct LiveStage {
    pub id: StageId,
    pub authority: Arc<AbortAuthority>,
}

impl Broker {
    pub fn open(config: ValidatedConfig) -> Result<Arc<Self>, BrokerError> {
        let root = policy::require_root_broker()?;
        policy::validate_state_paths(&config)?;
        let worker = lookup_account(&config.raw().worker_user).map_err(|error| {
            BrokerError::new(
                crate::protocol::StableCode::ConfigInvalid,
                error.to_string(),
                "Provision the locked worker with the checked-in sysusers policy.",
            )
        })?;
        if worker.uid == root.uid || worker.gid == root.gid {
            return Err(BrokerError::new(
                crate::protocol::StableCode::ConfigInvalid,
                format!("worker {} is not distinct from broker uid/gid", worker.name),
                "Recreate the locked worker with a dedicated uid and primary gid.",
            ));
        }
        let client_group_gid = lookup_group_gid(&config.raw().client_group).map_err(|error| {
            BrokerError::new(
                crate::protocol::StableCode::ConfigInvalid,
                error.to_string(),
                "Provision the configured client group with the checked-in sysusers policy.",
            )
        })?;
        if client_group_gid == worker.gid {
            return Err(BrokerError::new(
                crate::protocol::StableCode::ConfigInvalid,
                "worker primary group equals the broker client group",
                "Use distinct worker and client groups.",
            ));
        }

        let mut roots = BTreeMap::new();
        for (alias, configured) in config.roots() {
            let raw = configured.raw();
            let shared_owner = lookup_account(&raw.shared_owner)
                .map_err(|error| BrokerError::system("resolve shared owner", error))?;
            let private_owner = lookup_account(&raw.private_owner)
                .map_err(|error| BrokerError::system("resolve private owner", error))?;
            if shared_owner.uid != root.uid
                || shared_owner.gid != root.gid
                || private_owner.uid != root.uid
                || private_owner.gid != root.gid
            {
                return Err(BrokerError::new(
                    crate::protocol::StableCode::ConfigInvalid,
                    format!("managed root {alias} is not assigned to root:root"),
                    "All shared containers and quarantine roots must be root:root.",
                ));
            }
            let fs_root = FsRoot::open(FsRootSpec {
                alias: alias.clone(),
                common_ancestor: raw.common_ancestor.clone(),
                shared_path: raw.shared.clone(),
                private_path: raw.private.clone(),
                broker_uid: root.uid,
                broker_gid: root.gid,
                published_uid: worker.uid,
                published_gid: worker.gid,
                shared_mode: configured.shared_mode(),
                private_mode: configured.private_mode(),
                published_mode: configured.published_mode(),
            })
            .map_err(|error| {
                BrokerError::new(
                    crate::protocol::StableCode::CapabilityUnavailable,
                    format!("open managed root {alias}: {error}"),
                    "Repair the exact root policy or provide openat2, renameat2, and opaque file-handle capabilities.",
                )
            })?;
            roots.insert(alias.clone(), Arc::new(fs_root));
        }

        let mut execution_roots = BTreeMap::new();
        for (alias, configured) in config.execution_roots() {
            let owner = lookup_account(&configured.raw().expected_owner)
                .map_err(|error| BrokerError::system("resolve execution-root owner", error))?;
            let opened = ExecutionRoot::open(
                alias.clone(),
                configured.raw().path.clone(),
                owner.uid,
                configured.expected_mode(),
            )
            .map_err(|error| {
                BrokerError::new(
                    crate::protocol::StableCode::CapabilityUnavailable,
                    format!("open execution root {alias}: {error}"),
                    "Repair the exact source-root policy; symlink or canonical-path fallbacks are forbidden.",
                )
            })?;
            execution_roots.insert(alias.clone(), Arc::new(opened));
        }

        let journal = Journal::open(&config.raw().journal_path)
            .map_err(|error| BrokerError::journal("startup", error))?;
        let broker = Arc::new(Self {
            broker_cgroup: policy::broker_cgroup()?,
            config,
            journal: Mutex::new(journal),
            roots,
            execution_roots,
            worker,
            client_group_gid,
            runs: Mutex::new(HashMap::new()),
            fatal: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
        });
        recovery::replay(&broker)?;
        Ok(broker)
    }

    pub fn verify(config: ValidatedConfig) -> Result<(), BrokerError> {
        let broker = Self::open(config)?;
        if !broker.runs.lock().map_err(|_| poisoned("runs"))?.is_empty() {
            return Err(BrokerError::new(
                crate::protocol::StableCode::RecoveryRequired,
                "verification unexpectedly reconstructed an active run",
                "Inspect the journal before starting the service.",
            ));
        }
        Ok(())
    }

    pub fn serve(self: Arc<Self>, listener: SeqpacketListener) -> Result<(), BrokerError> {
        listener
            .verify_bound_path(&self.config.raw().socket_path)
            .map_err(|error| BrokerError::system("verify activated socket path", error))?;
        loop {
            if self.fatal.load(std::sync::atomic::Ordering::Acquire) {
                return Err(BrokerError::new(
                    crate::protocol::StableCode::RecoveryRequired,
                    "broker entered fatal recovery-required state",
                    "Inspect structured logs and the SQLite/filesystem source of truth.",
                ));
            }
            let connection = listener
                .accept()
                .map_err(|error| BrokerError::system("accept control connection", error))?;
            if self
                .connections
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                >= MAX_CONNECTION_THREADS
            {
                self.connections
                    .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                drop(connection);
                continue;
            }
            let broker = Arc::clone(&self);
            std::thread::spawn(move || {
                handlers::serve_connection(&broker, connection);
                broker
                    .connections
                    .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            });
        }
    }
}

pub(super) fn poisoned(name: &str) -> BrokerError {
    BrokerError::new(
        crate::protocol::StableCode::Internal,
        format!("broker {name} lock was poisoned"),
        "Inspect the preceding panic and restart only after source-of-truth verification.",
    )
    .fatal()
}
