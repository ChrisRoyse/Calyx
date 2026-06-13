//! Cross-vault grant model with default-deny semantics (PH60 · T03).
//!
//! [`GrantStore`] is the third layer of the defense-in-depth tenant-isolation
//! stack (key + keyspace + **grant** — PRD `30 §3`), above [`super::key`] and
//! [`super::keyspace`]. One vault = one tenant boundary. A cross-vault read is
//! **denied by default**: it requires an explicit, non-expired [`GrantEntry`],
//! and *every* decision — grant, deny, revoke — is recorded in an immutable
//! audit ring (the stub Ledger until PH36 is wired in PH61).
//!
//! This mirrors the established cross-tenant pattern: deny unless an explicit
//! grant says otherwise, and log every access decision for accountability.
//! A denied check **fails closed** with [`CALYX_VAULT_ACCESS_DENIED`] (A16) —
//! even when the caller already knows the target vault's id.

use calyx_core::{CalyxError, Result, Ts, VaultId};
use calyx_ledger::ActorId;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Maximum events retained in the audit ring (capacity-bounded, A26). The
/// oldest event is dropped once this is exceeded.
pub const AUDIT_RING_CAPACITY: usize = 1024;

/// An explicit permission for `actor` to read from `dst_vault` while operating
/// in `src_vault`. Absence of a matching active entry means **denied**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantEntry {
    /// Vault the actor is operating in (the reader's home vault).
    pub src_vault: VaultId,
    /// Vault being read across into.
    pub dst_vault: VaultId,
    /// Principal the grant is issued to.
    pub actor: ActorId,
    /// When the grant was issued (Unix millis).
    pub granted_at: Ts,
    /// Optional expiry (Unix millis); `None` = never expires.
    pub expires_at: Option<Ts>,
    /// Reserved for write-grant support; cross-vault is read-only today.
    pub read_only: bool,
}

impl GrantEntry {
    /// True iff this entry authorizes `(src, dst, actor)` and has not expired at
    /// `now`. Expiry is exclusive: a grant with `expires_at = e` is active while
    /// `now < e` and expired at `now == e` (no off-by-one — see expiry test).
    fn authorizes(&self, src: VaultId, dst: VaultId, actor: &ActorId, now: Ts) -> bool {
        self.src_vault == src
            && self.dst_vault == dst
            && &self.actor == actor
            && self.expires_at.is_none_or(|e| now < e)
    }

    /// True iff this entry has the same identity tuple `(src, dst, actor)` —
    /// used for idempotent add and for revoke.
    fn same_principal(&self, src: VaultId, dst: VaultId, actor: &ActorId) -> bool {
        self.src_vault == src && self.dst_vault == dst && &self.actor == actor
    }
}

/// An immutable record of a single access-control decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEvent {
    /// A grant was added (or replaced).
    Granted {
        src_vault: VaultId,
        dst_vault: VaultId,
        actor: ActorId,
        at: Ts,
    },
    /// A cross-vault read was denied (default-deny or expired/absent grant).
    Denied {
        src_vault: VaultId,
        dst_vault: VaultId,
        actor: ActorId,
        at: Ts,
    },
    /// A grant was revoked.
    Revoked {
        src_vault: VaultId,
        dst_vault: VaultId,
        actor: ActorId,
        at: Ts,
    },
}

/// Default-deny cross-vault grant table with a bounded immutable audit ring.
#[derive(Debug, Default)]
pub struct GrantStore {
    grants: Vec<GrantEntry>,
    audit_ring: Arc<Mutex<VecDeque<AuditEvent>>>,
}

impl GrantStore {
    /// Builds an empty store: no grants, fresh audit ring.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a grant, idempotent by `(src, dst, actor)`: an existing entry for the
    /// same principal is **replaced** (re-issuing updates expiry, never
    /// duplicates). Writes an [`AuditEvent::Granted`].
    pub fn add_grant(&mut self, entry: GrantEntry) {
        let at = entry.granted_at;
        let (src, dst, actor) = (entry.src_vault, entry.dst_vault, entry.actor.clone());
        if let Some(existing) = self
            .grants
            .iter_mut()
            .find(|g| g.same_principal(src, dst, &actor))
        {
            *existing = entry;
        } else {
            self.grants.push(entry);
        }
        self.push_audit(AuditEvent::Granted {
            src_vault: src,
            dst_vault: dst,
            actor,
            at,
        });
    }

    /// Removes any grant for `(src, dst, actor)` and writes an
    /// [`AuditEvent::Revoked`] at `now`. A revoke of a non-existent grant is a
    /// no-op on the table but is still audited (the intent is recorded).
    pub fn revoke_grant(&mut self, src: VaultId, dst: VaultId, actor: ActorId, now: Ts) {
        self.grants.retain(|g| !g.same_principal(src, dst, &actor));
        self.push_audit(AuditEvent::Revoked {
            src_vault: src,
            dst_vault: dst,
            actor,
            at: now,
        });
    }

