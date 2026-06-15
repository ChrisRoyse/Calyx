# PH60 · T05 — `TlsConfig` / `MtlsConfig` / `AuthN` types + no-anonymous-write predicate

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-core` |
| **Files** | `crates/calyx-core/src/security.rs` (≤500) |
| **Depends on** | — (standalone types; no prior PH60 card required) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §2` (AuthN axis, Crypto in transit axis) |

## Goal

Define the canonical `TlsConfig`, `MtlsConfig`, and `AuthN` types that `calyxd`
(PH65) and the CLI (PH62) will reference. Implement the `no_anonymous_write`
predicate that all mutation entry points must satisfy: any principal that cannot
present a valid token (mTLS, Cloudflare Access, or in-process host identity) is
rejected with `CALYX_AUTHN_REQUIRED` before any write reaches the vault. These
types live in `calyx-core` so they can be imported by both `calyx-aster` and
`calyxd` without a circular dependency.

## Build (checklist of concrete, code-level steps)

- [ ] `struct TlsConfig { cert_pem_path: PathBuf, key_pem_path: PathBuf, ca_pem_path: Option<PathBuf> }` —
  for server-mode TLS; `ca_pem_path` present → mTLS (mutual).
- [ ] `struct MtlsConfig { tls: TlsConfig, require_client_cert: bool }` — when
  `require_client_cert = true` the server rejects connections without a valid
  client certificate.
- [ ] `enum AuthN { InProcess { host_app_id: String }, MtlsToken { fingerprint: [u8; 32] }, CloudflareAccess { service_token_id: String } }` —
  these are the three permitted identity modes; `serde::{Serialize, Deserialize}`.
- [ ] `fn no_anonymous_write(authn: Option<&AuthN>) -> Result<()>` — if `authn` is
  `None` returns `CALYX_AUTHN_REQUIRED` (fail closed, A16); if `Some(_)` returns
  `Ok(())`. Embedded vaults always supply `InProcess`; server mode must supply
  `MtlsToken` or `CloudflareAccess`.
- [ ] `impl AuthN { pub fn is_server_mode(&self) -> bool }` — returns `false` for
  `InProcess`, `true` for the other variants; used by `calyxd` to enforce the
  mTLS/CF requirement in server mode.
- [ ] `impl TlsConfig { pub fn validate(&self) -> Result<()> }` — checks that the
  cert and key paths exist and are readable (metadata only, no parse); returns
  `CALYX_TLS_CONFIG_INVALID` if not.
- [ ] Define module-local `pub const CALYX_AUTHN_REQUIRED` and
  `CALYX_TLS_CONFIG_INVALID` in `calyx-core/src/security.rs`. Do not add these
  PH60 codes to `calyx-core/src/error.rs` / `CALYX_ERROR_CODES` unless
  `docs/dbprdplans/18_API_TYPES_ERRORS.md` is amended in the same change.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `no_anonymous_write(None)` → `CALYX_AUTHN_REQUIRED`.
- [ ] unit: `no_anonymous_write(Some(AuthN::InProcess { .. }))` → `Ok(())`.
- [ ] unit: `no_anonymous_write(Some(AuthN::MtlsToken { .. }))` → `Ok(())`.
- [ ] unit: `is_server_mode` returns correct value for each variant.
- [ ] unit: `TlsConfig::validate` with non-existent paths → `CALYX_TLS_CONFIG_INVALID`;
  with existing temp files → `Ok(())`.
- [ ] edge (≥3): `AuthN` serde round-trip for all three variants → byte-exact; empty
  `host_app_id` string → still `Ok(())` from `no_anonymous_write` (identity is present,
  host app responsibility to validate content); `ca_pem_path = None` → server TLS without
  mTLS is permitted (client cert not required).
- [ ] fail-closed: `no_anonymous_write(None)` must never return `Ok(())` regardless of
  any future refactoring — assert the exact `CALYX_AUTHN_REQUIRED` code.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary + synthetic paths via `tempfile`.
- **Readback:** `cargo test -p calyx-core security -- --nocapture 2>&1` prints
  `no_anonymous_write(None) = Err(CALYX_AUTHN_REQUIRED)` and all serde round-trip
  confirmations.
- **Prove:** before: no security types in `calyx-core`; after: types compile; anonymous
  call path returns the structured error; `TlsConfig::validate` distinguishes missing vs
  present files; all three `AuthN` variants survive serde round-trip byte-exact.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
