mod handlers;
mod policy;
mod recovery;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
use crate::transport::{SeqpacketConnection, SeqpacketListener};

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
    connections: Arc<ConnectionCounter>,
}

/// Descriptor-backed authority proven during both static configuration
/// verification and broker startup. Constructing this value only reads
/// process, account, cgroup, and filesystem metadata; journal initialization
/// and recovery deliberately remain in `Broker::open`.
struct BootstrapAuthority {
    config: ValidatedConfig,
    roots: BTreeMap<RootAlias, Arc<FsRoot>>,
    execution_roots: BTreeMap<ExecutionRootAlias, Arc<ExecutionRoot>>,
    worker: Account,
    client_group_gid: u32,
    broker_cgroup: crate::protocol::AbsolutePath,
}

pub(super) struct RunRuntime {
    pub id: RunId,
    pub token: RunToken,
    pub owner_uid: u32,
    pub owner: Arc<OwnerLease>,
    pub lifecycle: RunLifecycle,
    pub objects: Mutex<BTreeMap<ObjectId, LiveObject>>,
    pub stage: (Mutex<Option<LiveStage>>, Condvar),
}

pub(super) struct RunLifecycle {
    /// Serializes live-state revalidation, operation intent, domain work, and
    /// durable operation terminalization. Abort may request an interrupt
    /// through `state`, but it must join this sequence before cleanup.
    pub sequence: Mutex<()>,
    pub state: Mutex<RunLifecycleState>,
    pub changed: Condvar,
    /// Lock-free handoff used only at the stage release boundary, where taking
    /// `state` while holding the stage mutex would invert abort lock order.
    pub abort_signal: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunLifecycleState {
    Active,
    AbortRequested {
        pending_operations: usize,
    },
    Terminal {
        state: crate::journal::RunState,
        pending_operations: usize,
    },
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

#[derive(Debug)]
struct ConnectionCounter {
    active: AtomicUsize,
    limit: usize,
}

#[derive(Debug, Clone, Copy)]
struct ConnectionCapacity {
    active: usize,
    limit: usize,
}

#[derive(Debug)]
struct ConnectionSlot {
    counter: Arc<ConnectionCounter>,
}

impl ConnectionCounter {
    fn new(limit: usize) -> Self {
        assert!(limit > 0, "connection limit must be nonzero");
        Self {
            active: AtomicUsize::new(0),
            limit,
        }
    }

    fn try_acquire(self: &Arc<Self>) -> Result<ConnectionSlot, ConnectionCapacity> {
        let mut current = self.active.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return Err(ConnectionCapacity {
                    active: current,
                    limit: self.limit,
                });
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(ConnectionSlot {
                        counter: Arc::clone(self),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let previous = self.counter.active.fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0, "connection slot accounting underflow");
    }
}

impl Broker {
    pub fn open(config: ValidatedConfig) -> Result<Arc<Self>, BrokerError> {
        let authority = verify_bootstrap_authority(config)?;
        let journal = Journal::open(&authority.config.raw().journal_path)
            .map_err(|error| BrokerError::journal("startup", error))?;
        let broker = Arc::new(Self {
            broker_cgroup: authority.broker_cgroup,
            config: authority.config,
            journal: Mutex::new(journal),
            roots: authority.roots,
            execution_roots: authority.execution_roots,
            worker: authority.worker,
            client_group_gid: authority.client_group_gid,
            runs: Mutex::new(HashMap::new()),
            fatal: AtomicBool::new(false),
            connections: Arc::new(ConnectionCounter::new(MAX_CONNECTION_THREADS)),
        });
        recovery::replay(&broker)?;
        // Recovery drains every journal-authorized worker boundary before it
        // touches writable objects. Only after that replay may startup demand
        // global worker idleness; an unrecorded worker process is never
        // adopted as stage authority.
        crate::systemd::verify_worker_idle(&broker.worker.name, broker.worker.uid).map_err(
            |error| {
                BrokerError::new(
                    crate::protocol::StableCode::RecoveryRequired,
                    format!(
                        "worker identity is not idle after recovery: user={} uid={}: {error}",
                        broker.worker.name, broker.worker.uid
                    ),
                    "Inspect recorded stage rows and the fixed slice, stop any unregistered worker process or user manager, then restart the broker.",
                )
                .fatal()
            },
        )?;
        Ok(broker)
    }

    /// Performs the complete static authority check without opening Turso,
    /// running recovery, or otherwise changing durable broker state.
    pub fn verify(config: ValidatedConfig) -> Result<(), BrokerError> {
        drop(verify_bootstrap_authority(config)?);
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
                    "Inspect structured logs and the Turso/filesystem source of truth.",
                ));
            }
            let connection = listener
                .accept()
                .map_err(|error| BrokerError::system("accept control connection", error))?;
            let slot = match self.connections.try_acquire() {
                Ok(slot) => slot,
                Err(capacity) => {
                    connection_overload_error(&connection, capacity).log("connection_overloaded");
                    // A request is not read without an accounting slot. This
                    // close is fail-fast, bounded, and always paired with the
                    // structured Busy diagnostic above.
                    drop(connection);
                    continue;
                }
            };
            let broker = Arc::clone(&self);
            let spawn = std::thread::Builder::new()
                .name("calyx-gate-connection".into())
                .spawn(move || {
                    let _slot = slot;
                    handlers::serve_connection(&broker, connection);
                });
            if let Err(error) = spawn {
                return Err(BrokerError::system("spawn connection worker", error));
            }
        }
    }
}

fn verify_bootstrap_authority(config: ValidatedConfig) -> Result<BootstrapAuthority, BrokerError> {
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

    Ok(BootstrapAuthority {
        broker_cgroup: policy::broker_cgroup()?,
        config,
        roots,
        execution_roots,
        worker,
        client_group_gid,
    })
}

fn connection_overload_error(
    connection: &SeqpacketConnection,
    capacity: ConnectionCapacity,
) -> BrokerError {
    let mut error = BrokerError::new(
        crate::protocol::StableCode::Busy,
        format!(
            "control connection capacity exhausted: active={} limit={}",
            capacity.active, capacity.limit
        ),
        "Wait for an in-flight broker request to finish, then retry the original request id.",
    )
    .context("active_connections", capacity.active.to_string())
    .context("connection_limit", capacity.limit.to_string());
    match connection.peer_credentials() {
        Ok(peer) => {
            error = error
                .context("peer_pid", peer.pid.to_string())
                .context("peer_uid", peer.uid.to_string())
                .context("peer_gid", peer.gid.to_string());
        }
        Err(peer_error) => {
            error = error.context("peer_credentials_error", peer_error.to_string());
        }
    }
    error
}

pub(super) fn poisoned(name: &str) -> BrokerError {
    BrokerError::new(
        crate::protocol::StableCode::Internal,
        format!("broker {name} lock was poisoned"),
        "Inspect the preceding panic and restart only after source-of-truth verification.",
    )
    .fatal()
}

#[cfg(test)]
mod connection_tests;