    /// Default-deny authorization check for a cross-vault read.
    ///
    /// Returns `Ok(())` iff `src == dst` (a vault always reads itself) **or** a
    /// matching non-expired [`GrantEntry`] exists. Otherwise records an
    /// [`AuditEvent::Denied`] and returns [`CALYX_VAULT_ACCESS_DENIED`].
    /// Never panics on an unknown principal — fail closed (A16).
    pub fn check_grant(&self, src: VaultId, dst: VaultId, actor: ActorId, now: Ts) -> Result<()> {
        // A vault reading its own keyspace needs no grant.
        if src == dst {
            return Ok(());
        }
        let authorized = self
            .grants
            .iter()
            .any(|g| g.authorizes(src, dst, &actor, now));
        if authorized {
            return Ok(());
        }
        self.push_audit(AuditEvent::Denied {
            src_vault: src,
            dst_vault: dst,
            actor: actor.clone(),
            at: now,
        });
        Err(CalyxError::vault_access_denied(format!(
            "no active grant for {actor:?} reading {src} -> {dst} at t={now}"
        )))
    }

    /// Returns up to `last_n` most-recent audit events, oldest-first, for FSV
    /// inspection. Reading the ring is a separate operation from the decision
    /// that wrote it (the Source-of-Truth read).
    pub fn audit_events(&self, last_n: usize) -> Vec<AuditEvent> {
        let ring = self.lock_ring();
        let start = ring.len().saturating_sub(last_n);
        ring.iter().skip(start).cloned().collect()
    }

