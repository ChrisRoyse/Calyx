# PH61 · T03 — `retention.rs`: TTL policy + `apply_retention` scan

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/retention.rs` (≤500) |
| **Depends on** | T01 (`erase` scope + `EraseResult`) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §4` (Retention / data minimization — per-collection TTL/rollup) |

## Goal

Implement `RetentionPolicy` — a per-collection TTL configuration — and the
`apply_retention(vault_ctx, now)` scan that identifies constellations whose
retention period has expired and enqueues them for erasure via the T01 `erase()`
path (including tombstone write and crypto-shred where applicable). This is the
automated data-minimization arm of the privacy model (`30 §4`). Retention expiry
is always surfaced explicitly — never silently dropped without an audit trail.

## Build (checklist of concrete, code-level steps)

- [ ] `struct RetentionPolicy { collection: String, ttl_secs: u64, rollup_after_secs: Option<u64> }` —
  `ttl_secs = 0` means retain indefinitely (explicit opt-in); `rollup_after_secs`
  is a future hook for cold-tier aggregation (stub for now: log "rollup not yet
  implemented").
- [ ] `struct RetentionStore { policies: HashMap<String, RetentionPolicy> }` —
  keyed by collection name.
- [ ] `impl RetentionStore { pub fn add_policy(&mut self, policy: RetentionPolicy) }`.
- [ ] `pub fn policy_for(&self, collection: &str) -> Option<&RetentionPolicy>` —
  returns `None` if no policy (no TTL = retain indefinitely, not a default deletion).
- [ ] `fn is_expired(ingested_at: Timestamp, policy: &RetentionPolicy, now: Timestamp) -> bool` —
  `now - ingested_at > policy.ttl_secs * 1_000_000_000` (nanoseconds); injected
  clock (`now` parameter, never `SystemTime::now()`).
- [ ] `pub fn apply_retention(vault_ctx: &mut VaultContext, store: &RetentionStore, registry: &EraseRegistry, ledger: &mut dyn LedgerAppend, now: Timestamp) -> Result<Vec<EraseResult>>` —
  scans all constellations in `vault_ctx`'s keyspace that have an associated
  collection with a TTL policy; for each expired CX calls `erase(Cx(cx_id), ...)`
  (which writes the tombstone and shreds); returns the list of `EraseResult`s;
  fails closed on any per-CX error (logs the error, continues scan, collects
  all errors, returns the first non-`ALREADY_TOMBSTONED` error if any).
- [ ] `fn scan_expired_cxs(vault_ctx: &VaultContext, store: &RetentionStore, now: Timestamp) -> Vec<(CxId, String)>` —
  scans the base CF using `vault_ctx.keyspace.owns_key()` filter; reads the
  `ingested_at` metadata field from each CX; returns `(CxId, collection_name)`
  pairs where `is_expired` is true.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `is_expired` with `ingested_at = 100s`, `ttl_secs = 60`, `now = 161s` →
  `true`; `now = 159s` → `false` (boundary correct).
- [ ] unit: `apply_retention` with 3 CXs (2 expired, 1 not) →
  `Vec<EraseResult>.len() == 2`; surviving CX still readable.
- [ ] unit: `apply_retention` with no TTL policy for the collection → 0 erasures
  (no policy = retain).
- [ ] unit: `is_expired` with `ttl_secs = 0` → always `false` (retain indefinitely).
- [ ] proptest: `∀ (ingested_at, ttl_secs, now)` with injected clock:
  `is_expired(ingested_at, ttl, now) == (now - ingested_at > ttl * 1e9)`.
- [ ] edge (≥3): `now < ingested_at` (clock skew) → `is_expired = false` (no negative
  unsigned overflow panic — use saturating subtraction); all CXs expired →
  entire vault erased via repeated `Cx` calls (not a single `Vault` erase, to
  preserve per-CX tombstones); `apply_retention` on already-tombstoned CX →
  `CALYX_ERASE_ALREADY_TOMBSTONED` counted but not fatal.
- [ ] fail-closed: `apply_retention` returns accumulated errors after scanning all CXs;
  a single CX erase failure does not abort the entire retention sweep.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** synthetic vault with known `ingested_at` timestamps and a configured
  TTL policy; injected clock set past the expiry threshold.
- **Readback:** `cargo test -p calyx-aster retention -- --nocapture 2>&1` prints
  `apply_retention: 2 erased, 1 retained`; the two erased CXs show tombstone entries
  in the Ledger; the surviving CX's CF record is still present.
- **Prove:** before: no retention scan; after: expired CXs removed + tombstoned;
  surviving CX unaffected; `is_expired` boundary at `ttl_secs` is exact (not ±1).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
