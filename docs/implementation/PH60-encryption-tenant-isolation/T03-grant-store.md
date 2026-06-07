# PH60 · T03 — `GrantStore`: grant entry + `check_grant` + `CALYX_VAULT_ACCESS_DENIED` + Ledger-stub audit

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/grant.rs` (≤500) |
| **Depends on** | T02 (`KeyspaceGuard`) · PH09 (VaultId, ActorId) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §3` (cross-vault read requires explicit Ledger-logged grant) |

## Goal

Implement the grant model that makes one vault = one tenant boundary with
default-deny semantics. Cross-vault read requires an explicit `GrantEntry`
(logged in the Ledger). Any attempt to read across vaults without a matching
grant returns `CALYX_VAULT_ACCESS_DENIED` and writes an audit record — even if
the caller knows the target vault's ID. This is the third layer of defense-in-depth
tenant isolation (key + keyspace + **grant** — `30 §3`). The Ledger write is a
stub (in-memory ring) until PH36 is wired in PH61.

## Build (checklist of concrete, code-level steps)

- [ ] `struct GrantEntry { src_vault: VaultId, dst_vault: VaultId, actor: ActorId, granted_at: Timestamp, expires_at: Option<Timestamp>, read_only: bool }` —
  `serde::{Serialize, Deserialize}`.
- [ ] `struct GrantStore { grants: Vec<GrantEntry>, audit_ring: Arc<Mutex<VecDeque<AuditEvent>>> }` —
  `audit_ring` holds the last 1024 events (capacity-bounded); this is the stub Ledger.
- [ ] `enum AuditEvent { Granted { .. }, Denied { src_vault, dst_vault, actor, at: Timestamp }, Revoked { .. } }` —
  stored in the ring.
- [ ] `impl GrantStore { pub fn new() -> Self }` — empty grant list, fresh ring.
- [ ] `pub fn add_grant(&mut self, entry: GrantEntry)` — appends; no duplicates
  (idempotent by `(src, dst, actor)` tuple — replace if exists).
- [ ] `pub fn revoke_grant(&mut self, src: VaultId, dst: VaultId, actor: ActorId)` —
  removes; writes `AuditEvent::Revoked` to ring.
- [ ] `pub fn check_grant(&self, src: VaultId, dst: VaultId, actor: ActorId, now: Timestamp) -> Result<()>` —
  default deny: if no matching active (non-expired) grant exists, pushes
  `AuditEvent::Denied` to the ring and returns `CALYX_VAULT_ACCESS_DENIED`.
  Never panics on missing principal — fail closed (A16).
- [ ] `pub fn audit_events(&self, last_n: usize) -> Vec<AuditEvent>` — returns up to
  `last_n` events from the ring for FSV inspection.
- [ ] Add `CALYX_VAULT_ACCESS_DENIED` to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: no grants → `check_grant(A, B, actor, T)` → `CALYX_VAULT_ACCESS_DENIED`;
  assert `audit_events(1)` contains a `Denied` entry with matching `(A, B, actor)`.
- [ ] unit: add grant `(A, B, actor)` → `check_grant(A, B, actor, T)` → `Ok(())`;
  no `Denied` entry in audit ring for this call.
- [ ] unit: expired grant (`expires_at = T - 1`) → `check_grant` at `T` →
  `CALYX_VAULT_ACCESS_DENIED`; `Denied` in ring.
- [ ] unit: grant `(A, B, actor1)` does not satisfy `check_grant(A, B, actor2)`.
- [ ] proptest: `∀ random grant lists`: `check_grant` returns `Ok(())` iff a matching
  non-expired grant exists (property: grant predicate is exact, no off-by-one in
  expiry comparison).
- [ ] edge (≥3): empty grant list; grant and immediately revoke — next check denied;
  ring overflow at 1025th event — oldest event dropped, newest retained; `src == dst`
  (self-vault — allowed without a grant entry, add explicit short-circuit).
- [ ] fail-closed: unknown actor with no grant → `CALYX_VAULT_ACCESS_DENIED`; never
  a silent `Ok(())`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary + in-memory `GrantStore`; later the integration test
  in T07 uses a live vault.
- **Readback:** `cargo test -p calyx-aster grant -- --nocapture 2>&1` prints:
  `check_grant(A,B,actor) = Err(CALYX_VAULT_ACCESS_DENIED)` for the no-grant case;
  `check_grant(A,B,actor) = Ok(())` after adding the grant; audit ring contents
  printed as JSON.
- **Prove:** before: no grant type; after: denied path returns the structured code and
  populates the audit ring; granted path returns `Ok`; `audit_events(10)` shows the
  `Denied` record with correct field values.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