    /// Number of active grant entries currently in the table.
    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }

    /// Total events currently retained in the audit ring.
    pub fn audit_len(&self) -> usize {
        self.lock_ring().len()
    }

    fn push_audit(&self, event: AuditEvent) {
        let mut ring = self.lock_ring();
        if ring.len() >= AUDIT_RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(event);
    }

    /// Locks the ring, recovering from poisoning (a panic while holding the lock
    /// must not wedge audit; the ring carries no cross-entry invariant).
    fn lock_ring(&self) -> std::sync::MutexGuard<'_, VecDeque<AuditEvent>> {
        self.audit_ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    const T: Ts = 1_000_000; // synthetic "now" in Unix millis

    fn vault(byte: u8) -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
    }

    fn actor(name: &str) -> ActorId {
        ActorId::Agent(name.to_string())
    }

    fn grant(src: VaultId, dst: VaultId, actor: ActorId, expires_at: Option<Ts>) -> GrantEntry {
        GrantEntry {
            src_vault: src,
            dst_vault: dst,
            actor,
            granted_at: T,
            expires_at,
            read_only: true,
        }
    }

    fn ring_json(store: &GrantStore) -> String {
        serde_json::to_string_pretty(&store.audit_events(16)).unwrap()
    }

    #[test]
    fn no_grant_denies_and_audits() {
        let store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        let err = store.check_grant(a, b, actor("agent1"), T).unwrap_err();
        println!("check_grant(A,B,agent1) = Err({})", err.code);
        println!("audit ring = {}", ring_json(&store));
        assert_eq!(err.code, "CALYX_VAULT_ACCESS_DENIED");
        let events = store.audit_events(1);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            AuditEvent::Denied {
                src_vault: a,
                dst_vault: b,
                actor: actor("agent1"),
                at: T,
            }
        );
    }

    #[test]
    fn explicit_grant_allows_without_denial() {
        let mut store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        store.add_grant(grant(a, b, actor("agent1"), None));
        // Trigger X: the granted cross-vault read.
        store
            .check_grant(a, b, actor("agent1"), T)
            .expect("grant must allow");
        println!("audit ring = {}", ring_json(&store));
        // Outcome Y: exactly one Granted event, no Denied event.
        let events = store.audit_events(16);
        assert_eq!(events.len(), 1, "Ok path must not append a Denied event");
        assert!(matches!(events[0], AuditEvent::Granted { .. }));
    }

    #[test]
    fn expired_grant_denies_at_boundary() {
        let store_setup = |expires_at: Option<Ts>| {
            let mut s = GrantStore::new();
            s.add_grant(grant(vault(0xA1), vault(0xB2), actor("agent1"), expires_at));
            s
        };
        let (a, b) = (vault(0xA1), vault(0xB2));
        // expires_at = T-1: at now=T the grant is expired -> denied.
        let expired = store_setup(Some(T - 1));
        assert_eq!(
            expired
                .check_grant(a, b, actor("agent1"), T)
                .unwrap_err()
                .code,
            "CALYX_VAULT_ACCESS_DENIED"
        );
        // expires_at = T (exclusive boundary): now=T is expired -> denied.
        let boundary = store_setup(Some(T));
        assert!(boundary.check_grant(a, b, actor("agent1"), T).is_err());
        // expires_at = T+1: now=T is still active -> allowed.
        let active = store_setup(Some(T + 1));
        assert!(active.check_grant(a, b, actor("agent1"), T).is_ok());
    }

    #[test]
    fn grant_is_actor_scoped() {
        let mut store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        store.add_grant(grant(a, b, actor("agent1"), None));
        // agent2 has no grant even though agent1 does.
        assert_eq!(
            store
                .check_grant(a, b, actor("agent2"), T)
                .unwrap_err()
                .code,
            "CALYX_VAULT_ACCESS_DENIED"
        );
        assert!(store.check_grant(a, b, actor("agent1"), T).is_ok());
    }

    #[test]
    fn add_grant_is_idempotent_by_principal() {
        let mut store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        store.add_grant(grant(a, b, actor("agent1"), Some(T + 10)));
        store.add_grant(grant(a, b, actor("agent1"), Some(T + 99))); // re-issue
        assert_eq!(
            store.grant_count(),
            1,
            "re-issuing must replace, not duplicate"
        );
        // The replacement's expiry is in effect.
        assert!(store.check_grant(a, b, actor("agent1"), T + 50).is_ok());
    }

    #[test]
    fn grant_then_revoke_denies() {
        let mut store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        store.add_grant(grant(a, b, actor("agent1"), None));
        assert!(store.check_grant(a, b, actor("agent1"), T).is_ok());
        store.revoke_grant(a, b, actor("agent1"), T);
        assert_eq!(store.grant_count(), 0);
        // After revoke: denied, and a Revoked event precedes the Denied one.
        assert_eq!(
            store
                .check_grant(a, b, actor("agent1"), T)
                .unwrap_err()
                .code,
            "CALYX_VAULT_ACCESS_DENIED"
        );
        let events = store.audit_events(16);
        println!("audit ring = {}", ring_json(&store));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditEvent::Revoked { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditEvent::Denied { .. }))
        );
    }

    #[test]
    fn self_vault_read_needs_no_grant() {
        let store = GrantStore::new();
        let a = vault(0xA1);
        assert!(store.check_grant(a, a, actor("agent1"), T).is_ok());
        // No audit event for the self-read short-circuit.
        assert_eq!(store.audit_len(), 0);
    }

    #[test]
    fn audit_ring_drops_oldest_on_overflow() {
        let store = GrantStore::new();
        let (a, b) = (vault(0xA1), vault(0xB2));
        // 1025 denials -> ring holds the last 1024; event #0 ("a0") is dropped.
        for i in 0..(AUDIT_RING_CAPACITY + 1) {
            let _ = store.check_grant(a, b, actor(&format!("a{i}")), T);
        }
        assert_eq!(
            store.audit_len(),
            AUDIT_RING_CAPACITY,
            "ring is capacity-bounded"
        );
        let all = store.audit_events(AUDIT_RING_CAPACITY + 100);
        assert_eq!(all.len(), AUDIT_RING_CAPACITY);
        // Oldest retained is "a1" (a0 evicted); newest is "a1024".
        let first_actor = match &all[0] {
            AuditEvent::Denied { actor, .. } => actor.clone(),
            other => panic!("unexpected first event: {other:?}"),
        };
        let last_actor = match all.last().unwrap() {
            AuditEvent::Denied { actor, .. } => actor.clone(),
            other => panic!("unexpected last event: {other:?}"),
        };
        assert_eq!(
            first_actor,
            actor("a1"),
            "oldest event (a0) must be evicted"
        );
        assert_eq!(last_actor, actor(&format!("a{AUDIT_RING_CAPACITY}")));
    }

    proptest::proptest! {
        #[test]
        fn check_grant_ok_iff_matching_active_grant_exists(
            seed in proptest::collection::vec(
                (0u8..4, 0u8..4, 0u8..3, proptest::option::of(0u64..2_000_000u64)),
                0..40,
            ),
            qs in 0u8..4,
            qd in 0u8..4,
            qa in 0u8..3,
        ) {
            let mut store = GrantStore::new();
            for (s, d, ac, exp) in &seed {
                store.add_grant(grant(vault(*s), vault(*d), actor(&format!("a{ac}")), *exp));
            }
            let (s, d, ac) = (vault(qs), vault(qd), actor(&format!("a{qa}")));
            // Independent oracle. add_grant replaces by principal (last write
            // wins), so the effective grant for the query principal is the LAST
            // matching seed entry — not "any" of them.
            let mut effective_expiry: Option<Option<Ts>> = None;
            for (gs, gd, gac, gexp) in &seed {
                if vault(*gs) == s && vault(*gd) == d && actor(&format!("a{gac}")) == ac {
                    effective_expiry = Some(*gexp);
                }
            }
            let oracle_authorized = qs == qd
                || matches!(effective_expiry, Some(exp) if exp.is_none_or(|e| T < e));
            let got = store.check_grant(s, d, ac, T);
            proptest::prop_assert_eq!(got.is_ok(), oracle_authorized);
        }
    }
}
