//! Cross-model transaction serialization for one Aster vault.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Clock, Result, Seq, VaultId};

use crate::vault::AsterVault;

mod cross_model;
mod validation;

pub use crate::collection::IsolationLevel;
pub use cross_model::CrossModelTxn;

pub const CALYX_TXN_TIMEOUT: &str = "CALYX_TXN_TIMEOUT";
pub const CALYX_TXN_COST_CAP: &str = "CALYX_TXN_COST_CAP";

pub(crate) const CALYX_TXN_INVALID_ARGUMENT: &str = "CALYX_TXN_INVALID_ARGUMENT";

#[derive(Clone)]
pub struct TxnHandle {
    vault_id: VaultId,
    inner: Arc<TxnInner>,
}

struct TxnInner {
    state: Mutex<TxnState>,
    ready: Condvar,
}

#[derive(Clone, Copy, Debug)]
pub enum TxnState {
    Idle,
    Active {
        started_at: Instant,
        cost_cap_ms: Option<u32>,
    },
}

impl TxnHandle {
    pub fn new(vault_id: VaultId) -> Self {
        Self {
            vault_id,
            inner: Arc::new(TxnInner {
                state: Mutex::new(TxnState::Idle),
                ready: Condvar::new(),
            }),
        }
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub fn state(&self) -> Result<TxnState> {
        self.inner
            .state
            .lock()
            .map(|state| *state)
            .map_err(|_| CalyxError::backpressure("txn state lock poisoned"))
    }

    pub fn begin(
        &self,
        isolation: IsolationLevel,
        cost_cap_ms: Option<u32>,
        timeout: Duration,
    ) -> Result<CrossModelTxn<'_>> {
        self.begin_at(0, isolation, cost_cap_ms, timeout)
    }

    pub fn begin_on<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        isolation: IsolationLevel,
        cost_cap_ms: Option<u32>,
        timeout: Duration,
    ) -> Result<CrossModelTxn<'_>> {
        self.verify_vault(vault)?;
        self.begin_at(vault.latest_seq(), isolation, cost_cap_ms, timeout)
    }

    pub fn begin_at(
        &self,
        snapshot_seq: Seq,
        isolation: IsolationLevel,
        cost_cap_ms: Option<u32>,
        timeout: Duration,
    ) -> Result<CrossModelTxn<'_>> {
        if cost_cap_ms == Some(0) {
            return Err(txn_error(
                CALYX_TXN_INVALID_ARGUMENT,
                "txn cost_cap_ms must be greater than zero when set",
                "begin the transaction with a positive cost cap",
            ));
        }
        let started_at = self.acquire(cost_cap_ms, timeout)?;
        Ok(CrossModelTxn::new(
            self,
            isolation,
            cost_cap_ms,
            snapshot_seq,
            started_at,
        ))
    }

    pub(crate) fn verify_vault<C: Clock>(&self, vault: &AsterVault<C>) -> Result<()> {
        if vault.vault_id() == self.vault_id {
            Ok(())
        } else {
            Err(CalyxError::vault_access_denied(
                "transaction handle belongs to another vault",
            ))
        }
    }

    pub(crate) fn release(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            *state = TxnState::Idle;
            self.inner.ready.notify_one();
        }
    }

    fn acquire(&self, cost_cap_ms: Option<u32>, timeout: Duration) -> Result<Instant> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| CalyxError::backpressure("txn state lock poisoned"))?;
        loop {
            match *state {
                TxnState::Idle => {
                    let started_at = Instant::now();
                    *state = TxnState::Active {
                        started_at,
                        cost_cap_ms,
                    };
                    return Ok(started_at);
                }
                TxnState::Active { .. } => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(txn_timeout());
                    }
                    let remaining = deadline.saturating_duration_since(now);
                    let (next_state, wait) = self
                        .inner
                        .ready
                        .wait_timeout(state, remaining)
                        .map_err(|_| CalyxError::backpressure("txn condvar lock poisoned"))?;
                    state = next_state;
                    if wait.timed_out() && matches!(*state, TxnState::Active { .. }) {
                        return Err(txn_timeout());
                    }
                }
            }
        }
    }
}

pub(crate) fn txn_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

pub(crate) fn txn_timeout() -> CalyxError {
    txn_error(
        CALYX_TXN_TIMEOUT,
        "another transaction is active for this vault",
        "retry after the active transaction commits or rolls back",
    )
}

#[cfg(test)]
mod tests;
