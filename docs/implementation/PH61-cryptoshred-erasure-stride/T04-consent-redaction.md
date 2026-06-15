# PH61 · T04 — `consent.rs` + `redaction.rs`: consent/purpose tags + PII hash-only mode

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-core`, `calyx-aster` |
| **Files** | `crates/calyx-core/src/consent.rs` (≤500), `crates/calyx-aster/src/redaction.rs` (≤500) |
| **Depends on** | — (standalone types; no prior PH61 card required) |
| **Axioms** | A33, A16, A2 |
| **PRD** | `dbprdplans/30 §4` (Consent & purpose — processing exceeding consent fails closed; PII redaction — hash-only input) |

## Goal

Implement the consent and purpose-tagging model so that every ingest operation
carries a declared lawful basis and a purpose tag, and any downstream processing
that exceeds the declared consent fails closed with `CALYX_CONSENT_VIOLATION`.
Implement PII redaction so that raw input text may be stored as hash-only (the
vector persists; the PII source is removed), enabling search and intelligence to
work without the PII being present in the vault. Both together implement data
minimization and consent-governed processing from `30 §4`.

## Build (checklist of concrete, code-level steps)

### `calyx-core/src/consent.rs`

- [ ] `enum LawfulBasis { Consent, LegitimateInterest, ContractPerformance, LegalObligation, VitalInterests, PublicTask }` —
  `serde`, `Display`.
- [ ] `enum Purpose { Search, Intelligence, Reranking, Analytics, Export, AuditOnly }` —
  `serde`. Calyx's own operations declare their purpose; a vault declares a set of
  permitted purposes.
- [ ] `struct ConsentTag { lawful_basis: LawfulBasis, permitted_purposes: Vec<Purpose>, expires_at: Option<Timestamp> }` —
  `serde`. Stored alongside each constellation (or vault-wide as a default policy).
- [ ] `fn check_consent(tag: &ConsentTag, requested_purpose: Purpose, now: Timestamp) -> Result<()>` —
  returns `Ok(())` if `tag.permitted_purposes` contains `requested_purpose` and the
  tag has not expired; returns `CALYX_CONSENT_VIOLATION` otherwise (fail closed, A16).
- [ ] `fn consent_expired(tag: &ConsentTag, now: Timestamp) -> bool` —
  `tag.expires_at.map_or(false, |exp| now >= exp)`.
- [ ] Define module-local `pub const CALYX_CONSENT_VIOLATION` in
  `calyx-core/src/consent.rs`. Do not add this PH61 code to
  `calyx-core/src/error.rs` / `CALYX_ERROR_CODES` unless
  `docs/dbprdplans/18_API_TYPES_ERRORS.md` is amended in the same change.

### `calyx-aster/src/redaction.rs`

- [ ] `enum InputMode { Full(String), HashOnly([u8; 32]), Redacted }` — `serde`;
  `Full` stores the raw text; `HashOnly` stores `blake3(raw_bytes)` and discards
  the original; `Redacted` stores nothing (vectors only).
- [ ] `fn redact_to_hash(raw: &str) -> InputMode` — computes `blake3(raw.as_bytes())`
  and returns `InputMode::HashOnly(hash)`.
- [ ] `fn assert_hash_only_mode(mode: &InputMode) -> Result<()>` — returns `Ok(())`
  for `HashOnly` or `Redacted`; returns `CALYX_PII_REDACTION_REQUIRED` for `Full`
  when the vault's consent tag specifies hash-only input (called at ingest boundary).
- [ ] `fn pii_input_for_ingest(raw: &str, require_redacted: bool) -> InputMode` —
  if `require_redacted` → `redact_to_hash(raw)`; else → `Full(raw.to_string())`.
- [ ] Define module-local `pub const CALYX_PII_REDACTION_REQUIRED` in
  `calyx-aster/src/redaction.rs`. Do not add this PH61 code to
  `calyx-core/src/error.rs` / `CALYX_ERROR_CODES` unless
  `docs/dbprdplans/18_API_TYPES_ERRORS.md` is amended in the same change.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `check_consent` with `permitted = [Search]`, `requested = Intelligence`
  → `CALYX_CONSENT_VIOLATION`.
- [ ] unit: `check_consent` with `permitted = [Search, Intelligence]`,
  `requested = Intelligence`, non-expired → `Ok(())`.
- [ ] unit: expired `ConsentTag` (`expires_at = T - 1`, `now = T`) →
  `CALYX_CONSENT_VIOLATION` even for a permitted purpose.
- [ ] unit: `redact_to_hash("hello world")` → `InputMode::HashOnly(h)` where `h`
  equals the hard-coded `blake3("hello world")` golden constant.
- [ ] unit: `assert_hash_only_mode(Full(...))` → `CALYX_PII_REDACTION_REQUIRED`;
  `assert_hash_only_mode(HashOnly(...))` → `Ok(())`.
- [ ] proptest: `∀ raw ∈ String[0..1024]`: `redact_to_hash(raw)` returns `HashOnly`;
  the hash is deterministic (same input → same hash).
- [ ] edge (≥3): empty permitted purposes list → `CALYX_CONSENT_VIOLATION` for any
  purpose; `AuditOnly` purpose always permitted (special-case: Calyx audit operations
  never violate consent); `consent_expired` with `expires_at = None` → always
  `false` (indefinite consent).
- [ ] fail-closed: processing with `CALYX_CONSENT_VIOLATION` must not proceed; the
  constellation must not be returned to the caller.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary; synthetic `ConsentTag` structs with fixed timestamps
  (injected clock).
- **Readback:** `cargo test -p calyx-core consent -- --nocapture 2>&1` and
  `cargo test -p calyx-aster redaction -- --nocapture 2>&1` both print:
  - `check_consent(Intelligence, permitted=[Search]) = Err(CALYX_CONSENT_VIOLATION)`
  - `redact_to_hash("hello world") = HashOnly(<golden 32-byte hex>)`
  - `assert_hash_only_mode(Full) = Err(CALYX_PII_REDACTION_REQUIRED)`
- **Prove:** before: no consent types; after: consent violation blocked; hash-only
  mode produces deterministic blake3 output matching the golden constant; `Full`
  mode at a hash-only ingest boundary is rejected with the structured error.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines each (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
