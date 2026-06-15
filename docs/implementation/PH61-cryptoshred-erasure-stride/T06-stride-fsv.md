# PH61 · T06 — `stride_fsv.rs`: six STRIDE defenses FSV-proven

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/stride_fsv.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 · PH60 T01–T07 |
| **Axioms** | A33, A16, A15 |
| **PRD** | `dbprdplans/30 §1` (STRIDE threat model — all six rows) |

## Goal

Implement six named test functions (one per STRIDE threat category) that each
byte-prove a specific defense control on aiwonder. These functions are the FSV
gate for the STRIDE model row in `30 §1`. Running all six passing is necessary
and sufficient to claim the STRIDE defense column of the `SECURITY` predicate.

Each function is an `#[test]` using synthetic, seeded, deterministic data and
an injected clock. All six must run with `cargo test -p calyx-aster stride_fsv --
--nocapture` and each must print its result to stdout for evidence collection.

## Build (checklist of concrete, code-level steps)

### Spoofing — mTLS / no anonymous writes

- [ ] `fn stride_s_spoofing_anonymous_write_denied()` — construct a mutation call
  with `authn = None`; call `no_anonymous_write(None)` from `calyx-core/src/security.rs`;
  assert `Err(CALYX_AUTHN_REQUIRED)`. Print:
  `"[STRIDE S] anonymous_write = Err(CALYX_AUTHN_REQUIRED) ✓"`.

### Tampering — ZFS checksums + Ledger hash-chain

- [ ] `fn stride_t_tampering_ledger_chain_detected()` — write a sequence of
  `LedgerEntry`s; flip one byte in entry at seq=2 using raw CF write; call
  `verify_chain(range 0..5)` from PH36; assert the result is
  `Err(CALYX_LEDGER_CHAIN_BROKEN)` with the broken sequence number == 2.
  Print: `"[STRIDE T] verify_chain after tamper = Err(CALYX_LEDGER_CHAIN_BROKEN at seq=2) ✓"`.

### Repudiation — append-only actor-stamped Ledger

- [ ] `fn stride_r_repudiation_ledger_immutable()` — write a `LedgerEntry` with
  `actor = ActorId::new("actor-a")`; attempt to overwrite the same seq via a direct
  CF put (bypassing the append-only API); assert the overwrite returns
  `CALYX_LEDGER_APPEND_ONLY`. Print:
  `"[STRIDE R] ledger append-only overwrite = Err(CALYX_LEDGER_APPEND_ONLY) ✓"`.

### Information disclosure — default-deny isolation + redaction

- [ ] `fn stride_i_info_disclosure_cross_vault_denied()` — construct two `VaultContext`s;
  attempt cross-vault read without grant; assert `CALYX_VAULT_ACCESS_DENIED`;
  read audit ring and assert `Denied` event present. Print:
  `"[STRIDE I] cross_vault_read = Err(CALYX_VAULT_ACCESS_DENIED), audit_event = Denied ✓"`.

### Denial of service — bounded queues + quotas

- [ ] `fn stride_d_dos_quota_backpressure()` — construct a `QuotaGuard` with
  `max_ingest_cx_per_sec = 100`; charge 101 CXs in the same window; assert
  `CALYX_QUOTA_EXCEEDED` on the 101st charge. Print:
  `"[STRIDE D] charge_ingest(101) at T = Err(CALYX_QUOTA_EXCEEDED) ✓"`.

### Elevation of privilege — least privilege + no unallowlisted external commands

- [ ] `fn stride_e_elevation_no_external_cmd()` — call a synthetic `run_external_cmd`
  function (stub that checks an allowlist) with a command not on the allowlist
  (`"rm -rf /"` for example); assert `CALYX_EXTERNAL_CMD_NOT_ALLOWED`. Print:
  `"[STRIDE E] external_cmd not allowlisted = Err(CALYX_EXTERNAL_CMD_NOT_ALLOWED) ✓"`.
- [ ] Implement the stub `fn run_external_cmd(cmd: &str, allowlist: &[&str]) -> Result<()>` —
  if `!allowlist.contains(&cmd)` return `CALYX_EXTERNAL_CMD_NOT_ALLOWED` (fail
  closed, A16); otherwise `Ok(())`. This is the minimal allowlist gate the lens
  sandbox must enforce (lenses may not call external commands not in the allowlist —
  `30 §2` Elevation of privilege defense).
- [ ] Add `CALYX_LEDGER_APPEND_ONLY`, `CALYX_EXTERNAL_CMD_NOT_ALLOWED` to
  `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] All six `stride_*` functions are themselves the tests; each asserts the exact
  `CALYX_*` error code and prints the `[STRIDE X] ... ✓` confirmation line.
- [ ] edge (≥3 across the six): Spoofing — `InProcess` authn → no error (not denied);
  Tampering — flip a byte that is *not* in the hash field → chain still detects via
  entry hash re-computation; DoS — quota resets after 1-second window → charge
  accepted again.
- [ ] fail-closed invariant across all six: the denied/rejected path always returns
  a structured `CALYX_*` code, never `unwrap`/`panic`/silent OK.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** all six test functions running via `cargo test`.
- **Readback:** `cargo test -p calyx-aster stride_fsv -- --nocapture 2>&1` must print
  all six `[STRIDE X] ... ✓` lines and exit 0. Screenshot of this output is the
  primary FSV evidence for the STRIDE column of the `SECURITY` predicate.
- **Prove:** before: no STRIDE test harness; after: six independent proofs of six
  independent defense controls, each byte-level (error codes returned, CF bytes
  inspected, audit ring populated). All six pass = STRIDE defense set proven.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence — screenshot of all six `[STRIDE X] ✓` lines — attached to
  the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
